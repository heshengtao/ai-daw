use crate::model::{
    AudioSource, AutomationPoint, ClipId, ClipPlacement, ClipRef, Note, NoteEdit, Send, TrackId,
};

/// 命令：期望的状态变更请求。
///
/// 所有数值都必须在 `Engine::translate` 里被归一化/夹紧——命令既来自界面，也来自
/// AI 代理与工程文件，绝不能假设调用方已经校验过。
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// 设置工程速度（BPM）。
    SetTempo(f64),
    /// 设置拍号（分子 / 分母）；越界值会被归一化。
    SetTimeSignature { num: u8, den: u8 },
    /// 在第 `beat` 拍插入/更新一个关键点（同时带 BPM 与拍号）。`beat <= 0` 会被忽略。
    SetKeyframe { beat: f64, tempo: f64, num: u8, den: u8 },
    /// 删除第 `beat` 拍的关键点（容差内匹配）。
    RemoveKeyframe { beat: f64 },
    /// 新增一条轨道。
    AddTrack { name: String },
    /// 新建一条 MIDI 轨道，并在其上放一个带音符的片段（**一次事务** = 一条撤销记录）。
    ///
    /// 与「AddTrack + AddClip + AddNote×N」等价，但轨道 / 片段的 id 在**同一次** `translate` 里
    /// 分配，所以调用方不必先建轨道再回读 id——「音频转 MIDI」正是这种用法。
    AddNoteTrack {
        name: String,
        start: f64,
        length: f64,
        notes: Vec<Note>,
        /// 新轨道插到第几行（`None` = 追加到末尾）。AI 转 MIDI 时插在源轨道下面。
        at: Option<usize>,
    },
    /// 新建一条音频轨道，并在其上放一个引用素材的片段（**一次事务** = 一条撤销记录）。
    ///
    /// 与 `AddNoteTrack` 对称：id 在同一次 `translate` 里分配。「降噪」用它把处理结果放到
    /// 源轨道**下面**的一条新轨道上，方便和原轨对比试听。
    AddAudioTrack {
        name: String,
        start: f64,
        length: f64,
        source: AudioSource,
        /// 新轨道插到第几行（`None` = 追加到末尾）。
        at: Option<usize>,
    },
    /// 删除一条轨道。
    RemoveTrack(TrackId),
    /// 重命名一条轨道。
    RenameTrack { track: TrackId, name: String },
    /// 为轨道设置乐器插件。
    SetTrackInstrument { track: TrackId, plugin_id: String },
    /// 在轨道上新增一个 MIDI 片段。
    AddClip { track: TrackId, start: f64, length: f64 },
    /// 在轨道上新增一个**音频片段**（素材由 `source.path` 指向）。
    ///
    /// `length`（拍）决定时间轴跨度；素材会**铺满**该跨度（缩放/改速度 → 按比例重采样，会变调）。
    AddAudioClip { track: TrackId, start: f64, length: f64, source: AudioSource },
    /// 改写片段的音频素材。`None` 表示把它**变回 MIDI 片段**（同时清空音符）。
    SetClipAudio { track: TrackId, clip: ClipId, audio: Option<AudioSource> },
    /// 从轨道上删除一个片段。
    RemoveClip { track: TrackId, clip: ClipId },
    /// 把片段移动到新的起始拍。
    MoveClip { track: TrackId, clip: ClipId, start: f64 },
    /// 批量移动片段（多选拖动，一次提交一条撤销记录）。
    MoveClips { items: Vec<ClipPlacement> },
    /// 改变片段长度（拍）。
    ResizeClip { track: TrackId, clip: ClipId, length: f64 },
    /// 在片段的指定位置新增一个音符。
    AddNote { track: TrackId, clip: ClipId, note: Note },
    /// 改写片段内的某个音符（按片段内的索引定位）。
    SetNote { track: TrackId, clip: ClipId, index: usize, note: Note },
    /// 批量改写片段内的多个音符（多选拖动、属性面板一次改多个）。
    SetNotes { track: TrackId, clip: ClipId, entries: Vec<NoteEdit> },
    /// 删除片段内的某个音符（按片段内的索引定位）。
    RemoveNote { track: TrackId, clip: ClipId, index: usize },
    /// 批量删除片段内的多个音符（框选后删除）。
    RemoveNotes { track: TrackId, clip: ClipId, indices: Vec<usize> },
    /// 批量删除片段（可跨轨道）。
    RemoveClips { items: Vec<ClipRef> },
    /// 设置轨道增益（dB）。
    SetGain { track: TrackId, gain_db: f32 },
    /// 设置轨道声像（-1.0 最左，1.0 最右）。
    SetPan { track: TrackId, pan: f32 },
    /// 设置轨道静音。
    SetMute { track: TrackId, mute: bool },
    /// 设置轨道独奏。
    SetSolo { track: TrackId, solo: bool },
    /// 设置总输出增益（dB）。
    SetMasterGain(f32),
    /// 设置轨道的发送列表（**整体替换**；空 = 清空发送）。
    SetSends { track: TrackId, sends: Vec<Send> },
    /// 设置一条自动化车道（**整体替换**该参数的曲线；`points` 为空 = 删除该车道）。
    ///
    /// `param` 取名如 `"gain_db"` / `"pan"`；未知名字会被保留但求值时忽略。
    SetAutomation { track: TrackId, param: String, points: Vec<AutomationPoint> },
    /// 设置轨道乐器的某个参数。
    SetParam { track: TrackId, param: String, value: f32 },
    /// 在轨道效果链末尾追加一个效果器。
    AddEffect { track: TrackId, plugin_id: String },
    /// 从轨道效果链移除一个效果器（按链内序号）。
    RemoveEffect { track: TrackId, index: usize },
    /// 调整效果器在链中的位置（`from` → `to`）。
    MoveEffect { track: TrackId, from: usize, to: usize },
    /// 设置轨道效果器的某个参数（按链内序号定位）。
    SetEffectParam { track: TrackId, index: usize, param: String, value: f32 },
    /// 在第 `beat` 拍切分一个片段（razor）。
    ///
    /// 左半段沿用原 `clip`（id 不变），右半段是新片段。`beat` 必须落在片段内部，否则安静忽略。
    /// 音频片段需要有素材窗口长度（`AudioSource::frames`）才能切分——否则无法确定「左半段到哪里停」。
    SplitClip { track: TrackId, clip: ClipId, beat: f64 },
    /// 重命名整个工程。
    RenameProject { name: String },
    /// 新建工程：清空并铺上默认分轨模板（引擎里的 `STARTER_TRACKS`）。
    ///
    /// `track_names` 是**按模板顺序**给出的分轨名（界面按当前语言生成）。
    /// 传 `None` 就用模板自带的默认名（引擎侧不认语言，只有界面知道该写什么）。
    NewProject { name: String, track_names: Option<Vec<String>> },
}
