use ds_soundfont::SoundFont;

use crate::{
    pipe::{AudioPipe, AudioStreamParams, ChannelCount},
    voice::Voice,
};

/// Maximum number of simultaneous voices.
const MAX_VOICES: usize = 256;

/// The top-level synthesizer.
///
/// Manages voice allocation, note events, and audio rendering.
/// No CC, no controllers, no automation—just bare note-on/note-off.
pub struct Synthesizer {
    /// Output stream configuration.
    stream_params: AudioStreamParams,
    /// Loaded soundfont.
    soundfont: SoundFont,
    /// Active voices.
    voices: Vec<Voice>,
    /// Pending note-on events: (key, velocity).
    pending_notes: Vec<(u8, u8)>,
    /// Pending note-off events: key.
    pending_note_offs: Vec<u8>,
}

impl Synthesizer {
    pub fn new(stream_params: AudioStreamParams, soundfont: SoundFont) -> Self {
        Self {
            stream_params,
            soundfont,
            voices: Vec::with_capacity(MAX_VOICES),
            pending_notes: Vec::new(),
            pending_note_offs: Vec::new(),
        }
    }

    /// Request a note-on event. The voice will be spawned on the next render call.
    pub fn note_on(&mut self, key: u8, velocity: u8) {
        self.pending_notes.push((key, velocity));
    }

    /// Request a note-off event. Handled on the next render call.
    pub fn note_off(&mut self, key: u8) {
        self.pending_note_offs.push(key);
    }

    /// Get the output stream parameters.
    pub fn stream_params(&self) -> AudioStreamParams {
        self.stream_params
    }

    /// Process pending events (called at start of each render block).
    fn flush_events(&mut self) {
        let note_offs: Vec<u8> = std::mem::take(&mut self.pending_note_offs);
        let note_ons: Vec<(u8, u8)> = std::mem::take(&mut self.pending_notes);

        // Process note-offs first
        for key in note_offs {
            for voice in &mut self.voices {
                if voice.key == key {
                    voice.note_off();
                }
            }
        }

        // Process note-ons
        for (key, vel) in note_ons {
            let params_list = self.soundfont.voice_params(
                0, 0, key, vel, self.stream_params.sample_rate,
            );

            for params in &params_list {
                // Voice stealing if at capacity
                if self.voices.len() >= MAX_VOICES {
                    self.steal_voice();
                }
                self.voices.push(Voice::new(
                    params,
                    self.stream_params.sample_rate,
                    key,
                    vel,
                ));
            }
        }
    }

    /// Steal the quietest releasing voice, or the oldest voice.
    fn steal_voice(&mut self) {
        // Prefer to steal a releasing voice
        if let Some(idx) = self
            .voices
            .iter()
            .position(|v| v.is_releasing())
        {
            self.voices.swap_remove(idx);
            return;
        }
        // Otherwise steal the oldest (first)
        self.voices.remove(0);
    }
}

impl AudioPipe for Synthesizer {
    fn stream_params(&self) -> AudioStreamParams {
        self.stream_params
    }

    fn read_samples(&mut self, buffer: &mut [f32]) {
        self.flush_events();

        match self.stream_params.channels {
            ChannelCount::Mono => {
                for sample in buffer.iter_mut() {
                    let mut mix: f32 = 0.0;
                    let mut i = 0;
                    while i < self.voices.len() {
                        mix += self.voices[i].process();
                        if self.voices[i].finished() {
                            self.voices.swap_remove(i);
                        } else {
                            i += 1;
                        }
                    }
                    *sample = mix;
                }
            }
            ChannelCount::Stereo => {
                for chunk in buffer.chunks_exact_mut(2) {
                    let mut mix: f32 = 0.0;
                    let mut i = 0;
                    while i < self.voices.len() {
                        mix += self.voices[i].process();
                        if self.voices[i].finished() {
                            self.voices.swap_remove(i);
                        } else {
                            i += 1;
                        }
                    }
                    chunk[0] = mix;
                    chunk[1] = mix;
                }
            }
        }
    }
}
