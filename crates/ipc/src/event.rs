use serde::{Deserialize, Serialize};

use crate::model::{AudioSource, ClipId, ClipPlacement, ClipRef, Note, NoteEdit, TrackId};

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
    /// 拍号改变（已归一化）。
    TimeSignatureChanged { actor: Actor, num: u8, den: u8 },
    /// 关键点被设置（已归一化）。
    KeyframeSet { actor: Actor, beat: f64, tempo: f64, num: u8, den: u8 },
    /// 关键点被删除。
    KeyframeRemoved { actor: Actor, beat: f64 },
    /// 轨道新增。
    TrackAdded { actor: Actor, id: TrackId, name: String },
    /// 轨道删除。
    TrackRemoved { actor: Actor, id: TrackId },
    /// 轨道被重命名。
    TrackRenamed { actor: Actor, track: TrackId, name: String },
    /// 轨道乐器被设置。
    TrackInstrumentSet { actor: Actor, track: TrackId, plugin_id: String },
    /// 轨道乐器被卸载。
    TrackInstrumentCleared { actor: Actor, track: TrackId },
    /// 片段新增。
    ClipAdded { actor: Actor, track: TrackId, clip: ClipId, start: f64, length: f64 },
    /// 音频片段新增（事件自带素材引用，重放无需另查）。
    AudioClipAdded { actor: Actor, track: TrackId, clip: ClipId, start: f64, length: f64, source: AudioSource },
    /// 片段的音频素材被改写；`audio` 为 `None` 表示变回 MIDI 片段。
    ClipAudioChanged { actor: Actor, track: TrackId, clip: ClipId, audio: Option<AudioSource> },
    /// 片段删除。
    ClipRemoved { actor: Actor, track: TrackId, clip: ClipId },
    /// 片段被切分：左半段沿用原 `clip`，右半段是 `new_clip`（自 `at_beat` 起）。
    ///
    /// 两半段的内容（音符 / 音频窗口）直接带在事件里，重放无需再算——切分是确定性的，
    /// 但把结果写进事件能让重放与实时编辑**逐字一致**。
    ClipSplit {
        actor: Actor,
        track: TrackId,
        clip: ClipId,
        new_clip: ClipId,
        at_beat: f64,
        left_notes: Vec<Note>,
        right_notes: Vec<Note>,
        left_audio: Option<AudioSource>,
        right_audio: Option<AudioSource>,
    },
    /// 片段被移动到新的起始拍。
    ClipMoved { actor: Actor, track: TrackId, clip: ClipId, start: f64 },
    /// 多个片段被移动（多选拖动）。
    ClipsMoved { actor: Actor, items: Vec<ClipPlacement> },
    /// 片段长度被改变。
    ClipResized { actor: Actor, track: TrackId, clip: ClipId, length: f64 },
    /// 音符新增。
    NoteAdded { actor: Actor, track: TrackId, clip: ClipId, note: Note },
    /// 音符被改写（`index` 为片段内的序号）。
    NoteChanged { actor: Actor, track: TrackId, clip: ClipId, index: usize, note: Note },
    /// 多个音符被改写（多选拖动 / 属性面板）。
    NotesChanged { actor: Actor, track: TrackId, clip: ClipId, entries: Vec<NoteEdit> },
    /// 音符被删除（`index` 为片段内的序号）。
    NoteRemoved { actor: Actor, track: TrackId, clip: ClipId, index: usize },
    /// 多个音符被删除（框选后删除）。
    NotesRemoved { actor: Actor, track: TrackId, clip: ClipId, indices: Vec<usize> },
    /// 多个片段被删除（可跨轨道）。
    ClipsRemoved { actor: Actor, items: Vec<ClipRef> },
    /// 增益改变（dB）。
    GainChanged { actor: Actor, track: TrackId, gain_db: f32 },
    /// 声像改变。
    PanChanged { actor: Actor, track: TrackId, pan: f32 },
    /// 静音改变。
    MuteChanged { actor: Actor, track: TrackId, mute: bool },
    /// 独奏改变。
    SoloChanged { actor: Actor, track: TrackId, solo: bool },
    /// 总输出增益改变（dB）。
    MasterGainChanged { actor: Actor, gain_db: f32 },
    /// 乐器参数改变。
    ParamChanged { actor: Actor, track: TrackId, param: String, value: f32 },
    /// 效果器被追加到轨道效果链末尾。
    EffectAdded { actor: Actor, track: TrackId, plugin_id: String },
    /// 效果器被移除（按链内序号）。
    EffectRemoved { actor: Actor, track: TrackId, index: usize },
    /// 效果器在链中换了位置。
    EffectMoved { actor: Actor, track: TrackId, from: usize, to: usize },
    /// 效果器参数改变（按链内序号定位）。
    EffectParamChanged { actor: Actor, track: TrackId, index: usize, param: String, value: f32 },
    /// 工程被重命名。
    ProjectRenamed { actor: Actor, name: String },
    /// 工程被重置为空。`actor` 为发起者。
    ProjectCleared { actor: Actor, name: String },
}

impl Event {
    /// 返回本事件的变更来源。
    pub fn actor(&self) -> &Actor {
        match self {
            Event::TempoChanged { actor, .. }
            | Event::TimeSignatureChanged { actor, .. }
            | Event::KeyframeSet { actor, .. }
            | Event::KeyframeRemoved { actor, .. }
            | Event::TrackAdded { actor, .. }
            | Event::TrackRemoved { actor, .. }
            | Event::TrackRenamed { actor, .. }
            | Event::TrackInstrumentSet { actor, .. }
            | Event::TrackInstrumentCleared { actor, .. }
            | Event::ClipAdded { actor, .. }
            | Event::AudioClipAdded { actor, .. }
            | Event::ClipAudioChanged { actor, .. }
            | Event::ClipRemoved { actor, .. }
            | Event::ClipSplit { actor, .. }
            | Event::ClipMoved { actor, .. }
            | Event::ClipsMoved { actor, .. }
            | Event::ClipResized { actor, .. }
            | Event::NoteAdded { actor, .. }
            | Event::NoteChanged { actor, .. }
            | Event::NotesChanged { actor, .. }
            | Event::NoteRemoved { actor, .. }
            | Event::NotesRemoved { actor, .. }
            | Event::ClipsRemoved { actor, .. }
            | Event::GainChanged { actor, .. }
            | Event::PanChanged { actor, .. }
            | Event::MuteChanged { actor, .. }
            | Event::SoloChanged { actor, .. }
            | Event::MasterGainChanged { actor, .. }
            | Event::ParamChanged { actor, .. }
            | Event::EffectAdded { actor, .. }
            | Event::EffectRemoved { actor, .. }
            | Event::EffectMoved { actor, .. }
            | Event::EffectParamChanged { actor, .. }
            | Event::ProjectRenamed { actor, .. }
            | Event::ProjectCleared { actor, .. } => actor,
        }
    }

    /// 该事件是否只改变实时混音参数。
    ///
    /// 这类事件不需要重建播放器的定帧事件表：宿主只需把新的混音参数直接推给音频线程
    /// 即可，因此拖动推子/切换静音时不会造成音频中断。其余事件（增删轨道、移动片段、
    /// 改速度……）都会改变事件表，必须重建。
    pub fn is_mix_only(&self) -> bool {
        matches!(
            self,
            Event::GainChanged { .. }
                | Event::PanChanged { .. }
                | Event::MuteChanged { .. }
                | Event::SoloChanged { .. }
                | Event::MasterGainChanged { .. }
        )
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

    #[test]
    fn project_cleared_roundtrips_through_json() {
        let e = Event::ProjectCleared { actor: Actor::Agent("claude".into()), name: "x".into() };
        let json = serde_json::to_string(&e).unwrap();
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
        assert_eq!(e.actor(), &Actor::Agent("claude".into()));
    }

    #[test]
    fn new_events_roundtrip_through_json() {
        let events = vec![
            Event::TrackRenamed { actor: Actor::Human, track: crate::model::TrackId(1), name: "Lead".into() },
            Event::ClipMoved { actor: Actor::Human, track: crate::model::TrackId(1), clip: crate::model::ClipId(2), start: 3.5 },
            Event::ClipResized { actor: Actor::Human, track: crate::model::TrackId(1), clip: crate::model::ClipId(2), length: 6.0 },
            Event::NoteChanged {
                actor: Actor::Human,
                track: crate::model::TrackId(1),
                clip: crate::model::ClipId(2),
                index: 0,
                note: crate::model::Note { pitch: 64, start: 1.0, duration: 0.5, velocity: 90, volume: 127 },
            },
            Event::NoteRemoved { actor: Actor::Human, track: crate::model::TrackId(1), clip: crate::model::ClipId(2), index: 1 },
            Event::NotesChanged {
                actor: Actor::Human,
                track: crate::model::TrackId(1),
                clip: crate::model::ClipId(2),
                entries: vec![crate::model::NoteEdit {
                    index: 0,
                    note: crate::model::Note { pitch: 60, start: 0.0, duration: 1.0, velocity: 64, volume: 127 },
                }],
            },
            Event::NotesRemoved {
                actor: Actor::Human,
                track: crate::model::TrackId(1),
                clip: crate::model::ClipId(2),
                indices: vec![0, 2],
            },
            Event::ClipsRemoved {
                actor: Actor::Human,
                items: vec![crate::model::ClipRef {
                    track: crate::model::TrackId(1),
                    clip: crate::model::ClipId(2),
                }],
            },
            Event::MuteChanged { actor: Actor::Human, track: crate::model::TrackId(1), mute: true },
            Event::SoloChanged { actor: Actor::Human, track: crate::model::TrackId(1), solo: true },
            Event::MasterGainChanged { actor: Actor::Human, gain_db: -3.0 },
            Event::ParamChanged { actor: Actor::Human, track: crate::model::TrackId(1), param: "gain".into(), value: 0.5 },
            Event::KeyframeSet { actor: Actor::Human, beat: 2.0, tempo: 90.0, num: 3, den: 4 },
            Event::KeyframeRemoved { actor: Actor::Human, beat: 2.0 },
        ];
        for e in events {
            let json = serde_json::to_string(&e).unwrap();
            let back: Event = serde_json::from_str(&json).unwrap();
            assert_eq!(e, back);
            assert_eq!(e.actor(), &Actor::Human);
        }
    }

    #[test]
    fn only_mixer_events_are_mix_only() {
        assert!(Event::GainChanged { actor: Actor::Human, track: crate::model::TrackId(1), gain_db: -6.0 }.is_mix_only());
        assert!(Event::PanChanged { actor: Actor::Human, track: crate::model::TrackId(1), pan: 0.5 }.is_mix_only());
        assert!(Event::MuteChanged { actor: Actor::Human, track: crate::model::TrackId(1), mute: true }.is_mix_only());
        assert!(Event::SoloChanged { actor: Actor::Human, track: crate::model::TrackId(1), solo: true }.is_mix_only());
        assert!(Event::MasterGainChanged { actor: Actor::Human, gain_db: 0.0 }.is_mix_only());
        // 结构性变更不能走"仅混音"通道。
        assert!(!Event::TrackAdded { actor: Actor::Human, id: crate::model::TrackId(1), name: "A".into() }.is_mix_only());
        assert!(!Event::ClipMoved { actor: Actor::Human, track: crate::model::TrackId(1), clip: crate::model::ClipId(1), start: 1.0 }.is_mix_only());
        assert!(!Event::TempoChanged { actor: Actor::Human, tempo: 90.0 }.is_mix_only());
    }

    /// 事件是**外部标记**枚举（`{ Variant: body }`）——前端 `stores/engineEvents.ts` 按这个形状解析。
    #[test]
    fn event_wire_shape_is_externally_tagged() {
        let e = Event::GainChanged { actor: Actor::Human, track: crate::model::TrackId(2), gain_db: -3.0 };
        let v = serde_json::to_value(&e).unwrap();
        assert!(v["GainChanged"].is_object(), "事件必须是外部标记：{{\"GainChanged\": {{…}}}}");
        assert_eq!(v["GainChanged"]["track"], 2, "TrackId 序列化成裸数字");
        assert_eq!(v["GainChanged"]["actor"], "Human", "Human 是 unit 变体");
        let agent = serde_json::to_value(Event::TrackRemoved { actor: Actor::Agent("mcp".into()), id: crate::model::TrackId(4) }).unwrap();
        assert_eq!(agent["TrackRemoved"]["actor"]["Agent"], "mcp", "Agent 携带代理名");
    }
}
