/// Streaming audio pipe trait.
///
/// Implementations produce audio samples on demand.
pub trait AudioPipe {
    /// Parameters describing the output audio stream.
    fn stream_params(&self) -> AudioStreamParams;

    /// Render audio into `buffer`. Both channels are interleaved if stereo:
    /// `[L, R, L, R, ...]`.
    fn read_samples(&mut self, buffer: &mut [f32]);
}

/// Output audio stream configuration.
#[derive(Debug, Clone, Copy)]
pub struct AudioStreamParams {
    pub sample_rate: u32,
    pub channels: ChannelCount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelCount {
    Mono,
    Stereo,
}
