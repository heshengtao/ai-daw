use crate::apply::apply;
use crate::command::Command;
use crate::event::{Actor, Event};
use crate::model::{ClipId, Project, TrackId};

/// 引擎：持有工程状态、执行命令、产出事件、支持撤销。
pub struct Engine {
    project: Project,
    history: Vec<Vec<Event>>,
    /// 被撤销的命令批次，供 `redo` 重放；一旦执行新命令即清空。
    redo: Vec<Vec<Event>>,
    next_track_id: u64,
    next_clip_id: u64,
    /// 引擎的初始工程名；撤销一次 `Clear` 时用它重建，恢复被清空前的名字。
    initial_name: String,
}

impl Engine {
    /// 创建引擎：空工程（默认传输设置）、空历史，id 计数器从 1 开始。
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        Engine {
            project: Project::new(name.clone()),
            history: vec![],
            redo: vec![],
            next_track_id: 1,
            next_clip_id: 1,
            initial_name: name,
        }
    }

    /// 从已存在的工程构建引擎，并把 id 计数器恢复到不冲突的高水位。
    pub fn from_project(project: Project) -> Self {
        let next_track_id = project.tracks.iter().map(|t| t.id.0).max().unwrap_or(0) + 1;
        let next_clip_id = project
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.id.0)
            .max()
            .unwrap_or(0)
            + 1;
        let initial_name = project.name.clone();
        Engine { project, history: vec![], redo: vec![], next_track_id, next_clip_id, initial_name }
    }

    fn alloc_track_id(&mut self) -> TrackId {
        let id = TrackId(self.next_track_id);
        self.next_track_id += 1;
        id
    }

    fn alloc_clip_id(&mut self) -> ClipId {
        let id = ClipId(self.next_clip_id);
        self.next_clip_id += 1;
        id
    }

    /// 当前工程状态。
    pub fn project(&self) -> &Project {
        &self.project
    }

    /// 可撤销的命令次数。
    pub fn undo_depth(&self) -> usize {
        self.history.len()
    }

    /// 执行命令：产出事件、应用到状态、记录历史。
    pub fn execute(&mut self, command: Command, actor: Actor) -> Vec<Event> {
        let events = self.translate(&command, &actor);
        for ev in &events {
            apply(&mut self.project, ev);
        }
        if !events.is_empty() {
            self.history.push(events.clone());
            // 新命令会作废已有的重做历史。
            self.redo.clear();
        }
        events
    }

    /// 撤销最后一次命令；成功返回 true。
    ///
    /// 采用"重放"实现：从初始工程重新应用剩余历史，因此撤销永远得到一致状态，
    /// 无需为每条命令编写逆操作。
    pub fn undo(&mut self) -> bool {
        match self.history.pop() {
            None => false,
            Some(batch) => {
                self.redo.push(batch);
                self.replay_history();
                true
            }
        }
    }

    /// 重做上一次被撤销的命令；成功返回 true。
    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            None => false,
            Some(batch) => {
                for ev in &batch {
                    apply(&mut self.project, ev);
                }
                self.history.push(batch);
                true
            }
        }
    }

    /// 可重做的命令次数。
    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    /// 从初始工程重放当前历史，重建撤销后的状态。
    fn replay_history(&mut self) {
        self.project = Project::new(self.initial_name.clone());
        for batch in self.history.clone() {
            for ev in batch {
                apply(&mut self.project, &ev);
            }
        }
    }

    fn translate(&mut self, command: &Command, actor: &Actor) -> Vec<Event> {
        match command {
            Command::SetTempo(tempo) => {
                // 夹紧到安全区间：`tempo` 会参与 `60 / bpm` 的秒数换算，0 或负数会
                // 让渲染时长变成 Inf / NaN，最终触发超大分配而中止进程。
                vec![Event::TempoChanged { actor: actor.clone(), tempo: tempo.clamp(1.0, 1000.0) }]
            }
            Command::AddTrack { name } => {
                let id = self.alloc_track_id();
                vec![Event::TrackAdded { actor: actor.clone(), id, name: name.clone() }]
            }
            Command::RemoveTrack(track) => {
                vec![Event::TrackRemoved { actor: actor.clone(), id: *track }]
            }
            Command::SetTrackInstrument { track, plugin_id } => {
                vec![Event::TrackInstrumentSet {
                    actor: actor.clone(),
                    track: *track,
                    plugin_id: plugin_id.clone(),
                }]
            }
            Command::AddClip { track, start, length } => {
                let clip = self.alloc_clip_id();
                vec![Event::ClipAdded { actor: actor.clone(), track: *track, clip, start: *start, length: *length }]
            }
            Command::RemoveClip { track, clip } => {
                vec![Event::ClipRemoved { actor: actor.clone(), track: *track, clip: *clip }]
            }
            Command::AddNote { track, clip, note } => {
                vec![Event::NoteAdded { actor: actor.clone(), track: *track, clip: *clip, note: *note }]
            }
            Command::SetGain { track, gain_db } => {
                vec![Event::GainChanged { actor: actor.clone(), track: *track, gain_db: *gain_db }]
            }
            Command::SetPan { track, pan } => {
                vec![Event::PanChanged { actor: actor.clone(), track: *track, pan: *pan }]
            }
            Command::Clear { name } => {
                vec![Event::ProjectCleared { actor: actor.clone(), name: name.clone() }]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
    use crate::event::Actor;

    #[test]
    fn execute_returns_events_and_mutates_state() {
        let mut e = Engine::new("demo");
        let events = e.execute(Command::AddTrack { name: "Bass".into() }, Actor::Human);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], crate::event::Event::TrackAdded { .. }));
        assert_eq!(e.project().tracks.len(), 1);
    }

    #[test]
    fn undo_reverts_last_change() {
        let mut e = Engine::new("demo");
        e.execute(Command::AddTrack { name: "Bass".into() }, Actor::Human);
        e.execute(Command::SetTempo(150.0), Actor::Human);
        assert_eq!(e.project().transport.tempo, 150.0);
        assert!(e.undo());
        assert_eq!(e.project().transport.tempo, 120.0);
        assert!(e.undo());
        assert!(e.project().tracks.is_empty());
        assert!(!e.undo());
    }

    #[test]
    fn set_tempo_is_clamped_to_safe_range() {
        let mut e = Engine::new("demo");
        // 0 BPM 会让渲染时长变成 Inf，必须被夹紧到 1.0。
        e.execute(Command::SetTempo(0.0), Actor::Human);
        assert_eq!(e.project().transport.tempo, 1.0);
        // 过大的 BPM 同样被封顶到 1000.0。
        e.execute(Command::SetTempo(100000.0), Actor::Human);
        assert_eq!(e.project().transport.tempo, 1000.0);
    }

    #[test]
    fn add_clip_uses_allocated_ids_and_is_consistent() {
        let mut e = Engine::new("demo");
        let ev = e.execute(Command::AddTrack { name: "Keys".into() }, Actor::Human);
        let track = match ev[0] { crate::event::Event::TrackAdded { id, .. } => id, _ => unreachable!() };
        let ev = e.execute(Command::AddClip { track, start: 0.0, length: 8.0 }, Actor::Human);
        let clip = match ev[0] { crate::event::Event::ClipAdded { clip, .. } => clip, _ => unreachable!() };
        assert_eq!(e.project().track(track).unwrap().clips[0].id, clip);
    }

    #[test]
    fn from_project_recovers_id_counters() {
        let mut e = Engine::new("demo");
        e.execute(Command::AddTrack { name: "A".into() }, Actor::Human);
        e.execute(Command::AddTrack { name: "B".into() }, Actor::Human);
        let project = e.project().clone();

        let mut loaded = Engine::from_project(project);
        let ev = loaded.execute(Command::AddTrack { name: "C".into() }, Actor::Human);
        let id = match ev[0] { crate::event::Event::TrackAdded { id, .. } => id, _ => unreachable!() };
        // must not collide with the two loaded tracks
        assert_eq!(id.0, 3);
        assert_eq!(loaded.project().tracks.len(), 3);
    }

    #[test]
    fn clear_resets_project_and_is_undoable() {
        let mut e = Engine::new("demo");
        e.execute(Command::AddTrack { name: "Bass".into() }, Actor::Human);
        assert_eq!(e.project().tracks.len(), 1);

        let events = e.execute(Command::Clear { name: "x".into() }, Actor::Human);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], crate::event::Event::ProjectCleared { .. }));
        assert!(e.project().tracks.is_empty());
        assert_eq!(e.project().name, "x");

        assert!(e.undo());
        assert_eq!(e.project().tracks.len(), 1);
        // 撤销 Clear 必须恢复被清空前的工程名，而不是保留 Clear 传入的新名。
        assert_eq!(e.project().name, "demo");
    }

    #[test]
    fn redo_reapplies_the_undone_command() {
        let mut e = Engine::new("demo");
        e.execute(Command::AddTrack { name: "Bass".into() }, Actor::Human);
        e.execute(Command::SetTempo(150.0), Actor::Human);

        assert!(e.undo());
        assert_eq!(e.project().transport.tempo, 120.0);
        assert_eq!(e.redo_depth(), 1);

        assert!(e.redo());
        assert_eq!(e.project().transport.tempo, 150.0);
        assert_eq!(e.project().tracks.len(), 1);
        assert_eq!(e.redo_depth(), 0);
        // 重做到底之后再重做应当失败。
        assert!(!e.redo());
    }

    #[test]
    fn redo_restores_a_cleared_project_with_all_tracks() {
        let mut e = Engine::new("demo");
        e.execute(Command::AddTrack { name: "A".into() }, Actor::Human);
        e.execute(Command::AddTrack { name: "B".into() }, Actor::Human);
        e.execute(Command::Clear { name: "gone".into() }, Actor::Human);
        assert_eq!(e.project().tracks.len(), 0);

        assert!(e.undo());
        assert_eq!(e.project().tracks.len(), 2);
        assert!(e.redo());
        assert_eq!(e.project().tracks.len(), 0);
        assert_eq!(e.project().name, "gone");
    }

    #[test]
    fn new_command_invalidates_redo_history() {
        let mut e = Engine::new("demo");
        e.execute(Command::AddTrack { name: "A".into() }, Actor::Human);
        assert!(e.undo());
        assert_eq!(e.redo_depth(), 1);

        e.execute(Command::AddTrack { name: "B".into() }, Actor::Human);
        assert_eq!(e.redo_depth(), 0);
        assert!(!e.redo());
        assert_eq!(e.project().tracks.len(), 1);
        assert_eq!(e.project().tracks[0].name, "B");
    }

    #[test]
    fn redo_is_a_noop_with_empty_history() {
        let mut e = Engine::new("demo");
        assert!(!e.redo());
        assert_eq!(e.redo_depth(), 0);
    }

    #[test]
    fn from_empty_project_starts_ids_at_one() {
        let mut e = Engine::from_project(crate::model::Project::new("empty"));
        let ev = e.execute(Command::AddTrack { name: "A".into() }, Actor::Human);
        let id = match ev[0] { crate::event::Event::TrackAdded { id, .. } => id, _ => unreachable!() };
        assert_eq!(id.0, 1);
    }
}
