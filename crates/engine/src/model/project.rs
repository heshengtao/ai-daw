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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::track::{MixerChannel, Track};
    use crate::model::Clip;

    #[test]
    fn project_roundtrips_through_json() {
        let mut p = Project::new("demo");
        p.tracks.push(Track {
            id: TrackId(1),
            name: "Lead".into(),
            instrument: None,
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
            clips: vec![Clip { id: crate::model::ClipId(1), start: 2.0, length: 3.0, notes: vec![] }],
            mixer: MixerChannel::default(),
        });
        assert_eq!(p.length_beats(), 5.0);
    }
}
