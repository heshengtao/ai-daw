use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::clip::ClipId;
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

/// 音符级标签：装饰音 / 奏法。**不改变播放**，只给分析器提供意图。
///
/// 放在工程级 `note_tags` 里而不是 `Note` 字段上，是为了避免给每个音符都加一个通常为空的
/// 可选字段——老工程与现有音符字面量形状完全不变。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct NoteTag {
    /// 装饰音类型：`grace` / `acciaccatura` / `mordent` / `trill`…（自由文本）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ornament: Option<String>,
    /// 奏法：`staccato` / `accent` / `legato` / `marcato`…（自由文本）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub articulation: Option<String>,
}

impl NoteTag {
    /// 两个字段都为空。
    pub fn is_empty(&self) -> bool {
        self.ornament.is_none() && self.articulation.is_none()
    }

    /// 去掉空白字段；空字符串视为未设置。
    pub fn normalized(mut self) -> Self {
        self.ornament = self
            .ornament
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        self.articulation = self
            .articulation
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        self
    }
}

/// `note_tags` 的键：`"track:clip:id"`。
pub fn note_tag_key(track: u64, clip: u64, id: u64) -> String {
    format!("{track}:{clip}:{id}")
}

/// 解析 `"track:clip:id"`；格式不对返回 `None`。
pub fn parse_note_tag_key(key: &str) -> Option<(u64, u64, u64)> {
    let mut parts = key.split(':');
    let track = parts.next()?.parse().ok()?;
    let clip = parts.next()?.parse().ok()?;
    let id = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((track, clip, id))
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
    /// 音符级标签（键 = `"track:clip:id"`）。空时不写进 JSON。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub note_tags: BTreeMap<String, NoteTag>,
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
            note_tags: BTreeMap::new(),
        }
    }

    /// 读取一个音符的标签。
    pub fn note_tag(&self, track: TrackId, clip: ClipId, id: u64) -> Option<&NoteTag> {
        self.note_tags.get(&note_tag_key(track.0, clip.0, id))
    }

    /// 写入 / 清除一个音符的标签（空标签 = 移除）。
    pub fn set_note_tag(&mut self, track: TrackId, clip: ClipId, id: u64, tag: Option<NoteTag>) {
        let key = note_tag_key(track.0, clip.0, id);
        match tag.map(NoteTag::normalized).filter(|t| !t.is_empty()) {
            Some(tag) => {
                self.note_tags.insert(key, tag);
            }
            None => {
                self.note_tags.remove(&key);
            }
        }
    }

    /// 删除一个音符的标签。
    pub fn remove_note_tag(&mut self, track: TrackId, clip: ClipId, id: u64) {
        self.note_tags.remove(&note_tag_key(track.0, clip.0, id));
    }

    /// 删除一个片段下所有音符的标签（整体替换 / 变成音频 / 片段被删时用）。
    pub fn remove_clip_note_tags(&mut self, track: TrackId, clip: ClipId) {
        let prefix = format!("{}:{}:", track.0, clip.0);
        self.note_tags.retain(|key, _| !key.starts_with(&prefix));
    }

    /// 删除一条轨道下所有音符的标签。
    pub fn remove_track_note_tags(&mut self, track: TrackId) {
        let prefix = format!("{}:", track.0);
        self.note_tags.retain(|key, _| !key.starts_with(&prefix));
    }

    /// 片段跨轨道移动时，把它的音符标签一起搬到新轨道。
    pub fn move_clip_note_tags(&mut self, from: TrackId, to: TrackId, clip: ClipId) {
        if from == to {
            return;
        }
        let from_prefix = format!("{}:{}:", from.0, clip.0);
        let to_prefix = format!("{}:{}:", to.0, clip.0);
        let moved: Vec<(String, NoteTag)> = self
            .note_tags
            .iter()
            .filter(|(key, _)| key.starts_with(&from_prefix))
            .map(|(key, tag)| {
                (format!("{to_prefix}{}", &key[from_prefix.len()..]), tag.clone())
            })
            .collect();
        let old_keys: Vec<String> = self
            .note_tags
            .keys()
            .filter(|key| key.starts_with(&from_prefix))
            .cloned()
            .collect();
        for key in old_keys {
            self.note_tags.remove(&key);
        }
        for (key, tag) in moved {
            self.note_tags.insert(key, tag);
        }
    }

    /// 切分片段时，把右半段音符的标签迁到新片段。
    ///
    /// 跨过切点的音符会在左右两半**共用同一个 id**；这种 id 要复制标签而不是移走，
    /// 否则左半段的标签会丢。
    pub fn move_note_tags_to_clip(
        &mut self,
        track: TrackId,
        from: ClipId,
        to: ClipId,
        right_ids: &[u64],
        left_ids: &[u64],
    ) {
        if from == to {
            return;
        }
        for id in right_ids {
            let from_key = note_tag_key(track.0, from.0, *id);
            if let Some(tag) = self.note_tags.get(&from_key).cloned() {
                self.note_tags.insert(note_tag_key(track.0, to.0, *id), tag);
                if !left_ids.contains(id) {
                    self.note_tags.remove(&from_key);
                }
            }
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
    fn note_tags_are_optional_and_roundtrip() {
        let mut p = Project::new("t");
        p.set_note_tag(
            TrackId(1),
            ClipId(2),
            3,
            Some(NoteTag { ornament: Some("grace".into()), articulation: None }),
        );
        assert_eq!(
            p.note_tag(TrackId(1), ClipId(2), 3).and_then(|t| t.ornament.as_deref()),
            Some("grace")
        );
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("note_tags"), "{json}");
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);

        p.set_note_tag(TrackId(1), ClipId(2), 3, None);
        assert!(p.note_tags.is_empty());
        let empty = serde_json::to_string(&Project::new("t")).unwrap();
        assert!(!empty.contains("note_tags"), "{empty}");
    }

    #[test]
    fn note_tag_keys_parse_and_reject_garbage() {
        assert_eq!(parse_note_tag_key("1:2:3"), Some((1, 2, 3)));
        assert_eq!(parse_note_tag_key("1:2"), None);
        assert_eq!(parse_note_tag_key("1:2:3:4"), None);
        assert_eq!(parse_note_tag_key("x:2:3"), None);
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
