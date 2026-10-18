use serde::{Deserialize, Serialize};

use super::track::{MixerChannel, Track, TrackId};
use super::transport::Transport;

/// 一个小节的和弦标记（编曲元数据）。
///
/// v1 不强制工程带它：`analyze_harmony` 也接受调用时显式传入的 `bars`。存进工程只是让
/// 「和弦谱」跟着工程走，AI 下次不必再问一遍。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarmonyBar {
    /// 起始拍。
    pub start: f64,
    /// 长度（拍）；缺省由下一小节推出。
    pub length: f64,
    /// 和弦符号，如 `Dm9` / `G13` / `Cmaj7`。
    pub symbol: String,
    /// 来源（`manual` / `ai` / …）。缺省不写。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// 段落标记（前奏 / 主歌 / 副歌…）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub name: String,
    /// 起始拍。
    pub start: f64,
    /// 结束拍（不含）。
    pub end: f64,
    /// 角色：`intro` / `verse` / `chorus` / `bridge` / `outro`…。缺省不写。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// 工程：领域聚合根。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub transport: Transport,
    pub tracks: Vec<Track>,
    pub master: MixerChannel,
    /// 小节和弦表。空时不写进 JSON（旧工程形状不变）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub harmony: Vec<HarmonyBar>,
    /// 段落表。空时不写进 JSON。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<Section>,
}

impl Project {
    /// 创建空工程：默认传输设置（120 BPM、4/4）、无轨道、默认总输出通道。
    pub fn new(name: impl Into<String>) -> Self {
        Project {
            name: name.into(),
            transport: Transport::default(),
            tracks: vec![],
            master: MixerChannel::default(),
            harmony: vec![],
            sections: vec![],
        }
    }

    /// 按 id 查找轨道。
    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    /// 按 id 可变查找轨道。
    pub fn track_mut(&mut self, id: TrackId) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    /// 工程内容总时长（拍），取所有片段结束位置的最大值。
    pub fn length_beats(&self) -> f64 {
        self.tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.start + c.length)
            .fold(0.0f64, f64::max)
    }

    /// 被音频片段引用到的素材路径（去重、升序）。
    ///
    /// 宿主据此在**构建调度表之前**把素材加载好（解码是 IO，绝不能发生在音频回调里）。
    pub fn audio_paths(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .filter_map(|c| c.audio.as_ref().map(|a| a.path.clone()))
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// 给缺 id 的音符补号（旧工程 / 手写 JSON 的 `id` 全是 0）。
    ///
    /// 按**片段内顺序**发号，因此同一个旧工程每次加载得到的 id 完全一致——「稳定」指的就是
    /// 这个：一次会话内不会被后续编辑改动，跨会话也可复现。
    pub fn ensure_note_ids(&mut self) {
        for track in &mut self.tracks {
            for clip in &mut track.clips {
                let mut next = clip.notes.iter().map(|n| n.id).max().unwrap_or(0) + 1;
                let mut seen = std::collections::HashSet::new();
                for note in &mut clip.notes {
                    if note.id == 0 || !seen.insert(note.id) {
                        note.id = next;
                        next += 1;
                        seen.insert(note.id);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::track::{MixerChannel, Track};
    use crate::model::{Clip, ClipId, Note};

    #[test]
    fn project_roundtrips_through_json() {
        let mut p = Project::new("demo");
        p.tracks.push(Track {
            meta: None,
            id: TrackId(1),
            name: "Lead".into(),
            instrument: None,
            effects: vec![],
            clips: vec![],
            mixer: MixerChannel::default(),
            sends: vec![],
            automation: vec![],
        });
        let json = serde_json::to_string_pretty(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
        assert_eq!(back.tracks[0].name, "Lead");
    }

    #[test]
    fn length_beats_is_max_clip_end() {
        let mut p = Project::new("demo");
        p.tracks.push(Track {
            meta: None,
            id: TrackId(1),
            name: "Lead".into(),
            instrument: None,
            effects: vec![],
            clips: vec![Clip { id: crate::model::ClipId(1), start: 2.0, length: 3.0, notes: vec![], audio: None }],
            mixer: MixerChannel::default(),
            sends: vec![],
            automation: vec![],
        });
        assert_eq!(p.length_beats(), 5.0);
    }

    #[test]
    fn project_wire_shape_is_stable() {
        // `src/api/types.ts` 依赖 TrackId / ClipId 序列化成裸数字、字段为 snake_case；
        // 往返测试无法发现形状漂移，这里断言具体 JSON 形状。
        let mut p = Project::new("t");
        p.tracks.push(Track {
            meta: None,
            id: TrackId(7),
            name: "Lead".into(),
            instrument: None,
            effects: vec![],
            clips: vec![Clip {
                id: ClipId(3),
                start: 0.0,
                length: 1.0,
                notes: vec![Note { id: 0, pitch: 60, start: 0.0, duration: 1.0, velocity: 100, volume: 127, offset: 0.0, legato: false }],
                audio: None,
            }],
            mixer: MixerChannel::default(),
            sends: vec![],
            automation: vec![],
        });
        let v = serde_json::to_value(&p).unwrap();
        assert!(v["tracks"][0]["id"].is_u64(), "TrackId must serialize as a bare number");
        assert!(v["tracks"][0]["clips"][0]["id"].is_u64(), "ClipId must serialize as a bare number");
        assert!(v["transport"]["time_sig_num"].is_u64(), "fields must be snake_case");
        assert!(v["tracks"][0]["mixer"]["gain_db"].is_f64());
        // 空的发送 / 自动化**不写进 JSON**：老工程形状逐字不变。
        assert!(v["tracks"][0].get("sends").is_none(), "空发送不该出现");
        assert!(v["tracks"][0].get("automation").is_none(), "空自动化不该出现");
    }

    #[test]
    fn sends_and_automation_roundtrip_and_serialize_when_present() {
        use crate::model::{AutomationLane, AutomationPoint, Send};
        let mut p = Project::new("t");
        p.tracks.push(Track {
            meta: None,
            id: TrackId(1),
            name: "Bus".into(),
            instrument: None,
            effects: vec![],
            clips: vec![],
            mixer: MixerChannel::default(),
            sends: vec![Send { target: TrackId(2), gain_db: -6.0 }],
            automation: vec![AutomationLane {
                param: "gain_db".into(),
                points: vec![AutomationPoint { beat: 0.0, value: 0.0 }],
            }],
        });
        let json = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["tracks"][0]["sends"][0]["gain_db"], -6.0);
        assert_eq!(v["tracks"][0]["automation"][0]["param"], "gain_db");
    }
}
