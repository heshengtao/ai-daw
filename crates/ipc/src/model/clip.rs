use serde::{Deserialize, Serialize};

use super::track::TrackId;

/// 片段标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ClipId(pub u64);

/// 音符；位置与时长单位为拍。
///
/// 两个「响度类」属性刻意分开，语义不同：
/// - `velocity` **演奏强度**：手弹得多重。真实乐器里它决定音色（明亮度、击弦力度），
///   内置合成器则把它折算成振幅。将来接入的插件会各自解释它。
/// - `volume` **音量**：单音电平微调，纯粹的增益，不改变音色。默认 127（不衰减）。
///
/// `id` 是**片段内稳定**的标识：批量工具（`update_notes` / `remove_notes`）靠它定位，
/// 而不是靠数组下标——删掉一个音符不该让后面所有音符的定位全部漂移。旧工程没有这个字段，
/// 读入时按片段顺序补齐（见 `Project::ensure_note_ids`）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// 片段内唯一标识。旧工程 / 手写 JSON 缺这个字段时按 0 读入，由引擎补号。
    #[serde(default)]
    pub id: u64,
    pub pitch: u8,
    pub start: f64,
    pub duration: f64,
    pub velocity: u8,
    /// 单音音量（0..=127，127 = 不衰减）。旧工程文件缺这个字段时按 127 读入。
    #[serde(default = "default_note_volume")]
    pub volume: u8,
    /// **起始偏移**（拍；正 = 延后发声）。缺省 0 = 与 `start` 同时发声。
    ///
    /// 记谱位置（`start`，网格上的那一格）与演奏位置分开：扫弦就是给同一 `start` 的一组音符
    /// 写递增偏移（下扫 = 低音先响），「自由网格」下的手感微调也用它。**不参与网格吸附**，
    /// 量化也只动 `start`。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub offset: f64,
    /// **连奏**：NoteOff 延到同音高的下一个音起点（同片段内），没有下一个就按原时值。
    ///
    /// 指弹 / 单音线条里很常用：重复同一个音高时不该在音与音之间留一道缝（采样器会听成断音）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub legato: bool,
}

/// 旧工程文件（以及手写的 JSON）可能没有 `volume` 字段——按「不衰减」读入，
/// 这样老工程的听感完全不变。
fn default_note_volume() -> u8 {
    Note::MAX_VOLUME
}

/// 是否为 `false`（`legato` 缺省）——为假时不写进 JSON。
fn is_false(v: &bool) -> bool {
    !*v
}

impl Note {
    /// 最短时长：**1/32 拍**（= 0.03125 拍，4/4 下相当于 1/128 音符；76 BPM 时约 25 ms）。
    ///
    /// 注意别和「32 分音符」混淆：32 分音符是 **1/8 拍**（= 0.125 拍）。
    /// 这个下限只是为了避免零长度音符（会让 NoteOn/NoteOff 同帧），远低于真实倚音的 30–60 ms。
    pub const MIN_DURATION_BEATS: f64 = 1.0 / 32.0;
    /// 最长时长（拍）：64 小节 4/4，足够容纳任何合法输入，同时挡住天文数字。
    pub const MAX_DURATION_BEATS: f64 = 256.0;
    /// 起点上界（拍）：约 250000 小节，仅用于挡住非有限/荒谬输入。
    pub const MAX_START_BEATS: f64 = 1_000_000.0;
    /// 起始偏移的上界（拍，正负对称）。
    ///
    /// 4 拍（4/4 的一整小节）远超任何扫弦与手感微调的需要，只用来挡住非有限/荒谬输入；
    /// 非有限值回落到 0，别把 NaN 带进播放。
    pub const MAX_OFFSET_BEATS: f64 = 4.0;
    /// 音量的上界（= 不衰减）。
    pub const MAX_VOLUME: u8 = 127;

    /// 归一化：音高/力度/音量夹到 MIDI 0..=127，起点非负且有上界，时长落在合法区间。
    ///
    /// 手写工程、旧文件、AI 代理都可能送来越界值；所有写入路径都必须经过这里。
    /// `id` 原样保留——它是**定位依据**，不能被「修正」。`legato` 是布尔，没有可夹的东西。
    pub fn normalize(note: Note) -> Note {
        Note {
            id: note.id,
            pitch: note.pitch.min(127),
            // 音符起点是**相对片段起点**的拍，**允许为负**：左边缘往右拖时音符要停在原处（被
            // 方框裁掉但不删），往回拖就能恢复。
            start: clamp_signed_beats(note.start, Self::MAX_START_BEATS),
            duration: clamp_range(note.duration, Self::MIN_DURATION_BEATS, Self::MAX_DURATION_BEATS),
            velocity: note.velocity.min(127),
            volume: note.volume.min(Self::MAX_VOLUME),
            // 偏移同样允许为负（提前一点起音也是演奏姿态），只夹上下界。
            offset: clamp_signed_beats(note.offset, Self::MAX_OFFSET_BEATS),
            legato: note.legato,
        }
    }

    /// 音高 → 音级（0..=11，C=0）。八度折返只改音级上方的八度，不改音级本身。
    pub fn pitch_class(pitch: u8) -> u8 {
        pitch % 12
    }

    /// 该音高属于哪个八度（MIDI 60 = C4 → 4）。
    pub fn octave(pitch: u8) -> i16 {
        (pitch as i16) / 12 - 1
    }
}

/// 音频素材引用：片段指向一段外部音频文件。
///
/// v1 存**绝对路径**（录音落在应用数据目录、导入的文件沿用原位置）；素材库/相对路径留后续。
///
/// **播放速度与工程 BPM 解耦**：`rate` / `pitch_semitones` 是片段自己的参数——改它们不影响
/// 工程速度；改工程速度也不影响音频（音频默认 1:1 原速）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioSource {
    /// 素材文件路径。
    pub path: String,
    /// 素材内的起始帧（crop 起点）。旧工程/手写 JSON 缺省时按 0 读入。
    #[serde(default)]
    pub offset_frames: u64,
    /// 素材窗口长度（源帧）：从 `offset_frames` 起**取多少帧**（裁剪 / 切分记录下来的）。
    ///
    /// `None` = 取到素材结尾（旧工程/手写 JSON 缺省）。切分音频片段时必须知道窗口长度，
    /// 否则「左半段」会一路播到结尾——所以导入时会把素材总帧数写进来。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<u64>,
    /// 播放速度倍率（1.0 = 原速，>1 更快）。改它**只改时长，不变调**（时值伸缩）。
    #[serde(default = "default_rate", skip_serializing_if = "is_default_rate")]
    pub rate: f64,
    /// 升降调（半音，0 = 不变；正 = 升高）。改它**只变调，不改时长**。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pitch_semitones: f64,
}

/// `rate` 的缺省值（原速）。
fn default_rate() -> f64 {
    1.0
}

/// 是否等于缺省速度——等于时不写进 JSON（保持旧工程形状）。
fn is_default_rate(v: &f64) -> bool {
    (*v - 1.0).abs() < 1e-9
}

/// 是否为 0（升降调缺省）——为 0 时不写进 JSON。
fn is_zero(v: &f64) -> bool {
    v.abs() < 1e-9
}

impl AudioSource {
    /// 最低速度倍率。
    pub const MIN_RATE: f64 = 0.25;
    /// 最高速度倍率。
    pub const MAX_RATE: f64 = 4.0;
    /// 升降调上限（半音，正负对称）。
    pub const MAX_PITCH_SEMITONES: f64 = 24.0;

    /// 归一化：速度/升降调夹到合法区间，非有限值回落到缺省（而不是把 NaN 带进播放）。
    pub fn normalize_rate(rate: f64) -> f64 {
        if rate.is_finite() {
            rate.clamp(Self::MIN_RATE, Self::MAX_RATE)
        } else {
            default_rate()
        }
    }

    /// 归一化升降调。
    pub fn normalize_pitch(semitones: f64) -> f64 {
        if semitones.is_finite() {
            semitones.clamp(-Self::MAX_PITCH_SEMITONES, Self::MAX_PITCH_SEMITONES)
        } else {
            0.0
        }
    }

    /// 归一化整个素材引用（速度 / 升降调）。
    pub fn normalize(mut self) -> Self {
        self.rate = Self::normalize_rate(self.rate);
        self.pitch_semitones = Self::normalize_pitch(self.pitch_semitones);
        self
    }
}

/// 片段：MIDI（`notes`）或音频（`audio`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    pub start: f64,
    pub length: f64,
    pub notes: Vec<Note>,
    /// 音频素材；`Some` 表示这是**音频片段**（此时 `notes` 必为空）。
    ///
    /// 旧工程没有这个字段 → `None`（MIDI 片段）；`skip_serializing_if` 保证 MIDI 片段的
    /// 输出形状与从前**逐字一致**（`src/api/types.ts` 与 wire-shape 测试都依赖这一点）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioSource>,
}

/// 片段引用（轨道 + 片段）：批量操作（多选框选后删除）需要跨轨道寻址。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClipRef {
    pub track: TrackId,
    pub clip: ClipId,
}

/// 片段放置：把某个片段移动到新的位置（批量拖动 / 跨轨道拖动用）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClipPlacement {
    /// 片段**当前**所在轨道。
    pub track: TrackId,
    pub clip: ClipId,
    /// 目标轨道；与 `track` 相同表示只改起点。
    pub to_track: TrackId,
    /// 新的起始拍。
    pub start: f64,
}

/// 音符改写项：片段内序号 + 新值。
///
/// 音符没有独立 id（模型刻意保持扁平），因此片段内**序号**是唯一的定位方式；批量改写
/// 一次带上全部项，才能保证「改 10 个音符」只产生一条撤销记录。
///
/// 界面（多选拖动、属性面板）继续用它；AI 侧的新工具改用 [`NotePatch`]（按稳定 id 定位）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NoteEdit {
    pub index: usize,
    pub note: Note,
}

/// 按 **id** 定位的音符改写项：只带需要改的字段，其余保持不变。
///
/// 与 [`NoteEdit`] 的差别只在定位方式——批量工具需要「删掉几个之后仍然指得准」，
/// 数组下标做不到，稳定 id 可以。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NotePatch {
    /// 目标音符的 id（片段内唯一）。
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<u8>,
    /// 新的起始偏移（拍）。扫弦 / 手感微调用它；`Some(0.0)` = 回到「同时发声」。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<f64>,
    /// 新的连奏开关。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legato: Option<bool>,
}

impl NotePatch {
    /// 把改写项落到一个音符上；未给出的字段保持不变。
    pub fn apply_to(&self, note: Note) -> Note {
        Note {
            id: note.id,
            pitch: self.pitch.unwrap_or(note.pitch),
            start: self.start.unwrap_or(note.start),
            duration: self.duration.unwrap_or(note.duration),
            velocity: self.velocity.unwrap_or(note.velocity),
            volume: self.volume.unwrap_or(note.volume),
            offset: self.offset.unwrap_or(note.offset),
            legato: self.legato.unwrap_or(note.legato),
        }
    }
}

/// 音符范围（**片段内**拍坐标，左闭右开）：批量变换只作用于起点落在区间内的音符。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NoteRange {
    pub start: f64,
    pub end: f64,
}

impl NoteRange {
    /// 该音符是否落在范围内（按**起点**判定，与钢琴卷帘的框选一致）。
    pub fn contains(&self, note: &Note) -> bool {
        note.start >= self.start && note.start < self.end
    }

    /// 归一化：保证 `start <= end`，非有限值视为「整段」。
    pub fn normalize(self) -> Self {
        let start = if self.start.is_finite() { self.start } else { f64::NEG_INFINITY };
        let end = if self.end.is_finite() { self.end } else { f64::INFINITY };
        if start <= end {
            NoteRange { start, end }
        } else {
            NoteRange { start: end, end: start }
        }
    }
}

/// 拍位过滤（`set_note_velocity`）：只作用于落在 `beat + k * grid` 附近的音符。
///
/// 例：`grid = 1.0, beat = 0.5` 只改每拍反拍（`and`）上的音；
/// `grid = 1.0, beat = 0.0` 只改正拍。按**起点**判定，容差 1e-6 拍。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BeatFilter {
    /// 网格长度（拍），必须 > 0。
    pub grid: f64,
    /// 网格内的目标位置（拍），按 `grid` 取模。
    pub beat: f64,
}

impl BeatFilter {
    /// 该音符是否落在目标拍位上。
    pub fn contains(&self, note: &Note) -> bool {
        if !(self.grid.is_finite() && self.grid > 0.0 && self.beat.is_finite()) {
            return true;
        }
        let phase = note.start.rem_euclid(self.grid);
        let target = self.beat.rem_euclid(self.grid);
        let diff = (phase - target).abs();
        let diff = diff.min(self.grid - diff);
        diff <= 1e-6
    }
}

/// 力度变换的形状（`set_note_velocity`）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VelocityShape {
    /// 全部设为同一力度。
    Fixed(u8),
    /// 按拍位在范围内线性变化（渐强 / 渐弱）。
    Ramp { from: u8, to: u8 },
    /// 相对缩放（1.0 = 不变，0.5 = 减半，1.5 = 放大一半）。
    Scale(f64),
}

/// 扫弦方向（`strum_notes`）：同一位置上的一组音符谁先响。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrumDirection {
    /// 下扫：低音先响（吉他上最常用）。
    Down,
    /// 上扫：高音先响。
    Up,
}

impl StrumDirection {
    /// 解析字符串（MCP / CLI 传的是字面量）。
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "down" | "d" | "下扫" => Some(StrumDirection::Down),
            "up" | "u" | "上扫" => Some(StrumDirection::Up),
            _ => None,
        }
    }
}

/// 音域约束的处理方式（`fit_notes_to_range`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FitMode {
    /// 八度折返：只把越界的音上下挪整八度，**不改音级**。
    Fold,
    /// 只报告，不改动任何音符。
    Report,
}

impl FitMode {
    /// 解析字符串（MCP / CLI 传的是字面量）。
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "fold" | "octave" | "octave_fold" => Some(FitMode::Fold),
            "report" | "none" | "dry_run" => Some(FitMode::Report),
            _ => None,
        }
    }
}

impl Clip {
    /// 按 id 找音符下标。
    pub fn note_index(&self, id: u64) -> Option<usize> {
        self.notes.iter().position(|n| n.id == id)
    }

    /// 片段内下一个可用的音符 id（当前最大值 + 1，至少 1）。
    ///
    /// 只用于**给新音符发号**：id 只需在片段内唯一，删掉的号不回收——回收会让
    /// 「刚删掉的 id」被新音符复用，外部拿着旧 id 的操作就会打到别的音符上。
    pub fn next_note_id(&self) -> u64 {
        self.notes.iter().map(|n| n.id).max().unwrap_or(0) + 1
    }
}

impl Clip {
    /// 最短长度（拍）＝一个 32 分音符。
    pub const MIN_LENGTH_BEATS: f64 = 1.0 / 32.0;
    /// 最长长度（拍）。
    ///
    /// **不能太小**：导入音频时片段拍长 = 素材的自然时长；早先这里是 256 拍（4/4 的 64 小节），
    /// 于是任何超过 64 小节的音频**尾部会被静默截断**，右边缘拖到那儿也「回弹」。现在放到
    /// 65536 拍（120 BPM 下约 9 小时），任何现实素材都放得下。
    pub const MAX_LENGTH_BEATS: f64 = 65_536.0;
    /// 起始位置上界（拍）。
    pub const MAX_START_BEATS: f64 = 1_000_000.0;

    /// 归一化长度。
    pub fn normalize_length(length: f64) -> f64 {
        clamp_range(length, Self::MIN_LENGTH_BEATS, Self::MAX_LENGTH_BEATS)
    }

    /// 归一化起始位置。
    pub fn normalize_start(start: f64) -> f64 {
        clamp_beats(start, Self::MAX_START_BEATS)
    }

    /// 这是不是音频片段。
    pub fn is_audio(&self) -> bool {
        self.audio.is_some()
    }

    /// 归一化整个片段：夹紧起点/长度；音频片段的 `notes` 必须为空
    /// （两种内容不能共存，否则播放与界面会各画一半），并归一化速度 / 升降调。
    pub fn normalize(mut clip: Clip) -> Clip {
        clip.start = Self::normalize_start(clip.start);
        clip.length = Self::normalize_length(clip.length);
        if let Some(audio) = clip.audio.take() {
            clip.notes.clear();
            clip.audio = Some(audio.normalize());
        }
        clip
    }
}

/// 把「拍」坐标夹到 `0..=max`；非有限值回落到 0（而不是 NaN 传播到整条时间轴）。
pub fn clamp_beats(value: f64, max: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, max)
    } else {
        0.0
    }
}

/// 把「拍」坐标夹到 `-max..=max`：**相对片段起点**的音符允许落在方框左侧（被裁掉但仍保留）。
pub fn clamp_signed_beats(value: f64, max: f64) -> f64 {
    if value.is_finite() {
        value.clamp(-max, max)
    } else {
        0.0
    }
}

/// 把长度类数值夹到 `min..=max`；非有限值回落到 `min`。
pub fn clamp_range(value: f64, min: f64, max: f64) -> f64 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        min
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_normalization_clamps_every_field() {
        let n = Note::normalize(Note { id: 0, pitch: 200, start: -3.0, duration: 0.0, velocity: 250, volume: 250, offset: 0.0, legato: false });
        assert_eq!(n.pitch, 127);
        // 起点是**相对片段起点**的拍，允许为负（收起时被裁掉的音符要留着）。
        assert_eq!(n.start, -3.0);
        assert_eq!(n.duration, Note::MIN_DURATION_BEATS);
        assert_eq!(n.velocity, 127);
        assert_eq!(n.volume, Note::MAX_VOLUME);
    }

    #[test]
    fn note_normalization_rejects_non_finite() {
        let n = Note::normalize(Note { id: 0, pitch: 60, start: f64::NAN, duration: f64::INFINITY, velocity: 100, volume: 127, offset: 0.0, legato: false });
        // 非有限的起点回落到 0；非有限的时长回落到最短时长（而不是让它永远挂着）。
        assert_eq!(n.start, 0.0);
        assert_eq!(n.duration, Note::MIN_DURATION_BEATS);
    }

    /// `offset` 夹到 ±4 拍、非有限值回落 0；`legato` 原样保留。
    #[test]
    fn note_normalization_clamps_the_offset() {
        let base = Note { id: 1, pitch: 60, start: 0.0, duration: 1.0, velocity: 100, volume: 127, offset: 0.0, legato: false };
        assert_eq!(Note::normalize(Note { offset: 99.0, ..base }).offset, Note::MAX_OFFSET_BEATS);
        assert_eq!(Note::normalize(Note { offset: -99.0, ..base }).offset, -Note::MAX_OFFSET_BEATS);
        assert_eq!(Note::normalize(Note { offset: f64::NAN, ..base }).offset, 0.0);
        assert!(Note::normalize(Note { legato: true, ..base }).legato);
    }

    /// 扫弦 / 连奏要能往返；缺省的 0 / false 则**不写进 JSON**（旧工程形状逐字不变）。
    #[test]
    fn offset_and_legato_round_trip_and_stay_optional() {
        let json = r#"{"pitch":60,"start":0.0,"duration":1.0,"velocity":100}"#;
        let n: Note = serde_json::from_str(json).unwrap();
        assert_eq!((n.offset, n.legato), (0.0, false), "旧工程缺字段时按「同时发声、不连奏」读入");

        let plain = serde_json::to_string(&n).unwrap();
        assert!(!plain.contains("offset") && !plain.contains("legato"), "缺省值不该进 JSON：{plain}");

        let strummed = Note { offset: 0.0625, legato: true, ..n };
        let back: Note = serde_json::from_str(&serde_json::to_string(&strummed).unwrap()).unwrap();
        assert_eq!(back, strummed);
    }

    #[test]
    fn old_projects_without_volume_load_at_full_level() {
        // 旧工程 JSON 里没有 volume 字段——必须按「不衰减」读入，听感不变。
        let json = r#"{"pitch":60,"start":0.0,"duration":1.0,"velocity":100}"#;
        let n: Note = serde_json::from_str(json).expect("旧格式必须仍可解析");
        assert_eq!(n.volume, Note::MAX_VOLUME);
    }

    #[test]
    fn clip_normalization_is_bounded() {
        assert_eq!(Clip::normalize_length(0.0), Clip::MIN_LENGTH_BEATS);
        assert_eq!(Clip::normalize_length(1e9), Clip::MAX_LENGTH_BEATS);
        assert_eq!(Clip::normalize_length(f64::NAN), Clip::MIN_LENGTH_BEATS);
        assert_eq!(Clip::normalize_start(-5.0), 0.0);
        assert_eq!(Clip::normalize_start(f64::NAN), 0.0);
    }

    /// MIDI 片段的输出形状必须与「没有 `audio` 字段」的时代**逐字一致**。
    #[test]
    fn midi_clip_omits_the_audio_key() {
        let clip = Clip { id: ClipId(1), start: 0.0, length: 1.0, notes: vec![], audio: None };
        let json = serde_json::to_string(&clip).unwrap();
        assert!(!json.contains("audio"), "MIDI 片段不该写出 audio：{json}");
    }

    /// 旧工程（没有 `audio` 字段）必须照旧可读。
    #[test]
    fn clips_without_audio_load_as_midi() {
        let json = r#"{"id":1,"start":0.0,"length":1.0,"notes":[]}"#;
        let clip: Clip = serde_json::from_str(json).unwrap();
        assert!(!clip.is_audio());
        assert_eq!(clip.audio, None);
    }

    #[test]
    fn audio_clip_roundtrips_and_normalizes_notes_away() {
        let clip = Clip {
            id: ClipId(2),
            start: -3.0,
            length: 0.0,
            notes: vec![Note { id: 0, pitch: 60, start: 0.0, duration: 1.0, velocity: 100, volume: 127, offset: 0.0, legato: false }],
            audio: Some(AudioSource {
                path: "/tmp/kick.wav".into(),
                offset_frames: 0,
                frames: Some(480),
                rate: 1.0,
                pitch_semitones: 0.0,
            }),
        };
        let back: Clip = serde_json::from_str(&serde_json::to_string(&clip).unwrap()).unwrap();
        assert_eq!(back, clip);

        let n = Clip::normalize(clip);
        assert!(n.is_audio());
        assert!(n.notes.is_empty(), "音频片段不得同时带音符");
        assert_eq!(n.start, 0.0);
        assert_eq!(n.length, Clip::MIN_LENGTH_BEATS);
    }

    /// `offset_frames` 缺省时按 0 读入（手写 JSON / 旧版本），`frames` 缺省为「到结尾」。
    #[test]
    fn audio_source_offset_defaults_to_zero() {
        let json = r#"{"id":1,"start":0.0,"length":1.0,"notes":[],"audio":{"path":"a.wav"}}"#;
        let clip: Clip = serde_json::from_str(json).unwrap();
        let audio = clip.audio.unwrap();
        assert_eq!(audio.offset_frames, 0);
        assert_eq!(audio.frames, None, "旧工程没有窗口长度 → 到素材结尾");
    }

    /// 没有窗口长度时不该写进 JSON（保持旧工程形状）；缺省速度 / 升降调也不写。
    #[test]
    fn audio_source_omits_frames_when_absent() {
        let src = AudioSource {
            path: "a.wav".into(),
            offset_frames: 0,
            frames: None,
            rate: 1.0,
            pitch_semitones: 0.0,
        };
        assert_eq!(serde_json::to_string(&src).unwrap(), r#"{"path":"a.wav","offset_frames":0}"#);
    }

    /// 旧工程没有 rate/pitch → 按原速 / 不变调读入。
    #[test]
    fn audio_source_rate_defaults_without_json_keys() {
        let json = r#"{"path":"a.wav","offset_frames":10}"#;
        let src: AudioSource = serde_json::from_str(json).unwrap();
        assert_eq!(src.rate, 1.0);
        assert_eq!(src.pitch_semitones, 0.0);
    }

    /// 非缺省的速度 / 升降调会写进 JSON 并能往返；越界 / 非有限值被归一化。
    #[test]
    fn audio_source_rate_and_pitch_roundtrip_and_normalize() {
        let src = AudioSource {
            path: "a.wav".into(),
            offset_frames: 0,
            frames: None,
            rate: 2.5,
            pitch_semitones: -12.0,
        };
        let json = serde_json::to_string(&src).unwrap();
        assert!(json.contains("\"rate\":2.5"), "{json}");
        assert!(json.contains("\"pitch_semitones\":-12.0"), "{json}");
        assert_eq!(serde_json::from_str::<AudioSource>(&json).unwrap(), src);

        let hi = AudioSource { rate: 99.0, pitch_semitones: 99.0, ..src.clone() };
        let n = hi.normalize();
        assert_eq!(n.rate, AudioSource::MAX_RATE);
        assert_eq!(n.pitch_semitones, AudioSource::MAX_PITCH_SEMITONES);

        let nan = AudioSource { rate: f64::NAN, pitch_semitones: f64::NAN, ..src };
        let n = nan.normalize();
        assert_eq!(n.rate, 1.0, "非有限速度应回落原速");
        assert_eq!(n.pitch_semitones, 0.0);
    }
}
