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

/// MIDI 片段。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    pub start: f64,
    pub length: f64,
    pub notes: Vec<Note>,
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
}
