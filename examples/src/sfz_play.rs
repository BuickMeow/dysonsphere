use dysonphere_core::{
    AudioPipe, AudioStreamParams, ChannelCount,
    event::{ChannelAudioEvent, ChannelConfigEvent, ChannelEvent, ControlEvent, SynthEvent},
    soundfont::{SoundFontWrapper, SoundfontBase},
    synth::Synthesizer,
};
use std::sync::Arc;
use hound::{WavSpec, WavWriter};

fn main() {
    let sfz_path = &std::env::args().nth(1)
        .unwrap_or_else(|| "/Users/jieneng/Documents/Soundfonts/Starry Studio Grand 2.5/Presets/D_Simplicity/Studio Grand - Simplicity (2 Sec Release).sfz".into());
    let out_path = std::env::args().nth(2).unwrap_or_else(|| "sfz_test.wav".into());

    let sample_rate = 44100u32;
    let stream_params = AudioStreamParams { sample_rate, channels: ChannelCount::Stereo };

    let soundfont = dysonphere_soundfont::sfz::load(sfz_path, sample_rate)
        .expect("Failed to load SFZ");

    let wrapper = SoundFontWrapper::new(
        soundfont,
        stream_params,
        sfz_path.to_string(),
    );

    let mut synth = Synthesizer::new(stream_params);

    let sf_base: Arc<dyn SoundfontBase> = Arc::new(wrapper);
    synth.send_event(SynthEvent::Channel(0, ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(vec![sf_base]))));
    synth.send_event(SynthEvent::Channel(0, ChannelEvent::Audio(ChannelAudioEvent::Control(ControlEvent::Raw(0, 0)))));
    synth.send_event(SynthEvent::Channel(0, ChannelEvent::Audio(ChannelAudioEvent::ProgramChange(0))));

    // Play C-E-G chord
    let notes = [60u8, 64u8, 67u8];
    for &note in &notes {
        synth.send_event(SynthEvent::Channel(0, ChannelEvent::Audio(ChannelAudioEvent::NoteOn { key: note, vel: 100 })));
    }

    let mut buffer = vec![0.0f32; (3.0 * sample_rate as f32) as usize * 2];
    synth.read_samples(&mut buffer[..(1.0 * sample_rate as f32) as usize * 2]);

    // Note off
    for &note in &notes {
        synth.send_event(SynthEvent::Channel(0, ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key: note })));
    }

    // Release tail
    synth.read_samples(&mut buffer[(1.0 * sample_rate as f32) as usize * 2..]);

    let max_val = buffer.iter().fold(0.0f32, |acc, &s| acc.max(s.abs()));
    let non_zero = buffer.iter().filter(|&&s| s.abs() > 0.0001).count();
    eprintln!("Max: {max_val}, Non-zero: {non_zero}/{}", buffer.len());

    let spec = WavSpec { channels: 2, sample_rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut writer = WavWriter::create(&out_path, spec).unwrap();
    for &sample in &buffer {
        writer.write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).unwrap();
    }
    writer.finalize().unwrap();
    eprintln!("Wrote {out_path}");
}
