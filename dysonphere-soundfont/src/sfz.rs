use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::types::{
    db_to_amp, EnvelopeDescriptor, LoopMode, Preset, Region, SoundFont,
};

/// Parses an SFZ file and returns a unified SoundFont.
pub fn load(path: impl AsRef<Path>, sample_rate: u32) -> Result<SoundFont, Error> {
    let path = path.as_ref();
    let base_dir = path
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

    let text = load_with_includes(path, &base_dir)?;

    let tokens = tokenize(&text);
    let regions = parse_regions(&tokens, &base_dir)?;

    if regions.is_empty() {
        return Err(Error::Parse("No regions found in SFZ file".into()));
    }

    // Collect unique sample files and load them (with their original sample rates)
    let mut sample_cache: HashMap<PathBuf, (Arc<[f32]>, u32)> = HashMap::new();
    let mut sample_buffers: Vec<Arc<[f32]>> = Vec::new();

    for region in &regions {
        if let std::collections::hash_map::Entry::Vacant(entry) =
            sample_cache.entry(region.sample_path.clone())
        {
            let (raw, orig_rate) = load_wav(&region.sample_path)?;
            let data: Arc<[f32]> = if orig_rate != sample_rate && !raw.is_empty() {
                resample_vec(&raw, orig_rate, sample_rate).into()
            } else {
                raw.into()
            };
            sample_buffers.push(data.clone());
            entry.insert((data, orig_rate));
        }
    }

    let mut preset_regions = Vec::new();
    for region in &regions {
        let (sample, orig_rate) = sample_cache[&region.sample_path].clone();
        let loop_start = convert_sample_index(
            region.loop_start,
            orig_rate,
            sample_rate,
        )
        .min(sample.len() as u32);
        let loop_end = convert_sample_index(
            region.loop_end,
            orig_rate,
            sample_rate,
        )
        .min(sample.len() as u32);
        let offset = convert_sample_index(
            region.offset,
            orig_rate,
            sample_rate,
        )
        .min(sample.len() as u32);
        let sample_end = sample.len() as u32;

        let volume = db_to_amp(region.volume as f32);

        preset_regions.push(Region {
            key_range: (region.lokey as u8)..=(region.hikey as u8),
            vel_range: region.lovel..=region.hivel,
            root_key: region.pitch_keycenter as u8,
            sample,
            original_sample_rate: orig_rate,
            volume,
            pan: region.pan as f32,
            loop_mode: region.loop_mode,
            loop_start,
            loop_end,
            sample_end,
            offset,
            envelope: region.envelope,
            fine_tune_cents: region.tune as f32,
            exclusive_class: None,
        });
    }

    Ok(SoundFont {
        presets: vec![Preset {
            bank: 0,
            program: 0,
            name: "SFZ Preset".into(),
            regions: preset_regions,
        }],
        sample_buffers,
    })
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error, PathBuf),
    Parse(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e, p) => write!(f, "IO error reading {}: {e}", p.display()),
            Error::Parse(s) => write!(f, "Parse error: {s}"),
        }
    }
}

impl std::error::Error for Error {}

// ── Include / preprocessor ──────────────────────────────────────────

fn load_with_includes(path: &Path, base_dir: &Path) -> Result<String, Error> {
    let text = fs::read_to_string(path).map_err(|e| Error::Io(e, path.to_path_buf()))?;
    let mut result = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("#include") {
            let rest = trimmed["#include".len()..].trim();
            let include_path = rest.trim_matches('"').replace('\\', "/");
            let resolved = base_dir.join(&include_path);
            if let Ok(include_text) = load_with_includes(&resolved, &resolved.parent().unwrap_or(base_dir)) {
                result.push_str(&include_text);
            }
            // Non-fatal: silently skip missing includes
        } else if trimmed.starts_with("#define") {
            // Minimal #define support: store definition for later substitution
            // For now, just skip defines — most SFZ files work without them
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }
    Ok(result)
}

// ── Tokenization ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Token {
    Header(String),
    Opcode(String, String),
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if line.starts_with("<") && line.ends_with(">") {
            tokens.push(Token::Header(line[1..line.len() - 1].to_lowercase()));
        } else if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim().to_lowercase();
            let value = line[eq_pos + 1..].trim();
            // Remove inline comment
            let value = match value.find("//") {
                Some(pos) => value[..pos].trim(),
                None => value,
            };
            tokens.push(Token::Opcode(key, value.to_string()));
        }
    }
    tokens
}

// ── Region building ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct PendingRegion {
    sample_filename: PathBuf,
    default_path: Option<PathBuf>,
    sample_path: PathBuf,
    lokey: i8,
    hikey: i8,
    lovel: u8,
    hivel: u8,
    pitch_keycenter: i8,
    volume: i16,    // dB
    pan: i8,        // -100 to 100
    tune: i16,      // cents
    loop_mode: LoopMode,
    loop_start: u32,
    loop_end: u32,
    offset: u32,
    envelope: EnvelopeDescriptor,
    cutoff: Option<f32>,
    resonance: f32,
}

impl Default for PendingRegion {
    fn default() -> Self {
        Self {
            sample_filename: PathBuf::new(),
            default_path: None,
            sample_path: PathBuf::new(),
            lokey: 0,
            hikey: 127,
            lovel: 0,
            hivel: 127,
            pitch_keycenter: 60,
            volume: 0,
            pan: 0,
            tune: 0,
            loop_mode: LoopMode::NoLoop,
            loop_start: 0,
            loop_end: 0,
            offset: 0,
            envelope: EnvelopeDescriptor::default(),
            cutoff: None,
            resonance: 0.0,
        }
    }
}

fn parse_regions(tokens: &[Token], base_dir: &Path) -> Result<Vec<PendingRegion>, Error> {
    let mut regions = Vec::new();
    let mut stack: Vec<PendingRegion> = vec![PendingRegion::default()]; // [global, master, group, region]
    let mut in_control = false;

    for token in tokens {
        match token {
            Token::Header(h) => {
                in_control = h == "control";
                if in_control {
                    continue;
                }

                let level = match h.as_str() {
                    "global" | "master" => 1,
                    "group" => 3,
                    "region" => {
                        if stack.len() >= 4 {
                            let region = stack.pop().unwrap();
                            if let Some(built) = finalize_region(region, base_dir) {
                                regions.push(built);
                            }
                        }
                        4
                    }
                    _ => continue,
                };

                while stack.len() < level {
                    let parent = stack.last().cloned().unwrap_or_default();
                    stack.push(parent);
                }
                while stack.len() > level {
                    stack.pop();
                }
            }
            Token::Opcode(key, value) => {
                let current = if in_control {
                    // In <control>: only handle global settings like default_path
                    if key != "default_path" {
                        continue;
                    }
                    stack.last_mut()
                } else {
                    stack.last_mut()
                };

                if let Some(current) = current {
                    apply_opcode(current, key, value);
                }
            }
        }
    }

    // Final region
    if stack.len() >= 4 {
        let region = stack.pop().unwrap();
        if let Some(built) = finalize_region(region, base_dir) {
            regions.push(built);
        }
    }

    Ok(regions)
}

fn apply_opcode(region: &mut PendingRegion, key: &str, value: &str) {
    let parse_f32 = || value.parse::<f32>().ok();
    let parse_i16 = || value.parse::<i16>().ok();
    let parse_i8 = || value.parse::<i8>().ok();
    let parse_u8 = || value.parse::<u8>().ok();
    let parse_u32 = || value.parse::<u32>().ok();

    match key {
        "sample" => {
            region.sample_filename = PathBuf::from(value.replace('\\', "/"));
        }
        "default_path" => {
            region.default_path = Some(PathBuf::from(value.replace('\\', "/")));
        }
        "lokey" => {
            if let Some(v) = parse_i8() {
                region.lokey = v
            }
        }
        "hikey" => {
            if let Some(v) = parse_i8() {
                region.hikey = v
            }
        }
        "key" => {
            if let Some(v) = parse_i8() {
                region.lokey = v;
                region.hikey = v;
                region.pitch_keycenter = v;
            }
        }
        "lovel" => {
            if let Some(v) = parse_u8() {
                region.lovel = v
            }
        }
        "hivel" => {
            if let Some(v) = parse_u8() {
                region.hivel = v
            }
        }
        "pitch_keycenter" => {
            if let Some(v) = parse_i8() {
                region.pitch_keycenter = v
            }
        }
        "volume" => {
            if let Some(v) = parse_i16() {
                region.volume = v
            }
        }
        "pan" => {
            if let Some(v) = parse_i8() {
                region.pan = v
            }
        }
        "tune" => {
            if let Some(v) = parse_i16() {
                region.tune = v
            }
        }
        "loop_mode" => {
            region.loop_mode = match value {
                "no_loop" => LoopMode::NoLoop,
                "loop_continuous" => LoopMode::LoopContinuous,
                "loop_sustain" => LoopMode::LoopSustain,
                "one_shot" => LoopMode::OneShot,
                _ => LoopMode::NoLoop,
            };
        }
        "loop_start" => {
            if let Some(v) = parse_u32() {
                region.loop_start = v
            }
        }
        "loop_end" => {
            if let Some(v) = parse_u32() {
                region.loop_end = v
            }
        }
        "offset" => {
            if let Some(v) = parse_u32() {
                region.offset = v
            }
        }
        "ampeg_delay" => {
            if let Some(v) = parse_f32() {
                region.envelope.delay = v
            }
        }
        "ampeg_attack" => {
            if let Some(v) = parse_f32() {
                region.envelope.attack = v
            }
        }
        "ampeg_hold" => {
            if let Some(v) = parse_f32() {
                region.envelope.hold = v
            }
        }
        "ampeg_decay" => {
            if let Some(v) = parse_f32() {
                region.envelope.decay = v
            }
        }
        "ampeg_sustain" => {
            if let Some(v) = parse_f32() {
                region.envelope.sustain = v / 100.0
            }
        }
        "ampeg_release" => {
            if let Some(v) = parse_f32() {
                region.envelope.release = v
            }
        }
        "cutoff" => {
            region.cutoff = parse_f32()
        }
        "resonance" => {
            if let Some(v) = parse_f32() {
                region.resonance = v
            }
        }
        _ => {}
    }
}

fn finalize_region(
    mut region: PendingRegion,
    base_dir: &Path,
) -> Option<PendingRegion> {
    if region.sample_filename.as_os_str().is_empty() {
        return None;
    }

    // Merge default_path + sample_filename, then resolve relative to base_dir
    let sample_rel = match &region.default_path {
        Some(dp) => dp.join(&region.sample_filename),
        None => region.sample_filename.clone(),
    };

    let sample_path = base_dir.join(&sample_rel);
    match sample_path.canonicalize() {
        Ok(p) => {
            region.sample_path = p;
            Some(region)
        }
        Err(_) => None,
    }
}

// ── WAV loading (minimal, handles 16-bit mono/stereo PCM) ──────────

/// Returns (samples, sample_rate)
fn load_wav(path: &Path) -> Result<(Vec<f32>, u32), Error> {
    let data = fs::read(path).map_err(|e| Error::Io(e, path.to_path_buf()))?;

    if data.len() < 44 {
        return Err(Error::Parse("WAV file too short".into()));
    }

    // Verify RIFF/WAVE header
    if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(Error::Parse("Not a valid WAV file".into()));
    }

    // Read fmt chunk to get audio parameters
    if &data[12..16] != b"fmt " {
        return Err(Error::Parse("Missing fmt chunk in WAV".into()));
    }
    let fmt_size = u32::from_le_bytes([data[16], data[17], data[18], data[19]]) as usize;
    let num_channels = u16::from_le_bytes([data[22], data[23]]);
    let sample_rate = u32::from_le_bytes([data[24], data[25], data[26], data[27]]);
    let bits_per_sample = u16::from_le_bytes([data[34], data[35]]);

    // Find data chunk — scan from end of fmt chunk (correctly handles extended fmt)
    let mut data_start = 20 + fmt_size;
    while data_start + 8 <= data.len() {
        let chunk_id = &data[data_start..data_start + 4];
        let chunk_size = u32::from_le_bytes([
            data[data_start + 4], data[data_start + 5], data[data_start + 6], data[data_start + 7],
        ]) as usize;
        if chunk_id == b"data" {
            break;
        }
        data_start += 8 + chunk_size + chunk_size % 2; // pad to even
    }

    if data_start + 8 > data.len() {
        return Err(Error::Parse("No data chunk found in WAV".into()));
    }

    let data_size = u32::from_le_bytes([
        data[data_start + 4],
        data[data_start + 5],
        data[data_start + 6],
        data[data_start + 7],
    ]) as usize;
    let data_offset = data_start + 8;
    let data_end = (data_offset + data_size).min(data.len());

    let raw = &data[data_offset..data_end];
    let samples_per_channel = raw.len() / (num_channels as usize * (bits_per_sample as usize / 8));

    let mut out = Vec::with_capacity(samples_per_channel);

    match bits_per_sample {
        16 => {
            for chunk in raw.chunks(num_channels as usize * 2) {
                if chunk.len() < 2 {
                    break;
                }
                let sum: f32 = chunk
                    .chunks(2)
                    .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / i16::MAX as f32)
                    .sum::<f32>();
                out.push(sum / num_channels as f32);
            }
        }
        24 => {
            for chunk in raw.chunks(num_channels as usize * 3) {
                if chunk.len() < 3 {
                    break;
                }
                let sum: f32 = chunk
                    .chunks(3)
                    .map(|c| {
                        let sample = i32::from_le_bytes([c[0], c[1], c[2], if c[2] & 0x80 != 0 { 0xFF } else { 0x00 }]);
                        sample as f32 / 8_388_607.0
                    })
                    .sum::<f32>();
                out.push(sum / num_channels as f32);
            }
        }
        8 => {
            for chunk in raw.chunks(num_channels as usize) {
                if chunk.is_empty() {
                    break;
                }
                let sum: f32 = chunk
                    .iter()
                    .map(|&c| (c as f32 - 128.0) / 128.0)
                    .sum::<f32>();
                out.push(sum / num_channels as f32);
            }
        }
        32 => {
            for chunk in raw.chunks(num_channels as usize * 4) {
                if chunk.len() < 4 {
                    break;
                }
                let sum: f32 = chunk
                    .chunks(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .sum::<f32>();
                out.push(sum / num_channels as f32);
            }
        }
        _ => return Err(Error::Parse(format!("Unsupported bit depth: {bits_per_sample}"))),
    }

    Ok((out, sample_rate))
}

/// Simple linear resampling.
fn resample_vec(data: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate {
        return data.to_vec();
    }
    let ratio = from_rate as f64 / to_rate as f64;
    let out_len = (data.len() as f64 / ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f64 * ratio;
        let idx = src as usize;
        let frac = src - idx as f64;
        let a = data.get(idx).copied().unwrap_or(0.0);
        let b = data.get(idx + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac as f32);
    }
    out
}

/// Convert sample index between rates.
fn convert_sample_index(idx: u32, old_rate: u32, new_rate: u32) -> u32 {
    crate::types::convert_sample_index(idx, old_rate, new_rate)
}
