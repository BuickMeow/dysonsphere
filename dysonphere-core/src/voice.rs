use dysonphere_soundfont::types::VoiceParams;

use crate::{envelope::Envelope, sampler::Sampler};

/// A single active voice.
///
/// Combines a sampler (sample playback with pitch) and an envelope
/// (amplitude over time). No per-voice filter, no CC modulation—
/// this is the minimal sound-producing unit.
pub struct Voice {
    /// Sample playback engine.
    sampler: Sampler,
    /// Volume envelope.
    envelope: Envelope,
    /// Master volume (velocity × region volume).
    volume: f32,
    /// Exclusive class for voice stealing.
    exclusive_class: Option<u8>,
    /// MIDI note number this voice is playing.
    pub key: u8,
    /// MIDI velocity (for voice stealing priority).
    pub velocity: u8,
}

impl Voice {
    pub fn new(params: &VoiceParams, sample_rate: u32, key: u8, velocity: u8) -> Self {
        let allow_release = params.loop_mode != dysonphere_soundfont::types::LoopMode::LoopSustain;

        Self {
            sampler: Sampler::new(
                params.sample.clone(),
                params.speed_mult,
                params.loop_mode,
                params.loop_start,
                params.loop_end,
                params.sample_end,
                params.offset,
            ),
            envelope: Envelope::new(params.envelope, sample_rate, allow_release, velocity),
            volume: params.volume,
            exclusive_class: params.exclusive_class,
            key,
            velocity,
        }
    }

    /// Whether this voice has finished and can be removed.
    pub fn finished(&self) -> bool {
        self.envelope.finished() || self.sampler.finished()
    }

    /// Whether the voice is currently releasing.
    pub fn is_releasing(&self) -> bool {
        self.envelope.is_releasing()
    }

    /// Exclusive class for voice stealing.
    pub fn exclusive_class(&self) -> Option<u8> {
        self.exclusive_class
    }

    /// Signal note-off.
    pub fn note_off(&mut self) {
        self.sampler.release();
        self.envelope.release();
    }

    /// Kill the voice immediately (voice stealing).
    pub fn kill(&mut self) {
        self.envelope.kill();
    }

    /// Render one sample. Returns the monophonic output value.
    #[inline]
    pub fn process(&mut self) -> f32 {
        let sample = self.sampler.process();
        self.envelope.process();
        sample * self.envelope.value() * self.volume
    }
}
