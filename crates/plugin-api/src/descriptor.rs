use serde::{Deserialize, Serialize};

/// 插件类别。
///
/// - [`PluginKind::Instrument`]：音源，接收 MIDI、产出音频；[`IoLayout::inputs`] 必须为 0。
/// - [`PluginKind::Effect`]：效果，接收并产出音频；v1 要求 `inputs == outputs`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// 插件描述符。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginDescriptor {
    pub id: String,
    pub name: String,
    pub kind: PluginKind,
    pub io: IoLayout,
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
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: PluginDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }
}
