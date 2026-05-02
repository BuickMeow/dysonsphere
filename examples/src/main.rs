use dysonphere_core::event::{ChannelConfigEvent, ChannelEvent, SynthEvent};
use dysonphere_core::pipe::{AudioPipe, AudioStreamParams, ChannelCount};
use dysonphere_core::soundfont::{SoundFontWrapper, SoundfontBase};
use dysonphere_core::Synthesizer;
use hound::{WavSpec, WavWriter};
use std::path::PathBuf;
use std::sync::Arc;

fn load_soundfont(path: &PathBuf, sample_rate: u32) -> Result<dysonphere_soundfont::SoundFont, String> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("sf2") => dysonphere_soundfont::sf2::load(path, sample_rate).map_err(|e| e.to_string()),
        Some("sfz") => dysonphere_soundfont::sfz::load(path, sample_rate).map_err(|e| e.to_string()),
        _ => Err("Unsupported format. Use .sf2 or .sfz files.".into()),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <soundfont.sf2|soundfont.sfz> [output.wav]", args[0]);
        std::process::exit(1);
    }

    let sf_path = PathBuf::from(&args[1]);
    let out_path = if args.len() >= 3 {
        PathBuf::from(&args[2])
    } else {
        PathBuf::from("output.wav")
    };

    let sample_rate = 44100;
    let stream_params = AudioStreamParams {
        sample_rate,
        channels: ChannelCount::Mono,
    };

    eprintln!("Loading soundfont: {}", sf_path.display());

    let soundfont = match load_soundfont(&sf_path, sample_rate) {
        Ok(sf) => sf,
        Err(e) => {
            eprintln!("Failed to load soundfont: {e}");
            std::process::exit(1);
        }
    };

    // Wrap in SoundfontBase for the event-based API
    let wrapper = SoundFontWrapper::new(
        soundfont,
        stream_params,
        sf_path.to_string_lossy().to_string(),
    );

    eprintln!("Loaded {} presets", wrapper.presets().len());
    for p in wrapper.presets() {
        eprintln!("  Bank {} Program {}: {}", p.bank, p.program, p.name);
    }

    let mut synth = Synthesizer::new(stream_params);

    // Send SetSoundfonts config event
    let sf_base: Arc<dyn dysonphere_core::soundfont::SoundfontBase> = Arc::new(wrapper);
    synth.send_event(SynthEvent::Channel(
        0,
        ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(vec![sf_base])),
    ));

    // Play a C major scale using the event-based API
    let notes = [60, 62, 64, 65, 67, 69, 71, 72];
    let seconds_per_note = 0.5;
    let note_samples = (seconds_per_note * sample_rate as f32) as usize;
    let total_samples = note_samples * notes.len() + sample_rate as usize;

    let mut buffer = vec![0.0f32; total_samples];

    for (i, &note) in notes.iter().enumerate() {
        synth.note_on(note, 100);
        let offset = i * note_samples;
        synth.read_samples(&mut buffer[offset..offset + note_samples]);
        synth.note_off(note);
    }

    // Let the last note ring out
    let offset = notes.len() * note_samples;
    synth.read_samples(&mut buffer[offset..]);

    // Normalize
    let max = buffer.iter().fold(0.0f32, |acc, &s| acc.max(s.abs()));
    if max > 0.0 {
        let gain = 0.9 / max;
        for s in &mut buffer {
            *s *= gain;
        }
    }

    // Write WAV
    let spec = WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = WavWriter::create(&out_path, spec).expect("Failed to create WAV file");
    for &sample in &buffer {
        let clamped = sample.clamp(-1.0, 1.0);
        writer
            .write_sample((clamped * i16::MAX as f32) as i16)
            .expect("Failed to write sample");
    }
    writer.finalize().expect("Failed to finalize WAV");

    eprintln!("Wrote {} samples to {}", buffer.len(), out_path.display());
    eprintln!("Done! ✨");
}
