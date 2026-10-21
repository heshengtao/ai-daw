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
    /// 通道级弯音：音高偏移，单位音分（100 音分 = 1 个半音）。
    ///
    /// 宿主按 [`Plugin::set_pitch_bend_range`](crate::Plugin::set_pitch_bend_range)
    /// 设定的范围把它映射成 14-bit MIDI 弯音。同一通道上所有正在发声的音都会受影响，
    /// 所以它是「逐音表达」不可用时的退化路径。
    PitchBend { cents: i16 },
    /// 逐音弯音：只作用于指定音高，单位音分。
    ///
    /// 支持 [`PluginCapabilities::per_note_pitch`](crate::PluginCapabilities::per_note_pitch)
    /// 的插件（VST3 Note Expression / 内置合成器）自行处理；不支持的插件由引擎折叠成
    /// [`MidiEvent::PitchBend`]。
    NotePitch { note: u8, cents: i16 },
}

/// 音分 → MIDI 14-bit 弯音值（中心 8192）。
///
/// `range_semitones` 是插件实际使用的弯音范围（默认 2 个半音）。非有限或非正范围回落到 2。
pub fn cents_to_pitch_bend(cents: i16, range_semitones: f32) -> u16 {
    let range = if range_semitones.is_finite() && range_semitones > 0.0 {
        range_semitones.clamp(0.01, 24.0)
    } else {
        2.0
    };
    let normalized = f64::from(cents) / (f64::from(range) * 100.0);
    let value = 8192.0 + normalized * 8191.0;
    value.round().clamp(0.0, 16383.0) as u16
}

/// 音分 → 原始 MIDI 1.0 弯音消息（3 字节，通道 0）。
///
/// CLAP / AU / SoundFont 采样器都走这条「原始 MIDI」路径；与
/// [`cents_to_pitch_bend`] 共用同一套范围换算。
pub fn pitch_bend_midi_bytes(cents: i16, range_semitones: f32) -> [u8; 3] {
    let value = cents_to_pitch_bend(cents, range_semitones);
    [0xE0, (value & 0x7F) as u8, (value >> 7) as u8]
}

/// 音分 → 逐音表达式归一化值（双极，中心 0.5）。
///
/// 与 [`cents_to_pitch_bend`] 使用同一个弯音范围约定，便于不同后端得到一致的听感。
pub fn cents_to_note_expression(cents: i16, range_semitones: f32) -> f64 {
    let range = if range_semitones.is_finite() && range_semitones > 0.0 {
        range_semitones.clamp(0.01, 24.0)
    } else {
        2.0
    };
    let normalized = 0.5 + f64::from(cents) / (f64::from(range) * 100.0) * 0.5;
    normalized.clamp(0.0, 1.0)
}

/// 带块内帧偏移的 MIDI 事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimedMidiEvent {
    pub frame: u32,
    pub event: MidiEvent,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cents_map_to_the_midi_bend_center_and_extremes() {
        assert_eq!(cents_to_pitch_bend(0, 2.0), 8192);
        assert_eq!(cents_to_pitch_bend(200, 2.0), 16383, "上推一个全音 = 满量程");
        assert_eq!(cents_to_pitch_bend(-200, 2.0), 1, "下滑一个全音 = 最低");
        assert_eq!(cents_to_pitch_bend(100, 2.0), 12288, "半个量程");
        assert_eq!(cents_to_pitch_bend(50, 1.0), 12288, "范围越小，同样的音分越接近满量程");
    }

    #[test]
    fn cents_map_to_note_expression_around_the_center() {
        assert!((cents_to_note_expression(0, 2.0) - 0.5).abs() < 1e-9);
        assert!((cents_to_note_expression(200, 2.0) - 1.0).abs() < 1e-9);
        assert!((cents_to_note_expression(-200, 2.0) - 0.0).abs() < 1e-9);
        assert!((cents_to_note_expression(100, 2.0) - 0.75).abs() < 1e-9);
    }

    #[test]
    fn pitch_bend_message_is_three_byte_midi() {
        assert_eq!(pitch_bend_midi_bytes(0, 2.0), [0xE0, 0x00, 0x40], "中心 = 8192");
        assert_eq!(pitch_bend_midi_bytes(200, 2.0), [0xE0, 0x7F, 0x7F], "满量程");
    }

    #[test]
    fn a_bad_range_falls_back_to_two_semitones() {
        assert_eq!(cents_to_pitch_bend(200, 0.0), cents_to_pitch_bend(200, 2.0));
        assert_eq!(cents_to_pitch_bend(200, f32::NAN), cents_to_pitch_bend(200, 2.0));
        assert!((cents_to_note_expression(200, -1.0) - 1.0).abs() < 1e-9);
    }
}
