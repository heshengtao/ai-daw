use crate::model::{ClipId, Note, TrackId};

/// 变更来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    /// 人类用户。
    Human,
    /// AI 代理（携带代理标识）。
    Agent(String),
}

/// 事件：一次已发生、可重放的状态变更。
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// 速度改变。
    TempoChanged { actor: Actor, tempo: f64 },
    /// 轨道新增。
    TrackAdded { actor: Actor, id: TrackId, name: String },
    /// 轨道删除。
    TrackRemoved { actor: Actor, id: TrackId },
    /// 轨道乐器被设置。
    TrackInstrumentSet { actor: Actor, track: TrackId, plugin_id: String },
    /// 片段新增。
    ClipAdded { actor: Actor, track: TrackId, clip: ClipId, start: f64, length: f64 },
    /// 片段删除。
    ClipRemoved { actor: Actor, track: TrackId, clip: ClipId },
    /// 音符新增。
    NoteAdded { actor: Actor, track: TrackId, clip: ClipId, note: Note },
    /// 增益改变（dB）。
    GainChanged { actor: Actor, track: TrackId, gain_db: f32 },
    /// 声像改变。
    PanChanged { actor: Actor, track: TrackId, pan: f32 },
}

impl Event {
    /// 返回本事件的变更来源。
    pub fn actor(&self) -> &Actor {
        match self {
            Event::TempoChanged { actor, .. }
            | Event::TrackAdded { actor, .. }
            | Event::TrackRemoved { actor, .. }
            | Event::TrackInstrumentSet { actor, .. }
            | Event::ClipAdded { actor, .. }
            | Event::ClipRemoved { actor, .. }
            | Event::NoteAdded { actor, .. }
            | Event::GainChanged { actor, .. }
            | Event::PanChanged { actor, .. } => actor,
        }
    }
}
