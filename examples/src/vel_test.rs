use dysonphere_core::{
    AudioPipe, AudioStreamParams, ChannelCount,
    event::{ChannelAudioEvent, ChannelConfigEvent, ChannelEvent, ControlEvent, SynthEvent},
    soundfont::{SampleSoundfont, SoundfontBase},
    synth::Synthesizer,
};
use std::sync::Arc;

fn main() {
    let sf_path = "/Users/jieneng/Documents/Soundfonts/GeneralUser-GS.sf2";
    let sample_rate = 44100u32;
    let stream_params = AudioStreamParams { sample_rate, channels: ChannelCount::Mono };

    let mut synth = Synthesizer::new(stream_params);

    let sf = SampleSoundfont::new(sf_path, stream_params).expect("load SF2");
    let sf: Arc<dyn SoundfontBase> = Arc::new(sf);
    synth.send_event(SynthEvent::Channel(0, ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(vec![sf]))));
    synth.send_event(SynthEvent::Channel(0, ChannelEvent::Audio(ChannelAudioEvent::Control(ControlEvent::Raw(0, 0)))));
    synth.send_event(SynthEvent::Channel(0, ChannelEvent::Audio(ChannelAudioEvent::ProgramChange(0))));
    // Flush init events
    let mut dummy = vec![0.0f32; 100];
    synth.read_samples(&mut dummy);

    let velocities = [127u8, 100, 64, 32, 10];
    for &vel in &velocities {
        synth.send_event(SynthEvent::Channel(0, ChannelEvent::Audio(ChannelAudioEvent::NoteOn { key: 60, vel })));
        let mut buf = vec![0.0f32; 4410]; // 100ms
        synth.read_samples(&mut buf);
        let peak = buf.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        eprintln!("vel={vel:3} peak={peak:.6}");

        // Clean up
        synth.send_event(SynthEvent::Channel(0, ChannelEvent::Audio(ChannelAudioEvent::AllNotesKilled)));
        synth.read_samples(&mut dummy);
    }
}
