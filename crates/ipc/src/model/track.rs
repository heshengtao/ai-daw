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
        MixerChannel {
            gain_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
        }
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

/// 一条自动化曲线上的一个关键点。
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

/// 通用 MIDI 事件表里的一条：绝对拍 + 通道 + 事件。
///
/// 这是「插件能做但宿主没有专门 UI」的逃生口：keyswitch、一次性 CC、program change、
/// 触后、逐音表达都可以直接写。调度期会按帧合并进轨道事件流。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MidiEventAt {
    /// 位置（绝对拍，自工程起点）。
    pub beat: f64,
    /// MIDI 通道（0..=15）；旧工程缺省 0。
    #[serde(default)]
    pub channel: u8,
    pub event: ai_daw_plugin_api::MidiEvent,
}

impl MidiEventAt {
    /// 归一化：拍非负、通道夹到 0..=15。
    pub fn normalized(self) -> Self {
        Self {
            beat: if self.beat.is_finite() {
                self.beat.max(0.0)
            } else {
                0.0
            },
            channel: self.channel.min(15),
            event: self.event,
        }
    }
}

/// 奏法切换用的 CC。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CcSwitch {
    pub controller: u8,
    pub value: u8,
}

impl CcSwitch {
    /// 归一化：控制器 / 值都夹到 0..=127。
    pub fn normalized(self) -> Self {
        Self {
            controller: self.controller.min(127),
            value: self.value.min(127),
        }
    }
}

/// 奏法切换里的单条 MIDI 消息。
///
/// 导入的映射（如 ArtiCluster Universal Definition）允许一条奏法发多条消息；
/// `offset_beats` 是从切换时刻起算的相对拍，`channel = None` 表示跟随音符通道。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArticulationEvent {
    /// 相对切换时刻的偏移（拍，非负）。
    #[serde(default, skip_serializing_if = "is_zero_beats")]
    pub offset_beats: f32,
    /// MIDI 通道（0..=15）；`None` = 跟随音符 / 轨道。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<u8>,
    pub event: ai_daw_plugin_api::MidiEvent,
}

fn is_zero_beats(value: &f32) -> bool {
    *value == 0.0
}

impl ArticulationEvent {
    /// 归一化：偏移非负且有限，通道夹到 0..=15。
    pub fn normalized(mut self) -> Self {
        self.offset_beats = if self.offset_beats.is_finite() {
            self.offset_beats
                .clamp(0.0, ArticulationSwitch::MAX_LEAD_BEATS)
        } else {
            0.0
        };
        self.channel = self.channel.map(|channel| channel.min(15));
        self
    }
}

/// 一条奏法映射：把音符上的 `articulation` 标签翻译成插件认识的 keyswitch / CC / program。
///
/// 键位与 CC 号完全由插件决定（不同音色库各不相同），所以这里不内置任何厂商数据；
/// 用户 / 代理按插件手册填写，或从映射库导入成 `events` 序列。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ArticulationSwitch {
    /// keyswitch 音符（0..=127）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyswitch: Option<u8>,
    /// CC 切换。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cc: Option<CcSwitch>,
    /// program change。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<u8>,
    /// 通用消息序列（导入映射用）。非空时优先于上面的简写字段。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<ArticulationEvent>,
    /// 提前量（拍）；缺省 1/64 拍。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead_beats: Option<f32>,
    /// true = 音符结束后切回 `default_articulation`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub momentary: Option<bool>,
}

impl ArticulationSwitch {
    /// 默认提前量（拍）：一整拍分成 64 份，足够在起音前完成切换。
    pub const DEFAULT_LEAD_BEATS: f32 = 1.0 / 64.0;
    /// 提前量上界（拍）。
    pub const MAX_LEAD_BEATS: f32 = 4.0;
    /// 单条奏法最多保存多少条消息（导入数据的护栏）。
    pub const MAX_EVENTS: usize = 16;
    /// keyswitch 的按住时长（拍）：只触发切换，不需要真的延音。
    pub const KEYSWITCH_HOLD_BEATS: f32 = 1.0 / 64.0;

    /// 是否什么也没配置。
    pub fn is_empty(&self) -> bool {
        self.keyswitch.is_none()
            && self.cc.is_none()
            && self.program.is_none()
            && self.events.is_empty()
    }

    /// 实际提前量（拍）。
    pub fn effective_lead(&self) -> f32 {
        self.lead_beats
            .filter(|v| v.is_finite())
            .unwrap_or(Self::DEFAULT_LEAD_BEATS)
            .clamp(0.0, Self::MAX_LEAD_BEATS)
    }

    /// 展开成实际要发送的消息序列。
    ///
    /// - `events` 非空：直接使用；某个 NoteOn 后面没有同音高 NoteOff 时，
    ///   自动补一个 [`Self::KEYSWITCH_HOLD_BEATS`] 之后的 NoteOff。
    /// - 否则由旧字段合成：keyswitch、CC、program 按此顺序。
    pub fn messages(&self) -> Vec<ArticulationEvent> {
        let source = if self.events.is_empty() {
            let mut legacy = Vec::new();
            if let Some(keyswitch) = self.keyswitch {
                legacy.push(ArticulationEvent {
                    offset_beats: 0.0,
                    channel: None,
                    event: ai_daw_plugin_api::MidiEvent::NoteOn {
                        note: keyswitch.min(127),
                        velocity: 1,
                    },
                });
            }
            if let Some(cc) = self.cc {
                legacy.push(ArticulationEvent {
                    offset_beats: 0.0,
                    channel: None,
                    event: ai_daw_plugin_api::MidiEvent::ControlChange {
                        controller: cc.controller,
                        value: cc.value,
                    },
                });
            }
            if let Some(program) = self.program {
                legacy.push(ArticulationEvent {
                    offset_beats: 0.0,
                    channel: None,
                    event: ai_daw_plugin_api::MidiEvent::ProgramChange { program },
                });
            }
            legacy
        } else {
            self.events.clone()
        };

        let mut out = Vec::with_capacity(source.len() * 2);
        for (index, item) in source.iter().enumerate() {
            out.push(item.clone());
            if let ai_daw_plugin_api::MidiEvent::NoteOn { note, .. } = item.event {
                let has_off = source[index + 1..].iter().any(|other| {
                    matches!(
                        other.event,
                        ai_daw_plugin_api::MidiEvent::NoteOff { note: other_note }
                            if other_note == note
                    )
                });
                if !has_off {
                    out.push(ArticulationEvent {
                        offset_beats: item.offset_beats + Self::KEYSWITCH_HOLD_BEATS,
                        channel: item.channel,
                        event: ai_daw_plugin_api::MidiEvent::NoteOff { note },
                    });
                }
            }
        }
        out
    }

    /// 归一化：音高 / program / CC 夹到 0..=127，事件序列截断到上限。
    pub fn normalized(self) -> Self {
        let events = self
            .events
            .into_iter()
            .take(Self::MAX_EVENTS)
            .map(ArticulationEvent::normalized)
            .collect();
        Self {
            keyswitch: self.keyswitch.map(|v| v.min(127)),
            cc: self.cc.map(CcSwitch::normalized),
            program: self.program.map(|v| v.min(127)),
            events,
            lead_beats: self.lead_beats,
            momentary: self.momentary,
        }
    }
}

/// 奏法映射的来源：导入自映射库时写入，用于界面提示与工程可追溯性。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ArticulationSource {
    /// 音色库厂商。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    /// 产品 / 音色库名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// patch 名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
    /// 映射库里的稳定 id（例如 ArtiCluster 的 UUID）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub map_id: Option<String>,
}

impl ArticulationSource {
    /// 归一化：去空白；所有字段都为空时返回 `None`。
    pub fn normalized(mut self) -> Option<Self> {
        let clean = |value: Option<String>| {
            value
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        self.manufacturer = clean(self.manufacturer);
        self.product = clean(self.product);
        self.patch = clean(self.patch);
        self.map_id = clean(self.map_id);
        if self.manufacturer.is_none()
            && self.product.is_none()
            && self.patch.is_none()
            && self.map_id.is_none()
        {
            None
        } else {
            Some(self)
        }
    }
}

/// MPE 区域：Lower 用主通道 1、成员 2..n+1；Upper 用主通道 16、成员 15..16-n。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MpeZone {
    Lower,
    Upper,
}

/// MPE 配置。
///
/// 宿主只负责把每颗音符分到成员通道并逐通道发弯音/压力；插件是否按 MPE 解释
/// 取决于它自己的开关。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MpeConfig {
    /// 成员通道数（1..=15）。
    #[serde(default = "default_mpe_channels")]
    pub channels: u8,
    #[serde(default = "default_mpe_zone")]
    pub zone: MpeZone,
}

fn default_mpe_channels() -> u8 {
    15
}

fn default_mpe_zone() -> MpeZone {
    MpeZone::Lower
}

impl MpeConfig {
    /// 成员通道数上界（主通道之外还有 15 个）。
    pub const MAX_CHANNELS: u8 = 15;

    /// 归一化：至少 1 个成员通道，最多 15 个。
    pub fn normalized(self) -> Self {
        Self {
            channels: self.channels.clamp(1, Self::MAX_CHANNELS),
            zone: self.zone,
        }
    }

    /// 该配置下可用的成员通道（0 基）。
    pub fn member_channels(self) -> Vec<u8> {
        let count = self.channels.clamp(1, Self::MAX_CHANNELS);
        match self.zone {
            // 主通道 = 0，成员从 1 开始。
            MpeZone::Lower => (1..=count).collect(),
            // 主通道 = 15，成员从 14 往下。
            MpeZone::Upper => (0..count).map(|i| 14 - i).collect(),
        }
    }

    /// 主通道（0 基）：Lower = 0，Upper = 15。
    pub fn master_channel(self) -> u8 {
        match self.zone {
            MpeZone::Lower => 0,
            MpeZone::Upper => 15,
        }
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
    /// MPE 配置；`None` = 关闭（所有音符走主通道 0）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mpe: Option<MpeConfig>,
    /// 奏法名 → keyswitch / CC / program 映射。空时不写进 JSON。
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub articulations: std::collections::BTreeMap<String, ArticulationSwitch>,
    /// 默认奏法（`momentary` 在音符结束后回到它）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_articulation: Option<String>,
    /// 奏法映射的来源（从映射库导入时写入）；`None` = 手工配置。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub articulation_source: Option<ArticulationSource>,
    /// 通用 MIDI 事件表（keyswitch / 一次性 CC / program change…）。空时不写进 JSON。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub midi_events: Vec<MidiEventAt>,
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
        self.role.is_none()
            && self.pitch_range.is_none()
            && self.pitch_bend_range.is_none()
            && self.mpe.is_none()
            && self.articulations.is_empty()
            && self.default_articulation.is_none()
            && self.articulation_source.is_none()
            && self.midi_events.is_empty()
    }

    /// 归一化：夹紧可选的弯音范围与 MPE 通道数，非有限值直接丢弃。
    pub fn normalized(mut self) -> Self {
        self.pitch_bend_range = self.pitch_bend_range.and_then(|range| {
            range
                .is_finite()
                .then(|| range.clamp(Self::MIN_PITCH_BEND_RANGE, Self::MAX_PITCH_BEND_RANGE))
        });
        self.mpe = self.mpe.map(MpeConfig::normalized);
        self.articulations = self
            .articulations
            .into_iter()
            .filter_map(|(name, switch)| {
                let name = name.trim().to_string();
                (!name.is_empty() && !switch.is_empty()).then(|| (name, switch.normalized()))
            })
            .collect();
        self.default_articulation = self
            .default_articulation
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty());
        self.articulation_source = self
            .articulation_source
            .and_then(ArticulationSource::normalized);
        self.midi_events = self
            .midi_events
            .into_iter()
            .map(MidiEventAt::normalized)
            .collect();
        self.midi_events.sort_by(|a, b| {
            a.beat
                .partial_cmp(&b.beat)
                .unwrap_or(std::cmp::Ordering::Equal)
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
    use super::{
        lane_value_at, ArticulationEvent, ArticulationSource, ArticulationSwitch, AutomationPoint,
        CcSwitch, MidiEventAt, MixerChannel, MpeConfig, MpeZone, Send, TrackId, TrackMeta,
    };

    #[test]
    fn gain_and_pan_are_normalized() {
        assert_eq!(
            MixerChannel::normalize_gain_db(100.0),
            MixerChannel::MAX_GAIN_DB
        );
        assert_eq!(
            MixerChannel::normalize_gain_db(-100.0),
            MixerChannel::MIN_GAIN_DB
        );
        assert_eq!(MixerChannel::normalize_gain_db(f32::NAN), 0.0);
        assert_eq!(MixerChannel::normalize_pan(3.0), 1.0);
        assert_eq!(MixerChannel::normalize_pan(-3.0), -1.0);
        assert_eq!(MixerChannel::normalize_pan(f32::NAN), 0.0);
    }

    #[test]
    fn lane_value_interpolates_and_clamps_at_both_ends() {
        let points = [
            AutomationPoint {
                beat: 0.0,
                value: 0.0,
            },
            AutomationPoint {
                beat: 4.0,
                value: -12.0,
            },
        ];
        assert_eq!(lane_value_at(&points, -1.0), Some(0.0), "起点前夹住");
        assert_eq!(lane_value_at(&points, 0.0), Some(0.0));
        assert_eq!(lane_value_at(&points, 2.0), Some(-6.0), "中点线性插值");
        assert_eq!(lane_value_at(&points, 4.0), Some(-12.0));
        assert_eq!(lane_value_at(&points, 9.0), Some(-12.0), "终点后夹住");
        assert_eq!(lane_value_at(&[], 0.0), None, "空曲线没有值");
        // 单点：处处取该值。
        assert_eq!(
            lane_value_at(
                &[AutomationPoint {
                    beat: 3.0,
                    value: 0.5
                }],
                0.0
            ),
            Some(0.5)
        );
    }

    #[test]
    fn send_normalizes_gain() {
        assert_eq!(
            Send {
                target: TrackId(2),
                gain_db: 999.0
            }
            .normalize()
            .gain_db,
            MixerChannel::MAX_GAIN_DB
        );
        assert_eq!(
            Send {
                target: TrackId(2),
                gain_db: f32::NAN
            }
            .normalize()
            .gain_db,
            0.0
        );
    }

    #[test]
    fn mpe_member_channels_cover_lower_and_upper_zones() {
        let lower = MpeConfig {
            channels: 4,
            zone: MpeZone::Lower,
        };
        assert_eq!(lower.master_channel(), 0);
        assert_eq!(lower.member_channels(), vec![1, 2, 3, 4]);

        let upper = MpeConfig {
            channels: 4,
            zone: MpeZone::Upper,
        };
        assert_eq!(upper.master_channel(), 15);
        assert_eq!(upper.member_channels(), vec![14, 13, 12, 11]);

        // 越界的通道数被夹住，而不是生成非法通道。
        let clamped = MpeConfig {
            channels: 99,
            zone: MpeZone::Lower,
        }
        .normalized();
        assert_eq!(clamped.channels, MpeConfig::MAX_CHANNELS);
        assert_eq!(
            MpeConfig {
                channels: 0,
                zone: MpeZone::Lower
            }
            .normalized()
            .channels,
            1
        );
    }

    #[test]
    fn mpe_is_omitted_from_json_when_absent() {
        let meta = TrackMeta::default();
        let json = serde_json::to_string(&meta).unwrap();
        assert!(!json.contains("mpe"), "{json}");
        assert!(meta.is_empty());

        let with = TrackMeta {
            mpe: Some(MpeConfig {
                channels: 3,
                zone: MpeZone::Upper,
            }),
            ..TrackMeta::default()
        };
        assert!(!with.is_empty());
        let back: TrackMeta = serde_json::from_str(&serde_json::to_string(&with).unwrap()).unwrap();
        assert_eq!(back.mpe, with.mpe);
    }

    /// 通用 MIDI 事件表与奏法映射：空时整个 `meta` 不出现；非空时能往返。
    #[test]
    fn midi_events_and_articulations_roundtrip_without_empty_keys() {
        let empty = serde_json::to_string(&TrackMeta::default()).unwrap();
        assert!(!empty.contains("midi_events"), "{empty}");
        assert!(!empty.contains("articulations"), "{empty}");

        let mut articulations = std::collections::BTreeMap::new();
        articulations.insert(
            "harmonics".to_string(),
            ArticulationSwitch {
                keyswitch: Some(24),
                ..Default::default()
            },
        );
        let meta = TrackMeta {
            midi_events: vec![MidiEventAt {
                beat: 1.0,
                channel: 2,
                event: ai_daw_plugin_api::MidiEvent::ControlChange {
                    controller: 64,
                    value: 127,
                },
            }],
            articulations,
            default_articulation: Some("harmonics".into()),
            ..TrackMeta::default()
        };
        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("midi_events"), "{json}");
        assert!(json.contains("articulations"), "{json}");
        assert_eq!(serde_json::from_str::<TrackMeta>(&json).unwrap(), meta);
    }

    /// 导入的事件序列能往返；没有显式 NoteOff 的 NoteOn 会自动补一个短 NoteOff。
    #[test]
    fn articulation_events_roundtrip_and_pair_note_offs() {
        let switch = ArticulationSwitch {
            events: vec![
                ArticulationEvent {
                    offset_beats: 0.0,
                    channel: Some(3),
                    event: ai_daw_plugin_api::MidiEvent::ControlChange {
                        controller: 32,
                        value: 5,
                    },
                },
                ArticulationEvent {
                    offset_beats: 0.0,
                    channel: None,
                    event: ai_daw_plugin_api::MidiEvent::NoteOn {
                        note: 24,
                        velocity: 1,
                    },
                },
            ],
            ..Default::default()
        };
        let messages = switch.messages();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].channel, Some(3));
        assert!(matches!(
            messages[2].event,
            ai_daw_plugin_api::MidiEvent::NoteOff { note: 24 }
        ));
        assert_eq!(
            messages[2].offset_beats,
            ArticulationSwitch::KEYSWITCH_HOLD_BEATS
        );

        let json = serde_json::to_string(&switch).unwrap();
        assert!(json.contains("events"), "{json}");
        assert_eq!(
            serde_json::from_str::<ArticulationSwitch>(&json).unwrap(),
            switch
        );
    }

    /// 旧字段仍然可用，并且按 keyswitch / CC / program 的顺序展开。
    #[test]
    fn legacy_switch_expands_without_copy() {
        let switch = ArticulationSwitch {
            keyswitch: Some(24),
            cc: Some(CcSwitch {
                controller: 1,
                value: 0,
            }),
            program: Some(3),
            ..Default::default()
        };
        let messages = switch.messages();
        assert!(matches!(
            messages[0].event,
            ai_daw_plugin_api::MidiEvent::NoteOn { note: 24, .. }
        ));
        assert!(matches!(
            messages[1].event,
            ai_daw_plugin_api::MidiEvent::NoteOff { note: 24 }
        ));
        assert!(matches!(
            messages[2].event,
            ai_daw_plugin_api::MidiEvent::ControlChange {
                controller: 1,
                value: 0
            }
        ));
        assert!(matches!(
            messages[3].event,
            ai_daw_plugin_api::MidiEvent::ProgramChange { program: 3 }
        ));
    }

    #[test]
    fn articulation_source_is_trimmed_and_omitted_when_empty() {
        let empty = TrackMeta {
            articulation_source: Some(ArticulationSource {
                manufacturer: Some("  ".into()),
                ..Default::default()
            }),
            ..Default::default()
        }
        .normalized();
        assert!(empty.is_empty());
        assert!(!serde_json::to_string(&empty)
            .unwrap()
            .contains("articulation_source"));

        let with = TrackMeta {
            articulation_source: Some(ArticulationSource {
                product: Some("  BBCSO  ".into()),
                ..Default::default()
            }),
            ..Default::default()
        }
        .normalized();
        assert_eq!(
            with.articulation_source
                .as_ref()
                .and_then(|source| source.product.as_deref()),
            Some("BBCSO")
        );
    }
}
