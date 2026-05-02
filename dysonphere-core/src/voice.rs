use dysonphere_soundfont::types::{LoopMode, VoiceParams};

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
    pub(crate) volume: f32,
    /// Exclusive class for voice stealing.
    exclusive_class: Option<u8>,
    /// MIDI note number this voice is playing.
    pub key: u8,
    /// MIDI velocity (for voice stealing priority).
    pub velocity: u8,
    /// Whether this voice is being held by the damper/sustain pedal.
    pub damper_sustained: bool,
}

impl Voice {
    pub fn new(params: &VoiceParams, sample_rate: u32, key: u8, velocity: u8) -> Self {
        let vel_norm = velocity as f32 / 127.0;
        // SF2 default velocity→attenuation modulator: concave curve, ~960cb range.
        // Approximated as powf(5.0) → vel=64: 0.031 (-30dB), vel=127: 1.0, vel=1: ~0.
        // This mirrors xsynth's default_note_modulators() attenuation behavior.
        let vel_amp = vel_norm.powf(5.0);
        let allow_release = !matches!(params.loop_mode, LoopMode::OneShot);

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
            volume: params.volume * vel_amp,
            exclusive_class: params.exclusive_class,
            key,
            velocity,
            damper_sustained: false,
        }
    }

    /// Whether this voice has finished and can be removed.
    pub fn finished(&self) -> bool {
        if self.envelope.finished() {
            return true;
        }
        // During release, the sampler may end before the envelope tail fades out.
        // Only allow sampler to kill the voice when we're NOT releasing.
        if !self.envelope.is_releasing() && self.sampler.finished() {
            return true;
        }
        false
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
