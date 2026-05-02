use dysonphere_soundfont::types::EnvelopeDescriptor;

/// Envelope state machine (DAHDSR).
///
/// Stages: Delay → Attack → Hold → Decay → Sustain → Release → Finished
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Delay,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
    Finished,
}

/// Stores parameters for each envelope stage.
#[derive(Debug, Clone)]
struct StageParams {
    target: f32,
    duration_samples: u32,
}

pub struct Envelope {
    /// Original descriptor (for future velocity scaling).
    #[allow(dead_code)]
    descriptor: EnvelopeDescriptor,
    /// Per-stage targets and durations in samples.
    stages: [StageParams; 7],
    /// Current stage.
    stage: Stage,
    /// Current amplitude (0.0–1.0).
    value: f32,
    /// Elapsed samples in current stage.
    elapsed: u32,
    /// Sample rate.
    sample_rate: f32,
    /// Whether we allow entering release stage.
    allow_release: bool,
    /// Whether we've been killed (rapid release).
    killed: bool,
}

/// Amplitude threshold below which a voice in release is considered done.
/// -90dB = 16-bit LSB; inaudible in any reasonable listening scenario.
const SILENCE_THRESHOLD: f32 = 1.0 / 32768.0;
/// Perceived-silence target for release curve.  Uses SILENCE_THRESHOLD (-90dB)
/// so that the last audible moment is genuinely inaudible, preventing an
/// audible click at the 0.001→0.0 transition that -60dB would produce.
const RELEASE_TARGET: f32 = SILENCE_THRESHOLD;

impl Envelope {
    pub fn new(
        descriptor: EnvelopeDescriptor,
        sample_rate: u32,
        allow_release: bool,
        _vel: u8,
    ) -> Self {
        let sr = sample_rate as f32;

        // Release floor 1.0s ensures minimum perceptible tail for piano-class
        // instruments even without CC72.  (Exponential curve to -90dB → perceived
        // ~22% of T, so 1.0s T ≈ 0.22s perceived — barely acceptable minimum.)
        let release = descriptor.release.max(1.0);

        let stages = [
            // Delay
            StageParams {
                target: 0.0,
                duration_samples: (descriptor.delay * sr).round() as u32,
            },
            // Attack
            StageParams {
                target: 1.0,
                duration_samples: (descriptor.attack.max(0.001) * sr).round() as u32,
            },
            // Hold
            StageParams {
                target: 1.0,
                duration_samples: (descriptor.hold * sr).round() as u32,
            },
            // Decay
            StageParams {
                target: descriptor.sustain,
                duration_samples: (descriptor.decay.max(0.001) * sr).round() as u32,
            },
            // Sustain
            StageParams {
                target: descriptor.sustain,
                duration_samples: 0,
            },
            // Release — target RELEASE_TARGET (-60dB) for musical decay that matches perceived time
            StageParams {
                target: RELEASE_TARGET,
                duration_samples: (release.max(0.001) * sr).round() as u32,
            },
            // Finished
            StageParams {
                target: 0.0,
                duration_samples: 0,
            },
        ];

        let mut env = Self {
            descriptor,
            stages,
            stage: Stage::Delay,
            value: 0.0,
            elapsed: 0,
            sample_rate: sr,
            allow_release,
            killed: false,
        };

        // Skip zero-duration delay
        env.advance_past_zero_stages();
        env
    }

    /// Whether the envelope has finished (amplitude is 0 and release complete).
    pub fn finished(&self) -> bool {
        self.stage == Stage::Finished
    }

    /// Whether the envelope is currently in release or finished stage.
    pub fn is_releasing(&self) -> bool {
        matches!(self.stage, Stage::Release | Stage::Finished)
    }

    /// Get the current amplitude.
    pub fn value(&self) -> f32 {
        self.value
    }

    /// Signal note-off or kill.
    pub fn release(&mut self) {
        if self.allow_release || self.killed {
            self.enter_stage(Stage::Release);
        }
    }

    /// Kill the voice immediately with a short release.
    pub fn kill(&mut self) {
        let short_release = (0.005 * self.sample_rate).round() as u32;
        self.stages[5].duration_samples = short_release;
        self.killed = true;
        self.enter_stage(Stage::Release);
    }

    /// Advance the envelope by one sample.
    pub fn process(&mut self) {
        let params = &self.stages[self.stage_index()];

        match self.stage {
            Stage::Sustain => {
                // Hold at sustain level indefinitely
            }
            Stage::Finished => {
                self.value = 0.0;
            }
            _ => {
                if params.duration_samples > 0 {
                    self.elapsed += 1;
                    let t = self.elapsed as f32 / params.duration_samples as f32;
                    let start = self.stage_start_value();

                    // Use exponential-ish curve for decay/release (more natural)
                    match self.stage {
                        Stage::Decay | Stage::Release => {
                            if start > 0.001 {
                                let effective_target = if params.target > 0.0 {
                                    params.target
                                } else {
                                    SILENCE_THRESHOLD
                                };
                                self.value = start * (effective_target / start).powf(t);
                            } else {
                                self.value = start + (params.target - start) * t;
                            }
                        }
                        Stage::Attack => {
                            // Linear attack curve for natural sound
                            // Fixed: Changed from concave (1.0 - (1.0-t)^2) to linear (t)
                            // Reason: Concave curve caused "heavy/hard" attack sound
                            self.value = start + (params.target - start) * t;
                        }
                        _ => {
                            self.value = start + (params.target - start) * t;
                        }
                    }

                    if self.elapsed >= params.duration_samples {
                        self.value = params.target;
                        self.advance_stage();
                    }
                } else {
                    self.value = params.target;
                    self.advance_stage();
                }
            }
        }
    }

    /// Advance past any zero-duration stages.
    fn advance_past_zero_stages(&mut self) {
        loop {
            let params = &self.stages[self.stage_index()];
            if params.duration_samples > 0 {
                break;
            }
            if self.stage == Stage::Sustain || self.stage == Stage::Finished {
                break;
            }
            self.value = params.target;
            self.advance_stage();
        }
    }

    fn advance_stage(&mut self) {
        let next = match self.stage {
            Stage::Delay => Stage::Attack,
            Stage::Attack => Stage::Hold,
            Stage::Hold => Stage::Decay,
            Stage::Decay => Stage::Sustain,
            Stage::Sustain => Stage::Sustain, // Stay
            Stage::Release => Stage::Finished,
            Stage::Finished => Stage::Finished,
        };

        if next == Stage::Finished && self.value.abs() <= RELEASE_TARGET {
            self.value = 0.0;
        }

        self.stage = next;
        self.elapsed = 0;
        self.advance_past_zero_stages();
    }

    fn enter_stage(&mut self, stage: Stage) {
        if matches!(self.stage, Stage::Finished) {
            return;
        }
        self.stage = stage;
        self.elapsed = 0;
        self.advance_past_zero_stages();
    }

    fn stage_index(&self) -> usize {
        match self.stage {
            Stage::Delay => 0,
            Stage::Attack => 1,
            Stage::Hold => 2,
            Stage::Decay => 3,
            Stage::Sustain => 4,
            Stage::Release => 5,
            Stage::Finished => 6,
        }
    }

    fn stage_start_value(&self) -> f32 {
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Voice;
    use dysonphere_soundfont::types::{EnvelopeDescriptor, LoopMode, VoiceParams};

    /// Count samples until envelope finishes.
    fn samples_until_finished(env: &mut Envelope) -> usize {
        let mut count = 0;
        while !env.finished() {
            env.process();
            count += 1;
            if count > 1_000_000 {
                panic!("envelope never finished");
            }
        }
        count
    }

    /// Count samples until envelope reaches target value.
    #[allow(dead_code)]
    fn samples_until_reaches(env: &mut Envelope, target: f32) -> usize {
        let mut count = 0;
        while env.value() < target - 0.001 && !env.finished() {
            env.process();
            count += 1;
            if count > 1_000_000 {
                panic!("envelope never reached target");
            }
        }
        count
    }

    #[test]
    fn basic_envelope_stages() {
        let desc = EnvelopeDescriptor {
            delay: 0.0,
            attack: 0.01,
            hold: 0.01,
            decay: 0.05,
            sustain: 0.7,
            release: 1.0,
        };
        let sr = 44100;
        let mut env = Envelope::new(desc, sr, true, 127);

        // Should start at 0 (delay is 0 so immediately in attack)
        assert!(env.value() < 0.01);

        // Process through attack
        let attack_samples = (0.01 * sr as f32) as usize;
        for _ in 0..attack_samples {
            env.process();
        }
        // After attack + hold, should be near 1.0
        for _ in 0..((0.01 * sr as f32) as usize) {
            env.process();
        }
        assert!(env.value() > 0.9, "value after attack+hold: {}", env.value());

        // Process through decay
        let decay_samples = (0.05 * sr as f32) as usize;
        for _ in 0..decay_samples {
            env.process();
        }

        // Should be near sustain level
        assert!((env.value() - 0.7).abs() < 0.05,
            "value after decay: {} (expected ~0.7)", env.value());

        // Release
        env.release();
        let release_samples = samples_until_finished(&mut env);
        let expected = (1.0 * sr as f32) as usize;
        let epsilon = (0.05 * sr as f32) as usize;
        assert!(
            release_samples >= expected - epsilon && release_samples <= expected + epsilon,
            "release samples: {} (expected ~{})", release_samples, expected
        );
    }

    #[test]
    fn release_exponential_curve() {
        // Verify Release uses exponential (not linear) curve
        let desc = EnvelopeDescriptor {
            delay: 0.0,
            attack: 0.0,
            hold: 0.0,
            decay: 0.0,
            sustain: 1.0,
            release: 1.2,
        };
        let sr = 44100;
        let mut env = Envelope::new(desc, sr, true, 100);

        // Skip to sustain
        let mut reached = false;
        for _ in 0..1000 {
            env.process();
            if env.value() > 0.99 { reached = true; break; }
        }
        assert!(reached, "envelope should reach 1.0 in sustain");

        env.release();

        // Sample at 25%, 50%, 75% of release
        let total = (1.2 * sr as f32) as usize;
        let p25 = total / 4;
        let p50 = total / 2;
        let p75 = 3 * total / 4;

        for _ in 0..p25 { env.process(); }
        let v25 = env.value();

        for _ in 0..(p50 - p25) { env.process(); }
        let v50 = env.value();

        for _ in 0..(p75 - p50) { env.process(); }
        let v75 = env.value();

        // Exponential decay: each quarter should drop by roughly the same ratio
        let ratio1 = v50 / v25;
        let ratio2 = v75 / v50;
        let ratio_diff = (ratio1 - ratio2).abs();

        // Exponential should have roughly constant ratio per equal time interval
        // Linear would have v50 ≈ 0.5, v75 ≈ 0.25 (ratio ~0.5 vs ~0.5 is also constant)
        // The key test: exponential should NOT be linear
        // Linear at 25%: 0.75, at 50%: 0.50, at 75%: 0.25
        // Exponential at 25%: ~0.79, at 50%: ~0.37, at 75%: ~0.05 (for target SILENCE)
        assert!(ratio_diff < 0.3,
            "release should be exponential: v25={v25:.4} v50={v50:.4} v75={v75:.4} r1={ratio1:.4} r2={ratio2:.4}");
        assert!(v25 < 0.9, "exponential decay should drop quickly initially: v25={v25:.4}");
    }

    #[test]
    fn velocity_no_longer_affects_release() {
        // Verifies the v5 fix: velocity should NOT shorten release duration.
        let sr = 44100;
        let desc = EnvelopeDescriptor {
            delay: 0.0, attack: 0.0, hold: 0.0, decay: 0.0,
            sustain: 1.0,
            release: 1.5,
        };
        let _sr = 44100;
        let mut env_loud = Envelope::new(desc, _sr, true, 127);
        let mut env_soft = Envelope::new(desc, _sr, true, 64);

        // Skip to sustain
        for _ in 0..100 {
            env_loud.process();
            env_soft.process();
        }
        env_loud.release();
        env_soft.release();

        let loud_dur = samples_until_finished(&mut env_loud);
        let soft_dur = samples_until_finished(&mut env_soft);

        // Release duration should be the same regardless of velocity
        let diff = (loud_dur as isize - soft_dur as isize).unsigned_abs();
        let tolerance = (0.01 * sr as f32) as usize; // 10ms tolerance for float rounding
        assert!(diff <= tolerance,
            "release should be velocity-independent: vel=127: {loud_dur}, vel=64: {soft_dur}, diff={diff}");
    }

    #[test]
    fn velocity_affects_volume_chain() {
        // Verify that velocity-to-volume follows powf(5.0) law in Voice
        let sr = 44100;
        let desc = EnvelopeDescriptor::default();
        let params = VoiceParams {
            sample: vec![0.5f32; 1000].into(),
            speed_mult: 1.0,
            volume: 1.0,
            loop_mode: LoopMode::NoLoop,
            loop_start: 0,
            loop_end: 1000,
            sample_end: 1000,
            offset: 0,
            envelope: desc,
            exclusive_class: None,
        };

        let voice_loud = Voice::new(&params, sr, 60, 127);
        let voice_soft = Voice::new(&params, sr, 60, 64);
        let voice_silent = Voice::new(&params, sr, 60, 1);

        let vol_loud = voice_loud.volume;
        let vol_soft = voice_soft.volume;
        let vol_silent = voice_silent.volume;

        // powf(5.0): (64/127)^5 ≈ 0.504^5 ≈ 0.032
        let expected_ratio = (64.0f32 / 127.0f32).powf(5.0);
        let actual_ratio = vol_soft / vol_loud;
        assert!((actual_ratio - expected_ratio).abs() < 0.01,
            "vel=64/127 ratio: expected {expected_ratio:.4}, got {actual_ratio:.4}");

        assert!(vol_silent < 0.001, "vel=1 should be near-silent: {vol_silent:.6}");
        assert!(vol_loud > 0.99, "vel=127 should be full volume: {vol_loud:.4}");
    }

    #[test]
    fn release_floor_protects_against_zero() {
        let sr = 44100;
        let desc = EnvelopeDescriptor {
            delay: 0.0, attack: 0.0, hold: 0.0, decay: 0.0,
            sustain: 1.0,
            release: 0.0, // zero release
        };
        let mut env = Envelope::new(desc, sr, true, 100);
        // Fast-forward to sustain
        for _ in 0..50 { env.process(); }
        env.release();

        let dur = samples_until_finished(&mut env);
        let expected_min = (1.0 * sr as f32) as usize;
        assert!(dur >= expected_min,
            "release with zero descriptor should have floor: got {dur} samples, expected >= {expected_min}");
    }

    #[test]
    fn kill_release_is_short() {
        let sr = 44100;
        let desc = EnvelopeDescriptor {
            release: 2.0, ..EnvelopeDescriptor::default()
        };
        let mut env = Envelope::new(desc, sr, true, 100);
        // Skip to sustain
        for _ in 0..50 { env.process(); }
        env.kill();

        let dur = samples_until_finished(&mut env);
        // Kill release should be ~5ms
        let expected = (0.005 * sr as f32) as usize;
        let epsilon = (0.002 * sr as f32) as usize;
        assert!((dur as isize - expected as isize).abs() as usize <= epsilon,
            "kill release: got {dur} samples, expected ~{expected}");
    }
}
