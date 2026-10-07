use serde::{Deserialize, Serialize};

use crate::model::{ClipId, Note, TrackId};

/// 变更来源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Actor {
    /// 人类用户。
    Human,
    /// AI 代理（携带代理标识）。
    Agent(String),
}

/// 事件：一次已发生、可重放的状态变更。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[cfg(test)]
mod tests {
    use super::Event;
    use crate::event::Actor;

    #[test]
    fn event_roundtrips_through_json() {
        let e = Event::TrackAdded { actor: Actor::Agent("claude".into()), id: crate::model::TrackId(3), name: "Bass".into() };
        let json = serde_json::to_string(&e).unwrap();
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }
}
