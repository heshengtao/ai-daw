//! **SheetSage2 转谱**：录音 → ABC 谱。YuE2「翻唱」的第一步。
//!
//! 翻唱 = `转谱 → 拿 ABC 让 YuE2 按谱重唱`。第二步是 [`crate::yue_engine`] 的
//! `compose_song`（`abc`/`abc_file` + `cot=melody|full`），这里只负责第一步。
//!
//! ## 为什么单起一个引擎进程
//!
//! YuE2 和 SheetSage2 是**两份互不相干的大权重**（4.3 GB + 2.5 GB）。写歌引擎是
//! `lazy_load=false`（启动即把 YuE2 全塞进显存），再挂一个 2.5 GB 的转谱模型多半会 OOM。
//! 所以转谱**先把写歌引擎停掉**，用一份只含 SheetSage2 的配置起一个**一次性**进程，
//! 跑完就杀——顺序是对的：用户接下来要写歌，写歌引擎会在需要时重新拉起。
//!
//! ## 协议（audio.cpp v0.9.3）
//!
//! - 模型规格 `model_specs/sheetsage2.json`（`tasks: ["midi"]`）。发布包里带着它，但老安装
//!   只解出了 `yue2.json`，所以启动前用编译期内嵌的副本**补齐**（`ensure_sheetsage2_spec`）。
//! - `POST /v1/tasks/run`，`request.audio` 传**服务端本机路径**（写成绝对路径，避免和
//!   「相对配置目录」的模型路径混淆）。
//! - 结果是一份 ABC artifact；官方文档说它写的是 ABC 而不是二进制 MIDI（真收到 `MThd`
//!   会明确报错，而不是把乱码灌进谱面框）。
//! - 编码器按 **300 秒窗口**做注意力：默认全窗口在显存上要 3.6 GB 的 score 张量，
//!   所以固定传 `attention_tile_rows=1024`（每个张量 < 512 MiB，不改变数值结果，只降吞吐）。

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::Manager;

use ai_daw_local_ai::engine;
use ai_daw_local_ai::models::catalog;

use crate::state::AppState;

/// 转谱用的模型 id（清单里那一条）。
pub const MODEL_ID: &str = "sheetsage2-orig";

/// 转谱输入（界面 / MCP 共用）。
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct TranscribeArgs {
    /// 音频文件路径；与 `track` 二选一，同时给时以它为准。
    pub audio: Option<String>,
    /// 用这条轨道的第一个音频片段。
    pub track: Option<u64>,
    /// 指定片段 id。
    pub clip: Option<u64>,
}

/// 转谱结果。
#[derive(Debug, Clone, serde::Serialize)]
pub struct TranscribeOutcome {
    /// 落盘的 `.abc` 路径。
    pub score: String,
    /// ABC 文本（界面直接填进谱面框）。
    pub abc: String,
    /// ABC 字符数（判断「是不是空谱」用）。
    pub chars: usize,
    /// 输入音频时长（秒）。
    pub seconds: f32,
    /// 用时（秒）。
    pub elapsed: f32,
}

/// 正在跑的转谱子进程（取消 = 杀它）。
fn child_slot() -> &'static Mutex<Option<Child>> {
    static CHILD: OnceLock<Mutex<Option<Child>>> = OnceLock::new();
    CHILD.get_or_init(|| Mutex::new(None))
}

fn cancel_flag() -> &'static AtomicBool {
    static CANCEL: OnceLock<AtomicBool> = OnceLock::new();
    CANCEL.get_or_init(|| AtomicBool::new(false))
}

/// 同一时刻只允许一个转谱：两个引擎进程会把显存吃干净。
fn busy_flag() -> &'static AtomicBool {
    static BUSY: OnceLock<AtomicBool> = OnceLock::new();
    BUSY.get_or_init(|| AtomicBool::new(false))
}

/// 转谱互斥的守卫（离开作用域自动放开，线程 panic 也不会把按钮永久禁掉）。
///
/// 抓歌词共用同一把：两者都起一次性引擎进程，同时跑会把显存吃干净。
pub(crate) struct BusyGuard;

impl BusyGuard {
    pub(crate) fn acquire() -> Result<Self, String> {
        if busy_flag().swap(true, Ordering::SeqCst) {
            return Err("已经有一个转谱在跑（等它结束，或先取消）".into());
        }
        Ok(Self)
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        busy_flag().store(false, Ordering::SeqCst);
    }
}

/// 取消当前转谱（杀掉一次性引擎进程；阻塞中的请求会立刻返回失败）。
#[tauri::command]
pub fn ai_yue_transcribe_cancel() {
    cancel_flag().store(true, Ordering::Relaxed);
    kill_child();
}

fn kill_child() {
    if let Some(mut child) = child_slot().lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

// ---------------------------------------------------------------------------
// 输入音频
// ---------------------------------------------------------------------------

/// 把输入解析成「一段 PCM + 采样率」（轨道片段按 offset/frames 切，文件整体读）。
///
/// 抓歌词（`yue_lyrics`）复用这一份：音频输入的解析规则只能有一套。
pub(crate) fn load_input(
    app: &tauri::AppHandle,
    audio: Option<&str>,
    track: Option<u64>,
    clip: Option<u64>,
) -> Result<(Vec<f32>, usize, f64), String> {
    let state = app.state::<AppState>();

    // 1) 直接给文件：整段用。
    if let Some(path) = audio.filter(|p| !p.trim().is_empty()) {
        if !Path::new(path).is_file() {
            return Err(format!("音频文件不存在：{path}"));
        }
        let audio = state.assets.load(path)?;
        return Ok((audio.samples.to_vec(), audio.channels, audio.sample_rate));
    }

    // 2) 给轨道：取第一个音频片段（或指定片段），按片段自己的 offset/frames 切。
    let track_id = track.ok_or("要么给 audio 文件，要么给 track")?;
    let (source_path, offset, frames) = {
        let engine = state.engine.lock().unwrap();
        let project = engine.project();
        let track = project
            .tracks
            .iter()
            .find(|t| t.id.0 == track_id)
            .ok_or("没有这条轨道")?;
        let clip = track
            .clips
            .iter()
            .filter(|c| c.audio.is_some())
            .find(|c| clip.is_none_or(|id| c.id.0 == id))
            .ok_or("这条轨道上没有音频片段（转谱只吃音频轨）")?;
        let source = clip.audio.clone().expect("已确认是音频片段");
        (source.path, source.offset_frames as usize, source.frames)
    };

    let audio = state.assets.load(&source_path)?;
    let channels = audio.channels.max(1);
    let total = audio.frames();
    let start = offset.min(total);
    let count = frames
        .map(|n| n as usize)
        .unwrap_or(total - start)
        .min(total - start);
    let begin = start * channels;
    let end = begin + count * channels;
    let slice = audio
        .samples
        .get(begin..end)
        .ok_or("片段范围超出音频长度")?
        .to_vec();
    Ok((slice, channels, audio.sample_rate))
}

/// 写一段 16-bit PCM WAV（引擎吃 WAV 路径；顺手把非 WAV / 切片统一成同一份格式）。
pub(crate) fn write_wav(
    path: &Path,
    samples: &[f32],
    channels: usize,
    sample_rate: f64,
) -> Result<(), String> {
    let channels = channels.max(1) as u16;
    let rate = sample_rate.round().max(1.0) as u32;
    let data_len = (samples.len() * 2) as u32;
    let block_align = channels * 2;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * block_align as u32).to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * 32_767.0).round() as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    std::fs::write(path, out).map_err(|e| format!("写临时 WAV 失败：{e}"))
}

// ---------------------------------------------------------------------------
// 引擎
// ---------------------------------------------------------------------------

/// 一次性转谱引擎的配置（只有 SheetSage2 一个模型）。
fn write_config(device: &str, model_dir: &Path, port: u16) -> Result<PathBuf, String> {
    let config = serde_json::json!({
        "host": "127.0.0.1",
        "port": port,
        "backend": crate::yue_engine::backend_arg(device),
        "device": 0,
        "threads": crate::yue_engine::thread_count(),
        "lazy_load": false,
        "max_loaded_models": 1,
        "ui": false,
        "models": [{
            "id": "sheetsage2",
            "family": "sheetsage2",
            "task": "midi",
            "mode": "offline",
            "path": model_dir.to_string_lossy(),
            "session_options": { "sheetsage2.attention_tile_rows": 1024 },
        }],
    });
    let path = engine::dir().join("transcribe.json");
    let text = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("写转谱配置失败：{e}"))?;
    Ok(path)
}

fn log_tail(lines: usize) -> String {
    let path = engine::dir().join("transcribe.log");
    let Ok(file) = std::fs::File::open(path) else {
        return String::new();
    };
    let mut all: Vec<String> = BufReader::new(file).lines().map_while(Result::ok).collect();
    let start = all.len().saturating_sub(lines);
    all.drain(..start);
    all.join("\n")
}

/// 起引擎、等 `/health`、提交任务、拿 ABC、杀进程。
fn run_engine(
    pack: &'static engine::EnginePack,
    model_dir: &Path,
    wav: &Path,
) -> Result<String, String> {
    engine::ensure_sheetsage2_spec(pack)?;
    let port = crate::yue_engine::free_port()?;
    let config = write_config(crate::yue::effective_device(), model_dir, port)?;
    let dir = engine::dir_of(pack);

    let log_path = engine::dir().join("transcribe.log");
    let log = std::fs::File::create(&log_path).map_err(|e| format!("建日志文件失败：{e}"))?;
    let log_err = log.try_clone().map_err(|e| e.to_string())?;

    let mut command = Command::new(engine::executable(pack));
    command
        .current_dir(&dir)
        .arg("--config")
        .arg(&config)
        .arg("--backend")
        .arg(crate::yue_engine::backend_arg(crate::yue::effective_device()))
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
        command.creation_flags(0x0800_0000);
    }

    let child = command
        .spawn()
        .map_err(|e| format!("拉不起 audiocpp_server：{e}"))?;
    *child_slot().lock().unwrap() = Some(child);

    // 权重 2.5 GB：加载 + 建图给 10 分钟，和写歌引擎一个尺度。
    let agent = crate::yue_engine::probe_agent();
    let health = format!("http://127.0.0.1:{port}/health");
    let started = Instant::now();
    loop {
        if cancel_flag().load(Ordering::Relaxed) {
            kill_child();
            return Err("已取消".into());
        }
        {
            let mut guard = child_slot().lock().unwrap();
            if let Some(child) = guard.as_mut() {
                if let Ok(Some(code)) = child.try_wait() {
                    return Err(format!("转谱引擎启动后立刻退出（{code}）：{}", log_tail(6)));
                }
            }
        }
        if let Ok(resp) = agent.get(&health).call() {
            if resp.status().as_u16() == 200 {
                break;
            }
        }
        if started.elapsed() > Duration::from_secs(600) {
            kill_child();
            return Err(
                "转谱引擎 10 分钟还没就绪（看日志：runtime/audiocpp/transcribe.log）".into(),
            );
        }
        std::thread::sleep(Duration::from_millis(700));
    }

    let body = serde_json::json!({
        "model": "sheetsage2",
        "request": { "audio": wav.to_string_lossy() },
    });
    let work = crate::yue_engine::work_agent();
    let resp = work
        .post(&format!("http://127.0.0.1:{port}/v1/tasks/run"))
        .header("content-type", "application/json")
        .send(body.to_string().as_bytes());
    let result = match resp {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let mut text = String::new();
            resp.into_body()
                .into_reader()
                .read_to_string(&mut text)
                .map_err(|e| e.to_string())?;
            if status != 200 {
                Err(format!("引擎回了 HTTP {status}：{}", brief(&text)))
            } else {
                parse_abc(&text)
            }
        }
        Err(e) => {
            if cancel_flag().load(Ordering::Relaxed) {
                Err("已取消".into())
            } else {
                Err(format!("提交转谱任务失败：{e}"))
            }
        }
    };

    kill_child();
    result
}

fn brief(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= 300 {
        return text.to_string();
    }
    text.chars().take(300).collect::<String>() + "…"
}

pub(crate) fn decode_base64(text: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    let payload = text.split_once(',').map(|(_, rest)| rest).unwrap_or(text);
    base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .map_err(|e| e.to_string())
}

/// 从 `/v1/tasks/run` 的结果里取 ABC。
///
/// SheetSage2 的 artifact 带 `midi` / `midi_artifact` 语义；这里按「kind / id / meta」三条
/// 线索一起认，取不到就报出原文前 300 字符（比静默失败好查）。
fn parse_abc(text: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| format!("引擎回了怪东西：{e}（{}）", brief(text)))?;
    if let Some(message) = value
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
    {
        return Err(format!("引擎报错：{message}"));
    }

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
        let looks_symbolic = kind == "midi"
            || kind == "score"
            || id.contains("score")
            || id.contains("abc")
            || format.contains("abc")
            || format.contains("midi")
            || extension == "abc"
            || mime.contains("abc");
        if !looks_symbolic {
            continue;
        }
        let Some(payload) = item
            .get("payload")
            .and_then(|p| p.as_str())
            .and_then(|p| decode_base64(p).ok())
        else {
            continue;
        };
        if payload.starts_with(b"MThd") {
            return Err(
                "引擎返回的是二进制 MIDI 而不是 ABC——这条链路要的是符号谱文本，先别用它".into(),
            );
        }
        if let Ok(abc) = String::from_utf8(payload) {
            if !abc.trim().is_empty() {
                return Ok(abc);
            }
        }
    }

    // 有些任务把文本放在顶层 `text`。
    if let Some(text) = value.get("text").and_then(|t| t.as_str()) {
        if !text.trim().is_empty() {
            return Ok(text.to_string());
        }
    }
    Err(format!("引擎没有给出谱面（{}）", brief(text)))
}

/// 同步跑一次转谱（界面异步包装 / MCP 共用）。
///
/// 会把写歌引擎停掉腾显存——**转谱和写歌不并行**，顺序永远是「先转谱，再按谱重唱」。
pub fn transcribe_blocking(
    app: &tauri::AppHandle,
    args: TranscribeArgs,
) -> Result<TranscribeOutcome, String> {
    let _busy = BusyGuard::acquire()?;
    cancel_flag().store(false, Ordering::Relaxed);
    let started = Instant::now();

    let entry = catalog::by_id(MODEL_ID).ok_or("清单里没有 SheetSage2 条目")?;
    let Some(model_dir) = catalog::group_dir(entry) else {
        return Err("还没下载 SheetSage2 权重（管理器 → 模型 → SheetSage2，约 2.5 GB）".into());
    };
    let Some(pack) = crate::yue::effective_pack() else {
        return Err("还没装 audio.cpp 引擎（下载管理器 → 运行时 → audio.cpp 引擎）".into());
    };
    if !engine::installed(pack) {
        return Err(format!("{} 没装完整，去下载管理器重装一次", pack.label));
    }

    let (samples, channels, sample_rate) =
        load_input(app, args.audio.as_deref(), args.track, args.clip)?;
    if samples.is_empty() {
        return Err("这段音频是空的".into());
    }
    let seconds = (samples.len() / channels.max(1)) as f32 / sample_rate.max(1.0) as f32;

    // 先停写歌引擎：两份大权重同时占显存会 OOM。
    let _ = crate::yue_engine::ai_yue_engine_stop(app.clone());

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let dir = crate::paths::data_dir(app)?.join("local-ai");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let wav = dir.join(format!("transcribe-{stamp}.wav"));
    write_wav(&wav, &samples, channels, sample_rate)?;

    let outcome = run_engine(pack, &model_dir, &wav);
    let _ = std::fs::remove_file(&wav);

    let abc = outcome?;
    let score = dir.join(format!("transcribe-{stamp}.abc"));
    std::fs::write(&score, &abc).map_err(|e| format!("写谱面失败：{e}"))?;

    Ok(TranscribeOutcome {
        score: score.to_string_lossy().into_owned(),
        chars: abc.chars().count(),
        abc,
        seconds,
        elapsed: started.elapsed().as_secs_f32(),
    })
}

/// 转谱（界面；分钟级，异步跑在阻塞线程池里）。
#[tauri::command]
pub async fn ai_yue_transcribe(
    app: tauri::AppHandle,
    args: TranscribeArgs,
) -> Result<TranscribeOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || transcribe_blocking(&app, args))
        .await
        .map_err(|e| format!("转谱线程崩了：{e}"))?
}
