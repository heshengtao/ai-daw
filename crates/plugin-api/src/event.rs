use serde::{Deserialize, Serialize};

/// 单个 MIDI 事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MidiEvent {
    NoteOn { note: u8, velocity: u8 },
    NoteOff { note: u8 },
    /// 单音音量（0..=127，127 = 不衰减）：纯增益微调，不改变音色。
    ///
    /// 与 `NoteOn` 的力度**分开**送达：支持 [`PluginCapabilities::per_note_volume`] 的插件
    /// 自行把它作用在该音上；不支持的插件忽略它，此时宿主应改走「折进力度」的兼容路径。
    ///
    /// [`PluginCapabilities::per_note_volume`]: crate::PluginCapabilities::per_note_volume
    NoteVolume { note: u8, value: u8 },
    ControlChange { controller: u8, value: u8 },
}

/// 带块内帧偏移的 MIDI 事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimedMidiEvent {
    pub frame: u32,
    pub event: MidiEvent,
}
