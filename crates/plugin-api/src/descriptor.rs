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

/// 插件能力位：宿主据此选择信号通路与界面入口。
///
/// 缺省（`Default`）为「都不支持」，因此不含该字段的旧描述符仍按最保守方式解读。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PluginCapabilities {
    /// 能把「单音音量」当作独立于力度的信号处理（如 VST3 的 Note Expression）。
    ///
    /// 不支持时宿主无从单独传达单音音量，只能把它折进力度（[`crate::MidiEvent`] 的约定）。
    pub per_note_volume: bool,
    /// 自带可视化编辑器（原生窗口）。
    pub has_editor: bool,
    /// 支持逐音音高表达（VST3 Note Expression 的 Tuning / 内置合成器）。
    ///
    /// 不支持时，引擎会把 [`crate::MidiEvent::NotePitch`] 折叠成通道级
    /// [`crate::MidiEvent::PitchBend`]。
    pub per_note_pitch: bool,
    /// 响应通道级弯音（[`crate::MidiEvent::PitchBend`]）。音源通常为 true，效果器为 false。
    pub channel_pitch_bend: bool,
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
            io: IoLayout { inputs: 0, outputs: 2 },
            capabilities: PluginCapabilities {
                per_note_volume: true,
                has_editor: false,
                per_note_pitch: true,
                channel_pitch_bend: true,
            },
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: PluginDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }
}
