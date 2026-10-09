use crate::apply::apply;
use crate::command::Command;
use crate::event::{Actor, Event};
use crate::model::{ClipId, Project, TrackId};

/// 引擎：持有工程状态、执行命令、产出事件、支持撤销。
pub struct Engine {
    project: Project,
    /// **重放基线**：撤销靠「回到基线 + 重放剩余历史」，所以基线必须是引擎接手时的那份
    /// 工程，而不是空工程——否则启动时铺的默认分轨、打开的文件里的所有内容都会在第一次
    /// 撤销时凭空消失。
    initial: Project,
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
        let project = Project::new(name.clone());
        Engine {
            initial: project.clone(),
            project,
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
        let initial = project.clone();
        Engine { initial, project, history: vec![], redo: vec![], next_track_id, next_clip_id, initial_name }
    }

    /// 新建工程：清空并铺上默认分轨模板（见 [`crate::STARTER_TRACKS`]）。
    ///
    /// 与 `execute(Command::NewProject)` 的唯一区别是**不写入历史**：这是「应用启动时的
    /// 初始状态」，不该能一键撤销成空工程。用户主动点「新建工程」时走命令，那次是可撤销的。
    pub fn new_project(name: impl Into<String>) -> Self {
        let mut engine = Engine::new(name);
        let name = engine.initial_name.clone();
        let events = engine.translate(&Command::NewProject { name, track_names: None }, &Actor::Human);
        for event in &events {
            apply(&mut engine.project, event);
        }
        // 这一步铺出来的分轨不在历史里，所以它才是重放基线。
        engine.initial = engine.project.clone();
        engine
    }

    /// 把「刚铺好的默认分轨」的名称换成给定的一批名字，返回是否真的改了。
    ///
    /// 用途：应用**启动时**的初始工程是在界面起来之前建的，那时还不知道界面语言，只能先用
    /// 模板里的默认名。界面一上来会按当前语言调这个方法——但**只有工程仍是原封不动的模板时
    /// 才改名**：顺序、名称、乐器逐条对得上且没有任何片段/效果器。用户动过就一个字都不动。
    ///
    /// 改名会同时刷新重放基线，因此撤销不会退回旧语言的名字——按语言命名属于「初始状态」，
    /// 不是用户的一次编辑，不该出现在撤销栈里。
    pub fn localize_starter_tracks(&mut self, names: &[String]) -> bool {
        if names.len() != crate::STARTER_TRACKS.len() || !self.is_pristine_starter() {
            return false;
        }
        for (track, name) in self.project.tracks.iter_mut().zip(names) {
            track.name = name.clone();
        }
        self.initial = self.project.clone();
        true
    }

    /// 工程是不是「刚铺好的默认模板」——没被用户动过。
    fn is_pristine_starter(&self) -> bool {
        if self.project.tracks.len() != crate::STARTER_TRACKS.len() {
            return false;
        }
        crate::STARTER_TRACKS
            .iter()
            .zip(&self.project.tracks)
            .all(|(starter, track)| {
                track.name == starter.name
                    && track.clips.is_empty()
                    && track.effects.is_empty()
                    && track
                        .instrument
                        .as_ref()
                        .is_some_and(|inst| inst.plugin_id == starter.plugin_id)
            })
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

    /// 执行一批命令，**合成一个撤销步骤**。
    ///
    /// 用于「一次改很多东西」的场景——例如切换预设会写回上百个参数：逐条 [`Self::execute`]
    /// 会让用户要按上百次撤销才能回到切换前。
    pub fn execute_all(&mut self, commands: Vec<Command>, actor: Actor) -> Vec<Event> {
        let mut events = Vec::new();
        for command in commands {
            let produced = self.translate(&command, &actor);
            for ev in &produced {
                apply(&mut self.project, ev);
            }
            events.extend(produced);
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
        self.project = self.initial.clone();
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
            Command::SetTimeSignature { num, den } => {
                let (num, den) = crate::model::Transport::normalize_time_sig(*num, *den);
                vec![Event::TimeSignatureChanged { actor: actor.clone(), num, den }]
            }
            Command::SetKeyframe { beat, tempo, num, den } => vec![Event::KeyframeSet {
                actor: actor.clone(),
                beat: *beat,
                tempo: if tempo.is_finite() { tempo.clamp(1.0, 1000.0) } else { 120.0 },
                num: *num,
                den: *den,
            }],
            Command::RemoveKeyframe { beat } => {
                vec![Event::KeyframeRemoved { actor: actor.clone(), beat: *beat }]
            }
            Command::AddTrack { name } => {
                let id = self.alloc_track_id();
                vec![Event::TrackAdded { actor: actor.clone(), id, name: name.clone() }]
            }
            Command::RemoveTrack(track) => {
                vec![Event::TrackRemoved { actor: actor.clone(), id: *track }]
            }
            Command::RenameTrack { track, name } => {
                vec![Event::TrackRenamed { actor: actor.clone(), track: *track, name: name.clone() }]
            }
            Command::SetTrackInstrument { track, plugin_id } => {
                // 空 id 表示「卸载乐器」，而不是挂一个不存在的插件（后者会静默无声）。
                if plugin_id.is_empty() {
                    vec![Event::TrackInstrumentCleared { actor: actor.clone(), track: *track }]
                } else {
                    vec![Event::TrackInstrumentSet {
                        actor: actor.clone(),
                        track: *track,
                        plugin_id: plugin_id.clone(),
                    }]
                }
            }
            Command::AddClip { track, start, length } => {
                let clip = self.alloc_clip_id();
                vec![Event::ClipAdded {
                    actor: actor.clone(),
                    track: *track,
                    clip,
                    start: crate::model::Clip::normalize_start(*start),
                    length: crate::model::Clip::normalize_length(*length),
                }]
            }
            Command::RemoveClip { track, clip } => {
                vec![Event::ClipRemoved { actor: actor.clone(), track: *track, clip: *clip }]
            }
            Command::MoveClip { track, clip, start } => {
                vec![Event::ClipMoved {
                    actor: actor.clone(),
                    track: *track,
                    clip: *clip,
                    start: crate::model::Clip::normalize_start(*start),
                }]
            }
            Command::MoveClips { items } => {
                if items.is_empty() {
                    return vec![];
                }
                vec![Event::ClipsMoved {
                    actor: actor.clone(),
                    items: items
                        .iter()
                        .map(|i| crate::model::ClipPlacement {
                            track: i.track,
                            clip: i.clip,
                            to_track: i.to_track,
                            start: crate::model::Clip::normalize_start(i.start),
                        })
                        .collect(),
                }]
            }
            Command::ResizeClip { track, clip, length } => {
                vec![Event::ClipResized {
                    actor: actor.clone(),
                    track: *track,
                    clip: *clip,
                    length: crate::model::Clip::normalize_length(*length),
                }]
            }
            Command::AddNote { track, clip, note } => {
                vec![Event::NoteAdded {
                    actor: actor.clone(),
                    track: *track,
                    clip: *clip,
                    note: crate::model::Note::normalize(*note),
                }]
            }
            Command::SetNote { track, clip, index, note } => {
                vec![Event::NoteChanged {
                    actor: actor.clone(),
                    track: *track,
                    clip: *clip,
                    index: *index,
                    note: crate::model::Note::normalize(*note),
                }]
            }
            Command::SetNotes { track, clip, entries } => {
                if entries.is_empty() {
                    return vec![];
                }
                vec![Event::NotesChanged {
                    actor: actor.clone(),
                    track: *track,
                    clip: *clip,
                    entries: entries
                        .iter()
                        .map(|e| crate::model::NoteEdit {
                            index: e.index,
                            note: crate::model::Note::normalize(e.note),
                        })
                        .collect(),
                }]
            }
            Command::RemoveNote { track, clip, index } => {
                vec![Event::NoteRemoved {
                    actor: actor.clone(),
                    track: *track,
                    clip: *clip,
                    index: *index,
                }]
            }
            Command::RemoveNotes { track, clip, indices } => {
                if indices.is_empty() {
                    return vec![];
                }
                vec![Event::NotesRemoved {
                    actor: actor.clone(),
                    track: *track,
                    clip: *clip,
                    indices: indices.clone(),
                }]
            }
            Command::RemoveClips { items } => {
                if items.is_empty() {
                    return vec![];
                }
                vec![Event::ClipsRemoved { actor: actor.clone(), items: items.clone() }]
            }
            Command::SetGain { track, gain_db } => {
                vec![Event::GainChanged {
                    actor: actor.clone(),
                    track: *track,
                    gain_db: crate::model::MixerChannel::normalize_gain_db(*gain_db),
                }]
            }
            Command::SetPan { track, pan } => {
                vec![Event::PanChanged {
                    actor: actor.clone(),
                    track: *track,
                    pan: crate::model::MixerChannel::normalize_pan(*pan),
                }]
            }
            Command::SetMute { track, mute } => {
                vec![Event::MuteChanged { actor: actor.clone(), track: *track, mute: *mute }]
            }
            Command::SetSolo { track, solo } => {
                vec![Event::SoloChanged { actor: actor.clone(), track: *track, solo: *solo }]
            }
            Command::SetMasterGain(gain_db) => {
                vec![Event::MasterGainChanged {
                    actor: actor.clone(),
                    gain_db: crate::model::MixerChannel::normalize_gain_db(*gain_db),
                }]
            }
            Command::SetParam { track, param, value } => {
                // 非有限值会让插件内部出现 NaN；在命令层就丢掉，事件因此不会带毒。
                if !value.is_finite() {
                    return vec![];
                }
                vec![Event::ParamChanged {
                    actor: actor.clone(),
                    track: *track,
                    param: param.clone(),
                    value: *value,
                }]
            }
            Command::AddEffect { track, plugin_id } => {
                // 空 id 不构成一个效果器，安静忽略（与空乐器 id 的语义不同：那是「卸载」）。
                if plugin_id.is_empty() {
                    return vec![];
                }
                vec![Event::EffectAdded {
                    actor: actor.clone(),
                    track: *track,
                    plugin_id: plugin_id.clone(),
                }]
            }
            Command::RemoveEffect { track, index } => {
                vec![Event::EffectRemoved { actor: actor.clone(), track: *track, index: *index }]
            }
            Command::MoveEffect { track, from, to } => {
                vec![Event::EffectMoved {
                    actor: actor.clone(),
                    track: *track,
                    from: *from,
                    to: *to,
                }]
            }
            Command::SetEffectParam { track, index, param, value } => {
                // 非有限值会让插件内部出现 NaN，在命令层就丢掉。
                if !value.is_finite() {
                    return vec![];
                }
                vec![Event::EffectParamChanged {
                    actor: actor.clone(),
                    track: *track,
                    index: *index,
                    param: param.clone(),
                    value: *value,
                }]
            }
            Command::NewProject { name, track_names } => {
                // 先清空，再铺默认分轨：每一步都走正常事件，因此整条链天然可撤销。
                let mut events =
                    vec![Event::ProjectCleared { actor: actor.clone(), name: name.clone() }];
                for (index, starter) in crate::STARTER_TRACKS.iter().enumerate() {
                    let id = self.alloc_track_id();
                    // 名字由界面按语言给出（数量对不上就退回模板默认名，绝不 panic）。
                    let track_name = track_names
                        .as_ref()
                        .and_then(|names| names.get(index))
                        .cloned()
                        .unwrap_or_else(|| starter.name.to_string());
                    events.push(Event::TrackAdded {
                        actor: actor.clone(),
                        id,
                        name: track_name,
                    });
                    events.push(Event::TrackInstrumentSet {
                        actor: actor.clone(),
                        track: id,
                        plugin_id: starter.plugin_id.to_string(),
                    });
                }
                events
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
    fn set_time_signature_is_normalized_and_undoable() {
        let mut e = Engine::new("demo");
        e.execute(Command::SetTimeSignature { num: 7, den: 8 }, Actor::Human);
        assert_eq!(e.project().transport.time_sig_num, 7);
        assert_eq!(e.project().transport.time_sig_den, 8);

        // 非法分母被吸附、越界分子被夹紧。
        e.execute(Command::SetTimeSignature { num: 0, den: 9 }, Actor::Human);
        assert_eq!(e.project().transport.time_sig_num, 1);
        assert_eq!(e.project().transport.time_sig_den, 8);

        assert!(e.undo());
        assert_eq!(e.project().transport.time_sig_den, 8);
        assert_eq!(e.project().transport.time_sig_num, 7);
        assert!(e.undo());
        assert_eq!(e.project().transport.time_sig_num, 4);
        assert_eq!(e.project().transport.time_sig_den, 4);
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
    fn new_project_lays_down_the_starter_tracks_and_is_undoable() {
        let mut e = Engine::new("demo");
        e.execute(Command::AddTrack { name: "Bass".into() }, Actor::Human);
        assert_eq!(e.project().tracks.len(), 1);

        let events = e.execute(
            Command::NewProject { name: "x".into(), track_names: None },
            Actor::Human,
        );
        // 1 个 ProjectCleared + 每条分轨（TrackAdded + TrackInstrumentSet）
        assert_eq!(events.len(), 1 + crate::STARTER_TRACKS.len() * 2);
        assert!(matches!(events[0], crate::event::Event::ProjectCleared { .. }));
        assert_eq!(e.project().name, "x");

        let names: Vec<&str> = e.project().tracks.iter().map(|t| t.name.as_str()).collect();
        let expected: Vec<&str> = crate::STARTER_TRACKS.iter().map(|s| s.name).collect();
        assert_eq!(names, expected);
        // 每条默认分轨都要挂上乐器（而且是该功能位对应的预设），否则新工程一画音符就是哑的。
        for (track, starter) in e.project().tracks.iter().zip(crate::STARTER_TRACKS.iter()) {
            assert_eq!(
                track.instrument.as_ref().map(|i| i.plugin_id.as_str()),
                Some(starter.plugin_id),
                "轨道「{}」的默认音色不对",
                starter.name
            );
        }

        assert!(e.undo());
        assert_eq!(e.project().tracks.len(), 1);
        // 撤销 NewProject 必须恢复被清空前的工程名，而不是保留传入的新名。
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
    fn redo_restores_a_rebuilt_project_with_all_tracks() {
        let mut e = Engine::new("demo");
        e.execute(Command::AddTrack { name: "A".into() }, Actor::Human);
        e.execute(Command::AddTrack { name: "B".into() }, Actor::Human);
        e.execute(Command::NewProject { name: "gone".into(), track_names: None }, Actor::Human);
        assert_eq!(e.project().tracks.len(), crate::STARTER_TRACKS.len());

        assert!(e.undo());
        assert_eq!(e.project().tracks.len(), 2);
        assert!(e.redo());
        assert_eq!(e.project().tracks.len(), crate::STARTER_TRACKS.len());
        assert_eq!(e.project().name, "gone");
    }

    /// 撤销必须回到**引擎接手时那份工程**，而不是空工程。
    ///
    /// 曾经的实现从 `Project::new` 重放，于是启动时铺的默认分轨会在第一次撤销时凭空消失
    /// ——「撤销把我的工程清空了」。
    #[test]
    fn undo_never_loses_the_startup_tracks() {
        let mut e = Engine::new_project("demo");
        let tracks = e.project().tracks.len();
        assert!(tracks > 0, "启动就有默认分轨");

        e.execute(Command::AddTrack { name: "Extra".into() }, Actor::Human);
        assert_eq!(e.project().tracks.len(), tracks + 1);

        assert!(e.undo());
        assert_eq!(e.project().tracks.len(), tracks, "撤销不该丢掉启动时的分轨");
        assert_eq!(e.project().name, "demo");

        // 撤到底也不许清空。
        while e.undo() {}
        assert_eq!(e.project().tracks.len(), tracks);
    }

    /// 打开的文件同理：撤销后原来那条轨道（连同音符）必须还在。
    #[test]
    fn undo_of_a_loaded_project_keeps_its_content() {
        use crate::model::{Clip, Note, PluginInstance, Track};

        let mut loaded = Project::new("loaded");
        loaded.tracks.push(Track {
            id: TrackId(7),
            name: "Lead".into(),
            instrument: Some(PluginInstance {
                plugin_id: "builtin.synth".into(),
                params: Default::default(),
                state: vec![],
            }),
            effects: vec![],
            clips: vec![Clip {
                id: ClipId(3),
                start: 0.0,
                length: 4.0,
                notes: vec![Note { pitch: 60, start: 0.0, duration: 1.0, velocity: 100, volume: 127 }],
            }],
            mixer: Default::default(),
        });

        let mut e = Engine::from_project(loaded);
        e.execute(Command::AddTrack { name: "Extra".into() }, Actor::Human);
        assert_eq!(e.project().tracks.len(), 2);

        assert!(e.undo());
        assert_eq!(e.project().tracks.len(), 1, "打开的文件里的轨道不能被撤销掉");
        assert_eq!(e.project().tracks[0].name, "Lead");
        assert_eq!(e.project().tracks[0].clips[0].notes.len(), 1, "音符也要还在");
        assert_eq!(e.project().name, "loaded");
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

    /// 一批命令只占**一个**撤销步骤。
    ///
    /// 切换预设会一次写回上百个参数，如果逐条占一步，用户要按上百次撤销才回得去。
    #[test]
    fn execute_all_is_a_single_undo_step() {
        let mut e = Engine::new("demo");
        let ev = e.execute(Command::AddTrack { name: "Lead".into() }, Actor::Human);
        let track = match ev[0] {
            crate::event::Event::TrackAdded { id, .. } => id,
            _ => unreachable!(),
        };
        e.execute(
            Command::SetTrackInstrument { track, plugin_id: "builtin.synth".into() },
            Actor::Human,
        );
        let before = e.project().tracks[0].instrument.as_ref().unwrap().params.clone();

        e.execute_all(
            vec![
                Command::SetParam { track, param: "attack".into(), value: 0.5 },
                Command::SetParam { track, param: "release".into(), value: 0.9 },
            ],
            Actor::Human,
        );
        let after = &e.project().tracks[0].instrument.as_ref().unwrap().params;
        assert_eq!(after.get("attack"), Some(&0.5));
        assert_eq!(after.get("release"), Some(&0.9));

        // 撤掉这一批之后，轨道与乐器本身都还在（说明它确实只占了一步）。
        assert!(e.undo());
        let track = e.project().tracks.first().expect("撤销不该把轨道也带走");
        assert_eq!(
            track.instrument.as_ref().unwrap().params,
            before,
            "一次撤销要把这一批参数一起还原"
        );
    }

    #[test]
    fn from_empty_project_starts_ids_at_one() {
        let mut e = Engine::from_project(crate::model::Project::new("empty"));
        let ev = e.execute(Command::AddTrack { name: "A".into() }, Actor::Human);
        let id = match ev[0] { crate::event::Event::TrackAdded { id, .. } => id, _ => unreachable!() };
        assert_eq!(id.0, 1);
    }

    /// 建一条带片段与音符的轨道，返回 (track, clip)。
    fn seeded(e: &mut Engine) -> (TrackId, ClipId) {
        let ev = e.execute(Command::AddTrack { name: "Lead".into() }, Actor::Human);
        let track = match ev[0] { crate::event::Event::TrackAdded { id, .. } => id, _ => unreachable!() };
        let ev = e.execute(Command::AddClip { track, start: 0.0, length: 4.0 }, Actor::Human);
        let clip = match ev[0] { crate::event::Event::ClipAdded { clip, .. } => clip, _ => unreachable!() };
        (track, clip)
    }

    #[test]
    fn mixer_commands_are_normalized_and_undoable() {
        let mut e = Engine::new("demo");
        let (track, _) = seeded(&mut e);

        e.execute(Command::SetGain { track, gain_db: 100.0 }, Actor::Human);
        assert_eq!(e.project().track(track).unwrap().mixer.gain_db, crate::model::MixerChannel::MAX_GAIN_DB);
        e.execute(Command::SetPan { track, pan: f32::NAN }, Actor::Human);
        assert_eq!(e.project().track(track).unwrap().mixer.pan, 0.0);
        e.execute(Command::SetMute { track, mute: true }, Actor::Human);
        e.execute(Command::SetSolo { track, solo: true }, Actor::Human);
        e.execute(Command::SetMasterGain(-3.0), Actor::Human);
        assert!(e.project().track(track).unwrap().mixer.mute);
        assert!(e.project().track(track).unwrap().mixer.solo);
        assert_eq!(e.project().master.gain_db, -3.0);

        assert!(e.undo());
        assert_eq!(e.project().master.gain_db, 0.0);
        assert!(e.project().track(track).unwrap().mixer.solo);
    }

    #[test]
    fn clip_move_and_resize_commands_clamp_input() {
        let mut e = Engine::new("demo");
        let (track, clip) = seeded(&mut e);

        e.execute(Command::MoveClip { track, clip, start: -4.0 }, Actor::Human);
        assert_eq!(e.project().track(track).unwrap().clips[0].start, 0.0);
        e.execute(Command::ResizeClip { track, clip, length: f64::NAN }, Actor::Human);
        assert_eq!(
            e.project().track(track).unwrap().clips[0].length,
            crate::model::Clip::MIN_LENGTH_BEATS
        );
        e.execute(Command::MoveClip { track, clip, start: 6.0 }, Actor::Human);
        assert_eq!(e.project().track(track).unwrap().clips[0].start, 6.0);
    }

    #[test]
    fn note_commands_round_trip_through_history() {
        let mut e = Engine::new("demo");
        let (track, clip) = seeded(&mut e);
        for pitch in [60u8, 64] {
            e.execute(
                Command::AddNote { track, clip, note: crate::model::Note { pitch, start: 0.0, duration: 1.0, velocity: 100, volume: 127 } },
                Actor::Human,
            );
        }
        e.execute(
            Command::SetNote {
                track,
                clip,
                index: 0,
                note: crate::model::Note { pitch: 62, start: 1.5, duration: 0.5, velocity: 80, volume: 127 },
            },
            Actor::Human,
        );
        let notes = &e.project().track(track).unwrap().clips[0].notes;
        assert_eq!(notes[0].pitch, 62);
        assert_eq!(notes[0].start, 1.5);

        e.execute(Command::RemoveNote { track, clip, index: 1 }, Actor::Human);
        assert_eq!(e.project().track(track).unwrap().clips[0].notes.len(), 1);

        assert!(e.undo());
        assert_eq!(e.project().track(track).unwrap().clips[0].notes.len(), 2);
        assert!(e.undo());
        assert_eq!(e.project().track(track).unwrap().clips[0].notes[0].pitch, 60);
    }

    #[test]
    fn param_command_is_dropped_when_not_finite() {
        let mut e = Engine::new("demo");
        let (track, _) = seeded(&mut e);
        e.execute(Command::SetTrackInstrument { track, plugin_id: "builtin.synth".into() }, Actor::Human);
        let depth = e.undo_depth();

        let events = e.execute(Command::SetParam { track, param: "gain".into(), value: f32::NAN }, Actor::Human);
        assert!(events.is_empty(), "非有限参数不得产出事件");
        assert_eq!(e.undo_depth(), depth, "被拒绝的命令不应进入撤销历史");

        e.execute(Command::SetParam { track, param: "gain".into(), value: 0.25 }, Actor::Human);
        let params = &e.project().track(track).unwrap().instrument.as_ref().unwrap().params;
        assert_eq!(params.get("gain"), Some(&0.25));
    }

    /// 批量改写 / 批量删除必须各自只产生**一条**撤销记录，否则框选删 20 个音符要按 20 次撤销。
    #[test]
    fn batch_note_edits_and_removals_are_single_history_entries() {
        use crate::model::{Note, NoteEdit};
        let mut e = Engine::new("demo");
        let (track, clip) = seeded(&mut e);
        for pitch in [60u8, 64, 67, 71] {
            e.execute(
                Command::AddNote { track, clip, note: Note { pitch, start: 0.0, duration: 1.0, velocity: 100, volume: 127 } },
                Actor::Human,
            );
        }
        let depth = e.undo_depth();

        let entries: Vec<NoteEdit> = (0..4)
            .map(|i| NoteEdit {
                index: i,
                note: Note { pitch: 60 + i as u8, start: 0.0, duration: 1.0, velocity: 40, volume: 127 },
            })
            .collect();
        e.execute(Command::SetNotes { track, clip, entries }, Actor::Human);
        assert_eq!(e.undo_depth(), depth + 1, "批量改写应只记一条");
        assert!(e.project().track(track).unwrap().clips[0].notes.iter().all(|n| n.velocity == 40));

        // 乱序 + 重复 + 越界的序号：只应删掉 0 与 3 两个。
        e.execute(Command::RemoveNotes { track, clip, indices: vec![3, 0, 0, 99] }, Actor::Human);
        assert_eq!(e.undo_depth(), depth + 2);
        let pitches: Vec<u8> = e.project().track(track).unwrap().clips[0].notes.iter().map(|n| n.pitch).collect();
        assert_eq!(pitches, vec![61, 62], "倒序删除 + 去重 + 越界保护");

        assert!(e.undo());
        assert_eq!(e.project().track(track).unwrap().clips[0].notes.len(), 4);
        assert!(e.undo());
        assert!(e.project().track(track).unwrap().clips[0].notes.iter().all(|n| n.velocity == 100));
    }

    #[test]
    fn empty_batch_commands_do_not_touch_history() {
        use crate::model::NoteEdit;
        let mut e = Engine::new("demo");
        let (track, clip) = seeded(&mut e);
        let depth = e.undo_depth();
        assert!(e.execute(Command::SetNotes { track, clip, entries: vec![] }, Actor::Human).is_empty());
        assert!(e.execute(Command::RemoveNotes { track, clip, indices: vec![] }, Actor::Human).is_empty());
        assert!(e.execute(Command::RemoveClips { items: vec![] }, Actor::Human).is_empty());
        assert_eq!(e.undo_depth(), depth);
        // 仅仅是构造一下类型，确保它在公开 API 上可用。
        let _ = NoteEdit { index: 0, note: crate::model::Note { pitch: 60, start: 0.0, duration: 1.0, velocity: 100, volume: 127 } };
    }

    #[test]
    fn batch_clip_removal_spans_tracks_in_one_entry() {
        use crate::model::ClipRef;
        let mut e = Engine::new("demo");
        let (t1, c1) = seeded(&mut e);
        let ev = e.execute(Command::AddTrack { name: "B".into() }, Actor::Human);
        let t2 = match ev[0] { crate::event::Event::TrackAdded { id, .. } => id, _ => unreachable!() };
        let ev = e.execute(Command::AddClip { track: t2, start: 0.0, length: 2.0 }, Actor::Human);
        let c2 = match ev[0] { crate::event::Event::ClipAdded { clip, .. } => clip, _ => unreachable!() };

        let depth = e.undo_depth();
        e.execute(
            Command::RemoveClips { items: vec![ClipRef { track: t1, clip: c1 }, ClipRef { track: t2, clip: c2 }] },
            Actor::Human,
        );
        assert_eq!(e.undo_depth(), depth + 1, "跨轨道批量删除应只记一条");
        assert!(e.project().track(t1).unwrap().clips.is_empty());
        assert!(e.project().track(t2).unwrap().clips.is_empty());

        assert!(e.undo());
        assert_eq!(e.project().track(t1).unwrap().clips.len(), 1);
        assert_eq!(e.project().track(t2).unwrap().clips.len(), 1);
    }

    #[test]
    fn clips_move_across_tracks_and_keep_data() {
        use crate::model::ClipPlacement;
        let mut e = Engine::new("demo");
        let (t1, c1) = seeded(&mut e);
        let ev = e.execute(Command::AddTrack { name: "B".into() }, Actor::Human);
        let t2 = match ev[0] { crate::event::Event::TrackAdded { id, .. } => id, _ => unreachable!() };

        e.execute(
            Command::MoveClips {
                items: vec![ClipPlacement { track: t1, clip: c1, to_track: t2, start: 4.0 }],
            },
            Actor::Human,
        );
        assert!(e.project().track(t1).unwrap().clips.is_empty(), "原轨道必须腾空");
        let moved = &e.project().track(t2).unwrap().clips;
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].id, c1, "跨轨移动不得换 id");
        assert_eq!(moved[0].start, 4.0);

        assert!(e.undo());
        assert_eq!(e.project().track(t1).unwrap().clips.len(), 1);
        assert!(e.project().track(t2).unwrap().clips.is_empty());
    }

    #[test]
    fn moving_a_clip_to_a_missing_track_keeps_it_where_it_was() {
        use crate::model::{ClipPlacement, TrackId};
        let mut e = Engine::new("demo");
        let (t1, c1) = seeded(&mut e);
        e.execute(
            Command::MoveClips {
                items: vec![ClipPlacement { track: t1, clip: c1, to_track: TrackId(999), start: 8.0 }],
            },
            Actor::Human,
        );
        let clips = &e.project().track(t1).unwrap().clips;
        assert_eq!(clips.len(), 1, "目标轨道不存在时不得丢片段");
        assert_eq!(clips[0].start, 0.0, "也不得改起点");
    }

    #[test]
    fn empty_plugin_id_clears_the_instrument() {        let mut e = Engine::new("demo");
        let (track, _) = seeded(&mut e);
        e.execute(Command::SetTrackInstrument { track, plugin_id: "builtin.synth".into() }, Actor::Human);
        assert!(e.project().track(track).unwrap().instrument.is_some());

        // 空 id 表示卸载：不能留下一个指向不存在插件的实例（那会让轨道静默无声）。
        let events = e.execute(Command::SetTrackInstrument { track, plugin_id: String::new() }, Actor::Human);
        assert!(matches!(events[0], crate::event::Event::TrackInstrumentCleared { .. }));
        assert!(e.project().track(track).unwrap().instrument.is_none());

        assert!(e.undo());
        assert!(e.project().track(track).unwrap().instrument.is_some());
    }

    #[test]
    fn rename_track_keeps_identity_and_survives_undo() {        let mut e = Engine::new("demo");
        let (track, _) = seeded(&mut e);
        e.execute(Command::RenameTrack { track, name: "和弦".into() }, Actor::Human);
        assert_eq!(e.project().track(track).unwrap().name, "和弦");
        // 重命名不得改变 id（否则选中态与片段都会错位）。
        assert!(e.project().track(track).is_some());
        assert!(e.undo());
        assert_eq!(e.project().track(track).unwrap().name, "Lead");
    }

    /// 默认分轨的顺序是界面自上而下看到的顺序，改它要连测试一起改。
    #[test]
    fn starter_tracks_run_lead_chords_bass_drums_pad() {
        let e = Engine::new_project("demo");
        let ids: Vec<&str> = e
            .project()
            .tracks
            .iter()
            .map(|t| t.instrument.as_ref().unwrap().plugin_id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "builtin.synth.lead",
                "builtin.synth.pad",
                "builtin.synth.bass",
                "builtin.synth.pluck",
                "builtin.synth.pad",
            ],
            "默认分轨按 主旋律→和弦→低音→鼓组→铺底 排列"
        );
    }

    /// 新建工程可以带一批**按语言生成**的分轨名。
    #[test]
    fn new_project_uses_the_given_track_names() {
        let mut e = Engine::new("demo");
        let names = ["Lead", "Chords", "Bass", "Drums", "Pad"].map(String::from);
        e.execute(
            Command::NewProject { name: "x".into(), track_names: Some(names.to_vec()) },
            Actor::Human,
        );
        let got: Vec<&str> = e.project().tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(got, names);

        // 名字数量对不上时退回模板默认名，而不是少几条轨道或 panic。
        let mut e = Engine::new("demo");
        e.execute(
            Command::NewProject { name: "x".into(), track_names: Some(vec!["Solo".into()]) },
            Actor::Human,
        );
        assert_eq!(e.project().tracks.len(), crate::STARTER_TRACKS.len());
        assert_eq!(e.project().tracks[0].name, "Solo");
        assert_eq!(e.project().tracks[1].name, crate::STARTER_TRACKS[1].name);
    }

    /// 启动时的初始工程只有「原封不动的模板」才允许按语言改名。
    #[test]
    fn localize_starter_tracks_only_touches_a_pristine_project() {
        let names = ["Lead", "Chords", "Bass", "Drums", "Pad"].map(String::from);

        let mut fresh = Engine::new_project("untitled");
        assert!(fresh.localize_starter_tracks(&names));
        let got: Vec<&str> = fresh.project().tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(got, names);
        // 改名属于「初始状态」：撤销不该退回旧语言。
        assert!(!fresh.undo(), "改名不进历史");

        // 用户动过一次（这里：加了一个片段）之后就不该再被改名。
        let mut edited = Engine::new_project("untitled");
        let track = edited.project().tracks[0].id;
        edited.execute(Command::AddClip { track, start: 0.0, length: 1.0 }, Actor::Human);
        assert!(!edited.localize_starter_tracks(&names));
        assert_eq!(edited.project().tracks[0].name, crate::STARTER_TRACKS[0].name);
    }

    #[test]
    fn keyframe_commands_are_normalized_and_undoable() {
        let mut e = Engine::new("t");
        e.execute(Command::SetKeyframe { beat: 4.0, tempo: 90.0, num: 3, den: 4 }, Actor::Human);
        assert_eq!(e.project().transport.keyframes.len(), 1);
        assert_eq!(e.project().transport.keyframes[0].tempo, 90.0);
        assert_eq!(
            (e.project().transport.keyframes[0].time_sig_num, e.project().transport.keyframes[0].time_sig_den),
            (3, 4)
        );

        // beat<=0 → 安静忽略（列表不变），但命令仍算一次撤销步骤。
        e.execute(Command::SetKeyframe { beat: 0.0, tempo: 200.0, num: 4, den: 4 }, Actor::Human);
        assert_eq!(e.project().transport.keyframes.len(), 1);

        assert!(e.undo()); // 撤销 beat=0 那次
        assert!(e.undo()); // 撤销 beat=4 那次
        assert_eq!(e.project().transport.keyframes.len(), 0);

        e.execute(Command::SetKeyframe { beat: 4.0, tempo: 90.0, num: 4, den: 4 }, Actor::Human);
        e.execute(Command::RemoveKeyframe { beat: 4.0 }, Actor::Human);
        assert_eq!(e.project().transport.keyframes.len(), 0);
    }

    #[test]
    fn keyframe_events_are_structural() {
        assert!(!Event::KeyframeSet { actor: Actor::Human, beat: 1.0, tempo: 120.0, num: 4, den: 4 }.is_mix_only());
        assert!(!Event::KeyframeRemoved { actor: Actor::Human, beat: 1.0 }.is_mix_only());
    }
}
