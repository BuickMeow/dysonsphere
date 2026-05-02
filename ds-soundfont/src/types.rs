use std::{ops::RangeInclusive, sync::Arc};

/// How a sample loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoopMode {
    #[default]
    NoLoop,
    LoopContinuous,
    LoopSustain,
}

/// Envelope timing descriptor, in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvelopeDescriptor {
    pub delay: f32,
    pub attack: f32,
    pub hold: f32,
    pub decay: f32,
    pub sustain: f32,  // 0.0–1.0 level
    pub release: f32,
}

impl Default for EnvelopeDescriptor {
    fn default() -> Self {
        Self {
            delay: 0.0,
            attack: 0.01,
            hold: 0.0,
            decay: 0.0,
            sustain: 1.0,
            release: 0.01,
        }
    }
}

/// Parameters for rendering a single region at a specific note and velocity.
#[derive(Clone)]
pub struct VoiceParams {
    /// Reference to the monophonic sample data.
    pub sample: Arc<[f32]>,
    /// Playback speed multiplier relative to root key.
    pub speed_mult: f32,
    /// Amplitude (0.0–1.0, linear).
    pub volume: f32,
    /// Loop mode.
    pub loop_mode: LoopMode,
    /// Loop start (sample index at output rate).
    pub loop_start: u32,
    /// Loop end (sample index at output rate).
    pub loop_end: u32,
    /// Where the sample ends (for NoLoop/OneShot). u32::MAX if not applicable.
    pub sample_end: u32,
    /// Start offset (sample index).
    pub offset: u32,
    /// Envelope descriptor.
    pub envelope: EnvelopeDescriptor,
    /// Exclusive class for voice stealing.
    pub exclusive_class: Option<u8>,
}

/// A single SF2/SFZ region, with all parameters resolved.
#[derive(Clone)]
pub struct Region {
    pub key_range: RangeInclusive<u8>,
    pub vel_range: RangeInclusive<u8>,
    pub root_key: u8,
    pub sample: Arc<[f32]>,
    pub original_sample_rate: u32,
    pub volume: f32,
    pub pan: f32,
    pub loop_mode: LoopMode,
    pub loop_start: u32,
    pub loop_end: u32,
    pub sample_end: u32,
    pub offset: u32,
    pub envelope: EnvelopeDescriptor,
    pub fine_tune_cents: f32,
    pub exclusive_class: Option<u8>,
}

/// A preset (bank + program + regions).
#[derive(Clone)]
pub struct Preset {
    pub bank: u16,
    pub program: u16,
    pub name: String,
    pub regions: Vec<Region>,
}

/// Top-level loaded soundfont.
pub struct SoundFont {
    pub presets: Vec<Preset>,
    /// Holds ownership of all sample data.
    pub sample_buffers: Vec<Arc<[f32]>>,
}

impl SoundFont {
    /// Find the voice params for a given (bank, program, key, velocity).
    pub fn voice_params(
        &self,
        bank: u8,
        program: u8,
        key: u8,
        vel: u8,
        _sample_rate: u32,
    ) -> Vec<VoiceParams> {
        let preset = self
            .presets
            .iter()
            .find(|p| p.bank as u8 == bank && p.program as u8 == program);

        let Some(preset) = preset else {
            return Vec::new();
        };

        let mut result = Vec::new();
        for region in &preset.regions {
            if !region.key_range.contains(&key) || !region.vel_range.contains(&vel) {
                continue;
            }

            let cents = (key as f32 - region.root_key as f32) * 100.0 + region.fine_tune_cents;
            let speed_mult = 2.0f32.powf(cents / 1200.0);

            let env = region.envelope;

            result.push(VoiceParams {
                sample: region.sample.clone(),
                speed_mult,
                volume: region.volume,
                loop_mode: region.loop_mode,
                loop_start: region.loop_start,
                loop_end: region.loop_end,
                sample_end: region.sample_end,
                offset: region.offset,
                envelope: env,
                exclusive_class: region.exclusive_class,
            });
        }

        result
    }
}

/// Helper: convert cents to a frequency multiplier.
pub fn cents_factor(cents: f32) -> f32 {
    2.0f32.powf(cents / 1200.0)
}

/// Helper: convert dB to linear amplitude.
pub fn db_to_amp(db: f32) -> f32 {
    10.0f32.powf(db / 20.0)
}

/// Convert a sample index when resampling.
pub fn convert_sample_index(idx: u32, old_rate: u32, new_rate: u32) -> u32 {
    (new_rate as f64 * idx as f64 / old_rate as f64).round() as u32
}
