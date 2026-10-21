use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::clip::Clip;

/// 轨道标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TrackId(pub u64);

/// 插件实例（工程内保存的插件引用与状态）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginInstance {
    pub plugin_id: String,
    pub params: BTreeMap<String, f32>,
    pub state: Vec<u8>,
}

/// 混音通道。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MixerChannel {
    pub gain_db: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
}

impl MixerChannel {
    /// 增益下界（dB）：约等于静音，避免 -inf 进入增益换算。
    pub const MIN_GAIN_DB: f32 = -60.0;
    /// 增益上界（dB）：+12 dB 足够补偿，同时挡住会削爆输出的夸张值。
    pub const MAX_GAIN_DB: f32 = 12.0;

    /// 归一化增益；非有限值回落到 0 dB。
    pub fn normalize_gain_db(db: f32) -> f32 {
        if db.is_finite() {
            db.clamp(Self::MIN_GAIN_DB, Self::MAX_GAIN_DB)
        } else {
            0.0
        }
    }

    /// 归一化声像；非有限值回落到居中。
    pub fn normalize_pan(pan: f32) -> f32 {
        if pan.is_finite() {
            pan.clamp(-1.0, 1.0)
        } else {
            0.0
        }
    }
}

impl Default for MixerChannel {
    fn default() -> Self {
        MixerChannel { gain_db: 0.0, pan: 0.0, mute: false, solo: false }
    }
}

/// 一路发送（aux send）：把本轨的**推子后**信号按增益送进目标轨道。
///
/// 目标通常是一条「总线」轨道（一条没有片段、专门汇总的轨道）。总线本身也照常送往 master，
/// 所以「发送」只是**额外**复制一份信号过去。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Send {
    /// 目标轨道。
    pub target: TrackId,
    /// 发送增益（dB）；归一化到 `MixerChannel` 的区间。
    pub gain_db: f32,
}

impl Send {
    /// 归一化发送增益（非有限值 → 0 dB）。
    pub fn normalize(mut self) -> Self {
        self.gain_db = MixerChannel::normalize_gain_db(self.gain_db);
        self
    }
}

/// 自动化曲线上的一个关键点。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AutomationPoint {
    /// 位置（拍）。
    pub beat: f64,
    /// 该点的参数值。
    pub value: f32,
}

/// 一条自动化车道：一个参数 + 一条折线（相邻点之间线性插值）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationLane {
    /// 参数名：v1 支持 `"gain_db"` 与 `"pan"`（未知参数在求值时被忽略）。
    pub param: String,
    /// 关键点（按 beat 升序）。空表示「无自动化」。
    pub points: Vec<AutomationPoint>,
}

impl AutomationLane {
    /// 归一化参数名（去首尾空白）。空名字表示无效车道。
    pub fn normalize_param(param: &str) -> String {
        param.trim().to_string()
    }

    /// 在 `beat` 处求值（线性插值；两端用最近点夹住）。空曲线返回 `None`。
    pub fn value_at(&self, beat: f64) -> Option<f32> {
        lane_value_at(&self.points, beat)
    }
}

/// 在折线上求值：相邻点线性插值，两端用最近点夹住。空返回 `None`。
///
/// 纯算术，**不分配**——音频线程逐块求值会用到它。
pub fn lane_value_at(points: &[AutomationPoint], beat: f64) -> Option<f32> {
    let first = points.first()?;
    if beat <= first.beat {
        return Some(first.value);
    }
    let last = points.last()?;
    if beat >= last.beat {
        return Some(last.value);
    }
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if beat >= a.beat && beat <= b.beat {
            let span = b.beat - a.beat;
            if span <= f64::EPSILON {
                return Some(b.value);
            }
            let t = ((beat - a.beat) / span) as f32;
            return Some(a.value + (b.value - a.value) * t);
        }
    }
    Some(last.value)
}

/// 轨道目标音域（MIDI 音高，含两端）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PitchRange {
    pub min: u8,
    pub max: u8,
}

impl PitchRange {
    /// 归一化：夹到 0..=127，并保证 `min <= max`。
    pub fn normalize(self) -> Self {
        let min = self.min.min(127);
        let max = self.max.min(127);
        if min <= max {
            PitchRange { min, max }
        } else {
            PitchRange { min: max, max: min }
        }
    }

    /// 该音高是否落在音域内。
    pub fn contains(&self, pitch: u8) -> bool {
        pitch >= self.min && pitch <= self.max
    }
}

/// 编曲元数据：轨道在曲子里的**分工**与目标音域。
///
/// 全部可选、全部不写进 JSON 当为空——旧工程的文件形状逐字不变。工具在没有这些字段时
/// 仍然接受显式参数，所以它们是「备忘 + 默认值」，不是必需品。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TrackMeta {
    /// 分工：`lead` / `chords` / `bass` / `drums` / `pad` / `fx`…（自由字符串）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// 目标音域；`fit_notes_to_range` 不传参数时用它。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch_range: Option<PitchRange>,
    /// MIDI 弯音范围（半音，默认 2.0）。音源自身范围不同时改这里。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch_bend_range: Option<f32>,
}

impl TrackMeta {
    /// 默认弯音范围（半音）：General MIDI 与多数音源的标准值。
    pub const DEFAULT_PITCH_BEND_RANGE: f32 = 2.0;
    /// 弯音范围下界（半音）。
    pub const MIN_PITCH_BEND_RANGE: f32 = 0.01;
    /// 弯音范围上界（半音）。
    pub const MAX_PITCH_BEND_RANGE: f32 = 24.0;

    /// 是否什么都没设（用于决定要不要序列化）。
    pub fn is_empty(&self) -> bool {
        self.role.is_none() && self.pitch_range.is_none() && self.pitch_bend_range.is_none()
    }

    /// 归一化：夹紧可选的弯音范围，非有限值直接丢弃。
    pub fn normalized(mut self) -> Self {
        self.pitch_bend_range = self.pitch_bend_range.and_then(|range| {
            range
                .is_finite()
                .then(|| range.clamp(Self::MIN_PITCH_BEND_RANGE, Self::MAX_PITCH_BEND_RANGE))
        });
        self
    }

    /// 实际生效的弯音范围（缺省 2 个半音；手改工程里的越界值也会被夹住）。
    pub fn effective_pitch_bend_range(&self) -> f32 {
        self.pitch_bend_range
            .filter(|range| range.is_finite())
            .unwrap_or(Self::DEFAULT_PITCH_BEND_RANGE)
            .clamp(Self::MIN_PITCH_BEND_RANGE, Self::MAX_PITCH_BEND_RANGE)
    }
}

/// 轨道。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub name: String,
    pub instrument: Option<PluginInstance>,
    /// 效果器链：按顺序处理（乐器输出 → effects[0] → effects[1] → … → 轨道混音）。
    /// 旧工程没有这个字段时读作空链。
    #[serde(default)]
    pub effects: Vec<PluginInstance>,
    pub clips: Vec<Clip>,
    pub mixer: MixerChannel,
    /// 发送（aux send）。旧工程缺省为空，且空时不写进 JSON（保持旧工程形状）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sends: Vec<Send>,
    /// 自动化车道。旧工程缺省为空，且空时不写进 JSON。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub automation: Vec<AutomationLane>,
    /// 编曲元数据（分工 / 目标音域）。缺省或为空时不写进 JSON。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<TrackMeta>,
}

impl Track {
    /// 轨道分工（没有元数据时为 `None`）。
    pub fn role(&self) -> Option<&str> {
        self.meta.as_ref().and_then(|m| m.role.as_deref())
    }

    /// 轨道目标音域（没有元数据时为 `None`）。
    pub fn pitch_range(&self) -> Option<PitchRange> {
        self.meta.as_ref().and_then(|m| m.pitch_range)
    }
}

#[cfg(test)]
mod tests {
    use super::{lane_value_at, AutomationPoint, MixerChannel, Send, TrackId};

    #[test]
    fn gain_and_pan_are_normalized() {
        assert_eq!(MixerChannel::normalize_gain_db(100.0), MixerChannel::MAX_GAIN_DB);
        assert_eq!(MixerChannel::normalize_gain_db(-100.0), MixerChannel::MIN_GAIN_DB);
        assert_eq!(MixerChannel::normalize_gain_db(f32::NAN), 0.0);
        assert_eq!(MixerChannel::normalize_pan(3.0), 1.0);
        assert_eq!(MixerChannel::normalize_pan(-3.0), -1.0);
        assert_eq!(MixerChannel::normalize_pan(f32::NAN), 0.0);
    }

    #[test]
    fn lane_value_interpolates_and_clamps_at_both_ends() {
        let points = [
            AutomationPoint { beat: 0.0, value: 0.0 },
            AutomationPoint { beat: 4.0, value: -12.0 },
        ];
        assert_eq!(lane_value_at(&points, -1.0), Some(0.0), "起点前夹住");
        assert_eq!(lane_value_at(&points, 0.0), Some(0.0));
        assert_eq!(lane_value_at(&points, 2.0), Some(-6.0), "中点线性插值");
        assert_eq!(lane_value_at(&points, 4.0), Some(-12.0));
        assert_eq!(lane_value_at(&points, 9.0), Some(-12.0), "终点后夹住");
        assert_eq!(lane_value_at(&[], 0.0), None, "空曲线没有值");
        // 单点：处处取该值。
        assert_eq!(lane_value_at(&[AutomationPoint { beat: 3.0, value: 0.5 }], 0.0), Some(0.5));
    }

    #[test]
    fn send_normalizes_gain() {
        assert_eq!(Send { target: TrackId(2), gain_db: 999.0 }.normalize().gain_db, MixerChannel::MAX_GAIN_DB);
        assert_eq!(Send { target: TrackId(2), gain_db: f32::NAN }.normalize().gain_db, 0.0);
    }
}
