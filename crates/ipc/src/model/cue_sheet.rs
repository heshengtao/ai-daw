//! 影片清单导入计划：一次事务内建轨道 / 乐器 / 片段 / 段落 / 和弦。
//!
//! 这是 `import_cue_sheet` 与引擎之间的契约：MCP 层负责把 `film.json` 的秒换算成拍、
//! 解析音色名；引擎只负责按计划产生事件。所有 id 在同一次 `translate` 里分配，
//! 因此整份清单 = 一个撤销步。

use std::collections::BTreeMap;

use super::clip::Note;
use super::project::{HarmonyBar, Section};
use super::track::PitchRange;

/// 一份完整的导入计划。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CueSheetPlan {
    /// 先清空当前工程（含默认分轨模板）再导入；`false` = 追加到现有轨道后面。
    pub clear: bool,
    /// 工程名；`None` = 不改名。
    pub name: Option<String>,
    /// 工程速度（BPM）；`None` = 保持当前值。
    pub tempo: Option<f64>,
    /// 拍号；`None` = 保持当前值。
    pub time_sig: Option<(u8, u8)>,
    /// 段落表（整体替换）。
    pub sections: Vec<Section>,
    /// 小节和弦表（整体替换）。
    pub harmony: Vec<HarmonyBar>,
    /// 要新建的轨道（按顺序追加到末尾）。
    pub tracks: Vec<CueTrack>,
}

/// 一条要新建的轨道。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CueTrack {
    pub name: String,
    /// 乐器插件 id；`None` = 空轨，留给 agent 后补。
    pub plugin_id: Option<String>,
    /// 乐器参数补丁（在默认参数之上写入；预览音色 / 出厂音色的 params）。
    pub params: BTreeMap<String, f32>,
    /// 效果器插件 id（按顺序追加到效果链）。
    pub effects: Vec<String>,
    /// 编曲分工（lead / chords / bass / drums…）。
    pub role: Option<String>,
    /// 建议音域。
    pub pitch_range: Option<PitchRange>,
    /// 轨道增益（dB）；`None` = 保持默认。
    pub gain_db: Option<f32>,
    /// 轨道声像；`None` = 保持默认。
    pub pan: Option<f32>,
    /// 轨道上的片段（按清单顺序写入）。
    pub clips: Vec<CueClip>,
}

/// 一条要写入轨道的 MIDI 片段。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CueClip {
    /// 起始拍。
    pub start: f64,
    /// 长度（拍）。
    pub length: f64,
    /// 片段内音符；`start` 相对片段起点（拍）。
    pub notes: Vec<Note>,
}
