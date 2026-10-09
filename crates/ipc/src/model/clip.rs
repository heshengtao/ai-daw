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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub pitch: u8,
    pub start: f64,
    pub duration: f64,
    pub velocity: u8,
    /// 单音音量（0..=127，127 = 不衰减）。旧工程文件缺这个字段时按 127 读入。
    #[serde(default = "default_note_volume")]
    pub volume: u8,
}

/// 旧工程文件（以及手写的 JSON）可能没有 `volume` 字段——按「不衰减」读入，
/// 这样老工程的听感完全不变。
fn default_note_volume() -> u8 {
    Note::MAX_VOLUME
}

impl Note {
    /// 最短时长（拍）＝一个 32 分音符，避免出现零长度音符（会让 NoteOn/NoteOff 同帧）。
    pub const MIN_DURATION_BEATS: f64 = 1.0 / 32.0;
    /// 最长时长（拍）：64 小节 4/4，足够容纳任何合法输入，同时挡住天文数字。
    pub const MAX_DURATION_BEATS: f64 = 256.0;
    /// 起点上界（拍）：约 250000 小节，仅用于挡住非有限/荒谬输入。
    pub const MAX_START_BEATS: f64 = 1_000_000.0;
    /// 音量的上界（= 不衰减）。
    pub const MAX_VOLUME: u8 = 127;

    /// 归一化：音高/力度/音量夹到 MIDI 0..=127，起点非负且有上界，时长落在合法区间。
    ///
    /// 手写工程、旧文件、AI 代理都可能送来越界值；所有写入路径都必须经过这里。
    pub fn normalize(note: Note) -> Note {
        Note {
            pitch: note.pitch.min(127),
            start: clamp_beats(note.start, Self::MAX_START_BEATS),
            duration: clamp_range(note.duration, Self::MIN_DURATION_BEATS, Self::MAX_DURATION_BEATS),
            velocity: note.velocity.min(127),
            volume: note.volume.min(Self::MAX_VOLUME),
        }
    }
}

/// 音频素材引用：片段指向一段外部音频文件。
///
/// v1 存**绝对路径**（录音落在应用数据目录、导入的文件沿用原位置）；素材库/相对路径留后续。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioSource {
    /// 素材文件路径。
    pub path: String,
    /// 素材内的起始帧（crop 起点）。旧工程/手写 JSON 缺省时按 0 读入。
    #[serde(default)]
    pub offset_frames: u64,
    /// 素材窗口长度（源帧）：从 `offset_frames` 起**取多少帧**铺满整个片段。
    ///
    /// `None` = 取到素材结尾（旧工程/手写 JSON 缺省）。切分音频片段时必须知道窗口长度，
    /// 否则「左半段」会一路播到结尾、被拉伸变调——所以导入时会把素材总帧数写进来。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<u64>,
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NoteEdit {
    pub index: usize,
    pub note: Note,
}

impl Clip {
    /// 最短长度（拍）＝一个 32 分音符。
    pub const MIN_LENGTH_BEATS: f64 = 1.0 / 32.0;
    /// 最长长度（拍）：64 小节 4/4。
    pub const MAX_LENGTH_BEATS: f64 = 256.0;
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
    /// （两种内容不能共存，否则播放与界面会各画一半）。
    pub fn normalize(mut clip: Clip) -> Clip {
        clip.start = Self::normalize_start(clip.start);
        clip.length = Self::normalize_length(clip.length);
        if clip.audio.is_some() {
            clip.notes.clear();
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
        let n = Note::normalize(Note { pitch: 200, start: -3.0, duration: 0.0, velocity: 250, volume: 250 });
        assert_eq!(n.pitch, 127);
        assert_eq!(n.start, 0.0);
        assert_eq!(n.duration, Note::MIN_DURATION_BEATS);
        assert_eq!(n.velocity, 127);
        assert_eq!(n.volume, Note::MAX_VOLUME);
    }

    #[test]
    fn note_normalization_rejects_non_finite() {
        let n = Note::normalize(Note { pitch: 60, start: f64::NAN, duration: f64::INFINITY, velocity: 100, volume: 127 });
        // 非有限的起点回落到 0；非有限的时长回落到最短时长（而不是让它永远挂着）。
        assert_eq!(n.start, 0.0);
        assert_eq!(n.duration, Note::MIN_DURATION_BEATS);
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
            notes: vec![Note { pitch: 60, start: 0.0, duration: 1.0, velocity: 100, volume: 127 }],
            audio: Some(AudioSource { path: "/tmp/kick.wav".into(), offset_frames: 0, frames: Some(480) }),
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

    /// 没有窗口长度时不该写进 JSON（保持旧工程形状）。
    #[test]
    fn audio_source_omits_frames_when_absent() {
        let src = AudioSource { path: "a.wav".into(), offset_frames: 0, frames: None };
        assert_eq!(serde_json::to_string(&src).unwrap(), r#"{"path":"a.wav","offset_frames":0}"#);
    }
}
