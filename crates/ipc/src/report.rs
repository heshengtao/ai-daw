//! 分析结果类型：只读工具（统计 / 重叠 / 和声检查）的输出形状。
//!
//! 这些类型**不进工程、不进撤销历史**——它们只是把一次计算的结果说清楚。放在契约层是因为
//! MCP 工具、CLI 与界面都要解析同一份形状；计算本身在 `ai-daw-engine` 里（纯函数）。
//!
//! 全部字段名都是 snake_case，与工程 JSON 一致；`u64` 的轨道 id 序列化成裸数字。

use serde::{Deserialize, Serialize};

/// 严重程度（`Info < Warning < Error`，取最大值即「这一小节有多脏」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// 只是提示：经过音、允许的延伸音。
    Info,
    /// 需要注意：长时间的和弦外音、音区打架。
    Warning,
    /// 明显问题：不协和且没有解决。
    Error,
}

/// 每小节的音符密度。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BarDensity {
    /// 第几小节（0 起）。
    pub bar: usize,
    /// 小节起始拍（**工程绝对拍**）。
    pub start: f64,
    /// 起点落在该小节的音符数。
    pub count: usize,
}

/// 单条轨道（或其中某个片段）的音符统计。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoteStats {
    pub track: u64,
    /// 只统计了某个片段时给出它的 id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<u64>,
    pub note_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lowest_pitch: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highest_pitch: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_pitch: Option<f64>,
    /// 音域跨度（半音）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_semitones: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_velocity: Option<f64>,
    /// 音符覆盖的拍数（首尾跨度）。
    pub span_beats: f64,
    /// 平均每小节音符数。
    pub mean_notes_per_bar: f64,
    pub bars: Vec<BarDensity>,
    /// 同音高、时间上互相重叠的音符对数（同轨自我掩蔽）。
    pub overlapping_notes: usize,
    /// 起点落在片段之外的音符数（会被裁掉、听不见）。
    pub notes_outside_clip: usize,
}

/// 两条轨道之间的音区重叠。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackOverlap {
    pub a: u64,
    pub b: u64,
    /// 两条轨道**同时发声**的总拍数。
    pub overlap_beats: f64,
    /// 音高完全相同、且时间重叠的音符对数（最容易糊在一起）。
    pub unison_notes: usize,
    /// 同时发声时共用的音级（0..=11，C=0）。
    pub shared_pitch_classes: Vec<u8>,
    /// 两条轨道平均音高之差（半音，`a - b`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch_gap_semitones: Option<i32>,
    /// 是否构成分工冲突（同音区 + 大量同时发声）。
    pub conflict: bool,
    /// 可读建议。
    pub suggestion: String,
}

/// 跨轨重叠报告。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverlapReport {
    /// 参与分析的轨道（按传入顺序）。
    pub tracks: Vec<u64>,
    pub pairs: Vec<TrackOverlap>,
    /// 整体建议（没有冲突时为空）。
    pub summary: Vec<String>,
}

/// 和声检查的宽松策略。
///
/// 默认值是「正常流行编曲」的尺度：允许延伸音与短经过音——这两条正是「不要把正常写法误报成
/// 错误」的关键。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HarmonyPolicy {
    /// 允许 9 / 11 / 13 这类延伸音。
    pub allow_extensions: bool,
    /// 允许短经过音 / 辅助音（时值不超过 `allow_non_chord_tone_duration`）。
    pub allow_approach_notes: bool,
    /// 短经过音的时值上限（拍）。
    pub allow_non_chord_tone_duration: f64,
}

impl Default for HarmonyPolicy {
    fn default() -> Self {
        HarmonyPolicy {
            allow_extensions: true,
            allow_approach_notes: true,
            allow_non_chord_tone_duration: 0.5,
        }
    }
}

impl HarmonyPolicy {
    /// 归一化：时值上限夹到 0..=8 拍，非有限值回落默认。
    pub fn normalize(self) -> Self {
        let d = if self.allow_non_chord_tone_duration.is_finite() {
            self.allow_non_chord_tone_duration.clamp(0.0, 8.0)
        } else {
            0.5
        };
        HarmonyPolicy { allow_non_chord_tone_duration: d, ..self }
    }
}

/// 和声问题类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarmonyIssueKind {
    /// 和弦外音（既不属和弦，也不在允许的延伸音里）。
    NonChordTone,
    /// 导音没有解决到主音。
    UnresolvedLeadingTone,
    /// 悬挂音：从小节前延续进来、且不属于当前和弦。
    Suspension,
    /// 不推荐的延伸音（例如在小三和弦上叠大七度）。
    Extension,
    /// 轨道之间在同一音区重叠。
    TrackOverlap,
}

/// 一条和声问题。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarmonyIssue {
    pub kind: HarmonyIssueKind,
    pub severity: Severity,
    /// 第几小节（0 起）。
    pub bar: usize,
    /// 小节起始拍（工程绝对拍）。
    pub bar_start: f64,
    /// 小节内拍位（0 起）。
    pub beat: f64,
    pub track: u64,
    pub pitch: u8,
    /// 音名，如 `F#4`（读报告时不必自己算）。
    pub pitch_name: String,
    /// 持续时值（拍）。
    pub duration: f64,
    /// 可读解释。
    pub message: String,
}

/// 每个小节的和声判定。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarmonyBarReport {
    pub bar: usize,
    pub start: f64,
    pub length: f64,
    pub symbol: String,
    /// 构成音的音名（读报告用）。
    pub chord_tones: Vec<String>,
    /// 该小节的冲突等级。
    pub tension: Severity,
    pub issues: usize,
}

/// 和声检查报告。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarmonyReport {
    pub bars: Vec<HarmonyBarReport>,
    pub issues: Vec<HarmonyIssue>,
    /// 全曲最高冲突等级。
    pub conflict_level: Severity,
    /// 全局提示（例如某条轨道整段都不在和弦内）。
    pub notes: Vec<String>,
}

/// MIDI 音高 → 音名（`60` → `C4`，`61` → `C#4`）。
pub fn pitch_name(pitch: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[(pitch % 12) as usize], (pitch as i16) / 12 - 1)
}

/// 音级（0..=11）→ 音名（不带八度）。
pub fn pitch_class_name(pc: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    NAMES[(pc % 12) as usize].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitch_names_use_midi_octaves() {
        assert_eq!(pitch_name(60), "C4");
        assert_eq!(pitch_name(61), "C#4");
        assert_eq!(pitch_name(0), "C-1");
        assert_eq!(pitch_name(127), "G9");
    }

    #[test]
    fn severity_orders_info_below_error() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
    }
}
