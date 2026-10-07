use crate::apply::apply;
use crate::command::Command;
use crate::event::{Actor, Event};
use crate::model::{ClipId, Project, TrackId};

/// 引擎：持有工程状态、执行命令、产出事件、支持撤销。
pub struct Engine {
    project: Project,
    history: Vec<Vec<Event>>,
    next_track_id: u64,
    next_clip_id: u64,
}

impl Engine {
    /// 创建引擎：空工程（默认传输设置）、空历史，id 计数器从 1 开始。
    pub fn new(name: impl Into<String>) -> Self {
        Engine {
            project: Project::new(name),
            history: vec![],
            next_track_id: 1,
            next_clip_id: 1,
        }
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
        }
        events
    }

    /// 撤销最后一次命令；成功返回 true。
    pub fn undo(&mut self) -> bool {
        match self.history.pop() {
            None => false,
            Some(events) => {
                self.project = Project::new(self.project.name.clone());
                let remaining: Vec<Vec<Event>> = self.history.clone();
                for batch in remaining {
                    for ev in batch {
                        apply(&mut self.project, &ev);
                    }
                }
                let _ = events;
                true
            }
        }
    }

    fn translate(&mut self, command: &Command, actor: &Actor) -> Vec<Event> {
        match command {
            Command::SetTempo(tempo) => {
                vec![Event::TempoChanged { actor: actor.clone(), tempo: *tempo }]
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
    fn add_clip_uses_allocated_ids_and_is_consistent() {
        let mut e = Engine::new("demo");
        let ev = e.execute(Command::AddTrack { name: "Keys".into() }, Actor::Human);
        let track = match ev[0] { crate::event::Event::TrackAdded { id, .. } => id, _ => unreachable!() };
        let ev = e.execute(Command::AddClip { track, start: 0.0, length: 8.0 }, Actor::Human);
        let clip = match ev[0] { crate::event::Event::ClipAdded { clip, .. } => clip, _ => unreachable!() };
        assert_eq!(e.project().track(track).unwrap().clips[0].id, clip);
    }
}
