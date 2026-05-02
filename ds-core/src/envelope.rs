use ds_soundfont::types::EnvelopeDescriptor;

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
const SILENCE_THRESHOLD: f32 = 1.0 / 32768.0;

impl Envelope {
    pub fn new(
        descriptor: EnvelopeDescriptor,
        sample_rate: u32,
        allow_release: bool,
        vel: u8,
    ) -> Self {
        let sr = sample_rate as f32;

        // Apply velocity scaling to release (soft notes release faster)
        let vel_factor = vel as f32 / 127.0;
        let release = descriptor.release + vel_factor * 0.0; // Reserve for vel2release

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
                duration_samples: (descriptor.decay * sr).round() as u32,
            },
            // Sustain
            StageParams {
                target: descriptor.sustain,
                duration_samples: 0,
            },
            // Release
            StageParams {
                target: 0.0,
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
                            // Exponential interpolation: start * (target/start)^t
                            if start > 0.001 && params.target > 0.0 {
                                self.value = start * (params.target / start).powf(t);
                            } else {
                                self.value = start + (params.target - start) * t;
                            }
                        }
                        Stage::Attack => {
                            // Concave (quicker attack start)
                            let curved = 1.0 - (1.0 - t).powi(2);
                            self.value = start + (params.target - start) * curved;
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

        if next == Stage::Finished && self.value.abs() < SILENCE_THRESHOLD {
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
