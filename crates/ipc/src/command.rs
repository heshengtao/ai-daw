use crate::model::{
    ArticulationSource, ArticulationSwitch, AudioSource, AutomationPoint, BeatFilter, ClipId,
    ClipPlacement, ClipRef, CueSheetPlan, FitMode, HarmonyBar, MidiEventAt, MpeConfig, Note,
    NoteEdit, NotePatch, NoteRange, NoteTag, PitchRange, Section, Send, StrumDirection, TrackId,
    VelocityShape,
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
    SetKeyframe {
        beat: f64,
        tempo: f64,
        num: u8,
        den: u8,
    },
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
    AddClip {
        track: TrackId,
        start: f64,
        length: f64,
    },
    /// 在轨道上新增一个**音频片段**（素材由 `source.path` 指向）。
    ///
    /// `length`（拍）决定时间轴跨度；素材会**铺满**该跨度（缩放/改速度 → 按比例重采样，会变调）。
    AddAudioClip {
        track: TrackId,
        start: f64,
        length: f64,
        source: AudioSource,
    },
    /// 改写片段的音频素材。`None` 表示把它**变回 MIDI 片段**（同时清空音符）。
    SetClipAudio {
        track: TrackId,
        clip: ClipId,
        audio: Option<AudioSource>,
    },
    /// 设置音频片段的**播放速度**（`rate`，1.0 = 原速）与**升降调**（半音，0 = 不变）。
    ///
    /// 只对**音频片段**有效：对 MIDI 片段安静忽略（`translate` 里返回空）。数值会被归一化。
    SetClipRate {
        track: TrackId,
        clip: ClipId,
        rate: f64,
        pitch_semitones: f64,
    },
    /// 从轨道上删除一个片段。
    RemoveClip { track: TrackId, clip: ClipId },
    /// 把片段移动到新的起始拍。
    MoveClip {
        track: TrackId,
        clip: ClipId,
        start: f64,
    },
    /// 批量移动片段（多选拖动，一次提交一条撤销记录）。
    MoveClips { items: Vec<ClipPlacement> },
    /// 改变片段长度（拍）。
    ResizeClip {
        track: TrackId,
        clip: ClipId,
        length: f64,
    },
    /// 在片段的指定位置新增一个音符。
    AddNote {
        track: TrackId,
        clip: ClipId,
        note: Note,
    },
    /// 改写片段内的某个音符（按片段内的索引定位）。
    SetNote {
        track: TrackId,
        clip: ClipId,
        index: usize,
        note: Note,
    },
    /// 批量改写片段内的多个音符（多选拖动、属性面板一次改多个）。
    SetNotes {
        track: TrackId,
        clip: ClipId,
        entries: Vec<NoteEdit>,
    },
    /// 删除片段内的某个音符（按片段内的索引定位）。
    RemoveNote {
        track: TrackId,
        clip: ClipId,
        index: usize,
    },
    /// 批量删除片段内的多个音符（框选后删除）。
    RemoveNotes {
        track: TrackId,
        clip: ClipId,
        indices: Vec<usize>,
    },
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
    SetAutomation {
        track: TrackId,
        param: String,
        points: Vec<AutomationPoint>,
    },
    /// 设置轨道乐器的某个参数。
    SetParam {
        track: TrackId,
        param: String,
        value: f32,
    },
    /// 在轨道效果链末尾追加一个效果器。
    AddEffect { track: TrackId, plugin_id: String },
    /// 从轨道效果链移除一个效果器（按链内序号）。
    RemoveEffect { track: TrackId, index: usize },
    /// 调整效果器在链中的位置（`from` → `to`）。
    MoveEffect {
        track: TrackId,
        from: usize,
        to: usize,
    },
    /// 设置轨道效果器的某个参数（按链内序号定位）。
    SetEffectParam {
        track: TrackId,
        index: usize,
        param: String,
        value: f32,
    },
    /// 在第 `beat` 拍切分一个片段（razor）。
    ///
    /// 左半段沿用原 `clip`（id 不变），右半段是新片段。`beat` 必须落在片段内部，否则安静忽略。
    /// 音频片段需要有素材窗口长度（`AudioSource::frames`）才能切分——否则无法确定「左半段到哪里停」。
    SplitClip {
        track: TrackId,
        clip: ClipId,
        beat: f64,
    },
    /// 重命名整个工程。
    RenameProject { name: String },
    /// 新建工程：清空并铺上默认分轨模板（引擎里的 `STARTER_TRACKS`）。
    ///
    /// `track_names` 是**按模板顺序**给出的分轨名（界面按当前语言生成）。
    /// 传 `None` 就用模板自带的默认名（引擎侧不认语言，只有界面知道该写什么）。
    NewProject {
        name: String,
        track_names: Option<Vec<String>>,
    },
    /// 新建工程，**只创建给定的轨道**（不铺默认分轨模板）。
    ///
    /// 与 `NewProject` 的区别：`NewProject` 是「模板 + 按语言改名」，这里则是「按名字建 N 条」。
    /// agent 从影片清单起工程时用它，避免建完再删掉三条模板轨。
    NewProjectWithTracks {
        name: String,
        track_names: Vec<String>,
    },

    // ---- 批量音符（编曲主力） ----
    /// 一次新增一组音符（**一次事务** = 一条撤销记录）。
    ///
    /// 编曲通常一次产生几十到几百个音符；逐个 `AddNote` 会让撤销变成灾难。
    /// 音符的 `id` 由引擎发放，调用方给的 id 会被忽略。
    AddNotes {
        track: TrackId,
        clip: ClipId,
        notes: Vec<Note>,
    },
    /// **原子替换**片段内的全部音符（清空后写入；`notes` 为空 = 清空片段）。
    ReplaceNotes {
        track: TrackId,
        clip: ClipId,
        notes: Vec<Note>,
    },
    /// 按 **id** 批量改写音符；只改给出的字段。
    UpdateNotes {
        track: TrackId,
        clip: ClipId,
        updates: Vec<NotePatch>,
    },
    /// 按 **id** 批量删除音符。
    RemoveNotesById {
        track: TrackId,
        clip: ClipId,
        ids: Vec<u64>,
    },
    /// 批量移调（半音）；可选范围限定。
    TransposeNotes {
        track: TrackId,
        clip: ClipId,
        semitones: i32,
        range: Option<NoteRange>,
    },
    /// 批量设置力度（固定值 / 渐变 / 缩放），可按拍位过滤。
    SetNoteVelocity {
        track: TrackId,
        clip: ClipId,
        shape: VelocityShape,
        range: Option<NoteRange>,
        /// 只作用于落在目标拍位上的音符（`None` = 不按拍位过滤）。
        beat_filter: Option<BeatFilter>,
    },
    /// 量化：把起点（以及可选的时值）吸到 `grid`（拍）的整数倍，`strength` 决定吸多少。
    QuantizeNotes {
        track: TrackId,
        clip: ClipId,
        grid: f64,
        strength: f64,
        range: Option<NoteRange>,
        /// 是否同时量化时值（缺省 false，只动起点）。
        quantize_duration: bool,
    },
    /// 扫弦：把**起点相同**的音符按音高依次错开。
    ///
    /// `step_ms` 是相邻两音的毫秒差（0 = 清掉扫弦、回到同时发声），`direction` 决定谁先响。
    /// 毫秒在引擎里按**该处生效的速度**换算成拍（模型里的时间是拍），所以改工程速度不会改变扫弦
    /// 的「几根弦差这么多」。写入的是每个音符的 `Note::offset`。
    StrumNotes {
        track: TrackId,
        clip: ClipId,
        step_ms: f64,
        direction: StrumDirection,
        range: Option<NoteRange>,
    },
    /// Swing：把指定 `subdivision`（拍）的**第二个子拍**往后推，`ratio` 是它与前一个子拍的距离占比。
    ///
    /// 8 分 swing = `subdivision: 0.5` + `ratio: 0.66`（第二拍落到约 2/3 拍处）。
    ApplySwing {
        track: TrackId,
        clip: ClipId,
        subdivision: f64,
        ratio: f64,
        strength: f64,
        range: Option<NoteRange>,
    },
    /// 人性化：给位置 / 力度加**确定性**随机偏移（同一个 `seed` 结果完全一致，重放不会漂）。
    HumanizeNotes {
        track: TrackId,
        clip: ClipId,
        /// 位置抖动上限（拍）。
        timing: f64,
        /// 力度抖动上限（0..=127 的绝对量）。
        velocity: f64,
        range: Option<NoteRange>,
        seed: Option<u64>,
        /// 跳过短于这个时值（拍）的音符；0 = 不跳过。用来保护倚音/装饰音。
        min_duration: f64,
        /// 起点相同的音符共用同一个位置抖动（保护和弦 / 扫弦的同步）。
        group_by_start: bool,
    },
    /// 写入 / 清除一个音符的标签（装饰音 / 奏法）；`None` = 移除。
    SetNoteTag {
        track: TrackId,
        clip: ClipId,
        id: u64,
        tag: Option<NoteTag>,
    },
    /// 一次给多个音符写同一个标签（多选切换奏法 = 一个撤销步）。
    SetNoteTags {
        track: TrackId,
        clip: ClipId,
        ids: Vec<u64>,
        tag: Option<NoteTag>,
    },
    /// 把音符收进指定音域：`Fold` 做八度折返（不改音级），`Report` 只统计不改动。
    FitNotesToRange {
        track: TrackId,
        clip: ClipId,
        min_pitch: u8,
        max_pitch: u8,
        mode: FitMode,
        range: Option<NoteRange>,
    },

    // ---- 片段编辑（复制 / 合并） ----
    /// 复制片段：默认贴在原片段末尾（`start = 原起点 + 原长度`），可换轨道。
    DuplicateClip {
        track: TrackId,
        clip: ClipId,
        /// 目标轨道（`None` = 原轨道）。
        to_track: Option<TrackId>,
        /// 目标起点（`None` = 紧接在原片段之后）。
        start: Option<f64>,
    },
    /// 把同一轨道上的多个 MIDI 片段合并成一个（保留第一个的 id，跨度取并集）。
    ///
    /// 音频片段不参与合并（各自的素材窗口无法用「挪音符」表达）——遇到就整体拒绝。
    MergeClips { track: TrackId, clips: Vec<ClipId> },

    // ---- 编曲元数据 ----
    /// 整体替换工程的小节和弦表（空 = 清除）。
    SetProjectHarmony { bars: Vec<HarmonyBar> },
    /// 整体替换工程的段落表（空 = 清除）。
    SetProjectSections { sections: Vec<Section> },
    /// 设置轨道的编曲元数据（分工 / 目标音域）；两者都为空 = 清除。
    SetTrackMeta {
        track: TrackId,
        role: Option<String>,
        pitch_range: Option<PitchRange>,
        pitch_bend_range: Option<f32>,
    },
    /// 设置轨道的 MPE 配置；`None` = 关闭（所有音符走主通道）。
    SetMpe {
        track: TrackId,
        config: Option<MpeConfig>,
    },
    /// 整体替换轨道的通用 MIDI 事件表（空 = 清空）。
    SetMidiEvents {
        track: TrackId,
        events: Vec<MidiEventAt>,
    },
    /// 整体替换轨道的奏法映射（空 = 清空）。
    SetArticulations {
        track: TrackId,
        articulations: std::collections::BTreeMap<String, ArticulationSwitch>,
        default: Option<String>,
        /// 映射来源（从映射库导入时写入）；`None` = 手工配置。
        source: Option<ArticulationSource>,
    },

    /// 按影片清单一次性建立编曲骨架：速度 / 拍号 / 段落 / 和弦 / 轨道 / 乐器 / 片段。
    ///
    /// 所有轨道与片段的 id 都在**同一次** `translate` 里分配，因此整份清单只占一个撤销步——
    /// 这正是 `import_cue_sheet` 需要的语义（agent 不应该为了导入 20 段配乐按 200 次撤销）。
    ApplyCueSheet { plan: CueSheetPlan },
}
