use crate::model::{ClipId, ClipPlacement, ClipRef, Note, NoteEdit, TrackId};

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
    /// 删除一条轨道。
    RemoveTrack(TrackId),
    /// 重命名一条轨道。
    RenameTrack { track: TrackId, name: String },
    /// 为轨道设置乐器插件。
    SetTrackInstrument { track: TrackId, plugin_id: String },
    /// 在轨道上新增一个 MIDI 片段。
    AddClip { track: TrackId, start: f64, length: f64 },
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
    /// 新建工程：清空并铺上默认分轨模板（引擎里的 `STARTER_TRACKS`）。
    ///
    /// `track_names` 是**按模板顺序**给出的分轨名（界面按当前语言生成）。
    /// 传 `None` 就用模板自带的默认名（引擎侧不认语言，只有界面知道该写什么）。
    NewProject { name: String, track_names: Option<Vec<String>> },
}
