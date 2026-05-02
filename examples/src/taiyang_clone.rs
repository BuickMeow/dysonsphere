use dysonphere_core::{
    AudioPipe, AudioStreamParams, ChannelCount,
    event::{ChannelAudioEvent, ChannelEvent, ChannelConfigEvent, ControlEvent, SynthEvent},
    soundfont::{SampleSoundfont, SoundfontBase},
    synth::Synthesizer,
};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct PresetInfo {
    pub name: String,
    pub bank: u16,
    pub program: u16,
    pub source_file: String,
}

#[derive(Clone, Debug, Default)]
pub struct SoundfontEntry {
    pub path: String,
    pub name: String,
    pub enabled: bool,
}

lazy_static::lazy_static! {
    static ref GLOBAL_SF_CACHE: Mutex<HashMap<(String, u32), Arc<dyn SoundfontBase>>> =
        Mutex::new(HashMap::new());
}

pub struct SynthEngine {
    core: Synthesizer,
    sample_rate: f32,
    presets: Vec<PresetInfo>,
}

impl SynthEngine {
    pub fn new(sample_rate: f32, _max_voices: usize) -> Self {
        let audio_params = AudioStreamParams {
            sample_rate: sample_rate as u32,
            channels: ChannelCount::Stereo,
        };
        let core = Synthesizer::new(audio_params);
        Self {
            core,
            sample_rate,
            presets: Vec::new(),
        }
    }

    pub fn send_event(&mut self, event: SynthEvent) {
        self.core.send_event(event);
    }

    pub fn load_soundfonts(&mut self, entries: &[SoundfontEntry]) -> Result<(), String> {
        let mut soundfonts: Vec<Arc<dyn SoundfontBase>> = Vec::new();
        let mut all_presets: Vec<PresetInfo> = Vec::new();

        for entry in entries {
            if !entry.enabled {
                continue;
            }

            let cache_key = (entry.path.clone(), self.sample_rate as u32);

            let sf = if let Some(sf) = GLOBAL_SF_CACHE.lock().unwrap().get(&cache_key) {
                sf.clone()
            } else {
                match SampleSoundfont::new(
                    &entry.path,
                    self.core.stream_params(),
                ) {
                    Ok(sf) => {
                        let arc: Arc<dyn SoundfontBase> = Arc::new(sf);
                        GLOBAL_SF_CACHE.lock().unwrap().insert(cache_key, arc.clone());
                        eprintln!("Loaded soundfont into global cache: {}", entry.path);
                        arc
                    }
                    Err(e) => {
                        eprintln!("Failed to load {}: {:?}", entry.path, e);
                        continue;
                    }
                }
            };

            soundfonts.push(sf);

            if entry.path.ends_with(".sf2") || entry.path.ends_with(".SF2") {
                if let Ok(sf) = dysonphere_soundfont::sf2::load(&entry.path, self.sample_rate as u32) {
                    for p in sf.presets {
                        all_presets.push(PresetInfo {
                            name: format!("Bank {} Prog {}", p.bank, p.program),
                            bank: p.bank,
                            program: p.program,
                            source_file: entry.name.clone(),
                        });
                    }
                }
            }
        }

        self.core.send_event(SynthEvent::Channel(
            0,
            ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(soundfonts))
        ));

        all_presets.sort_by(|a, b| {
            a.bank.cmp(&b.bank)
                .then_with(|| a.program.cmp(&b.program))
        });

        self.presets = all_presets;
        Ok(())
    }

    pub fn set_percussion_mode(&mut self, percussion: bool) {
        self.core.send_event(SynthEvent::Channel(
            0,
            ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(percussion)),
        ));
    }

    pub fn send_preset(&mut self, bank: u8, program: u8) {
        self.core.send_event(SynthEvent::Channel(
            0,
            ChannelEvent::Audio(ChannelAudioEvent::Control(ControlEvent::Raw(0, bank))),
        ));
        self.core.send_event(SynthEvent::Channel(
            0,
            ChannelEvent::Audio(ChannelAudioEvent::ProgramChange(program)),
        ));
    }

    pub fn read_samples(&mut self, buffer: &mut [f32]) {
        self.core.read_samples(buffer);
    }

    pub fn active_voices(&self) -> u64 {
        self.core.voice_count()
    }
}

fn main() {
    let sf_path = "/Users/jieneng/Documents/Soundfonts/GeneralUser-GS.sf2";
    let sample_rate = 44100.0f32;

    let mut engine = SynthEngine::new(sample_rate, 0);

    let entries = vec![SoundfontEntry {
        path: sf_path.into(),
        name: "GeneralUser".into(),
        enabled: true,
    }];

    engine.load_soundfonts(&entries).unwrap();
    engine.set_percussion_mode(false);
    engine.send_preset(0, 0);

    // Simulate MIDI NoteOn
    engine.send_event(SynthEvent::Channel(
        0,
        ChannelEvent::Audio(ChannelAudioEvent::NoteOn { key: 60, vel: 100 }),
    ));

    // Render 2 seconds
    let mut buffer = vec![0.0f32; (2.0 * sample_rate) as usize * 2];
    engine.read_samples(&mut buffer);

    let max_val = buffer.iter().fold(0.0f32, |acc, &s| acc.max(s.abs()));
    let non_zero = buffer.iter().filter(|&&s| s.abs() > 0.0001).count();
    eprintln!("Voice count: {}, Max: {}, Non-zero: {}/{}", engine.active_voices(), max_val, non_zero, buffer.len());

    if max_val < 0.0001 {
        eprintln!("ERROR: Output is silent!");
        std::process::exit(1);
    } else {
        eprintln!("SUCCESS: Output has audio!");
    }
}
