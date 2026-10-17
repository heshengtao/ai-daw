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
use std::sync::{Arc, Mutex};
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

use ai_daw_engine::command::Command;
use ai_daw_engine::event::Event;
use ai_daw_engine::model::{AudioSource, Clip, ClipId, Note, TrackId};
use ai_daw_engine::transport::TransportCommand;

use crate::commands;
use crate::state::AppState;

/// 采样率的合法区间，与 `commands::validate_sample_rate` 保持一致。
const SAMPLE_RATES: std::ops::RangeInclusive<u32> = 8_000..=192_000;
/// 音频设备不可用时，定位换算用的兜底采样率。
const FALLBACK_SAMPLE_RATE: u32 = 44_100;

/// 工具错误 → MCP 的 `internal_error`。
fn fail(msg: impl Into<String>) -> ErrorData {
    ErrorData::new(ErrorCode::INTERNAL_ERROR, msg.into(), None)
}

/// 工具返回值统一是一段结构化 JSON 文本。
fn json<T: serde::Serialize>(value: &T) -> Result<String, ErrorData> {
    serde_json::to_string_pretty(value).map_err(|e| fail(format!("序列化失败：{e}")))
}

/// 校验采样率。
fn check_sample_rate(sample_rate: u32) -> Result<u32, ErrorData> {
    if SAMPLE_RATES.contains(&sample_rate) {
        Ok(sample_rate)
    } else {
        Err(fail(format!("非法采样率：{sample_rate}（应在 8000..=192000）")))
    }
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
struct TempoArgs {
    /// 新的速度（BPM）。
    bpm: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TimeSigArgs {
    /// 拍号分子（1..=32）。
    num: u8,
    /// 拍号分母（会吸附到 2 的幂）。
    den: u8,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TrackArgs {
    /// 轨道 id。
    track: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SeparateArgs {
    /// 轨道 id。
    track: u64,
    /// 分离模型：`htdemucs-ort-v1`（4 分轨，默认）或 `htdemucs-6s`（6 分轨）。
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DetectTempoArgs {
    /// 轨道 id（用它的第一段音频做节拍检测）。
    track: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct NameArgs {
    /// 轨道名。
    name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RenameArgs {
    /// 轨道 id。
    track: u64,
    /// 新名字。
    name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct InstrumentArgs {
    /// 轨道 id。
    track: u64,
    /// 乐器插件 id（内置用 `builtin.synth`，外部形如 `vst3:…` / `clap:…`）。
    plugin_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ParamArgs {
    /// 轨道 id。
    track: u64,
    /// 参数 id（由插件定义）。
    param: String,
    /// 参数值（按插件给出的范围）。
    value: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ClipArgs {
    /// 轨道 id。
    track: u64,
    /// 片段起始拍。
    start: f64,
    /// 片段长度（拍）。
    length: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ClipRefArgs {
    /// 轨道 id。
    track: u64,
    /// 片段 id。
    clip: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct NoteArgs {
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
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GainArgs {
    /// 轨道 id。
    track: u64,
    /// 增益（dB）。
    gain_db: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SendArg {
    /// 目标轨道 id（通常是一条总线轨道）。
    target: u64,
    /// 发送增益（dB）。
    gain_db: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SetSendsArgs {
    /// 轨道 id。
    track: u64,
    /// 发送列表（整体替换；空数组 = 清空）。
    sends: Vec<SendArg>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PointArg {
    /// 位置（拍）。
    beat: f64,
    /// 参数值。
    value: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SetAutomationArgs {
    /// 轨道 id。
    track: u64,
    /// 参数名：`gain_db` 或 `pan`，或 `inst:<参数id>`（轨道乐器）/ `fx<i>:<参数id>`（效果链第 i 个，i 从 0 起）。
    param: String,
    /// 关键点（整体替换；空数组 = 删除该车道）。
    points: Vec<PointArg>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HelpArgs {
    /// 能力域：`project` / `track` / `clip` / `note` / `mix` / `audio` / `transport` / `analysis` / `plugin`；
    /// 留空返回索引。
    #[serde(default)]
    topic: Option<String>,
}

/// 技能文档（编译期内嵌，无需资源打包）：`topic` → markdown。空 topic 是索引。
///
/// 目录结构 = 一个标准 Agent Skill：`skills/aidaw/SKILL.md`（入口，带 frontmatter）
/// + `skills/aidaw/references/*.md`（9 个能力域，按需加载）。
const SKILL_DOCS: &[(&str, &str)] = &[
    ("", include_str!("../../skills/aidaw/SKILL.md")),
    ("project", include_str!("../../skills/aidaw/references/project.md")),
    ("track", include_str!("../../skills/aidaw/references/track.md")),
    ("clip", include_str!("../../skills/aidaw/references/clip.md")),
    ("note", include_str!("../../skills/aidaw/references/note.md")),
    ("mix", include_str!("../../skills/aidaw/references/mix.md")),
    ("audio", include_str!("../../skills/aidaw/references/audio.md")),
    ("transport", include_str!("../../skills/aidaw/references/transport.md")),
    ("analysis", include_str!("../../skills/aidaw/references/analysis.md")),
    ("plugin", include_str!("../../skills/aidaw/references/plugin.md")),
];

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PanArgs {
    /// 轨道 id。
    track: u64,
    /// 声像：-1.0 最左，1.0 最右，0 居中。
    pan: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ToggleArgs {
    /// 轨道 id。
    track: u64,
    /// 开 / 关。
    on: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MasterGainArgs {
    /// 总输出增益（dB）。
    gain_db: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SeekArgs {
    /// 目标位置（秒）。
    seconds: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AnalyzeArgs {
    /// 采样率（Hz）；省略则用 44100。
    sample_rate: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ExportArgs {
    /// 目标 WAV 文件的绝对路径。
    path: String,
    /// 采样率（Hz）；省略则用 44100。
    sample_rate: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MasterExportArgs {
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
struct ImportAudioArgs {
    /// 音频文件的绝对路径（wav / flac / mp3 / ogg / m4a）。
    path: String,
    /// 目标轨道 id。
    track: u64,
    /// 片段起始拍。
    start: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WaveformArgs {
    /// 音频文件的绝对路径。
    path: String,
    /// 桶数（每桶给 min/max 两个值）。
    buckets: usize,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ApplyPresetArgs {
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

    fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }
}

#[tool_router]
impl DawMcpServer {
    // ---- 只读 ----

    #[tool(description = "读取当前工程的完整快照：轨道、片段、音符、速度/拍号（含关键点）、混音。")]
    fn get_project(&self) -> Result<String, ErrorData> {
        let project = self.state().engine.lock().unwrap().project().clone();
        json(&project)
    }

    #[tool(description = "读取传输状态：是否可用、是否播放、播放头位置（帧）、采样率、是否循环。")]
    fn get_transport(&self) -> Result<String, ErrorData> {
        json(&self.state().audio.snapshot())
    }

    #[tool(description = "列出本机已扫描到的插件（id、名称、格式）。可传给 set_track_instrument。")]
    fn list_plugins(&self) -> Result<String, ErrorData> {
        json(&self.state().catalog_snapshot())
    }

    // ---- 传输 ----

    #[tool(description = "开始播放。")]
    fn transport_play(&self) -> Result<String, ErrorData> {
        self.state().audio.send(TransportCommand::Play).map_err(fail)?;
        Ok("已开始播放".into())
    }

    #[tool(description = "暂停播放（保留当前位置）。")]
    fn transport_pause(&self) -> Result<String, ErrorData> {
        self.state().audio.send(TransportCommand::Pause).map_err(fail)?;
        Ok("已暂停".into())
    }

    #[tool(description = "停止播放并回到起点。")]
    fn transport_stop(&self) -> Result<String, ErrorData> {
        self.state().audio.send(TransportCommand::Stop).map_err(fail)?;
        Ok("已停止".into())
    }

    #[tool(description = "把播放头定位到指定秒数（按当前设备采样率换算成帧）。")]
    fn transport_seek(&self, Parameters(args): Parameters<SeekArgs>) -> Result<String, ErrorData> {
        let state = self.state();
        let sample_rate = {
            let rate = state.audio.snapshot().sample_rate;
            if rate == 0 { FALLBACK_SAMPLE_RATE } else { rate }
        };
        let frame = (args.seconds.max(0.0) * sample_rate as f64).round() as u64;
        state.audio.send(TransportCommand::Seek { frame }).map_err(fail)?;
        Ok(format!("已定位到 {} 秒（第 {} 帧）", args.seconds, frame))
    }

    #[tool(description = "开关循环播放：播到工程结尾无缝回到起点继续。")]
    fn transport_set_loop(&self, Parameters(args): Parameters<ToggleNoTrackArgs>) -> Result<String, ErrorData> {
        self.state().audio.send(TransportCommand::SetLoop(args.on)).map_err(fail)?;
        Ok(if args.on { "循环已开启".into() } else { "循环已关闭".into() })
    }

    // ---- 工程与轨道 ----

    #[tool(description = "设置工程速度（BPM）。")]
    fn set_tempo(&self, Parameters(args): Parameters<TempoArgs>) -> Result<String, ErrorData> {
        commands::run_agent(&self.app, Command::SetTempo(args.bpm));
        Ok(format!("速度已设为 {} BPM", args.bpm))
    }

    #[tool(description = "设置拍号；越界值由引擎归一化（分子 1..=32，分母吸附到 2 的幂）。")]
    fn set_time_signature(&self, Parameters(args): Parameters<TimeSigArgs>) -> Result<String, ErrorData> {
        commands::run_agent(&self.app, Command::SetTimeSignature { num: args.num, den: args.den });
        Ok(format!("拍号已设为 {}/{}", args.num, args.den))
    }

    #[tool(description = "新增一条轨道；返回新轨道的 id。")]
    fn add_track(&self, Parameters(args): Parameters<NameArgs>) -> Result<String, ErrorData> {
        let (project, events) = commands::run_agent(&self.app, Command::AddTrack { name: args.name.clone() });
        let id = track_added_id(&events);
        let tracks = project.tracks.len();
        Ok(format!("已新增轨道「{}」{}（现有 {} 条轨道）", args.name, id.map(|i| format!("，id={i}")).unwrap_or_default(), tracks))
    }

    #[tool(description = "删除一条轨道（连同它的片段与音符）。")]
    fn remove_track(&self, Parameters(args): Parameters<TrackArgs>) -> Result<String, ErrorData> {
        commands::run_agent(&self.app, Command::RemoveTrack(TrackId(args.track)));
        Ok(format!("已删除轨道 {}", args.track))
    }

    #[tool(description = "重命名一条轨道。")]
    fn rename_track(&self, Parameters(args): Parameters<RenameArgs>) -> Result<String, ErrorData> {
        commands::run_agent(&self.app, Command::RenameTrack { track: TrackId(args.track), name: args.name.clone() });
        Ok(format!("轨道 {} 已更名为「{}」", args.track, args.name))
    }

    #[tool(description = "为轨道挂上乐器插件（id 用 list_plugins 拿）。")]
    fn set_track_instrument(&self, Parameters(args): Parameters<InstrumentArgs>) -> Result<String, ErrorData> {
        commands::run_agent(&self.app, Command::SetTrackInstrument { track: TrackId(args.track), plugin_id: args.plugin_id.clone() });
        Ok(format!("轨道 {} 的乐器已设为 {}", args.track, args.plugin_id))
    }

    #[tool(description = "设置轨道乐器的某个参数。")]
    fn set_param(&self, Parameters(args): Parameters<ParamArgs>) -> Result<String, ErrorData> {
        commands::run_agent(&self.app, Command::SetParam { track: TrackId(args.track), param: args.param.clone(), value: args.value });
        Ok(format!("轨道 {} 的参数 {} 已设为 {}", args.track, args.param, args.value))
    }

    // ---- 片段与音符 ----

    #[tool(description = "在轨道上新增一个 MIDI 片段；返回新片段 id。")]
    fn add_clip(&self, Parameters(args): Parameters<ClipArgs>) -> Result<String, ErrorData> {
        let (_, events) = commands::run_agent(&self.app, Command::AddClip { track: TrackId(args.track), start: args.start, length: args.length });
        let id = clip_added_id(&events);
        Ok(format!("已在轨道 {} 的第 {} 拍新增片段{}", args.track, args.start, id.map(|i| format!("，id={i}")).unwrap_or_default()))
    }

    #[tool(description = "删除一个片段。")]
    fn remove_clip(&self, Parameters(args): Parameters<ClipRefArgs>) -> Result<String, ErrorData> {
        commands::run_agent(&self.app, Command::RemoveClip { track: TrackId(args.track), clip: ClipId(args.clip) });
        Ok(format!("已删除轨道 {} 上的片段 {}", args.track, args.clip))
    }

    #[tool(description = "在片段的指定位置新增一个音符。")]
    fn add_note(&self, Parameters(args): Parameters<NoteArgs>) -> Result<String, ErrorData> {
        let note = Note { pitch: args.pitch, start: args.start, duration: args.duration, velocity: args.velocity, volume: Note::MAX_VOLUME };
        commands::run_agent(&self.app, Command::AddNote { track: TrackId(args.track), clip: ClipId(args.clip), note });
        Ok(format!("已在片段 {} 加入音高 {} @ {} 拍", args.clip, args.pitch, args.start))
    }

    // ---- 混音 ----

    #[tool(description = "设置轨道增益（dB）。")]
    fn set_gain(&self, Parameters(args): Parameters<GainArgs>) -> Result<String, ErrorData> {
        commands::run_agent(&self.app, Command::SetGain { track: TrackId(args.track), gain_db: args.gain_db });
        Ok(format!("轨道 {} 增益已设为 {} dB", args.track, args.gain_db))
    }

    #[tool(description = "设置轨道声像（-1.0 最左，1.0 最右）。")]
    fn set_pan(&self, Parameters(args): Parameters<PanArgs>) -> Result<String, ErrorData> {
        commands::run_agent(&self.app, Command::SetPan { track: TrackId(args.track), pan: args.pan });
        Ok(format!("轨道 {} 声像已设为 {}", args.track, args.pan))
    }

    #[tool(description = "设置轨道静音。")]
    fn set_mute(&self, Parameters(args): Parameters<ToggleArgs>) -> Result<String, ErrorData> {
        commands::run_agent(&self.app, Command::SetMute { track: TrackId(args.track), mute: args.on });
        Ok(format!("轨道 {} 静音已{}", args.track, if args.on { "开启" } else { "关闭" }))
    }

    #[tool(description = "设置轨道独奏。")]
    fn set_solo(&self, Parameters(args): Parameters<ToggleArgs>) -> Result<String, ErrorData> {
        commands::run_agent(&self.app, Command::SetSolo { track: TrackId(args.track), solo: args.on });
        Ok(format!("轨道 {} 独奏已{}", args.track, if args.on { "开启" } else { "关闭" }))
    }

    #[tool(description = "设置总输出增益（dB）。")]
    fn set_master_gain(&self, Parameters(args): Parameters<MasterGainArgs>) -> Result<String, ErrorData> {
        commands::run_agent(&self.app, Command::SetMasterGain(args.gain_db));
        Ok(format!("总输出增益已设为 {} dB", args.gain_db))
    }

    #[tool(description = "设置轨道发送列表（整体替换；空数组 = 清空）。目标通常是一条总线轨道。")]
    fn set_sends(&self, Parameters(args): Parameters<SetSendsArgs>) -> Result<String, ErrorData> {
        let sends = args
            .sends
            .into_iter()
            .map(|s| ai_daw_engine::model::Send { target: TrackId(s.target), gain_db: s.gain_db })
            .collect();
        commands::run_agent(&self.app, Command::SetSends { track: TrackId(args.track), sends });
        Ok(format!("轨道 {} 的发送已更新", args.track))
    }

    #[tool(description = "设置一条自动化车道（参数 `gain_db` / `pan`，或 `inst:<参数id>`（轨道乐器）/ `fx<i>:<参数id>`（效果链第 i 个，i 从 0 起）；整体替换该曲线，空 points = 删除）。")]
    fn set_automation(&self, Parameters(args): Parameters<SetAutomationArgs>) -> Result<String, ErrorData> {
        let points = args
            .points
            .into_iter()
            .map(|p| ai_daw_engine::model::AutomationPoint { beat: p.beat, value: p.value })
            .collect();
        commands::run_agent(
            &self.app,
            Command::SetAutomation { track: TrackId(args.track), param: args.param.clone(), points },
        );
        Ok(format!("轨道 {} 的自动化（{}）已更新", args.track, args.param))
    }

    #[tool(description = "取回某个能力域的用法文档（渐进式披露）：留空给索引，按域取细则。")]
    fn aidaw_help(&self, Parameters(args): Parameters<HelpArgs>) -> Result<String, ErrorData> {
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

    // ---- 历史 ----

    #[tool(description = "撤销最后一次编辑。")]
    fn undo(&self) -> Result<String, ErrorData> {
        let out = commands::undo_now(&self.state());
        json(&out)
    }

    #[tool(description = "重做上一次被撤销的编辑。")]
    fn redo(&self) -> Result<String, ErrorData> {
        let out = commands::redo_now(&self.state());
        json(&out)
    }

    // ---- 渲染与分析（重活，放到阻塞线程池） ----

    #[tool(description = "离线渲染当前工程并分析：积分响度 LUFS、响度范围 LRA、采样/真峰值、RMS 与平均幅度谱。这是「听感」入口。")]
    async fn analyze_project(&self, Parameters(args): Parameters<AnalyzeArgs>) -> Result<String, ErrorData> {
        let sample_rate = check_sample_rate(args.sample_rate.unwrap_or(FALLBACK_SAMPLE_RATE))?;
        let inputs = {
            let state = self.state();
            commands::render_inputs(&state)
        };
        let report = tauri::async_runtime::spawn_blocking(move || {
            // 只分析工程正文，不加尾音。
            let buffer = commands::render_offline(inputs, sample_rate, 0.0);
            ai_daw_analysis::analyze(buffer.data(), buffer.channels(), sample_rate as f64)
        })
        .await
        .map_err(|e| fail(e.to_string()))?
        .map_err(|e| fail(e.to_string()))?;
        json(&report)
    }

    #[tool(description = "把当前工程离线渲染并导出为 WAV 文件；返回写入路径。")]
    async fn export_wav(&self, Parameters(args): Parameters<ExportArgs>) -> Result<String, ErrorData> {
        let sample_rate = check_sample_rate(args.sample_rate.unwrap_or(FALLBACK_SAMPLE_RATE))?;
        let path = args.path.clone();
        let inputs = {
            let state = self.state();
            commands::render_inputs(&state)
        };
        tauri::async_runtime::spawn_blocking(move || {
            let buffer = commands::render_offline(inputs, sample_rate, 0.5);
            ai_daw_engine::wav::write_wav(&path, &buffer, sample_rate).map_err(|e| fail(e.to_string()))?;
            Ok::<String, ErrorData>(format!("已导出到 {path}"))
        })
        .await
        .map_err(|e| fail(e.to_string()))?
    }

    #[tool(description = "对整个工程做一次母带处理（响度归一 + 真峰值限幅 + 立体声宽度），把结果写成 WAV 文件；返回处理前后的响度与真峰值，以及写入路径。")]
    async fn master_project(&self, Parameters(args): Parameters<MasterExportArgs>) -> Result<String, ErrorData> {
        let path = args.path.clone();
        let options = ai_daw_mastering::MasterOptions {
            target_lufs: args.target_lufs.unwrap_or(ai_daw_mastering::DEFAULT_TARGET_LUFS),
            ceiling_dbtp: args.ceiling_dbtp.unwrap_or(ai_daw_mastering::DEFAULT_CEILING_DBTP),
            stereo_width: args.stereo_width.unwrap_or(ai_daw_mastering::DEFAULT_WIDTH),
        };
        let sample_rate = FALLBACK_SAMPLE_RATE;
        let inputs = {
            let state = self.state();
            commands::render_inputs(&state)
        };
        let (report, path) = tauri::async_runtime::spawn_blocking(move || {
            let buffer = commands::render_offline(inputs, sample_rate, 0.5);
            let channels = buffer.channels();
            let (processed, report) =
                ai_daw_mastering::master(buffer.data(), channels, sample_rate as f64, &options)
                    .map_err(|e| fail(e.to_string()))?;
            let mut out = ai_daw_engine::audio::AudioBuffer::silence(channels, processed.len() / channels);
            out.data_mut().copy_from_slice(&processed);
            ai_daw_engine::wav::write_wav(&path, &out, sample_rate).map_err(|e| fail(e.to_string()))?;
            Ok::<_, ErrorData>((report, path))
        })
        .await
        .map_err(|e| fail(e.to_string()))??;
        json(&MasterExport { path, report })
    }

    #[tool(description = "音源分离：把指定轨道的第一段音频拆成多条分轨，插到源轨道下面（一次事务，可整体撤销）。`model` 缺省为 4 分轨的 `htdemucs-ort-v1`；`htdemucs-6s` 出 6 分轨（+吉他/钢琴）。需要已安装对应模型（在「管理器 → 可下载」下载）。")]
    async fn separate_stems(&self, Parameters(args): Parameters<SeparateArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        let count = tauri::async_runtime::spawn_blocking(move || {
            crate::ai::separate_track(&app, args.track, args.model)
        })
        .await
        .map_err(|e| fail(e.to_string()))?
        .map_err(fail)?;
        Ok(format!("已生成 {count} 条分轨（HTDemucs）"))
    }

    #[tool(description = "节拍检测：对指定轨道的第一段音频跑 Beat This!，推出**单一全局 BPM + 拍号**写入工程，并把该片段对齐到小节线（速度 / 拍号 / 片段位置在同一笔撤销记录里，可整体撤销）。**只对节拍稳定**的音乐有效（流行/摇滚/电子/舞曲等）；自由速度（古典散板）、氛围/环境音、人声朗读、短于 2 秒的素材测出的 BPM 没有意义。需要已安装 Beat This! 模型（「管理器 → 可下载」下载 final0，约 83 MB）。")]
    async fn detect_tempo(&self, Parameters(args): Parameters<DetectTempoArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || crate::ai::detect_tempo_track(&app, args.track))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "把本机的一个音频文件导入成音频片段（wav/flac/mp3/ogg/m4a）；返回新片段 id。")]
    async fn import_audio(&self, Parameters(args): Parameters<ImportAudioArgs>) -> Result<String, ErrorData> {
        let assets = { Arc::clone(&self.state().assets) };
        let path_for_load = args.path.clone();
        let audio = tauri::async_runtime::spawn_blocking(move || assets.load(&path_for_load))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)?;
        let seconds = audio.frames() as f64 / audio.sample_rate;
        if !(seconds.is_finite() && seconds > 0.0) {
            return Err(fail(format!("音频素材没有可用长度：{}", args.path)));
        }
        let start = Clip::normalize_start(args.start);
        // 让片段的拍长正好等于素材的自然时长（比值 ≈ 1，导入时不变调）。
        let length = {
            let state = self.state();
            let engine = state.engine.lock().unwrap();
            commands::natural_length_beats(engine.project(), start, seconds)
        };
        let (_, events) = commands::run_agent(
            &self.app,
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

    #[tool(description = "取某个音频文件的波形包络（每桶 min/max 两个值），用来确认素材内容。")]
    async fn audio_waveform(&self, Parameters(args): Parameters<WaveformArgs>) -> Result<String, ErrorData> {
        let assets = { Arc::clone(&self.state().assets) };
        let values = tauri::async_runtime::spawn_blocking(move || assets.waveform(&args.path, args.buckets))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)?;
        json(&values)
    }

    /// 一次事务写回「整组乐器参数 + 整条效果链」＝ 界面上点一下预设做的那件事。
    ///
    /// 效果链是**替换**而不是叠加：预设 = 整套音色。分两次调用会变成两个撤销步骤。
    #[tool(description = "应用一条预设：一次事务写回整组乐器参数 + 整条效果链（效果链是替换而非叠加）。")]
    async fn apply_preset(&self, Parameters(args): Parameters<ApplyPresetArgs>) -> Result<String, ErrorData> {
        let track = TrackId(args.track);
        let (project, events) =
            commands::apply_preset_agent(&self.app, track, &args.params, &args.effects);
        let _ = events;
        let chain = project
            .tracks
            .iter()
            .find(|t| t.id == track)
            .map(|t| {
                t.effects
                    .iter()
                    .map(|e| e.plugin_id.clone())
                    .collect::<Vec<_>>()
                    .join(" → ")
            })
            .unwrap_or_default();
        Ok(format!(
            "已应用到轨道 {}：{} 个参数，效果链 [{}]",
            args.track,
            args.params.len(),
            if chain.is_empty() { "空".into() } else { chain }
        ))
    }
}

/// `transport_set_loop` 的参数（没有轨道字段，但语义上仍是「开关」）。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ToggleNoTrackArgs {
    /// 开 / 关。
    on: bool,
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for DawMcpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "AI DAW 的内嵌 MCP 服务器。所有写操作与人在界面上的操作走同一条管线：\
             会进入撤销历史、在界面上留痕，人随时可以撤销。先用 get_project 了解现状，\
             改完可以用 analyze_project 听感确认。",
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

    /// 工具表是「AI 能做什么」的唯一契约；漏注册一个这里就会响。
    #[test]
    fn every_tool_is_registered() {
        let router = DawMcpServer::tool_router();
        let names: Vec<String> = router.list_all().into_iter().map(|t| t.name.to_string()).collect();
        let expected = [
            "get_project",
            "get_transport",
            "list_plugins",
            "transport_play",
            "transport_pause",
            "transport_stop",
            "transport_seek",
            "transport_set_loop",
            "set_tempo",
            "set_time_signature",
            "add_track",
            "remove_track",
            "rename_track",
            "set_track_instrument",
            "set_param",
            "add_clip",
            "remove_clip",
            "add_note",
            "set_gain",
            "set_pan",
            "set_mute",
            "set_solo",
            "set_master_gain",
            "set_sends",
            "set_automation",
            "aidaw_help",
            "undo",
            "redo",
            "analyze_project",
            "export_wav",
            "master_project",
            "separate_stems",
            "detect_tempo",
            "import_audio",
            "audio_waveform",
            "apply_preset",
        ];
        for name in expected {
            assert!(router.get(name).is_some(), "缺少工具 {name}（现有：{names:?}）");
        }
        assert_eq!(names.len(), expected.len(), "工具数量对不上：{names:?}");
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
            "", "project", "track", "clip", "note", "mix", "audio", "transport", "analysis", "plugin",
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
}
