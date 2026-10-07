use crate::model::{ClipId, Note, TrackId};

/// 命令：期望的状态变更请求。
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// 设置工程速度（BPM）。
    SetTempo(f64),
    /// 新增一条轨道。
    AddTrack { name: String },
    /// 删除一条轨道。
    RemoveTrack(TrackId),
    /// 为轨道设置乐器插件。
    SetTrackInstrument { track: TrackId, plugin_id: String },
    /// 在轨道上新增一个 MIDI 片段。
    AddClip { track: TrackId, start: f64, length: f64 },
    /// 从轨道上删除一个片段。
    RemoveClip { track: TrackId, clip: ClipId },
    /// 在片段的指定位置新增一个音符。
    AddNote { track: TrackId, clip: ClipId, note: Note },
    /// 设置轨道增益（dB）。
    SetGain { track: TrackId, gain_db: f32 },
    /// 设置轨道声像（-1.0 最左，1.0 最右）。
    SetPan { track: TrackId, pan: f32 },
}
