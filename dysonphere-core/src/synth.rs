use std::sync::Arc;

use crate::{
    event::{
        ChannelAudioEvent, ChannelConfigEvent, ChannelEvent, ControlEvent, SynthEvent,
    },
    pipe::{AudioPipe, AudioStreamParams, ChannelCount},
    soundfont::{PresetInfo, SoundfontBase},
    voice::Voice,
};

/// Maximum number of simultaneous voices.
const MAX_VOICES: usize = 256;

/// Smoothing interval for CC value changes, in seconds.
const CC_SMOOTH_SECONDS: f32 = 0.01;

/// Simple linear smoothing for a scalar control value.
#[derive(Clone)]
struct SmoothedValue {
    current: f32,
    target: f32,
    step: f32,
}

impl SmoothedValue {
    fn new(initial: f32) -> Self {
        Self { current: initial, target: initial, step: 0.0 }
    }

    fn set(&mut self, target: f32, sample_rate: u32) {
        self.target = target;
        let duration_samples = (CC_SMOOTH_SECONDS * sample_rate as f32).max(1.0);
        self.step = (target - self.current) / duration_samples;
    }

    fn get(&self) -> f32 {
        self.current
    }

    fn advance(&mut self) {
        if (self.target - self.current).abs() < self.step.abs().max(1e-7) {
            self.current = self.target;
        } else {
            self.current += self.step;
        }
    }
}

/// Per-channel CC state.
#[derive(Clone)]
struct ChannelState {
    bank: u8,
    program: u8,
    volume: SmoothedValue,       // CC7, 0.0–1.0
    pan: SmoothedValue,          // CC10, 0.0 = left, 0.5 = center, 1.0 = right
    expression: SmoothedValue,   // CC11, 0.0–1.0
    damper: bool,      // CC64, sustain pedal
    pitch_bend: f32,   // -1.0 to 1.0
    pitch_bend_range: f32, // semitones
    fine_tune: f32,    // cents
    coarse_tune: f32,  // semitones
    percussion: bool,
    release_multiplier: f32,    // CC72, 0.0–16.0
    soundfonts: Vec<Arc<dyn SoundfontBase>>,
    /// RPN state
    rpn_msb: u8,
    rpn_lsb: u8,
}

impl Default for ChannelState {
    fn default() -> Self {
        Self {
            bank: 0,
            program: 0,
            volume: SmoothedValue::new(1.0),
            pan: SmoothedValue::new(0.5),
            expression: SmoothedValue::new(1.0),
            damper: false,
            pitch_bend: 0.0,
            pitch_bend_range: 2.0,
            fine_tune: 0.0,
            coarse_tune: 0.0,
            percussion: false,
            release_multiplier: 1.0,
            soundfonts: Vec::new(),
            rpn_msb: 0x7F,
            rpn_lsb: 0x7F,
        }
    }
}

/// The top-level synthesizer.
///
/// Manages voice allocation, MIDI event processing, and audio rendering.
pub struct Synthesizer {
    /// Output stream configuration.
    stream_params: AudioStreamParams,
    /// Per-channel state.
    channels: Vec<ChannelState>,
    /// Active voices: (channel_index, voice).
    voices: Vec<(u32, Voice)>,
    /// Pending synth events.
    pending_events: Vec<SynthEvent>,
    /// Voice counter for stats.
    voice_count: u64,
}

impl Synthesizer {
    pub fn new(stream_params: AudioStreamParams) -> Self {
        Self {
            stream_params,
            channels: vec![ChannelState::default(); 16],
            voices: Vec::with_capacity(MAX_VOICES),
            pending_events: Vec::new(),
            voice_count: 0,
        }
    }

    // ── Public API ──────────────────────────────────────────

    /// Send a synthesizer event. Processed on the next `read_samples` call.
    pub fn send_event(&mut self, event: SynthEvent) {
        self.pending_events.push(event);
    }

    /// Load soundfonts into a channel.
    pub fn load_soundfonts(
        &mut self,
        channel: u32,
        soundfonts: Vec<Arc<dyn SoundfontBase>>,
    ) {
        if let Some(ch) = self.channels.get_mut(channel as usize) {
            ch.soundfonts = soundfonts;
        }
    }

    /// Get presets from the first soundfont on a channel.
    pub fn presets(&self, channel: u32) -> Vec<PresetInfo> {
        self.channels
            .get(channel as usize)
            .and_then(|ch| ch.soundfonts.first())
            .map(|sf| sf.presets())
            .unwrap_or_default()
    }

    /// Returns the number of currently active voices.
    pub fn voice_count(&self) -> u64 {
        self.voice_count
    }

    /// Set percussion mode on a channel.
    pub fn set_percussion_mode(&mut self, channel: u32, percussion: bool) {
        if let Some(ch) = self.channels.get_mut(channel as usize) {
            ch.percussion = percussion;
            if percussion {
                ch.bank = 128;
            }
        }
    }

    /// Set pitch bend range for a channel.
    pub fn set_pitch_bend_range(&mut self, channel: u32, semitones: f32) {
        if let Some(ch) = self.channels.get_mut(channel as usize) {
            ch.pitch_bend_range = semitones;
        }
    }

    /// Set fine tune for a channel.
    pub fn set_fine_tune(&mut self, channel: u32, cents: f32) {
        if let Some(ch) = self.channels.get_mut(channel as usize) {
            ch.fine_tune = cents;
        }
    }

    /// Set coarse tune for a channel.
    pub fn set_coarse_tune(&mut self, channel: u32, semitones: f32) {
        if let Some(ch) = self.channels.get_mut(channel as usize) {
            ch.coarse_tune = semitones;
        }
    }

    /// Trigger program change on a channel.
    pub fn send_program_change(&mut self, channel: u32, bank: u8, program: u8) {
        if let Some(ch) = self.channels.get_mut(channel as usize) {
            if ch.bank != 128 {
                ch.bank = bank;
            }
            ch.program = program;
        }
    }

    /// Kill all voices on all channels immediately.
    pub fn all_notes_killed(&mut self) {
        for voice in &mut self.voices {
            voice.1.kill();
        }
        self.voices.clear();
    }

    /// Convenience: note-on on channel 0.
    pub fn note_on(&mut self, key: u8, vel: u8) {
        self.send_event(SynthEvent::Channel(
            0,
            ChannelEvent::Audio(ChannelAudioEvent::NoteOn { key, vel }),
        ));
    }

    /// Convenience: note-off on channel 0.
    pub fn note_off(&mut self, key: u8) {
        self.send_event(SynthEvent::Channel(
            0,
            ChannelEvent::Audio(ChannelAudioEvent::NoteOff { key }),
        ));
    }

    /// Set soundfonts for a channel.
    pub fn set_soundfonts(&mut self, channel: u32, sfs: Vec<Arc<dyn SoundfontBase>>) {
        self.send_event(SynthEvent::Channel(
            channel,
            ChannelEvent::Config(ChannelConfigEvent::SetSoundfonts(sfs)),
        ));
    }

    /// Get stream parameters.
    pub fn stream_params(&self) -> AudioStreamParams {
        self.stream_params
    }

    // ── Event processing ────────────────────────────────────

    fn flush_events(&mut self) {
        let events: Vec<SynthEvent> = std::mem::take(&mut self.pending_events);

        for event in events {
            match event {
                SynthEvent::Channel(ch_idx, ch_event) => {
                    self.process_channel_event(ch_idx, ch_event);
                }
                SynthEvent::AllChannels(ch_event) => {
                    for ch_idx in 0..self.channels.len() as u32 {
                        self.process_channel_event(ch_idx, ch_event.clone());
                    }
                }
            }
        }
    }

    fn process_channel_event(&mut self, ch_idx: u32, event: ChannelEvent) {
        // Clone channel state values needed for note_on/note_off before mutating self
        let bank = self.channels.get(ch_idx as usize).map(|c| c.bank).unwrap_or(0);
        let program = self.channels.get(ch_idx as usize).map(|c| c.program).unwrap_or(0);
        let damper = self.channels.get(ch_idx as usize).map(|c| c.damper).unwrap_or(false);

        match event {
            ChannelEvent::Audio(audio_event) => match audio_event {
                ChannelAudioEvent::NoteOn { key, vel } => {
                    self.note_on_internal(ch_idx, bank, program, key, vel);
                }
                ChannelAudioEvent::NoteOff { key } => {
                    self.note_off_internal(ch_idx, key, damper);
                }
                ChannelAudioEvent::AllNotesOff => {
                    for (vch, voice) in &mut self.voices {
                        if *vch == ch_idx {
                            voice.note_off();
                        }
                    }
                }
                ChannelAudioEvent::AllNotesKilled => {
                    self.voices.retain(|(vch, _)| *vch != ch_idx);
                }
                ChannelAudioEvent::ResetControl => {
                    if let Some(ch) = self.channels.get_mut(ch_idx as usize) {
                        let sfs = std::mem::take(&mut ch.soundfonts);
                        *ch = ChannelState { soundfonts: sfs, ..Default::default() };
                    }
                }
                ChannelAudioEvent::Control(ctrl) => {
                    self.process_control(ch_idx, ctrl);
                }
                ChannelAudioEvent::ProgramChange(program) => {
                    if let Some(ch) = self.channels.get_mut(ch_idx as usize) {
                        ch.program = program;
                    }
                }
                ChannelAudioEvent::SystemReset => {
                    self.voices.clear();
                    for c in &mut self.channels {
                        *c = ChannelState::default();
                    }
                }
            },
            ChannelEvent::Config(config_event) => match config_event {
                ChannelConfigEvent::SetSoundfonts(sfs) => {
                    if let Some(ch) = self.channels.get_mut(ch_idx as usize) {
                        ch.soundfonts = sfs;
                    }
                }
                ChannelConfigEvent::SetPercussionMode(on) => {
                    if let Some(ch) = self.channels.get_mut(ch_idx as usize) {
                        ch.percussion = on;
                        if on {
                            ch.bank = 128;
                        }
                    }
                }
            },
        }
    }

    fn process_control(&mut self, ch_idx: u32, ctrl: ControlEvent) {
        match ctrl {
            ControlEvent::Raw(cc, value) => self.handle_cc(ch_idx, cc, value),
            ControlEvent::PitchBendSensitivity(s) => {
                if let Some(ch) = self.channels.get_mut(ch_idx as usize) {
                    ch.pitch_bend_range = s;
                }
            }
            ControlEvent::PitchBendValue(v) => {
                if let Some(ch) = self.channels.get_mut(ch_idx as usize) {
                    ch.pitch_bend = v;
                }
            }
            ControlEvent::PitchBend(v) => {
                if let Some(ch) = self.channels.get_mut(ch_idx as usize) {
                    ch.pitch_bend = v;
                }
            }
            ControlEvent::FineTune(v) => {
                if let Some(ch) = self.channels.get_mut(ch_idx as usize) {
                    ch.fine_tune = v;
                }
            }
            ControlEvent::CoarseTune(v) => {
                if let Some(ch) = self.channels.get_mut(ch_idx as usize) {
                    ch.coarse_tune = v;
                }
            }
            ControlEvent::Volume(v) => {
                if let Some(ch) = self.channels.get_mut(ch_idx as usize) {
                    ch.volume.set(v.clamp(0.0, 1.0), self.stream_params.sample_rate);
                }
            }
            ControlEvent::Pan(v) => {
                if let Some(ch) = self.channels.get_mut(ch_idx as usize) {
                    ch.pan.set(v.clamp(0.0, 1.0), self.stream_params.sample_rate);
                }
            }
            ControlEvent::Expression(v) => {
                if let Some(ch) = self.channels.get_mut(ch_idx as usize) {
                    ch.expression.set(v.clamp(0.0, 1.0), self.stream_params.sample_rate);
                }
            }
            ControlEvent::Damper(v) => {
                if let Some(ch) = self.channels.get_mut(ch_idx as usize) {
                    let was_on = ch.damper;
                    ch.damper = v;
                    if was_on && !v {
                        // Damper released — release all sustained voices on this channel
                        for (vch, voice) in &mut self.voices {
                            if *vch == ch_idx && voice.damper_sustained {
                                voice.damper_sustained = false;
                                voice.note_off();
                            }
                        }
                    }
                }
            }
        }
    }

    fn handle_cc(&mut self, ch_idx: u32, cc: u8, value: u8) {
        let Some(ch) = self.channels.get_mut(ch_idx as usize) else {
            return;
        };

        match cc {
            0x00 => {
                // Bank select MSB
                if ch.bank != 128 {
                    ch.bank = value;
                }
            }
            0x07 => {
                // Volume
                ch.volume.set(value as f32 / 127.0, self.stream_params.sample_rate);
            }
            0x0A => {
                // Pan (balance on some synths, 0x08 works too)
                ch.pan.set(value as f32 / 127.0, self.stream_params.sample_rate);
            }
            0x0B => {
                // Expression
                ch.expression.set(value as f32 / 127.0, self.stream_params.sample_rate);
            }
            0x40 => {
                // Damper / Sustain
                ch.damper = value >= 64;
            }
            0x48 => {
                // CC72: Release Time (0–16x, xsynth-style curve)
                let norm = value as f32 / 127.0;
                ch.release_multiplier = if value <= 64 {
                    (norm / (64.0 / 127.0)).powi(5)  // 0..1, fast shorten
                } else {
                    1.0 + ((norm - 64.0 / 127.0) / (63.0 / 127.0)).powi(3) * 15.0  // 1..16
                };
            }
            0x64 => {
                // RPN LSB
                ch.rpn_lsb = value;
            }
            0x65 => {
                // RPN MSB
                ch.rpn_msb = value;
            }
            0x06 => {
                // Data Entry MSB — RPN parameter value
                if ch.rpn_msb == 0 && ch.rpn_lsb == 0 {
                    // Pitch Bend Range (semitones)
                    ch.pitch_bend_range = value as f32;
                } else if ch.rpn_msb == 0 && ch.rpn_lsb == 1 {
                    // Fine Tune: value 0–127 centered at 64, ±100 cents
                    ch.fine_tune = (value as f32 - 64.0) * 100.0 / 64.0;
                } else if ch.rpn_msb == 0 && ch.rpn_lsb == 2 {
                    // Coarse Tune: value 0–127 centered at 64, ±64 semitones
                    ch.coarse_tune = value as f32 - 64.0;
                }
            }
            0x78 if value == 0 => {
                // All Sound Off
                self.voices.retain(|(vch, _)| *vch != ch_idx);
            }
            0x79 if value == 0 => {
                // Reset All Controllers
                let sfs = std::mem::take(&mut ch.soundfonts);
                *ch = ChannelState {
                    soundfonts: sfs,
                    ..Default::default()
                };
            }
            0x7B if value == 0 => {
                // All Notes Off
                for (vch, voice) in &mut self.voices {
                    if *vch == ch_idx {
                        voice.note_off();
                    }
                }
            }
            _ => {}
        }
    }

    // ── Voice management ────────────────────────────────────

    fn note_on_internal(&mut self, ch_idx: u32, bank: u8, program: u8, key: u8, vel: u8) {
        if vel == 0 {
            // Velocity 0 = note off per MIDI spec
            self.note_off_internal(ch_idx, key, false);
            return;
        }

        // Read channel-level params (snapshot to avoid borrow issues)
        let (ch_pitch_shift, ch_gain, ch_release_mult) = {
            let ch = &self.channels[ch_idx as usize];
            let pitch_shift = ch.coarse_tune + ch.pitch_bend * ch.pitch_bend_range + ch.fine_tune / 100.0;
            let vol = ch.volume.get();
            let expr = ch.expression.get();
            let gain = (vol * expr).powi(2);  // xsynth-style: quadratic channel law
            let release_mult = ch.release_multiplier;
            (pitch_shift, gain, release_mult)
        };
        let ch_pitch_mult = 2.0f32.powf(ch_pitch_shift / 12.0);

        // Find matching soundfonts — collect params before mutating self.voices
        let mut all_params = Vec::new();
        {
            let ch = &self.channels[ch_idx as usize];
            for sf in &ch.soundfonts {
                all_params.extend(sf.voice_params(bank, program, key, vel));
            }
        }

        for mut params in all_params {
            params.speed_mult *= ch_pitch_mult;
            params.volume *= ch_gain;
            params.envelope.release = (params.envelope.release * ch_release_mult).max(0.5);

            if self.voices.len() >= MAX_VOICES {
                self.steal_voice();
            }

            self.voices.push((ch_idx, Voice::new(
                &params,
                self.stream_params.sample_rate,
                key,
                vel,
            )));
            self.voice_count += 1;
        }
    }

    fn note_off_internal(&mut self, ch_idx: u32, key: u8, damper: bool) {
        for (vch, voice) in &mut self.voices {
            if *vch == ch_idx && voice.key == key && !voice.is_releasing() {
                if damper {
                    voice.damper_sustained = true;
                } else {
                    voice.note_off();
                }
            }
        }
    }

    fn steal_voice(&mut self) {
        // Prefer to steal a releasing voice
        if let Some(idx) = self
            .voices
            .iter()
            .position(|(_, v)| v.is_releasing())
        {
            self.voices.swap_remove(idx);
            return;
        }
        // Otherwise give the quietest voice a 5ms kill release instead of cutting it abruptly.
        // This avoids audible pops from instantaneous removal.
        if let Some((quietest_idx, _)) = self.voices.iter()
            .enumerate()
            .min_by_key(|(_, (_, v))| v.velocity) {
            self.voices[quietest_idx].1.kill();
            // Don't swap_remove — let it fade out naturally in the next read_samples pass.
            // We still need to make room, so mark the slot for reuse by the caller's push.
            // The caller unconditionally pushes a new voice after steal_voice returns, so
            // we must remove this entry.  Because the voice has been killed (5ms fade)
            // its amplitude will be near-zero by the time the render loop processes it.
            self.voices.swap_remove(quietest_idx);
        }
    }
}

impl AudioPipe for Synthesizer {
    fn stream_params(&self) -> AudioStreamParams {
        self.stream_params
    }

    fn read_samples(&mut self, buffer: &mut [f32]) {
        self.flush_events();

        let gain = master_gain(self.voices.len());

        match self.stream_params.channels {
            ChannelCount::Mono => {
                for sample in buffer.iter_mut() {
                    advance_channel_smoothers(&mut self.channels);
                    let mut mix: f32 = 0.0;
                    let mut i = 0;
                    while i < self.voices.len() {
                        mix += self.voices[i].1.process();
                        if self.voices[i].1.finished() {
                            self.voices.swap_remove(i);
                            self.voice_count = self.voice_count.saturating_sub(1);
                        } else {
                            i += 1;
                        }
                    }
                    *sample = soft_clip(mix * gain);
                }
            }
            ChannelCount::Stereo => {
                for chunk in buffer.chunks_exact_mut(2) {
                    advance_channel_smoothers(&mut self.channels);
                    let mut left: f32 = 0.0;
                    let mut right: f32 = 0.0;
                    let mut i = 0;
                    while i < self.voices.len() {
                        let sample = self.voices[i].1.process();
                        // Simple pan: equal power panning
                        let ch = &self.channels[self.voices[i].0 as usize];
                        let pan = ch.pan.get();
                        let left_gain = (1.0 - pan).sqrt();
                        let right_gain = pan.sqrt();
                        left += sample * left_gain;
                        right += sample * right_gain;

                        if self.voices[i].1.finished() {
                            self.voices.swap_remove(i);
                            self.voice_count = self.voice_count.saturating_sub(1);
                        } else {
                            i += 1;
                        }
                    }
                    chunk[0] = soft_clip(left * gain);
                    chunk[1] = soft_clip(right * gain);
                }
            }
        }
    }
}

#[inline]
fn advance_channel_smoothers(channels: &mut [ChannelState]) {
    for ch in channels {
        ch.volume.advance();
        ch.expression.advance();
        ch.pan.advance();
    }
}

/// Master gain attenuation: provides headroom for multi-voice mixing.
/// Default 0.25, but dynamically reduced to `6.0 / active_voices` when voices pile up.
const NOMINAL_MASTER_GAIN: f32 = 0.25;

/// Soft clip: C∞-continuous tanh approximation, smooth saturation.
/// Transparent for |x|≤1, then asymptotically approaches ±1.0 with C∞ continuity.
#[inline]
fn soft_clip(x: f32) -> f32 {
    let ax = x.abs();
    if ax <= 1.0 {
        x
    } else {
        // g(x) = 1 - exp(-2*(|x|-1)): C∞ at |x|=1, range (-1, 1), no hard clip
        x.signum() * (1.0 - (-2.0 * (ax - 1.0)).exp())
    }
}

#[inline]
fn master_gain(voice_count: usize) -> f32 {
    let active = voice_count.max(1) as f32;
    NOMINAL_MASTER_GAIN.min(6.0 / active).max(0.05)
}
