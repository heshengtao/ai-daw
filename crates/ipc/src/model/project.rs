use serde::{Deserialize, Serialize};

use super::track::{MixerChannel, Track, TrackId};
use super::transport::Transport;

/// 工程：领域聚合根。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub transport: Transport,
    pub tracks: Vec<Track>,
    pub master: MixerChannel,
}

impl Project {
    /// 创建空工程：默认传输设置（120 BPM、4/4）、无轨道、默认总输出通道。
    pub fn new(name: impl Into<String>) -> Self {
        Project {
            name: name.into(),
            transport: Transport::default(),
            tracks: vec![],
            master: MixerChannel::default(),
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
            id: TrackId(1),
            name: "Lead".into(),
            instrument: None,
            effects: vec![],
            clips: vec![],
            mixer: MixerChannel::default(),
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
            id: TrackId(1),
            name: "Lead".into(),
            instrument: None,
            effects: vec![],
            clips: vec![Clip { id: crate::model::ClipId(1), start: 2.0, length: 3.0, notes: vec![], audio: None }],
            mixer: MixerChannel::default(),
        });
        assert_eq!(p.length_beats(), 5.0);
    }

    #[test]
    fn project_wire_shape_is_stable() {
        // `src/api/types.ts` 依赖 TrackId / ClipId 序列化成裸数字、字段为 snake_case；
        // 往返测试无法发现形状漂移，这里断言具体 JSON 形状。
        let mut p = Project::new("t");
        p.tracks.push(Track {
            id: TrackId(7),
            name: "Lead".into(),
            instrument: None,
            effects: vec![],
            clips: vec![Clip {
                id: ClipId(3),
                start: 0.0,
                length: 1.0,
                notes: vec![Note { pitch: 60, start: 0.0, duration: 1.0, velocity: 100, volume: 127 }],
                audio: None,
            }],
            mixer: MixerChannel::default(),
        });
        let v = serde_json::to_value(&p).unwrap();
        assert!(v["tracks"][0]["id"].is_u64(), "TrackId must serialize as a bare number");
        assert!(v["tracks"][0]["clips"][0]["id"].is_u64(), "ClipId must serialize as a bare number");
        assert!(v["transport"]["time_sig_num"].is_u64(), "fields must be snake_case");
        assert!(v["tracks"][0]["mixer"]["gain_db"].is_f64());
    }
}
