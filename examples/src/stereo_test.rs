use dysonphere_core::event::{ChannelAudioEvent, ChannelConfigEvent, ChannelEvent, ControlEvent, SynthEvent};
use dysonphere_core::pipe::{AudioPipe, AudioStreamParams, ChannelCount};
use dysonphere_core::soundfont::{SoundFontWrapper, SoundfontBase};
use dysonphere_core::Synthesizer;
use hound::{WavSpec, WavWriter};
use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    let sf_path = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "/Users/jieneng/Documents/Soundfonts/GeneralUser-GS.sf2".into()),
    );
    let out_path = PathBuf::from(std::env::args().nth(2).unwrap_or_else(|| "stereo_test.wav".into()));

    let sample_rate = 44100u32;
    let stream_params = AudioStreamParams {
        sample_rate,
        channels: ChannelCount::Stereo,
    };

    eprintln!("Loading soundfont: {}", sf_path.display());

    let soundfont = match dysonphere_soundfont::sf2::load(&sf_path, sample_rate) {
        Ok(sf) => sf,
        Err(e) => {
            eprintln!("Failed to load soundfont: {e}");
            std::process::exit(1);
        }
    };

    let wrapper = SoundFontWrapper::new(soundfont, stream_params, sf_path.to_string_lossy().to_string());
    eprintln!("Loaded {} presets", wrapper.presets().len());

    let mut synth = Synthesizer::new(stream_params);

    // 1. SetSoundfonts (like taiyang::initialize)
    let sf_base: Arc<dyn dysonphere_core::soundfont::SoundfontBase> = Arc::new(wrapper);
    synth.send_event(SynthEvent::Channel(
        0,
        ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(vec![sf_base])),
    ));

    // 2. SetPercussionMode(false)
    synth.send_event(SynthEvent::Channel(
        0,
        ChannelEvent::Config(ChannelConfigEvent::SetPercussionMode(false)),
    ));

    // 3. Send preset bank=0, program=0 (like taiyang)
    synth.send_event(SynthEvent::Channel(
        0,
        ChannelEvent::Audio(ChannelAudioEvent::Control(ControlEvent::Raw(0, 0))),
    ));
    synth.send_event(SynthEvent::Channel(
        0,
        ChannelEvent::Audio(ChannelAudioEvent::ProgramChange(0)),
    ));

    // 4. NoteOn (like MIDI event)
    synth.send_event(SynthEvent::Channel(
        0,
        ChannelEvent::Audio(ChannelAudioEvent::NoteOn { key: 60, vel: 100 }),
    ));

    // 5. Render 2 seconds of stereo audio
    let total_frames = (2.0 * sample_rate as f32) as usize;
    let mut buffer = vec![0.0f32; total_frames * 2];
    synth.read_samples(&mut buffer);

    eprintln!("Voice count after render: {}", synth.voice_count());

    // Check if buffer has non-zero samples
    let max_val = buffer.iter().fold(0.0f32, |acc, &s| acc.max(s.abs()));
    let non_zero = buffer.iter().filter(|&&s| s.abs() > 0.0001).count();
    eprintln!("Max sample: {max_val}, Non-zero samples: {non_zero} / {}", buffer.len());

    // 6. NoteOff
    synth.send_event(SynthEvent::Channel(
        0,
        ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: 60 }),
    ));

    // Render release tail
    let mut tail_buffer = vec![0.0f32; sample_rate as usize * 2];
    synth.read_samples(&mut tail_buffer);

    // Write WAV
    let spec = WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = WavWriter::create(&out_path, spec).expect("Failed to create WAV");
    for &sample in &buffer {
        let clamped = sample.clamp(-1.0, 1.0);
        writer.write_sample((clamped * i16::MAX as f32) as i16).unwrap();
    }
    for &sample in &tail_buffer {
        let clamped = sample.clamp(-1.0, 1.0);
        writer.write_sample((clamped * i16::MAX as f32) as i16).unwrap();
    }
    writer.finalize().unwrap();
    eprintln!("Wrote {} frames to {}", total_frames + sample_rate as usize, out_path.display());
}
