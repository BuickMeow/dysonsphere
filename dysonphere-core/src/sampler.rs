use std::sync::Arc;

use dysonphere_soundfont::types::LoopMode;

/// Sample playback state.
///
/// Tracks position within a sample buffer, handling pitch speed,
/// looping, and interpolation.
pub struct Sampler {
    /// The sample data (mono, f32).
    data: Arc<[f32]>,
    /// Current playback position (fractional, in samples).
    position: f64,
    /// Sample increment per output sample (controls pitch).
    speed: f64,
    /// Loop mode.
    loop_mode: LoopMode,
    /// Loop region start (in samples).
    loop_start: u32,
    /// Loop region end (in samples).
    loop_end: u32,
    /// Where the sample truly ends (for NoLoop/Sustain release).
    sample_end: u32,
    /// Whether the note has been released (for LoopSustain).
    released: bool,
    /// The position at release time (for LoopSustain).
    position_at_release: f64,
}

impl Sampler {
    pub fn new(
        data: Arc<[f32]>,
        speed: f32,
        loop_mode: LoopMode,
        loop_start: u32,
        loop_end: u32,
        sample_end: u32,
        offset: u32,
    ) -> Self {
        Self {
            data,
            position: offset as f64,
            speed: speed as f64,
            loop_mode,
            loop_start,
            loop_end,
            sample_end,
            released: false,
            position_at_release: 0.0,
        }
    }

    /// Whether the sampler has passed the end of the sample.
    pub fn finished(&self) -> bool {
        match self.loop_mode {
            LoopMode::LoopContinuous => false,
            LoopMode::LoopSustain if !self.released => false,
            LoopMode::OneShot => self.position >= self.sample_end as f64,
            _ => self.position >= self.sample_end as f64,
        }
    }

    /// Signal note release.
    pub fn release(&mut self) {
        match self.loop_mode {
            LoopMode::LoopSustain if !self.released => {
                self.released = true;
                // Record position for release-tail playback from loop_end onward.
                // This matches xsynth's SampleReaderLoopSustain::signal_release.
                self.position_at_release = self.position;
            }
            LoopMode::LoopContinuous => {
                // Keep looping but envelope fade will handle it
            }
            LoopMode::OneShot => {
                // One-shot ignores note-off — play to natural end
            }
            _ => {} // NoLoop: nothing to do
        }
    }

    /// Read the next sample and advance position.
    pub fn process(&mut self) -> f32 {
        // LoopSustain release: after note-off, playback continues from loop_end
        // (the natural release tail) rather than from wherever the loop position
        // happened to be.  This mirrors xsynth's SampleReaderLoopSustain behaviour
        // and prevents playing non-loop data that may contain attack transients.
        let read_pos = if self.released && self.loop_mode == LoopMode::LoopSustain {
            self.loop_end as f64 + (self.position - self.position_at_release)
        } else {
            self.position
        };
        let sample = self.read_sample(read_pos);

        self.position += self.speed;

        // Handle looping
        if self.position >= self.loop_end as f64 {
            match self.loop_mode {
                LoopMode::LoopContinuous => {
                    self.position -= (self.loop_end - self.loop_start) as f64;
                }
                LoopMode::LoopSustain if !self.released => {
                    self.position_at_release = self.position;
                    self.position -= (self.loop_end - self.loop_start) as f64;
                }
                _ => {}
            }
        }

        sample
    }

    /// Linear interpolation read at a fractional position.
    #[inline]
    fn read_sample(&self, pos: f64) -> f32 {
        let idx = pos as usize;
        let frac = (pos - idx as f64) as f32;

        let a = self.get(idx);
        // At the sample_end boundary, use a for b as well to avoid a
        // discontinuity between interpolated and held-last-sample values.
        let b = if idx + 1 >= self.sample_end as usize {
            a
        } else {
            self.get(idx + 1)
        };

        a + (b - a) * frac
    }

    /// Get sample at index, returning last valid value for out-of-bounds.
    #[inline]
    fn get(&self, idx: usize) -> f32 {
        // LoopContinuous never leaves the loop region, so it never hits sample_end.
        // All other modes can advance past sample_end, where we hold the last sample.
        if self.loop_mode != LoopMode::LoopContinuous && idx >= self.sample_end as usize {
            return self.data.get(self.sample_end as usize - 1)
                .copied().unwrap_or(0.0);
        }
        self.data.get(idx).copied().unwrap_or(0.0)
    }
}
