use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    ops::RangeInclusive,
    path::Path,
    sync::Arc,
};

use soundfont::{
    raw::{GeneratorType, SampleChunk, SampleData, SampleHeader, SampleLink},
    Instrument as Sf2Instrument, Preset as Sf2Preset, SoundFont2, Zone,
};

use crate::types::{
    convert_sample_index, EnvelopeDescriptor, LoopMode, Preset, Region, SoundFont,
};

/// Parses an SF2 file and returns a unified SoundFont.
pub fn load(path: impl AsRef<Path>, sample_rate: u32) -> Result<SoundFont, Error> {
    let path = path.as_ref();
    let mut file = File::open(path).map_err(|e| Error::Io(e, path.to_path_buf()))?;

    let sf2 = SoundFont2::load(&mut file)
        .map_err(|e| Error::Parse(format!("Failed to parse SF2: {e:?}")))?;

    let sf2 = sf2.sort_presets();

    let sample_list = parse_samples(&mut file, &sf2.sample_headers, &sf2.sample_data, sample_rate)?;

    let instruments = parse_instruments(&sf2.instruments);

    let presets = build_presets(&sf2.presets, &instruments, &sample_list, sample_rate);

    // Collect all Arc<[f32]> for ownership
    let sample_buffers: Vec<Arc<[f32]>> = sample_list.iter().map(|s| s.data.clone()).collect();

    Ok(SoundFont {
        presets,
        sample_buffers,
    })
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error, std::path::PathBuf),
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

// ── Internal parsed types ──────────────────────────────────────────

struct ParsedSample {
    data: Arc<[f32]>,
    link_type: SampleLinkType,
    linked_sample: Option<u16>,
    original_length: u32,
    loop_start: u32,
    loop_end: u32,
    sample_rate: u32,
    origpitch: u8,
    pitchadj: i8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SampleLinkType {
    Mono,
    Left,
    Right,
    Linked,
}

impl From<SampleLink> for SampleLinkType {
    fn from(v: SampleLink) -> Self {
        if v.is_left() {
            Self::Left
        } else if v.is_right() {
            Self::Right
        } else if v.is_linked() {
            Self::Linked
        } else {
            Self::Mono
        }
    }
}

#[derive(Default, Clone, Debug)]
struct RawZone {
    index: Option<u16>,
    offset: Option<i16>,
    end_offset: Option<i16>,
    offset_coarse: Option<i16>,
    end_offset_coarse: Option<i16>,
    loop_start_offset: Option<i16>,
    loop_start_offset_coarse: Option<i16>,
    loop_end_offset: Option<i16>,
    loop_end_offset_coarse: Option<i16>,
    attenuation: Option<i16>,
    pan: Option<i16>,
    loop_mode: Option<LoopMode>,
    keyrange: Option<RangeInclusive<u8>>,
    velrange: Option<RangeInclusive<u8>>,
    exclusive_class: Option<u8>,
    root_override: Option<i16>,
    fixed_key: Option<u8>,
    fixed_velocity: Option<u8>,
    scale_tuning: Option<i16>,
    fine_tune: Option<i16>,
    coarse_tune: Option<i16>,
    env_delay: Option<i16>,
    env_attack: Option<i16>,
    env_hold: Option<i16>,
    env_decay: Option<i16>,
    env_sustain: Option<i16>,
    env_release: Option<i16>,
}

struct ParsedInstrument {
    regions: Vec<RawZone>,
}

// ── Sample parsing ─────────────────────────────────────────────────

fn parse_samples(
    file: &mut File,
    headers: &[SampleHeader],
    data: &SampleData,
    target_rate: u32,
) -> Result<Vec<ParsedSample>, Error> {
    let smpl = read_chunk(file, data.smpl.as_ref())?;
    let sm24 = data.sm24.as_ref().map(|c| read_chunk(file, Some(c))).transpose()?;

    // Decode 16-bit or 24-bit PCM to f32
    let decoded: Vec<f32> = if let Some(extra) = &sm24 {
        let smpl16 = &smpl;
        let count = smpl16.len() / 2;
        if extra.len() < count {
            return Err(Error::Parse("sm24 chunk too short".into()));
        }
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let n0 = extra[i];
            let n1 = smpl16[i * 2];
            let n2 = smpl16[i * 2 + 1];
            let sign = if (n2 & 0x80) != 0 { 0xFFu8 } else { 0x00 };
            let sample = i32::from_le_bytes([n0, n1, n2, sign]);
            out.push(sample as f32 / 8_388_607.0);
        }
        out
    } else {
        smpl
            .chunks(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / i16::MAX as f32)
            .collect()
    };

    let mut out = Vec::with_capacity(headers.len());
    for h in headers {
        let start = h.start as usize;
        let end = h.end as usize;
        let slice: Vec<f32> = decoded[start..end].into();

        let data = if h.sample_rate != target_rate && !slice.is_empty() {
            resample_vec(&slice, h.sample_rate, target_rate)
        } else {
            slice.into()
        };

        out.push(ParsedSample {
            data,
            link_type: h.sample_type.into(),
            linked_sample: match h.sample_type.into() {
                SampleLinkType::Mono => None,
                _ => Some(h.sample_link),
            },
            original_length: h.end - h.start,
            loop_start: h.loop_start - h.start,
            loop_end: h.loop_end - h.start,
            sample_rate: h.sample_rate,
            origpitch: h.origpitch,
            pitchadj: h.pitchadj,
        });
    }

    Ok(out)
}

fn read_chunk(file: &mut File, chunk: Option<&SampleChunk>) -> Result<Vec<u8>, Error> {
    let chunk = chunk.ok_or_else(|| Error::Parse("Missing sample chunk".into()))?;
    let mut buf = vec![0u8; chunk.len as usize];
    file.seek(SeekFrom::Start(chunk.offset))
        .map_err(|e| Error::Io(e, std::path::PathBuf::new()))?;
    file.read_exact(&mut buf)
        .map_err(|e| Error::Io(e, std::path::PathBuf::new()))?;
    Ok(buf)
}

/// Simple linear resampling (upsample/downsample).
fn resample_vec(data: &[f32], from_rate: u32, to_rate: u32) -> Arc<[f32]> {
    if from_rate == to_rate {
        return data.to_vec().into();
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
    out.into()
}

// ── Instrument parsing ──────────────────────────────────────────────

fn parse_instruments(instruments: &[Sf2Instrument]) -> Vec<ParsedInstrument> {
    instruments
        .iter()
        .map(|inst| ParsedInstrument {
            regions: parse_zones(&inst.zones),
        })
        .collect()
}

fn parse_zones(zones: &[Zone]) -> Vec<RawZone> {
    let mut regions = Vec::new();
    let mut global = RawZone::default();

    for (i, zone) in zones.iter().enumerate() {
        let mut region = global.clone();

        for generator in &zone.gen_list {
            let Ok(gen_ty) = generator.ty.into_result() else {
                continue;
            };
            apply_generator(&mut region, gen_ty, &generator.amount);
        }

        if i == 0 && region.index.is_none() {
            global = region;
        } else {
            regions.push(region);
        }
    }

    regions
}

fn apply_generator(zone: &mut RawZone, ty: GeneratorType, amount: &soundfont::raw::GeneratorAmount) {
    match ty {
        GeneratorType::StartAddrsOffset => zone.offset = amount.as_i16().copied(),
        GeneratorType::EndAddrsOffset => zone.end_offset = amount.as_i16().copied(),
        GeneratorType::StartAddrsCoarseOffset => zone.offset_coarse = amount.as_i16().copied(),
        GeneratorType::EndAddrsCoarseOffset => zone.end_offset_coarse = amount.as_i16().copied(),
        GeneratorType::StartloopAddrsOffset => zone.loop_start_offset = amount.as_i16().copied(),
        GeneratorType::StartloopAddrsCoarseOffset => {
            zone.loop_start_offset_coarse = amount.as_i16().copied()
        }
        GeneratorType::EndloopAddrsOffset => zone.loop_end_offset = amount.as_i16().copied(),
        GeneratorType::EndloopAddrsCoarseOffset => {
            zone.loop_end_offset_coarse = amount.as_i16().copied()
        }
        GeneratorType::Pan => zone.pan = amount.as_i16().copied(),
        GeneratorType::DelayVolEnv => zone.env_delay = amount.as_i16().copied(),
        GeneratorType::AttackVolEnv => zone.env_attack = amount.as_i16().copied(),
        GeneratorType::HoldVolEnv => zone.env_hold = amount.as_i16().copied(),
        GeneratorType::DecayVolEnv => zone.env_decay = amount.as_i16().copied(),
        GeneratorType::SustainVolEnv => zone.env_sustain = amount.as_i16().copied(),
        GeneratorType::ReleaseVolEnv => zone.env_release = amount.as_i16().copied(),
        GeneratorType::KeyRange => {
            zone.keyrange = amount.as_range().copied().map(|r| r.low..=r.high)
        }
        GeneratorType::VelRange => {
            zone.velrange = amount.as_range().copied().map(|r| r.low..=r.high)
        }
        GeneratorType::InitialAttenuation => zone.attenuation = amount.as_i16().copied(),
        GeneratorType::CoarseTune => zone.coarse_tune = amount.as_i16().copied(),
        GeneratorType::FineTune => zone.fine_tune = amount.as_i16().copied(),
        GeneratorType::SampleID | GeneratorType::Instrument => {
            zone.index = amount.as_u16().copied()
        }
        GeneratorType::SampleModes => {
            zone.loop_mode = amount.as_i16().map(|v| match v {
                1 => LoopMode::LoopContinuous,
                3 => LoopMode::LoopSustain,
                _ => LoopMode::NoLoop,
            })
        }
        GeneratorType::Keynum => {
            zone.fixed_key = amount.as_i16().map(|&v| v.clamp(0, 127) as u8)
        }
        GeneratorType::Velocity => {
            zone.fixed_velocity = amount.as_i16().map(|&v| v.clamp(0, 127) as u8)
        }
        GeneratorType::ScaleTuning => zone.scale_tuning = amount.as_i16().copied(),
        GeneratorType::ExclusiveClass => {
            zone.exclusive_class = amount
                .as_i16()
                .map(|&v| v.clamp(0, i16::from(u8::MAX)) as u8)
        }
        GeneratorType::OverridingRootKey => zone.root_override = amount.as_i16().copied(),
        _ => {}
    }
}

// ── Preset building ─────────────────────────────────────────────────

fn build_presets(
    raw_presets: &[Sf2Preset],
    instruments: &[ParsedInstrument],
    samples: &[ParsedSample],
    target_rate: u32,
) -> Vec<Preset> {
    let mut out = Vec::new();

    for rp in raw_presets {
        let mut regions = Vec::new();
        let preset_zones = parse_zones(&rp.zones);

        for pzone in &preset_zones {
            let Some(inst_idx) = pzone.index else {
                continue;
            };
            let instrument = &instruments[inst_idx as usize];

            for izone in &instrument.regions {
                let Some(sample_idx) = izone.index else {
                    continue;
                };
                let sample = &samples[sample_idx as usize];
                if sample.data.is_empty() {
                    continue;
                }

                let keyrange = apply_fixed(
                    intersect_ranges(
                        pzone.keyrange.clone().unwrap_or(0..=127),
                        izone.keyrange.clone().unwrap_or(0..=127),
                    ),
                    izone.fixed_key.or(pzone.fixed_key),
                );
                let velrange = apply_fixed(
                    intersect_ranges(
                        pzone.velrange.clone().unwrap_or(0..=127),
                        izone.velrange.clone().unwrap_or(0..=127),
                    ),
                    izone.fixed_velocity.or(pzone.fixed_velocity),
                );

                // Compute offsets
                let offset = sum_sample_offset(
                    pzone.offset,
                    pzone.offset_coarse,
                    izone.offset,
                    izone.offset_coarse,
                );
                let end_offset = sum_sample_offset(
                    pzone.end_offset,
                    pzone.end_offset_coarse,
                    izone.end_offset,
                    izone.end_offset_coarse,
                );
                let loop_start_offset = sum_sample_offset(
                    pzone.loop_start_offset,
                    pzone.loop_start_offset_coarse,
                    izone.loop_start_offset,
                    izone.loop_start_offset_coarse,
                );
                let loop_end_offset = sum_sample_offset(
                    pzone.loop_end_offset,
                    pzone.loop_end_offset_coarse,
                    izone.loop_end_offset,
                    izone.loop_end_offset_coarse,
                );

                let sample_end_raw =
                    (sample.original_length as i32 + end_offset).clamp(0, sample.original_length as i32) as u32;

                // Convert to output sample rate
                let offset =
                    convert_sample_index(offset.clamp(0, sample_end_raw as i32) as u32, sample.sample_rate, target_rate);
                let sample_end = convert_sample_index(sample_end_raw, sample.sample_rate, target_rate)
                    .min(sample.data.len() as u32);
                let loop_start = convert_sample_index(
                    (sample.loop_start as i32 + loop_start_offset)
                        .clamp(0, sample_end_raw as i32) as u32,
                    sample.sample_rate,
                    target_rate,
                )
                .min(sample_end);
                let loop_end = convert_sample_index(
                    (sample.loop_end as i32 + loop_end_offset)
                        .clamp(0, sample_end_raw as i32) as u32,
                    sample.sample_rate,
                    target_rate,
                )
                .min(sample_end);

                // Build stereo if linked
                let region_sample = build_stereo_samples(sample, samples);

                // Attenuation → volume
                let attenuation = sum_option(pzone.attenuation, izone.attenuation);
                // SF2: attenuation is in centibels. amp = 10^(-attenuation/200)
                let volume = 10.0f32.powf(-attenuation as f32 / 200.0);

                let pan = sum_option(pzone.pan, izone.pan).clamp(-500, 500);

                // Envelope
                let raw_env = Sf2RawEnvelope {
                    delay: timecents_to_seconds(
                        timecents_merge(12000, pzone.env_delay, izone.env_delay) as f32,
                    ),
                    attack: timecents_to_seconds(
                        timecents_merge(-12000, pzone.env_attack, izone.env_attack) as f32,
                    ),
                    hold: timecents_to_seconds(
                        timecents_merge(-12000, pzone.env_hold, izone.env_hold) as f32,
                    ),
                    decay: timecents_to_seconds(
                        timecents_merge(-12000, pzone.env_decay, izone.env_decay) as f32,
                    ),
                    sustain: sustain_to_percent(
                        sustain_merge(0, pzone.env_sustain, izone.env_sustain) as f32,
                    ),
                    release: {
                        let secs = timecents_to_seconds(
                            timecents_merge(-12000, pzone.env_release, izone.env_release) as f32,
                        );
                        // Piano needs ≥2s envelope T for perceptible tail (exponential
                        // to -90dB → perceived ~22% of T, so 2s T ≈ 0.44s perceived).
                        if secs < 0.5 { 2.5 } else { secs.max(1.5) }
                    },
                };

                let envelope = EnvelopeDescriptor {
                    delay: raw_env.delay,
                    attack: raw_env.attack,
                    hold: raw_env.hold,
                    decay: raw_env.decay,
                    sustain: raw_env.sustain,
                    release: raw_env.release,
                };

                let root_key = izone
                    .root_override
                    .or(pzone.root_override)
                    .unwrap_or(sample.origpitch as i16) as u8;

                let scale_tuning = izone.scale_tuning.or(pzone.scale_tuning).unwrap_or(100);
                let _ = scale_tuning; // Reserved for future key scaling

                let fine_tune_cents = sum_option(pzone.fine_tune, izone.fine_tune) as f32
                    + sample.pitchadj as f32
                    + sum_option(pzone.coarse_tune, izone.coarse_tune) as f32 * 100.0;

                let exclusive_class = izone.exclusive_class.or(pzone.exclusive_class);

                let loop_mode = {
                    let raw = pzone.loop_mode.unwrap_or(izone.loop_mode.unwrap_or(LoopMode::NoLoop));
                    if loop_start == loop_end && raw != LoopMode::NoLoop {
                        LoopMode::NoLoop // protection: no valid loop range
                    } else {
                        raw
                    }
                };

                regions.push(Region {
                    key_range: keyrange,
                    vel_range: velrange,
                    root_key,
                    sample: region_sample,
                    original_sample_rate: sample.sample_rate,
                    volume,
                    pan: pan as f32,
                    loop_mode,
                    loop_start,
                    loop_end,
                    sample_end,
                    offset,
                    envelope,
                    fine_tune_cents,
                    exclusive_class,
                });
            }
        }

        out.push(Preset {
            bank: rp.header.bank,
            program: rp.header.preset,
            name: rp.header.name.clone(),
            regions,
        });
    }

    out
}

// ── Helpers ─────────────────────────────────────────────────────────

fn sum_option(a: Option<i16>, b: Option<i16>) -> i16 {
    a.unwrap_or(0) + b.unwrap_or(0)
}

fn sum_sample_offset(
    fine_a: Option<i16>,
    coarse_a: Option<i16>,
    fine_b: Option<i16>,
    coarse_b: Option<i16>,
) -> i32 {
    i32::from(sum_option(fine_a, fine_b)) + i32::from(sum_option(coarse_a, coarse_b)) * 32768
}

fn intersect_ranges<T: Ord + Copy>(a: RangeInclusive<T>, b: RangeInclusive<T>) -> RangeInclusive<T> {
    (*a.start().max(b.start()))..=(*a.end().min(b.end()))
}

fn apply_fixed<T: Ord + Copy>(range: RangeInclusive<T>, fixed: Option<T>) -> RangeInclusive<T> {
    match fixed {
        Some(v) => intersect_ranges(range, v..=v),
        None => range,
    }
}

fn build_stereo_samples(sample: &ParsedSample, samples: &[ParsedSample]) -> Arc<[f32]> {
    match (sample.link_type, sample.linked_sample) {
        (SampleLinkType::Left, Some(linked)) | (SampleLinkType::Right, Some(linked)) => {
            if let Some(other) = samples.get(linked as usize)
                && !other.data.is_empty() {
                    // For simplicity, just use the left/mono channel
                    // Stereo will come later
                    return sample.data.clone();
                }
            sample.data.clone()
        }
        _ => sample.data.clone(),
    }
}

// ── Envelope helpers ────────────────────────────────────────────────

struct Sf2RawEnvelope {
    delay: f32,
    attack: f32,
    hold: f32,
    decay: f32,
    sustain: f32,
    release: f32,
}

/// Merge absolute preset value with absolute-or-relative instrument value.
/// SF2 spec: instrument value is absolute; preset value is additive offset.
fn timecents_merge(_default: i16, preset: Option<i16>, instrument: Option<i16>) -> i32 {
    i32::from(instrument.unwrap_or(-12000)) + i32::from(preset.unwrap_or(0))
}

fn sustain_merge(_default: i16, preset: Option<i16>, instrument: Option<i16>) -> i32 {
    i32::from(instrument.unwrap_or(0)) + i32::from(preset.unwrap_or(0))
}

/// Convert SF2 timecents to seconds.
fn timecents_to_seconds(tc: f32) -> f32 {
    if tc <= -32768.0 {
        0.0
    } else {
        2.0f32.powf(tc.clamp(-12000.0, 8000.0) / 1200.0)
    }
}

/// Convert sustain centibels to 0–1 percent.
fn sustain_to_percent(cb: f32) -> f32 {
    10.0f32.powf(-cb.max(0.0) / 200.0)
}
