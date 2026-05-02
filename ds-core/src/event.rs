use std::sync::Arc;

use crate::soundfont::SoundfontBase;

/// MIDI audio events for a single channel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChannelAudioEvent {
    /// Start a new note.
    NoteOn { key: u8, vel: u8 },
    /// Release a note.
    NoteOff { key: u8 },
    /// Release all active notes.
    AllNotesOff,
    /// Kill all voices immediately (no release tail).
    AllNotesKilled,
    /// Reset all CC values to defaults.
    ResetControl,
    /// A control event.
    Control(ControlEvent),
    /// Program change.
    ProgramChange(u8),
    /// System reset.
    SystemReset,
}

/// MIDI control events.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ControlEvent {
    /// Raw CC message: (controller_number, value).
    Raw(u8, u8),
    /// Pitch bend range in semitones.
    PitchBendSensitivity(f32),
    /// Normalized pitch bend value (-1.0 to 1.0).
    PitchBendValue(f32),
    /// Computed pitch bend in semitones (value × sensitivity).
    PitchBend(f32),
    /// Fine tune in cents.
    FineTune(f32),
    /// Coarse tune in semitones.
    CoarseTune(f32),
    /// Volume (0.0–1.0).
    Volume(f32),
    /// Pan (0.0 = left, 0.5 = center, 1.0 = right).
    Pan(f32),
    /// Expression (0.0–1.0).
    Expression(f32),
    /// Sustain/damper pedal.
    Damper(bool),
}

/// Channel configuration events.
#[derive(Clone, Debug)]
pub enum ChannelConfigEvent {
    /// Set the soundfonts for this channel.
    SetSoundfonts(Vec<Arc<dyn SoundfontBase>>),
    /// Enable/disable percussion mode (drum kit on channel 10).
    SetPercussionMode(bool),
}

/// Events for a single channel.
#[derive(Clone, Debug)]
pub enum ChannelEvent {
    Audio(ChannelAudioEvent),
    Config(ChannelConfigEvent),
}

/// Top-level synthesizer events.
#[derive(Clone, Debug)]
pub enum SynthEvent {
    /// Route an event to a specific channel.
    Channel(u32, ChannelEvent),
    /// Broadcast an event to all channels.
    AllChannels(ChannelEvent),
}
