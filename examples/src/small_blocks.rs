use dysonphere_core::{
    AudioPipe, AudioStreamParams, ChannelCount,
    event::{ChannelAudioEvent, ChannelEvent, ChannelConfigEvent, ControlEvent, SynthEvent},
    soundfont::{SampleSoundfont, SoundfontBase},
    synth::Synthesizer,
};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;

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
}

impl SynthEngine {
    pub fn new(sample_rate: f32) -> Self {
        let audio_params = AudioStreamParams {
            sample_rate: sample_rate as u32,
            channels: ChannelCount::Stereo,
        };
        let core = Synthesizer::new(audio_params);
        Self { core, sample_rate }
    }

    pub fn send_event(&mut self, event: SynthEvent) {
        self.core.send_event(event);
    }

    pub fn load_soundfonts(&mut self, entries: &[SoundfontEntry]) -> Result<(), String> {
        let mut soundfonts: Vec<Arc<dyn SoundfontBase>> = Vec::new();
        for entry in entries {
            if !entry.enabled { continue; }
            let cache_key = (entry.path.clone(), self.sample_rate as u32);
            let sf = if let Some(sf) = GLOBAL_SF_CACHE.lock().unwrap().get(&cache_key) {
                sf.clone()
            } else {
                match SampleSoundfont::new(&entry.path, self.core.stream_params(),
                ) {
                    Ok(sf) => {
                        let arc: Arc<dyn SoundfontBase> = Arc::new(sf);
                        GLOBAL_SF_CACHE.lock().unwrap().insert(cache_key, arc.clone());
                        arc
                    }
                    Err(e) => {
                        eprintln!("Failed to load {}: {:?}", entry.path, e);
                        continue;
                    }
                }
            };
            soundfonts.push(sf);
        }
        self.core.send_event(SynthEvent::Channel(
            0, ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(soundfonts))
        ));
        Ok(())
    }

    pub fn set_percussion_mode(&mut self, percussion: bool) {
        self.core.send_event(SynthEvent::Channel(
            0, ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(percussion)),
        ));
    }

    pub fn send_preset(&mut self, bank: u8, program: u8) {
        self.core.send_event(SynthEvent::Channel(
            0, ChannelEvent::Audio(ChannelAudioEvent::Control(ControlEvent::Raw(0, bank))),
        ));
        self.core.send_event(SynthEvent::Channel(
            0, ChannelEvent::Audio(ChannelAudioEvent::ProgramChange(program)),
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
    let mut engine = SynthEngine::new(sample_rate);

    engine.load_soundfonts(&[SoundfontEntry {
        path: sf_path.into(), name: "GeneralUser".into(), enabled: true,
    }]).unwrap();
    engine.set_percussion_mode(false);
    engine.send_preset(0, 0);

    // Simulate DAW process blocks: small buffers
    let block_sizes = [64, 128, 256, 512, 64, 128];
    let mut all_samples: Vec<f32> = Vec::new();
    let mut note_on_sent = false;

    for &block_size in &block_sizes {
        if !note_on_sent {
            // Send NoteOn before first block
            engine.send_event(SynthEvent::Channel(
                0, ChannelEvent::Audio(ChannelAudioEvent::NoteOn { key: 60, vel: 100 }),
            ));
            note_on_sent = true;
        }

        let mut block = vec![0.0f32; block_size * 2];
        engine.read_samples(&mut block);
        all_samples.extend_from_slice(&block);
    }

    let max_val = all_samples.iter().fold(0.0f32, |acc, &s| acc.max(s.abs()));
    let non_zero = all_samples.iter().filter(|&&s| s.abs() > 0.0001).count();
    eprintln!("Blocks: {}, Total samples: {}, Max: {}, Non-zero: {}/{}",
        block_sizes.len(), all_samples.len(), max_val, non_zero, all_samples.len());

    if max_val < 0.0001 {
        eprintln!("ERROR: Output is silent!");
        std::process::exit(1);
    } else {
        eprintln!("SUCCESS: Output has audio!");
    }
}
