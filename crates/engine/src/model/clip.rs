use serde::{Deserialize, Serialize};

/// 片段标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ClipId(pub u64);

/// 音符；位置与时长单位为拍。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub pitch: u8,
    pub start: f64,
    pub duration: f64,
    pub velocity: u8,
}

/// MIDI 片段。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    pub start: f64,
    pub length: f64,
    pub notes: Vec<Note>,
}
