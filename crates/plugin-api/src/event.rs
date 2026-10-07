use serde::{Deserialize, Serialize};

/// 单个 MIDI 事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MidiEvent {
    NoteOn { note: u8, velocity: u8 },
    NoteOff { note: u8 },
    ControlChange { controller: u8, value: u8 },
}

/// 带块内帧偏移的 MIDI 事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimedMidiEvent {
    pub frame: u32,
    pub event: MidiEvent,
}
