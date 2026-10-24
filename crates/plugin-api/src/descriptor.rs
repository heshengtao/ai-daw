use serde::{Deserialize, Serialize};

/// 插件类别。
///
/// - [`PluginKind::Instrument`]：音源，接收 MIDI、产出音频；[`IoLayout::inputs`] 必须为 0。
/// - [`PluginKind::Effect`]：效果，接收并产出音频；v1 要求 `inputs == outputs`。
///
/// 序列化成小写（`"instrument"` / `"effect"`）：前端按这个字面量分流，之前默认的
/// `"Instrument"` 会让外部插件既不算乐器也不算效果，在音色列表里彻底消失。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    /// 音源（接收 MIDI，产出音频）。
    Instrument,
    /// 效果（接收音频，产出音频）。
    Effect,
}

/// 通道布局（`inputs` / `outputs` 为通道数，不是总线数）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IoLayout {
    pub inputs: u16,
    pub outputs: u16,
}

/// 插件声明支持的逐音表达维度。
///
/// 这些值来自插件的自述（VST3 的 `INoteExpressionController`）；AU / CLAP 没有等价探测，
/// 因此保守报 false。宿主只会自动投递 `volume` 与 `tuning`，其余维度经由
/// [`crate::MidiEvent::NoteExpression`] 显式发送。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NoteExpressionSupport {
    /// 逐音音量。
    pub volume: bool,
    /// 逐音声像。
    pub pan: bool,
    /// 逐音音高（Tuning）。
    pub tuning: bool,
    /// 逐音颤音深度。
    pub vibrato: bool,
    /// 逐音表情 / 动态。
    pub expression: bool,
    /// 逐音明亮度。
    pub brightness: bool,
}

/// 插件能力位：宿主据此选择信号通路与界面入口。
///
/// 缺省（`Default`）为「都不支持」，因此不含该字段的旧描述符仍按最保守方式解读。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginCapabilities {
    /// 接收 MIDI（音源通常为 true，效果器为 false）。
    pub midi_input: bool,
    /// 自带可视化编辑器（原生窗口）。
    pub has_editor: bool,
    /// 响应通道级弯音（[`crate::MidiEvent::PitchBend`]）。
    pub channel_pitch_bend: bool,
    /// 能接收通道触后（[`crate::MidiEvent::ChannelPressure`]）。
    pub channel_pressure: bool,
    /// 能接收复音压力（[`crate::MidiEvent::PolyPressure`]）。
    pub poly_pressure: bool,
    /// 能接收音色切换（[`crate::MidiEvent::ProgramChange`]）。
    pub program_change: bool,
    /// 宿主能把每颗音符送到独立 MIDI 通道（MPE 的**传输前提**）。
    ///
    /// true 只代表「送得到」；插件是否按 MPE 解释由它自己的开关决定。
    pub per_note_channel: bool,
    /// 插件声明的逐音表达维度。
    pub note_expression: NoteExpressionSupport,
}

impl PluginCapabilities {
    /// 支持逐音音高表达（VST3 Note Expression 的 Tuning / 内置合成器）。
    pub fn per_note_pitch(&self) -> bool {
        self.note_expression.tuning
    }

    /// 能把「单音音量」当作独立于力度的信号处理。
    pub fn per_note_volume(&self) -> bool {
        self.note_expression.volume
    }
}

/// 插件描述符。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginDescriptor {
    pub id: String,
    pub name: String,
    pub kind: PluginKind,
    pub io: IoLayout,
    /// 插件能力位；旧数据缺省为「都不支持」。
    #[serde(default)]
    pub capabilities: PluginCapabilities,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_roundtrips_through_json() {
        let d = PluginDescriptor {
            id: "builtin.synth".into(),
            name: "Builtin Synth".into(),
            kind: PluginKind::Instrument,
            io: IoLayout {
                inputs: 0,
                outputs: 2,
            },
            capabilities: PluginCapabilities {
                midi_input: true,
                has_editor: false,
                channel_pitch_bend: true,
                channel_pressure: true,
                poly_pressure: true,
                program_change: false,
                per_note_channel: true,
                note_expression: NoteExpressionSupport {
                    volume: true,
                    pan: false,
                    tuning: true,
                    vibrato: false,
                    expression: false,
                    brightness: false,
                },
            },
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: PluginDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }

    /// 老描述符（只有四个 bool）必须按「其余都不支持」读入。
    #[test]
    fn old_capabilities_default_the_new_flags_to_false() {
        let json = r#"{"per_note_volume":true,"has_editor":true,"per_note_pitch":true,"channel_pitch_bend":true}"#;
        let caps: PluginCapabilities = serde_json::from_str(json).unwrap();
        assert!(!caps.channel_pressure);
        assert!(!caps.poly_pressure);
        assert!(!caps.program_change);
        assert!(!caps.per_note_channel);
        assert_eq!(caps.note_expression, NoteExpressionSupport::default());
    }

    #[test]
    fn capability_helpers_follow_the_expression_mask() {
        let caps = PluginCapabilities {
            note_expression: NoteExpressionSupport {
                tuning: true,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(caps.per_note_pitch());
        assert!(!caps.per_note_volume());
    }
}
