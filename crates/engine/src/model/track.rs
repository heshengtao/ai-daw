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
    pub clips: Vec<Clip>,
    pub mixer: MixerChannel,
}
