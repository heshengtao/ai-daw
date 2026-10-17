use crate::apply::apply;
use crate::command::Command;
use crate::event::{Actor, Event};
use crate::model::{ClipId, Project, TrackId};
use crate::tempo::TempoMap;

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
    /// 最近一次「保存到文件」时的快照。当前工程与它不同即表示有未保存的改动（dirty）。
    ///
    /// 用整份快照而不是「有没有执行过命令」来判断：撤销回到保存点、改回原值等
    /// 都会让 dirty 自动变回 false，不必额外维护。
    saved: Project,
}

impl Engine {
    /// 创建引擎：空工程（默认传输设置）、空历史，id 计数器从 1 开始。
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        let project = Project::new(name.clone());
        Engine {
            initial: project.clone(),
            project: project.clone(),
            history: vec![],
            redo: vec![],
            next_track_id: 1,
            next_clip_id: 1,
            initial_name: name,
            saved: project,
        }
    }

    /// 从已存在的工程构建引擎，并把 id 计数器恢复到不冲突的高水位。
    ///
    /// 顺手给旧工程里没有 id 的音符补号（按片段内顺序，可复现）——从这一刻起，
    /// 批量工具才能用 id 稳定定位。
    pub fn from_project(mut project: Project) -> Self {
        project.ensure_note_ids();
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
        let saved = project.clone();
        Engine { initial, project, history: vec![], redo: vec![], next_track_id, next_clip_id, initial_name, saved }
    }

    /// 从「工作副本 + 保存基线」构建引擎（会话缓存恢复用）。
    ///
    /// 与 [`Self::from_project`] 的唯一区别是基线可以不同于当前工程：若缓存里记录着
    /// 未保存的改动，恢复后 `is_dirty()` 依旧为真。
    pub fn restore(project: Project, saved: Project) -> Self {
        let mut engine = Engine::from_project(project);
        engine.saved = saved;
        engine
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
        // 刚铺好的模板没有任何未保存的改动。
        engine.saved = engine.project.clone();
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
        // 按语言改名属于「初始状态」，不该把刚启动的工程标成 dirty。
        self.saved = self.project.clone();
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

    /// 是否有未保存的改动。
    pub fn is_dirty(&self) -> bool {
        self.project != self.saved
    }

    /// 最近一次保存/打开/新建时的工程快照（会话缓存用它记录「保存基线」）。
    pub fn saved(&self) -> &Project {
        &self.saved
    }

    /// 按「轨道 + 片段」取片段（只读）。
    fn find_clip(&self, track: TrackId, clip: ClipId) -> Option<&crate::model::Clip> {
        self.project.track(track)?.clips.iter().find(|c| c.id == clip)
    }

    /// 把当前工程记为「已保存」的基线（保存成功后调用）。
    pub fn mark_saved(&mut self) {
        self.saved = self.project.clone();
    }

    /// 记下轨道乐器实例的**二进制状态**（不进撤销历史、不广播事件）。
    ///
    /// 这不是用户编辑，而是「插件内部补丁」的缓存：外部插件（VST3 / CLAP）的旋钮与预设只
    /// 活在实例里，重建播放器就会丢。宿主在重建前把它抢救进工程，重建后由播放器
    /// `load_state` 还原，于是「改效果链 / 移动片段」这类**结构变更不会重置乐器配置**。
    ///
    /// 同步写进 `saved` 基线：否则光是开关一次插件窗口就会把工程标成「未保存」。
    ///
    /// `plugin_id` 是这份状态**来自哪件乐器**。必须核对：换乐器是结构性变更，调用方手里
    /// 可能还攥着**上一件乐器**的实例句柄（句柄要等这一轮重建之后才清理），而 `apply` 已经
    /// 把轨道换成新乐器了。不核对就会把旧乐器的二进制写进新乐器——把 VST3 鼓机的状态塞给
    /// 内置合成器，后者按 f32 解出来的 `gain` 是 1e-8，于是**整轨无声**，而且状态进工程 /
    /// 会话缓存，重启也不好。用户看到的就是「点开鼓组插件、点几个音色，再回内置合成器就没声了，
    /// 切个预设又恢复」（切预设写的是 `params`，把被读坏的 gain 盖了回去）。
    pub fn set_instrument_state(&mut self, track: TrackId, plugin_id: &str, state: Vec<u8>) {
        let Some(t) = self.project.tracks.iter_mut().find(|t| t.id == track) else {
            return;
        };
        let Some(inst) = t.instrument.as_mut() else { return };
        // 乐器已经换掉了：这份状态属于上一件乐器，写进来只会毒化新乐器。
        if inst.plugin_id != plugin_id {
            return;
        }
        if inst.state == state {
            return;
        }
        inst.state = state.clone();
        if let Some(saved) = self.saved.tracks.iter_mut().find(|t| t.id == track) {
            if let Some(inst) = saved.instrument.as_mut() {
                inst.state = state;
            }
        }
    }

    /// 记下轨道**效果器**实例的二进制状态（不进撤销历史、不广播事件）。
    ///
    /// 与 [`Self::set_instrument_state`] 同理：外部效果器的旋钮与预设只活在实例里，重建播放器
    /// 就会丢。宿主在重建前按**效果链下标** `index` 把状态抢救进工程，重建后由播放器
    /// `load_state` 还原。同步写进 `saved` 基线，避免开关一次界面就把工程标成「未保存」。
    pub fn set_effect_state(&mut self, track: TrackId, index: usize, state: Vec<u8>) {
        let Some(t) = self.project.tracks.iter_mut().find(|t| t.id == track) else {
            return;
        };
        let Some(fx) = t.effects.get_mut(index) else { return };
        if fx.state == state {
            return;
        }
        fx.state = state.clone();
        if let Some(saved) = self.saved.tracks.iter_mut().find(|t| t.id == track) {
            if let Some(fx) = saved.effects.get_mut(index) {
                fx.state = state;
            }
        }
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
                vec![Event::TrackAdded { actor: actor.clone(), id, name: name.clone(), at: None }]
            }
            Command::AddNoteTrack { name, start, length, notes, at } => {
                // 轨道 / 片段的 id 在同一次 translate 里分配，整条链只占一个撤销步骤。
                let track = self.alloc_track_id();
                let clip = self.alloc_clip_id();
                let mut events = vec![Event::TrackAdded {
                    actor: actor.clone(),
                    id: track,
                    name: name.clone(),
                    at: *at,
                }];
                events.push(Event::ClipAdded {
                    actor: actor.clone(),
                    track,
                    clip,
                    start: crate::model::Clip::normalize_start(*start),
                    length: crate::model::Clip::normalize_length(*length),
                });
                for note in notes {
                    events.push(Event::NoteAdded {
                        actor: actor.clone(),
                        track,
                        clip,
                        note: crate::model::Note::normalize(*note),
                    });
                }
                events
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
            Command::AddAudioClip { track, start, length, source } => {
                let clip = self.alloc_clip_id();
                vec![Event::AudioClipAdded {
                    actor: actor.clone(),
                    track: *track,
                    clip,
                    start: crate::model::Clip::normalize_start(*start),
                    length: crate::model::Clip::normalize_length(*length),
                    source: source.clone(),
                }]
            }
            Command::AddAudioTrack { name, start, length, source, at } => {
                // 与 `AddNoteTrack` 同理：轨道 / 片段的 id 在同一次 translate 里分配，
                // 整条链只占一个撤销步骤，调用方不必先建轨道再回读 id。
                let track = self.alloc_track_id();
                let clip = self.alloc_clip_id();
                vec![
                    Event::TrackAdded { actor: actor.clone(), id: track, name: name.clone(), at: *at },
                    Event::AudioClipAdded {
                        actor: actor.clone(),
                        track,
                        clip,
                        start: crate::model::Clip::normalize_start(*start),
                        length: crate::model::Clip::normalize_length(*length),
                        source: source.clone(),
                    },
                ]
            }
            Command::SetClipAudio { track, clip, audio } => {
                vec![Event::ClipAudioChanged {
                    actor: actor.clone(),
                    track: *track,
                    clip: *clip,
                    audio: audio.clone(),
                }]
            }
            Command::SetClipRate { track, clip, rate, pitch_semitones } => {
                // 只对**音频片段**有效；MIDI 片段 / 不存在的片段安静忽略。
                let Some(existing) = self
                    .project
                    .track(*track)
                    .and_then(|t| t.clips.iter().find(|c| c.id == *clip))
                else {
                    return vec![];
                };
                if existing.audio.is_none() {
                    return vec![];
                }
                vec![Event::ClipRateChanged {
                    actor: actor.clone(),
                    track: *track,
                    clip: *clip,
                    rate: crate::model::AudioSource::normalize_rate(*rate),
                    pitch_semitones: crate::model::AudioSource::normalize_pitch(*pitch_semitones),
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
            Command::SplitClip { track, clip, beat } => {
                let Some(existing) = self
                    .project
                    .track(*track)
                    .and_then(|t| t.clips.iter().find(|c| c.id == *clip))
                else {
                    return vec![];
                };
                let (start, len) = (existing.start, existing.length);
                let at = *beat;
                // 必须切在片段内部，且两侧都留得下一段合法长度。
                let eps = 1e-6;
                if !(at.is_finite() && at > start + eps && at < start + len - eps) {
                    return vec![];
                }
                let rel = at - start;

                // 音符：完全在左 / 完全在右 / 跨过切点（切成两半）。
                let mut left_notes = Vec::new();
                let mut right_notes = Vec::new();
                for n in &existing.notes {
                    let n_end = n.start + n.duration;
                    if n_end <= rel + eps {
                        left_notes.push(*n);
                    } else if n.start >= rel - eps {
                        right_notes.push(crate::model::Note { start: n.start - rel, ..*n });
                    } else {
                        left_notes.push(crate::model::Note { duration: rel - n.start, ..*n });
                        right_notes.push(crate::model::Note { start: 0.0, duration: n_end - rel, ..*n });
                    }
                }

                // 音频：按**秒**比例切素材窗口。采样率在比值里约掉，所以只需要速度表。
                let mut left_audio = None;
                let mut right_audio = None;
                if let Some(src) = &existing.audio {
                    // 没有窗口长度信息（旧工程）→ 无从确定左半段到哪停，安静忽略。
                    let Some(window) = src.frames else { return vec![] };
                    if window < 2 {
                        return vec![];
                    }
                    let map = TempoMap::from_transport(&self.project.transport);
                    let span = map.beats_to_seconds(start + len) - map.beats_to_seconds(start);
                    let frac = if span > 0.0 {
                        (map.beats_to_seconds(at) - map.beats_to_seconds(start)) / span
                    } else {
                        0.5
                    };
                    let left_frames = ((window as f64) * frac.clamp(0.0, 1.0)).round() as u64;
                    let left_frames = left_frames.clamp(1, window - 1);
                    left_audio = Some(crate::model::AudioSource {
                        path: src.path.clone(),
                        offset_frames: src.offset_frames,
                        frames: Some(left_frames),
                        rate: src.rate,
                        pitch_semitones: src.pitch_semitones,
                    });
                    right_audio = Some(crate::model::AudioSource {
                        path: src.path.clone(),
                        offset_frames: src.offset_frames + left_frames,
                        frames: Some(window - left_frames),
                        rate: src.rate,
                        pitch_semitones: src.pitch_semitones,
                    });
                }

                let new_clip = self.alloc_clip_id();
                vec![Event::ClipSplit {
                    actor: actor.clone(),
                    track: *track,
                    clip: *clip,
                    new_clip,
                    at_beat: at,
                    left_notes,
                    right_notes,
                    left_audio,
                    right_audio,
                }]
            }
            Command::AddNote { track, clip, note } => {
                let Some(existing) = self.find_clip(*track, *clip) else { return vec![] };
                let notes = assign_ids(&existing.notes, std::slice::from_ref(note));
                vec![Event::NoteAdded {
                    actor: actor.clone(),
                    track: *track,
                    clip: *clip,
                    note: notes[0],
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
            Command::SetSends { track, sends } => {
                // 自发送无意义，在命令层就丢掉（否则会给总线图引入一个必然被忽略的环）。
                let sends: Vec<crate::model::Send> = sends
                    .iter()
                    .filter(|s| s.target != *track)
                    .map(|s| s.normalize())
                    .collect();
                vec![Event::TrackSendsChanged { actor: actor.clone(), track: *track, sends }]
            }
            Command::SetAutomation { track, param, points } => {
                let param = crate::model::AutomationLane::normalize_param(param);
                if param.is_empty() {
                    return vec![];
                }
                // 过滤非有限值、夹到非负拍、按拍排序；点数封顶（挡住天文数字的载荷）。
                const MAX_POINTS: usize = 4096;
                let mut points: Vec<crate::model::AutomationPoint> = points
                    .iter()
                    .copied()
                    .filter(|p| p.beat.is_finite() && p.value.is_finite())
                    .map(|p| crate::model::AutomationPoint { beat: p.beat.max(0.0), value: p.value })
                    .collect();
                points.sort_by(|a, b| a.beat.partial_cmp(&b.beat).unwrap_or(std::cmp::Ordering::Equal));
                points.truncate(MAX_POINTS);
                vec![Event::AutomationChanged { actor: actor.clone(), track: *track, param, points }]
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
            Command::RenameProject { name } => {
                vec![Event::ProjectRenamed { actor: actor.clone(), name: name.clone() }]
            }
            // ---- 批量音符（编曲主力） ----
            Command::AddNotes { track, clip, notes } => {
                if notes.is_empty() {
                    return vec![];
                }
                let Some(existing) = self.find_clip(*track, *clip) else { return vec![] };
                let notes = assign_ids(&existing.notes, notes);
                vec![Event::NotesAdded {
                    actor: actor.clone(),
                    track: *track,
                    clip: *clip,
                    notes,
                }]
            }
            Command::ReplaceNotes { track, clip, notes } => {
                if self.find_clip(*track, *clip).is_none() {
                    return vec![];
                }
                // 整体替换：旧音符全丢，所以 id 从 1 重新发（除非调用方明确给了非零 id）。
                let notes = assign_ids(&[], notes);
                vec![Event::NotesReplaced {
                    actor: actor.clone(),
                    track: *track,
                    clip: *clip,
                    notes,
                }]
            }
            Command::UpdateNotes { track, clip, updates } => {
                if updates.is_empty() {
                    return vec![];
                }
                let Some(existing) = self.find_clip(*track, *clip) else { return vec![] };
                let transform = crate::compose::update_by_id(existing, updates);
                changed_notes(actor, *track, *clip, transform)
            }
            Command::RemoveNotesById { track, clip, ids } => {
                let Some(existing) = self.find_clip(*track, *clip) else { return vec![] };
                let mut indices: Vec<usize> =
                    ids.iter().filter_map(|id| existing.note_index(*id)).collect();
                indices.sort_unstable();
                indices.dedup();
                if indices.is_empty() {
                    return vec![];
                }
                vec![Event::NotesRemoved {
                    actor: actor.clone(),
                    track: *track,
                    clip: *clip,
                    indices,
                }]
            }
            Command::TransposeNotes { track, clip, semitones, range } => {
                let Some(existing) = self.find_clip(*track, *clip) else { return vec![] };
                let transform = crate::compose::transpose(existing, *semitones, *range);
                changed_notes(actor, *track, *clip, transform)
            }
            Command::SetNoteVelocity { track, clip, shape, range } => {
                let Some(existing) = self.find_clip(*track, *clip) else { return vec![] };
                let transform = crate::compose::set_velocity(existing, *shape, *range);
                changed_notes(actor, *track, *clip, transform)
            }
            Command::QuantizeNotes { track, clip, grid, strength, range } => {
                let Some(existing) = self.find_clip(*track, *clip) else { return vec![] };
                let transform = crate::compose::quantize(existing, *grid, *strength, *range);
                changed_notes(actor, *track, *clip, transform)
            }
            Command::ApplySwing { track, clip, subdivision, ratio, strength, range } => {
                let Some(existing) = self.find_clip(*track, *clip) else { return vec![] };
                let transform =
                    crate::compose::swing(existing, *subdivision, *ratio, *strength, *range);
                changed_notes(actor, *track, *clip, transform)
            }
            Command::HumanizeNotes { track, clip, timing, velocity, range, seed } => {
                let Some(existing) = self.find_clip(*track, *clip) else { return vec![] };
                let transform =
                    crate::compose::humanize(existing, *timing, *velocity, *range, *seed);
                changed_notes(actor, *track, *clip, transform)
            }
            Command::FitNotesToRange { track, clip, min_pitch, max_pitch, mode, range } => {
                let Some(existing) = self.find_clip(*track, *clip) else { return vec![] };
                let report =
                    crate::compose::fit_to_range(existing, *min_pitch, *max_pitch, *mode, *range);
                changed_notes(actor, *track, *clip, report.transform)
            }

            // ---- 片段编辑（复制 / 合并） ----
            Command::DuplicateClip { track, clip, to_track, start } => {
                let Some((audio, notes, length, src_start)) = self
                    .find_clip(*track, *clip)
                    .map(|c| (c.audio.clone(), c.notes.clone(), c.length, c.start))
                else {
                    return vec![];
                };
                let dest = to_track.unwrap_or(*track);
                if self.project.track(dest).is_none() {
                    return vec![];
                }
                let start = crate::model::Clip::normalize_start(start.unwrap_or(src_start + length));
                let new_clip = self.alloc_clip_id();
                match audio {
                    Some(source) => vec![Event::AudioClipAdded {
                        actor: actor.clone(),
                        track: dest,
                        clip: new_clip,
                        start,
                        length,
                        source,
                    }],
                    None => {
                        let mut events = vec![Event::ClipAdded {
                            actor: actor.clone(),
                            track: dest,
                            clip: new_clip,
                            start,
                            length,
                        }];
                        if !notes.is_empty() {
                            events.push(Event::NotesAdded {
                                actor: actor.clone(),
                                track: dest,
                                clip: new_clip,
                                notes: assign_ids(&[], &notes),
                            });
                        }
                        events
                    }
                }
            }
            Command::MergeClips { track, clips } => {
                if clips.len() < 2 {
                    return vec![];
                }
                // 收集要合并的片段（按起点排序）；缺一个、或其中有音频片段 → 整体拒绝，
                // 免得「合并了一半」这种半成品状态进历史。
                let Some(t) = self.project.track(*track) else { return vec![] };
                let mut picked: Vec<crate::model::Clip> = Vec::with_capacity(clips.len());
                for id in clips {
                    let Some(c) = t.clips.iter().find(|c| c.id == *id) else { return vec![] };
                    if c.is_audio() {
                        return vec![];
                    }
                    picked.push(c.clone());
                }
                picked.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
                let start = picked[0].start;
                let end = picked
                    .iter()
                    .map(|c| c.start + c.length)
                    .fold(f64::NEG_INFINITY, f64::max);
                // 音符改成相对**合并后**片段起点的坐标。
                let mut merged: Vec<crate::model::Note> = Vec::new();
                for c in &picked {
                    let offset = c.start - start;
                    for n in &c.notes {
                        merged.push(crate::model::Note { start: n.start + offset, ..*n });
                    }
                }
                let target = picked[0].id;
                let mut events = vec![
                    Event::ClipMoved { actor: actor.clone(), track: *track, clip: target, start },
                    Event::ClipResized {
                        actor: actor.clone(),
                        track: *track,
                        clip: target,
                        length: end - start,
                    },
                    Event::NotesReplaced {
                        actor: actor.clone(),
                        track: *track,
                        clip: target,
                        notes: assign_ids(&[], &merged),
                    },
                ];
                events.push(Event::ClipsRemoved {
                    actor: actor.clone(),
                    items: picked[1..]
                        .iter()
                        .map(|c| crate::model::ClipRef { track: *track, clip: c.id })
                        .collect(),
                });
                events
            }

            // ---- 编曲元数据 ----
            Command::SetProjectHarmony { bars } => {
                vec![Event::ProjectHarmonyChanged { actor: actor.clone(), bars: bars.clone() }]
            }
            Command::SetProjectSections { sections } => {
                vec![Event::ProjectSectionsChanged {
                    actor: actor.clone(),
                    sections: sections.clone(),
                }]
            }
            Command::SetTrackMeta { track, role, pitch_range } => vec![Event::TrackMetaChanged {
                actor: actor.clone(),
                track: *track,
                role: role.clone(),
                pitch_range: *pitch_range,
            }],

            Command::NewProject { name, track_names } => {                // 先清空，再铺默认分轨：每一步都走正常事件，因此整条链天然可撤销。
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
                        at: None,
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

/// 给一批音符发号：保留调用方给出的非零且不重复的 id，其余按「当前最大值 + 1」顺延。
///
/// 发号在**命令层**做（而不是应用事件时），事件因此自带最终 id——撤销/重做重放同一串事件
/// 得到完全一样的 id，外部拿着旧 id 的操作在撤销后仍然指得准。
fn assign_ids(existing: &[crate::model::Note], notes: &[crate::model::Note]) -> Vec<crate::model::Note> {
    let mut next = existing.iter().map(|n| n.id).max().unwrap_or(0) + 1;
    let mut seen: std::collections::HashSet<u64> = existing.iter().map(|n| n.id).collect();
    notes
        .iter()
        .map(|note| {
            let mut note = crate::model::Note::normalize(*note);
            if note.id == 0 || !seen.insert(note.id) {
                note.id = next;
                next += 1;
                seen.insert(note.id);
            }
            note
        })
        .collect()
}

/// 把一次变换的结果包成事件；没改任何音符时返回空（不产生空撤销步骤）。
fn changed_notes(
    actor: &Actor,
    track: TrackId,
    clip: ClipId,
    transform: crate::compose::Transform,
) -> Vec<Event> {
    if transform.edits.is_empty() {
        return vec![];
    }
    vec![Event::NotesChanged { actor: actor.clone(), track, clip, entries: transform.edits }]
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
    fn add_audio_clip_is_normalized_and_undoable() {
        use crate::model::AudioSource;
        let mut e = Engine::new("demo");
        let ev = e.execute(Command::AddTrack { name: "Audio".into() }, Actor::Human);
        let track = match ev[0] { crate::event::Event::TrackAdded { id, .. } => id, _ => unreachable!() };
        let ev = e.execute(
            Command::AddAudioClip {
                track,
                start: -5.0,
                length: 1e9,
                source: AudioSource { path: "/tmp/a.wav".into(), offset_frames: 0, frames: None, rate: 1.0, pitch_semitones: 0.0 },
            },
            Actor::Human,
        );
        let clip = match ev[0] { crate::event::Event::AudioClipAdded { clip, .. } => clip, _ => unreachable!() };

        let stored = &e.project().track(track).unwrap().clips[0];
        assert!(stored.is_audio(), "存进去的必须是音频片段");
        assert_eq!(stored.id, clip);
        assert_eq!(stored.start, 0.0, "负起点必须被夹到 0");
        assert_eq!(stored.length, crate::model::Clip::MAX_LENGTH_BEATS, "超长必须被夹紧");

        assert!(e.undo());
        assert!(e.project().track(track).unwrap().clips.is_empty());
        assert!(e.redo());
        assert!(e.project().track(track).unwrap().clips[0].is_audio());
    }

    #[test]
    fn set_clip_audio_switches_between_midi_and_audio() {
        use crate::model::{AudioSource, Note};
        let mut e = Engine::new("demo");
        let (track, clip) = seeded(&mut e);
        e.execute(
            Command::AddNote {
                track,
                clip,
                note: Note { id: 0, pitch: 60, start: 0.0, duration: 1.0, velocity: 100, volume: 127 },
            },
            Actor::Human,
        );
        assert_eq!(e.project().track(track).unwrap().clips[0].notes.len(), 1);

        e.execute(
            Command::SetClipAudio {
                track,
                clip,
                audio: Some(AudioSource { path: "x.wav".into(), offset_frames: 0, frames: None, rate: 1.0, pitch_semitones: 0.0 }),
            },
            Actor::Human,
        );
        let c = &e.project().track(track).unwrap().clips[0];
        assert!(c.is_audio());
        assert!(c.notes.is_empty(), "转成音频片段必须清空音符");

        e.execute(Command::SetClipAudio { track, clip, audio: None }, Actor::Human);
        assert!(!e.project().track(track).unwrap().clips[0].is_audio(), "去掉素材应当变回 MIDI 片段");
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
            meta: None,
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
                notes: vec![Note { id: 0, pitch: 60, start: 0.0, duration: 1.0, velocity: 100, volume: 127 }],
                audio: None,
            }],
            mixer: Default::default(),
            sends: vec![],
            automation: vec![],
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

    /// **上一件乐器**的状态不能写进新乐器。
    ///
    /// 真实故障（用户可复现）：鼓组轨道上是 VST3 鼓机，点开它的界面点几个音色，然后把乐器
    /// 换成内置合成器 —— 宿主在重建前把鼓机的状态「抢救」进工程，但那时工程里已经是内置
    /// 合成器了。内置合成器把这段二进制当 f32 参数表读，`gain` 变成 1e-8 → **整轨无声**，
    /// 而且状态进了工程 / 会话缓存，重启也不好；切个预设（写 `params`）才会恢复。
    #[test]
    fn instrument_state_from_the_previous_plugin_is_ignored() {
        let mut e = Engine::new("demo");
        let ev = e.execute(Command::AddTrack { name: "Drums".into() }, Actor::Human);
        let track = match ev[0] {
            crate::event::Event::TrackAdded { id, .. } => id,
            _ => unreachable!(),
        };
        e.execute(
            Command::SetTrackInstrument { track, plugin_id: "vst3:drums".into() },
            Actor::Human,
        );
        // 换成内置合成器（结构性变更：旧乐器的句柄要等重建后才清理）。
        e.execute(
            Command::SetTrackInstrument { track, plugin_id: "builtin.synth".into() },
            Actor::Human,
        );

        // 旧乐器迟到的状态：必须被忽略，不能污染新乐器。
        e.set_instrument_state(track, "vst3:drums", vec![1, 2, 3, 4]);
        assert!(
            e.project().tracks[0].instrument.as_ref().unwrap().state.is_empty(),
            "上一件乐器的状态不能写进新乐器"
        );

        // 新乐器自己的状态照收。
        e.set_instrument_state(track, "builtin.synth", vec![9, 9, 9, 9]);
        assert_eq!(e.project().tracks[0].instrument.as_ref().unwrap().state, vec![9, 9, 9, 9]);
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
                Command::AddNote { track, clip, note: crate::model::Note { id: 0, pitch, start: 0.0, duration: 1.0, velocity: 100, volume: 127 } },
                Actor::Human,
            );
        }
        e.execute(
            Command::SetNote {
                track,
                clip,
                index: 0,
                note: crate::model::Note { id: 0, pitch: 62, start: 1.5, duration: 0.5, velocity: 80, volume: 127 },
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
                Command::AddNote { track, clip, note: Note { id: 0, pitch, start: 0.0, duration: 1.0, velocity: 100, volume: 127 } },
                Actor::Human,
            );
        }
        let depth = e.undo_depth();

        let entries: Vec<NoteEdit> = (0..4)
            .map(|i| NoteEdit {
                index: i,
                note: Note { id: 0, pitch: 60 + i as u8, start: 0.0, duration: 1.0, velocity: 40, volume: 127 },
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
        let _ = NoteEdit { index: 0, note: crate::model::Note { id: 0, pitch: 60, start: 0.0, duration: 1.0, velocity: 100, volume: 127 } };
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
                "builtin.synth.strings",
                "builtin.synth.bass",
                "builtin.synth.drums",
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

    #[test]
    fn rename_project_emits_event_and_is_undoable() {
        let mut e = Engine::new("demo");
        let events = e.execute(Command::RenameProject { name: "Song".into() }, Actor::Human);
        assert!(matches!(events.as_slice(), [Event::ProjectRenamed { name, .. }] if name == "Song"));
        assert_eq!(e.project().name, "Song");
        assert!(e.undo());
        assert_eq!(e.project().name, "demo");
    }

    #[test]
    fn dirty_tracks_the_saved_baseline() {
        let mut e = Engine::new_project("demo");
        assert!(!e.is_dirty(), "刚建好的工程是干净的");

        e.execute(Command::AddTrack { name: "A".into() }, Actor::Human);
        assert!(e.is_dirty(), "编辑之后变成 dirty");

        e.mark_saved();
        assert!(!e.is_dirty(), "保存后回到干净");

        // 撤销回到保存点之后，也应当自动变回干净。
        e.execute(Command::AddTrack { name: "B".into() }, Actor::Human);
        assert!(e.is_dirty());
        e.undo();
        assert!(!e.is_dirty(), "撤销回到保存点 → 不再是 dirty");

        // 改了名字也算改动。
        e.execute(Command::RenameProject { name: "Renamed".into() }, Actor::Human);
        assert!(e.is_dirty());
    }

    #[test]
    fn restore_keeps_the_unsaved_difference() {
        let mut e = Engine::new("demo");
        let saved = e.project().clone();
        e.execute(Command::AddTrack { name: "A".into() }, Actor::Human);
        let working = e.project().clone();

        let restored = Engine::restore(working, saved);
        assert!(restored.is_dirty(), "缓存里带着未保存改动，恢复后仍应为 dirty");
    }

    #[test]
    fn split_clip_cuts_notes_at_the_beat() {
        let mut e = Engine::new("t");
        let ev = e.execute(Command::AddTrack { name: "A".into() }, Actor::Human);
        let track = match ev[0] { Event::TrackAdded { id, .. } => id, _ => unreachable!() };
        let ev = e.execute(Command::AddClip { track, start: 0.0, length: 4.0 }, Actor::Human);
        let clip = match ev[0] { Event::ClipAdded { clip, .. } => clip, _ => unreachable!() };
        let note = |pitch, start, duration| crate::model::Note { id: 0, pitch, start, duration, velocity: 100, volume: 127 };
        e.execute(Command::AddNote { track, clip, note: note(60, 0.0, 1.0) }, Actor::Human);
        e.execute(Command::AddNote { track, clip, note: note(62, 1.0, 2.0) }, Actor::Human); // 跨过第 2 拍
        e.execute(Command::AddNote { track, clip, note: note(64, 3.0, 0.5) }, Actor::Human);

        let events = e.execute(Command::SplitClip { track, clip, beat: 2.0 }, Actor::Human);
        assert!(matches!(events.as_slice(), [Event::ClipSplit { .. }]));

        let t = e.project().track(track).unwrap();
        assert_eq!(t.clips.len(), 2);
        let (left, right) = (&t.clips[0], &t.clips[1]);
        assert_eq!((left.start, left.length), (0.0, 2.0));
        assert_eq!((right.start, right.length), (2.0, 2.0));

        let left_pitches: Vec<u8> = left.notes.iter().map(|n| n.pitch).collect();
        assert_eq!(left_pitches, vec![60, 62]);
        assert_eq!(left.notes[1].duration, 1.0, "跨过切点的音符左半段被截断");
        let right_pitches: Vec<u8> = right.notes.iter().map(|n| n.pitch).collect();
        assert_eq!(right_pitches, vec![62, 64]);
        assert_eq!((right.notes[0].start, right.notes[0].duration), (0.0, 1.0));
        assert_eq!((right.notes[1].start, right.notes[1].duration), (1.0, 0.5));

        assert!(e.undo());
        assert_eq!(e.project().track(track).unwrap().clips.len(), 1, "切分可撤销");
    }

    #[test]
    fn split_audio_clip_divides_the_source_window() {
        use crate::model::AudioSource;
        let mut e = Engine::new("t");
        let ev = e.execute(Command::AddTrack { name: "A".into() }, Actor::Human);
        let track = match ev[0] { Event::TrackAdded { id, .. } => id, _ => unreachable!() };
        let ev = e.execute(
            Command::AddAudioClip {
                track,
                start: 0.0,
                length: 4.0,
                source: AudioSource { path: "/tmp/a.wav".into(), offset_frames: 100, frames: Some(800), rate: 1.0, pitch_semitones: 0.0 },
            },
            Actor::Human,
        );
        let clip = match ev[0] { Event::AudioClipAdded { clip, .. } => clip, _ => unreachable!() };

        e.execute(Command::SplitClip { track, clip, beat: 2.0 }, Actor::Human);
        let t = e.project().track(track).unwrap();
        let left = t.clips[0].audio.clone().unwrap();
        let right = t.clips[1].audio.clone().unwrap();
        assert_eq!(left.offset_frames, 100);
        assert_eq!(left.frames, Some(400), "左半段窗口取一半");
        assert_eq!(right.offset_frames, 500);
        assert_eq!(right.frames, Some(400));
        // 两半段沿用同一速度 / 升降调参数 → 切分不改音高、不变速。
        assert_eq!(left.rate, right.rate);
        assert_eq!(left.pitch_semitones, right.pitch_semitones);
    }

    #[test]
    fn split_clip_ignores_beats_outside_the_clip() {
        let mut e = Engine::new("t");
        let ev = e.execute(Command::AddTrack { name: "A".into() }, Actor::Human);
        let track = match ev[0] { Event::TrackAdded { id, .. } => id, _ => unreachable!() };
        let ev = e.execute(Command::AddClip { track, start: 2.0, length: 2.0 }, Actor::Human);
        let clip = match ev[0] { Event::ClipAdded { clip, .. } => clip, _ => unreachable!() };
        for beat in [2.0, 1.0, 4.0, 9.0, f64::NAN] {
            assert!(e.execute(Command::SplitClip { track, clip, beat }, Actor::Human).is_empty());
        }
        assert_eq!(e.project().track(track).unwrap().clips.len(), 1);
    }

    /// 一次事务建「轨道 + 带音符的片段」，只占**一条**撤销记录。
    #[test]
    fn add_note_track_is_one_undo_step() {
        use crate::model::Note;
        let mut e = Engine::new("demo");
        let depth = e.undo_depth();
        let notes = vec![
            Note { id: 0, pitch: 60, start: 0.0, duration: 0.5, velocity: 100, volume: 127 },
            Note { id: 0, pitch: 64, start: 0.5, duration: 0.5, velocity: 90, volume: 127 },
        ];
        e.execute(
            Command::AddNoteTrack { name: "MIDI".into(), start: 2.0, length: 4.0, notes, at: None },
            Actor::Human,
        );
        assert_eq!(e.undo_depth(), depth + 1, "整条链只占一个撤销步骤");
        let t = e.project().tracks.last().expect("轨道应已建出");
        assert_eq!(t.name, "MIDI");
        assert_eq!(t.clips.len(), 1);
        assert_eq!(t.clips[0].notes.len(), 2);
        assert_eq!(t.clips[0].start, 2.0);

        assert!(e.undo());
        assert!(e.project().tracks.is_empty(), "撤销应把新轨道一并收回");
    }

    /// 新轨道可以插在指定行号（AI 转 MIDI 就插在源轨道下面），撤销/重做都复原顺序。
    #[test]
    fn add_note_track_can_insert_below_the_source_track() {
        let names = |e: &Engine| -> Vec<String> {
            e.project().tracks.iter().map(|t| t.name.clone()).collect()
        };
        let mut e = Engine::new("demo");
        e.execute(Command::AddTrack { name: "Audio".into() }, Actor::Human);
        e.execute(Command::AddTrack { name: "Bass".into() }, Actor::Human);

        e.execute(
            Command::AddNoteTrack { name: "MIDI".into(), start: 0.0, length: 4.0, notes: vec![], at: Some(1) },
            Actor::Human,
        );
        assert_eq!(names(&e), vec!["Audio", "MIDI", "Bass"], "应插在第 1 行");

        // 越界的位置夹到末尾，不 panic。
        e.execute(
            Command::AddNoteTrack { name: "Tail".into(), start: 0.0, length: 1.0, notes: vec![], at: Some(99) },
            Actor::Human,
        );
        assert_eq!(names(&e), vec!["Audio", "MIDI", "Bass", "Tail"]);

        assert!(e.undo());
        assert_eq!(names(&e), vec!["Audio", "MIDI", "Bass"]);
        assert!(e.undo());
        assert_eq!(names(&e), vec!["Audio", "Bass"], "撤销后顺序复原");
        assert!(e.redo());
        assert_eq!(names(&e), vec!["Audio", "MIDI", "Bass"], "重做要把轨道放回中间");
    }

    /// 一次事务建「音频轨道 + 引用素材的片段」（降噪就靠它把结果放到源轨道下面），只占一条撤销记录。
    #[test]
    fn add_audio_track_inserts_below_and_is_one_undo_step() {
        let mut e = Engine::new("demo");
        e.execute(Command::AddTrack { name: "Audio".into() }, Actor::Human);
        e.execute(Command::AddTrack { name: "Bass".into() }, Actor::Human);
        let depth = e.undo_depth();

        e.execute(
            Command::AddAudioTrack {
                name: "Audio · 降噪".into(),
                start: 2.0,
                length: 4.0,
                source: crate::model::AudioSource {
                    path: "clean.wav".into(),
                    offset_frames: 100,
                    frames: Some(800),
                    rate: 1.0,
                    pitch_semitones: 0.0,
                },
                at: Some(1),
            },
            Actor::Human,
        );
        assert_eq!(e.undo_depth(), depth + 1, "整条链只占一个撤销步骤");

        let t = &e.project().tracks[1];
        assert_eq!(t.name, "Audio · 降噪", "应插在源轨道下面");
        assert_eq!(t.clips.len(), 1);
        let clip = &t.clips[0];
        assert!(clip.is_audio(), "必须是音频片段");
        assert_eq!(clip.start, 2.0);
        assert_eq!(clip.length, 4.0);
        let src = clip.audio.as_ref().unwrap();
        assert_eq!(src.path, "clean.wav");
        assert_eq!((src.offset_frames, src.frames), (100, Some(800)), "素材窗口原样保留");

        assert!(e.undo());
        assert_eq!(e.project().tracks.len(), 2, "撤销应把新轨道一并收回");
    }

    #[test]
    fn set_sends_normalizes_and_drops_self_sends() {
        use crate::model::Send;
        let mut e = Engine::new("demo");
        let (t1, _) = seeded(&mut e);
        let ev = e.execute(Command::AddTrack { name: "Bus".into() }, Actor::Human);
        let t2 = match ev[0] { Event::TrackAdded { id, .. } => id, _ => unreachable!() };

        e.execute(
            Command::SetSends {
                track: t1,
                sends: vec![
                    Send { target: t2, gain_db: 999.0 },
                    Send { target: t1, gain_db: 0.0 }, // 自发送 → 丢弃
                ],
            },
            Actor::Human,
        );
        let sends = &e.project().track(t1).unwrap().sends;
        assert_eq!(sends.len(), 1, "自发送必须被丢弃");
        assert_eq!(sends[0].target, t2);
        assert_eq!(sends[0].gain_db, crate::model::MixerChannel::MAX_GAIN_DB, "增益归一化");

        assert!(e.undo(), "发送可撤销");
        assert!(e.project().track(t1).unwrap().sends.is_empty());
    }

    #[test]
    fn set_automation_sorts_filters_and_replaces() {
        use crate::model::AutomationPoint;
        let mut e = Engine::new("demo");
        let (track, _) = seeded(&mut e);

        e.execute(
            Command::SetAutomation {
                track,
                param: "gain_db".into(),
                points: vec![
                    AutomationPoint { beat: 4.0, value: -6.0 },
                    AutomationPoint { beat: 0.0, value: 0.0 },
                    AutomationPoint { beat: f64::NAN, value: 1.0 }, // 非有限 → 丢弃
                ],
            },
            Actor::Human,
        );
        let lanes = &e.project().track(track).unwrap().automation;
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].param, "gain_db");
        let beats: Vec<f64> = lanes[0].points.iter().map(|p| p.beat).collect();
        assert_eq!(beats, vec![0.0, 4.0], "点按拍排序且滤掉非有限值");

        // 同一参数整体替换。
        e.execute(
            Command::SetAutomation {
                track,
                param: "gain_db".into(),
                points: vec![AutomationPoint { beat: 1.0, value: 1.0 }],
            },
            Actor::Human,
        );
        assert_eq!(e.project().track(track).unwrap().automation[0].points.len(), 1);

        // 空曲线 = 删除车道。
        e.execute(Command::SetAutomation { track, param: "gain_db".into(), points: vec![] }, Actor::Human);
        assert!(e.project().track(track).unwrap().automation.is_empty());

        // 空参数名 = 无效命令，不产事件、不进历史。
        let depth = e.undo_depth();
        assert!(e
            .execute(Command::SetAutomation { track, param: "  ".into(), points: vec![] }, Actor::Human)
            .is_empty());
        assert_eq!(e.undo_depth(), depth);
    }
}
