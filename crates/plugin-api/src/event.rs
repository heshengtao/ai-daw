use serde::{Deserialize, Serialize};

/// 单个 MIDI 事件。
///
/// **不带通道**：通道是路由属性，放在 [`TimedMidiEvent::channel`]（与帧偏移同级）。
/// 这样所有事件共享同一条通道规则，也让 MPE 的逐音通道不必污染每种事件的形状。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum MidiEvent {
    NoteOn {
        note: u8,
        velocity: u8,
    },
    NoteOff {
        note: u8,
    },
    /// 单音音量（0..=127，127 = 不衰减）：纯增益微调，不改变音色。
    ///
    /// 与 `NoteOn` 的力度**分开**送达：插件声明
    /// [`NoteExpressionSupport::volume`](crate::NoteExpressionSupport::volume) 时自行把它作用在该音上；
    /// 不支持的插件忽略它，此时宿主改走「折进力度」的兼容路径。
    NoteVolume {
        note: u8,
        value: u8,
    },
    ControlChange {
        controller: u8,
        value: u8,
    },
    /// 通道级弯音：音高偏移，单位音分（100 音分 = 1 个半音）。
    ///
    /// 宿主按 [`Plugin::set_pitch_bend_range`](crate::Plugin::set_pitch_bend_range)
    /// 设定的范围把它映射成 14-bit MIDI 弯音。同一通道上所有正在发声的音都会受影响，
    /// 所以它是「逐音表达」不可用时的退化路径。
    PitchBend {
        cents: i16,
    },
    /// 逐音弯音：只作用于指定音高，单位音分。
    ///
    /// 插件声明 [`NoteExpressionSupport::tuning`](crate::NoteExpressionSupport::tuning)
    /// 时走逐音表达（VST3 Note Expression / 内置合成器）；否则由引擎折叠成
    /// [`MidiEvent::PitchBend`]。
    NotePitch {
        note: u8,
        cents: i16,
    },

    /// 通道触后（channel pressure / aftertouch），0..=127。
    ///
    /// 对多数采样音源，这是「揉弦 / 渐强」最常用的表达；是否响应由插件决定。
    ChannelPressure {
        value: u8,
    },
    /// 复音压力（polyphonic key pressure），0..=127。
    ///
    /// VST3 有原生 `kPolyPressureEvent`；AU/CLAP 走 3 字节 `0xA0`。
    PolyPressure {
        note: u8,
        value: u8,
    },
    /// 音色切换（0..=127）。是否响应由插件决定。
    ProgramChange {
        program: u8,
    },
    /// 插件声明的逐音表达维度，值为归一化的 `0.0..=1.0`。
    ///
    /// 双极维度（Pan / Tuning）中心为 `0.5`。Tuning / Volume 仍走专用变体，
    /// 因为它们用音分 / 7-bit 语义，换算更明确。
    NoteExpression {
        note: u8,
        kind: NoteExpressionKind,
        value: f32,
    },
}

/// VST3 / CLAP 的逐音表达维度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteExpressionKind {
    /// 声像（双极）。
    Pan,
    /// 音高（双极，通常用 [`MidiEvent::NotePitch`]）。
    Tuning,
    /// 颤音深度。
    Vibrato,
    /// 表情 / 动态。
    Expression,
    /// 明亮度 / 音色。
    Brightness,
    /// 插件自定义维度。
    Custom(u32),
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
    pitch_bend_midi_bytes_on(cents, range_semitones, 0)
}

/// 同 [`pitch_bend_midi_bytes`]，但带上 MIDI 通道（0..=15）。
pub fn pitch_bend_midi_bytes_on(cents: i16, range_semitones: f32, channel: u8) -> [u8; 3] {
    let value = cents_to_pitch_bend(cents, range_semitones);
    [
        0xE0 | channel.min(15),
        (value & 0x7F) as u8,
        (value >> 7) as u8,
    ]
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TimedMidiEvent {
    pub frame: u32,
    /// MIDI 通道（0..=15）。缺省 0 = 传统单通道。
    ///
    /// 旧工程 / 手写 JSON 没有这个字段时按 0 读入。
    #[serde(default)]
    pub channel: u8,
    pub event: MidiEvent,
}

impl TimedMidiEvent {
    /// 通道 0 上的事件（绝大多数插件的默认路径）。
    pub fn on_main(frame: u32, event: MidiEvent) -> Self {
        Self {
            frame,
            channel: 0,
            event,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cents_map_to_the_midi_bend_center_and_extremes() {
        assert_eq!(cents_to_pitch_bend(0, 2.0), 8192);
        assert_eq!(
            cents_to_pitch_bend(200, 2.0),
            16383,
            "上推一个全音 = 满量程"
        );
        assert_eq!(cents_to_pitch_bend(-200, 2.0), 1, "下滑一个全音 = 最低");
        assert_eq!(cents_to_pitch_bend(100, 2.0), 12288, "半个量程");
        assert_eq!(
            cents_to_pitch_bend(50, 1.0),
            12288,
            "范围越小，同样的音分越接近满量程"
        );
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
        assert_eq!(
            pitch_bend_midi_bytes(0, 2.0),
            [0xE0, 0x00, 0x40],
            "中心 = 8192"
        );
        assert_eq!(
            pitch_bend_midi_bytes(200, 2.0),
            [0xE0, 0x7F, 0x7F],
            "满量程"
        );
    }

    #[test]
    fn pitch_bend_message_carries_the_channel() {
        assert_eq!(pitch_bend_midi_bytes_on(0, 2.0, 3), [0xE3, 0x00, 0x40]);
        // 越界通道被夹到 15，而不是回绕到 0。
        assert_eq!(pitch_bend_midi_bytes_on(0, 2.0, 99), [0xEF, 0x00, 0x40]);
    }

    #[test]
    fn a_bad_range_falls_back_to_two_semitones() {
        assert_eq!(cents_to_pitch_bend(200, 0.0), cents_to_pitch_bend(200, 2.0));
        assert_eq!(
            cents_to_pitch_bend(200, f32::NAN),
            cents_to_pitch_bend(200, 2.0)
        );
        assert!((cents_to_note_expression(200, -1.0) - 1.0).abs() < 1e-9);
    }

    /// 事件会写进工程 JSON（`Track.midi_events`），形状必须钉住。
    #[test]
    fn midi_event_wire_shape_is_stable() {
        let event = TimedMidiEvent::on_main(
            0,
            MidiEvent::ControlChange {
                controller: 64,
                value: 127,
            },
        );
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(
            json,
            r#"{"frame":0,"channel":0,"event":{"ControlChange":{"controller":64,"value":127}}}"#
        );

        let old: TimedMidiEvent =
            serde_json::from_str(r#"{"frame":12,"event":{"NoteOn":{"note":60,"velocity":100}}}"#)
                .unwrap();
        assert_eq!(old.channel, 0, "旧数据没有 channel 时按 0 读入");
    }

    #[test]
    fn note_expression_kind_roundtrips() {
        let event = MidiEvent::NoteExpression {
            note: 60,
            kind: NoteExpressionKind::Brightness,
            value: 0.25,
        };
        let back: MidiEvent =
            serde_json::from_str(&serde_json::to_string(&event).unwrap()).unwrap();
        assert_eq!(back, event);
        assert_eq!(
            serde_json::to_string(&NoteExpressionKind::Custom(7)).unwrap(),
            r#"{"custom":7}"#
        );
    }
}
