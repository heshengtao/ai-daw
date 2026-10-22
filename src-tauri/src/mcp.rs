//! 内嵌的 MCP 服务器：把工程 / 传输 / 编曲 / 渲染分析暴露成 AI 代理可调用的工具。
//!
//! 传输走 **Streamable HTTP**，绑定 `127.0.0.1` 的临时端口——GUI 进程没法被 MCP 客户端用
//! stdio 拉起，本地 HTTP 是唯一可行的形态。rmcp 默认只放行回环 `Host`（防 DNS rebinding），
//! 我们也只绑回环地址。
//!
//! 所有**写操作**都复用 [`commands::run_agent`]——与界面走同一条管线：撤销历史、`engine://event`
//! 事件流、音频同步、结构变更时关闭插件窗口。因此代理做的每一步都会在界面上原样出现，
//! 人也随时可以撤销它。
//!
//! 只读工具（`get_project` / `get_transport` / `list_plugins`）不需要这条管线，直接读共享状态。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{ErrorCode, ServerCapabilities, ServerConfig};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tokio_util::sync::CancellationToken;

use ai_daw_engine::audio::AudioBuffer;
use ai_daw_engine::command::Command;
use ai_daw_engine::event::{Actor, Event};
use ai_daw_engine::model::{
    AudioSource, BeatFilter, Clip, ClipId, FitMode, Note, NotePatch, NoteRange, NoteTag,
    PitchExpression, PluginInstance, Project, StrumDirection, Track, TrackId, Transport,
    VelocityShape,
};
use ai_daw_engine::registry::{PluginRegistry, PluginSlot};
use ai_daw_engine::tempo::TempoMap;
use ai_daw_engine::transport::TransportCommand;
use ai_daw_plugin_api::{MidiEvent, ParamId, PluginKind, ProcessContext, TimedMidiEvent};

use crate::commands;
use crate::factory_presets;
use crate::plugin_info;
use crate::sound_memories::MemoryMeta;
use crate::state::AppState;

/// 采样率的合法区间，与 `commands::validate_sample_rate` 保持一致。
const SAMPLE_RATES: std::ops::RangeInclusive<u32> = 8_000..=192_000;
/// 音频设备不可用时，定位换算用的兜底采样率。
const FALLBACK_SAMPLE_RATE: u32 = 44_100;

/// 工具错误 → MCP 的 `internal_error`。
fn fail(msg: impl Into<String>) -> ErrorData {
    ErrorData::new(ErrorCode::INTERNAL_ERROR, msg.into(), None)
}

/// 从事件里取出新建轨道的 id。
fn track_added_id(events: &[Event]) -> Option<u64> {
    events.iter().find_map(|e| match e {
        Event::TrackAdded { id, .. } => Some(id.0),
        _ => None,
    })
}

/// 从事件里取出新建片段的 id（MIDI 与音频片段都算）。
fn clip_added_id(events: &[Event]) -> Option<u64> {
    events.iter().find_map(|e| match e {
        Event::ClipAdded { clip, .. } | Event::AudioClipAdded { clip, .. } => Some(clip.0),
        _ => None,
    })
}

// ---------------------------------------------------------------------------
// 工具参数
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct TempoArgs {
    /// 新的速度（BPM）。
    bpm: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct TimeSigArgs {
    /// 拍号分子（1..=32）。
    num: u8,
    /// 拍号分母（会吸附到 2 的幂）。
    den: u8,
}

/// 在指定拍写入 / 更新一个速度关键点（BPM + 拍号）。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct KeyframeArgs {
    /// 关键点所在的拍。
    beat: f64,
    /// 从这一拍起的速度（BPM）。
    tempo: f64,
    /// 拍号分子（1..=32）。
    num: u8,
    /// 拍号分母（会吸附到 2 的幂）。
    den: u8,
}

/// 删除指定拍的速度关键点。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct BeatArgs {
    /// 关键点所在的拍。
    beat: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct TrackArgs {
    /// 轨道 id。
    track: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SeparateArgs {
    /// 轨道 id。
    track: u64,
    /// 分离模型：`htdemucs-ort-v1`（4 分轨，默认）或 `htdemucs-6s`（6 分轨）。
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct DetectTempoArgs {
    /// 轨道 id（用它的第一段音频做节拍检测）。
    track: u64,
}

/// 降噪：`mode` 决定走语音模型还是内置 DSP。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct EnhanceArgs {
    /// 轨道 id（用它的第一段音频做降噪）。
    track: u64,
    /// `voice`（人声降噪 / 去混响，优先 LocalVQE 模型）或 `music`（乐器底噪，内置 DSP）。
    /// 缺省 `voice`。
    #[serde(default)]
    mode: Option<String>,
}

/// 开始录音：从 `start` 拍开始在轨道上录一条新的音频片段。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct RecordStartArgs {
    /// 目标轨道 id。
    track: u64,
    /// 录音起始拍。
    start: f64,
    /// 输入设备名（缺省 = 系统默认设备）。
    #[serde(default)]
    device: Option<String>,
}

/// 选择 / 关闭输入设备。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct InputDeviceArgs {
    /// 设备名；不传 = 关闭输入流（释放麦克风）。
    #[serde(default)]
    device: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct NameArgs {
    /// 轨道名。
    name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct RenameArgs {
    /// 轨道 id。
    track: u64,
    /// 新名字。
    name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct InstrumentArgs {
    /// 轨道 id。
    track: u64,
    /// 乐器插件 id（内置用 `builtin.synth`，外部形如 `vst3:…` / `clap:…`）。
    /// 也可以直接写名字（`JC303` / `Ultramaster KR-106`），会按名字解析成 id。
    plugin_id: String,
}

/// `list_plugins` 的参数：按类别 / 关键词过滤。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ListPluginsArgs {
    /// 只看某一类：`instrument` / `effect` / `all`（缺省 `all`）。
    #[serde(default)]
    kind: Option<String>,
    /// 关键词过滤（名字 / 厂商 / 分类 / 描述 / 标签，大小写不敏感）；缺省不过滤。
    #[serde(default)]
    query: Option<String>,
}

/// `list_synth_presets` 的参数：按类别 / 关键词过滤内置合成器的出厂音色。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ListSynthPresetsArgs {
    /// 类别：`8bit` / `epiano` / `guitar` / `bass` / `drums` / `strings` / `winds` / `pad` / `fx`；
    /// 缺省返回全部。
    #[serde(default)]
    category: Option<String>,
    /// 关键词过滤（中文名 / 描述 / id，大小写不敏感）；缺省不过滤。
    #[serde(default)]
    query: Option<String>,
}

/// `describe_plugin` 的参数。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct DescribePluginArgs {
    /// 插件 id 或名字（`vst3:…` / `builtin.synth` / `JC303`）。
    plugin_id: String,
    /// 只看名字 / id 含这个词的参数（大小写不敏感）；缺省给全部音色参数。
    #[serde(default)]
    query: Option<String>,
    /// 最多返回多少个参数；**缺省给全部**（上限 400，纯属安全阀）。
    ///
    /// MIDI CC 映射这类管道参数（动辄两千条）本来就不返回，会以 `param_hidden` 报个数。
    #[serde(default)]
    max_params: Option<usize>,
}

/// `add_effect` / `remove_effect` 的参数。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct EffectArgs {
    /// 轨道 id。
    track: u64,
    /// 效果器插件 id 或名字（`builtin.reverb` / `ValhallaSupermassive`）。
    plugin_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct EffectIndexArgs {
    /// 轨道 id。
    track: u64,
    /// 效果链里的序号（0 起，从链头算）。
    index: usize,
}

/// 设置单个效果器参数。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct EffectParamArgs {
    /// 轨道 id。
    track: u64,
    /// 效果链里的序号（0 起）。
    index: usize,
    /// 参数 id（由效果器定义，用 `describe_plugin` 查）。
    param: String,
    /// 参数值（按插件给出的范围）。
    value: f32,
}

/// 一次写入一个效果器的整组参数（一个撤销步）。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct EffectParamsArgs {
    /// 轨道 id。
    track: u64,
    /// 效果链里的序号（0 起）。
    index: usize,
    /// 参数 id → 值。
    params: HashMap<String, f32>,
}

/// 调整效果器在链中的位置。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct MoveEffectArgs {
    /// 轨道 id。
    track: u64,
    /// 原序号（0 起）。
    from: usize,
    /// 目标序号（0 起）。
    to: usize,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ParamArgs {
    /// 轨道 id。
    track: u64,
    /// 参数 id（由插件定义）。
    param: String,
    /// 参数值（按插件给出的范围）。
    value: f32,
}

/// 一次写入整组乐器参数（捏音色的主入口）。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SetParamsArgs {
    /// 轨道 id。
    track: u64,
    /// 参数 id → 值。**一次事务 = 一个撤销步**；未知参数会报错并列出可用的 id。
    params: HashMap<String, f32>,
    /// 先把所有参数复位到参数表默认值，再写 `params`（缺省 false = 只改给出的键）。
    ///
    /// 切换音色时建议开：否则上一条音色独有的参数会残留下来。
    #[serde(default)]
    reset: bool,
}

/// 试听一件**内置乐器**的参数补丁：离线渲染一个音，返回响度与音色指纹。
///
/// 不写工程、不进撤销历史——用来在「捏音色」时反复试参数，满意后再用 `set_params` 落地。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct PreviewSynthArgs {
    /// 用这条轨道当前的乐器与参数作为起点（可选）。缺省用 `builtin.synth` 的出厂默认。
    #[serde(default)]
    track: Option<u64>,
    /// 参数覆盖（id → 值）；写在起点之上。
    #[serde(default)]
    params: HashMap<String, f32>,
    /// 试听音高（MIDI，缺省 60 = C4）。
    #[serde(default)]
    note: Option<u8>,
    /// 力度（0..=127，缺省 100）。
    #[serde(default)]
    velocity: Option<u8>,
    /// 按住时长（毫秒，缺省 900，范围 50..=5000）。
    #[serde(default)]
    duration_ms: Option<u32>,
    /// 松键后的尾巴（毫秒）；缺省按 release 参数自动取（至少 300 ms）。
    #[serde(default)]
    tail_ms: Option<u32>,
    /// 采样率（缺省 48000）。
    #[serde(default)]
    sample_rate: Option<u32>,
    /// 从参数表默认值开始（忽略轨道状态与轨道参数），只应用 `params`。
    #[serde(default)]
    reset: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ClipArgs {
    /// 轨道 id。
    track: u64,
    /// 片段起始拍。
    start: f64,
    /// 片段长度（拍）。
    length: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ClipRefArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
}

/// 裁剪 / 移动片段：新的起点与拍长。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct TrimClipArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
    /// 新的起始拍。
    start: f64,
    /// 新的长度（拍）。
    length: f64,
}

/// 音频片段的播放速度与升降调。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ClipRateArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
    /// 播放速度（1.0 = 原速）。
    rate: f64,
    /// 升降调（半音，0 = 不变）。
    #[serde(default)]
    pitch_semitones: f64,
}

/// 音符级音高表达：推弦 / 滑音 / 揉弦。全部可选，单位见字段说明。
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub(crate) struct ExprArg {
    /// 起音时的音高偏移（音分，100 = 1 个半音）。预推弦 = 200。
    #[serde(default)]
    bend: Option<i16>,
    /// 推弦 / 滑音的目标偏移（音分）。
    #[serde(default)]
    bend_to: Option<i16>,
    /// 从 `bend` 到 `bend_to` 的时间（拍）；0 = 立即到达。
    #[serde(default)]
    bend_time: Option<f32>,
    /// 揉弦深度（音分，0 = 不揉）。
    #[serde(default)]
    vibrato: Option<i16>,
    /// 揉弦速率（Hz）；0 = 默认 5.5。
    #[serde(default)]
    vibrato_rate: Option<f32>,
    /// 起音后延迟多少拍开始揉。
    #[serde(default)]
    vibrato_delay: Option<f32>,
    /// 揉弦渐入时长（拍）；0 = 立即满深度。
    #[serde(default)]
    vibrato_fade: Option<f32>,
}

impl ExprArg {
    fn to_expr(&self) -> PitchExpression {
        PitchExpression {
            bend: self.bend.unwrap_or(0),
            bend_to: self.bend_to.unwrap_or(0),
            bend_time: self.bend_time.unwrap_or(0.0),
            vibrato: self.vibrato.unwrap_or(0),
            vibrato_rate: self.vibrato_rate.unwrap_or(0.0),
            vibrato_delay: self.vibrato_delay.unwrap_or(0.0),
            vibrato_fade: self.vibrato_fade.unwrap_or(0.0),
        }
        .normalize()
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct NoteArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
    /// MIDI 音高（0..=127）。
    pitch: u8,
    /// 音符在片段内的起始拍。
    start: f64,
    /// 时值（拍）。
    duration: f64,
    /// 力度（0..=127）。
    velocity: u8,
    /// 单音音量（0..=127，127 = 不衰减）；缺省 127。
    #[serde(default)]
    volume: Option<u8>,
    /// 起始偏移（拍，正 = 延后发声）；缺省 0 = 与 `start` 同时发声。
    #[serde(default)]
    offset: Option<f64>,
    /// 连奏（NoteOff 延到同音高的下一个音）；缺省 false。
    #[serde(default)]
    legato: Option<bool>,
    /// 音高表达（推弦 / 揉弦 / 滑音）；缺省 = 无。
    #[serde(default)]
    expr: Option<ExprArg>,
    /// 放置吸附：`off`（缺省，原样写入）/ `bar` / `1/4` / `1/8` / `1/16` / `1/12` / `1/24` / `1/32`。
    /// **只吸起点**。
    #[serde(default)]
    snap: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct GainArgs {
    /// 轨道 id。
    track: u64,
    /// 增益（dB）。
    gain_db: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SendArg {
    /// 目标轨道 id（通常是一条总线轨道）。
    target: u64,
    /// 发送增益（dB）。
    gain_db: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SetSendsArgs {
    /// 轨道 id。
    track: u64,
    /// 发送列表（整体替换；空数组 = 清空）。
    sends: Vec<SendArg>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct PointArg {
    /// 位置（拍）。
    beat: f64,
    /// 参数值。
    value: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SetAutomationArgs {
    /// 轨道 id。
    track: u64,
    /// 参数名：`gain_db` / `pan` / `cc:<0..=127>`（MIDI 控制变化），或 `inst:<参数id>`（轨道乐器）/
    /// `fx<i>:<参数id>`（效果链第 i 个，i 从 0 起）。
    param: String,
    /// 关键点（整体替换；空数组 = 删除该车道）。
    points: Vec<PointArg>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct HelpArgs {
    /// 能力域：`project` / `track` / `clip` / `note` / `composition` / `mix` / `audio` / `transport` /
    /// `analysis` / `plugin` / `sound-design` / `sound-memory` / `theory` / `rhythm` / `genres` / `workflow`；留空返回索引。
    #[serde(default)]
    topic: Option<String>,
}

/// 技能文档（编译期内嵌，无需资源打包）：`topic` → markdown。空 topic 是索引。
///
/// 目录结构 = 一个标准 Agent Skill：`skills/aidaw/SKILL.md`（入口，带 frontmatter）
/// + `skills/aidaw/references/*.md`（能力域，按需加载）。
pub(crate) const SKILL_DOCS: &[(&str, &str)] = &[
    ("", include_str!("../../skills/aidaw/SKILL.md")),
    ("project", include_str!("../../skills/aidaw/references/project.md")),
    ("track", include_str!("../../skills/aidaw/references/track.md")),
    ("clip", include_str!("../../skills/aidaw/references/clip.md")),
    ("note", include_str!("../../skills/aidaw/references/note.md")),
    ("composition", include_str!("../../skills/aidaw/references/composition.md")),
    ("mix", include_str!("../../skills/aidaw/references/mix.md")),
    ("audio", include_str!("../../skills/aidaw/references/audio.md")),
    ("transport", include_str!("../../skills/aidaw/references/transport.md")),
    ("analysis", include_str!("../../skills/aidaw/references/analysis.md")),
    ("plugin", include_str!("../../skills/aidaw/references/plugin.md")),
    ("sound-design", include_str!("../../skills/aidaw/references/sound-design.md")),
    ("sound-memory", include_str!("../../skills/aidaw/references/sound-memory.md")),
    ("theory", include_str!("../../skills/aidaw/references/theory.md")),
    ("rhythm", include_str!("../../skills/aidaw/references/rhythm.md")),
    ("genres", include_str!("../../skills/aidaw/references/genres.md")),
    ("workflow", include_str!("../../skills/aidaw/references/workflow.md")),
];

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct PanArgs {
    /// 轨道 id。
    track: u64,
    /// 声像：-1.0 最左，1.0 最右，0 居中。
    pan: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ToggleArgs {
    /// 轨道 id。
    track: u64,
    /// 开 / 关。
    on: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct MasterGainArgs {
    /// 总输出增益（dB）。
    gain_db: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SeekArgs {
    /// 目标位置（秒）。
    seconds: f64,
}

/// 保存工程：不传 `path` 就存回当前工程文件。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SaveProjectArgs {
    /// 目标 .json 文件路径；省略 = 当前工程的路径（从未保存过则报错，要求给出 path）。
    #[serde(default)]
    path: Option<String>,
}

/// 打开工程文件。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct OpenProjectArgs {
    /// 工程 JSON 文件的绝对路径。
    path: String,
}

/// 新建工程。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct NewProjectArgs {
    /// 按模板顺序给出的轨道名；省略用引擎的兜底名。
    #[serde(default)]
    track_names: Option<Vec<String>>,
    /// 新工程的速度（BPM）；省略 = 模板默认 120。
    #[serde(default)]
    tempo: Option<f64>,
}

/// 重命名当前工程。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct RenameProjectArgs {
    /// 新名字。
    name: String,
}

/// 历史工程的操作目标。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct HistoryIdArgs {
    /// 历史条目的 id（用 `list_project_history` 查）。
    id: String,
}

/// 新增一条用户预设。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SavePresetArgs {
    /// 预设名（不能为空）。
    name: String,
    /// 归属的插件 id（`builtin.synth` / `vst3:…` / `clap:…`）。
    plugin_id: String,
    /// 参数快照（参数 id → 值）。
    params: HashMap<String, f32>,
}

/// 覆盖一条用户预设。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct UpdatePresetArgs {
    /// 预设 id（用 list_presets 查）。
    id: String,
    /// 新名字。
    name: String,
    /// 新的参数快照（整体替换）。
    params: HashMap<String, f32>,
}

/// 删除一条用户预设。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct PresetIdArgs {
    /// 预设 id（用 list_presets 查）。
    id: String,
}

/// 列出音色记忆（只返回摘要）。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ListSoundMemoriesArgs {
    /// 关键词：匹配名称、描述、标签、分工、乐器/效果器 plugin id。
    #[serde(default)]
    query: Option<String>,
    /// 精确匹配一个标签（大小写不敏感）。
    #[serde(default)]
    tag: Option<String>,
    /// 只看使用这个插件的记忆（乐器或效果链里出现即可）。
    #[serde(default)]
    plugin_id: Option<String>,
    /// 最多返回多少条；缺省 50，上限 200。
    #[serde(default)]
    limit: Option<usize>,
}

/// 读取一条音色记忆的完整快照。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SoundMemoryIdArgs {
    /// 记忆 id（用 list_sound_memories 查）。
    id: String,
}

/// 把当前轨道的音色链存成记忆。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SaveSoundMemoryArgs {
    /// 源轨道 id（后端直接读它的乐器/效果链/增益声像）。
    track: u64,
    /// 记忆名称（不能为空）。
    name: String,
    /// 一句话描述（建议写听感/用途）。
    #[serde(default)]
    description: Option<String>,
    /// 标签（角色 / 风格 / 听感，2–5 个）。
    #[serde(default)]
    tags: Option<Vec<String>>,
    /// 传了就是覆盖这条已有记忆的内容；不传 = 新建。
    #[serde(default)]
    id: Option<String>,
}

/// 只改音色记忆的元数据。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct UpdateSoundMemoryArgs {
    /// 记忆 id。
    id: String,
    /// 新名称；省略 = 不改。
    #[serde(default)]
    name: Option<String>,
    /// 新描述；省略 = 不改。
    #[serde(default)]
    description: Option<String>,
    /// 新标签（整体替换）；省略 = 不改。
    #[serde(default)]
    tags: Option<Vec<String>>,
    /// 新分工；省略 = 不改，空字符串 = 清除。
    #[serde(default)]
    role: Option<String>,
}

/// 把一条音色记忆应用到轨道。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ApplySoundMemoryArgs {
    /// 记忆 id。
    id: String,
    /// 目标轨道 id。
    track: u64,
}

/// 网关：用「操作名 + JSON 参数」调用任意工具（等价于直接调用那个工具）。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct CallArgs {
    /// 工具名，例如 `add_notes` / `set_effect_param`。
    op: String,
    /// 该工具的参数对象；省略 = 空对象。
    #[serde(default)]
    args: serde_json::Value,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct AnalyzeArgs {
    /// 采样率（Hz）；省略则用 44100。
    sample_rate: Option<u32>,
    /// 局部渲染的起始拍（工程绝对拍）；省略 = 0。给 `start_beat` 或 `end_beat` 即局部渲染。
    #[serde(default)]
    start_beat: Option<f64>,
    /// 局部渲染的结束拍（不含）；省略 = 工程结尾。
    #[serde(default)]
    end_beat: Option<f64>,
    /// 只渲染这些轨道（其他轨道静音）；省略 = 全部。
    #[serde(default)]
    tracks: Option<Vec<u64>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ExportArgs {
    /// 目标 WAV 文件的绝对路径。
    path: String,
    /// 采样率（Hz）；省略则用 44100。
    sample_rate: Option<u32>,
    /// 局部导出的起始拍（工程绝对拍）；省略 = 0。给 `start_beat` 或 `end_beat` 即局部导出。
    #[serde(default)]
    start_beat: Option<f64>,
    /// 局部导出的结束拍（不含）；省略 = 工程结尾。
    #[serde(default)]
    end_beat: Option<f64>,
    /// 只导出这些轨道（其他轨道静音）；省略 = 全部。
    #[serde(default)]
    tracks: Option<Vec<u64>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct MasterExportArgs {
    /// 目标 WAV 文件的绝对路径。
    path: String,
    /// 目标积分响度（LUFS）；省略则用 -14。
    target_lufs: Option<f64>,
    /// 真峰值上限（dBTP）；省略则用 -1。
    ceiling_dbtp: Option<f64>,
    /// 立体声宽度（0–2，1 = 原样）；省略则用 1。
    stereo_width: Option<f32>,
}

/// 母带导出的返回体：写到哪 + 前后观测。
#[derive(serde::Serialize)]
struct MasterExport {
    path: String,
    report: ai_daw_mastering::MasterReport,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ImportAudioArgs {
    /// 音频文件的绝对路径（wav / flac / mp3 / ogg / m4a）。
    path: String,
    /// 目标轨道 id。
    track: u64,
    /// 片段起始拍。
    start: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct WaveformArgs {
    /// 音频文件的绝对路径。
    path: String,
    /// 桶数（每桶给 min/max 两个值）。
    buckets: usize,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ApplyPresetArgs {
    /// 目标轨道 id。
    track: u64,
    /// 整组乐器参数（参数名 → 值）。
    #[serde(default)]
    params: HashMap<String, f32>,
    /// 整条效果链；**替换**轨道现有的效果链（不传 = 清空）。
    #[serde(default)]
    effects: Vec<commands::EffectSpec>,
}

// ---------------------------------------------------------------------------
// 编曲工具参数
// ---------------------------------------------------------------------------

/// 一次批量写入里的一个音符。`id` 由引擎发放，调用方给不给都行。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct NoteInput {
    /// MIDI 音高（0..=127）。
    pitch: u8,
    /// 片段内起始拍（相对片段起点）。
    start: f64,
    /// 时值（拍）。
    duration: f64,
    /// 演奏力度（0..=127）；缺省 100。
    #[serde(default = "default_velocity")]
    velocity: u8,
    /// 单音音量（0..=127，127 = 不衰减）；缺省 127。
    #[serde(default = "default_note_volume")]
    volume: u8,
    /// 起始偏移（拍；正 = 延后发声）。扫弦与手感微调用它；缺省 0 = 与 `start` 同时发声。
    #[serde(default)]
    offset: f64,
    /// 连奏：NoteOff 延到同音高的下一个音起点；缺省 false。
    #[serde(default)]
    legato: bool,
    /// 音高表达（推弦 / 揉弦 / 滑音）；缺省 = 无。
    #[serde(default)]
    expr: Option<ExprArg>,
}

fn default_velocity() -> u8 {
    100
}

fn default_note_volume() -> u8 {
    Note::MAX_VOLUME
}

impl NoteInput {
    fn to_note(&self) -> Note {
        Note {
            id: 0,
            pitch: self.pitch,
            start: self.start,
            duration: self.duration,
            velocity: self.velocity,
            volume: self.volume,
            offset: self.offset,
            legato: self.legato,
            expr: self.expr.as_ref().map(ExprArg::to_expr).unwrap_or(PitchExpression::NONE),
        }
    }
}

/// 按页读取一个片段的音符（只读）。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct GetNotesArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
    /// 跳过前多少个匹配的音符；缺省 0。
    #[serde(default)]
    offset: Option<usize>,
    /// 本次最多返回多少个；缺省 200，上限 1000。
    #[serde(default)]
    limit: Option<usize>,
    /// 只返回起点落在这个范围内的音符（片段内拍）。
    #[serde(default)]
    range: Option<RangeArg>,
}

/// 片段内拍范围（左闭右开），批量变换只作用于起点落在其中的音符。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct RangeArg {
    /// 起始拍（相对片段起点）。
    start: f64,
    /// 结束拍（不含）。
    end: f64,
}

impl RangeArg {
    fn to_range(&self) -> NoteRange {
        NoteRange { start: self.start, end: self.end }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct NotesArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
    /// 音符列表（一次事务写入，可整体撤销）。
    notes: Vec<NoteInput>,
    /// 放置吸附：`off`（缺省，原样写入）/ `bar` / `1/4` / `1/8` / `1/16` / `1/12` / `1/24` / `1/32`。
    /// **只吸起点**（时值按调用方给的算，偏移是演奏姿态、不动）。
    #[serde(default)]
    snap: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct NoteIdsArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
    /// 要删除的音符 id（`get_project` / 批量工具返回的稳定 id）。
    ids: Vec<u64>,
}

/// 按 id 改写一个音符：只填要改的字段。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct NotePatchInput {
    /// 目标音符 id。
    id: u64,
    /// 新音高（0..=127）。
    #[serde(default)]
    pitch: Option<u8>,
    /// 新起点（片段内拍）。
    #[serde(default)]
    start: Option<f64>,
    /// 新时值（拍）。
    #[serde(default)]
    duration: Option<f64>,
    /// 新力度（0..=127）。
    #[serde(default)]
    velocity: Option<u8>,
    /// 新音量（0..=127）。
    #[serde(default)]
    volume: Option<u8>,
    /// 新起始偏移（拍，正 = 延后发声；0 = 与记谱位置同时发声）。扫弦与手感微调用它。
    #[serde(default)]
    offset: Option<f64>,
    /// 新连奏开关（true = NoteOff 延到同音高的下一个音）。
    #[serde(default)]
    legato: Option<bool>,
    /// 新的音高表达；给了就整体替换，`{}` = 清除。
    #[serde(default)]
    expr: Option<ExprArg>,
}

impl NotePatchInput {
    fn to_patch(&self) -> NotePatch {
        NotePatch {
            id: self.id,
            pitch: self.pitch,
            start: self.start,
            duration: self.duration,
            velocity: self.velocity,
            volume: self.volume,
            offset: self.offset,
            legato: self.legato,
            expr: self.expr.as_ref().map(ExprArg::to_expr),
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SetNoteTagArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
    /// 音符 id（`get_project` / `get_clip_notes` 返回的稳定 id）。
    id: u64,
    /// 装饰音类型：`grace` / `acciaccatura` / `mordent` …；留空 = 清除该字段。
    #[serde(default)]
    ornament: Option<String>,
    /// 奏法：`staccato` / `accent` / `legato` …；留空 = 清除该字段。
    #[serde(default)]
    articulation: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct UpdateNotesArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
    /// 改写项（按稳定 id 定位）。
    updates: Vec<NotePatchInput>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct TransposeArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
    /// 移调半音数（正 = 升高）。
    semitones: i32,
    /// 只处理这个范围内的音符（缺省 = 整个片段）。
    #[serde(default)]
    range: Option<RangeArg>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct VelocityArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
    /// 固定力度（0..=127）。
    #[serde(default)]
    value: Option<u8>,
    /// 渐变的起始力度（配合 `to`）。
    #[serde(default)]
    from: Option<u8>,
    /// 渐变的结束力度。
    #[serde(default)]
    to: Option<u8>,
    /// 相对缩放（1.0 = 不变，0.5 = 减半）。
    #[serde(default)]
    scale: Option<f64>,
    /// 只处理这个范围内的音符。
    #[serde(default)]
    range: Option<RangeArg>,
    /// 拍位过滤网格（拍）；与 `beat` 一起给。例如 `grid: 1.0, beat: 0.5` 只改每拍反拍。
    #[serde(default)]
    grid: Option<f64>,
    /// 网格内目标位置（拍）；与 `grid` 一起给。
    #[serde(default)]
    beat: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct QuantizeArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
    /// 网格（拍）：四分音符 = 1.0，八分 = 0.5，十六分 = 0.25；三连音 = 1/3。（最小值 1/64）
    grid: f64,
    /// 吸附强度 0..=1（0 = 不动，1 = 完全对齐）；缺省 1。
    #[serde(default)]
    strength: Option<f64>,
    /// 只处理这个范围内的音符。
    #[serde(default)]
    range: Option<RangeArg>,
    /// 是否同时量化时值（缺省 false，只动起点）。
    #[serde(default)]
    duration: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SwingArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
    /// 子拍长度（拍）：8 分 swing = 0.5，16 分 = 0.25；缺省 0.5。
    #[serde(default)]
    subdivision: Option<f64>,
    /// 第二个子拍在一对子拍里的位置比例（0.5 = 平直，< 0.5 = 抢拍 / push，0.66 ≈ 三连音感，0.75 = 硬 shuffle）；缺省 0.66。
    #[serde(default)]
    ratio: Option<f64>,
    /// 应用程度 0..=1；缺省 1。
    #[serde(default)]
    strength: Option<f64>,
    /// 只处理这个范围内的音符。
    #[serde(default)]
    range: Option<RangeArg>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct StrumArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
    /// 相邻两音的毫秒差（30 ≈ 常见扫弦；0 = 清掉扫弦、回到同时发声）。
    step_ms: f64,
    /// `"down"`（下扫，低音先响，缺省）或 `"up"`。
    #[serde(default)]
    direction: Option<String>,
    /// 只处理这个范围内的音符（缺省 = 整个片段）。
    #[serde(default)]
    range: Option<RangeArg>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct HumanizeArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
    /// 位置抖动上限（拍）；缺省 0.02。
    #[serde(default)]
    timing: Option<f64>,
    /// 力度抖动上限（0..=127）；缺省 8。
    #[serde(default)]
    velocity: Option<f64>,
    /// 只处理这个范围内的音符。
    #[serde(default)]
    range: Option<RangeArg>,
    /// 随机种子（同一个种子结果完全一致）；缺省固定值。
    #[serde(default)]
    seed: Option<u64>,
    /// 跳过短于这个时值（拍）的音符；缺省 0（不跳过）。用来保护倚音/装饰音。
    #[serde(default)]
    min_duration: Option<f64>,
    /// 起点相同的音符共用同一个位置抖动；缺省 false。用来保护和弦/扫弦的同步。
    #[serde(default)]
    group_by_start: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct FitArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
    /// 音域下界（缺省用轨道元数据里的 `pitch_range.min`）。
    #[serde(default)]
    min_pitch: Option<u8>,
    /// 音域上界（缺省用轨道元数据里的 `pitch_range.max`）。
    #[serde(default)]
    max_pitch: Option<u8>,
    /// `fold`（八度折返，默认）或 `report`（只统计，不改动）。
    #[serde(default)]
    mode: Option<String>,
    /// 只处理这个范围内的音符。
    #[serde(default)]
    range: Option<RangeArg>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct DuplicateClipArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
    /// 目标轨道（缺省 = 原轨道）。
    #[serde(default)]
    to_track: Option<u64>,
    /// 目标起点（缺省 = 紧接在原片段之后）。
    #[serde(default)]
    start: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct MergeClipsArgs {
    /// 轨道 id。
    track: u64,
    /// 要合并的片段 id（至少两个；全部必须是 MIDI 片段）。
    clips: Vec<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct MoveClipArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
    /// 新的起始拍。
    start: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ResizeClipArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
    /// 新的长度（拍）。
    length: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SplitArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
    /// 切分位置（绝对拍，必须严格落在片段内部）。
    beat: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct TrackStatsArgs {
    /// 轨道 id。
    track: u64,
    /// 只统计这个片段（缺省 = 整条轨道）。
    #[serde(default)]
    clip: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct OverlapArgs {
    /// 参与比较的轨道 id（至少两条）。
    tracks: Vec<u64>,
    /// 只比较这些片段 id（缺省 = 整条轨道）。
    #[serde(default)]
    clip: Option<u64>,
}

/// 一个小节的和弦标记。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct HarmonyBarArg {
    /// 起始拍。
    start: f64,
    /// 长度（拍）；缺省到下一小节或一个小节。
    #[serde(default)]
    length: Option<f64>,
    /// 和弦符号（`Dm9` / `G13` / `Cmaj7` / `Bm7b5` / `E7b9` …）。
    symbol: String,
}

impl HarmonyBarArg {
    fn to_bar(&self, bar_beats: f64) -> ai_daw_engine::model::HarmonyBar {
        ai_daw_engine::model::HarmonyBar {
            start: self.start,
            length: self.length.filter(|l| *l > 0.0).unwrap_or(bar_beats),
            symbol: self.symbol.clone(),
            source: None,
        }
    }
}

/// 显式装饰音引用：`track` + `clip` + 音符 `id`。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct OrnamentRefArg {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
    /// 音符 id（`get_project` / `get_clip_notes` 返回的稳定 id）。
    id: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct HarmonyArgs {
    /// 小节和弦表；缺省时用工程里存的（`set_project_harmony` 写入的那份）。
    #[serde(default)]
    bars: Option<Vec<HarmonyBarArg>>,
    /// 要检查的轨道 id（缺省 = 所有有音符的轨道）。
    #[serde(default)]
    tracks: Option<Vec<u64>>,
    /// 允许 9 / 11 / 13 延伸音；缺省 true。
    #[serde(default)]
    allow_extensions: Option<bool>,
    /// 允许短经过音；缺省 true。
    #[serde(default)]
    allow_approach_notes: Option<bool>,
    /// 短经过音的时值上限（拍）；缺省 0.5。
    #[serde(default)]
    allow_non_chord_tone_duration: Option<f64>,
    /// 小节边界的时间容差（拍）；缺省 1/16（0.0625）。humanize/swing 后贴着小节线的音
    /// 不再被判成 suspension。它不改变按 `start` 分小节的口径。
    #[serde(default)]
    time_tolerance: Option<f64>,
    /// 调性根音（`F` / `Bb` / `F#`）；与 `scale` 一起给。
    #[serde(default)]
    key: Option<String>,
    /// 调式：major / minor / dorian / phrygian / lydian / mixolydian / locrian /
    /// harmonic_minor / melodic_minor / major_pentatonic / minor_pentatonic / blues；
    /// 缺省 major。给 `scale` 时必须同时给 `key`。
    #[serde(default)]
    scale: Option<String>,
    /// 显式装饰音 `[{track, clip, id}]`：命中时无论时值长短都记成 `ornament` / `info`。
    #[serde(default)]
    ornaments: Option<Vec<OrnamentRefArg>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SetHarmonyArgs {
    /// 小节和弦表（整体替换；空数组 = 清除）。
    bars: Vec<HarmonyBarArg>,
}

/// 一个段落标记。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SectionArg {
    /// 段落名（`主歌` / `Chorus` …）。
    name: String,
    /// 起始拍。
    start: f64,
    /// 结束拍（不含）。
    end: f64,
    /// 角色：`intro` / `verse` / `chorus` / `bridge` / `outro`…
    #[serde(default)]
    role: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SetSectionsArgs {
    /// 段落表（整体替换；空数组 = 清除）。
    sections: Vec<SectionArg>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct TrackMetaArgs {
    /// 轨道 id。
    track: u64,
    /// 分工：`lead` / `chords` / `bass` / `drums` / `pad` / `fx`…（留空 = 清除）。
    #[serde(default)]
    role: Option<String>,
    /// 目标音域下界（与 `max_pitch` 一起给；都不给 = 清除音域）。
    #[serde(default)]
    min_pitch: Option<u8>,
    /// 目标音域上界。
    #[serde(default)]
    max_pitch: Option<u8>,
    /// MIDI 弯音范围（半音，默认 2）。推弦/揉弦依赖它把音分换算成插件实际范围。
    #[serde(default)]
    pitch_bend_range: Option<f32>,
}

// ---------------------------------------------------------------------------
// 工具实现层（与协议无关）
// ---------------------------------------------------------------------------

/// 工具的**实现**：与 rmcp 无关，写操作统一走 [`commands::run_agent_as`]。
///
/// rmcp 的 `#[tool]` 处理器与内置「写歌智能体」的循环**共用这一份实现**，避免出现两套会
/// 漂移的工具逻辑。署名由 `actor` 决定（MCP = `"mcp"`，内置智能体 = `"chat"`），因此撤销历史、
/// `engine://event` 事件流与音频同步对两个入口完全一致。
pub(crate) mod tools {
    use super::*;

    /// MCP 服务器的工具署名。
    pub(crate) const MCP_ACTOR: &str = "mcp";
    /// 内置「写歌智能体」的工具署名。
    pub(crate) const CHAT_ACTOR: &str = "chat";

    /// 工具返回值统一是一段结构化 JSON 文本（字符串错误版）。
    ///
    /// 用**紧凑格式**：工具结果会直接进模型上下文，缩进只是白白多花 token。
    fn json_s<T: serde::Serialize>(value: &T) -> Result<String, String> {
        serde_json::to_string(value).map_err(|e| format!("序列化失败：{e}"))
    }

    /// 把插件二进制状态（`Vec<u8>`）换成体积摘要。
    ///
    /// `PluginInstance.state` 序列化后是一大串字节数字：一个 AU / VST3 插件的状态就能到
    /// 几百 KB，足以把整份工程快照挤出上下文。工具侧本来也无法回写这段二进制，所以只报
    /// `state_bytes`。参数（`params`）保留——那是可读、可改的部分。
    fn summarize_plugin_states(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(serde_json::Value::Array(bytes)) = map.get("state") {
                    let len = bytes.len();
                    map.remove("state");
                    map.insert("state_bytes".into(), serde_json::json!(len));
                }
                for child in map.values_mut() {
                    summarize_plugin_states(child);
                }
            }
            serde_json::Value::Array(items) => {
                for child in items {
                    summarize_plugin_states(child);
                }
            }
            _ => {}
        }
    }

    /// 工程快照 → 文本：剥掉插件二进制状态后再序列化。
    pub(crate) fn project_json(project: &Project) -> Result<String, String> {
        let mut value = serde_json::to_value(project).map_err(|e| format!("序列化失败：{e}"))?;
        summarize_plugin_states(&mut value);
        json_s(&value)
    }

    /// 现有轨道清单（写进「找不到轨道」的错误里，省掉模型靠猜的第二轮）。
    fn track_choices(project: &Project) -> String {
        project
            .tracks
            .iter()
            .map(|t| format!("#{}「{}」", t.id.0, t.name))
            .collect::<Vec<_>>()
            .join("、")
    }

    pub(crate) fn missing_track(project: &Project, track: u64) -> String {
        if project.tracks.is_empty() {
            format!("没有轨道 {track}：当前工程还没有轨道（先用 add_track 建一条）")
        } else {
            format!("没有轨道 {track}；现有轨道：{}", track_choices(project))
        }
    }

    fn track_or_err<'a>(project: &'a Project, track: u64) -> Result<&'a Track, String> {
        project.track(TrackId(track)).ok_or_else(|| missing_track(project, track))
    }

    /// 片段找不到时把该轨道的现有片段列出来（id + 位置），下一次调用就能用对 id。
    pub(crate) fn missing_clip(track: &Track, clip: u64) -> String {
        if track.clips.is_empty() {
            format!(
                "轨道 {}「{}」上没有片段 {clip}：这条轨道还没有片段（先用 add_clip 建一个）",
                track.id.0, track.name
            )
        } else {
            let choices = track
                .clips
                .iter()
                .map(|c| {
                    format!(
                        "{}（{:.2}–{:.2} 拍，{} 个音符）",
                        c.id.0,
                        c.start,
                        c.start + c.length,
                        c.notes.len()
                    )
                })
                .collect::<Vec<_>>()
                .join("、");
            format!("轨道 {}「{}」上没有片段 {clip}；现有片段：{choices}", track.id.0, track.name)
        }
    }

    /// 校验采样率（字符串错误版）。
    fn check_sample_rate_s(sample_rate: u32) -> Result<u32, String> {
        if SAMPLE_RATES.contains(&sample_rate) {
            Ok(sample_rate)
        } else {
            Err(format!("非法采样率：{sample_rate}（应在 8000..=192000）"))
        }
    }

    /// 解析局部渲染范围：两个参数都没给 = 整曲（`None`）。
    fn resolve_beat_range(
        project: &Project,
        start_beat: Option<f64>,
        end_beat: Option<f64>,
    ) -> Result<Option<(f64, f64)>, String> {
        if start_beat.is_none() && end_beat.is_none() {
            return Ok(None);
        }
        let total = project.length_beats();
        if total <= 0.0 {
            return Err("工程没有内容（length_beats = 0）".into());
        }
        let start = start_beat.unwrap_or(0.0);
        if !start.is_finite() || start < 0.0 {
            return Err(format!("start_beat 必须是非负数（拍），收到 {start}"));
        }
        if start >= total {
            return Err(format!("start_beat {start} 超出工程长度 {total} 拍"));
        }
        let end = end_beat.unwrap_or(total);
        if !end.is_finite() || end <= start {
            return Err(format!("end_beat 必须大于 start_beat（收到 {start}..{end}）"));
        }
        Ok(Some((start, end.min(total))))
    }

    /// 只保留 `tracks` 里的轨道：其他轨道静音（工程快照会被改，不影响正在运行的工程）。
    fn mute_other_tracks(project: &mut Project, tracks: &[u64]) -> Result<(), String> {
        if tracks.is_empty() {
            return Err("tracks 不能为空（不传 = 全部轨道）".into());
        }
        for id in tracks {
            if project.track(TrackId(*id)).is_none() {
                return Err(missing_track(project, *id));
            }
        }
        for track in &mut project.tracks {
            // 局部渲染的语义是「只要这些轨道」：先把 solo 清掉，再静音其余轨道。
            track.mixer.solo = false;
            if !tracks.contains(&track.id.0) {
                track.mixer.mute = true;
            }
        }
        Ok(())
    }

    /// 把整曲渲染结果按**拍**裁出一段（`tail` 是段尾额外保留的秒数）。
    pub(crate) fn slice_render_by_beats(
        buffer: &AudioBuffer,
        transport: &Transport,
        sample_rate: u32,
        start_beat: f64,
        end_beat: f64,
        tail_seconds: f64,
    ) -> AudioBuffer {
        let map = TempoMap::from_transport(transport);
        let sr = sample_rate as f64;
        let start_frame =
            ((map.beats_to_seconds(start_beat) * sr).round() as usize).min(buffer.frames());
        let end_frame = (((map.beats_to_seconds(end_beat) + tail_seconds) * sr).round() as usize)
            .min(buffer.frames());
        let frames = end_frame.saturating_sub(start_frame);
        let channels = buffer.channels();
        let mut out = AudioBuffer::silence(channels, frames);
        if frames > 0 {
            let from = start_frame * channels;
            let to = end_frame * channels;
            out.data_mut().copy_from_slice(&buffer.data()[from..to]);
        }
        out
    }

    // ---- 只读 ----

    pub(crate) fn get_project(app: &AppHandle) -> Result<String, String> {
        let project = app.state::<AppState>().engine.lock().unwrap().project().clone();
        project_json(&project)
    }

    /// 工程的**轻量概览**：id / 名字 / 乐器 / 片段骨架 / 计数，不含音符与插件二进制状态。
    ///
    /// `get_project` 是全量快照；大工程里模型真正先要的是「有哪些轨道、各挂什么、片段 id
    /// 是多少」。把这层单独拆出来，避免为了看一眼结构就吞掉几百 KB 的插件状态。
    pub(crate) fn get_project_overview(app: &AppHandle) -> Result<String, String> {
        let project = snapshot(app);
        let entries = catalog_entries(app);
        json_s(&project_overview(&project, &entries))
    }

    pub(crate) fn project_overview(project: &Project, entries: &[plugin_info::Entry]) -> serde_json::Value {
        let plugin = |instance: &PluginInstance| {
            serde_json::json!({
                "id": instance.plugin_id,
                "name": entries
                    .iter()
                    .find(|e| e.id == instance.plugin_id)
                    .map(|e| e.name.as_str()),
                "params": instance.params.len(),
                "state_bytes": instance.state.len(),
            })
        };

        let tracks: Vec<serde_json::Value> = project
            .tracks
            .iter()
            .map(|t| {
                let clips: Vec<serde_json::Value> = t
                    .clips
                    .iter()
                    .map(|c| {
                        let mut item = serde_json::json!({
                            "id": c.id.0,
                            "kind": if c.is_audio() { "audio" } else { "midi" },
                            "start": c.start,
                            "length": c.length,
                            "notes": c.notes.len(),
                        });
                        if let Some(audio) = &c.audio {
                            item["path"] = serde_json::json!(audio.path);
                        }
                        item
                    })
                    .collect();
                let note_count: usize = t.clips.iter().map(|c| c.notes.len()).sum();
                serde_json::json!({
                    "id": t.id.0,
                    "name": t.name,
                    "role": t.role(),
                    "instrument": t.instrument.as_ref().map(plugin),
                    "effects": t
                        .effects
                        .iter()
                        .enumerate()
                        .map(|(index, effect)| {
                            let mut item = plugin(effect);
                            item["index"] = serde_json::json!(index);
                            item
                        })
                        .collect::<Vec<_>>(),
                    "mixer": {
                        "gain_db": t.mixer.gain_db,
                        "pan": t.mixer.pan,
                        "mute": t.mixer.mute,
                        "solo": t.mixer.solo,
                    },
                    "note_count": note_count,
                    "clips": clips,
                })
            })
            .collect();

        let clip_count: usize = project.tracks.iter().map(|t| t.clips.len()).sum();
        let note_count: usize = project.tracks.iter().flat_map(|t| &t.clips).map(|c| c.notes.len()).sum();
        let bar_beats = project.transport.bar_beats();
        let length_beats = project.length_beats();
        let bars = if bar_beats > 0.0 { (length_beats / bar_beats).ceil() } else { 0.0 };

        serde_json::json!({
            "name": project.name,
            "transport": {
                "tempo": project.transport.tempo,
                "time_sig": format!(
                    "{}/{}",
                    project.transport.time_sig_num, project.transport.time_sig_den
                ),
                "keyframes": project.transport.keyframes.len(),
                "length_beats": length_beats,
                "bars": bars,
            },
            "totals": {
                "tracks": project.tracks.len(),
                "clips": clip_count,
                "notes": note_count,
            },
            "master": {
                "gain_db": project.master.gain_db,
                "pan": project.master.pan,
            },
            "harmony_bars": project.harmony.len(),
            "sections": project
                .sections
                .iter()
                .map(|s| serde_json::json!({
                    "name": s.name,
                    "start": s.start,
                    "end": s.end,
                    "role": s.role,
                }))
                .collect::<Vec<_>>(),
            "tracks": tracks,
            "hint": "概览不含音符与插件二进制状态。逐音核对用 get_clip_notes(track, clip)；\
                     要看全部字段（含音符与插件参数）再用 get_project。",
        })
    }

    pub(crate) fn get_transport(app: &AppHandle) -> Result<String, String> {
        json_s(&app.state::<AppState>().audio.snapshot())
    }

    /// 按页读取一个片段的音符：大工程里核对每个音符时用它，避免整份工程快照撑爆上下文。
    pub(crate) fn get_clip_notes(app: &AppHandle, args: GetNotesArgs) -> Result<String, String> {
        let project = snapshot(app);
        let clip = clip_of(&project, args.track, args.clip)?;
        let range = args.range.map(|r| r.to_range());
        let selected: Vec<&Note> = clip
            .notes
            .iter()
            .filter(|n| range.map_or(true, |r| r.contains(n)))
            .collect();
        let total = selected.len();
        let offset = args.offset.unwrap_or(0).min(total);
        let limit = args.limit.unwrap_or(200).clamp(1, 1000);
        let end = (offset + limit).min(total);
        let tags: serde_json::Map<String, serde_json::Value> = selected[offset..end]
            .iter()
            .filter_map(|note| {
                project
                    .note_tag(TrackId(args.track), ClipId(args.clip), note.id)
                    .map(|tag| {
                        (
                            note.id.to_string(),
                            serde_json::to_value(tag).unwrap_or(serde_json::Value::Null),
                        )
                    })
            })
            .collect();
        json_s(&serde_json::json!({
            "track": args.track,
            "clip": args.clip,
            "start": clip.start,
            "length": clip.length,
            "note_count": clip.notes.len(),
            "selected": total,
            "offset": offset,
            "limit": limit,
            "notes": &selected[offset..end],
            "tags": tags,
            "truncated": end < total,
            "next_offset": if end < total { Some(end) } else { None },
        }))
    }

    /// 当前可用插件：内置 + 本机扫描到的外部插件（每次调用现取，重扫后立刻生效）。
    fn catalog_entries(app: &AppHandle) -> Vec<plugin_info::Entry> {
        plugin_info::entries(&app.state::<AppState>().catalog_snapshot())
    }

    /// `list_plugins` 的固定尾巴：把「怎么读这张表」说清楚，省得模型只看名字猜用途。
    const LIST_PLUGINS_HINT: &str = "category / description / roles 说明「这件东西是干什么的」；\
pitch_range 是建议写作音域（MIDI 音高）；mono=true 是单音乐器，不要叠和弦。\
这张表**只给描述**，不带参数与预设；真要用某一件时再调 describe_plugin 拿它的全部参数与预设。别凭名字猜音色——认不出的条目会自己说明「未收录」。";

    /// 列出可用乐器与效果器，带**分类 + 一句话描述 + 推荐用途**。
    ///
    /// 老版本只回传扫描结果的原始字段（id / name / vendor / kind / format），模型看到
    /// `JC303` 完全不知道那是 TB-303 贝斯；而且**内置合成器根本不在里面**。
    pub(crate) fn list_plugins(app: &AppHandle, args: ListPluginsArgs) -> Result<String, String> {
        let want = match args.kind.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
            None | Some("all") | Some("全部") => None,
            Some("instrument" | "instruments" | "乐器") => Some(PluginKind::Instrument),
            Some("effect" | "effects" | "效果" | "效果器") => Some(PluginKind::Effect),
            Some(other) => {
                return Err(format!("kind 只能是 instrument / effect / all，收到「{other}」"))
            }
        };
        let query = args
            .query
            .as_deref()
            .map(|q| q.trim().to_lowercase())
            .filter(|q| !q.is_empty());

        let mut instruments = Vec::new();
        let mut effects = Vec::new();
        for entry in catalog_entries(app) {
            if want.is_some_and(|kind| entry.kind != kind) {
                continue;
            }
            let brief = plugin_info::brief(&entry);
            if let Some(query) = &query {
                let haystack = format!(
                    "{} {} {} {} {}",
                    brief.name,
                    brief.vendor,
                    brief.category,
                    brief.description,
                    brief.tags.join(" ")
                )
                .to_lowercase();
                if !haystack.contains(query) {
                    continue;
                }
            }
            match brief.kind {
                PluginKind::Instrument => instruments.push(brief),
                PluginKind::Effect => effects.push(brief),
            }
        }
        let counts = serde_json::json!({
            "instruments": instruments.len(),
            "effects": effects.len(),
        });
        json_s(&serde_json::json!({
            "counts": counts,
            "instruments": instruments,
            "effects": effects,
            "hint": LIST_PLUGINS_HINT,
        }))
    }

    /// 列出内置合成器的出厂音色（36 条）：id / 名称 / 分类 / 描述 / **完整参数补丁**。
    ///
    /// 这是「捏音色」的起点目录：先用 `preview_synth` 试听，满意后把 `params` 原样喂给
    /// `set_params(track, params, reset=true)`。它不依赖工程状态，所以不需要 `AppHandle`。
    pub(crate) fn list_synth_presets(args: ListSynthPresetsArgs) -> Result<String, String> {
        let category = args
            .category
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(|c| c.to_ascii_lowercase());
        let query = args
            .query
            .as_deref()
            .map(|q| q.trim().to_lowercase())
            .filter(|q| !q.is_empty());

        let items: Vec<&factory_presets::FactoryPreset> = factory_presets::all()
            .iter()
            .filter(|preset| {
                if let Some(category) = &category {
                    if !preset.category.eq_ignore_ascii_case(category) {
                        return false;
                    }
                }
                if let Some(query) = &query {
                    let haystack =
                        format!("{} {} {}", preset.id, preset.label, preset.description).to_lowercase();
                    if !haystack.contains(query) {
                        return false;
                    }
                }
                true
            })
            .collect();

        let items: Vec<serde_json::Value> = items
            .into_iter()
            .map(|preset| {
                serde_json::json!({
                    "id": preset.id,
                    "label": preset.label,
                    "category": preset.category,
                    "description": preset.description,
                    "params": preset.params,
                })
            })
            .collect();

        json_s(&serde_json::json!({
            "count": items.len(),
            "total": factory_presets::all().len(),
            "categories": factory_presets::categories(),
            "items": items,
            "hint": "params 是补丁；先用 preview_synth({params, reset:true}) 试听，满意后 set_params(track, params, reset:true) 落地（一个撤销步）。",
        }))
    }

    /// `describe_plugin` 一次最多返回多少个**音色**参数（安全阀：防某个插件真的暴露上千个旋钮）。
    const MAX_PARAMS: usize = 400;
    /// `describe_plugin` 一次最多带多少条用户预设。
    const MAX_PRESETS: usize = 50;

    /// 参数是不是「管道」而不是「音色旋钮」：MIDI CC 映射这类每个通道 128 个，动辄两千条，
    /// 调音色时完全用不上。默认不返回，但会在 `param_hidden` 里报个数——不假装它们不存在。
    pub(super) fn is_plumbing_param(name: &str) -> bool {
        let lower = name.trim().to_ascii_lowercase();
        lower.starts_with("midi cc") || lower.starts_with("midi learn")
    }

    /// 单个插件的详细说明：卡片 + **全部音色参数** + **该插件的预设** + 怎么用。
    ///
    /// 这是渐进披露的第二层：`list_plugins` 只给描述（不知道参数长什么样），
    /// 真要用某一件时再调这里，一次拿到它的参数与预设。
    ///
    /// 参数表要加载插件（外部插件是 dlopen），所以调用方必须已经把它放到阻塞线程上。
    pub(crate) fn describe_plugin(app: &AppHandle, args: DescribePluginArgs) -> Result<String, String> {
        let entries = catalog_entries(app);
        let entry = plugin_info::resolve(&entries, &args.plugin_id)?;
        let brief = plugin_info::brief(&entry);
        let catalog = app.state::<AppState>().catalog_snapshot();
        let params = commands::plugin_params_with(&catalog, &entry.id);

        let mut out = serde_json::to_value(&brief).map_err(|e| format!("序列化失败：{e}"))?;
        let Some(map) = out.as_object_mut() else {
            return Err("序列化失败：插件卡片不是对象".into());
        };
        match params {
            Ok(list) => {
                // 先摘掉 MIDI CC 这类管道参数：JC303 / KR-106 都靠这一刀从 2100 降到 50~90。
                let mut hidden = 0usize;
                let mut sound: Vec<&ai_daw_plugin_api::ParamInfo> = Vec::new();
                for param in &list {
                    if is_plumbing_param(&param.name) {
                        hidden += 1;
                    } else {
                        sound.push(param);
                    }
                }
                // `query` 只用来在**这一件**插件内部缩小范围（如只看 cutoff / resonance）。
                let needle = args
                    .query
                    .as_deref()
                    .map(|q| q.trim().to_lowercase())
                    .filter(|q| !q.is_empty());
                let matched: Vec<&ai_daw_plugin_api::ParamInfo> = match &needle {
                    Some(needle) => sound
                        .iter()
                        .copied()
                        .filter(|p| {
                            p.id.0.to_lowercase().contains(needle)
                                || p.name.to_lowercase().contains(needle)
                        })
                        .collect(),
                    None => sound.clone(),
                };
                // 缺省给**全部**音色参数；只有超过安全阀才截断。
                let cap = args.max_params.unwrap_or(MAX_PARAMS).clamp(1, MAX_PARAMS);
                let shown = &matched[..matched.len().min(cap)];
                map.insert("param_count".into(), serde_json::json!(sound.len()));
                map.insert("param_hidden".into(), serde_json::json!(hidden));
                map.insert("params_truncated".into(), serde_json::json!(matched.len() > shown.len()));
                if hidden > 0 {
                    map.insert(
                        "params_note".into(),
                        serde_json::json!(format!(
                            "另有 {hidden} 个 MIDI CC 映射参数没有列出（不是音色参数，调音色用不到）。"
                        )),
                    );
                }
                map.insert(
                    "params".into(),
                    serde_json::json!(shown
                        .iter()
                        .map(|p| serde_json::json!({
                            "id": p.id.0,
                            "name": p.name,
                            "min": p.min,
                            "max": p.max,
                            "default": p.default,
                        }))
                        .collect::<Vec<_>>()),
                );
            }
            // 参数拿不到不该挡住「它是什么」——降级给出原因，描述照常返回。
            Err(err) => {
                map.insert("params_error".into(), serde_json::json!(err));
            }
        }
        map.insert("presets".into(), serde_json::json!(presets_for(app, &entry.id)));
        // 内置合成器额外给一份出厂音色目录（不带参数，参数用 list_synth_presets 按需取）。
        if entry.id == ai_daw_engine::registry::BUILTIN_SYNTH {
            let items: Vec<serde_json::Value> = factory_presets::all()
                .iter()
                .map(|preset| {
                    serde_json::json!({
                        "id": preset.id,
                        "label": preset.label,
                        "category": preset.category,
                        "description": preset.description,
                    })
                })
                .collect();
            map.insert(
                "factory_presets".into(),
                serde_json::json!({
                    "count": items.len(),
                    "categories": factory_presets::categories(),
                    "items": items,
                    "hint": "完整参数用 list_synth_presets 取；落地用 set_params(track, params, reset=true)。",
                }),
            );
        }
        map.insert("usage".into(), serde_json::json!(usage_hints(&brief)));
        json_s(&out)
    }

    /// 这个插件的用户预设（跨工程，存在 `presets.json`）：名字 + 参数快照。
    ///
    /// 预设的 `params` 就是「这条音色怎么调」，模型可以原样喂给 `set_params` 复用；
    /// 只有确实要连效果链一起换时才用 `apply_preset`（它的 `effects` 是整体替换）。
    fn presets_for(app: &AppHandle, plugin_id: &str) -> serde_json::Value {
        let state = app.state::<AppState>();
        let store = state.presets.lock().unwrap();
        let all = store.all();
        let matched: Vec<&crate::presets::Preset> =
            all.iter().filter(|p| p.plugin_id == plugin_id).collect();
        let truncated = matched.len() > MAX_PRESETS;
        let list: Vec<serde_json::Value> = matched
            .iter()
            .take(MAX_PRESETS)
            .map(|p| {
                serde_json::json!({
                    "id": p.id,
                    "name": p.name,
                    "params": p.params,
                })
            })
            .collect();
        serde_json::json!({
            "count": matched.len(),
            "truncated": truncated,
            "items": list,
        })
    }

    /// 「这件东西在这个 DAW 里怎么用」——按乐器 / 效果器分流。
    fn usage_hints(brief: &plugin_info::Brief) -> Vec<String> {
        let mut out = Vec::new();
        match brief.kind {
            PluginKind::Instrument => {
                out.push(format!(
                    "挂到轨道：set_track_instrument(track, \"{}\")",
                    brief.id
                ));
                out.push(
                    "调参数：set_param(track, <参数 id>, 值)；**改多个旋钮用 set_params(track, {…}, reset=true)**——一次事务 = 一个撤销步。数值按上面 params 里的 min..max 给。".into(),
                );
                out.push(
                    "捏音色闭环：`preview_synth`（内置乐器，离线渲染一个音并给出频谱重心 / 平坦度 / 波峰因数 / 分段能量；**只渲染乐器本身，不含效果链**）→ 改参数 → 再 preview；满意后用 `set_params` 落地。外部插件没有离线预览，挂轨后用 `analyze_project` 核对。".into(),
                );
                out.push(
                    "参数被截断时用 describe_plugin 的 `query` 缩范围（如 query=\"cutoff\"），或调大 `max_params`。"
                        .into(),
                );
                if brief.id == ai_daw_engine::registry::BUILTIN_SYNTH {
                    out.push(
                        "出厂 36 条音色用 list_synth_presets(category?, query?) 取完整参数补丁；先 preview_synth 试听，再 set_params(track, params, reset=true) 落地。".into(),
                    );
                }
                if brief.mono {
                    out.push(
                        "**单音乐器**：同一时刻只发一个音，不要叠和弦（叠了会被抢占/吞掉）。".into(),
                    );
                }
                if let Some([min, max]) = brief.pitch_range {
                    out.push(format!(
                        "建议写作音域 MIDI {min}–{max}；可用 set_track_meta 把 role / pitch_range 写进轨道，fit_notes_to_range 会照它折返。"
                    ));
                }
                if !brief.roles.is_empty() {
                    out.push(format!("适合的分工：{}。", brief.roles.join(" / ")));
                }
            }
            PluginKind::Effect => {
                out.push(format!(
                    "追加到效果链末尾：add_effect(track, \"{}\")",
                    brief.id
                ));
                out.push(
                    "链上第 i 个效果器的参数可以用 set_automation 的 `fx<i>:<参数id>` 做自动化。"
                        .into(),
                );
                out.push(
                    "要整条换掉用 apply_preset 的 effects（**整体替换**，想保留的现有效果器要一并写回）。"
                        .into(),
                );
            }
        }
        out
    }

    // ---- 传输 ----

    pub(crate) fn transport_play(app: &AppHandle) -> Result<String, String> {
        app.state::<AppState>().audio.send(TransportCommand::Play).map_err(|e| e.to_string())?;
        Ok("已开始播放".into())
    }

    pub(crate) fn transport_pause(app: &AppHandle) -> Result<String, String> {
        app.state::<AppState>().audio.send(TransportCommand::Pause).map_err(|e| e.to_string())?;
        Ok("已暂停".into())
    }

    pub(crate) fn transport_stop(app: &AppHandle) -> Result<String, String> {
        app.state::<AppState>().audio.send(TransportCommand::Stop).map_err(|e| e.to_string())?;
        Ok("已停止".into())
    }

    pub(crate) fn transport_seek(app: &AppHandle, args: SeekArgs) -> Result<String, String> {
        let state = app.state::<AppState>();
        let sample_rate = {
            let rate = state.audio.snapshot().sample_rate;
            if rate == 0 { FALLBACK_SAMPLE_RATE } else { rate }
        };
        let frame = (args.seconds.max(0.0) * sample_rate as f64).round() as u64;
        state.audio.send(TransportCommand::Seek { frame }).map_err(|e| e.to_string())?;
        Ok(format!("已定位到 {} 秒（第 {} 帧）", args.seconds, frame))
    }

    pub(crate) fn transport_set_loop(app: &AppHandle, args: ToggleNoTrackArgs) -> Result<String, String> {
        app.state::<AppState>().audio.send(TransportCommand::SetLoop(args.on)).map_err(|e| e.to_string())?;
        Ok(if args.on { "循环已开启".into() } else { "循环已关闭".into() })
    }

    // ---- 工程与轨道 ----

    pub(crate) fn set_tempo(app: &AppHandle, actor: &Actor, args: TempoArgs) -> Result<String, String> {
        let project = commands::set_tempo_as(app, actor.clone(), args.bpm);
        Ok(format!("速度已设为 {:.1} BPM", project.transport.tempo))
    }

    pub(crate) fn set_time_signature(app: &AppHandle, actor: &Actor, args: TimeSigArgs) -> Result<String, String> {
        commands::run_agent_as(app, actor.clone(), Command::SetTimeSignature { num: args.num, den: args.den });
        Ok(format!("拍号已设为 {}/{}", args.num, args.den))
    }

    pub(crate) fn set_keyframe(app: &AppHandle, actor: &Actor, args: KeyframeArgs) -> Result<String, String> {
        let project = commands::set_keyframe_as(
            app,
            actor.clone(),
            args.beat,
            args.tempo,
            args.num,
            args.den,
        );
        Ok(format!(
            "第 {} 拍已写入速度关键点：{:.1} BPM · {}/{}（当前共 {} 个关键点）",
            args.beat,
            args.tempo,
            args.num,
            args.den,
            project.transport.keyframes.len()
        ))
    }

    pub(crate) fn remove_keyframe(app: &AppHandle, actor: &Actor, args: BeatArgs) -> Result<String, String> {
        let project = commands::remove_keyframe_as(app, actor.clone(), args.beat);
        Ok(format!(
            "第 {} 拍的速度关键点已删除（当前共 {} 个关键点）",
            args.beat,
            project.transport.keyframes.len()
        ))
    }

    pub(crate) fn add_track(app: &AppHandle, actor: &Actor, args: NameArgs) -> Result<String, String> {
        let (project, events) = commands::run_agent_as(app, actor.clone(), Command::AddTrack { name: args.name.clone() });
        let id = track_added_id(&events);
        let tracks = project.tracks.len();
        Ok(format!("已新增轨道「{}」{}（现有 {} 条轨道）", args.name, id.map(|i| format!("，id={i}")).unwrap_or_default(), tracks))
    }

    pub(crate) fn remove_track(app: &AppHandle, actor: &Actor, args: TrackArgs) -> Result<String, String> {
        commands::run_agent_as(app, actor.clone(), Command::RemoveTrack(TrackId(args.track)));
        Ok(format!("已删除轨道 {}", args.track))
    }

    pub(crate) fn rename_track(app: &AppHandle, actor: &Actor, args: RenameArgs) -> Result<String, String> {
        commands::run_agent_as(app, actor.clone(), Command::RenameTrack { track: TrackId(args.track), name: args.name.clone() });
        Ok(format!("轨道 {} 已更名为「{}」", args.track, args.name))
    }

    pub(crate) fn set_track_instrument(app: &AppHandle, actor: &Actor, args: InstrumentArgs) -> Result<String, String> {
        // 空 id = 卸载乐器（与引擎语义一致，不能拿去解析）。
        if args.plugin_id.trim().is_empty() {
            commands::run_agent_as(app, actor.clone(), Command::SetTrackInstrument { track: TrackId(args.track), plugin_id: String::new() });
            return Ok(format!("轨道 {} 的乐器已卸载", args.track));
        }
        // 名字也认（`JC303` / `Ultramaster KR-106`）：模型把名字当 id 传是常态。
        let entries = catalog_entries(app);
        let entry = plugin_info::resolve(&entries, &args.plugin_id)?;
        let brief = plugin_info::brief(&entry);
        if brief.kind == PluginKind::Effect {
            return Err(format!(
                "「{}」是效果器，不能当乐器挂在轨道上；要加进效果链请用 add_effect。",
                brief.name
            ));
        }
        commands::run_agent_as(app, actor.clone(), Command::SetTrackInstrument { track: TrackId(args.track), plugin_id: entry.id.clone() });
        // 挂上的同时把「这是什么」讲清楚：模型不必再查一次目录就知道接下来该怎么写音符。
        let mut text = format!(
            "轨道 {} 的乐器已设为 {}（{}，id={}）：{}",
            args.track,
            brief.name,
            plugin_info::category_label(brief.category),
            brief.id,
            brief.description
        );
        if brief.mono {
            text.push_str(" 这是单音乐器，不要叠和弦。");
        }
        if let Some([min, max]) = brief.pitch_range {
            text.push_str(&format!(" 建议写作音域 MIDI {min}–{max}。"));
        }
        if !brief.roles.is_empty() {
            text.push_str(&format!(" 建议分工：{}（可用 set_track_meta 写进轨道）。", brief.roles.join(" / ")));
        }
        Ok(text)
    }

    /// 轨道效果链的可读形式（`0:内置混响 → 1:JC303 Overdrive`），用于写操作的回执。
    fn effect_chain(app: &AppHandle, track: u64) -> Result<String, String> {
        let project = app.state::<AppState>().engine.lock().unwrap().project().clone();
        let t = track_or_err(&project, track)?;
        if t.effects.is_empty() {
            return Ok("空".into());
        }
        let entries = catalog_entries(app);
        let parts: Vec<String> = t
            .effects
            .iter()
            .enumerate()
            .map(|(i, effect)| {
                let name = entries
                    .iter()
                    .find(|e| e.id == effect.plugin_id)
                    .map(|e| e.name.clone())
                    .unwrap_or_else(|| effect.plugin_id.clone());
                format!("{i}:{name}")
            })
            .collect();
        Ok(parts.join(" → "))
    }

    pub(crate) fn add_effect(app: &AppHandle, actor: &Actor, args: EffectArgs) -> Result<String, String> {
        let entries = catalog_entries(app);
        let entry = plugin_info::resolve(&entries, &args.plugin_id)?;
        let brief = plugin_info::brief(&entry);
        if brief.kind == PluginKind::Instrument {
            return Err(format!(
                "「{}」是乐器，不能加进效果链；要挂乐器请用 set_track_instrument。",
                brief.name
            ));
        }
        commands::run_agent_as(app, actor.clone(), Command::AddEffect { track: TrackId(args.track), plugin_id: entry.id.clone() });
        let chain = effect_chain(app, args.track)?;
        Ok(format!(
            "已把 {}（{}，id={}）追加到轨道 {} 的效果链末尾。{} 当前链：{}",
            brief.name,
            plugin_info::category_label(brief.category),
            brief.id,
            args.track,
            brief.description,
            chain
        ))
    }

    pub(crate) fn remove_effect(app: &AppHandle, actor: &Actor, args: EffectIndexArgs) -> Result<String, String> {
        commands::run_agent_as(app, actor.clone(), Command::RemoveEffect { track: TrackId(args.track), index: args.index });
        let chain = effect_chain(app, args.track)?;
        Ok(format!("已移除轨道 {} 效果链的第 {} 个。当前链：{}", args.track, args.index, chain))
    }

    pub(crate) fn set_effect_param(
        app: &AppHandle,
        actor: &Actor,
        args: EffectParamArgs,
    ) -> Result<String, String> {
        if !args.value.is_finite() {
            return Err(format!("参数 {} 的值不是有限数：{}", args.param, args.value));
        }
        commands::run_agent_as(
            app,
            actor.clone(),
            Command::SetEffectParam {
                track: TrackId(args.track),
                index: args.index,
                param: args.param.clone(),
                value: args.value,
            },
        );
        Ok(format!(
            "轨道 {} 效果链第 {} 个的 {} 已设为 {}",
            args.track, args.index, args.param, args.value
        ))
    }

    /// 一次写入一个效果器的整组参数（一个撤销步）。
    pub(crate) fn set_effect_params(
        app: &AppHandle,
        actor: &Actor,
        args: EffectParamsArgs,
    ) -> Result<String, String> {
        if args.params.is_empty() {
            return Err("params 为空：至少给一个参数".into());
        }
        // 固定顺序：HashMap 的遍历顺序随机，排序后撤销历史与事件流才可复现。
        let mut pairs: Vec<(String, f32)> = args.params.into_iter().collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        let mut commands = Vec::with_capacity(pairs.len());
        let mut applied = Vec::with_capacity(pairs.len());
        for (param, value) in pairs {
            if !value.is_finite() {
                return Err(format!("参数 {param} 的值不是有限数：{value}"));
            }
            applied.push(format!("{param}={value}"));
            commands.push(Command::SetEffectParam {
                track: TrackId(args.track),
                index: args.index,
                param,
                value,
            });
        }
        commands::run_agent_all_as(app, actor.clone(), commands);
        Ok(format!(
            "轨道 {} 效果链第 {} 个已写入 {} 个参数（一个撤销步）：{}",
            args.track,
            args.index,
            applied.len(),
            applied.join(", ")
        ))
    }

    pub(crate) fn move_effect(
        app: &AppHandle,
        actor: &Actor,
        args: MoveEffectArgs,
    ) -> Result<String, String> {
        commands::run_agent_as(
            app,
            actor.clone(),
            Command::MoveEffect { track: TrackId(args.track), from: args.from, to: args.to },
        );
        let chain = effect_chain(app, args.track)?;
        Ok(format!(
            "轨道 {} 效果链的第 {} 个已移到第 {} 位。当前链：{}",
            args.track, args.from, args.to, chain
        ))
    }

    pub(crate) fn set_param(app: &AppHandle, actor: &Actor, args: ParamArgs) -> Result<String, String> {
        commands::run_agent_as(app, actor.clone(), Command::SetParam { track: TrackId(args.track), param: args.param.clone(), value: args.value });
        Ok(format!("轨道 {} 的参数 {} 已设为 {}", args.track, args.param, args.value))
    }

    /// 一次写入整组乐器参数（一个撤销步）。
    ///
    /// 捏音色一次要改 5–10 个旋钮；逐个 `set_param` 会产生同样多条撤销记录，
    /// 人想回到「上一个音色」时得按十几次撤销。
    pub(crate) fn set_params(
        app: &AppHandle,
        actor: &Actor,
        args: SetParamsArgs,
    ) -> Result<String, String> {
        if args.params.is_empty() && !args.reset {
            return Err("params 为空：至少给一个参数（或 reset=true 复位全部参数）".into());
        }
        let project = app.state::<AppState>().engine.lock().unwrap().project().clone();
        let track = project
            .tracks
            .iter()
            .find(|t| t.id == TrackId(args.track))
            .ok_or_else(|| format!("轨道 {} 不存在", args.track))?;
        let plugin_id = track
            .instrument
            .as_ref()
            .map(|i| i.plugin_id.clone())
            .ok_or_else(|| format!("轨道 {} 没有挂乐器", args.track))?;
        let catalog = app.state::<AppState>().catalog_snapshot();
        let specs = commands::plugin_params_with(&catalog, &plugin_id)?;

        let mut limits: HashMap<String, (f32, f32)> = HashMap::new();
        for spec in &specs {
            limits.insert(spec.id.0.clone(), (spec.min, spec.max));
        }
        let mut unknown: Vec<&str> = args
            .params
            .keys()
            .filter(|id| !limits.contains_key(id.as_str()))
            .map(String::as_str)
            .collect();
        unknown.sort_unstable();
        if !unknown.is_empty() {
            let mut known: Vec<&str> = limits.keys().map(String::as_str).collect();
            known.sort_unstable();
            return Err(format!(
                "这些参数在 {plugin_id} 上不存在：{}。可用参数：{}",
                unknown.join(", "),
                known.join(", ")
            ));
        }

        // 固定顺序：HashMap 的遍历顺序随机，排序后撤销历史与事件流才可复现。
        let mut pairs: Vec<(String, f32)> =
            args.params.iter().map(|(id, value)| (id.clone(), *value)).collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        let override_count = pairs.len();
        let mut clamped = 0usize;
        let mut commands = Vec::with_capacity(pairs.len() + specs.len());
        let mut applied = Vec::with_capacity(pairs.len() + 1);
        if args.reset {
            // 复位也走同一批命令：整组参数与覆盖值是**一个**撤销步。
            // 只复位音色旋钮——外部插件的 MIDI CC 映射动辄上千条，既不该被复位，也不该进撤销历史。
            let reset: Vec<_> = specs.iter().filter(|s| !is_plumbing_param(&s.name)).collect();
            for spec in &reset {
                commands.push(Command::SetParam {
                    track: TrackId(args.track),
                    param: spec.id.0.clone(),
                    value: spec.default,
                });
            }
            applied.push(format!("复位到默认（{} 个音色参数）", reset.len()));
        }
        for (param, value) in pairs {
            if !value.is_finite() {
                return Err(format!("参数 {param} 的值不是有限数：{value}"));
            }
            let (min, max) = limits[&param];
            let value = value.clamp(min, max);
            if (value - args.params[&param]).abs() > f32::EPSILON {
                clamped += 1;
            }
            applied.push(format!("{param}={value}"));
            commands.push(Command::SetParam { track: TrackId(args.track), param, value });
        }
        commands::run_agent_all_as(app, actor.clone(), commands);
        let suffix = if clamped > 0 { format!("；{clamped} 个值被夹到合法范围") } else { String::new() };
        Ok(format!(
            "轨道 {} 已应用音色补丁（一个撤销步，{} 个覆盖参数）{suffix}：{}",
            args.track,
            override_count,
            applied.join(", ")
        ))
    }

    /// 渲染一个「按下 → 松开 → 尾巴」的试听片段（单声道）。
    ///
    /// 独立出来是为了能脱离 `AppHandle` 测试真正的发声路径。
    pub(super) fn render_audition(
        plugin: &mut dyn ai_daw_plugin_api::Plugin,
        sample_rate: u32,
        note: u8,
        velocity: u8,
        duration_ms: u32,
        tail_ms: u32,
    ) -> Vec<f32> {
        let sr = sample_rate as f64;
        let total_frames = (((duration_ms + tail_ms) as f64) * sr / 1000.0).ceil() as usize;
        let note_off_frame = (duration_ms as f64 * sr / 1000.0).round() as usize;
        const BLOCK: usize = 256;
        let ctx = ProcessContext { sample_rate: sr, block_size: BLOCK, tempo: 120.0, playing: true };

        let mut mono: Vec<f32> = Vec::with_capacity(total_frames);
        let mut pos = 0usize;
        while pos < total_frames {
            let frames = (total_frames - pos).min(BLOCK);
            let mut left = vec![0.0f32; frames];
            let mut right = vec![0.0f32; frames];
            let mut events = Vec::new();
            if pos == 0 {
                events.push(TimedMidiEvent {
                    frame: 0,
                    event: MidiEvent::NoteOn { note, velocity },
                });
            }
            if note_off_frame >= pos && note_off_frame < pos + frames {
                events.push(TimedMidiEvent {
                    frame: (note_off_frame - pos) as u32,
                    event: MidiEvent::NoteOff { note },
                });
            }
            plugin.process(&ctx, &mut [&mut left, &mut right], &events);
            // 下混成立体声：`spread > 0` 时有些声部在右声道，只看左声道会漏掉它们。
            // `spread = 0` 时左右完全相等，`0.5*(l+r)` 与原来的 `l` 逐位一致。
            mono.extend(left.iter().zip(right.iter()).map(|(l, r)| (l + r) * 0.5));
            pos += frames;
        }
        mono
    }

    /// 离线试听一个内置乐器音色：一个音 + 包络尾巴，返回响度与音色指纹。
    ///
    /// 这是「捏音色」的耳朵：模型听不到音频，但能比较 `spectral_centroid_hz`（亮/暗）、
    /// `spectral_flatness`（噪声性）、`crest_factor_db`（冲击力）与分段能量。
    /// 不写工程、不进撤销历史，可以放心反复调。
    pub(crate) fn preview_synth(
        app: &AppHandle,
        args: PreviewSynthArgs,
    ) -> Result<String, String> {
        let sample_rate = check_sample_rate_s(args.sample_rate.unwrap_or(48_000))?;
        let sr = sample_rate as f64;

        let mut plugin_id = "builtin.synth".to_string();
        let mut base_params: Vec<(String, f32)> = Vec::new();
        let mut state: Vec<u8> = Vec::new();
        if let Some(track_id) = args.track {
            let project = app.state::<AppState>().engine.lock().unwrap().project().clone();
            let track = project
                .tracks
                .iter()
                .find(|t| t.id == TrackId(track_id))
                .ok_or_else(|| format!("轨道 {track_id} 不存在"))?;
            let instrument = track
                .instrument
                .as_ref()
                .ok_or_else(|| format!("轨道 {track_id} 没有挂乐器"))?;
            plugin_id = instrument.plugin_id.clone();
            base_params = instrument.params.iter().map(|(k, v)| (k.clone(), *v)).collect();
            state = instrument.state.clone();
        }

        let mut plugin = PluginRegistry::with_builtins()
            .create(&plugin_id, TrackId(0), PluginSlot::Instrument)
            .ok_or_else(|| {
                format!("preview_synth 只支持内置乐器；{plugin_id} 不是内置乐器（外部插件请挂轨后 analyze_project）")
            })?;

        if plugin.descriptor().kind != PluginKind::Instrument {
            return Err(format!("{plugin_id} 不是乐器，不能预听音色"));
        }

        let specs = plugin.params();
        let mut limits: HashMap<String, (f32, f32)> = HashMap::new();
        for spec in &specs {
            limits.insert(spec.id.0.clone(), (spec.min, spec.max));
        }

        let mut overrides: Vec<(String, f32)> =
            args.params.iter().map(|(id, value)| (id.clone(), *value)).collect();
        overrides.sort_by(|a, b| a.0.cmp(&b.0));
        let mut unknown: Vec<&str> = overrides
            .iter()
            .map(|(id, _)| id.as_str())
            .filter(|id| !limits.contains_key(*id))
            .collect();
        unknown.sort_unstable();
        if !unknown.is_empty() {
            let mut known: Vec<&str> = limits.keys().map(String::as_str).collect();
            known.sort_unstable();
            return Err(format!(
                "这些参数在 {plugin_id} 上不存在：{}。可用参数：{}",
                unknown.join(", "),
                known.join(", ")
            ));
        }
        for (id, value) in overrides.iter() {
            if !value.is_finite() {
                return Err(format!("参数 {id} 的值不是有限数：{value}"));
            }
        }

        // 轨道状态优先（它可能包含参数之外的内部状态），随后轨道参数与本次覆盖依次生效。
        // `reset` 则跳过两者，从参数表默认值开始——换音色试听时不会被上一条音色污染。
        if args.reset {
            for spec in &specs {
                plugin.set_param(&spec.id, spec.default);
            }
        } else {
            if !state.is_empty() {
                let _ = plugin.load_state(&state);
            }
            for (id, value) in base_params.iter() {
                if let Some((min, max)) = limits.get(id.as_str()) {
                    plugin.set_param(&ParamId::from(id.as_str()), (*value).clamp(*min, *max));
                }
            }
        }
        for (id, value) in overrides.iter() {
            if let Some((min, max)) = limits.get(id.as_str()) {
                plugin.set_param(&ParamId::from(id.as_str()), (*value).clamp(*min, *max));
            }
        }
        plugin.prepare(sr, 256);

        let note = args.note.unwrap_or(60).min(127);
        let velocity = args.velocity.unwrap_or(100).min(127);
        let duration_ms = args.duration_ms.unwrap_or(900).clamp(50, 5_000);
        let release = plugin.get_param(&ParamId::from("release")).unwrap_or(0.3).max(0.0);
        let tail_ms = args
            .tail_ms
            .unwrap_or_else(|| (release * 1000.0 + 300.0).clamp(300.0, 4_000.0) as u32)
            .min(10_000);

        let mono = render_audition(&mut *plugin, sample_rate, note, velocity, duration_ms, tail_ms);
        let report = ai_daw_analysis::analyze(&mono, 1, sr).map_err(|e| e.to_string())?;
        let timbre = ai_daw_analysis::timbre(&mono, 1, sr).map_err(|e| e.to_string())?;
        let clipped = mono.iter().any(|s| s.abs() >= 0.999);

        // 把「生效后的完整参数」回给模型：满意时原样喂给 set_params 就能落地。
        let effective: std::collections::BTreeMap<String, f32> = specs
            .iter()
            .filter_map(|spec| {
                plugin
                    .get_param(&ParamId::from(spec.id.0.as_str()))
                    .map(|value| (spec.id.0.clone(), value))
            })
            .collect();

        json_s(&serde_json::json!({
            "plugin_id": plugin_id,
            "note": note,
            "velocity": velocity,
            "duration_ms": duration_ms,
            "tail_ms": tail_ms,
            "sample_rate": sample_rate,
            "clipped": clipped,
            "peak_dbfs": report.loudness.sample_peak_dbfs,
            "rms_dbfs": report.rms_dbfs,
            "integrated_lufs": report.loudness.integrated_lufs,
            "timbre": timbre,
            "params": effective,
            "tip": "params 是生效后的完整参数；满意后原样喂给 set_params(track, params) 即可落地（一个撤销步）。",
        }))
    }

    // ---- 片段与音符 ----

    pub(crate) fn add_clip(app: &AppHandle, actor: &Actor, args: ClipArgs) -> Result<String, String> {
        let (_, events) = commands::run_agent_as(app, actor.clone(), Command::AddClip { track: TrackId(args.track), start: args.start, length: args.length });
        let id = clip_added_id(&events);
        Ok(format!("已在轨道 {} 的第 {} 拍新增片段{}", args.track, args.start, id.map(|i| format!("，id={i}")).unwrap_or_default()))
    }

    pub(crate) fn remove_clip(app: &AppHandle, actor: &Actor, args: ClipRefArgs) -> Result<String, String> {
        commands::run_agent_as(app, actor.clone(), Command::RemoveClip { track: TrackId(args.track), clip: ClipId(args.clip) });
        Ok(format!("已删除轨道 {} 上的片段 {}", args.track, args.clip))
    }

    pub(crate) fn add_note(app: &AppHandle, actor: &Actor, args: NoteArgs) -> Result<String, String> {
        let before = note_count(app, args.track, args.clip)?;
        let project = app.state::<AppState>().engine.lock().unwrap().project().clone();
        let clip = clip_of(&project, args.track, args.clip)?;
        let mut note = Note {
            id: 0,
            pitch: args.pitch,
            start: args.start,
            duration: args.duration,
            velocity: args.velocity,
            volume: args.volume.unwrap_or(Note::MAX_VOLUME),
            offset: args.offset.unwrap_or(0.0),
            legato: args.legato.unwrap_or(false),
            expr: args.expr.as_ref().map(ExprArg::to_expr).unwrap_or(PitchExpression::NONE),
        };
        snap_note_starts(&project, &clip, std::slice::from_mut(&mut note), args.snap.as_deref())?;
        let snapped = args.snap.is_some() && (note.start - args.start).abs() > 1e-9;

        let (project, _) = commands::run_agent_as(
            app,
            actor.clone(),
            Command::AddNote { track: TrackId(args.track), clip: ClipId(args.clip), note },
        );
        let summary = clip_summary(&project, args.track, args.clip)?;
        let added = summary
            .get("note_count")
            .and_then(serde_json::Value::as_u64)
            .map(|count| count.saturating_sub(before as u64))
            .unwrap_or(1);
        Ok(format!(
            "已在轨道 {} 的片段 {} 加入 {} 个音符（音高 {} @ {} 拍{}）",
            args.track,
            args.clip,
            added,
            note.pitch,
            note.start,
            if snapped { "，已按 snap 吸附" } else { "" }
        ))
    }

    // ---- 批量音符 ----

    /// 一次写入的音符上限：编曲常态是几十到几百，5000 足够，同时挡住失控的载荷。
    const MAX_BATCH_NOTES: usize = 5000;

    /// 片段摘要：批量工具统一返回这个形状（音符多时只给前 32 个）。
    fn clip_summary(project: &Project, track: u64, clip: u64) -> Result<serde_json::Value, String> {
        let t = track_or_err(project, track)?;
        let c = t
            .clips
            .iter()
            .find(|c| c.id == ClipId(clip))
            .ok_or_else(|| missing_clip(t, clip))?;
        let preview = &c.notes[..c.notes.len().min(32)];
        Ok(serde_json::json!({
            "track": track,
            "clip": clip,
            "start": c.start,
            "length": c.length,
            "audio": c.audio.is_some(),
            "note_count": c.notes.len(),
            "notes": preview,
            "notes_truncated": c.notes.len() > preview.len(),
        }))
    }

    /// 当前片段里的音符数（用于算「这次加了 / 删了几个」）。
    fn note_count(app: &AppHandle, track: u64, clip: u64) -> Result<usize, String> {
        let project = app.state::<AppState>().engine.lock().unwrap().project().clone();
        let t = track_or_err(&project, track)?;
        let c = t
            .clips
            .iter()
            .find(|c| c.id == ClipId(clip))
            .ok_or_else(|| missing_clip(t, clip))?;
        Ok(c.notes.len())
    }

    /// 取片段（不存在时报错，信息里带现有轨道 / 片段，便于代理自我纠正）。
    fn clip_of(project: &Project, track: u64, clip: u64) -> Result<Clip, String> {
        let t = track_or_err(project, track)?;
        t.clips
            .iter()
            .find(|c| c.id == ClipId(clip))
            .cloned()
            .ok_or_else(|| missing_clip(t, clip))
    }

    /// 把 `snap` 字面量换算成步长（拍）；`None` = 不吸附。
    ///
    /// 口径与卷帘里的网格一致：「一拍」指拍号里那一拍（4/4 = 四分音符、6/8 = 八分音符），
    /// `1/12` / `1/24` 是三分（三连音），`bar` = 一小节。
    pub(crate) fn grid_step_beats(spec: &str, meter_beat: f64, bar_beats: f64) -> Result<Option<f64>, String> {
        Ok(match spec.trim().to_ascii_lowercase().as_str() {
            "" | "off" | "none" | "free" => None,
            "bar" | "measure" => Some(bar_beats),
            "1/4" | "quarter" => Some(meter_beat),
            "1/8" | "eighth" => Some(meter_beat / 2.0),
            "1/16" | "sixteenth" => Some(meter_beat / 4.0),
            "1/32" => Some(meter_beat / 8.0),
            "1/12" | "triplet" => Some(meter_beat / 3.0),
            "1/24" => Some(meter_beat / 6.0),
            other => {
                return Err(format!(
                    "snap 不认识「{other}」：可用 off / bar / 1/4 / 1/8 / 1/16 / 1/12 / 1/24 / 1/32"
                ))
            }
        })
    }

    /// 按 `snap` 吸附一批音符的**起点**（`duration` 原样，偏移也不动）。
    ///
    /// 步长按**每个音符自己所在位置**的拍号取——关键点可以在片段中途换拍号，与卷帘里的
    /// `stepAt(beat)` 是同一套口径。
    fn snap_note_starts(
        project: &Project,
        clip: &Clip,
        notes: &mut [Note],
        snap: Option<&str>,
    ) -> Result<(), String> {
        let Some(spec) = snap else { return Ok(()) };
        let map = TempoMap::from_transport(&project.transport);
        for note in notes.iter_mut() {
            let (num, den) = map.meter_at(clip.start + note.start);
            let meter_beat = 4.0 / f64::from(den.max(1));
            let bar_beats = f64::from(num.max(1)) * meter_beat;
            if let Some(step) = grid_step_beats(spec, meter_beat, bar_beats)? {
                note.start = (note.start / step).round() * step;
            }
        }
        Ok(())
    }

    /// 这次命令真正改了几个音符（从事件里数，和撤销记录一一对应）。
    fn changed_count(events: &[Event]) -> usize {
        events
            .iter()
            .map(|e| match e {
                Event::NotesChanged { entries, .. } => entries.len(),
                Event::NotesRemoved { indices, .. } => indices.len(),
                _ => 0,
            })
            .sum()
    }

    /// 批量工具的统一返回：改了几个 + 片段摘要。
    fn batch_result(
        app: &AppHandle,
        track: u64,
        clip: u64,
        events: &[Event],
        extra: serde_json::Value,
    ) -> Result<String, String> {
        let project = app.state::<AppState>().engine.lock().unwrap().project().clone();
        let mut summary = clip_summary(&project, track, clip)?;
        if let Some(map) = summary.as_object_mut() {
            map.insert("changed".into(), serde_json::json!(changed_count(events)));
            if let Some(extra) = extra.as_object() {
                for (k, v) in extra {
                    map.insert(k.clone(), v.clone());
                }
            }
        }
        json_s(&summary)
    }

    pub(crate) fn add_notes(app: &AppHandle, actor: &Actor, args: NotesArgs) -> Result<String, String> {
        if args.notes.is_empty() {
            return Err("notes 不能为空".into());
        }
        if args.notes.len() > MAX_BATCH_NOTES {
            return Err(format!("一次最多写 {MAX_BATCH_NOTES} 个音符（收到 {}）", args.notes.len()));
        }
        let before = note_count(app, args.track, args.clip)?;
        let mut notes: Vec<Note> = args.notes.iter().map(|n| n.to_note()).collect();
        // 放置吸附：只吸**起点**（时值由调用方按网格算好；偏移是演奏姿态，更不该被吸）。
        let project_before = app.state::<AppState>().engine.lock().unwrap().project().clone();
        let clip_before = clip_of(&project_before, args.track, args.clip)?;
        snap_note_starts(&project_before, &clip_before, &mut notes, args.snap.as_deref())?;

        let (project, _) = commands::run_agent_as(
            app,
            actor.clone(),
            Command::AddNotes { track: TrackId(args.track), clip: ClipId(args.clip), notes },
        );
        let t = track_or_err(&project, args.track)?;
        let clip = t
            .clips
            .iter()
            .find(|c| c.id == ClipId(args.clip))
            .ok_or_else(|| missing_clip(t, args.clip))?;
        let added: Vec<u64> = clip.notes[before.min(clip.notes.len())..].iter().map(|n| n.id).collect();
        let summary = clip_summary(&project, args.track, args.clip)?;
        let mut summary = summary;
        if let Some(map) = summary.as_object_mut() {
            map.insert("added".into(), serde_json::json!(added.len()));
            map.insert("added_ids".into(), serde_json::json!(added));
        }
        json_s(&summary)
    }

    pub(crate) fn replace_notes(
        app: &AppHandle,
        actor: &Actor,
        args: NotesArgs,
    ) -> Result<String, String> {
        if args.notes.len() > MAX_BATCH_NOTES {
            return Err(format!("一次最多写 {MAX_BATCH_NOTES} 个音符（收到 {}）", args.notes.len()));
        }
        let notes: Vec<Note> = args.notes.iter().map(|n| n.to_note()).collect();
        let (_, events) = commands::run_agent_as(
            app,
            actor.clone(),
            Command::ReplaceNotes { track: TrackId(args.track), clip: ClipId(args.clip), notes },
        );
        batch_result(app, args.track, args.clip, &events, serde_json::json!({}))
    }

    pub(crate) fn update_notes(
        app: &AppHandle,
        actor: &Actor,
        args: UpdateNotesArgs,
    ) -> Result<String, String> {
        if args.updates.is_empty() {
            return Err("updates 不能为空".into());
        }
        let updates: Vec<NotePatch> = args.updates.iter().map(|u| u.to_patch()).collect();
        let (_, events) = commands::run_agent_as(
            app,
            actor.clone(),
            Command::UpdateNotes { track: TrackId(args.track), clip: ClipId(args.clip), updates },
        );
        batch_result(app, args.track, args.clip, &events, serde_json::json!({}))
    }

    /// 给单个音符写 / 清除装饰音与奏法标签（不改播放，只给分析器提供意图）。
    pub(crate) fn set_note_tag(
        app: &AppHandle,
        actor: &Actor,
        args: SetNoteTagArgs,
    ) -> Result<String, String> {
        let project = snapshot(app);
        let clip = clip_of(&project, args.track, args.clip)?;
        if !clip.notes.iter().any(|n| n.id == args.id) {
            return Err(format!(
                "轨道 {} 的片段 {} 上没有音符 {}",
                args.track, args.clip, args.id
            ));
        }
        let tag = NoteTag { ornament: args.ornament.clone(), articulation: args.articulation.clone() }
            .normalized();
        let clear = tag.is_empty();
        commands::run_agent_as(
            app,
            actor.clone(),
            Command::SetNoteTag {
                track: TrackId(args.track),
                clip: ClipId(args.clip),
                id: args.id,
                tag: if clear { None } else { Some(tag.clone()) },
            },
        );
        Ok(if clear {
            format!("已清除音符 {}/{}:{} 的标签", args.track, args.clip, args.id)
        } else {
            let mut parts = Vec::new();
            if let Some(v) = &tag.ornament {
                parts.push(format!("装饰音 {v}"));
            }
            if let Some(v) = &tag.articulation {
                parts.push(format!("奏法 {v}"));
            }
            format!(
                "已标记音符 {}/{}:{}：{}",
                args.track,
                args.clip,
                args.id,
                parts.join("，")
            )
        })
    }

    pub(crate) fn remove_notes(
        app: &AppHandle,
        actor: &Actor,
        args: NoteIdsArgs,
    ) -> Result<String, String> {
        if args.ids.is_empty() {
            return Err("ids 不能为空".into());
        }
        let (_, events) = commands::run_agent_as(
            app,
            actor.clone(),
            Command::RemoveNotesById { track: TrackId(args.track), clip: ClipId(args.clip), ids: args.ids.clone() },
        );
        let removed = changed_count(&events);
        batch_result(
            app,
            args.track,
            args.clip,
            &events,
            serde_json::json!({ "removed": removed, "requested": args.ids.len() }),
        )
    }

    // ---- 变换 ----

    pub(crate) fn transpose_notes(
        app: &AppHandle,
        actor: &Actor,
        args: TransposeArgs,
    ) -> Result<String, String> {
        let (_, events) = commands::run_agent_as(
            app,
            actor.clone(),
            Command::TransposeNotes {
                track: TrackId(args.track),
                clip: ClipId(args.clip),
                semitones: args.semitones,
                range: args.range.map(|r| r.to_range()),
            },
        );
        batch_result(app, args.track, args.clip, &events, serde_json::json!({}))
    }

    pub(crate) fn set_note_velocity(
        app: &AppHandle,
        actor: &Actor,
        args: VelocityArgs,
    ) -> Result<String, String> {
        let shape = match (args.value, args.from, args.to, args.scale) {
            (Some(value), None, None, None) => VelocityShape::Fixed(value),
            (None, Some(from), Some(to), None) => VelocityShape::Ramp { from, to },
            (None, None, None, Some(scale)) => VelocityShape::Scale(scale),
            _ => {
                return Err(
                    "力度参数只能三选一：value（固定）/ from + to（渐变）/ scale（缩放）".into(),
                )
            }
        };
        let beat_filter = match (args.grid, args.beat) {
            (None, None) => None,
            (Some(grid), Some(beat))
                if grid.is_finite() && grid > 0.0 && beat.is_finite() =>
            {
                Some(BeatFilter { grid, beat })
            }
            _ => return Err("grid 与 beat 要同时给，且 grid 必须是正数（拍）".into()),
        };
        let (_, events) = commands::run_agent_as(
            app,
            actor.clone(),
            Command::SetNoteVelocity {
                track: TrackId(args.track),
                clip: ClipId(args.clip),
                shape,
                range: args.range.map(|r| r.to_range()),
                beat_filter,
            },
        );
        batch_result(app, args.track, args.clip, &events, serde_json::json!({}))
    }

    pub(crate) fn quantize_notes(
        app: &AppHandle,
        actor: &Actor,
        args: QuantizeArgs,
    ) -> Result<String, String> {
        if !(args.grid.is_finite() && args.grid > 0.0) {
            return Err(format!("grid 必须是正数（拍），收到 {}", args.grid));
        }
        let (_, events) = commands::run_agent_as(
            app,
            actor.clone(),
            Command::QuantizeNotes {
                track: TrackId(args.track),
                clip: ClipId(args.clip),
                grid: args.grid,
                strength: args.strength.unwrap_or(1.0),
                range: args.range.map(|r| r.to_range()),
                quantize_duration: args.duration.unwrap_or(false),
            },
        );
        batch_result(app, args.track, args.clip, &events, serde_json::json!({}))
    }

    pub(crate) fn apply_swing(
        app: &AppHandle,
        actor: &Actor,
        args: SwingArgs,
    ) -> Result<String, String> {
        let ratio = args.ratio.unwrap_or(0.66);
        if !(ratio.is_finite() && (0.2..=0.8).contains(&ratio)) {
            return Err(format!(
                "ratio 应在 0.2..=0.8（< 0.5 = 抢拍，0.66 ≈ 8 分 swing），收到 {ratio}"
            ));
        }
        let (_, events) = commands::run_agent_as(
            app,
            actor.clone(),
            Command::ApplySwing {
                track: TrackId(args.track),
                clip: ClipId(args.clip),
                subdivision: args.subdivision.unwrap_or(0.5),
                ratio,
                strength: args.strength.unwrap_or(1.0),
                range: args.range.map(|r| r.to_range()),
            },
        );
        batch_result(app, args.track, args.clip, &events, serde_json::json!({}))
    }

    /// 扫弦：把**起点相同**的音符按音高依次错开（`step_ms = 0` 清掉扫弦）。
    pub(crate) fn strum_notes(
        app: &AppHandle,
        actor: &Actor,
        args: StrumArgs,
    ) -> Result<String, String> {
        if !(args.step_ms.is_finite() && args.step_ms >= 0.0) {
            return Err(format!("step_ms 必须是非负数（毫秒），收到 {}", args.step_ms));
        }
        let direction = match args.direction.as_deref().map(str::trim) {
            None | Some("") => StrumDirection::Down,
            Some(value) => StrumDirection::parse(value)
                .ok_or_else(|| format!("direction 只认 \"down\" / \"up\"，收到 {value}"))?,
        };
        let (_, events) = commands::run_agent_as(
            app,
            actor.clone(),
            Command::StrumNotes {
                track: TrackId(args.track),
                clip: ClipId(args.clip),
                step_ms: args.step_ms,
                direction,
                range: args.range.map(|r| r.to_range()),
            },
        );
        batch_result(app, args.track, args.clip, &events, serde_json::json!({}))
    }

    pub(crate) fn humanize_notes(
        app: &AppHandle,
        actor: &Actor,
        args: HumanizeArgs,
    ) -> Result<String, String> {
        let (_, events) = commands::run_agent_as(
            app,
            actor.clone(),
            Command::HumanizeNotes {
                track: TrackId(args.track),
                clip: ClipId(args.clip),
                timing: args.timing.unwrap_or(0.02),
                velocity: args.velocity.unwrap_or(8.0),
                range: args.range.map(|r| r.to_range()),
                seed: args.seed,
                min_duration: args.min_duration.unwrap_or(0.0),
                group_by_start: args.group_by_start.unwrap_or(false),
            },
        );
        batch_result(app, args.track, args.clip, &events, serde_json::json!({}))
    }

    pub(crate) fn fit_notes_to_range(
        app: &AppHandle,
        actor: &Actor,
        args: FitArgs,
    ) -> Result<String, String> {
        let mode = match args.mode.as_deref() {
            None => FitMode::Fold,
            Some(text) => FitMode::parse(text)
                .ok_or_else(|| format!("未知 mode：{text}（可选 fold / report）"))?,
        };
        let project = app.state::<AppState>().engine.lock().unwrap().project().clone();
        let (min, max) = match (args.min_pitch, args.max_pitch) {
            (Some(a), Some(b)) => (a, b),
            _ => {
                let meta = project
                    .track(TrackId(args.track))
                    .and_then(|t| t.pitch_range())
                    .ok_or_else(|| {
                        "没有给 min_pitch / max_pitch，轨道也没有 pitch_range 元数据（可用 set_track_meta 写一份）"
                            .to_string()
                    })?;
                (args.min_pitch.unwrap_or(meta.min), args.max_pitch.unwrap_or(meta.max))
            }
        };
        let t = track_or_err(&project, args.track)?;
        let clip = t
            .clips
            .iter()
            .find(|c| c.id == ClipId(args.clip))
            .ok_or_else(|| missing_clip(t, args.clip))?;
        let range = args.range.map(|r| r.to_range());
        // 先按「只报告」算一遍越界数量（fold 之后再算就算不出来了）。
        let preview = ai_daw_engine::compose::fit_to_range(clip, min, max, FitMode::Report, range);
        let out_of_range = preview.out_of_range;
        if mode == FitMode::Report {
            return json_s(&serde_json::json!({
                "mode": "report",
                "out_of_range": out_of_range,
                "min_pitch": min,
                "max_pitch": max,
                "note_count": clip.notes.len(),
            }));
        }
        let (_, events) = commands::run_agent_as(
            app,
            actor.clone(),
            Command::FitNotesToRange {
                track: TrackId(args.track),
                clip: ClipId(args.clip),
                min_pitch: min,
                max_pitch: max,
                mode,
                range,
            },
        );
        batch_result(
            app,
            args.track,
            args.clip,
            &events,
            serde_json::json!({ "mode": "fold", "out_of_range": out_of_range, "min_pitch": min, "max_pitch": max }),
        )
    }

    // ---- 片段编辑 ----

    pub(crate) fn trim_clip(
        app: &AppHandle,
        actor: &Actor,
        args: TrimClipArgs,
    ) -> Result<String, String> {
        let project =
            commands::trim_clip_as(app, actor.clone(), args.track, args.clip, args.start, args.length)?;
        json_s(&clip_summary(&project, args.track, args.clip)?)
    }

    pub(crate) fn set_clip_rate(
        app: &AppHandle,
        actor: &Actor,
        args: ClipRateArgs,
    ) -> Result<String, String> {
        let project = commands::set_clip_rate_as(
            app,
            actor.clone(),
            args.track,
            args.clip,
            args.rate,
            args.pitch_semitones,
        );
        let summary = clip_summary(&project, args.track, args.clip)?;
        Ok(format!(
            "片段速度已设为 {}×、升降调 {} 半音：{}",
            args.rate,
            args.pitch_semitones,
            serde_json::to_string(&summary).unwrap_or_default()
        ))
    }

    pub(crate) fn move_clip(app: &AppHandle, actor: &Actor, args: MoveClipArgs) -> Result<String, String> {
        let (_, events) = commands::run_agent_as(
            app,
            actor.clone(),
            Command::MoveClip { track: TrackId(args.track), clip: ClipId(args.clip), start: args.start },
        );
        batch_result(app, args.track, args.clip, &events, serde_json::json!({}))
    }

    pub(crate) fn resize_clip(
        app: &AppHandle,
        actor: &Actor,
        args: ResizeClipArgs,
    ) -> Result<String, String> {
        if !(args.length.is_finite() && args.length > 0.0) {
            return Err(format!("length 必须是正数（拍），收到 {}", args.length));
        }
        let (_, events) = commands::run_agent_as(
            app,
            actor.clone(),
            Command::ResizeClip { track: TrackId(args.track), clip: ClipId(args.clip), length: args.length },
        );
        batch_result(app, args.track, args.clip, &events, serde_json::json!({}))
    }

    pub(crate) fn split_clip(app: &AppHandle, actor: &Actor, args: SplitArgs) -> Result<String, String> {
        let (project, events) = commands::run_agent_as(
            app,
            actor.clone(),
            Command::SplitClip { track: TrackId(args.track), clip: ClipId(args.clip), beat: args.beat },
        );
        let new_clip = events.iter().find_map(|e| match e {
            Event::ClipSplit { new_clip, .. } => Some(new_clip.0),
            _ => None,
        });
        let Some(new_clip) = new_clip else {
            return Err(format!(
                "切分点 {} 不在片段 {} 内部（必须严格落在片段起止之间，音频片段还需要素材窗口长度）",
                args.beat, args.clip
            ));
        };
        let left = clip_summary(&project, args.track, args.clip)?;
        let right = clip_summary(&project, args.track, new_clip)?;
        json_s(&serde_json::json!({ "left": left, "right": right, "at_beat": args.beat }))
    }

    pub(crate) fn duplicate_clip(
        app: &AppHandle,
        actor: &Actor,
        args: DuplicateClipArgs,
    ) -> Result<String, String> {
        let (project, events) = commands::run_agent_as(
            app,
            actor.clone(),
            Command::DuplicateClip {
                track: TrackId(args.track),
                clip: ClipId(args.clip),
                to_track: args.to_track.map(TrackId),
                start: args.start,
            },
        );
        let new_clip = events.iter().find_map(|e| match e {
            Event::ClipAdded { clip, .. } | Event::AudioClipAdded { clip, .. } => Some(clip.0),
            _ => None,
        });
        let Some(new_clip) = new_clip else {
            return Err(format!(
                "复制失败：片段 {} 或目标轨道不存在",
                args.clip
            ));
        };
        json_s(&clip_summary(&project, args.to_track.unwrap_or(args.track), new_clip)?)
    }

    pub(crate) fn merge_clips(
        app: &AppHandle,
        actor: &Actor,
        args: MergeClipsArgs,
    ) -> Result<String, String> {
        if args.clips.len() < 2 {
            return Err("clips 至少要有两个片段".into());
        }
        let project = app.state::<AppState>().engine.lock().unwrap().project().clone();
        let track = track_or_err(&project, args.track)?;
        for id in &args.clips {
            let Some(c) = track.clips.iter().find(|c| c.id == ClipId(*id)) else {
                return Err(missing_clip(track, *id));
            };
            if c.is_audio() {
                return Err(format!(
                    "片段 {id} 是音频片段：音频各自的素材窗口无法用「挪音符」表达，v1 不支持合并"
                ));
            }
        }
        let (project, events) = commands::run_agent_as(
            app,
            actor.clone(),
            Command::MergeClips {
                track: TrackId(args.track),
                clips: args.clips.iter().map(|c| ClipId(*c)).collect(),
            },
        );
        let target = events.iter().find_map(|e| match e {
            Event::NotesReplaced { clip, .. } => Some(clip.0),
            _ => None,
        });
        let Some(target) = target else {
            return Err("合并没有产生任何变更（片段可能已被删除）".into());
        };
        json_s(&clip_summary(&project, args.track, target)?)
    }

    // ---- 统计与和声（只读） ----

    fn snapshot(app: &AppHandle) -> Project {
        app.state::<AppState>().engine.lock().unwrap().project().clone()
    }

    pub(crate) fn track_note_stats(app: &AppHandle, args: TrackStatsArgs) -> Result<String, String> {
        let project = snapshot(app);
        let track = track_or_err(&project, args.track)?;
        let stats = ai_daw_engine::compose::note_stats(
            track,
            args.clip.map(ClipId),
            project.transport.bar_beats(),
        );
        json_s(&stats)
    }

    pub(crate) fn analyze_track_overlap(app: &AppHandle, args: OverlapArgs) -> Result<String, String> {
        if args.tracks.len() < 2 {
            return Err("tracks 至少要有两条轨道".into());
        }
        let project = snapshot(app);
        let picked: Vec<&ai_daw_engine::model::Track> = args
            .tracks
            .iter()
            .map(|id| track_or_err(&project, *id))
            .collect::<Result<_, _>>()?;
        let mut pairs = Vec::new();
        for (i, a) in picked.iter().enumerate() {
            for b in picked.iter().skip(i + 1) {
                pairs.push(ai_daw_engine::compose::track_overlap(a, b, args.clip.map(ClipId)));
            }
        }
        let summary: Vec<String> = pairs
            .iter()
            .filter(|p| p.conflict && !p.suggestion.is_empty())
            .map(|p| p.suggestion.clone())
            .collect();
        json_s(&ai_daw_engine::report::OverlapReport {
            tracks: args.tracks.clone(),
            pairs,
            summary,
        })
    }

    pub(crate) fn analyze_harmony(app: &AppHandle, args: HarmonyArgs) -> Result<String, String> {
        let project = snapshot(app);
        let bar_beats = project.transport.bar_beats();
        let bars: Vec<ai_daw_engine::model::HarmonyBar> = match &args.bars {
            Some(bars) if !bars.is_empty() => bars.iter().map(|b| b.to_bar(bar_beats)).collect(),
            _ if !project.harmony.is_empty() => project.harmony.clone(),
            _ => {
                return Err("没有和弦信息：传 bars（[{start, length, symbol}]），或先用 set_project_harmony 存一份".into())
            }
        };
        let picked: Vec<&ai_daw_engine::model::Track> = match &args.tracks {
            Some(ids) => ids
                .iter()
                .map(|id| track_or_err(&project, *id))
                .collect::<Result<_, _>>()?,
            None => project
                .tracks
                .iter()
                .filter(|t| t.clips.iter().any(|c| !c.notes.is_empty()))
                .collect(),
        };
        if picked.is_empty() {
            return Err("没有可检查的音符：指定的轨道上没有任何音符".into());
        }
        let policy = ai_daw_engine::report::HarmonyPolicy {
            allow_extensions: args.allow_extensions.unwrap_or(true),
            allow_approach_notes: args.allow_approach_notes.unwrap_or(true),
            allow_non_chord_tone_duration: args.allow_non_chord_tone_duration.unwrap_or(0.5),
        };
        let scale = match (&args.key, &args.scale) {
            (Some(key), mode) => Some(ai_daw_engine::harmony::parse_scale(
                key,
                mode.as_deref().unwrap_or("major"),
            )?),
            (None, Some(_)) => {
                return Err("给了 scale 就必须同时给 key（例如 key=\"F\", scale=\"major\"）".into())
            }
            (None, None) => None,
        };
        let mut ornament_ids: Vec<(u64, u64, u64)> = args
            .ornaments
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|o| (o.track, o.clip, o.id))
            .collect();
        // 工程里持久化的装饰音标签也自动算进来。
        for (key, tag) in &project.note_tags {
            if tag.ornament.as_deref().map_or(false, |v| !v.trim().is_empty()) {
                if let Some(ids) = ai_daw_engine::model::parse_note_tag_key(key) {
                    ornament_ids.push(ids);
                }
            }
        }
        ornament_ids.sort_unstable();
        ornament_ids.dedup();
        let report = ai_daw_engine::harmony::analyze_with_options(
            &bars,
            &picked,
            policy,
            bar_beats,
            args.time_tolerance.unwrap_or(1.0 / 16.0),
            scale.as_deref(),
            if ornament_ids.is_empty() { None } else { Some(ornament_ids.as_slice()) },
        )?;
        json_s(&report)
    }

    // ---- 编曲元数据 ----

    pub(crate) fn set_project_harmony(
        app: &AppHandle,
        actor: &Actor,
        args: SetHarmonyArgs,
    ) -> Result<String, String> {
        let bar_beats = snapshot(app).transport.bar_beats();
        let bars: Vec<ai_daw_engine::model::HarmonyBar> =
            args.bars.iter().map(|b| b.to_bar(bar_beats)).collect();
        // 先解析一遍符号：写进工程的和弦表必须是**能用**的，别等分析时才炸。
        for bar in &bars {
            ai_daw_engine::harmony::parse_chord(&bar.symbol)
                .map_err(|e| format!("和弦「{}」无法识别：{e}", bar.symbol))?;
        }
        let count = bars.len();
        commands::run_agent_as(app, actor.clone(), Command::SetProjectHarmony { bars });
        Ok(format!("工程和弦表已更新：{count} 个小节"))
    }

    pub(crate) fn set_project_sections(
        app: &AppHandle,
        actor: &Actor,
        args: SetSectionsArgs,
    ) -> Result<String, String> {
        let sections: Vec<ai_daw_engine::model::Section> = args
            .sections
            .iter()
            .map(|s| ai_daw_engine::model::Section {
                name: s.name.clone(),
                start: s.start,
                end: s.end,
                role: s.role.clone(),
            })
            .collect();
        let count = sections.len();
        commands::run_agent_as(app, actor.clone(), Command::SetProjectSections { sections });
        Ok(format!("工程段落表已更新：{count} 段"))
    }

    pub(crate) fn set_track_meta(
        app: &AppHandle,
        actor: &Actor,
        args: TrackMetaArgs,
    ) -> Result<String, String> {
        let pitch_range = match (args.min_pitch, args.max_pitch) {
            (Some(min), Some(max)) => Some(ai_daw_engine::model::PitchRange { min, max }),
            (None, None) => None,
            _ => return Err("音域要同时给 min_pitch 和 max_pitch（或都不给）".into()),
        };
        commands::run_agent_as(
            app,
            actor.clone(),
            Command::SetTrackMeta {
                track: TrackId(args.track),
                role: args.role.clone(),
                pitch_range,
                pitch_bend_range: args.pitch_bend_range,
            },
        );
        let mut parts = Vec::new();
        if let Some(role) = &args.role {
            parts.push(format!("分工 {role}"));
        }
        if let Some(range) = pitch_range {
            parts.push(format!("音域 {}..={}", range.min, range.max));
        }
        if let Some(range) = args.pitch_bend_range {
            parts.push(format!("弯音范围 ±{range} 半音"));
        }
        Ok(if parts.is_empty() {
            format!("轨道 {} 的编曲元数据已清除", args.track)
        } else {
            format!("轨道 {} 的编曲元数据已更新：{}", args.track, parts.join("，"))
        })
    }

    // ---- 混音 ----

    pub(crate) fn set_gain(app: &AppHandle, actor: &Actor, args: GainArgs) -> Result<String, String> {
        commands::run_agent_as(app, actor.clone(), Command::SetGain { track: TrackId(args.track), gain_db: args.gain_db });
        Ok(format!("轨道 {} 增益已设为 {} dB", args.track, args.gain_db))
    }

    pub(crate) fn set_pan(app: &AppHandle, actor: &Actor, args: PanArgs) -> Result<String, String> {
        commands::run_agent_as(app, actor.clone(), Command::SetPan { track: TrackId(args.track), pan: args.pan });
        Ok(format!("轨道 {} 声像已设为 {}", args.track, args.pan))
    }

    pub(crate) fn set_mute(app: &AppHandle, actor: &Actor, args: ToggleArgs) -> Result<String, String> {
        commands::run_agent_as(app, actor.clone(), Command::SetMute { track: TrackId(args.track), mute: args.on });
        Ok(format!("轨道 {} 静音已{}", args.track, if args.on { "开启" } else { "关闭" }))
    }

    pub(crate) fn set_solo(app: &AppHandle, actor: &Actor, args: ToggleArgs) -> Result<String, String> {
        commands::run_agent_as(app, actor.clone(), Command::SetSolo { track: TrackId(args.track), solo: args.on });
        Ok(format!("轨道 {} 独奏已{}", args.track, if args.on { "开启" } else { "关闭" }))
    }

    pub(crate) fn set_master_gain(app: &AppHandle, actor: &Actor, args: MasterGainArgs) -> Result<String, String> {
        commands::run_agent_as(app, actor.clone(), Command::SetMasterGain(args.gain_db));
        Ok(format!("总输出增益已设为 {} dB", args.gain_db))
    }

    pub(crate) fn set_sends(app: &AppHandle, actor: &Actor, args: SetSendsArgs) -> Result<String, String> {
        let sends = args
            .sends
            .into_iter()
            .map(|s| ai_daw_engine::model::Send { target: TrackId(s.target), gain_db: s.gain_db })
            .collect();
        commands::run_agent_as(app, actor.clone(), Command::SetSends { track: TrackId(args.track), sends });
        Ok(format!("轨道 {} 的发送已更新", args.track))
    }

    pub(crate) fn set_automation(app: &AppHandle, actor: &Actor, args: SetAutomationArgs) -> Result<String, String> {
        let points = args
            .points
            .into_iter()
            .map(|p| ai_daw_engine::model::AutomationPoint { beat: p.beat, value: p.value })
            .collect();
        commands::run_agent_as(
            app,
            actor.clone(),
            Command::SetAutomation { track: TrackId(args.track), param: args.param.clone(), points },
        );
        Ok(format!("轨道 {} 的自动化（{}）已更新", args.track, args.param))
    }

    // ---- 技能文档 ----

    pub(crate) fn aidaw_help(args: HelpArgs) -> Result<String, String> {
        let topic = args.topic.unwrap_or_default().trim().to_ascii_lowercase();
        match SKILL_DOCS.iter().find(|(key, _)| *key == topic) {
            Some((_, text)) => Ok((*text).to_string()),
            None => {
                let known: Vec<&str> =
                    SKILL_DOCS.iter().map(|(k, _)| *k).filter(|k| !k.is_empty()).collect();
                Ok(format!(
                    "未知能力域「{topic}」。可选：{}。下面是索引：\n\n{}",
                    known.join(" / "),
                    SKILL_DOCS[0].1
                ))
            }
        }
    }

    /// 网关：把「操作名 + 参数」原样转给对应工具。
    ///
    /// 固定两条工具（`aidaw_help` + 本工具）就能覆盖全部能力，适合不想一次塞 80 多条
    /// function 定义进上下文的客户端；效果与直接调用那个工具完全一致（同样的署名与撤销步）。
    pub(crate) fn aidaw_call(
        app: &AppHandle,
        actor: &Actor,
        args: CallArgs,
    ) -> Result<String, String> {
        if args.op == "aidaw_call" {
            return Err("aidaw_call 不能调用自己".into());
        }
        dispatch(app, actor, &args.op, args.args)
    }

    // ---- 历史 ----

    pub(crate) fn undo(app: &AppHandle) -> Result<String, String> {
        let state = app.state::<AppState>();
        json_s(&commands::undo_now(&state))
    }

    pub(crate) fn redo(app: &AppHandle) -> Result<String, String> {
        let state = app.state::<AppState>();
        json_s(&commands::redo_now(&state))
    }

    // ---- 工程文件 ----

    pub(crate) fn get_project_path(app: &AppHandle) -> Result<String, String> {
        let state = app.state::<AppState>();
        let dirty = state.engine.lock().unwrap().is_dirty();
        json_s(&serde_json::json!({ "path": state.session_path(), "dirty": dirty }))
    }

    pub(crate) fn save_project(
        app: &AppHandle,
        args: SaveProjectArgs,
    ) -> Result<String, String> {
        let state = app.state::<AppState>();
        let path = args
            .path
            .or_else(|| state.session_path())
            .ok_or_else(|| "这个工程还没保存过：请给出 path".to_string())?;
        let written = commands::save_project_impl(app, &path)?;
        Ok(format!("工程已保存到 {written}"))
    }

    pub(crate) fn open_project(
        app: &AppHandle,
        args: OpenProjectArgs,
    ) -> Result<String, String> {
        let project = commands::open_project_impl(app, &args.path)?;
        Ok(format!(
            "已打开「{}」：{} 条轨道、{:.1} BPM。原工程已进历史（可用 list_project_history 找回）。",
            project.name,
            project.tracks.len(),
            project.transport.tempo
        ))
    }

    pub(crate) fn new_project(
        app: &AppHandle,
        actor: &Actor,
        args: NewProjectArgs,
    ) -> Result<String, String> {
        let project = commands::new_project_as(app, actor.clone(), args.track_names, args.tempo);
        Ok(format!(
            "已新建工程「{}」：{} 条轨道、{:.1} BPM。原工程已进历史（可用 list_project_history 找回）。",
            project.name,
            project.tracks.len(),
            project.transport.tempo
        ))
    }

    pub(crate) fn rename_project(
        app: &AppHandle,
        actor: &Actor,
        args: RenameProjectArgs,
    ) -> Result<String, String> {
        let (project, _) = commands::run_agent_as(
            app,
            actor.clone(),
            Command::RenameProject { name: args.name },
        );
        Ok(format!("工程已重命名为「{}」", project.name))
    }

    pub(crate) fn list_project_history(app: &AppHandle) -> Result<String, String> {
        let entries = crate::history_dir(app)
            .map(|dir| crate::history::load_index(&dir))
            .unwrap_or_default();
        json_s(&entries)
    }

    pub(crate) fn restore_project_history(
        app: &AppHandle,
        args: HistoryIdArgs,
    ) -> Result<String, String> {
        let project = commands::restore_project_history_impl(app, &args.id)?;
        Ok(format!(
            "已恢复到历史工程「{}」：{} 条轨道、{:.1} BPM",
            project.name,
            project.tracks.len(),
            project.transport.tempo
        ))
    }

    // ---- 渲染与分析（重活；调用方负责放到阻塞线程池） ----

    pub(crate) fn analyze_project(app: &AppHandle, args: AnalyzeArgs) -> Result<String, String> {
        let sample_rate = check_sample_rate_s(args.sample_rate.unwrap_or(FALLBACK_SAMPLE_RATE))?;
        let mut inputs = commands::render_inputs(&app.state::<AppState>());
        let range = resolve_beat_range(&inputs.project, args.start_beat, args.end_beat)?;
        if let Some(tracks) = &args.tracks {
            mute_other_tracks(&mut inputs.project, tracks)?;
        }
        let transport = inputs.project.transport.clone();
        // 插件加载失败的记录由注册表工厂写进这里（离线渲染走隔离，崩了也不会杀主进程）。
        let warnings = std::sync::Arc::clone(&inputs.warnings);
        // 只分析工程正文，不加尾音。
        let buffer = commands::render_offline(inputs, sample_rate, 0.0);
        let buffer = match range {
            Some((start, end)) => {
                slice_render_by_beats(&buffer, &transport, sample_rate, start, end, 0.0)
            }
            None => buffer,
        };
        if buffer.frames() == 0 {
            return Err("渲染范围为空（检查 start_beat / end_beat）".into());
        }
        let report = ai_daw_analysis::analyze(buffer.data(), buffer.channels(), sample_rate as f64)
            .map_err(|e| e.to_string())?;
        // 有插件加载失败时，分析结果只是「剩下的能响的部分」——必须让代理知道哪些插件缺了，
        // 否则它会把一条静音轨当成「混音太轻」。
        let mut value = serde_json::to_value(&report).map_err(|e| format!("序列化失败：{e}"))?;
        if let Some(object) = value.as_object_mut() {
            if let Some((start, end)) = range {
                object.insert(
                    "range".into(),
                    serde_json::json!({ "start_beat": start, "end_beat": end }),
                );
            }
            if let Some(tracks) = &args.tracks {
                object.insert("tracks".into(), serde_json::json!(tracks));
            }
            let warnings: Vec<String> = warnings.lock().unwrap().clone();
            if !warnings.is_empty() {
                object.insert("warnings".into(), serde_json::Value::from(warnings));
            }
        }
        serde_json::to_string_pretty(&value).map_err(|e| format!("序列化失败：{e}"))
    }

    pub(crate) fn export_wav(app: &AppHandle, args: ExportArgs) -> Result<String, String> {
        let sample_rate = check_sample_rate_s(args.sample_rate.unwrap_or(FALLBACK_SAMPLE_RATE))?;
        let path = args.path.clone();
        let mut inputs = commands::render_inputs(&app.state::<AppState>());
        let range = resolve_beat_range(&inputs.project, args.start_beat, args.end_beat)?;
        if let Some(tracks) = &args.tracks {
            mute_other_tracks(&mut inputs.project, tracks)?;
        }
        let transport = inputs.project.transport.clone();
        let warnings = std::sync::Arc::clone(&inputs.warnings);
        let buffer = commands::render_offline(inputs, sample_rate, 0.5);
        let buffer = match range {
            Some((start, end)) => {
                slice_render_by_beats(&buffer, &transport, sample_rate, start, end, 0.5)
            }
            None => buffer,
        };
        if buffer.frames() == 0 {
            return Err("导出范围为空（检查 start_beat / end_beat）".into());
        }
        ai_daw_engine::wav::write_wav(&path, &buffer, sample_rate).map_err(|e| e.to_string())?;
        let range_note =
            range.map(|(start, end)| format!("（{start}..{end} 拍）")).unwrap_or_default();
        let warnings: Vec<String> = warnings.lock().unwrap().clone();
        if warnings.is_empty() {
            Ok(format!("已导出到 {path}{range_note}"))
        } else {
            Ok(format!(
                "已导出到 {path}{range_note}\n\n警告（这些插件加载失败，对应轨道按静音导出）：\n- {}",
                warnings.join("\n- ")
            ))
        }
    }

    pub(crate) fn master_project(app: &AppHandle, args: MasterExportArgs) -> Result<String, String> {
        let path = args.path.clone();
        let options = ai_daw_mastering::MasterOptions {
            target_lufs: args.target_lufs.unwrap_or(ai_daw_mastering::DEFAULT_TARGET_LUFS),
            ceiling_dbtp: args.ceiling_dbtp.unwrap_or(ai_daw_mastering::DEFAULT_CEILING_DBTP),
            stereo_width: args.stereo_width.unwrap_or(ai_daw_mastering::DEFAULT_WIDTH),
        };
        let sample_rate = FALLBACK_SAMPLE_RATE;
        let inputs = commands::render_inputs(&app.state::<AppState>());
        let buffer = commands::render_offline(inputs, sample_rate, 0.5);
        let channels = buffer.channels();
        let (processed, report) =
            ai_daw_mastering::master(buffer.data(), channels, sample_rate as f64, &options)
                .map_err(|e| e.to_string())?;
        let mut out = ai_daw_engine::audio::AudioBuffer::silence(channels, processed.len() / channels);
        out.data_mut().copy_from_slice(&processed);
        ai_daw_engine::wav::write_wav(&path, &out, sample_rate).map_err(|e| e.to_string())?;
        json_s(&MasterExport { path, report })
    }

    pub(crate) fn separate_stems(app: &AppHandle, args: SeparateArgs) -> Result<String, String> {
        let count = crate::ai::separate_track(app, args.track, args.model)?;
        Ok(format!("已生成 {count} 条分轨（HTDemucs）"))
    }

    pub(crate) fn detect_tempo(app: &AppHandle, args: DetectTempoArgs) -> Result<String, String> {
        crate::ai::detect_tempo_track(app, args.track)
    }

    pub(crate) fn transcribe_audio(app: &AppHandle, args: TrackArgs) -> Result<String, String> {
        crate::ai::transcribe_track(app, args.track)
    }

    pub(crate) fn enhance_audio(app: &AppHandle, args: EnhanceArgs) -> Result<String, String> {
        let dsp = match args.mode.as_deref().unwrap_or("voice") {
            "voice" => false,
            "music" | "dsp" => true,
            other => {
                return Err(format!("mode 只能是 voice（人声）或 music（乐器底噪），收到：{other}"))
            }
        };
        crate::ai::enhance_track(app, args.track, dsp)
    }

    pub(crate) fn import_audio(app: &AppHandle, actor: &Actor, args: ImportAudioArgs) -> Result<String, String> {
        let state = app.state::<AppState>();
        let audio = state.assets.load(&args.path).map_err(|e| e.to_string())?;
        let seconds = audio.frames() as f64 / audio.sample_rate;
        if !(seconds.is_finite() && seconds > 0.0) {
            return Err(format!("音频素材没有可用长度：{}", args.path));
        }
        let start = Clip::normalize_start(args.start);
        // 让片段的拍长正好等于素材的自然时长（比值 ≈ 1，导入时不变调）。
        let length = {
            let engine = state.engine.lock().unwrap();
            commands::natural_length_beats(engine.project(), start, seconds)
        };
        let (_, events) = commands::run_agent_as(
            app,
            actor.clone(),
            Command::AddAudioClip {
                track: TrackId(args.track),
                start,
                length,
                // MCP 只给了路径：窗口长度未知（到结尾）。切分这类片段会被引擎拒绝。
                source: AudioSource { path: args.path.clone(), offset_frames: 0, frames: None, rate: 1.0, pitch_semitones: 0.0 },
            },
        );
        let id = clip_added_id(&events);
        Ok(format!(
            "已把 {} 导入到轨道 {} 的第 {} 拍{}",
            args.path,
            args.track,
            start,
            id.map(|i| format!("，片段 id={i}")).unwrap_or_default()
        ))
    }

    pub(crate) fn audio_waveform(app: &AppHandle, args: WaveformArgs) -> Result<String, String> {
        let values = app
            .state::<AppState>()
            .assets
            .waveform(&args.path, args.buckets)
            .map_err(|e| e.to_string())?;
        json_s(&values)
    }

    // ---- 录音 ----

    pub(crate) fn list_input_devices() -> Result<String, String> {
        json_s(&ai_daw_engine::recorder::list_input_devices())
    }

    pub(crate) fn set_input_device(app: &AppHandle, args: InputDeviceArgs) -> Result<String, String> {
        let state = app.state::<AppState>();
        match args.device {
            Some(name) => {
                state.audio.arm_input(Some(name.clone()), false)?;
                Ok(format!("输入设备已选为 {name}（尚未占用；开始录音时才打开）"))
            }
            None => {
                state.audio.disarm_input();
                Ok("输入流已关闭（麦克风已释放）".into())
            }
        }
    }

    pub(crate) fn record_start(app: &AppHandle, args: RecordStartArgs) -> Result<String, String> {
        if !args.start.is_finite() || args.start < 0.0 {
            return Err(format!("录音起始拍不合法：{}", args.start));
        }
        let state = app.state::<AppState>();
        let exists = {
            let engine = state.engine.lock().unwrap();
            engine.project().tracks.iter().any(|t| t.id == TrackId(args.track))
        };
        if !exists {
            return Err(format!("轨道 {} 不存在", args.track));
        }
        commands::record_start_impl(&state, args.track, args.start, args.device)?;
        Ok(format!(
            "已开始录音：轨道 {}，从第 {} 拍起。录完后调用 record_stop 落成音频片段。",
            args.track, args.start
        ))
    }

    pub(crate) fn record_stop(app: &AppHandle, actor: &Actor) -> Result<String, String> {
        match commands::record_stop_as(app, actor.clone())? {
            Some(_) => Ok("录音已结束，片段已挂到轨道上（一个撤销步）".into()),
            None => Ok("没有正在进行的录音，或没有录到内容".into()),
        }
    }

    pub(crate) fn apply_preset(app: &AppHandle, actor: &Actor, args: ApplyPresetArgs) -> Result<String, String> {
        let track = TrackId(args.track);
        let (project, _events) =
            commands::apply_preset_agent_as(app, actor.clone(), track, &args.params, &args.effects);
        let chain = project
            .tracks
            .iter()
            .find(|t| t.id == track)
            .map(|t| t.effects.iter().map(|e| e.plugin_id.clone()).collect::<Vec<_>>().join(" → "))
            .unwrap_or_default();
        Ok(format!(
            "已应用到轨道 {}：{} 个参数，效果链 [{}]",
            args.track,
            args.params.len(),
            if chain.is_empty() { "空".into() } else { chain }
        ))
    }

    // ---- 插件目录 / 用户预设 ----

    /// 重新扫描本机插件。扫描本身较慢（会 dlopen 每个插件），调用方应放到阻塞线程池。
    pub(crate) fn rescan_plugins(app: &AppHandle) -> Result<String, String> {
        let catalog = crate::manager::rescan_with_managed(app);
        let instruments = catalog.iter().filter(|p| p.kind == PluginKind::Instrument).count();
        Ok(format!(
            "插件扫描完成：共 {} 个（乐器 {} / 效果器 {}）",
            catalog.len(),
            instruments,
            catalog.len() - instruments
        ))
    }

    pub(crate) fn list_presets(app: &AppHandle) -> Result<String, String> {
        let state = app.state::<AppState>();
        let presets = state.presets.lock().unwrap().all();
        json_s(&presets)
    }

    pub(crate) fn save_preset(app: &AppHandle, args: SavePresetArgs) -> Result<String, String> {
        let state = app.state::<AppState>();
        let preset = state
            .presets
            .lock()
            .unwrap()
            .create(&args.name, &args.plugin_id, args.params)?;
        json_s(&preset)
    }

    pub(crate) fn update_preset(app: &AppHandle, args: UpdatePresetArgs) -> Result<String, String> {
        let state = app.state::<AppState>();
        let preset = state
            .presets
            .lock()
            .unwrap()
            .update(&args.id, &args.name, args.params)?;
        json_s(&preset)
    }

    pub(crate) fn delete_preset(app: &AppHandle, args: PresetIdArgs) -> Result<String, String> {
        let state = app.state::<AppState>();
        state.presets.lock().unwrap().remove(&args.id)?;
        Ok(format!("预设 {} 已删除", args.id))
    }

    // ---- 音色记忆库 ----

    /// 按 query / tag / plugin_id / limit 过滤摘要。抽成纯函数方便测试。
    pub(crate) fn filter_sound_memories(
        items: Vec<crate::sound_memories::SoundMemorySummary>,
        args: &ListSoundMemoriesArgs,
    ) -> Vec<crate::sound_memories::SoundMemorySummary> {
        let query = args.query.as_deref().unwrap_or("").trim().to_lowercase();
        let tag = args.tag.as_deref().unwrap_or("").trim().to_lowercase();
        let plugin_id = args.plugin_id.as_deref().unwrap_or("").trim().to_string();
        let limit = args.limit.unwrap_or(50).min(200);
        // 老工程的内置合成器可能直接挂在预设 id 上（`builtin.synth.lead`）。
        let matches_plugin = |id: &str| {
            id == plugin_id
                || (plugin_id == "builtin.synth" && id.starts_with("builtin.synth."))
        };
        items
            .into_iter()
            .filter(|m| {
                if !plugin_id.is_empty()
                    && !m.instrument_plugin_id.as_deref().is_some_and(matches_plugin)
                    && !m.effect_plugin_ids.iter().any(|id| matches_plugin(id))
                {
                    return false;
                }
                if !tag.is_empty() && !m.tags.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
                    return false;
                }
                if query.is_empty() {
                    return true;
                }
                let haystack = format!(
                    "{} {} {} {} {}",
                    m.name,
                    m.description,
                    m.tags.join(" "),
                    m.role.as_deref().unwrap_or(""),
                    m.instrument_plugin_id.as_deref().unwrap_or(""),
                )
                .to_lowercase();
                haystack.contains(&query)
            })
            .take(limit)
            .collect()
    }

    pub(crate) fn list_sound_memories(
        app: &AppHandle,
        args: ListSoundMemoriesArgs,
    ) -> Result<String, String> {
        let state = app.state::<AppState>();
        let all = state.sound_memories.lock().unwrap().summaries();
        let total = all.len();
        let memories = filter_sound_memories(all, &args);
        json_s(&serde_json::json!({
            "total": total,
            "returned": memories.len(),
            "memories": memories,
        }))
    }

    pub(crate) fn get_sound_memory(
        app: &AppHandle,
        args: SoundMemoryIdArgs,
    ) -> Result<String, String> {
        let state = app.state::<AppState>();
        let memory = state
            .sound_memories
            .lock()
            .unwrap()
            .get(&args.id)
            .ok_or_else(|| format!("音色记忆不存在：{}", args.id))?;
        let mut value = serde_json::to_value(&memory).map_err(|e| format!("序列化失败：{e}"))?;
        summarize_plugin_states(&mut value);
        json_s(&value)
    }

    pub(crate) fn save_sound_memory(
        app: &AppHandle,
        args: SaveSoundMemoryArgs,
    ) -> Result<String, String> {
        let state = app.state::<AppState>();
        let snapshot = commands::sound_snapshot_for(&state, TrackId(args.track))?;
        let meta = MemoryMeta {
            name: args.name,
            description: args.description.unwrap_or_default(),
            tags: args.tags.unwrap_or_default(),
            role: snapshot.role.clone(),
        };
        let memory = {
            let mut store = state.sound_memories.lock().unwrap();
            match args.id {
                Some(id) => store.replace(&id, meta, snapshot)?,
                None => store.create(meta, snapshot)?,
            }
        };
        commands::notify_sound_memories(app);
        json_s(&memory.summary())
    }

    pub(crate) fn update_sound_memory(
        app: &AppHandle,
        args: UpdateSoundMemoryArgs,
    ) -> Result<String, String> {
        let state = app.state::<AppState>();
        let memory = state.sound_memories.lock().unwrap().update_metadata(
            &args.id,
            args.name.as_deref(),
            args.description.as_deref(),
            args.tags,
            args.role.as_deref(),
        )?;
        commands::notify_sound_memories(app);
        json_s(&memory.summary())
    }

    pub(crate) fn delete_sound_memory(
        app: &AppHandle,
        args: SoundMemoryIdArgs,
    ) -> Result<String, String> {
        let state = app.state::<AppState>();
        state.sound_memories.lock().unwrap().remove(&args.id)?;
        commands::notify_sound_memories(app);
        Ok(format!("音色记忆 {} 已删除", args.id))
    }

    pub(crate) fn apply_sound_memory(
        app: &AppHandle,
        actor: &Actor,
        args: ApplySoundMemoryArgs,
    ) -> Result<String, String> {
        let state = app.state::<AppState>();
        let memory = state
            .sound_memories
            .lock()
            .unwrap()
            .get(&args.id)
            .ok_or_else(|| format!("音色记忆不存在：{}", args.id))?;
        let track = TrackId(args.track);
        let (project, _events) =
            commands::apply_sound_memory_agent_as(app, actor.clone(), track, &memory)?;
        let t = project
            .tracks
            .iter()
            .find(|t| t.id == track)
            .ok_or_else(|| format!("轨道 {} 不存在", args.track))?;
        let instrument = t
            .instrument
            .as_ref()
            .map(|i| i.plugin_id.clone())
            .unwrap_or_else(|| "无".into());
        let chain = if t.effects.is_empty() {
            "空".into()
        } else {
            t.effects
                .iter()
                .map(|e| e.plugin_id.clone())
                .collect::<Vec<_>>()
                .join(" → ")
        };
        Ok(format!(
            "已把音色记忆「{}」应用到轨道 {}：乐器 {}，效果链 [{}]，增益 {:.1} dB，声像 {:.2}（一个撤销步）",
            memory.name, args.track, instrument, chain, t.mixer.gain_db, t.mixer.pan
        ))
    }

    // ---- 工具清单（供内置 agent 生成 OpenAI function 定义） ----

    /// 一个工具的对外定义：名字、描述、参数 JSON Schema。
    pub(crate) struct ToolDef {
        pub name: &'static str,
        pub description: &'static str,
        pub parameters: serde_json::Value,
    }

    /// schemars 生成的 JSON Schema（去掉 `$schema` 版本标记，兼容性更好）。
    fn schema_of<T: schemars::JsonSchema>() -> serde_json::Value {
        let mut v = serde_json::to_value(schemars::schema_for!(T))
            .unwrap_or_else(|_| serde_json::json!({ "type": "object" }));
        if let Some(obj) = v.as_object_mut() {
            obj.remove("$schema");
        }
        v
    }

    /// 无参工具的参数 schema。
    fn empty_schema() -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    macro_rules! tool_def {
        ($name:literal, $desc:literal, $args:ty) => {
            ToolDef { name: $name, description: $desc, parameters: schema_of::<$args>() }
        };
        ($name:literal, $desc:literal) => {
            ToolDef { name: $name, description: $desc, parameters: empty_schema() }
        };
    }

    /// 全部工具的清单。**要与 `#[tool_router]` 的工具集一一对应**（有守卫测试）。
    pub(crate) fn definitions() -> Vec<ToolDef> {
        vec![
            tool_def!("get_project", "读取当前工程的**全量**快照：轨道、片段、音符、速度/拍号（含关键点）、混音。插件二进制状态只给 `state_bytes`（不回传原始字节）。只想看结构时用 get_project_overview——这份快照在大工程里很占上下文。"),
            tool_def!("get_project_overview", "读取工程的**轻量概览**：轨道 id / 名字 / 分工 / 乐器（id 与名称）/ 效果链 / 混音、每个片段的 id / 位置 / 音符数、速度拍号与总长、段落表。不含音符与插件二进制状态。**了解现状先调这个**，再按需读片段音符。"),
            tool_def!("get_project_path", "读取当前工程对应的文件路径与是否有未保存改动（`path` / `dirty`）。"),
            tool_def!("get_clip_notes", "按页读取一个片段的音符（只读）：可按片段内拍范围过滤；返回 notes / note_count / truncated / next_offset。大工程核对音符时用它，别依赖批量工具的前 32 个摘要。", GetNotesArgs),
            tool_def!("get_transport", "读取传输状态：是否可用、是否播放、播放头位置（帧）、采样率、是否循环。"),
            tool_def!("list_plugins", "列出**当前可用的全部乐器与效果器**（内置 + 本机扫描到的 VST3/CLAP），每条带分类（bass / drums / reverb / dynamics…）、一句话音色描述、推荐分工与建议音域。挑音色、挑效果器、或不确定某个插件是什么时先调它；`kind` 只看乐器/效果器，`query` 按关键词过滤。", ListPluginsArgs),
            tool_def!("rescan_plugins", "重新扫描本机插件目录（装完新插件后调用），完成后 `list_plugins` 就能看到新装的东西。较慢。"),
            tool_def!("describe_plugin", "看**单个**插件的详情：分类与音色描述，加上它的**全部音色参数**（id / 名称 / 范围 / 默认值）与**用户预设**（名字 + 参数快照），以及用法提示。渐进披露的第二层——`list_plugins` 只给描述，这里才展开参数与预设。内置合成器还会列出出厂音色目录。", DescribePluginArgs),
            tool_def!("list_synth_presets", "列出内置合成器的**出厂音色**（36 条，9 类）：id / 中文名 / 分类 / 一句话描述 / 完整参数补丁。捏音色时先看这里，用 preview_synth 试听，再用 set_params(track, params, reset=true) 落地。", ListSynthPresetsArgs),
            tool_def!("transport_play", "开始播放。"),
            tool_def!("transport_pause", "暂停播放（保留当前位置）。"),
            tool_def!("transport_stop", "停止播放并回到起点。"),
            tool_def!("transport_seek", "把播放头定位到指定秒数（按当前设备采样率换算成帧）。", SeekArgs),
            tool_def!("transport_set_loop", "开关循环播放：播到工程结尾无缝回到起点继续。", ToggleNoTrackArgs),
            tool_def!("set_tempo", "设置工程速度（BPM）；音频片段的拍长会跟着重算。", TempoArgs),
            tool_def!("set_time_signature", "设置拍号；越界值由引擎归一化（分子 1..=32，分母吸附到 2 的幂）。", TimeSigArgs),
            tool_def!("set_keyframe", "在第 `beat` 拍写入 / 更新一个速度关键点（BPM + 拍号），用于渐快/渐慢。", KeyframeArgs),
            tool_def!("remove_keyframe", "删除第 `beat` 拍的速度关键点。", BeatArgs),
            tool_def!("add_track", "新增一条轨道；返回新轨道的 id。", NameArgs),
            tool_def!("remove_track", "删除一条轨道（连同它的片段与音符）。", TrackArgs),
            tool_def!("rename_track", "重命名一条轨道。", RenameArgs),
            tool_def!("set_track_instrument", "为轨道挂上乐器（id 用 list_plugins 拿；也接受 `JC303` 这样的名字）。返回里会说明这件乐器是什么、适合什么分工。", InstrumentArgs),
            tool_def!("set_param", "设置轨道乐器的某个参数（参数 id 用 describe_plugin 查）。改多个参数请用 set_params。", ParamArgs),
            tool_def!("set_params", "**一次写入整组**乐器参数（一个撤销步）——捏音色的主入口。`reset=true` 会先把所有参数复位到默认再写（换音色时用，避免残留）。未知参数会报错并列出可用 id。", SetParamsArgs),
            tool_def!("preview_synth", "离线试听一个**内置乐器**音色（不写工程、不进撤销、不含效果链）：渲染一个音，返回峰值/RMS/LUFS 与音色指纹（频谱重心、平坦度、波峰因数、分段能量）以及生效后的完整 params。捏音色时用它反复比较，满意后用 set_params 落地。", PreviewSynthArgs),
            tool_def!("add_effect", "把效果器追加到轨道效果链末尾（接受 id 或名字，用 list_plugins 看有哪些）。", EffectArgs),
            tool_def!("remove_effect", "移除轨道效果链里第 `index` 个效果器（0 起，链头是 0）。", EffectIndexArgs),
            tool_def!("set_effect_param", "设置轨道效果链里第 `index` 个效果器的一个参数（参数 id 用 describe_plugin 查）。改多个参数请用 set_effect_params。", EffectParamArgs),
            tool_def!("set_effect_params", "**一次写入**效果器的一整组参数（一个撤销步）——调混响/延迟/压缩的主入口。未知参数在插件侧会被忽略，先用 describe_plugin 核对 id。", EffectParamsArgs),
            tool_def!("move_effect", "调整效果器在效果链里的顺序（0 起）：`from` → `to`。", MoveEffectArgs),
            tool_def!("add_clip", "在轨道上新增一个 MIDI 片段；返回新片段 id。", ClipArgs),
            tool_def!("remove_clip", "删除一个片段。", ClipRefArgs),
            tool_def!("add_note", "在片段的指定位置新增一个音符。", NoteArgs),
            // ---- 批量音符 / 律动 / 音域 ----
            tool_def!("add_notes", "一次写入一组音符（**一次事务 = 一条撤销记录**，最多 5000 个）。返回新增的 id 与片段摘要；编曲请优先用它而不是循环 add_note。", NotesArgs),
            tool_def!("replace_notes", "**原子替换**片段内的全部音符（空数组 = 清空片段）。返回片段摘要。", NotesArgs),
            tool_def!("update_notes", "按**稳定 id** 批量改写音符（只改给出的字段）；删过音符之后 id 仍然指得准。", UpdateNotesArgs),
            tool_def!("set_note_tag", "给单个音符写 / 清除装饰音与奏法标签（工程级 `note_tags`，**不改变播放**）。装饰音标签会被 `analyze_harmony` 自动读取并标成 `ornament`。", SetNoteTagArgs),
            tool_def!("remove_notes", "按**稳定 id** 批量删除音符。", NoteIdsArgs),
            tool_def!("transpose_notes", "批量移调（半音），可限定拍范围。", TransposeArgs),
            tool_def!("set_note_velocity", "批量设置力度：`value` 固定值 / `from`+`to` 渐变 / `scale` 相对缩放（三选一），可限定拍范围；`grid`+`beat` 可按拍位过滤（例：grid=1.0、beat=0.5 只改反拍）。", VelocityArgs),
            tool_def!("quantize_notes", "量化：把音符起点吸到 `grid`（拍）的整数倍，`strength` 0..1 控制吸附程度。三连音用 `grid = 1/3`；`duration: true` 时时值也一起量化（缺省只动起点）。", QuantizeArgs),
            tool_def!("strum_notes", "扫弦：把**起点相同**的一组音按音高依次错开（下扫 = 低音先响），写出每个音符的起始偏移。`step_ms` 是相邻两音的毫秒差（0 = 回到同时发声）；间隔按当前速度换算，改速度不会毁掉扫弦。", StrumArgs),
            tool_def!("apply_swing", "Swing / 抢拍：移动每对子拍里的第二个。8 分 swing = subdivision 0.5 + ratio 0.66；ratio 0.5 平直，< 0.5 抢拍（push）。", SwingArgs),
            tool_def!("humanize_notes", "人性化：给位置与力度加确定性抖动（同一 seed 结果一致，撤销重做不漂移）。`min_duration` 可跳过倚音/装饰音，`group_by_start` 可让和弦/扫弦共用同一抖动。", HumanizeArgs),
            tool_def!("fit_notes_to_range", "把音符收进目标音域：`fold` 做八度折返（不改音级），`report` 只统计不改动。音域缺省取轨道的 pitch_range 元数据。", FitArgs),
            // ---- 片段编辑 ----
            tool_def!("move_clip", "把片段移动到新的起始拍。", MoveClipArgs),
            tool_def!("trim_clip", "裁剪 / 移动片段：同时给新的 `start` 与 `length`。音频片段按源帧调整素材窗口（不变速、不变调）；MIDI 片段只改方框，音符保持时间线位置。", TrimClipArgs),
            tool_def!("set_clip_rate", "设置**音频片段**的播放速度 `rate`（1.0 = 原速）与升降调 `pitch_semitones`；方框拍长会跟着重算。", ClipRateArgs),
            tool_def!("resize_clip", "改变片段长度（拍）；音频片段的素材窗口按比例跟着变。", ResizeClipArgs),
            tool_def!("split_clip", "在第 beat 拍切分片段（razor）：左半段沿用原 id，右半段是新片段。", SplitArgs),
            tool_def!("duplicate_clip", "复制片段：缺省贴在原片段之后，可换轨道。返回新片段摘要。", DuplicateClipArgs),
            tool_def!("merge_clips", "把同一轨道上的多个 **MIDI** 片段合并成一个（保留第一个的 id，跨度取并集）。音频片段不支持合并。", MergeClipsArgs),
            // ---- 编曲分析（只读） ----
            tool_def!("get_track_note_stats", "只读：轨道的音域（最低/最高/平均音高）、每小节密度、平均力度、同音重叠数、小二度/小九度冲突数、片段外音符数。", TrackStatsArgs),
            tool_def!("analyze_track_overlap", "只读：多条轨道之间的同时发声时长、同音重叠、小二度/小九度冲突、音区差与分工冲突建议。", OverlapArgs),
            tool_def!("analyze_harmony", "只读：按小节和弦表检查和声——和弦外音、装饰音、调内音、不推荐的延伸音、未解决导音、悬挂音、跨轨重叠。`key`+`scale` 提供调式上下文（调内非和弦音记成 scale_tone）；`ornaments` 用 `{track,clip,id}` 显式标记装饰音；`time_tolerance` 容忍 humanize/swing 的小节线漂移（缺省 1/16 拍）。和弦表缺省用工程里存的那份。", HarmonyArgs),
            // ---- 编曲元数据 ----
            tool_def!("set_project_harmony", "写入工程的小节和弦表（整体替换；空数组 = 清除）。符号写错会直接报错，不会存进工程。", SetHarmonyArgs),
            tool_def!("set_project_sections", "写入工程的段落表（前奏/主歌/副歌…，整体替换）。", SetSectionsArgs),
            tool_def!("set_track_meta", "设置轨道的编曲元数据：分工（lead/chords/bass/drums…）与目标音域。", TrackMetaArgs),
            tool_def!("set_gain", "设置轨道增益（dB）。", GainArgs),
            tool_def!("set_pan", "设置轨道声像（-1.0 最左，1.0 最右）。", PanArgs),
            tool_def!("set_mute", "设置轨道静音。", ToggleArgs),
            tool_def!("set_solo", "设置轨道独奏。", ToggleArgs),
            tool_def!("set_master_gain", "设置总输出增益（dB）。", MasterGainArgs),
            tool_def!("set_sends", "设置轨道发送列表（整体替换；空数组 = 清空）。目标通常是一条总线轨道。", SetSendsArgs),
            tool_def!("set_automation", "设置一条自动化车道（参数 `gain_db` / `pan` / `cc:<0..=127>`（MIDI 控制变化，值 0..127），或 `inst:<参数id>`（轨道乐器）/ `fx<i>:<参数id>`（效果链第 i 个，i 从 0 起）；整体替换该曲线，空 points = 删除）。", SetAutomationArgs),
            tool_def!("aidaw_help", "取回某个能力域的用法文档（渐进式披露）：留空给索引，按域取细则。", HelpArgs),
            tool_def!("aidaw_call", "网关：用「操作名 + JSON 参数」调用任意工具，例如 op=\"add_notes\"、args={...}。适合只接两条工具（本工具 + aidaw_help）的客户端；效果与直接调用那个工具完全一致。不能调用 aidaw_call 自己。", CallArgs),
            tool_def!("undo", "撤销最后一次编辑。"),
            tool_def!("redo", "重做上一次被撤销的编辑。"),
            tool_def!("save_project", "把当前工程保存为 JSON 文件（一个 `path` 参数；省略则存回当前工程文件）。保存前会把旧版本与当前状态记进历史。", SaveProjectArgs),
            tool_def!("open_project", "从 JSON 文件打开工程，**替换当前工程**（原工程先进历史）。", OpenProjectArgs),
            tool_def!("new_project", "新建空工程（默认多轨模板），**替换当前工程**（原工程先进历史）。", NewProjectArgs),
            tool_def!("rename_project", "重命名当前工程。", RenameProjectArgs),
            tool_def!("list_project_history", "列出历史工程快照（id / 名字 / 时间 / 轨道与音符数），供 restore_project_history 使用。"),
            tool_def!("restore_project_history", "从历史快照恢复工程（先备份当前工程）。id 由 list_project_history 给出。", HistoryIdArgs),
            tool_def!("analyze_project", "离线渲染并分析：积分响度 LUFS、响度范围 LRA、采样/真峰值、RMS 与平均幅度谱。可用 `start_beat` / `end_beat` 只分析一段，用 `tracks` 只分析指定轨道。", AnalyzeArgs),
            tool_def!("export_wav", "把工程离线渲染并导出为 WAV；返回写入路径。可用 `start_beat` / `end_beat` 只导出一段，用 `tracks` 只导出指定轨道。", ExportArgs),
            tool_def!("master_project", "对整个工程做一次母带处理（响度归一 + 真峰值限幅 + 立体声宽度），把结果写成 WAV 文件；返回处理前后的响度与真峰值，以及写入路径。", MasterExportArgs),
            tool_def!("separate_stems", "音源分离：把指定轨道的第一段音频拆成多条分轨，插到源轨道下面（一次事务，可整体撤销）。`model` 缺省为 4 分轨的 `htdemucs-ort-v1`；`htdemucs-6s` 出 6 分轨（+吉他/钢琴）。需要已安装对应模型（在「管理器 → 可下载」下载）。", SeparateArgs),
            tool_def!("detect_tempo", "节拍检测：对指定轨道的第一段音频跑 Beat This!，推出单一全局 BPM + 拍号写入工程，并把该片段对齐到小节线。需要已安装 Beat This! 模型。", DetectTempoArgs),
            tool_def!("transcribe_audio", "音频转 MIDI：对指定轨道的第一段音频做音高检测（有 Basic Pitch 模型就用，否则内置 DSP），把结果写成**源轨道下面的新 MIDI 轨**（一次撤销）。", TrackArgs),
            tool_def!("enhance_audio", "降噪：对指定轨道的第一段音频降噪，把结果写成**源轨道下面的新音频轨**（原轨不动，便于 A/B）。`mode=voice`（缺省，人声/去混响，优先 LocalVQE 模型）或 `mode=music`（乐器底噪，内置 DSP）。", EnhanceArgs),
            tool_def!("import_audio", "把本机的一个音频文件导入成音频片段（wav/flac/mp3/ogg/m4a）；返回新片段 id。", ImportAudioArgs),
            tool_def!("audio_waveform", "取某个音频文件的波形包络（每桶 min/max 两个值），用来确认素材内容。", WaveformArgs),
            tool_def!("list_input_devices", "列出本机可用的音频输入设备（录音用）。"),
            tool_def!("set_input_device", "选择录音输入设备；不传 device = 关闭输入流、释放麦克风。"),
            tool_def!("record_start", "开始录音：打开输入设备、从 `start` 拍走带采集到指定轨道。录完必须调用 record_stop 才会落成片段。", RecordStartArgs),
            tool_def!("record_stop", "结束录音：停带、写 WAV，并把录到的内容作为一条可撤销的音频片段挂到轨道上。"),
            tool_def!("apply_preset", "应用一条预设：一次事务写回整组乐器参数 + 整条效果链（效果链是替换而非叠加）。", ApplyPresetArgs),
            tool_def!("list_presets", "列出全部用户预设（跨工程）：id / 名字 / 归属插件 / 参数快照。"),
            tool_def!("save_preset", "把一组参数快照存成新的用户预设（跨工程）。", SavePresetArgs),
            tool_def!("update_preset", "覆盖一条已存在的用户预设（改名或整体替换参数快照）。", UpdatePresetArgs),
            tool_def!("delete_preset", "删除一条用户预设。", PresetIdArgs),
            tool_def!("list_sound_memories", "列出用户存过的音色记忆（跨工程）。只返回摘要；`query` 搜名称/描述/标签/分工/插件 id，`tag` 精确匹配标签，`plugin_id` 过滤乐器或效果链。做同类音色前先查这里。", ListSoundMemoriesArgs),
            tool_def!("get_sound_memory", "读取一条音色记忆的完整快照（乐器/效果链参数、增益/声像；插件二进制状态只给 `state_bytes`）。", SoundMemoryIdArgs),
            tool_def!("save_sound_memory", "把**当前轨道**的音色链存成一条音色记忆（后端从轨道抓快照，不需要逐字传参数）。传 `id` 表示覆盖该条记忆的内容。**只有用户明确要求或确认后才调用**。", SaveSoundMemoryArgs),
            tool_def!("update_sound_memory", "只修改音色记忆的元数据（名称/描述/标签/分工），不动声音快照。", UpdateSoundMemoryArgs),
            tool_def!("delete_sound_memory", "删除一条音色记忆。**只在用户明确要求时调用**。", SoundMemoryIdArgs),
            tool_def!("apply_sound_memory", "把一条音色记忆应用到轨道：一次事务写入乐器、整条效果链、增益/声像（一个撤销步）。", ApplySoundMemoryArgs),
        ]
    }

    /// 解析工具参数（`null` 视为空对象）。
    fn parse_args<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> Result<T, String> {
        let value = if value.is_null() { serde_json::Value::Object(Default::default()) } else { value };
        serde_json::from_value(value).map_err(|e| format!("参数解析失败：{e}"))
    }

    /// **按名字分派**一次工具调用。写操作以 `actor` 署名。
    pub(crate) fn dispatch(
        app: &AppHandle,
        actor: &Actor,
        name: &str,
        args: serde_json::Value,
    ) -> Result<String, String> {
        match name {
            "get_project" => get_project(app),
            "get_project_overview" => get_project_overview(app),
            "get_project_path" => get_project_path(app),
            "get_clip_notes" => get_clip_notes(app, parse_args(args)?),
            "get_transport" => get_transport(app),
            "list_plugins" => list_plugins(app, parse_args(args)?),
            "rescan_plugins" => rescan_plugins(app),
            "describe_plugin" => describe_plugin(app, parse_args(args)?),
            "list_synth_presets" => list_synth_presets(parse_args(args)?),
            "transport_play" => transport_play(app),
            "transport_pause" => transport_pause(app),
            "transport_stop" => transport_stop(app),
            "transport_seek" => transport_seek(app, parse_args(args)?),
            "transport_set_loop" => transport_set_loop(app, parse_args(args)?),
            "set_tempo" => set_tempo(app, actor, parse_args(args)?),
            "set_time_signature" => set_time_signature(app, actor, parse_args(args)?),
            "set_keyframe" => set_keyframe(app, actor, parse_args(args)?),
            "remove_keyframe" => remove_keyframe(app, actor, parse_args(args)?),
            "add_track" => add_track(app, actor, parse_args(args)?),
            "remove_track" => remove_track(app, actor, parse_args(args)?),
            "rename_track" => rename_track(app, actor, parse_args(args)?),
            "set_track_instrument" => set_track_instrument(app, actor, parse_args(args)?),
            "set_param" => set_param(app, actor, parse_args(args)?),
            "set_params" => set_params(app, actor, parse_args(args)?),
            "preview_synth" => preview_synth(app, parse_args(args)?),
            "add_effect" => add_effect(app, actor, parse_args(args)?),
            "remove_effect" => remove_effect(app, actor, parse_args(args)?),
            "set_effect_param" => set_effect_param(app, actor, parse_args(args)?),
            "set_effect_params" => set_effect_params(app, actor, parse_args(args)?),
            "move_effect" => move_effect(app, actor, parse_args(args)?),
            "add_clip" => add_clip(app, actor, parse_args(args)?),
            "remove_clip" => remove_clip(app, actor, parse_args(args)?),
            "add_note" => add_note(app, actor, parse_args(args)?),
            "add_notes" => add_notes(app, actor, parse_args(args)?),
            "replace_notes" => replace_notes(app, actor, parse_args(args)?),
            "update_notes" => update_notes(app, actor, parse_args(args)?),
            "set_note_tag" => set_note_tag(app, actor, parse_args(args)?),
            "remove_notes" => remove_notes(app, actor, parse_args(args)?),
            "transpose_notes" => transpose_notes(app, actor, parse_args(args)?),
            "set_note_velocity" => set_note_velocity(app, actor, parse_args(args)?),
            "quantize_notes" => quantize_notes(app, actor, parse_args(args)?),
            "strum_notes" => strum_notes(app, actor, parse_args(args)?),
            "apply_swing" => apply_swing(app, actor, parse_args(args)?),
            "humanize_notes" => humanize_notes(app, actor, parse_args(args)?),
            "fit_notes_to_range" => fit_notes_to_range(app, actor, parse_args(args)?),
            "move_clip" => move_clip(app, actor, parse_args(args)?),
            "trim_clip" => trim_clip(app, actor, parse_args(args)?),
            "set_clip_rate" => set_clip_rate(app, actor, parse_args(args)?),
            "resize_clip" => resize_clip(app, actor, parse_args(args)?),
            "split_clip" => split_clip(app, actor, parse_args(args)?),
            "duplicate_clip" => duplicate_clip(app, actor, parse_args(args)?),
            "merge_clips" => merge_clips(app, actor, parse_args(args)?),
            "get_track_note_stats" => track_note_stats(app, parse_args(args)?),
            "analyze_track_overlap" => analyze_track_overlap(app, parse_args(args)?),
            "analyze_harmony" => analyze_harmony(app, parse_args(args)?),
            "set_project_harmony" => set_project_harmony(app, actor, parse_args(args)?),
            "set_project_sections" => set_project_sections(app, actor, parse_args(args)?),
            "set_track_meta" => set_track_meta(app, actor, parse_args(args)?),
            "set_gain" => set_gain(app, actor, parse_args(args)?),
            "set_pan" => set_pan(app, actor, parse_args(args)?),
            "set_mute" => set_mute(app, actor, parse_args(args)?),
            "set_solo" => set_solo(app, actor, parse_args(args)?),
            "set_master_gain" => set_master_gain(app, actor, parse_args(args)?),
            "set_sends" => set_sends(app, actor, parse_args(args)?),
            "set_automation" => set_automation(app, actor, parse_args(args)?),
            "aidaw_help" => aidaw_help(parse_args(args)?),
            "aidaw_call" => aidaw_call(app, actor, parse_args(args)?),
            "undo" => undo(app),
            "redo" => redo(app),
            "save_project" => save_project(app, parse_args(args)?),
            "open_project" => open_project(app, parse_args(args)?),
            "new_project" => new_project(app, actor, parse_args(args)?),
            "rename_project" => rename_project(app, actor, parse_args(args)?),
            "list_project_history" => list_project_history(app),
            "restore_project_history" => restore_project_history(app, parse_args(args)?),
            "analyze_project" => analyze_project(app, parse_args(args)?),
            "export_wav" => export_wav(app, parse_args(args)?),
            "master_project" => master_project(app, parse_args(args)?),
            "separate_stems" => separate_stems(app, parse_args(args)?),
            "detect_tempo" => detect_tempo(app, parse_args(args)?),
            "transcribe_audio" => transcribe_audio(app, parse_args(args)?),
            "enhance_audio" => enhance_audio(app, parse_args(args)?),
            "import_audio" => import_audio(app, actor, parse_args(args)?),
            "audio_waveform" => audio_waveform(app, parse_args(args)?),
            "list_input_devices" => list_input_devices(),
            "set_input_device" => set_input_device(app, parse_args(args)?),
            "record_start" => record_start(app, parse_args(args)?),
            "record_stop" => record_stop(app, actor),
            "apply_preset" => apply_preset(app, actor, parse_args(args)?),
            "list_presets" => list_presets(app),
            "save_preset" => save_preset(app, parse_args(args)?),
            "update_preset" => update_preset(app, parse_args(args)?),
            "delete_preset" => delete_preset(app, parse_args(args)?),
            "list_sound_memories" => list_sound_memories(app, parse_args(args)?),
            "get_sound_memory" => get_sound_memory(app, parse_args(args)?),
            "save_sound_memory" => save_sound_memory(app, parse_args(args)?),
            "update_sound_memory" => update_sound_memory(app, parse_args(args)?),
            "delete_sound_memory" => delete_sound_memory(app, parse_args(args)?),
            "apply_sound_memory" => apply_sound_memory(app, actor, parse_args(args)?),
            other => Err(format!("未知工具：{other}")),
        }
    }
}

// ---------------------------------------------------------------------------
// 服务器
// ---------------------------------------------------------------------------

/// MCP 服务端：持有 `AppHandle`，每次工具调用都据此取到 `AppState`。
#[derive(Clone)]
pub struct DawMcpServer {
    app: AppHandle,
    tool_router: ToolRouter<Self>,
}

impl DawMcpServer {
    pub fn new(app: AppHandle) -> Self {
        Self { app, tool_router: Self::tool_router() }
    }

    /// MCP 工具写操作的署名。
    fn actor(&self) -> Actor {
        Actor::Agent(tools::MCP_ACTOR.into())
    }
}

#[tool_router]
impl DawMcpServer {
    // ---- 只读 ----

    #[tool(description = "读取当前工程的**全量**快照：轨道、片段、音符、速度/拍号（含关键点）、混音。插件二进制状态只给 `state_bytes`。只想看结构时用 get_project_overview——这份快照在大工程里很占上下文。")]
    fn get_project(&self) -> Result<String, ErrorData> {
        tools::get_project(&self.app).map_err(fail)
    }

    #[tool(description = "读取工程的**轻量概览**：轨道 id / 名字 / 分工 / 乐器（id 与名称）/ 效果链 / 混音、每个片段的 id / 位置 / 音符数、速度拍号与总长、段落表。不含音符与插件二进制状态。**了解现状先调这个**。")]
    fn get_project_overview(&self) -> Result<String, ErrorData> {
        tools::get_project_overview(&self.app).map_err(fail)
    }

    #[tool(description = "读取当前工程对应的文件路径与是否有未保存改动（`path` / `dirty`）。")]
    fn get_project_path(&self) -> Result<String, ErrorData> {
        tools::get_project_path(&self.app).map_err(fail)
    }

    #[tool(description = "按页读取一个片段的音符（只读）：可按片段内拍范围过滤；返回 notes / note_count / truncated / next_offset。大工程核对音符时用它，别依赖批量工具的前 32 个摘要。")]
    fn get_clip_notes(&self, Parameters(args): Parameters<GetNotesArgs>) -> Result<String, ErrorData> {
        tools::get_clip_notes(&self.app, args).map_err(fail)
    }

    #[tool(description = "读取传输状态：是否可用、是否播放、播放头位置（帧）、采样率、是否循环。")]
    fn get_transport(&self) -> Result<String, ErrorData> {
        tools::get_transport(&self.app).map_err(fail)
    }

    #[tool(description = "列出当前可用的全部乐器与效果器（内置 + 本机扫描到的 VST3/CLAP），每条带分类、一句话音色描述、推荐分工与建议音域。挑音色、挑效果器、或不确定某个插件是什么时先调它；`kind` 只看乐器/效果器，`query` 按关键词过滤。")]
    fn list_plugins(&self, Parameters(args): Parameters<ListPluginsArgs>) -> Result<String, ErrorData> {
        tools::list_plugins(&self.app, args).map_err(fail)
    }

    #[tool(description = "重新扫描本机插件目录（装完新插件后调用），完成后 `list_plugins` 就能看到新装的东西。较慢。")]
    async fn rescan_plugins(&self) -> Result<String, ErrorData> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || tools::rescan_plugins(&app))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "看**单个**插件的详情：分类与音色描述，加上它的**全部音色参数**（id / 名称 / 范围 / 默认值）与**用户预设**（名字 + 参数快照），以及用法提示。渐进披露的第二层——`list_plugins` 只给描述，这里才展开参数与预设。内置合成器还会列出出厂音色目录。")]
    async fn describe_plugin(&self, Parameters(args): Parameters<DescribePluginArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || tools::describe_plugin(&app, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "列出内置合成器的**出厂音色**（36 条，9 类）：id / 中文名 / 分类 / 一句话描述 / 完整参数补丁。捏音色时先看这里，用 preview_synth 试听，再用 set_params(track, params, reset=true) 落地。")]
    fn list_synth_presets(&self, Parameters(args): Parameters<ListSynthPresetsArgs>) -> Result<String, ErrorData> {
        tools::list_synth_presets(args).map_err(fail)
    }

    // ---- 传输 ----

    #[tool(description = "开始播放。")]
    fn transport_play(&self) -> Result<String, ErrorData> {
        tools::transport_play(&self.app).map_err(fail)
    }

    #[tool(description = "暂停播放（保留当前位置）。")]
    fn transport_pause(&self) -> Result<String, ErrorData> {
        tools::transport_pause(&self.app).map_err(fail)
    }

    #[tool(description = "停止播放并回到起点。")]
    fn transport_stop(&self) -> Result<String, ErrorData> {
        tools::transport_stop(&self.app).map_err(fail)
    }

    #[tool(description = "把播放头定位到指定秒数（按当前设备采样率换算成帧）。")]
    fn transport_seek(&self, Parameters(args): Parameters<SeekArgs>) -> Result<String, ErrorData> {
        tools::transport_seek(&self.app, args).map_err(fail)
    }

    #[tool(description = "开关循环播放：播到工程结尾无缝回到起点继续。")]
    fn transport_set_loop(&self, Parameters(args): Parameters<ToggleNoTrackArgs>) -> Result<String, ErrorData> {
        tools::transport_set_loop(&self.app, args).map_err(fail)
    }

    // ---- 工程与轨道 ----

    #[tool(description = "设置工程速度（BPM）。")]
    fn set_tempo(&self, Parameters(args): Parameters<TempoArgs>) -> Result<String, ErrorData> {
        tools::set_tempo(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "设置拍号；越界值由引擎归一化（分子 1..=32，分母吸附到 2 的幂）。")]
    fn set_time_signature(&self, Parameters(args): Parameters<TimeSigArgs>) -> Result<String, ErrorData> {
        tools::set_time_signature(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "在第 `beat` 拍写入 / 更新一个速度关键点（BPM + 拍号），用于渐快/渐慢。")]
    fn set_keyframe(&self, Parameters(args): Parameters<KeyframeArgs>) -> Result<String, ErrorData> {
        tools::set_keyframe(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "删除第 `beat` 拍的速度关键点。")]
    fn remove_keyframe(&self, Parameters(args): Parameters<BeatArgs>) -> Result<String, ErrorData> {
        tools::remove_keyframe(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "新增一条轨道；返回新轨道的 id。")]
    fn add_track(&self, Parameters(args): Parameters<NameArgs>) -> Result<String, ErrorData> {
        tools::add_track(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "删除一条轨道（连同它的片段与音符）。")]
    fn remove_track(&self, Parameters(args): Parameters<TrackArgs>) -> Result<String, ErrorData> {
        tools::remove_track(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "重命名一条轨道。")]
    fn rename_track(&self, Parameters(args): Parameters<RenameArgs>) -> Result<String, ErrorData> {
        tools::rename_track(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "为轨道挂上乐器（id 用 list_plugins 拿；也接受 `JC303` 这样的名字）。返回里会说明这件乐器是什么、适合什么分工。")]
    fn set_track_instrument(&self, Parameters(args): Parameters<InstrumentArgs>) -> Result<String, ErrorData> {
        tools::set_track_instrument(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "设置轨道乐器的某个参数（参数 id 用 describe_plugin 查）。改多个参数请用 set_params。")]
    fn set_param(&self, Parameters(args): Parameters<ParamArgs>) -> Result<String, ErrorData> {
        tools::set_param(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "**一次写入整组**乐器参数（一个撤销步）——捏音色的主入口。`reset=true` 会先把所有参数复位到默认再写（换音色时用，避免残留）。未知参数会报错并列出可用 id。")]
    fn set_params(&self, Parameters(args): Parameters<SetParamsArgs>) -> Result<String, ErrorData> {
        tools::set_params(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "离线试听一个**内置乐器**音色（不写工程、不进撤销、不含效果链）：渲染一个音，返回峰值/RMS/LUFS 与音色指纹（频谱重心、平坦度、波峰因数、分段能量）以及生效后的完整 params。捏音色时用它反复比较，满意后用 set_params 落地。")]
    fn preview_synth(&self, Parameters(args): Parameters<PreviewSynthArgs>) -> Result<String, ErrorData> {
        tools::preview_synth(&self.app, args).map_err(fail)
    }

    #[tool(description = "把效果器追加到轨道效果链末尾（接受 id 或名字，用 list_plugins 看有哪些）。")]
    fn add_effect(&self, Parameters(args): Parameters<EffectArgs>) -> Result<String, ErrorData> {
        tools::add_effect(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "移除轨道效果链里第 `index` 个效果器（0 起，链头是 0）。")]
    fn remove_effect(&self, Parameters(args): Parameters<EffectIndexArgs>) -> Result<String, ErrorData> {
        tools::remove_effect(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "设置轨道效果链里第 `index` 个效果器的一个参数（参数 id 用 describe_plugin 查）。改多个参数请用 set_effect_params。")]
    fn set_effect_param(&self, Parameters(args): Parameters<EffectParamArgs>) -> Result<String, ErrorData> {
        tools::set_effect_param(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "**一次写入**效果器的一整组参数（一个撤销步）——调混响/延迟/压缩的主入口。未知参数在插件侧会被忽略，先用 describe_plugin 核对 id。")]
    fn set_effect_params(&self, Parameters(args): Parameters<EffectParamsArgs>) -> Result<String, ErrorData> {
        tools::set_effect_params(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "调整效果器在效果链里的顺序（0 起）：`from` → `to`。")]
    fn move_effect(&self, Parameters(args): Parameters<MoveEffectArgs>) -> Result<String, ErrorData> {
        tools::move_effect(&self.app, &self.actor(), args).map_err(fail)
    }

    // ---- 片段与音符 ----

    #[tool(description = "在轨道上新增一个 MIDI 片段；返回新片段 id。")]
    fn add_clip(&self, Parameters(args): Parameters<ClipArgs>) -> Result<String, ErrorData> {
        tools::add_clip(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "删除一个片段。")]
    fn remove_clip(&self, Parameters(args): Parameters<ClipRefArgs>) -> Result<String, ErrorData> {
        tools::remove_clip(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "在片段的指定位置新增一个音符；可选 `expr` 写推弦 / 揉弦 / 滑音（单位见 schema）。")]
    fn add_note(&self, Parameters(args): Parameters<NoteArgs>) -> Result<String, ErrorData> {
        tools::add_note(&self.app, &self.actor(), args).map_err(fail)
    }

    // ---- 批量音符 / 律动 / 音域 ----

    #[tool(description = "一次写入一组音符（一次事务 = 一条撤销记录，最多 5000 个）。每个音符可带 `expr` 写推弦/揉弦/滑音；返回新增的 id 与片段摘要。编曲请优先用它而不是循环 add_note。")]
    fn add_notes(&self, Parameters(args): Parameters<NotesArgs>) -> Result<String, ErrorData> {
        tools::add_notes(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "原子替换片段内的全部音符（空数组 = 清空片段）；音符可带 `expr`。返回片段摘要。")]
    fn replace_notes(&self, Parameters(args): Parameters<NotesArgs>) -> Result<String, ErrorData> {
        tools::replace_notes(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "按稳定 id 批量改写音符（只改给出的字段）；`expr` 用于写/清除推弦、揉弦、滑音。删过音符之后 id 仍然指得准。")]
    fn update_notes(&self, Parameters(args): Parameters<UpdateNotesArgs>) -> Result<String, ErrorData> {
        tools::update_notes(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "给单个音符写 / 清除装饰音与奏法标签（工程级 `note_tags`，不改变播放）。装饰音标签会被 `analyze_harmony` 自动读取并标成 `ornament`。")]
    fn set_note_tag(&self, Parameters(args): Parameters<SetNoteTagArgs>) -> Result<String, ErrorData> {
        tools::set_note_tag(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "按稳定 id 批量删除音符。")]
    fn remove_notes(&self, Parameters(args): Parameters<NoteIdsArgs>) -> Result<String, ErrorData> {
        tools::remove_notes(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "批量移调（半音），可限定拍范围。")]
    fn transpose_notes(&self, Parameters(args): Parameters<TransposeArgs>) -> Result<String, ErrorData> {
        tools::transpose_notes(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "批量设置力度：`value` 固定值 / `from`+`to` 渐变 / `scale` 相对缩放（三选一），可限定拍范围；`grid`+`beat` 可按拍位过滤（例：grid=1.0、beat=0.5 只改反拍）。")]
    fn set_note_velocity(&self, Parameters(args): Parameters<VelocityArgs>) -> Result<String, ErrorData> {
        tools::set_note_velocity(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "量化：把音符起点吸到 `grid`（拍）的整数倍，`strength` 0..1 控制吸附程度。三连音用 `grid = 1/3`；`duration: true` 时时值也一起量化。")]
    fn quantize_notes(&self, Parameters(args): Parameters<QuantizeArgs>) -> Result<String, ErrorData> {
        tools::quantize_notes(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "扫弦：把**起点相同**的一组音按音高依次错开（下扫 = 低音先响），写出每个音符的起始偏移。`step_ms` 是相邻两音的毫秒差（0 = 回到同时发声）。")]
    fn strum_notes(&self, Parameters(args): Parameters<StrumArgs>) -> Result<String, ErrorData> {
        tools::strum_notes(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "Swing / 抢拍：移动每对子拍里的第二个。8 分 swing = subdivision 0.5 + ratio 0.66；ratio 0.5 平直，< 0.5 抢拍（push）。")]
    fn apply_swing(&self, Parameters(args): Parameters<SwingArgs>) -> Result<String, ErrorData> {
        tools::apply_swing(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "人性化：给位置与力度加确定性抖动（同一 seed 结果一致，撤销重做不漂移）。`min_duration` 可跳过倚音/装饰音，`group_by_start` 可让和弦/扫弦共用同一抖动。")]
    fn humanize_notes(&self, Parameters(args): Parameters<HumanizeArgs>) -> Result<String, ErrorData> {
        tools::humanize_notes(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "把音符收进目标音域：`fold` 做八度折返（不改音级），`report` 只统计不改动。音域缺省取轨道的 pitch_range 元数据。")]
    fn fit_notes_to_range(&self, Parameters(args): Parameters<FitArgs>) -> Result<String, ErrorData> {
        tools::fit_notes_to_range(&self.app, &self.actor(), args).map_err(fail)
    }

    // ---- 片段编辑 ----

    #[tool(description = "把片段移动到新的起始拍。")]
    fn move_clip(&self, Parameters(args): Parameters<MoveClipArgs>) -> Result<String, ErrorData> {
        tools::move_clip(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "裁剪 / 移动片段：同时给新的 `start` 与 `length`。音频片段按源帧调整素材窗口（不变速、不变调）；MIDI 片段只改方框，音符保持时间线位置。")]
    async fn trim_clip(&self, Parameters(args): Parameters<TrimClipArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        let actor = self.actor();
        tauri::async_runtime::spawn_blocking(move || tools::trim_clip(&app, &actor, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "设置**音频片段**的播放速度 `rate`（1.0 = 原速）与升降调 `pitch_semitones`；方框拍长会跟着重算。")]
    fn set_clip_rate(&self, Parameters(args): Parameters<ClipRateArgs>) -> Result<String, ErrorData> {
        tools::set_clip_rate(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "改变片段长度（拍）；音频片段的素材窗口按比例跟着变。")]
    fn resize_clip(&self, Parameters(args): Parameters<ResizeClipArgs>) -> Result<String, ErrorData> {
        tools::resize_clip(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "在第 beat 拍切分片段（razor）：左半段沿用原 id，右半段是新片段。")]
    fn split_clip(&self, Parameters(args): Parameters<SplitArgs>) -> Result<String, ErrorData> {
        tools::split_clip(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "复制片段：缺省贴在原片段之后，可换轨道。返回新片段摘要。")]
    fn duplicate_clip(&self, Parameters(args): Parameters<DuplicateClipArgs>) -> Result<String, ErrorData> {
        tools::duplicate_clip(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "把同一轨道上的多个 MIDI 片段合并成一个（保留第一个的 id，跨度取并集）。音频片段不支持合并。")]
    fn merge_clips(&self, Parameters(args): Parameters<MergeClipsArgs>) -> Result<String, ErrorData> {
        tools::merge_clips(&self.app, &self.actor(), args).map_err(fail)
    }

    // ---- 编曲分析（只读） ----

    #[tool(description = "只读：轨道的音域（最低/最高/平均音高）、每小节密度、平均力度、同音重叠数、小二度/小九度冲突数、片段外音符数。")]
    fn get_track_note_stats(&self, Parameters(args): Parameters<TrackStatsArgs>) -> Result<String, ErrorData> {
        tools::track_note_stats(&self.app, args).map_err(fail)
    }

    #[tool(description = "只读：多条轨道之间的同时发声时长、同音重叠、小二度/小九度冲突、音区差与分工冲突建议。")]
    async fn analyze_track_overlap(&self, Parameters(args): Parameters<OverlapArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || tools::analyze_track_overlap(&app, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "只读：按小节和弦表检查和声——和弦外音、装饰音、调内音、不推荐的延伸音、未解决导音、悬挂音、跨轨重叠。`key`+`scale` 提供调式上下文（调内非和弦音记成 scale_tone）；`ornaments` 用 `{track,clip,id}` 显式标记装饰音；`time_tolerance` 容忍 humanize/swing 的小节线漂移（缺省 1/16 拍）。和弦表缺省用工程里存的那份。")]
    async fn analyze_harmony(&self, Parameters(args): Parameters<HarmonyArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || tools::analyze_harmony(&app, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    // ---- 编曲元数据 ----

    #[tool(description = "写入工程的小节和弦表（整体替换；空数组 = 清除）。符号写错会直接报错，不会存进工程。")]
    fn set_project_harmony(&self, Parameters(args): Parameters<SetHarmonyArgs>) -> Result<String, ErrorData> {
        tools::set_project_harmony(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "写入工程的段落表（前奏/主歌/副歌…，整体替换）。")]
    fn set_project_sections(&self, Parameters(args): Parameters<SetSectionsArgs>) -> Result<String, ErrorData> {
        tools::set_project_sections(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "设置轨道的编曲元数据：分工（lead/chords/bass/drums…）、目标音域、MIDI 弯音范围（半音，默认 2）。")]
    fn set_track_meta(&self, Parameters(args): Parameters<TrackMetaArgs>) -> Result<String, ErrorData> {
        tools::set_track_meta(&self.app, &self.actor(), args).map_err(fail)
    }

    // ---- 混音 ----

    #[tool(description = "设置轨道增益（dB）。")]
    fn set_gain(&self, Parameters(args): Parameters<GainArgs>) -> Result<String, ErrorData> {
        tools::set_gain(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "设置轨道声像（-1.0 最左，1.0 最右）。")]
    fn set_pan(&self, Parameters(args): Parameters<PanArgs>) -> Result<String, ErrorData> {
        tools::set_pan(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "设置轨道静音。")]
    fn set_mute(&self, Parameters(args): Parameters<ToggleArgs>) -> Result<String, ErrorData> {
        tools::set_mute(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "设置轨道独奏。")]
    fn set_solo(&self, Parameters(args): Parameters<ToggleArgs>) -> Result<String, ErrorData> {
        tools::set_solo(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "设置总输出增益（dB）。")]
    fn set_master_gain(&self, Parameters(args): Parameters<MasterGainArgs>) -> Result<String, ErrorData> {
        tools::set_master_gain(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "设置轨道发送列表（整体替换；空数组 = 清空）。目标通常是一条总线轨道。")]
    fn set_sends(&self, Parameters(args): Parameters<SetSendsArgs>) -> Result<String, ErrorData> {
        tools::set_sends(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "设置一条自动化车道（参数 `gain_db` / `pan` / `cc:<0..=127>`（MIDI 控制变化，值 0..127），或 `inst:<参数id>`（轨道乐器）/ `fx<i>:<参数id>`（效果链第 i 个，i 从 0 起）；整体替换该曲线，空 points = 删除）。")]
    fn set_automation(&self, Parameters(args): Parameters<SetAutomationArgs>) -> Result<String, ErrorData> {
        tools::set_automation(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "取回某个能力域的用法文档（渐进式披露）：留空给索引，按域取细则。")]
    fn aidaw_help(&self, Parameters(args): Parameters<HelpArgs>) -> Result<String, ErrorData> {
        tools::aidaw_help(args).map_err(fail)
    }

    #[tool(description = "网关：用「操作名 + JSON 参数」调用任意工具，例如 op=\"add_notes\"、args={...}。适合只接两条工具（本工具 + aidaw_help）的客户端；效果与直接调用那个工具完全一致。不能调用 aidaw_call 自己。")]
    fn aidaw_call(&self, Parameters(args): Parameters<CallArgs>) -> Result<String, ErrorData> {
        tools::aidaw_call(&self.app, &self.actor(), args).map_err(fail)
    }

    // ---- 历史 ----

    #[tool(description = "撤销最后一次编辑。")]
    fn undo(&self) -> Result<String, ErrorData> {
        tools::undo(&self.app).map_err(fail)
    }

    #[tool(description = "重做上一次被撤销的编辑。")]
    fn redo(&self) -> Result<String, ErrorData> {
        tools::redo(&self.app).map_err(fail)
    }

    #[tool(description = "把当前工程保存为 JSON 文件（一个 `path` 参数；省略则存回当前工程文件）。保存前会把旧版本与当前状态记进历史。")]
    async fn save_project(&self, Parameters(args): Parameters<SaveProjectArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || tools::save_project(&app, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "从 JSON 文件打开工程，**替换当前工程**（原工程先进历史）。")]
    async fn open_project(&self, Parameters(args): Parameters<OpenProjectArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || tools::open_project(&app, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "新建空工程（默认多轨模板），**替换当前工程**（原工程先进历史）。")]
    fn new_project(&self, Parameters(args): Parameters<NewProjectArgs>) -> Result<String, ErrorData> {
        tools::new_project(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "重命名当前工程。")]
    fn rename_project(&self, Parameters(args): Parameters<RenameProjectArgs>) -> Result<String, ErrorData> {
        tools::rename_project(&self.app, &self.actor(), args).map_err(fail)
    }

    #[tool(description = "列出历史工程快照（id / 名字 / 时间 / 轨道与音符数），供 restore_project_history 使用。")]
    fn list_project_history(&self) -> Result<String, ErrorData> {
        tools::list_project_history(&self.app).map_err(fail)
    }

    #[tool(description = "从历史快照恢复工程（先备份当前工程）。id 由 list_project_history 给出。")]
    fn restore_project_history(&self, Parameters(args): Parameters<HistoryIdArgs>) -> Result<String, ErrorData> {
        tools::restore_project_history(&self.app, args).map_err(fail)
    }

    // ---- 渲染与分析（重活，放到阻塞线程池） ----

    #[tool(description = "离线渲染并分析：积分响度 LUFS、响度范围 LRA、采样/真峰值、RMS 与平均幅度谱。可用 `start_beat` / `end_beat` 只分析一段，用 `tracks` 只分析指定轨道。")]
    async fn analyze_project(&self, Parameters(args): Parameters<AnalyzeArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || tools::analyze_project(&app, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "把工程离线渲染并导出为 WAV；返回写入路径。可用 `start_beat` / `end_beat` 只导出一段，用 `tracks` 只导出指定轨道。")]
    async fn export_wav(&self, Parameters(args): Parameters<ExportArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || tools::export_wav(&app, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "对整个工程做一次母带处理（响度归一 + 真峰值限幅 + 立体声宽度），把结果写成 WAV 文件；返回处理前后的响度与真峰值，以及写入路径。")]
    async fn master_project(&self, Parameters(args): Parameters<MasterExportArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || tools::master_project(&app, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "音源分离：把指定轨道的第一段音频拆成多条分轨，插到源轨道下面（一次事务，可整体撤销）。`model` 缺省为 4 分轨的 `htdemucs-ort-v1`；`htdemucs-6s` 出 6 分轨（+吉他/钢琴）。需要已安装对应模型（在「管理器 → 可下载」下载）。")]
    async fn separate_stems(&self, Parameters(args): Parameters<SeparateArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || tools::separate_stems(&app, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "节拍检测：对指定轨道的第一段音频跑 Beat This!，推出**单一全局 BPM + 拍号**写入工程，并把该片段对齐到小节线（速度 / 拍号 / 片段位置在同一笔撤销记录里，可整体撤销）。**只对节拍稳定**的音乐有效（流行/摇滚/电子/舞曲等）；自由速度（古典散板）、氛围/环境音、人声朗读、短于 2 秒的素材测出的 BPM 没有意义。需要已安装 Beat This! 模型（「管理器 → 可下载」下载 final0，约 83 MB）。")]
    async fn detect_tempo(&self, Parameters(args): Parameters<DetectTempoArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || tools::detect_tempo(&app, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "音频转 MIDI：对指定轨道的第一段音频做音高检测（有 Basic Pitch 模型就用，否则内置 DSP），把结果写成**源轨道下面的新 MIDI 轨**（一次撤销）。")]
    async fn transcribe_audio(&self, Parameters(args): Parameters<TrackArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || tools::transcribe_audio(&app, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "降噪：对指定轨道的第一段音频降噪，把结果写成**源轨道下面的新音频轨**（原轨不动，便于 A/B）。`mode=voice`（缺省，人声/去混响，优先 LocalVQE 模型）或 `mode=music`（乐器底噪，内置 DSP）。")]
    async fn enhance_audio(&self, Parameters(args): Parameters<EnhanceArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || tools::enhance_audio(&app, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "把本机的一个音频文件导入成音频片段（wav/flac/mp3/ogg/m4a）；返回新片段 id。")]
    async fn import_audio(&self, Parameters(args): Parameters<ImportAudioArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        let actor = self.actor();
        tauri::async_runtime::spawn_blocking(move || tools::import_audio(&app, &actor, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "取某个音频文件的波形包络（每桶 min/max 两个值），用来确认素材内容。")]
    async fn audio_waveform(&self, Parameters(args): Parameters<WaveformArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || tools::audio_waveform(&app, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "列出本机可用的音频输入设备（录音用）。")]
    async fn list_input_devices(&self) -> Result<String, ErrorData> {
        tauri::async_runtime::spawn_blocking(tools::list_input_devices)
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "选择录音输入设备；不传 device = 关闭输入流、释放麦克风。")]
    fn set_input_device(&self, Parameters(args): Parameters<InputDeviceArgs>) -> Result<String, ErrorData> {
        tools::set_input_device(&self.app, args).map_err(fail)
    }

    #[tool(description = "开始录音：打开输入设备、从 `start` 拍走带采集到指定轨道。录完必须调用 record_stop 才会落成片段。")]
    async fn record_start(&self, Parameters(args): Parameters<RecordStartArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || tools::record_start(&app, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "结束录音：停带、写 WAV，并把录到的内容作为一条可撤销的音频片段挂到轨道上。")]
    async fn record_stop(&self) -> Result<String, ErrorData> {
        let app = self.app.clone();
        let actor = self.actor();
        tauri::async_runtime::spawn_blocking(move || tools::record_stop(&app, &actor))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    /// 一次事务写回「整组乐器参数 + 整条效果链」＝ 界面上点一下预设做的那件事。
    ///
    /// 效果链是**替换**而不是叠加：预设 = 整套音色。分两次调用会变成两个撤销步骤。
    #[tool(description = "应用一条预设：一次事务写回整组乐器参数 + 整条效果链（效果链是替换而非叠加）。")]
    async fn apply_preset(&self, Parameters(args): Parameters<ApplyPresetArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        let actor = self.actor();
        tauri::async_runtime::spawn_blocking(move || tools::apply_preset(&app, &actor, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "列出全部用户预设（跨工程）：id / 名字 / 归属插件 / 参数快照。")]
    fn list_presets(&self) -> Result<String, ErrorData> {
        tools::list_presets(&self.app).map_err(fail)
    }

    #[tool(description = "把一组参数快照存成新的用户预设（跨工程）。")]
    fn save_preset(&self, Parameters(args): Parameters<SavePresetArgs>) -> Result<String, ErrorData> {
        tools::save_preset(&self.app, args).map_err(fail)
    }

    #[tool(description = "覆盖一条已存在的用户预设（改名或整体替换参数快照）。")]
    fn update_preset(&self, Parameters(args): Parameters<UpdatePresetArgs>) -> Result<String, ErrorData> {
        tools::update_preset(&self.app, args).map_err(fail)
    }

    #[tool(description = "删除一条用户预设。")]
    fn delete_preset(&self, Parameters(args): Parameters<PresetIdArgs>) -> Result<String, ErrorData> {
        tools::delete_preset(&self.app, args).map_err(fail)
    }

    #[tool(description = "列出用户存过的音色记忆（跨工程）。只返回摘要；`query` 搜名称/描述/标签/分工/插件 id，`tag` 精确匹配标签，`plugin_id` 过滤乐器或效果链。做同类音色前先查这里。")]
    fn list_sound_memories(&self, Parameters(args): Parameters<ListSoundMemoriesArgs>) -> Result<String, ErrorData> {
        tools::list_sound_memories(&self.app, args).map_err(fail)
    }

    #[tool(description = "读取一条音色记忆的完整快照（乐器/效果链参数与二进制状态、增益/声像）。")]
    fn get_sound_memory(&self, Parameters(args): Parameters<SoundMemoryIdArgs>) -> Result<String, ErrorData> {
        tools::get_sound_memory(&self.app, args).map_err(fail)
    }

    #[tool(description = "把**当前轨道**的音色链存成一条音色记忆（后端从轨道抓快照，不需要逐字传参数）。传 `id` 表示覆盖该条记忆的内容。**只有用户明确要求或确认后才调用**。")]
    async fn save_sound_memory(&self, Parameters(args): Parameters<SaveSoundMemoryArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || tools::save_sound_memory(&app, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "只修改音色记忆的元数据（名称/描述/标签/分工），不动声音快照。")]
    fn update_sound_memory(&self, Parameters(args): Parameters<UpdateSoundMemoryArgs>) -> Result<String, ErrorData> {
        tools::update_sound_memory(&self.app, args).map_err(fail)
    }

    #[tool(description = "删除一条音色记忆。**只在用户明确要求时调用**。")]
    fn delete_sound_memory(&self, Parameters(args): Parameters<SoundMemoryIdArgs>) -> Result<String, ErrorData> {
        tools::delete_sound_memory(&self.app, args).map_err(fail)
    }

    #[tool(description = "把一条音色记忆应用到轨道：一次事务写入乐器、整条效果链、增益/声像（一个撤销步）。")]
    async fn apply_sound_memory(&self, Parameters(args): Parameters<ApplySoundMemoryArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        let actor = self.actor();
        tauri::async_runtime::spawn_blocking(move || tools::apply_sound_memory(&app, &actor, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }
}

/// `transport_set_loop` 的参数（没有轨道字段，但语义上仍是「开关」）。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ToggleNoTrackArgs {
    /// 开 / 关。
    on: bool,
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for DawMcpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "AI DAW 的内嵌 MCP 服务器。所有写操作与人在界面上的操作走同一条管线：\
             会进入撤销历史、在界面上留痕，人随时可以撤销。先用 get_project_overview 看结构\
             （轨道 / 乐器 / 片段 id 与音符数），需要逐音核对时用 get_clip_notes，只有确实要\
             全量字段时才用 get_project（它不含插件二进制状态）。\
             挑音色 / 效果器前先 list_plugins（带分类与音色描述，别凭插件名猜），\
             单个插件用 describe_plugin 看参数表；改完可以用 analyze_project 听感确认。",
        )
    }
}

// ---------------------------------------------------------------------------
// 配置与运行时
// ---------------------------------------------------------------------------

/// MCP 服务器的**默认监听端口**。设置页预填这个值；只有它被占用时才退回临时端口。
pub const DEFAULT_MCP_PORT: u16 = 50562;

/// 采样率无关的服务器配置：落到应用配置目录的 `mcp.json`，设置页可改。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    /// 是否运行（随应用启动，也可在设置里即时开关）。
    pub enabled: bool,
    /// 监听端口；默认 [`DEFAULT_MCP_PORT`]；`0` = 每次启动由系统分配临时端口。
    pub port: u16,
}

impl Default for McpConfig {
    fn default() -> Self {
        // 默认开启、固定端口（被占用时才退回临时端口）。
        Self { enabled: true, port: DEFAULT_MCP_PORT }
    }
}

/// 汇报给界面的配置 + 运行状态。
#[derive(Debug, Clone, Serialize)]
pub struct McpStatus {
    pub enabled: bool,
    pub port: u16,
    pub running: bool,
    /// 正在监听时的接入地址（形如 `http://127.0.0.1:8787/mcp`），否则为 `None`。
    pub url: Option<String>,
}

/// 运行中的服务器句柄。
struct Running {
    addr: SocketAddr,
    cancel: CancellationToken,
}

struct Inner {
    config: McpConfig,
    running: Option<Running>,
}

/// MCP 服务器的控制器：读写配置、按需启停。
///
/// 由 Tauri 管理（`app.manage`），设置页的 `mcp_status` / `set_mcp_config` 命令操作它。
pub struct McpRuntime {
    app: AppHandle,
    /// 配置文件位置；拿不到配置目录时为 `None`（配置只活在内存里）。
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
}

impl McpRuntime {
    /// 读取配置目录里的 `mcp.json`（缺失或损坏时退回默认值）。
    pub fn load(app: AppHandle, path: Option<PathBuf>) -> Self {
        let config = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str::<McpConfig>(&text).ok())
            .unwrap_or_default();
        Self { app, path, inner: Mutex::new(Inner { config, running: None }) }
    }

    /// 当前配置与运行状态。
    pub fn status(&self) -> McpStatus {
        let inner = self.inner.lock().unwrap();
        McpStatus {
            enabled: inner.config.enabled,
            port: inner.config.port,
            running: inner.running.is_some(),
            url: inner.running.as_ref().map(|r| format!("http://{}/mcp", r.addr)),
        }
    }

    /// 运行时**发现文件**：应用配置目录下的 `mcp-url`，内容是**实际**监听地址。
    ///
    /// 与配置 `mcp.json` 分开：这是运行时状态、不进版本控制。端口被占用而退回临时端口时，
    /// 只有这里能告诉外部 `aidaw` CLI 真正的地址。
    fn url_file(&self) -> Option<PathBuf> {
        self.path.as_ref().and_then(|p| p.parent()).map(|dir| dir.join("mcp-url"))
    }

    /// 按已存配置启动（启动时调用；未启用则什么都不做）。
    pub fn autostart(&self) {
        if self.inner.lock().unwrap().config.enabled {
            if let Err(err) = self.start() {
                eprintln!("[mcp] 启动失败：{err}");
            }
        } else if let Some(path) = self.url_file() {
            // 未启用时清掉上一次残留的发现文件，免得 CLI 以为软件还开着。
            let _ = std::fs::remove_file(path);
        }
    }

    /// 应用新配置：写盘，并让运行状态跟上（开 → 起；关或换端口 → 重启）。
    pub fn apply(&self, config: McpConfig) -> Result<McpStatus, String> {
        self.save(&config);
        self.stop();
        let enabled = {
            let mut inner = self.inner.lock().unwrap();
            inner.config = config;
            inner.config.enabled
        };
        if enabled {
            self.start()?;
        }
        Ok(self.status())
    }

    fn save(&self, config: &McpConfig) {
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_string_pretty(config) {
            Ok(json) => {
                if let Err(err) = std::fs::write(path, json) {
                    eprintln!("[mcp] 写入配置失败：{err}");
                }
            }
            Err(err) => eprintln!("[mcp] 序列化配置失败：{err}"),
        }
    }

    /// 起服务器（已运行则空转）。先同步 bind，端口占用能立刻报错、也能马上拿到实际端口。
    fn start(&self) -> Result<(), String> {
        let (port, already_running) = {
            let inner = self.inner.lock().unwrap();
            (inner.config.port, inner.running.is_some())
        };
        if already_running {
            return Ok(());
        }
        // 首选配置端口；**只有它被别的进程占用时**才退回系统临时端口（0）。
        let listener = match bind_loopback(port) {
            Ok(listener) => listener,
            Err(err) if port != 0 => {
                eprintln!("[mcp] 端口 {port} 不可用（{err}），改用临时端口");
                bind_loopback(0)?
            }
            Err(err) => return Err(err),
        };
        let addr = listener.local_addr().map_err(|e| e.to_string())?;
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let app = self.app.clone();
        tauri::async_runtime::spawn(async move {
            let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
                eprintln!("[mcp] 接管监听失败");
                return;
            };
            let service: StreamableHttpService<DawMcpServer, LocalSessionManager> =
                StreamableHttpService::new(
                    move || Ok(DawMcpServer::new(app.clone())),
                    Default::default(),
                    StreamableHttpServerConfig::default(),
                );
            let router = axum::Router::new().nest_service("/mcp", service);
            let shutdown = async move { token.cancelled_owned().await };
            if let Err(err) = axum::serve(listener, router).with_graceful_shutdown(shutdown).await {
                eprintln!("[mcp] 服务异常：{err}");
            }
        });
        eprintln!("[mcp] 已启动：http://{addr}/mcp（仅本机）");
        self.inner.lock().unwrap().running = Some(Running { addr, cancel });
        // 把**实际**地址写给外部 CLI（薄客户端）——端口退回临时端口时也只有这里知道。
        if let Some(path) = self.url_file() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(err) = std::fs::write(&path, format!("http://{addr}/mcp")) {
                eprintln!("[mcp] 写入发现文件失败：{err}");
            }
        }
        Ok(())
    }

    /// 停服务器：取消令牌会让 axum 优雅退出；同时清掉发现文件。
    fn stop(&self) {
        if let Some(running) = self.inner.lock().unwrap().running.take() {
            running.cancel.cancel();
        }
        if let Some(path) = self.url_file() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// 绑定回环地址。换端口 / 重启时旧监听可能还没释放，故做一小段重试。
fn bind_loopback(port: u16) -> Result<std::net::TcpListener, String> {
    let mut last = None;
    for _ in 0..20 {
        match std::net::TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => {
                listener.set_nonblocking(true).map_err(|e| e.to_string())?;
                return Ok(listener);
            }
            Err(err) => {
                last = Some(err);
                // 临时端口（0）不会冲突，重试只为等旧 socket 释放。
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    Err(format!("监听 127.0.0.1:{port} 失败：{}", last.map(|e| e.to_string()).unwrap_or_default()))
}

/// 读取 MCP 服务器的配置与运行状态。
#[tauri::command]
pub fn mcp_status(runtime: State<'_, McpRuntime>) -> McpStatus {
    runtime.status()
}

/// 更新 MCP 配置（写盘并即时启停），返回最新状态。
#[tauri::command]
pub fn set_mcp_config(
    runtime: State<'_, McpRuntime>,
    enabled: bool,
    port: u16,
) -> Result<McpStatus, String> {
    runtime.apply(McpConfig { enabled, port })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 音色记忆的检索：关键词、标签、插件都能组合过滤。
    #[test]
    fn sound_memory_filters_by_query_tag_and_plugin() {
        use crate::sound_memories::{SoundMemorySummary, SoundMixer};

        let summary = |id: &str, name: &str, tags: &[&str], inst: &str, effects: &[&str]| {
            SoundMemorySummary {
                id: id.into(),
                name: name.into(),
                description: format!("{name} description"),
                tags: tags.iter().map(|t| (*t).to_string()).collect(),
                role: Some("pad".into()),
                instrument_plugin_id: Some(inst.into()),
                effect_plugin_ids: effects.iter().map(|e| (*e).to_string()).collect(),
                mixer: SoundMixer {
                    gain_db: -6.0,
                    pan: 0.0,
                },
                source_project: None,
                created_at: 0,
                updated_at: 0,
            }
        };
        let items = vec![
            summary(
                "s1",
                "warm pad",
                &["pad", "warm"],
                "builtin.synth",
                &["builtin.reverb"],
            ),
            summary(
                "s2",
                "acid bass",
                &["bass", "acid"],
                "builtin.synth",
                &["builtin.delay"],
            ),
            summary("s3", "dry piano", &["keys"], "builtin.piano", &[]),
            summary(
                "s4",
                "old lead",
                &["lead"],
                "builtin.synth.lead",
                &[],
            ),
        ];

        let by_query = tools::filter_sound_memories(
            items.clone(),
            &ListSoundMemoriesArgs {
                query: Some("WARM".into()),
                tag: None,
                plugin_id: None,
                limit: None,
            },
        );
        assert_eq!(by_query.len(), 1);
        assert_eq!(by_query[0].id, "s1");

        let by_tag = tools::filter_sound_memories(
            items.clone(),
            &ListSoundMemoriesArgs {
                query: None,
                tag: Some("bass".into()),
                plugin_id: None,
                limit: None,
            },
        );
        assert_eq!(by_tag.len(), 1);
        assert_eq!(by_tag[0].id, "s2");

        let by_plugin = tools::filter_sound_memories(
            items.clone(),
            &ListSoundMemoriesArgs {
                query: None,
                tag: None,
                plugin_id: Some("builtin.reverb".into()),
                limit: None,
            },
        );
        assert_eq!(by_plugin.len(), 1);
        assert_eq!(by_plugin[0].id, "s1");

        // 旧工程的 `builtin.synth.lead` 也算 `builtin.synth`。
        let by_synth = tools::filter_sound_memories(
            items.clone(),
            &ListSoundMemoriesArgs {
                query: None,
                tag: None,
                plugin_id: Some("builtin.synth".into()),
                limit: None,
            },
        );
        assert_eq!(by_synth.len(), 3);
        assert!(by_synth.iter().any(|m| m.id == "s4"));

        let limited = tools::filter_sound_memories(
            items,
            &ListSoundMemoriesArgs {
                query: None,
                tag: None,
                plugin_id: None,
                limit: Some(2),
            },
        );
        assert_eq!(limited.len(), 2);
    }

    /// 工具表是「AI 能做什么」的唯一契约；漏注册一个这里就会响。
    #[test]
    fn every_tool_is_registered() {
        let router = DawMcpServer::tool_router();
        let names: Vec<String> = router.list_all().into_iter().map(|t| t.name.to_string()).collect();
        let expected = [
            "get_project",
            "get_project_overview",
            "get_project_path",
            "get_clip_notes",
            "get_transport",
            "list_plugins",
            "rescan_plugins",
            "describe_plugin",
            "list_synth_presets",
            "transport_play",
            "transport_pause",
            "transport_stop",
            "transport_seek",
            "transport_set_loop",
            "set_tempo",
            "set_time_signature",
            "set_keyframe",
            "remove_keyframe",
            "add_track",
            "remove_track",
            "rename_track",
            "set_track_instrument",
            "set_param",
            "set_params",
            "preview_synth",
            "add_effect",
            "remove_effect",
            "set_effect_param",
            "set_effect_params",
            "move_effect",
            "add_clip",
            "remove_clip",
            "add_note",
            "add_notes",
            "replace_notes",
            "update_notes",
            "set_note_tag",
            "remove_notes",
            "transpose_notes",
            "set_note_velocity",
            "quantize_notes",
            "strum_notes",
            "apply_swing",
            "humanize_notes",
            "fit_notes_to_range",
            "move_clip",
            "trim_clip",
            "set_clip_rate",
            "resize_clip",
            "split_clip",
            "duplicate_clip",
            "merge_clips",
            "get_track_note_stats",
            "analyze_track_overlap",
            "analyze_harmony",
            "set_project_harmony",
            "set_project_sections",
            "set_track_meta",
            "set_gain",
            "set_pan",
            "set_mute",
            "set_solo",
            "set_master_gain",
            "set_sends",
            "set_automation",
            "aidaw_help",
            "aidaw_call",
            "undo",
            "redo",
            "save_project",
            "open_project",
            "new_project",
            "rename_project",
            "list_project_history",
            "restore_project_history",
            "analyze_project",
            "export_wav",
            "master_project",
            "separate_stems",
            "detect_tempo",
            "transcribe_audio",
            "enhance_audio",
            "import_audio",
            "audio_waveform",
            "list_input_devices",
            "set_input_device",
            "record_start",
            "record_stop",
            "apply_preset",
            "list_presets",
            "save_preset",
            "update_preset",
            "delete_preset",
            "list_sound_memories",
            "get_sound_memory",
            "save_sound_memory",
            "update_sound_memory",
            "delete_sound_memory",
            "apply_sound_memory",
        ];
        for name in expected {
            assert!(router.get(name).is_some(), "缺少工具 {name}（现有：{names:?}）");
        }
        assert_eq!(names.len(), expected.len(), "工具数量对不上：{names:?}");

        // 内置 agent 的工具清单必须与 MCP 面一一对应——防止两个入口漂移。
        let defs: Vec<&str> = tools::definitions().iter().map(|d| d.name).collect();
        for name in expected {
            assert!(defs.contains(&name), "tools::definitions 缺少 {name}");
        }
        assert_eq!(defs.len(), expected.len(), "tools::definitions 数量对不上：{defs:?}");
    }

    #[test]
    fn slicing_by_beats_respects_tempo_and_offsets_the_output() {
        let mut transport = Transport::default();
        transport.tempo = 60.0; // 1 拍 = 1 秒
        let mut buffer = AudioBuffer::silence(2, 48_000);
        buffer.set_sample(0, 20_000, 1.0);

        let out = tools::slice_render_by_beats(&buffer, &transport, 48_000, 0.25, 0.5, 0.0);
        assert_eq!(out.frames(), 12_000, "0.25..0.5 秒 = 12000 帧");
        assert_eq!(out.sample(0, 8_000), 1.0, "原始第 20000 帧应当落到 8000 帧");
    }

    /// `snap` 字面量 → 步长（拍）：口径必须与卷帘里的网格一致（「一拍」= 拍号里那一拍）。
    #[test]
    fn snap_literals_match_the_piano_roll_grid() {
        // 4/4：一拍 = 1 拍；16 分 = 0.25，三连音（12 分）= 1/3，32 分 = 0.125，一小节 = 4。
        assert_eq!(tools::grid_step_beats("off", 1.0, 4.0).unwrap(), None);
        assert_eq!(tools::grid_step_beats("", 1.0, 4.0).unwrap(), None);
        assert_eq!(tools::grid_step_beats("bar", 1.0, 4.0).unwrap(), Some(4.0));
        assert_eq!(tools::grid_step_beats("1/4", 1.0, 4.0).unwrap(), Some(1.0));
        assert_eq!(tools::grid_step_beats("1/16", 1.0, 4.0).unwrap(), Some(0.25));
        assert_eq!(tools::grid_step_beats("1/32", 1.0, 4.0).unwrap(), Some(0.125));
        let triplet = tools::grid_step_beats("1/12", 1.0, 4.0).unwrap().unwrap();
        assert!((triplet - 1.0 / 3.0).abs() < 1e-12, "12 分音符 = 一拍的三分之一");
        // 6/8：一拍（八分音符）= 0.5 拍，一小节 = 3 拍。
        assert_eq!(tools::grid_step_beats("1/8", 0.5, 3.0).unwrap(), Some(0.25));
        assert_eq!(tools::grid_step_beats("bar", 0.5, 3.0).unwrap(), Some(3.0));
        // 不认识的字面量必须报错（而不是悄悄不吸附）。
        assert!(tools::grid_step_beats("1/5", 1.0, 4.0).is_err());
    }

    /// 每个工具都必须有描述，参数 schema 必须是一张 object 表——这是模型能不能正确调用它的下限。
    #[test]
    fn every_tool_has_a_description_and_an_object_schema() {
        for def in tools::definitions() {
            assert!(!def.description.trim().is_empty(), "工具 {} 缺描述", def.name);
            let params = &def.parameters;
            assert_eq!(params["type"], "object", "工具 {} 的参数 schema 必须是 object", def.name);
            assert!(
                params.get("properties").is_some(),
                "工具 {} 的 schema 缺 properties：{params}",
                def.name
            );
        }
    }

    /// 批量写入的参数契约：缺字段必须报错，而不是悄悄写进 0 个音符。
    #[test]
    fn add_notes_schema_requires_track_clip_and_notes() {
        let def = tools::definitions().into_iter().find(|d| d.name == "add_notes").unwrap();
        let required: Vec<&str> = def.parameters["required"]
            .as_array()
            .expect("required 数组")
            .iter()
            .map(|v| v.as_str().unwrap_or_default())
            .collect();
        for field in ["track", "clip", "notes"] {
            assert!(required.contains(&field), "add_notes 必须要求 {field}：{required:?}");
        }

        let err = serde_json::from_value::<NotesArgs>(serde_json::json!({ "track": 1 }))
            .expect_err("缺 clip / notes 必须解析失败");
        let text = err.to_string();
        assert!(text.contains("clip") || text.contains("notes"), "{text}");
    }

    /// 捏音色的两个入口：`set_params` 必须要求 track + params，`reset` 可省略；
    /// `preview_synth` 必须能无参调用（全部走默认）。
    #[test]
    fn sound_design_tool_schemas_match_their_contracts() {
        let def = tools::definitions().into_iter().find(|d| d.name == "set_params").unwrap();
        let required: Vec<&str> = def.parameters["required"]
            .as_array()
            .expect("required 数组")
            .iter()
            .map(|v| v.as_str().unwrap_or_default())
            .collect();
        assert!(required.contains(&"track"), "{required:?}");
        assert!(required.contains(&"params"), "{required:?}");
        assert!(!required.contains(&"reset"), "reset 必须是可选项：{required:?}");

        let parsed: SetParamsArgs = serde_json::from_value(serde_json::json!({
            "track": 1,
            "params": { "cutoff": 0.5 },
            "reset": true
        }))
        .unwrap();
        assert!(parsed.reset);
        let err = serde_json::from_value::<SetParamsArgs>(serde_json::json!({ "track": 1 }))
            .expect_err("缺 params 必须解析失败");
        assert!(err.to_string().contains("params"), "{err}");

        let preview: PreviewSynthArgs = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(preview.params.is_empty());
        assert!(!preview.reset);
        assert_eq!(preview.note, None);
    }

    /// 出厂音色目录：全量 36 条，按类别 / 关键词都能过滤。
    #[test]
    fn list_synth_presets_filters_by_category_and_query() {
        let all = tools::list_synth_presets(ListSynthPresetsArgs {
            category: None,
            query: None,
        })
        .unwrap();
        let all: serde_json::Value = serde_json::from_str(&all).unwrap();
        assert_eq!(all["count"], 36);
        assert_eq!(all["categories"].as_array().unwrap().len(), 9);

        let fx = tools::list_synth_presets(ListSynthPresetsArgs {
            category: Some("fx".into()),
            query: None,
        })
        .unwrap();
        let fx: serde_json::Value = serde_json::from_str(&fx).unwrap();
        assert_eq!(fx["count"], 4, "{fx}");

        let riser = tools::list_synth_presets(ListSynthPresetsArgs {
            category: None,
            query: Some("上升".into()),
        })
        .unwrap();
        let riser: serde_json::Value = serde_json::from_str(&riser).unwrap();
        assert_eq!(riser["count"], 1, "{riser}");
        assert_eq!(riser["items"][0]["id"], "builtin.synth.fx.riser");
        assert!(riser["items"][0]["params"].is_object());
    }

    /// 真正的预听路径：渲染必须出声，且关小截止会让频谱重心明显下降。
    #[test]
    fn audition_renders_and_cutoff_moves_the_centroid() {
        let synth = |cutoff: f32| {
            let mut plugin = PluginRegistry::with_builtins()
                .create("builtin.synth", TrackId(0), PluginSlot::Instrument)
                .expect("内置合成器必须存在");
            for (id, value) in [
                ("wave", 0.0),
                ("cutoff", cutoff),
                ("filterEnv", 0.0),
                ("attack", 0.0),
                ("decay", 2.0),
                ("sustain", 1.0),
                ("gain", 0.5),
            ] {
                plugin.set_param(&ParamId::from(id), value);
            }
            plugin.prepare(48_000.0, 256);
            let samples = tools::render_audition(&mut *plugin, 48_000, 60, 100, 300, 100);
            assert_eq!(samples.len(), 19_200, "400 ms @ 48 kHz");
            let peak = samples.iter().fold(0.0f32, |a, b| a.max(b.abs()));
            assert!(peak > 0.01, "试听必须出声：{peak}");
            ai_daw_analysis::timbre(&samples, 1, 48_000.0).unwrap()
        };

        let open = synth(1.0);
        let closed = synth(0.2);
        assert!(
            closed.spectral_centroid_hz < open.spectral_centroid_hz * 0.7,
            "关小截止应当明显变暗：open={} closed={}",
            open.spectral_centroid_hz,
            closed.spectral_centroid_hz
        );
    }

    /// 批量音符与 id 改写都要把 `expr` 带进引擎；`{}` 表示清除。
    #[test]
    fn note_input_and_patch_thread_pitch_expression() {
        let input: NoteInput = serde_json::from_value(serde_json::json!({
            "pitch": 64,
            "start": 0.0,
            "duration": 1.0,
            "expr": { "bend": 0, "bend_to": 200, "bend_time": 0.25 }
        }))
        .unwrap();
        let note = input.to_note();
        assert_eq!((note.expr.bend, note.expr.bend_to), (0, 200));
        assert!((note.expr.bend_time - 0.25).abs() < 1e-6);

        let patch: NotePatchInput =
            serde_json::from_value(serde_json::json!({ "id": 7, "expr": {} })).unwrap();
        assert_eq!(patch.to_patch().expr, Some(PitchExpression::NONE));

        let absent: NotePatchInput =
            serde_json::from_value(serde_json::json!({ "id": 7 })).unwrap();
        assert_eq!(absent.to_patch().expr, None, "没给 expr 时不应覆盖原值");
    }

    /// `describe_plugin` 只展开「音色参数」：MIDI CC 映射（每通道 128 个）不是旋钮，
    /// 不参与参数表——否则 JC303 / KR-106 这类插件会倒出两千多条，把上下文淹掉。
    #[test]
    fn midi_cc_mapping_is_not_a_sound_parameter() {
        assert!(tools::is_plumbing_param("MIDI CC 0|0"));
        assert!(tools::is_plumbing_param("midi cc 15|127"));
        assert!(!tools::is_plumbing_param("Cutoff"));
        assert!(!tools::is_plumbing_param("Bypass"));
        assert!(!tools::is_plumbing_param(""));
    }

    /// 和声工具的参数里，和弦符号是必填——没有和弦就没有检查的依据。
    #[test]
    fn analyze_harmony_accepts_a_bare_call() {
        // 全部参数可省（和弦表可以来自工程），所以空对象必须能解析。
        let args: HarmonyArgs = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(args.bars.is_none());
        assert!(args.tracks.is_none());
    }

    #[test]
    fn config_defaults_to_the_standard_port() {
        let config = McpConfig::default();
        assert!(config.enabled);
        assert_eq!(config.port, DEFAULT_MCP_PORT, "默认端口应是 {DEFAULT_MCP_PORT}");
        assert_eq!(DEFAULT_MCP_PORT, 50562);
    }

    /// skills 包必须覆盖每个能力域，且每篇都有内容。
    #[test]
    fn skill_docs_cover_every_domain() {
        let keys: Vec<&str> = SKILL_DOCS.iter().map(|(k, _)| *k).collect();
        for expected in [
            "", "project", "track", "clip", "note", "composition", "mix", "audio", "transport",
            "analysis", "plugin", "sound-design", "sound-memory", "theory", "rhythm", "genres",
            "workflow",
        ] {
            assert!(keys.contains(&expected), "缺少技能域「{expected}」");
        }
        for (topic, text) in SKILL_DOCS {
            assert!(!text.trim().is_empty(), "技能域「{topic}」内容为空");
        }
    }

    #[test]
    fn config_tolerates_missing_fields() {
        // 手改过的 / 旧版本的 `mcp.json` 缺字段时，缺的部分走默认值而不是整份丢弃。
        let config: McpConfig = serde_json::from_str(r#"{"port": 8787}"#).unwrap();
        assert!(config.enabled);
        assert_eq!(config.port, 8787);
    }

    #[test]
    fn binding_an_ephemeral_port_picks_a_real_one() {
        let listener = bind_loopback(0).expect("应当能绑到回环临时端口");
        assert_ne!(listener.local_addr().unwrap().port(), 0);
    }

    fn demo_track(id: u64, name: &str) -> Track {
        Track {
            id: TrackId(id),
            name: name.into(),
            instrument: None,
            effects: vec![],
            clips: vec![],
            mixer: ai_daw_engine::model::MixerChannel::default(),
            sends: vec![],
            automation: vec![],
            meta: None,
        }
    }

    fn demo_note(id: u64, pitch: u8) -> Note {
        Note {
            id,
            pitch,
            start: id as f64,
            duration: 1.0,
            velocity: 100,
            volume: Note::MAX_VOLUME,
            offset: 0.0,
            legato: false,
            expr: PitchExpression::NONE,
        }
    }

    /// 插件 state 是 `Vec<u8>`，直接序列化会膨胀成几百 KB 的字节数组——快照里必须只留体积。
    #[test]
    fn project_snapshot_summarizes_plugin_state() {
        let mut project = Project::new("demo");
        let mut track = demo_track(1, "主旋律");
        let mut params = std::collections::BTreeMap::new();
        params.insert("cutoff".to_string(), 800.0);
        track.instrument = Some(PluginInstance {
            plugin_id: "au:aumu:mlrs:mLab".into(),
            params,
            state: vec![1, 2, 3, 4],
        });
        project.tracks.push(track);

        let text = tools::project_json(&project).unwrap();
        assert!(!text.contains("\"state\""), "二进制状态不应回传：{text}");
        assert!(text.contains("\"state_bytes\":4"), "{text}");
        assert!(text.contains("cutoff"), "可读参数应当保留：{text}");
    }

    /// 概览只给 id / 骨架，不把音符数组带出来；片段 id 必须可直接用于后续工具。
    #[test]
    fn overview_keeps_ids_but_drops_notes() {
        let mut project = Project::new("demo");
        let mut track = demo_track(1, "主旋律");
        track.instrument = Some(PluginInstance {
            plugin_id: "builtin.synth".into(),
            params: std::collections::BTreeMap::new(),
            state: vec![0; 100],
        });
        track.clips.push(Clip {
            id: ClipId(7),
            start: 0.0,
            length: 8.0,
            notes: vec![demo_note(1, 60), demo_note(2, 64)],
            audio: None,
        });
        project.tracks.push(track);

        let value = tools::project_overview(&project, &[]);
        assert_eq!(value["tracks"][0]["id"], 1);
        assert_eq!(value["tracks"][0]["instrument"]["state_bytes"], 100);
        assert_eq!(value["tracks"][0]["clips"][0]["id"], 7);
        assert_eq!(value["tracks"][0]["clips"][0]["notes"], 2);
        assert_eq!(value["tracks"][0]["note_count"], 2);
        assert_eq!(value["totals"]["notes"], 2);
        assert!(!value.to_string().contains("\"pitch\""), "概览不应包含音符：{value}");
    }

    /// 找不到 id 的报错必须列出候选，模型才能一次改对，而不是挨个试。
    #[test]
    fn missing_id_errors_list_candidates() {
        let mut project = Project::new("demo");
        let mut track = demo_track(1, "主旋律");
        track.clips.push(Clip {
            id: ClipId(7),
            start: 0.0,
            length: 8.0,
            notes: vec![demo_note(1, 60)],
            audio: None,
        });
        let err = tools::missing_clip(&track, 9);
        assert!(err.contains("7（0.00–8.00 拍"), "{err}");
        project.tracks.push(track);
        let err = tools::missing_track(&project, 9);
        assert!(err.contains("#1「主旋律」"), "{err}");
    }
}
