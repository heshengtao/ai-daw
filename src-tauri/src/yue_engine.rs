//! **audio.cpp 引擎进程（`audiocpp_server`；本地写歌）**：起停、健康检查、提交生成任务、收结果入工程。
//!
//! 「AI 写歌」不绑定某一家模型：引擎按 **家族**（`family`）挂载权重——`yue2`（YuE2，
//! 先写谱再唱，有 ABC 谱 / 语义码续写 / 翻唱）或 `ace_step`（ACE-Step，描述 + 歌词 → 整首）。
//! 家族同时决定引擎配置与请求体的形状，两边都在本文件里（[`write_config`] / [`request_body`]）。
//!
//! 引擎是与宿主**完全解耦**的 HTTP 服务（`local-ai::engine` 负责把它下载下来）：
//!
//! ```text
//! audiocpp_server --config <runtime/yue2/server.json> --log-file <runtime/yue2/server.log>
//!                 --backend cpu|vulkan|cuda --no-ui
//! ```
//!
//! 配置里只有一条模型：
//!
//! ```json
//! {"models":[{"id":"yue2","family":"yue2","task":"gen","mode":"offline",
//!             "path":"<models/yue2>",
//!             "session_options":{"yue2.model_gguf":"yue2-3b-q8_0.gguf",
//!                                "yue2.vae_gguf":"yue2-vae-f16.gguf"}}]}
//! ```
//!
//! 协议只有两条：`POST /v1/tasks/run`（一次跑完，JSON 里 Base64 音频 + `score` artifact）
//! 和 `GET /health`（就绪）。生成是**分钟级同步请求**：没有轮询，取消只能杀进程（服务端没有
//! 取消端点），所以「取消」= 停引擎，下次生成重新拉起。
//!
//! 落地的东西：一段 48 kHz 立体声 WAV 进工程（一次生成 = 一条撤销记录），
//! 模型自己写了谱时另存一份 `.abc` 在音频旁边——**默认不把谱写成 MIDI 轨**：
//! YuE2 的 ABC 基本是它自己哼的草稿，直接铺进编排只会添乱；需要时在写歌页手动导入。

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::{Emitter, Manager};

use ai_daw_engine::command::Command as EngineCommand;
use ai_daw_engine::model::AudioSource;
use ai_daw_local_ai::engine;
use ai_daw_local_ai::models::catalog;

use crate::state::AppState;

/// 引擎状态广播的事件名。
pub const EVENT_ENGINE: &str = "engine://yue-engine";
/// 生成任务状态广播的事件名。
pub const EVENT_JOB: &str = "engine://yue-job";

/// 语义 token 的帧率（YuE2 文档：25 帧/秒）。界面给「时长」时换算成 `semantic_max_tokens`。
const SEMANTIC_FPS: f32 = 25.0;

/// 引擎状态（界面用）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct EngineStatus {
    /// `stopped` / `starting` / `ready` / `error`。
    pub state: String,
    /// 监听端口（就绪时有）。
    pub port: Option<u16>,
    /// 实际用的后端：`cuda13` / `cuda12` / `vulkan` / `cpu`。
    pub device: String,
    /// 模型 id（`yue2-q8-0` 这类；未解析时为空）。
    pub model: String,
    /// 一句话（启动中 / 出错时的原因）。
    pub message: Option<String>,
    /// 引擎日志最后几行（出错时就是它说的）。
    pub log: String,
}

impl Default for EngineStatus {
    fn default() -> Self {
        Self {
            state: "stopped".into(),
            port: None,
            device: "cpu".into(),
            model: String::new(),
            message: None,
            log: String::new(),
        }
    }
}

/// 生成任务状态（界面用）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct JobStatus {
    /// `idle` / `running` / `done` / `failed` / `cancelled`。
    pub state: String,
    /// 已经跑了多少秒。
    pub elapsed: f32,
    /// 一句话（完成时是摘要）。
    pub message: Option<String>,
    /// 落地的音频（完成时）。
    pub audio: Option<String>,
    /// 落地的谱面 `.abc`（模型给得出时）。
    pub score: Option<String>,
    /// 落地的语义码 `.json`（只有要了 `export_semantic` / `stop_after=semantic` 才有）。
    pub semantic: Option<String>,
    /// 这次生成的参数存档（`.json`，含 seed / 风格 / 谱 / 语义码路径，用来「重放」）。
    pub record: Option<String>,
    /// 成品时长（秒）。
    pub seconds: Option<f32>,
}

impl Default for JobStatus {
    fn default() -> Self {
        Self {
            state: "idle".into(),
            elapsed: 0.0,
            message: None,
            audio: None,
            score: None,
            semantic: None,
            record: None,
            seconds: None,
        }
    }
}

struct Engine {
    status: Mutex<EngineStatus>,
    child: Mutex<Option<Child>>,
    /// 取消标志：置位后不再收结果。
    cancel: AtomicBool,
    /// 已有一次生成在跑（服务端一次只跑一个）。
    busy: AtomicBool,
}

fn engine_state() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| Engine {
        status: Mutex::new(EngineStatus::default()),
        child: Mutex::new(None),
        cancel: AtomicBool::new(false),
        busy: AtomicBool::new(false),
    })
}

fn status() -> EngineStatus {
    engine_state().status.lock().unwrap().clone()
}

fn update(f: impl FnOnce(&mut EngineStatus)) {
    f(&mut engine_state().status.lock().unwrap());
}

fn emit_engine(app: &tauri::AppHandle) {
    let _ = app.emit(EVENT_ENGINE, status());
}

fn emit_job(app: &tauri::AppHandle, job: &JobStatus) {
    let _ = app.emit(EVENT_JOB, job.clone());
}

/// 当前引擎状态（界面初始化用）。
#[tauri::command]
pub fn ai_yue_engine_status() -> EngineStatus {
    status()
}

/// 引擎日志的最后若干行（界面轮询用；日志只落文件，服务端没有 SSE）。
#[tauri::command]
pub fn ai_yue_engine_log(lines: Option<usize>) -> String {
    log_tail(lines.unwrap_or(80).clamp(1, 500))
}

/// 是否有一次写歌任务正在生成。
///
/// `ai_yue_compose_cancel` 的语义是「停引擎」（服务端没有取消端点），只在真的在生成时
/// 才该调用——否则会把一个已就绪、权重还在显存里的引擎白白关掉。
pub(crate) fn composing() -> bool {
    engine_state().busy.load(Ordering::SeqCst)
}

/// 把生成时落盘的 `.abc` 谱转成工程 MIDI 轨（乐谱视图读的是工程里的 MIDI）。
#[tauri::command]
pub fn ai_yue_import_abc(app: tauri::AppHandle, path: String) -> Result<String, String> {
    let text = std::fs::read_to_string(&path).map_err(|e| format!("读谱面失败：{e}"))?;
    let voices = crate::abc::parse(&text)?;
    if voices.is_empty() {
        return Err("这份 ABC 里没有可导入的音符".into());
    }
    let mut note_count = 0usize;
    let mut commands = Vec::new();
    for voice in &voices {
        note_count += voice.notes.len();
        commands.push(EngineCommand::AddNoteTrack {
            name: voice.name.clone(),
            start: 0.0,
            length: voice.end.max(1.0),
            notes: voice.notes.clone(),
            at: None,
        });
    }
    crate::commands::run_agent_all(&app, commands);
    Ok(format!(
        "已把 {} 个声部 / {} 个音符写成 MIDI 轨（可在乐谱视图里看）",
        voices.len(),
        note_count
    ))
}

/// 引擎的 CUDA `cuda_perf` 预设开关（默认开；8 GB 显存的卡可关）。
#[tauri::command]
pub fn ai_yue_cuda_perf(app: tauri::AppHandle) -> bool {
    app.state::<crate::manager::StoreRuntime>()
        .network()
        .yue_cuda_perf
}

/// 写 `cuda_perf` 开关（下次启动引擎时生效）。
#[tauri::command]
pub fn ai_yue_set_cuda_perf(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let runtime = app.state::<crate::manager::StoreRuntime>();
    let mut network = runtime.network();
    network.yue_cuda_perf = enabled;
    runtime.set(network)
}

/// 读生成时落盘的 ABC 谱（写歌页「载入刚生成的谱」用）。
#[tauri::command]
pub fn ai_yue_score(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("读谱面失败：{e}"))
}

/// 读一份生成记录 `.json`（界面把参数填回表单 = 重放）。
#[tauri::command]
pub fn ai_yue_record(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("读生成记录失败：{e}"))
}

/// 一条能续写的轨道：语义码在哪、它有多长，以及**原曲是怎么生成的**。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ContinuableTrack {
    /// 轨道 id。
    pub track: u64,
    /// 语义码文件路径（直接当 `semantic_prefix_file` 用）。
    pub semantic: String,
    /// 前缀时长（秒）：语义码 25 帧/秒。
    pub seconds: f32,
    /// 原曲的风格（生成记录里的 `style`）：续写沿用，语义码才对得上。
    pub style: Option<String>,
    /// 原曲的歌词。
    pub lyrics: Option<String>,
    /// 原曲用的权重 id：换一档量化，同一段语义码也会漂。
    pub model: Option<String>,
}

/// 音频文件旁边的语义码：生成时 `<stem>.wav` 与 `<stem>.semantic.json` 同目录同名。
fn semantic_sidecar(audio: &str) -> PathBuf {
    std::path::Path::new(audio).with_extension("semantic.json")
}

/// 音频对应的**生成记录**（`<stem>.json`，与音频同名同目录）。
fn record_sidecar(audio: &str) -> PathBuf {
    std::path::Path::new(audio).with_extension("json")
}

/// 从生成记录里取「原曲的文本条件」：风格 / 歌词 / 权重。
///
/// 续写不是「再写一首」——语义码只是旋律的骨架，风格与歌词仍然是语义解码器的条件。
/// 不复用原曲的这两样，接出来的段落会像另一首歌（实测就是这个现象）。记录缺失时返回
/// 全空，界面退回让用户自己填。
fn record_conditions(audio: &str) -> (Option<String>, Option<String>, Option<String>) {
    let Ok(text) = std::fs::read_to_string(record_sidecar(audio)) else {
        return (None, None, None);
    };
    let Ok(record) = serde_json::from_str::<serde_json::Value>(&text) else {
        return (None, None, None);
    };
    let args = record.get("args");
    let pick = |key: &str| {
        args.and_then(|a| a.get(key))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty())
    };
    (pick("style"), pick("lyrics"), pick("model"))
}

/// 列出**所有能续写**的轨道（一次文件扫描，界面下拉直接用）。
///
/// 续写只吃引擎的 `semantic_prefix`（codec indices，不是音频），而语义码只在本机生成时落盘
/// （`<音频>.semantic.json`），所以外部导入的音频天然不在列表里——用户选不到它，也就不可能
/// 点出一个注定失败的「续写」。
///
/// 一条轨道可能已经续写过（多个音频片段）：取**最后一个**有音频的片段——续写总是接在
/// 最新那一段后面，而不是回到整条轨道的开头。
#[tauri::command]
pub fn ai_yue_continuable(app: tauri::AppHandle) -> Vec<ContinuableTrack> {
    let state = app.state::<AppState>();
    let tracks: Vec<(u64, String)> = {
        let engine = state.engine.lock().unwrap();
        engine
            .project()
            .tracks
            .iter()
            .filter_map(|t| {
                let clip = t.clips.iter().rev().find(|c| c.audio.is_some())?;
                let audio = clip.audio.as_ref()?.path.clone();
                Some((t.id.0, audio))
            })
            .collect()
    };
    tracks
        .into_iter()
        .filter_map(|(track, audio)| {
            let semantic = semantic_sidecar(&audio);
            let text = std::fs::read_to_string(&semantic).ok()?;
            let codes: Vec<i64> = serde_json::from_str(&text).ok()?;
            if codes.is_empty() {
                return None;
            }
            let (style, lyrics, model) = record_conditions(&audio);
            Some(ContinuableTrack {
                track,
                semantic: semantic.to_string_lossy().into_owned(),
                seconds: codes.len() as f32 / SEMANTIC_FPS,
                style,
                lyrics,
                model,
            })
        })
        .collect()
}

/// 当前的 LoRA 设置（两半各一份）。
#[tauri::command]
pub fn ai_yue_lora(app: tauri::AppHandle) -> crate::manager::YueLora {
    app.state::<crate::manager::StoreRuntime>()
        .network()
        .yue_lora
}

/// 写 LoRA 设置（**下次启动引擎生效**：适配器在会话创建时合并进权重）。
#[tauri::command]
pub fn ai_yue_set_lora(app: tauri::AppHandle, lora: crate::manager::YueLora) -> Result<(), String> {
    let runtime = app.state::<crate::manager::StoreRuntime>();
    let mut network = runtime.network();
    network.yue_lora = lora;
    runtime.set(network)
}

/// 引擎日志文件的最后若干行（出错时给用户看）。
fn log_tail(lines: usize) -> String {
    let path = engine::dir().join("server.log");
    let Ok(file) = std::fs::File::open(path) else {
        return String::new();
    };
    let reader = BufReader::new(file);
    let mut all: Vec<String> = reader.lines().map_while(Result::ok).collect();
    let start = all.len().saturating_sub(lines);
    all.drain(..start);
    all.join("\n")
}

/// 启动引擎（后台线程；状态走 `engine://yue-engine`）。
///
/// 已就绪时直接返回；正在启动时不重复拉起。
#[tauri::command]
pub fn ai_yue_engine_start(app: tauri::AppHandle, model: Option<String>) -> Result<(), String> {
    let current = status();
    let wanted = model.filter(|m| !m.trim().is_empty());
    // 换了权重就要用新权重重新加载（模型是启动参数，不能中途换）。
    if current.state == "ready" && wanted.as_deref().is_none_or(|m| current.model == m) {
        return Ok(());
    }
    if current.state == "starting" {
        return Ok(());
    }
    if current.state == "ready" {
        shutdown();
    }
    let Some(pack) = crate::yue::effective_pack() else {
        return Err("还没装 audio.cpp 引擎（下载管理器 → 运行时 → audio.cpp 引擎）".into());
    };
    if !engine::installed(pack) {
        return Err(format!("{} 没装完整，去下载管理器重装一次", pack.label));
    }
    update(|s| {
        s.state = "starting".into();
        s.message = Some("正在加载模型…".into());
        s.port = None;
        s.log.clear();
    });
    emit_engine(&app);

    let thread_app = app.clone();
    std::thread::Builder::new()
        .name("ai-daw-yue-boot".into())
        .spawn(move || {
            match boot(&thread_app, pack, wanted.as_deref()) {
                Ok(port) => {
                    update(|s| {
                        s.state = "ready".into();
                        s.port = Some(port);
                        s.message = None;
                    });
                }
                Err(message) => {
                    update(|s| {
                        s.state = "error".into();
                        s.message = Some(message);
                        s.log = log_tail(24);
                    });
                }
            }
            emit_engine(&thread_app);
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 停掉引擎（下次生成时重新拉起）。
#[tauri::command]
pub fn ai_yue_engine_stop(app: tauri::AppHandle) -> Result<(), String> {
    shutdown();
    update(|s| {
        s.state = "stopped".into();
        s.port = None;
        s.message = None;
        s.log.clear();
    });
    emit_engine(&app);
    Ok(())
}

fn shutdown() {
    let mut guard = engine_state().child.lock().unwrap();
    if let Some(mut child) = guard.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    engine_state().cancel.store(true, Ordering::Relaxed);
}

/// 下载 / 删除引擎文件前先停掉进程：Windows 上覆盖正在运行的 `.exe` / `.dll` 会
/// 共享冲突，macOS / Linux 上覆盖正在执行的二进制也不安全。
pub(crate) fn stop_for_install(app: &tauri::AppHandle) {
    shutdown();
    update(|s| {
        s.state = "stopped".into();
        s.port = None;
        s.message = None;
    });
    emit_engine(app);
}

/// 应用退出时别把子进程留下（它占着几个 GB 显存）。
pub fn shutdown_on_exit() {
    shutdown();
}

/// 一次生成用哪份权重，以及它属于哪个 audio.cpp **家族**。
struct ModelChoice {
    /// 权重 id（`yue2-q8-0` / `ace-step-turbo-q8` 这类）。
    id: String,
    /// audio.cpp 家族：`yue2` / `ace_step`。引擎配置与请求体都按它分叉。
    family: &'static str,
    /// 引擎配置里的 `path`：YuE2 是**目录**（模型 + VAE + sidecars 必须同目录），
    /// ACE-Step 是**单个 `.gguf`**（DiT + 规划器 + 文本编码器 + VAE 都装在那一个文件里）。
    path: PathBuf,
    /// 模型专属的 `session_options`；只有 YuE2 需要指名 gguf 文件。
    session: serde_json::Value,
}

/// 解析要用的模型：显式指定优先，否则按「YuE2 Q8_0 → Q4_0 → BF16 → ACE-Step」挑第一个装好的。
fn resolve_model(wanted: Option<&str>) -> Result<ModelChoice, String> {
    if let Some(id) = wanted.filter(|s| !s.trim().is_empty()) {
        let entry = catalog::by_id(id).ok_or_else(|| format!("未知模型：{id}"))?;
        return choice_of(entry)
            .ok_or_else(|| format!("「{}」还没下载完（下载管理器 → 模型）", entry.name));
    }
    const ORDER: [&str; 5] = [
        "yue2-q8-0",
        "yue2-q4-0",
        "yue2-bf16",
        "ace-step-turbo-q8",
        "ace-step-turbo-bf16",
    ];
    for id in ORDER {
        if let Some(choice) = catalog::by_id(id).and_then(choice_of) {
            return Ok(choice);
        }
    }
    Err("还没下载写歌权重（下载管理器 → 模型 → YuE2 / ACE-Step）".into())
}

/// 清单条目 → 引擎要的那几样东西；整组没就位返回 `None`。
fn choice_of(entry: &'static catalog::ModelEntry) -> Option<ModelChoice> {
    let family = catalog::compose_family(entry)?;
    // `group_dir` 要求**整组文件**都在同一个目录里：缺一个就还当没装好。
    let dir = catalog::group_dir(entry)?;
    let names: Vec<&str> = entry.files.iter().map(|f| f.filename).collect();
    if family == "ace_step" {
        // 自包含的单文件包：直接把 gguf 交给引擎（`path` 允许是一个文件）。
        let file = names.into_iter().find(|name| name.ends_with(".gguf"))?;
        return Some(ModelChoice {
            id: entry.id.to_string(),
            family,
            path: dir.join(file),
            session: serde_json::json!({}),
        });
    }
    // YuE2：模型目录 + 两个文件名（audio.cpp 拒绝绝对路径）。
    let model_file = names.iter().copied().find(|name| name.starts_with("yue2-3b-"))?;
    let vae_file = names.iter().copied().find(|name| name.starts_with("yue2-vae-"))?;
    Some(ModelChoice {
        id: entry.id.to_string(),
        family,
        path: dir,
        session: serde_json::json!({
            "yue2.model_gguf": model_file,
            "yue2.vae_gguf": vae_file,
        }),
    })
}

/// 找一个空闲端口（绑到 0 再放掉，唯一的竞争窗口在 bind 与 spawn 之间）。
pub(crate) fn free_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    drop(listener);
    Ok(port)
}

pub(crate) fn probe_agent() -> ureq::Agent {
    crate::manager::tls_agent(
        None,
        Some(Duration::from_secs(2)),
        Some(Duration::from_secs(3)),
        Some(Duration::from_secs(5)),
        None,
    )
}

/// 跑一次生成的客户端：**首响应等 1 小时**。
///
/// 服务端把整首歌算完才回 JSON（没有流式首包），CPU 上生成一首 2 分钟的歌可能要十几分钟；
/// 用默认的 30 秒「首响应」超时会在第 30 秒必然失败。
pub(crate) fn work_agent() -> ureq::Agent {
    crate::manager::tls_agent(
        None,
        Some(Duration::from_secs(5)),
        Some(Duration::from_secs(10)),
        Some(Duration::from_secs(3600)),
        None,
    )
}

/// 服务器的 `--backend` 参数（CUDA 12 / 13 都是 `cuda`，靠目录里的 DLL 区分）。
pub(crate) fn backend_arg(device: &str) -> &'static str {
    match device {
        "vulkan" => "vulkan",
        "metal" => "metal",
        "cuda12" | "cuda13" => "cuda",
        _ => "cpu",
    }
}

/// 线程数：物理核不可知，取逻辑核的一半（至少 2，最多 16）。
pub(crate) fn thread_count() -> usize {
    let logical = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    (logical / 2).clamp(2, 16)
}

/// 写配置文件（每次启动都重写：模型 / 后端 / LoRA 可能换过）。
///
/// 一次只挂**一个**模型（`max_loaded_models = 1`）：几 GB 的权重换一份就是整进程重来，
/// 换家族（YuE2 ↔ ACE-Step）也一样——所以 `ensure_ready` 只比 `model` 字段就该重启。
fn write_config(
    pack: &'static engine::EnginePack,
    model: &ModelChoice,
    port: u16,
    cuda_perf: bool,
    lora: &crate::manager::YueLora,
) -> Result<PathBuf, String> {
    let mut session = model
        .session
        .as_object()
        .cloned()
        .unwrap_or_default();
    let cuda = pack.device.starts_with("cuda");

    if model.family == "yue2" {
        // CUDA 上 audio.cpp 官方给了 `cuda_perf` 预设（packed projections + KV 对齐 + 不释放
        // 中间态），默认开；显存紧的用户在 AI 写歌页关掉。
        if cuda && cuda_perf {
            session.insert("yue2.perf_mode".into(), serde_json::json!("cuda_perf"));
        }
        // LoRA：AR 那一半管「作曲」，NAR 那一半管「音色」。只在填了路径时写进去——
        // 写空串会被当成「适配器文件不存在」。
        for (key, path, scale) in [
            ("ar", &lora.ar_path, lora.ar_scale),
            ("nar", &lora.nar_path, lora.nar_scale),
        ] {
            let path = path.trim();
            if path.is_empty() {
                continue;
            }
            session.insert(format!("yue2.{key}_lora"), serde_json::json!(path));
            session.insert(
                format!("yue2.{key}_lora_scale"),
                serde_json::json!(scale.clamp(0.0, 2.0)),
            );
        }
    } else if cuda && cuda_perf {
        // ACE-Step 的同名预设：CUDA 规划器加速 + 更大的 VAE 分块。只在 CUDA 上有效，
        // 所以 CPU / Vulkan 上不发（发了也只是被忽略，但别让配置看起来以为它在起作用）。
        session.insert("ace_step.perf_mode".into(), serde_json::json!("cuda_perf"));
    }

    let mut entry = serde_json::json!({
        // 引擎内部 id 与家族同名：`/v1/tasks/run` 的 `model` 字段要跟它逐字对上。
        "id": model.family,
        "family": model.family,
        "task": "gen",
        "mode": "offline",
        "path": model.path.to_string_lossy(),
        "session_options": session,
    });
    if model.family == "yue2" {
        // YuE2 默认先写谱；ACE-Step 没有这个开关（它压根不写谱）。
        entry["default_request_options"] = serde_json::json!({ "cot": "full" });
    }

    let config = serde_json::json!({
        "host": "127.0.0.1",
        "port": port,
        "backend": backend_arg(pack.device),
        "device": 0,
        "threads": thread_count(),
        "lazy_load": false,
        "max_loaded_models": 1,
        "ui": false,
        "models": [entry],
    });

    let path = engine::dir().join("server.json");
    let text = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("写配置失败：{e}"))?;
    Ok(path)
}

/// 拉起 `audiocpp_server` 并等到 `/health` 应答（模型加载完成才回，给 10 分钟）。
fn boot(
    app: &tauri::AppHandle,
    pack: &'static engine::EnginePack,
    wanted: Option<&str>,
) -> Result<u16, String> {
    let choice = resolve_model(wanted)?;
    let port = free_port()?;
    let cuda_perf = app
        .state::<crate::manager::StoreRuntime>()
        .network()
        .yue_cuda_perf;
    let lora = app
        .state::<crate::manager::StoreRuntime>()
        .network()
        .yue_lora;
    let config = write_config(pack, &choice, port, cuda_perf, &lora)?;
    let dir = engine::dir_of(pack);

    let log_path = engine::dir().join("server.log");
    let log = std::fs::File::create(&log_path).map_err(|e| format!("建日志文件失败：{e}"))?;
    let log_err = log.try_clone().map_err(|e| e.to_string())?;

    let mut command = Command::new(engine::executable(pack));
    command
        .current_dir(&dir)
        .arg("--config")
        .arg(&config)
        .arg("--backend")
        .arg(backend_arg(pack.device))
        .arg("--no-ui")
        .arg("--log")
        .arg("--log-file")
        .arg(&log_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 别弹一个黑框出来。
        command.creation_flags(0x0800_0000);
    }

    update(|s| {
        s.device = pack.device.to_string();
        s.model = choice.id.clone();
    });
    emit_engine(app);
    eprintln!(
        "[yue] 启动引擎：{}（{} · {}）端口 {port}",
        engine::executable(pack).display(),
        pack.device,
        choice.id
    );

    let child = command
        .spawn()
        .map_err(|e| format!("拉不起 audiocpp_server：{e}"))?;
    *engine_state().child.lock().unwrap() = Some(child);

    // 权重从磁盘进内存 + 上传显存要一阵；给 10 分钟，期间把已经加载的日志行带出去。
    let agent = probe_agent();
    let url = format!("http://127.0.0.1:{port}/health");
    let started = Instant::now();
    loop {
        {
            let mut guard = engine_state().child.lock().unwrap();
            if let Some(child) = guard.as_mut() {
                if let Ok(Some(code)) = child.try_wait() {
                    return Err(format!("引擎启动后立刻退出（{code}）"));
                }
            }
        }
        if let Ok(resp) = agent.get(&url).call() {
            if resp.status().as_u16() == 200 {
                return Ok(port);
            }
        }
        if started.elapsed() > Duration::from_secs(600) {
            return Err("引擎 10 分钟还没就绪".into());
        }
        if started.elapsed().as_secs() % 10 < 1 {
            let tail = log_tail(3);
            update(|s| {
                s.state = "starting".into();
                s.message = Some(if tail.is_empty() {
                    format!("正在加载模型…（{} 秒）", started.elapsed().as_secs())
                } else {
                    tail
                });
                s.log = log_tail(12);
            });
            emit_engine(app);
        }
        std::thread::sleep(Duration::from_millis(700));
    }
}

/// 一组采样旋钮（谱规划 / 语义码各一套，字段一一对应 audio.cpp 的 `abc_*` / `semantic_*`）。
///
/// 协议里有 14 个：温度、top_p、top_k、重复惩罚、惩罚窗、最少/最多 token。
/// 界面上一个都不填 = 全部用引擎默认（v0.9.3 的默认值对大多数歌够用）。
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct SamplingArgs {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub repetition_penalty: Option<f32>,
    pub penalty_window: Option<u32>,
    pub min_tokens: Option<u32>,
    pub max_tokens: Option<u32>,
}

impl SamplingArgs {
    fn is_empty(&self) -> bool {
        self.temperature.is_none()
            && self.top_p.is_none()
            && self.top_k.is_none()
            && self.repetition_penalty.is_none()
            && self.penalty_window.is_none()
            && self.min_tokens.is_none()
            && self.max_tokens.is_none()
    }

    /// 按协议的取值范围夹一遍（越界会被服务端整单判 400，不如先夹住）。
    fn write_into(&self, options: &mut serde_json::Value, prefix: &str) {
        if let Some(v) = self.temperature {
            options[format!("{prefix}_temperature")] = serde_json::json!(v.clamp(0.0, 5.0));
        }
        if let Some(v) = self.top_p {
            // 协议要求 (0, 1]。
            options[format!("{prefix}_top_p")] = serde_json::json!(v.clamp(0.001, 1.0));
        }
        if let Some(v) = self.top_k {
            options[format!("{prefix}_top_k")] = serde_json::json!(v.max(1));
        }
        if let Some(v) = self.repetition_penalty.filter(|v| *v > 0.0) {
            options[format!("{prefix}_repetition_penalty")] = serde_json::json!(v);
        }
        if let Some(v) = self.penalty_window {
            options[format!("{prefix}_penalty_window")] = serde_json::json!(v.max(1));
        }
        if let Some(v) = self.min_tokens {
            options[format!("{prefix}_min_tokens")] = serde_json::json!(v);
        }
        if let Some(v) = self.max_tokens {
            options[format!("{prefix}_max_tokens")] = serde_json::json!(v);
        }
    }
}

/// 一次生成的参数（界面 / MCP 共用）。
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct ComposeArgs {
    /// 风格标签（逗号分隔，例如 `pop, female vocal, city pop`）。
    pub style: String,
    /// 歌词（带 `[Verse]` / `[Chorus]` 这类结构标签）。留空 = 纯器乐。
    pub lyrics: String,
    /// 直接给一段 ABC 谱（空 = 让模型自己写）。
    pub abc: String,
    /// 从文件读 ABC 谱（与 `abc` 二选一；`abc` 优先）。
    pub abc_file: Option<String>,
    /// 目标时长（秒；默认 120，夹到 15..=600）。换算成语义 token 上限（25 帧/秒）。
    pub duration: Option<f32>,
    /// 流匹配步数（默认 8，夹到 1..=64）。
    pub steps: Option<u32>,
    /// 种子（-1 = 随机；>=0 可复现）。
    pub seed: Option<i64>,
    /// 结构模式：`full`（带和弦的谱）/ `melody`（只要旋律）/ `off`（跳过符号阶段）。
    pub cot: Option<String>,
    /// 生成停在哪一阶段：`abc`（只写谱）/ `semantic`（谱 + 语义码）/ `audio`（整首，默认）。
    pub stop_after: Option<String>,
    /// 把语义码作为 artifact 附在结果里（续写 / 复现的前提）。
    pub export_semantic: Option<bool>,
    /// 语义码前缀（JSON 数组文本）：这段码当开头，往后长 = 续写。
    pub semantic_prefix: Option<String>,
    /// 语义码前缀文件（与 `semantic_prefix` 二选一，避免把几万个数字塞进请求）。
    pub semantic_prefix_file: Option<String>,
    /// 谱规划的采样旋钮。
    pub abc_sampling: Option<SamplingArgs>,
    /// 语义码的采样旋钮。
    pub semantic_sampling: Option<SamplingArgs>,
    /// CFG：负值 = 用协议默认（`off` 模式 1.01，其余 1.0；1.0 等于单分支、解码快一倍）。
    pub cfg_scale: Option<f32>,
    /// 指定的权重 id；省略 = 用已安装的默认档。
    pub model: Option<String>,
    /// 插到哪条轨道旁边。语义：
    /// - 普通生成：**新建一条轨**插到这条轨道下面（省略 = 追加到工程末尾）；
    /// - 续写（带了 `semantic_prefix`）：把新片段**接在这条轨道最后一个音频片段的末尾**。
    pub after_track: Option<u64>,
    /// 新轨道的名字。
    pub name: Option<String>,
}

impl ComposeArgs {
    fn duration(&self) -> f32 {
        self.duration.unwrap_or(120.0).clamp(15.0, 600.0)
    }

    fn steps(&self) -> u32 {
        self.steps.unwrap_or(8).clamp(1, 64)
    }

    fn seed(&self) -> u64 {
        let raw = self.seed.unwrap_or(-1);
        if raw >= 0 {
            return raw as u64;
        }
        // 负数 = 随机：拿纳秒当种子（和时间戳一样能复现，只要记下来）。
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        nanos % (1 << 52)
    }

    fn cot(&self) -> &'static str {
        let given = self.cot.as_deref().unwrap_or("full");
        let has_abc = !self.abc.trim().is_empty() || self.abc_file.is_some();
        match given {
            // 给了外部谱就必须走符号路线；`off` 会直接忽略 abc，等于白给。
            "off" if has_abc => "full",
            "off" => "off",
            "melody" => "melody",
            _ => "full",
        }
    }

    /// 停在哪个阶段。未知值按 `audio`（最常用的那条路，出错也别把人挡在门外）。
    fn stop_after(&self) -> &'static str {
        match self.stop_after.as_deref().unwrap_or("audio") {
            "abc" => "abc",
            "semantic" => "semantic",
            _ => "audio",
        }
    }

    /// 要不要语义码：显式打开，或者本来就要停在 semantic（那一步没码等于白跑）。
    fn export_semantic(&self) -> bool {
        self.export_semantic.unwrap_or(false) || self.stop_after() == "semantic"
    }
}

/// 这次要跑哪个家族：显式选了权重就按那一档，否则按引擎已经加载的那份，最后兜底 YuE2
/// （历史行为：没装别的之前，写歌就是 YuE2）。
fn family_of(args: &ComposeArgs) -> &'static str {
    let id = args
        .model
        .as_deref()
        .filter(|m| !m.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| status().model);
    catalog::by_id(&id)
        .and_then(catalog::compose_family)
        .unwrap_or("yue2")
}

/// 组装 `/v1/tasks/run` 的请求体：形状由家族决定。
fn request_body(args: &ComposeArgs, family: &str) -> serde_json::Value {
    match family {
        "ace_step" => ace_step_body(args),
        _ => yue2_body(args),
    }
}

/// ACE-Step 的请求体：**描述**（顶层 `text`）+ 歌词 + 几个通用旋钮（全在 `options` 里）。
///
/// 两处与 YuE2 不同（按 v0.9.3 的 `ace_step_parse_request` 逐字对过）：
/// - **描述走顶层 `text`**，空着引擎直接回 `ACE-Step requires text_input`；
/// - `lyrics` / `seed` / `duration_sec` / `num_inference_steps` 都从 `options` 读（YuE2 的
///   `lyrics` 在顶层）。
///
/// Turbo 档不看 `guidance_scale`；用户自己填了（>= 0）才发，免得把默认值改成别的语义。
fn ace_step_body(args: &ComposeArgs) -> serde_json::Value {
    let mut options = serde_json::json!({
        "route": "text2music",
        // Turbo 的上限就是 8 步：发更大的值不会更细，只会让人以为「调了有用」。
        "num_inference_steps": args.steps().min(8),
        "duration_sec": args.duration(),
        // 引擎侧是 u32；`seed()` 给的是 u64（纳秒余数），取低 32 位即可。
        "seed": (args.seed() % u32::MAX as u64) as u32,
    });
    if !args.lyrics.trim().is_empty() {
        options["lyrics"] = serde_json::json!(args.lyrics);
    }
    if let Some(cfg) = args.cfg_scale.filter(|c| *c >= 0.0) {
        options["guidance_scale"] = serde_json::json!(cfg);
    }
    serde_json::json!({
        "model": "ace_step",
        "request": {
            "text": args.style,
            "options": options,
        },
    })
}

/// 组装 YuE2 的 `/v1/tasks/run` 请求体。
///
/// 实测得来的两条硬约束（v0.9.3）：
/// - 模型专属选项（`style` / `cot` / `abc` / `num_inference_steps` / `semantic_*` …）必须
///   放在 **`options`** 里。放顶层时服务端读不到 `style`，报「Yue2 requires non-empty style」。
/// - `lyrics` / `seed` 是通用字段，放顶层；`semantic_min_tokens` 默认 200，`semantic_max_tokens`
///   比它小会被判「invalid」——所以只设上限、不动下限（除非时长换算出来的比 200 还小）。
fn yue2_body(args: &ComposeArgs) -> serde_json::Value {
    let mut options = serde_json::json!({
        "style": args.style,
        "cot": args.cot(),
        "num_inference_steps": args.steps(),
        // 25 帧/秒：界面给「时长」、协议给「最多多少帧」。
        "export_semantic": args.export_semantic(),
        "stop_after": args.stop_after(),
    });

    // 语义码长度：显式给了 max_tokens 就以它为准，否则按时长换算；再兜住 min 的下限。
    let semantic = args.semantic_sampling.clone().unwrap_or_default();
    let max_tokens = semantic
        .max_tokens
        .unwrap_or_else(|| (args.duration() * SEMANTIC_FPS).round() as u32);
    let min_tokens = semantic.min_tokens.unwrap_or(200);
    options["semantic_max_tokens"] = serde_json::json!(max_tokens.max(min_tokens));
    if min_tokens != 200 {
        options["semantic_min_tokens"] = serde_json::json!(min_tokens);
    }

    if !args.abc.trim().is_empty() {
        options["abc"] = serde_json::json!(args.abc);
    } else if let Some(file) = args.abc_file.as_deref().filter(|f| !f.trim().is_empty()) {
        options["abc_file"] = serde_json::json!(file);
    }
    if let Some(prefix) = args
        .semantic_prefix
        .as_deref()
        .filter(|p| !p.trim().is_empty())
    {
        options["semantic_prefix"] = serde_json::json!(prefix);
    } else if let Some(file) = args
        .semantic_prefix_file
        .as_deref()
        .filter(|f| !f.trim().is_empty())
    {
        options["semantic_prefix_file"] = serde_json::json!(file);
    }
    if let Some(cfg) = args.cfg_scale.filter(|c| *c >= 0.0) {
        options["guidance_scale"] = serde_json::json!(cfg);
    }

    let abc_sampling = args.abc_sampling.clone().unwrap_or_default();
    if !abc_sampling.is_empty() {
        abc_sampling.write_into(&mut options, "abc");
    }
    // 语义码那套（温度/top_p/…）照写，但 min/max 已经在上面按「下限优先」处理过了，
    // 再写一遍会把兜底覆盖掉，所以只写采样相关的五项。
    let mut semantic_sampling = semantic;
    semantic_sampling.min_tokens = None;
    semantic_sampling.max_tokens = None;
    if !semantic_sampling.is_empty() {
        semantic_sampling.write_into(&mut options, "semantic");
    }

    serde_json::json!({
        "model": "yue2",
        "request": {
            "lyrics": args.lyrics,
            "seed": args.seed(),
            "options": options,
        }
    })
}

/// 把「语义码文件」读进来换成内联前缀，并锁死续写的路线。
///
/// 引擎对这个字段很挑：内容必须是**纯 JSON 数字数组**，文件开头一个 UTF-8 BOM 就会报
/// 「failed to parse json at byte 0」（实测）。文本编辑器另存的文件经常带 BOM，所以这里
/// 统一读一遍、去掉 BOM、验一遍类型，再以内联 `semantic_prefix` 发出去——码再长也就
/// 一百多 KB，比让用户去猜编码划算。
///
/// 还有一条硬约束：**前缀只在「不重新规划谱」或「手里有谱」时才被接受**
/// （服务端报 `semantic prefix requires abc or abc_file unless cot=off`）。没有外部谱时
/// 这里直接把 `cot` 钉成 `off`——续写就是接着已有的码往下长，重新规划一份谱会让
/// 接出来的段落和原曲脱节（那正是「续写像另一首歌」的来源之一）。
fn normalize_prefix(args: &mut ComposeArgs) -> Result<(), String> {
    if let Some(raw_path) = args.semantic_prefix_file.take() {
        let path = raw_path.trim();
        if !path.is_empty() {
            let raw = std::fs::read_to_string(path).map_err(|e| format!("读语义码文件失败：{e}"))?;
            let text = raw.trim_start_matches('\u{feff}').trim();
            let value: serde_json::Value = serde_json::from_str(text)
                .map_err(|e| format!("语义码文件不是合法 JSON（{e}）；应该是一段数字数组"))?;
            let Some(items) = value.as_array() else {
                return Err("语义码文件应该是一段 JSON 数组，例如 [12046, 4196, …]".into());
            };
            if items.is_empty() {
                return Err("语义码文件是空的".into());
            }
            if items.iter().any(|v| !v.is_u64()) {
                return Err("语义码文件里有非整数项（应该全是 codec 索引）".into());
            }
            args.semantic_prefix = Some(text.to_string());
        }
    }

    // 续写（内联或文件）但没有外部谱：强制 `cot=off`，引擎才会把前缀当开头往后长。
    let has_prefix = args
        .semantic_prefix
        .as_deref()
        .is_some_and(|p| !p.trim().is_empty());
    if has_prefix && args.abc.trim().is_empty() && args.abc_file.is_none() {
        args.cot = Some("off".into());
    }
    Ok(())
}

/// 这次请求是不是「续写」（带了语义码前缀）。
fn is_extension(args: &ComposeArgs) -> bool {
    args.semantic_prefix
        .as_deref()
        .is_some_and(|p| !p.trim().is_empty())
        || args
            .semantic_prefix_file
            .as_deref()
            .is_some_and(|f| !f.trim().is_empty())
}

/// 续写前缀在成品音频里占多少帧。
///
/// 语义码是 25 帧/秒，与成品采样率无关：`codes × 每帧音频帧数`。截到总帧数以内，
/// 免得引擎提前 EOS（码比请求的短）时算出越界的裁剪点。
fn prefix_frames_for(codes: usize, sample_rate: f64, total_frames: u64) -> u64 {
    let prefix_secs = codes as f64 / SEMANTIC_FPS as f64;
    ((prefix_secs * sample_rate).round() as u64).min(total_frames)
}

/// 续写只对 YuE2 有意义。
///
/// ACE-Step 的请求体里根本没有 `semantic_prefix` 字段：前缀会被**静默忽略**，结果就是
/// 「拿着原曲的时长又写了一首完全无关的歌」。这类请求必须在提交前挡掉，而不是让它跑几分钟。
fn reject_prefix_outside_yue2(args: &ComposeArgs, family: &str) -> Result<(), String> {
    if is_extension(args) && family != "yue2" {
        return Err("续写只能用 YuE2 权重：当前选的是 ACE-Step，它不支持语义码前缀".into());
    }
    Ok(())
}

/// 每秒把 `elapsed` 推给界面。服务端是「一次请求跑到底」，没有进度回调；
/// 没有这个线程，UI 会一直停在「已 0 秒」，看起来像卡死。
fn spawn_job_ticker(app: tauri::AppHandle, started: Instant) {
    let _ = std::thread::Builder::new()
        .name("ai-daw-yue-tick".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(500));
            if !engine_state().busy.load(Ordering::SeqCst)
                || engine_state().cancel.load(Ordering::Relaxed)
            {
                break;
            }
            emit_job(
                &app,
                &JobStatus {
                    state: "running".into(),
                    elapsed: started.elapsed().as_secs_f32(),
                    ..Default::default()
                },
            );
        });
}

/// 提交一次生成（后台跑；进度与结果走 `engine://yue-job`）。
///
/// 引擎没就绪也不报错：任务线程里先把它拉起来再提交（见 [`ensure_ready`]）。
#[tauri::command]
pub fn ai_yue_compose(app: tauri::AppHandle, args: ComposeArgs) -> Result<(), String> {
    if args.style.trim().is_empty() && args.lyrics.trim().is_empty() {
        return Err("风格和歌词至少给一样".into());
    }
    // ACE-Step 的提示词就是那句描述本身：只有歌词没有描述，引擎会直接回 400。
    if family_of(&args) == "ace_step" && args.style.trim().is_empty() {
        return Err("ACE-Step 需要一句音乐描述（它会按这句定风格）".into());
    }
    // 续写只对 YuE2 有意义：ACE-Step 会静默忽略前缀，等于又写了一首无关的歌。
    reject_prefix_outside_yue2(&args, family_of(&args))?;
    if engine_state().busy.swap(true, Ordering::SeqCst) {
        return Err("上一次生成还在跑".into());
    }
    engine_state().cancel.store(false, Ordering::Relaxed);

    let started = Instant::now();
    spawn_job_ticker(app.clone(), started);
    emit_job(
        &app,
        &JobStatus {
            state: "running".into(),
            ..Default::default()
        },
    );

    let thread_app = app.clone();
    std::thread::Builder::new()
        .name("ai-daw-yue-job".into())
        .spawn(move || {
            let guard_app = thread_app.clone();
            // 模型 id 先取出来：加载引擎要用它，`args` 之后整体交给任务。
            let model = args.model.clone();
            let outcome = crate::job::guard(|| {
                let port = ensure_ready(&guard_app, model)?;
                run_compose_job(&guard_app, port, args, started)
            });
            if let Err(message) = outcome {
                engine_state().busy.store(false, Ordering::SeqCst);
                // 取消可能发生在「还在加载引擎」的时候：那条事件已经由 cancel 发过了，
                // 这里别再补一条 failed 把它盖掉。
                if engine_state().cancel.load(Ordering::Relaxed) {
                    return;
                }
                emit_job(
                    &thread_app,
                    &JobStatus {
                        state: "failed".into(),
                        message: Some(message),
                        ..Default::default()
                    },
                );
            }
        })
        .map_err(|e| {
            engine_state().busy.store(false, Ordering::SeqCst);
            e.to_string()
        })?;
    Ok(())
}

/// 一次生成的实际工作（放在 `job::guard` 里跑，panic 会变成 failed 事件）。
fn run_compose_job(
    app: &tauri::AppHandle,
    port: u16,
    mut args: ComposeArgs,
    started: Instant,
) -> Result<String, String> {
    // 引擎已经就绪：`status().model` 就是这次真正跑的那一档，家族由它决定。
    let family = family_of(&args);
    reject_prefix_outside_yue2(&args, family)?;
    // 语义码前缀是 YuE2 专有的：别的家族没有「续写」这条路，提前挡掉只会误伤。
    if family == "yue2" {
        normalize_prefix(&mut args)?;
    }
    let body = request_body(&args, family);
    let result = submit(port, &body);
    engine_state().busy.store(false, Ordering::SeqCst);
    if engine_state().cancel.load(Ordering::Relaxed) {
        // 取消那条事件已经在 `ai_yue_compose_cancel` 里发过了。
        return Ok(String::new());
    }
    match result {
        Ok(run) => match collect(app, &args, &body, run, started.elapsed(), family) {
            Ok(job) => emit_job(app, &job),
            Err(message) => emit_job(
                app,
                &JobStatus {
                    state: "failed".into(),
                    message: Some(message),
                    ..Default::default()
                },
            ),
        },
        Err(message) => emit_job(
            app,
            &JobStatus {
                state: "failed".into(),
                message: Some(message),
                ..Default::default()
            },
        ),
    }
    Ok(String::new())
}

/// 同步写歌（MCP / 无界面调用）：必要时拉起引擎，等就绪，提交并等结果入轨。
pub(crate) fn compose_blocking(
    app: &tauri::AppHandle,
    args: ComposeArgs,
) -> Result<String, String> {
    // 与界面的 `ai_yue_compose` 共用同一个 busy 位：既挡住并发生成，也让
    // `cancel_ai_task` 能看出「现在真的在写歌」（取消 = 停引擎，服务端没有取消端点）。
    if engine_state().busy.swap(true, Ordering::SeqCst) {
        return Err("已有一个写歌任务在跑".into());
    }
    engine_state().cancel.store(false, Ordering::Relaxed);
    let result = compose_blocking_inner(app, args);
    engine_state().busy.store(false, Ordering::SeqCst);
    result
}

/// 引擎就绪就返回端口；没起（或权重换了）就顺手拉起来并等它加载完。
///
/// 「生成」是用户眼里唯一那一步：转谱 / 听写为了腾显存会把引擎停掉，翻唱一口气跑下来
/// 时必须能自己把引擎点着，而不是甩一句「先点启动引擎」再让用户点两次。
fn ensure_ready(app: &tauri::AppHandle, model: Option<String>) -> Result<u16, String> {
    let wanted = model.filter(|m| !m.trim().is_empty());
    let current = status();
    let same_model =
        current.state == "ready" && wanted.as_deref().is_none_or(|model| current.model == model);
    if same_model {
        return current.port.ok_or_else(|| "引擎没有端口".to_string());
    }
    ai_yue_engine_start(app.clone(), wanted)?;
    let deadline = Instant::now() + Duration::from_secs(600);
    loop {
        std::thread::sleep(Duration::from_millis(500));
        if engine_state().cancel.load(Ordering::Relaxed) {
            return Err("已取消：引擎已停止".into());
        }
        let state = status();
        match state.state.as_str() {
            "ready" => return state.port.ok_or_else(|| "引擎没有端口".to_string()),
            "error" => {
                return Err(state
                    .message
                    .or_else(|| state.log.lines().last().map(str::to_string))
                    .unwrap_or_else(|| "引擎启动失败".into()));
            }
            _ if Instant::now() > deadline => return Err("引擎 10 分钟还没就绪".into()),
            _ => {}
        }
    }
}

fn compose_blocking_inner(app: &tauri::AppHandle, mut args: ComposeArgs) -> Result<String, String> {
    if args.style.trim().is_empty() && args.lyrics.trim().is_empty() {
        return Err("风格和歌词至少给一样".into());
    }
    // 先把引擎拉到「就绪」：家族要看它到底加载了哪一档（`args.model` 没给时尤其如此）。
    let port = ensure_ready(app, args.model.clone())?;
    if engine_state().cancel.load(Ordering::Relaxed) {
        return Err("已取消：引擎已停止".into());
    }
    let family = family_of(&args);
    reject_prefix_outside_yue2(&args, family)?;
    if family == "yue2" {
        normalize_prefix(&mut args)?;
    }
    let started = Instant::now();
    let body = request_body(&args, family);
    let run = submit(port, &body)?;
    if engine_state().cancel.load(Ordering::Relaxed) {
        return Ok("已取消".into());
    }
    let job = collect(app, &args, &body, run, started.elapsed(), family)?;
    Ok(job.message.unwrap_or_else(|| "已生成".into()))
}

/// 取消当前生成：服务端没有取消端点，只能**停引擎**（权重还在显存里，杀掉最干净）。
#[tauri::command]
pub fn ai_yue_compose_cancel(app: tauri::AppHandle) {
    engine_state().cancel.store(true, Ordering::Relaxed);
    shutdown();
    update(|s| {
        s.state = "stopped".into();
        s.port = None;
        s.message = Some("已取消：引擎已停止，下次生成会重新加载".into());
    });
    emit_engine(&app);
    emit_job(
        &app,
        &JobStatus {
            state: "cancelled".into(),
            message: Some("已取消".into()),
            ..Default::default()
        },
    );
}

/// 提交并等结果。请求体由调用方先组好——**种子只在这里算一次**，否则「记录里的 seed」
/// 和「真正用的 seed」会是两个随机数，复现就断了。
fn submit(port: u16, body: &serde_json::Value) -> Result<RunResult, String> {
    let agent = work_agent();
    let body = body.to_string();
    let resp = agent
        .post(&format!("http://127.0.0.1:{port}/v1/tasks/run"))
        .header("content-type", "application/json")
        .send(body.as_bytes())
        .map_err(|e| format!("提交任务失败：{e}"))?;
    let status = resp.status().as_u16();
    let mut text = String::new();
    resp.into_body()
        .into_reader()
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if status != 200 {
        return Err(format!("引擎回了 HTTP {status}：{}", brief(&text)));
    }
    parse_result(&text)
}

/// 一次生成的结果：三个阶段各自可选（`stop_after` 决定跑到哪）。
#[derive(Debug, Default)]
struct RunResult {
    audio: Option<Vec<u8>>,
    score: Option<String>,
    semantic: Option<String>,
    /// 谱被 `abc_max_tokens` 截断了（引擎在 meta 里标 `truncated: "true"`）。
    score_truncated: bool,
}

/// 从 `/v1/tasks/run` 的 JSON 里拆出音频 / 谱面 / 语义码。
///
/// `stop_after=abc` 时**没有** `audio` 字段，只有 artifacts——所以这里不能把音频当必需项。
fn parse_result(text: &str) -> Result<RunResult, String> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| format!("引擎回了怪东西：{e}（{}）", brief(text)))?;
    if let Some(message) = value
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
    {
        return Err(format!("引擎报错：{message}"));
    }

    let audio = match value
        .get("audio")
        .and_then(|a| a.as_str())
        .filter(|s| !s.is_empty())
    {
        Some(encoded) => {
            let bytes = decode_base64(encoded).map_err(|e| format!("音频解码失败：{e}"))?;
            if !bytes.starts_with(b"RIFF") {
                return Err(format!(
                    "引擎给的音频不是 WAV（前 {:?}）",
                    &bytes[..bytes.len().min(4)]
                ));
            }
            Some(bytes)
        }
        None => None,
    };

    // 谱面：`meta.format = "abc"` 的 artifact（`kind` 实测是 `custom`，`id` 是 `score`）。
    // 语义码：mime `application/vnd.yue2.semantic+json` / `kind = acoustic_tokens`。
    let mut result = RunResult {
        audio,
        ..Default::default()
    };
    let empty = Vec::new();
    for item in value
        .get("artifacts")
        .and_then(|a| a.as_array())
        .unwrap_or(&empty)
    {
        let text_of = |key: &str| item.get(key).and_then(|v| v.as_str()).unwrap_or_default();
        let meta = |key: &str| {
            item.get("meta")
                .and_then(|m| m.get(key))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
        };
        let kind = text_of("kind");
        let id = text_of("id");
        let (format, extension, mime) = (meta("format"), meta("extension"), meta("mime"));

        let is_score = kind == "score"
            || id == "score"
            || id == "abc"
            || format.contains("abc")
            || extension == "abc"
            || mime.contains("abc");
        let is_semantic = kind == "acoustic_tokens"
            || id.contains("semantic")
            || format.contains("semantic")
            || mime.contains("semantic");

        // 二进制 MIDI 不是 ABC：宁可当没有，也别把乱码灌进谱面框。
        let Some(payload) = item
            .get("payload")
            .and_then(|p| p.as_str())
            .and_then(|p| decode_base64(p).ok())
        else {
            continue;
        };
        if payload.starts_with(b"MThd") {
            continue;
        }
        let Ok(payload) = String::from_utf8(payload) else {
            continue;
        };
        if payload.trim().is_empty() {
            continue;
        }
        if is_score && result.score.is_none() {
            result.score = Some(payload);
            result.score_truncated = meta("truncated") == "true";
        } else if is_semantic && result.semantic.is_none() {
            result.semantic = Some(payload);
        }
    }

    if result.audio.is_none() && result.score.is_none() && result.semantic.is_none() {
        return Err(format!("引擎没有给音频、谱面或语义码（{}）", brief(text)));
    }
    Ok(result)
}

fn brief(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= 300 {
        return text.to_string();
    }
    text.chars().take(300).collect::<String>() + "…"
}

fn decode_base64(text: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    // 兼容 `data:audio/wav;base64,` 前缀。
    let payload = text.split_once(',').map(|(_, rest)| rest).unwrap_or(text);
    base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .map_err(|e| e.to_string())
}

/// 默认轨道名：按家族起名，用户一眼能看出这首是谁写的。
fn default_track_name(family: &str, args: &ComposeArgs) -> String {
    args.name
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| match family {
            "ace_step" => "ACE-Step 写歌".to_string(),
            _ => "YuE2 写歌".to_string(),
        })
}

/// 把音频 / 谱面 / 语义码落盘（音频另并入工程），并写一份**参数存档**。
///
/// `stop_after=abc|semantic` 时没有音频：只落盘，不碰工程——那一步的产物就是谱/码本身。
/// ACE-Step 不写谱也没有语义码，所以它永远是「只有音频」那一路。
fn collect(
    app: &tauri::AppHandle,
    args: &ComposeArgs,
    body: &serde_json::Value,
    run: RunResult,
    elapsed: Duration,
    family: &str,
) -> Result<JobStatus, String> {
    let base = output_path(app, &family.replace('_', "-"))?;
    let stem = PathBuf::from(&base);

    let mut score_path = None;
    if let Some(abc) = run.score.as_deref() {
        let path = stem.with_extension("abc");
        std::fs::write(&path, abc).map_err(|e| format!("写谱面失败：{e}"))?;
        score_path = Some(path.to_string_lossy().into_owned());
    }
    let mut semantic_path = None;
    if let Some(semantic) = run.semantic.as_deref() {
        let path = stem.with_extension("semantic.json");
        std::fs::write(&path, semantic).map_err(|e| format!("写语义码失败：{e}"))?;
        semantic_path = Some(path.to_string_lossy().into_owned());
    }

    let state = app.state::<AppState>();
    let mode = args.stop_after();
    let truncated = run.score_truncated;
    let mut audio_path = None;
    let mut seconds = None;
    // 这次续写新长出来的秒数（`None` = 不是续写）。
    let mut appended: Option<f32> = None;
    // 新片段接在了哪条轨道上（只有续写会填）。
    let mut appended_to: Option<String> = None;

    if let Some(bytes) = run.audio {
        let wav = stem.with_extension("wav");
        std::fs::write(&wav, &bytes).map_err(|e| format!("写音频失败：{e}"))?;

        // 解一遍拿到时长（48 kHz 立体声），顺便进素材缓存。
        let wav_text = wav.to_string_lossy().into_owned();
        let decoded = state.assets.load(&wav_text)?;
        let sample_rate = decoded.sample_rate.max(1.0);
        let total_frames = decoded.frames() as u64;
        let render_secs = total_frames as f64 / sample_rate;

        // 续写：引擎把「前缀 + 新长出来的部分」**一起**渲染（`semantic_prefix` 是强制开头的码），
        // 所以文件最前面那 `prefix` 秒是原曲的重新渲染。接在原曲后面时必须把这一截裁掉，
        // 否则就是把原曲又叠了一遍——听感上「续写和原曲各说各话」有一半来自这里。
        let extension = args
            .semantic_prefix
            .as_deref()
            .and_then(|text| serde_json::from_str::<Vec<i64>>(text).ok())
            .filter(|codes| !codes.is_empty());
        let prefix_frames = extension
            .as_ref()
            .map(|codes| prefix_frames_for(codes.len(), sample_rate, total_frames))
            .unwrap_or(0);
        let window_frames = total_frames.saturating_sub(prefix_frames);
        let appended_secs = window_frames as f64 / sample_rate;
        if extension.is_some() && appended_secs < 0.05 {
            return Err("引擎只重渲染了前缀，没有长出新内容：把「加长」调大一点再试".into());
        }

        // 工程按拍记时间：换算成当前速度下的拍数（生成结果本身不跟速度走）。
        let tempo = {
            let engine = state.engine.lock().unwrap();
            engine.project().transport.tempo
        };
        let beats = appended_secs * tempo / 60.0;

        let name = default_track_name(family, args);

        // 谱面在生成时只落盘，**不自动解析成 MIDI 轨**。
        //
        // YuE2 的 ABC 是模型为了“先写旋律再唱”临时哼出来的草稿：音高/时值离能用的编曲差得
        // 很远，铺进工程只会多出一堆没用的轨（用户的原话是“没有什么用”）。需要的人可以点
        // 「把 ABC 谱导入工程」显式导入——那才是“我要它”的表达。
        let source = AudioSource {
            path: wav_text.clone(),
            offset_frames: prefix_frames,
            // 续写只保留新长出来的那一段；普通生成沿用整份素材（`None` = 到素材结尾）。
            frames: extension.as_ref().map(|_| window_frames),
            rate: 1.0,
            pitch_semitones: 0.0,
        };

        // 续写要给「接在哪」找一个锚点；不是续写 / 找不到锚点就是 `None`，走新建轨道。
        let append_anchor = if extension.is_some() {
            // 目标轨道 = 界面指过来的那条（续写时 `after_track` 指向被续写的轨道），
            // 新片段接在它**最后一个音频片段**的末尾：接着放就是连着的一整首。
            let engine = state.engine.lock().unwrap();
            args.after_track.and_then(|id| {
                let project = engine.project();
                let track = project.tracks.iter().find(|track| track.id.0 == id)?;
                let clip = track.clips.iter().rev().find(|c| c.audio.is_some())?;
                Some((track.id, clip.start + clip.length, track.name.clone()))
            })
        } else {
            None
        };

        let is_extension = extension.is_some();
        let mut commands = Vec::new();
        if let Some((track, start, track_name)) = append_anchor {
            appended_to = Some(track_name);
            commands.push(EngineCommand::AddAudioClip {
                track,
                start,
                length: beats.max(0.001),
                source,
            });
        } else {
            // 不是续写，或续写找不到锚点（例如 MCP 只给了前缀、没给 `after_track`）：
            // 退回「新建一条轨」的老行为，仍然只摆新长出来的那一段。
            let anchor = {
                let engine = state.engine.lock().unwrap();
                args.after_track.and_then(|id| {
                    engine
                        .project()
                        .tracks
                        .iter()
                        .position(|track| track.id.0 == id)
                })
            };
            let insert_at = anchor.map(|index| index + 1);
            commands.push(EngineCommand::AddAudioTrack {
                name: if is_extension {
                    format!("{name}（续写）")
                } else {
                    name
                },
                start: 0.0,
                length: if is_extension {
                    beats.max(0.001)
                } else {
                    beats.max(1.0)
                },
                source,
                at: insert_at,
            });
        }

        crate::commands::run_agent_all(app, commands);
        audio_path = Some(wav_text);
        seconds = Some(render_secs as f32);
        if is_extension {
            appended = Some(appended_secs as f32);
        }
    }

    let record = write_record(
        app,
        &base,
        args,
        body,
        audio_path.as_deref(),
        score_path.as_deref(),
        semantic_path.as_deref(),
        seconds,
        elapsed,
        family,
    )?;

    let message = match mode {
        "abc" => match score_path.as_deref() {
            // 截断是常态（15 秒的谱也会撞上限）：说清楚，别让人以为是模型写错了。
            Some(_) if truncated => format!(
                "已写出谱面（用时 {:.0} 秒，**被 token 上限截断**）：调大谱规划的「最多 token」能得到更完整的谱",
                elapsed.as_secs_f32()
            ),
            Some(_) => format!(
                "已写出谱面（用时 {:.0} 秒）：可以直接改，再「按谱演唱」",
                elapsed.as_secs_f32()
            ),
            None => "引擎跑完了，但没给谱面（可能撞上了 token 上限，调大 abc_max_tokens 再试）"
                .to_string(),
        },
        "semantic" => {
            let mut names: Vec<&str> = Vec::new();
            if score_path.is_some() {
                names.push("谱面");
            }
            if semantic_path.is_some() {
                names.push("语义码");
            }
            format!(
                "已写出{}（用时 {:.0} 秒）",
                if names.is_empty() {
                    "空结果".to_string()
                } else {
                    names.join(" + ")
                },
                elapsed.as_secs_f32()
            )
        }
        _ => {
            let name = default_track_name(family, args);
            let extra = match (&score_path, &semantic_path) {
                (Some(_), Some(_)) => "，并在音频旁留了 ABC 谱与语义码",
                (Some(_), None) => "，并在音频旁留了一份 ABC 谱（需要时可在写歌页导入）",
                (None, Some(_)) => "，并在音频旁留了语义码（可用它续写）",
                (None, None) => "",
            };
            match appended {
                // 续写：说清楚「接在哪、加了多久」，别让人以为又生成了一整首。
                Some(new_secs) => match appended_to.as_deref() {
                    Some(track) => format!(
                        "已在「{track}」末尾接上 {new_secs:.0} 秒（整条 {:.0} 秒，用时 {:.0} 秒）{extra}",
                        seconds.unwrap_or(0.0),
                        elapsed.as_secs_f32()
                    ),
                    None => format!(
                        "已生成续写片段：新增 {new_secs:.0} 秒（用时 {:.0} 秒）{extra}",
                        elapsed.as_secs_f32()
                    ),
                },
                None => format!(
                    "已生成「{name}」：{:.0} 秒（用时 {:.0} 秒）{extra}",
                    seconds.unwrap_or(0.0),
                    elapsed.as_secs_f32()
                ),
            }
        }
    };

    Ok(JobStatus {
        state: "done".into(),
        elapsed: elapsed.as_secs_f32(),
        message: Some(message),
        audio: audio_path,
        score: score_path,
        semantic: semantic_path,
        record: Some(record),
        seconds,
    })
}

/// 写一份生成记录（`<产物>.json`）：参数 + 真正用的种子 + 各产物路径。
///
/// 「重放」靠它：把 `args` 原样塞回表单，种子用记录里那个，就能复现同一条结果；
/// 要续写时拿 `semantic` 当 `semantic_prefix_file`。
#[allow(clippy::too_many_arguments)]
fn write_record(
    app: &tauri::AppHandle,
    base: &str,
    args: &ComposeArgs,
    body: &serde_json::Value,
    audio: Option<&str>,
    score: Option<&str>,
    semantic: Option<&str>,
    seconds: Option<f32>,
    elapsed: Duration,
    family: &str,
) -> Result<String, String> {
    let _ = app;
    // 种子的位置两家不同：YuE2 在 `request.seed`，ACE-Step 在 `request.options.seed`。
    let seed = body
        .get("request")
        .and_then(|r| {
            r.get("seed")
                .or_else(|| r.get("options").and_then(|o| o.get("seed")))
        })
        .and_then(|s| s.as_u64())
        .unwrap_or(0);
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let record = serde_json::json!({
        "version": 1,
        "created": created,
        "family": family,
        "seed": seed,
        "stop_after": args.stop_after(),
        "elapsed": elapsed.as_secs_f32(),
        "seconds": seconds,
        "audio": audio,
        "score": score,
        "semantic": semantic,
        "args": args,
    });
    let path = PathBuf::from(base).with_extension("json");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&record).unwrap_or_default(),
    )
    .map_err(|e| format!("写生成记录失败：{e}"))?;
    Ok(path.to_string_lossy().into_owned())
}

/// 输出路径（与其它 AI 产物同一个 `local-ai/` 目录）。
fn output_path(app: &tauri::AppHandle, kind: &str) -> Result<String, String> {
    let dir = crate::paths::data_dir(app)?.join("local-ai");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    Ok(dir
        .join(format!("{kind}-{stamp}"))
        .to_string_lossy()
        .into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_carries_duration_as_semantic_tokens() {
        let args = ComposeArgs {
            style: "pop".into(),
            lyrics: "la".into(),
            duration: Some(60.0),
            seed: Some(7),
            ..Default::default()
        };
        let body = request_body(&args, "yue2");
        assert_eq!(body["model"], "yue2");
        assert_eq!(body["request"]["options"]["semantic_max_tokens"], 1500);
        assert_eq!(body["request"]["seed"], 7);
        assert_eq!(body["request"]["options"]["cot"], "full");
        assert!(body["request"]["options"].get("abc").is_none());
        // `style` 必须在 options 里（放顶层服务端读不到）。
        assert_eq!(body["request"]["options"]["style"], "pop");
    }

    #[test]
    fn external_abc_forces_a_symbolic_route() {
        let args = ComposeArgs {
            style: "pop".into(),
            abc: "X:1\nK:C\nC D E F".into(),
            cot: Some("off".into()),
            ..Default::default()
        };
        let body = request_body(&args, "yue2");
        assert_eq!(body["request"]["options"]["cot"], "full");
        assert_eq!(body["request"]["options"]["abc"], "X:1\nK:C\nC D E F");
    }

    #[test]
    fn negative_seed_becomes_a_random_positive_one() {
        let args = ComposeArgs {
            seed: Some(-1),
            ..Default::default()
        };
        let seed = args.seed();
        assert!(seed > 0);
        assert!(seed < (1u64 << 63));
    }

    #[test]
    fn result_parses_audio_and_score_artifacts() {
        use base64::Engine as _;
        let engine = base64::engine::general_purpose::STANDARD;
        let audio = engine.encode(b"RIFFxxxxWAVE");
        let score = engine.encode(b"X:1\nK:C\nC D E");
        let body = serde_json::json!({
            "audio": audio,
            "sample_rate": 48000,
            "channels": 2,
            "artifacts": [{
                "id": "score",
                "kind": "custom",
                "payload": score,
                "meta": { "format": "abc", "extension": "abc", "mime": "text/vnd.abc" }
            }],
            "timing": { "wall_ms": 12.5 }
        });
        let run = parse_result(&body.to_string()).unwrap();
        assert_eq!(run.audio.as_deref(), Some(&b"RIFFxxxxWAVE"[..]));
        assert_eq!(run.score.as_deref(), Some("X:1\nK:C\nC D E"));
        assert!(run.semantic.is_none());
    }

    /// `stop_after=abc` 的响应**没有 audio**，只有一份 ABC artifact。这是「只写谱」模式
    /// 能不能用的关键——以前这里把音频当必需项，会直接判「引擎没有给音频」。
    #[test]
    fn score_only_result_is_not_an_error() {
        use base64::Engine as _;
        let engine = base64::engine::general_purpose::STANDARD;
        // 真实形状取自 v0.9.3 的 gen4-out.json。
        let body = serde_json::json!({
            "artifacts": [{
                "id": "score",
                "kind": "custom",
                "payload": engine.encode(b"X:1\nT:\nM:2/4\nK:C\nC D E F|"),
                "meta": {
                    "mime": "text/vnd.abc",
                    "extension": "abc",
                    "format": "abc",
                    "source": "generated",
                    "truncated": "true"
                }
            }],
            "timing": { "wall_ms": 15372.6 }
        });
        let run = parse_result(&body.to_string()).unwrap();
        assert!(run.audio.is_none());
        assert!(run.score.unwrap().contains("K:C"));
    }

    /// 语义码 artifact：mime 是 `application/vnd.yue2.semantic+json`，payload 是 JSON 数组。
    #[test]
    fn semantic_artifact_comes_back_as_text() {
        use base64::Engine as _;
        let engine = base64::engine::general_purpose::STANDARD;
        let body = serde_json::json!({
            "artifacts": [{
                "id": "semantic",
                "kind": "acoustic_tokens",
                "payload": engine.encode(b"[1,2,3,4]"),
                "meta": { "mime": "application/vnd.yue2.semantic+json", "format": "yue2-semantic-codec" }
            }]
        });
        let run = parse_result(&body.to_string()).unwrap();
        assert_eq!(run.semantic.as_deref(), Some("[1,2,3,4]"));
    }

    #[test]
    fn result_without_anything_is_an_error() {
        let body = serde_json::json!({ "artifacts": [] });
        assert!(parse_result(&body.to_string()).is_err());
    }

    #[test]
    fn stop_after_and_semantic_export_ride_in_options() {
        let args = ComposeArgs {
            style: "pop".into(),
            stop_after: Some("semantic".into()),
            ..Default::default()
        };
        let body = request_body(&args, "yue2");
        assert_eq!(body["request"]["options"]["stop_after"], "semantic");
        // 停在 semantic 却不导出码等于白跑，所以自动打开。
        assert_eq!(body["request"]["options"]["export_semantic"], true);
    }

    #[test]
    fn sampling_knobs_are_prefixed_per_stage() {
        let args = ComposeArgs {
            style: "pop".into(),
            abc_sampling: Some(SamplingArgs {
                temperature: Some(1.2),
                top_p: Some(0.9),
                top_k: Some(40),
                ..Default::default()
            }),
            semantic_sampling: Some(SamplingArgs {
                temperature: Some(0.8),
                max_tokens: Some(3000),
                ..Default::default()
            }),
            ..Default::default()
        };
        let body = request_body(&args, "yue2");
        let options = &body["request"]["options"];
        let number = |key: &str| options[key].as_f64().unwrap_or(f64::NAN);
        assert!((number("abc_temperature") - 1.2).abs() < 1e-5);
        assert!((number("abc_top_p") - 0.9).abs() < 1e-5);
        assert_eq!(options["abc_top_k"], 40);
        assert!((number("semantic_temperature") - 0.8).abs() < 1e-5);
        // 显式给的 max 覆盖时长换算。
        assert_eq!(options["semantic_max_tokens"], 3000);
    }

    /// ACE-Step 的请求体形状与 YuE2 不同：**描述走顶层 `text`**，歌词 / 种子 / 时长在 `options` 里
    /// （v0.9.3 的 `ace_step_parse_request` 就是这么读的；发错位置引擎只会回
    /// 「ACE-Step requires text_input」）。
    #[test]
    fn ace_step_body_puts_the_prompt_on_top_and_knobs_in_options() {
        let args = ComposeArgs {
            style: "cinematic synth pop".into(),
            lyrics: "We rise with the morning light".into(),
            duration: Some(90.0),
            steps: Some(32),
            seed: Some(1234),
            ..Default::default()
        };
        let body = request_body(&args, "ace_step");
        assert_eq!(body["model"], "ace_step");
        assert_eq!(body["request"]["text"], "cinematic synth pop");
        let options = &body["request"]["options"];
        assert_eq!(options["route"], "text2music");
        assert_eq!(options["lyrics"], "We rise with the morning light");
        assert_eq!(options["duration_sec"], 90.0);
        // Turbo 档上限 8 步：填 32 也只发 8。
        assert_eq!(options["num_inference_steps"], 8);
        assert_eq!(options["seed"], 1234);
        // CFG 没填（-1）就不发：Turbo 本来也不看它。
        assert!(options.get("guidance_scale").is_none());
        // 顶层的 seed 只给 YuE2 用。
        assert!(body["request"].get("seed").is_none());
    }

    #[test]
    fn semantic_prefix_is_forwarded_as_a_file() {
        let args = ComposeArgs {
            style: "pop".into(),
            semantic_prefix_file: Some("/tmp/take.semantic.json".into()),
            duration: Some(180.0),
            ..Default::default()
        };
        let body = request_body(&args, "yue2");
        let options = &body["request"]["options"];
        assert_eq!(options["semantic_prefix_file"], "/tmp/take.semantic.json");
        assert_eq!(options["semantic_max_tokens"], 4500);
    }

    /// 语义码文件被读成**内联前缀**：去掉 BOM、验数组，再把 `semantic_prefix_file` 清掉。
    /// 引擎只认纯 JSON 数组（一个 BOM 就报 byte 0 解析失败），这一步是替用户挡下来的。
    #[test]
    fn prefix_file_is_inlined_and_bom_is_stripped() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("yue-prefix-{}.json", std::process::id()));
        // 故意带 UTF-8 BOM。
        std::fs::write(&path, "\u{feff}[1, 2, 3]").unwrap();

        let mut args = ComposeArgs {
            style: "pop".into(),
            cot: Some("off".into()),
            semantic_prefix_file: Some(path.to_string_lossy().into_owned()),
            ..Default::default()
        };
        normalize_prefix(&mut args).unwrap();
        assert_eq!(args.semantic_prefix.as_deref(), Some("[1, 2, 3]"));
        assert!(args.semantic_prefix_file.is_none());

        let body = request_body(&args, "yue2");
        assert_eq!(body["request"]["options"]["semantic_prefix"], "[1, 2, 3]");
        assert!(body["request"]["options"]
            .get("semantic_prefix_file")
            .is_none());
        let _ = std::fs::remove_file(&path);
    }

    /// 续写要沿用原曲的文本条件：生成记录就在音频旁边（`<音频>.json`）。
    #[test]
    fn record_conditions_come_from_the_generation_record() {
        let dir = std::env::temp_dir().join(format!("yue-record-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let audio = dir.join("take.wav");
        let record = dir.join("take.json");
        std::fs::write(
            &record,
            r#"{"args":{"style":"jazz pop","lyrics":"la la","model":"yue2-bf16"}}"#,
        )
        .unwrap();

        let audio = audio.to_string_lossy().into_owned();
        let (style, lyrics, model) = record_conditions(&audio);
        assert_eq!(style.as_deref(), Some("jazz pop"));
        assert_eq!(lyrics.as_deref(), Some("la la"));
        assert_eq!(model.as_deref(), Some("yue2-bf16"));

        // 记录缺失 / 空字段：返回 None，界面退回让用户自己填。
        std::fs::remove_file(&record).unwrap();
        assert_eq!(record_conditions(&audio), (None, None, None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 前缀配 `cot != off` 又没给谱：以前会报错，现在**直接把路线钉成 `off`**——
    /// 续写不该重新规划谱（那会让接出来的段落和原曲脱节）。
    #[test]
    fn prefix_without_a_score_is_forced_to_cot_off() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("yue-prefix-bad-{}.json", std::process::id()));
        std::fs::write(&path, "[7, 8]").unwrap();
        let mut args = ComposeArgs {
            style: "pop".into(),
            cot: Some("melody".into()),
            semantic_prefix_file: Some(path.to_string_lossy().into_owned()),
            ..Default::default()
        };
        normalize_prefix(&mut args).unwrap();
        assert_eq!(args.cot.as_deref(), Some("off"));
        assert_eq!(args.semantic_prefix.as_deref(), Some("[7, 8]"));
        let body = request_body(&args, "yue2");
        assert_eq!(body["request"]["options"]["cot"], "off");
        let _ = std::fs::remove_file(&path);
    }

    /// 前缀 + 外部谱：保留调用方给的路线（引擎允许「手里有谱」时吃前缀），不强行改 off。
    #[test]
    fn prefix_with_a_score_keeps_the_route() {
        let mut args = ComposeArgs {
            style: "pop".into(),
            cot: Some("melody".into()),
            abc: "X:1\nK:C\nC D E".into(),
            semantic_prefix: Some("[7, 8]".into()),
            ..Default::default()
        };
        normalize_prefix(&mut args).unwrap();
        assert_eq!(args.cot.as_deref(), Some("melody"));
    }

    /// 续写只对 YuE2 有效：ACE-Step 会**静默忽略**前缀（请求体里根本没这个字段），
    /// 放过去就变成「又写了一首无关的歌」——这正是用户看到的「续写和原曲毫无关联」。
    #[test]
    fn extending_with_ace_step_is_rejected() {
        let args = ComposeArgs {
            semantic_prefix: Some("[1, 2, 3]".into()),
            ..Default::default()
        };
        let err = reject_prefix_outside_yue2(&args, "ace_step").unwrap_err();
        assert!(err.contains("YuE2"), "{err}");
        assert!(reject_prefix_outside_yue2(&args, "yue2").is_ok());
        // 不是续写的 ACE-Step 请求不受影响。
        let plain = ComposeArgs {
            style: "pop".into(),
            ..Default::default()
        };
        assert!(reject_prefix_outside_yue2(&plain, "ace_step").is_ok());
    }

    /// 前缀裁剪：语义码 25 帧/秒，与成品采样率无关；超长的码（引擎提前 EOS）截在总帧数上。
    #[test]
    fn prefix_frames_follow_the_semantic_rate() {
        // 3000 码 = 120 秒；48 kHz 下正好 5_760_000 帧。
        assert_eq!(prefix_frames_for(3000, 48_000.0, 9_600_000), 5_760_000);
        // 码比成品还长：裁到总帧数，`saturating_sub` 之后不会出现负的窗口。
        assert_eq!(prefix_frames_for(3000, 48_000.0, 1_000), 1_000);
        assert_eq!(prefix_frames_for(0, 48_000.0, 1_000), 0);
    }

    #[test]
    fn server_errors_are_surfaced() {
        let body =
            serde_json::json!({ "error": { "message": "yue2.model_gguf file does not exist" } });
        let err = parse_result(&body.to_string()).unwrap_err();
        assert!(err.contains("model_gguf"), "{err}");
    }

    #[test]
    fn backends_follow_the_device_names() {
        assert_eq!(backend_arg("cpu"), "cpu");
        assert_eq!(backend_arg("vulkan"), "vulkan");
        assert_eq!(backend_arg("metal"), "metal");
        assert_eq!(backend_arg("cuda12"), "cuda");
        assert_eq!(backend_arg("cuda13"), "cuda");
    }

    #[test]
    fn semantic_codes_sit_beside_the_audio() {
        // 生成时 `<stem>.wav` 与 `<stem>.semantic.json` 同名同目录：续写就靠这个约定找码。
        assert_eq!(
            semantic_sidecar("/tmp/take.wav"),
            PathBuf::from("/tmp/take.semantic.json")
        );
        assert_eq!(
            semantic_sidecar("C:\\yue2\\take.flac"),
            PathBuf::from("C:\\yue2\\take.semantic.json")
        );
    }
}
