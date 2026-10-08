use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::clip::Clip;

/// 轨道标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TrackId(pub u64);

/// 插件实例（工程内保存的插件引用与状态）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginInstance {
    pub plugin_id: String,
    pub params: BTreeMap<String, f32>,
    pub state: Vec<u8>,
}

/// 混音通道。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MixerChannel {
    pub gain_db: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
}

impl MixerChannel {
    /// 增益下界（dB）：约等于静音，避免 -inf 进入增益换算。
    pub const MIN_GAIN_DB: f32 = -60.0;
    /// 增益上界（dB）：+12 dB 足够补偿，同时挡住会削爆输出的夸张值。
    pub const MAX_GAIN_DB: f32 = 12.0;

    /// 归一化增益；非有限值回落到 0 dB。
    pub fn normalize_gain_db(db: f32) -> f32 {
        if db.is_finite() {
            db.clamp(Self::MIN_GAIN_DB, Self::MAX_GAIN_DB)
        } else {
            0.0
        }
    }

    /// 归一化声像；非有限值回落到居中。
    pub fn normalize_pan(pan: f32) -> f32 {
        if pan.is_finite() {
            pan.clamp(-1.0, 1.0)
        } else {
            0.0
        }
    }
}

impl Default for MixerChannel {
    fn default() -> Self {
        MixerChannel { gain_db: 0.0, pan: 0.0, mute: false, solo: false }
    }
}

/// 轨道。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub name: String,
    pub instrument: Option<PluginInstance>,
    /// 效果器链：按顺序处理（乐器输出 → effects[0] → effects[1] → … → 轨道混音）。
    /// 旧工程没有这个字段时读作空链。
    #[serde(default)]
    pub effects: Vec<PluginInstance>,
    pub clips: Vec<Clip>,
    pub mixer: MixerChannel,
}

#[cfg(test)]
mod tests {
    use super::MixerChannel;

    #[test]
    fn gain_and_pan_are_normalized() {
        assert_eq!(MixerChannel::normalize_gain_db(100.0), MixerChannel::MAX_GAIN_DB);
        assert_eq!(MixerChannel::normalize_gain_db(-100.0), MixerChannel::MIN_GAIN_DB);
        assert_eq!(MixerChannel::normalize_gain_db(f32::NAN), 0.0);
        assert_eq!(MixerChannel::normalize_pan(3.0), 1.0);
        assert_eq!(MixerChannel::normalize_pan(-3.0), -1.0);
        assert_eq!(MixerChannel::normalize_pan(f32::NAN), 0.0);
    }
}
