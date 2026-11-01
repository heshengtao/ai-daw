//! **audio.cpp 引擎进程（`audiocpp_server`；当前服务于 YuE2 写歌）**：起停、健康检查、提交生成任务、收结果入工程。
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
//! 落地的东西和以前一样：一段 48 kHz 立体声 WAV +（模型自己写了谱时）一份 `.abc`，
//! 再走宿主统一的 `AddAudioTrack` 进工程——一次生成 = 一条撤销记录。

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

/// 一次生成用哪份权重。
struct ModelChoice {
    id: String,
    dir: PathBuf,
    /// 相对模型目录的文件名（audio.cpp 拒绝绝对路径）。
    model_file: String,
    vae_file: String,
}

/// 解析要用的模型：显式指定优先，否则按「默认 Q8_0 → Q4_0 → BF16」挑第一个装好的。
fn resolve_model(wanted: Option<&str>) -> Result<ModelChoice, String> {
    const ORDER: [&str; 3] = ["yue2-q8-0", "yue2-q4-0", "yue2-bf16"];
    let mut ids: Vec<&str> = Vec::new();
    if let Some(id) = wanted.filter(|s| !s.trim().is_empty()) {
        ids.push(id);
    }
    ids.extend(ORDER.iter().copied());
    for id in ids {
        let Some(entry) = catalog::by_id(id) else {
            continue;
        };
        let Some(dir) = catalog::group_dir(entry) else {
            continue;
        };
        let Some(model_file) = entry
            .files
            .iter()
            .map(|f| f.filename)
            .find(|name| name.starts_with("yue2-3b-"))
            .map(str::to_string)
        else {
            continue;
        };
        let Some(vae_file) = entry
            .files
            .iter()
            .map(|f| f.filename)
            .find(|name| name.starts_with("yue2-vae-"))
            .map(str::to_string)
        else {
            continue;
        };
        return Ok(ModelChoice {
            id: id.to_string(),
            dir,
            model_file,
            vae_file,
        });
    }
    Err("还没下载 YuE2 的权重（下载管理器 → 模型 → YuE2 3B）".into())
}

/// 找一个空闲端口（绑到 0 再放掉，唯一的竞争窗口在 bind 与 spawn 之间）。
fn free_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    drop(listener);
    Ok(port)
}

fn probe_agent() -> ureq::Agent {
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
fn work_agent() -> ureq::Agent {
    crate::manager::tls_agent(
        None,
        Some(Duration::from_secs(5)),
        Some(Duration::from_secs(10)),
        Some(Duration::from_secs(3600)),
        None,
    )
}

/// 服务器的 `--backend` 参数（CUDA 12 / 13 都是 `cuda`，靠目录里的 DLL 区分）。
fn backend_arg(device: &str) -> &'static str {
    match device {
        "vulkan" => "vulkan",
        "metal" => "metal",
        "cuda12" | "cuda13" => "cuda",
        _ => "cpu",
    }
}

/// 线程数：物理核不可知，取逻辑核的一半（至少 2，最多 16）。
fn thread_count() -> usize {
    let logical = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    (logical / 2).clamp(2, 16)
}

/// 写配置文件（每次启动都重写：模型 / 后端可能换过）。
fn write_config(
    pack: &'static engine::EnginePack,
    model: &ModelChoice,
    port: u16,
    cuda_perf: bool,
) -> Result<PathBuf, String> {
    let mut session = serde_json::json!({
        "yue2.model_gguf": model.model_file,
        "yue2.vae_gguf": model.vae_file,
    });
    // CUDA 上 audio.cpp 官方给了 `cuda_perf` 预设（packed projections + KV 对齐 + 不释放
    // 中间态），默认开；显存紧的用户在 AI 写歌页关掉。
    if pack.device.starts_with("cuda") && cuda_perf {
        session["yue2.perf_mode"] = serde_json::json!("cuda_perf");
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
        "models": [{
            "id": "yue2",
            "family": "yue2",
            "task": "gen",
            "mode": "offline",
            "path": model.dir.to_string_lossy(),
            "session_options": session,
            "default_request_options": { "cot": "full" },
        }],
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
    let config = write_config(pack, &choice, port, cuda_perf)?;
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

/// 一次生成的参数（界面 / MCP 共用）。
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct ComposeArgs {
    /// 风格标签（逗号分隔，例如 `pop, female vocal, city pop`）。
    pub style: String,
    /// 歌词（带 `[Verse]` / `[Chorus]` 这类结构标签）。
    pub lyrics: String,
    /// 直接给一段 ABC 谱（空 = 让模型自己写）。
    pub abc: String,
    /// 目标时长（秒；默认 120，夹到 15..=360）。换算成语义 token 上限（25 帧/秒）。
    pub duration: Option<f32>,
    /// 流匹配步数（默认 8，夹到 1..=64）。
    pub steps: Option<u32>,
    /// 种子（-1 = 随机；>=0 可复现）。
    pub seed: Option<i64>,
    /// 结构模式：`full`（带和弦的谱）/ `melody`（只要旋律）/ `off`（跳过符号阶段）。
    pub cot: Option<String>,
    /// CFG：负值 = 用协议默认（`off` 模式 1.01，其余 1.0；1.0 等于单分支、解码快一倍）。
    pub cfg_scale: Option<f32>,
    /// 指定的权重 id；省略 = 用已安装的默认档。
    pub model: Option<String>,
    /// 生成结果插到这条轨道下面（轨道 id；省略 = 追加到工程末尾）。
    pub after_track: Option<u64>,
    /// 新轨道的名字。
    pub name: Option<String>,
}

impl ComposeArgs {
    fn duration(&self) -> f32 {
        self.duration.unwrap_or(120.0).clamp(15.0, 360.0)
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
        let has_abc = !self.abc.trim().is_empty();
        match given {
            // 给了外部谱就必须走符号路线；`off` 会直接忽略 abc，等于白给。
            "off" if has_abc => "full",
            "off" => "off",
            "melody" => "melody",
            _ => "full",
        }
    }
}

/// 组装 `/v1/tasks/run` 的请求体。
///
/// 实测得来的两条硬约束（v0.9.3）：
/// - 模型专属选项（`style` / `cot` / `abc` / `num_inference_steps` / `semantic_*` …）必须
///   放在 **`options`** 里。放顶层时服务端读不到 `style`，报「Yue2 requires non-empty style」。
/// - `lyrics` / `seed` 是通用字段，放顶层；`semantic_min_tokens` 默认 200，`semantic_max_tokens`
///   比它小会被判「invalid」——所以只设上限、不动下限（除非时长换算出来的比 200 还小）。
fn request_body(args: &ComposeArgs) -> serde_json::Value {
    let mut options = serde_json::json!({
        "style": args.style,
        "cot": args.cot(),
        "num_inference_steps": args.steps(),
        // 25 帧/秒：界面给「时长」、协议给「最多多少帧」。
        "semantic_max_tokens": (args.duration() * SEMANTIC_FPS).round() as u64,
        "export_semantic": false,
    });
    if !args.abc.trim().is_empty() {
        options["abc"] = serde_json::json!(args.abc);
    }
    if let Some(cfg) = args.cfg_scale.filter(|c| *c >= 0.0) {
        options["guidance_scale"] = serde_json::json!(cfg);
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
#[tauri::command]
pub fn ai_yue_compose(app: tauri::AppHandle, args: ComposeArgs) -> Result<(), String> {
    if args.style.trim().is_empty() && args.lyrics.trim().is_empty() {
        return Err("风格和歌词至少给一样".into());
    }
    let current = status();
    if current.state != "ready" {
        return Err("引擎还没就绪：先点「启动引擎」（首次要加载几分钟）".into());
    }
    let port = current.port.ok_or("引擎没有端口")?;
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
            let outcome = crate::job::guard(|| {
                run_compose_job(&guard_app, port, args, started)
            });
            if let Err(message) = outcome {
                engine_state().busy.store(false, Ordering::SeqCst);
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
    args: ComposeArgs,
    started: Instant,
) -> Result<String, String> {
    let result = submit(port, &args);
    engine_state().busy.store(false, Ordering::SeqCst);
    if engine_state().cancel.load(Ordering::Relaxed) {
        // 取消那条事件已经在 `ai_yue_compose_cancel` 里发过了。
        return Ok(String::new());
    }
    match result {
        Ok((audio, score)) => match collect(app, &args, audio, score, started.elapsed()) {
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

fn compose_blocking_inner(app: &tauri::AppHandle, args: ComposeArgs) -> Result<String, String> {
    if args.style.trim().is_empty() && args.lyrics.trim().is_empty() {
        return Err("风格和歌词至少给一样".into());
    }
    let wanted = args.model.clone().filter(|m| !m.trim().is_empty());
    let current = status();
    let same_model = current.state == "ready"
        && wanted.as_deref().is_none_or(|model| current.model == model);
    if !same_model {
        ai_yue_engine_start(app.clone(), wanted)?;
        let deadline = Instant::now() + Duration::from_secs(600);
        loop {
            std::thread::sleep(Duration::from_millis(500));
            if engine_state().cancel.load(Ordering::Relaxed) {
                return Err("已取消：引擎已停止".into());
            }
            let state = status();
            match state.state.as_str() {
                "ready" => break,
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
    if engine_state().cancel.load(Ordering::Relaxed) {
        return Err("已取消：引擎已停止".into());
    }
    let port = status().port.ok_or("引擎没有端口")?;
    let started = Instant::now();
    let (audio, score) = submit(port, &args)?;
    if engine_state().cancel.load(Ordering::Relaxed) {
        return Ok("已取消".into());
    }
    let job = collect(app, &args, audio, score, started.elapsed())?;
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

/// 提交并等结果：返回（WAV 字节, ABC 文本）。
fn submit(port: u16, args: &ComposeArgs) -> Result<(Vec<u8>, Option<String>), String> {
    let agent = work_agent();
    let body = request_body(args).to_string();
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

/// 从 `/v1/tasks/run` 的 JSON 里拆出音频与谱面。
fn parse_result(text: &str) -> Result<(Vec<u8>, Option<String>), String> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| format!("引擎回了怪东西：{e}（{}）", brief(text)))?;
    if let Some(message) = value
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
    {
        return Err(format!("引擎报错：{message}"));
    }
    let audio = value
        .get("audio")
        .and_then(|a| a.as_str())
        .filter(|s| !s.is_empty())
        .ok_or("引擎没有给音频")?;
    let bytes = decode_base64(audio).map_err(|e| format!("音频解码失败：{e}"))?;
    if !bytes.starts_with(b"RIFF") {
        return Err(format!(
            "引擎给的音频不是 WAV（前 {:?}）",
            &bytes[..bytes.len().min(4)]
        ));
    }

    // 谱面：`kind=score` 的 artifact，payload 是 Base64 的 ABC 文本。
    let score = value
        .get("artifacts")
        .and_then(|a| a.as_array())
        .and_then(|list| {
            list.iter().find(|item| {
                item.get("kind").and_then(|k| k.as_str()) == Some("score")
                    || item
                        .get("meta")
                        .and_then(|m| m.get("format"))
                        .and_then(|f| f.as_str())
                        .map(|f| f.contains("abc"))
                        .unwrap_or(false)
            })
        })
        .and_then(|item| item.get("payload").and_then(|p| p.as_str()))
        .map(decode_base64)
        .transpose()
        .map_err(|e| format!("谱面解码失败：{e}"))?
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .filter(|abc| !abc.trim().is_empty());
    Ok((bytes, score))
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

/// 把音频 / 谱面落盘并入工程。
fn collect(
    app: &tauri::AppHandle,
    args: &ComposeArgs,
    audio: Vec<u8>,
    score: Option<String>,
    elapsed: Duration,
) -> Result<JobStatus, String> {
    let base = output_path(app, "yue2")?;
    let wav = PathBuf::from(&base).with_extension("wav");
    std::fs::write(&wav, &audio).map_err(|e| format!("写音频失败：{e}"))?;

    let mut score_path = None;
    if let Some(abc) = score.as_deref() {
        let path = PathBuf::from(&base).with_extension("abc");
        let _ = std::fs::write(&path, abc);
        score_path = Some(path.to_string_lossy().into_owned());
    }

    // 解一遍拿到时长（48 kHz 立体声），顺便进素材缓存。
    let state = app.state::<AppState>();
    let wav_text = wav.to_string_lossy().into_owned();
    let decoded = state.assets.load(&wav_text)?;
    let seconds = (decoded.frames() as f64 / decoded.sample_rate.max(1.0)) as f32;

    // 工程按拍记时间：换算成当前速度下的拍数（生成结果本身不跟速度走）。
    let tempo = {
        let engine = state.engine.lock().unwrap();
        engine.project().transport.tempo
    };
    let beats = (seconds as f64) * tempo / 60.0;

    let name = args
        .name
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "YuE2 写歌".to_string());

    // 谱面在生成时就一起解析：音频与 MIDI 在**同一个事务**里插入，和歌声合成 / 音色替换
    // 一样“结果落在锚点轨道下面，一次撤销”。锚点 = 生成时选中的轨道；没选就追加到末尾。
    let voices = score
        .as_deref()
        .and_then(|abc| crate::abc::parse(abc).ok())
        .unwrap_or_default();
    let anchor = {
        let engine = state.engine.lock().unwrap();
        args.after_track
            .and_then(|id| {
                engine
                    .project()
                    .tracks
                    .iter()
                    .position(|track| track.id.0 == id)
            })
    };
    let insert_at = anchor.map(|index| index + 1);

    let mut commands = Vec::new();
    commands.push(EngineCommand::AddAudioTrack {
        name: name.clone(),
        start: 0.0,
        length: beats.max(1.0),
        source: AudioSource {
            path: wav_text.clone(),
            offset_frames: 0,
            frames: None,
            rate: 1.0,
            pitch_semitones: 0.0,
        },
        at: insert_at,
    });
    for (offset, voice) in voices.iter().enumerate() {
        commands.push(EngineCommand::AddNoteTrack {
            name: format!("{} · 谱", voice.name),
            start: 0.0,
            length: voice.end.max(1.0),
            notes: voice.notes.clone(),
            at: insert_at.map(|base| base + 1 + offset),
        });
    }
    crate::commands::run_agent_all(app, commands);

    let midi_part = if voices.is_empty() {
        String::new()
    } else {
        format!("，并写入 {} 条 MIDI 谱轨", voices.len())
    };
    Ok(JobStatus {
        state: "done".into(),
        elapsed: elapsed.as_secs_f32(),
        message: Some(format!(
            "已生成「{name}」：{:.0} 秒（用时 {:.0} 秒）{midi_part}",
            seconds,
            elapsed.as_secs_f32()
        )),
        audio: Some(wav_text),
        score: score_path,
        seconds: Some(seconds),
    })
}

/// 输出路径（与其它 AI 产物同一个 `local-ai/` 目录）。
fn output_path(app: &tauri::AppHandle, kind: &str) -> Result<String, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("拿不到应用数据目录：{e}"))?
        .join("local-ai");
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
        let body = request_body(&args);
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
        let body = request_body(&args);
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
                "kind": "score",
                "payload": score,
                "meta": { "format": "text/vnd.abc" }
            }],
            "timing": { "wall_ms": 12.5 }
        });
        let (bytes, abc) = parse_result(&body.to_string()).unwrap();
        assert_eq!(bytes, b"RIFFxxxxWAVE");
        assert_eq!(abc.as_deref(), Some("X:1\nK:C\nC D E"));
    }

    #[test]
    fn result_without_audio_is_an_error() {
        let body = serde_json::json!({ "artifacts": [] });
        assert!(parse_result(&body.to_string()).is_err());
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
}
