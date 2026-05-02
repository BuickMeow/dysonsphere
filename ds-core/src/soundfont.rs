use ds_soundfont::types::VoiceParams;

use crate::pipe::AudioStreamParams;

/// Trait for loaded soundfonts that can be shared across synthesizer instances.
///
/// This enables a global soundfont cache pattern: load a SF2/SFZ once,
/// share it via `Arc<dyn SoundfontBase>` across multiple engine instances.
pub trait SoundfontBase: Send + Sync + std::fmt::Debug {
    fn stream_params(&self) -> AudioStreamParams;
    fn voice_params(&self, bank: u8, program: u8, key: u8, vel: u8) -> Vec<VoiceParams>;
    fn presets(&self) -> Vec<PresetInfo>;
}

/// Minimal preset info for GUI display.
#[derive(Clone, Debug)]
pub struct PresetInfo {
    pub name: String,
    pub bank: u16,
    pub program: u16,
    pub source_file: String,
}

/// Errors from loading a soundfont via `SampleSoundfont`.
#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Parse(String),
    UnsupportedFormat,
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "IO error: {e}"),
            LoadError::Parse(s) => write!(f, "Parse error: {s}"),
            LoadError::UnsupportedFormat => write!(f, "Unsupported soundfont format"),
        }
    }
}

impl std::error::Error for LoadError {}

/// High-level soundfont loader — drop-in replacement for xsynth's `SampleSoundfont::new`.
pub struct SampleSoundfont {
    wrapper: SoundFontWrapper,
}

impl SampleSoundfont {
    pub fn new(
        path: impl AsRef<std::path::Path>,
        stream_params: AudioStreamParams,
    ) -> Result<Self, LoadError> {
        let path = path.as_ref();
        let name = path.to_string_lossy().to_string();

        let sf = match path.extension().and_then(|e| e.to_str()) {
            Some("sf2") | Some("SF2") => {
                ds_soundfont::sf2::load(path, stream_params.sample_rate)
                    .map_err(|e| LoadError::Parse(e.to_string()))
            }
            Some("sfz") | Some("SFZ") => {
                ds_soundfont::sfz::load(path, stream_params.sample_rate)
                    .map_err(|e| LoadError::Parse(e.to_string()))
            }
            _ => return Err(LoadError::UnsupportedFormat),
        }?;

        Ok(Self {
            wrapper: SoundFontWrapper::new(sf, stream_params, name),
        })
    }
}

impl std::fmt::Debug for SampleSoundfont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.wrapper.fmt(f)
    }
}

impl SoundfontBase for SampleSoundfont {
    fn stream_params(&self) -> AudioStreamParams {
        self.wrapper.stream_params()
    }
    fn voice_params(&self, bank: u8, program: u8, key: u8, vel: u8) -> Vec<VoiceParams> {
        self.wrapper.voice_params(bank, program, key, vel)
    }
    fn presets(&self) -> Vec<PresetInfo> {
        self.wrapper.presets()
    }
}

/// Wraps a raw `ds_soundfont::SoundFont` for the `SoundfontBase` trait.
pub struct SoundFontWrapper {
    inner: ds_soundfont::SoundFont,
    stream_params: AudioStreamParams,
    source_name: String,
    preset_cache: Vec<PresetInfo>,
}

impl SoundFontWrapper {
    pub fn new(
        soundfont: ds_soundfont::SoundFont,
        stream_params: AudioStreamParams,
        source_name: String,
    ) -> Self {
        let preset_cache = soundfont
            .presets
            .iter()
            .map(|p| PresetInfo {
                name: p.name.clone(),
                bank: p.bank,
                program: p.program,
                source_file: source_name.clone(),
            })
            .collect();

        Self {
            inner: soundfont,
            stream_params,
            source_name,
            preset_cache,
        }
    }
}

impl std::fmt::Debug for SoundFontWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SoundFontWrapper")
            .field("source", &self.source_name)
            .field("presets", &self.preset_cache.len())
            .finish()
    }
}

impl SoundfontBase for SoundFontWrapper {
    fn stream_params(&self) -> AudioStreamParams {
        self.stream_params
    }
    fn voice_params(&self, bank: u8, program: u8, key: u8, vel: u8) -> Vec<VoiceParams> {
        self.inner
            .voice_params(bank, program, key, vel, self.stream_params.sample_rate)
    }
    fn presets(&self) -> Vec<PresetInfo> {
        self.preset_cache.clone()
    }
}
