//! **ASR 抓歌词**：参考音频 → 文本（翻唱页的「获取歌词」）。
//!
//! 流水线：参考音频（工程内轨道片段 / 音频文件）→（默认）HT-Demucs 取人声 → 一次性 ASR 引擎
//! 进程 → 文本 → 按句读断行。三档模型都在清单里（`feature = "lyrics"`）：
//! Qwen3-ASR 1.7B / 0.6B、CrisperWhisper 2.0。只收听得懂「唱」的模型——纯语音 ASR
//! （如 SenseVoice-Small）抓歌词会听成念白，所以不在清单里。
//!
//! 和转谱同一套形态：**先停写歌引擎**（两份大权重同时占显存会 OOM），起一份只含 ASR 的配置，
//! 跑完就杀；两者共用 [`crate::yue_transcribe::BusyGuard`]，同一时刻只跑一个。
//!
//! 歌词只覆盖**文本**这一个用途：结果是给歌词框的初稿，用户随时能改。

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ai_daw_local_ai::engine;
use ai_daw_local_ai::models::{self, catalog};

/// 三档 ASR：清单条目 id → 引擎里的模型族。
struct AsrModel {
    id: &'static str,
    family: &'static str,
}

const MODELS: &[AsrModel] = &[
    AsrModel {
        id: "qwen3-asr-1.7b",
        family: "qwen3_asr",
    },
    AsrModel {
        id: "qwen3-asr-0.6b",
        family: "qwen3_asr",
    },
    AsrModel {
        id: "crisperwhisper-2-large",
        family: "crisperwhisper",
    },
];

/// 默认档：中文/多语都稳，且它的规格自述对「singing audio」更鲁棒。
pub const DEFAULT_MODEL: &str = "qwen3-asr-1.7b";

/// 抓歌词的输入（界面 / MCP 共用）。
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct LyricsArgs {
    /// 音频文件路径；与 `track` 二选一，同时给时以它为准。
    pub audio: Option<String>,
    /// 用这条轨道的第一个音频片段。
    pub track: Option<u64>,
    /// 指定片段 id。
    pub clip: Option<u64>,
    /// 模型条目 id（缺省 = [`DEFAULT_MODEL`]）。
    pub model: Option<String>,
    /// 先分离人声再听（默认开；没有分离权重时自动退回整轨）。
    pub separate: Option<bool>,
}

/// 抓歌词的结果。
#[derive(Debug, Clone, serde::Serialize)]
pub struct LyricsOutcome {
    /// 整理后的歌词（按行，`\n` 分隔）：直接填歌词框。
    pub text: String,
    /// 分行结果。
    pub lines: Vec<String>,
    /// 音节数（汉字逐字 + 西文按词）：翻唱要按这个对旋律。
    pub syllables: usize,
    /// 输入音频时长（秒）。
    pub seconds: f32,
    /// 用时（秒）。
    pub elapsed: f32,
    /// 实际用的模型条目 id。
    pub model: String,
    /// 是否真的先跑了人声分离。
    pub separated: bool,
}

/// 正在跑的 ASR 子进程（取消 = 杀它）。
fn child_slot() -> &'static Mutex<Option<Child>> {
    static CHILD: OnceLock<Mutex<Option<Child>>> = OnceLock::new();
    CHILD.get_or_init(|| Mutex::new(None))
}

fn cancel_flag() -> &'static AtomicBool {
    static CANCEL: OnceLock<AtomicBool> = OnceLock::new();
    CANCEL.get_or_init(|| AtomicBool::new(false))
}

/// 取消当前抓歌词（杀掉一次性引擎进程；阻塞中的请求会立刻返回失败）。
#[tauri::command]
pub fn ai_yue_lyrics_cancel() {
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
// 引擎
// ---------------------------------------------------------------------------

/// 一次性 ASR 引擎的配置（只有一个模型）。
fn write_config(
    device: &str,
    family: &str,
    model_dir: &PathBuf,
    port: u16,
) -> Result<PathBuf, String> {
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
            "id": family,
            "family": family,
            "task": "asr",
            "mode": "offline",
            "path": model_dir.to_string_lossy(),
        }],
    });
    let path = engine::dir().join("lyrics.json");
    let text = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("写抓歌词配置失败：{e}"))?;
    Ok(path)
}

fn log_tail(lines: usize) -> String {
    let path = engine::dir().join("lyrics.log");
    let Ok(file) = std::fs::File::open(path) else {
        return String::new();
    };
    let mut all: Vec<String> = BufReader::new(file).lines().map_while(Result::ok).collect();
    let start = all.len().saturating_sub(lines);
    all.drain(..start);
    all.join("\n")
}

/// 起引擎、等 `/health`、提交 ASR、拿文本、杀进程。
fn run_engine(
    pack: &engine::EnginePack,
    family: &str,
    model_dir: &PathBuf,
    wav: &std::path::Path,
) -> Result<String, String> {
    engine::ensure_asr_spec(pack, family)?;
    let port = crate::yue_engine::free_port()?;
    let config = write_config(crate::yue::effective_device(), family, model_dir, port)?;
    let dir = engine::dir_of(pack);

    let log_path = engine::dir().join("lyrics.log");
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
                    return Err(format!(
                        "抓歌词引擎启动后立刻退出（{code}）：{}",
                        log_tail(6)
                    ));
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
            return Err("抓歌词引擎 10 分钟还没就绪（看日志：runtime/audiocpp/lyrics.log）".into());
        }
        std::thread::sleep(Duration::from_millis(700));
    }

    // 分块方式固定（见 `task_body`）：不能吃规格里的默认 `auto`，那会去加载没随包发布的 VAD。
    let body = task_body(family, wav);
    let work = crate::yue_engine::work_agent();
    let resp = work
        .post(&format!("http://127.0.0.1:{port}/v1/tasks/run"))
        .header("content-type", "application/json")
        .send(body.as_bytes());
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
                parse_text(&text)
            }
        }
        Err(e) => {
            if cancel_flag().load(Ordering::Relaxed) {
                Err("已取消".into())
            } else {
                Err(format!("提交抓歌词任务失败：{e}"))
            }
        }
    };

    kill_child();
    result
}

/// `/v1/tasks/run` 的请求体。
///
/// 分块方式**必须显式给「固定窗口」**：规格里的默认值是 `auto`，引擎会去找
/// `assets/framework/models/silero_vad` 那套 VAD 资源——发布包没带，于是任务直接 500
/// （`no VAD model built into this binary can load ...`）。固定 30 秒窗口对歌词够用，
/// 且不依赖任何额外文件。
fn task_body(family: &str, wav: &std::path::Path) -> String {
    serde_json::json!({
        "model": family,
        "request": {
            "audio": wav.to_string_lossy(),
            "options": {
                "audio_chunk_mode": "fixed",
                "audio_chunk_duration_sec": 30.0,
            },
        },
    })
    .to_string()
}

fn brief(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= 300 {
        return text.to_string();
    }
    text.chars().take(300).collect::<String>() + "…"
}

/// 从 `/v1/tasks/run` 的结果里取识别文本。
///
/// ASR 的 artifact 字段各家不统一（`text` / `meta.text` / 一段 base64 payload），所以按
/// 「先看文本字段，再看 payload」的次序一起认；都取不到就把原文前 300 字符报出来。
fn parse_text(body: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("引擎回了怪东西：{e}（{}）", brief(body)))?;
    if let Some(message) = value
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
    {
        return Err(format!("引擎报错：{message}"));
    }

    let mut found = String::new();
    let empty = Vec::new();
    for item in value
        .get("artifacts")
        .and_then(|a| a.as_array())
        .unwrap_or(&empty)
    {
        for key in ["text", "transcript"] {
            if let Some(text) = item.get(key).and_then(|t| t.as_str()) {
                if !text.trim().is_empty() {
                    found.push_str(text);
                    found.push('\n');
                }
            }
        }
        if let Some(text) = item
            .get("meta")
            .and_then(|m| m.get("text"))
            .and_then(|t| t.as_str())
        {
            if !text.trim().is_empty() {
                found.push_str(text);
                found.push('\n');
            }
        }
        if found.trim().is_empty() {
            if let Some(payload) = item
                .get("payload")
                .and_then(|p| p.as_str())
                .and_then(|p| crate::yue_transcribe::decode_base64(p).ok())
                .and_then(|bytes| String::from_utf8(bytes).ok())
            {
                // 二进制（音频等）不是文本：UTF-8 解出来通常满是替换符，直接丢掉。
                if !payload.trim().is_empty() && !payload.contains('\u{FFFD}') {
                    found.push_str(&payload);
                    found.push('\n');
                }
            }
        }
    }

    if found.trim().is_empty() {
        for key in ["text", "transcript"] {
            if let Some(text) = value.get(key).and_then(|t| t.as_str()) {
                found.push_str(text);
            }
        }
    }

    if found.trim().is_empty() {
        return Err(format!("引擎没有给出文本（{}）", brief(body)));
    }
    Ok(found)
}

// ---------------------------------------------------------------------------
// 文本整理
// ---------------------------------------------------------------------------

/// 把 ASR 的原始输出整理成「按行」的歌词。
///
/// 做四件事：去 ASR 输出里的 `<|event|>` 控制标签、去 `[Music]` / `♪` 这类幻觉标记、按句读断行、
/// 过长的行再按逗号 / 硬宽度切开。翻唱要的是「一行一句」，所以断行结果直接决定后续能不能唱。
fn clean_lines(raw: &str) -> Vec<String> {
    let mut text = String::with_capacity(raw.len());
    // 0 = 正文，1 = `<|tag|>` 里，2 = `[Music]` / `【音乐】` 里：后两种整段丢掉。
    let mut mode = 0u8;
    for ch in raw.chars() {
        match mode {
            0 => match ch {
                '<' => mode = 1,
                '[' | '【' => mode = 2,
                _ => text.push(ch),
            },
            1 => {
                if ch == '>' {
                    mode = 0;
                }
            }
            _ => {
                if ch == ']' || ch == '】' {
                    mode = 0;
                }
            }
        }
    }

    let mut lines: Vec<String> = Vec::new();
    for raw_line in text.split(['\n', '。', '！', '？', '!', '?', '；', ';']) {
        let line = raw_line
            .chars()
            .filter(|c| !matches!(c, '♪' | '♫' | '#' | '*' | '|' | '_'))
            .collect::<String>();
        let line = line.trim().trim_matches(['、', '，', ',', ':', '：', '—', '-']).trim();
        if line.is_empty() || line.chars().all(|c| !c.is_alphanumeric() && !is_cjk(c)) {
            continue;
        }
        push_wrapped(&mut lines, line);
    }
    lines
}

/// 太长的一句（ASR 常常一口气几十秒不给标点）按逗号切开，再不行按宽度硬切。
fn push_wrapped(out: &mut Vec<String>, line: &str) {
    const MAX: usize = 24;
    if line.chars().count() <= MAX {
        out.push(line.to_string());
        return;
    }
    let mut chunk = String::new();
    for part in line.split(['，', ',', '、']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if !chunk.is_empty() && chunk.chars().count() + part.chars().count() > MAX {
            out.push(std::mem::take(&mut chunk));
        }
        if part.chars().count() > MAX {
            for piece in part.chars().collect::<Vec<_>>().chunks(MAX) {
                out.push(piece.iter().collect());
            }
            continue;
        }
        chunk.push_str(part);
    }
    if !chunk.is_empty() {
        out.push(chunk);
    }
    // 全切碎时要保证至少有一行，别把内容丢了。
    if out.is_empty() {
        out.push(line.to_string());
    }
}

fn is_cjk(c: char) -> bool {
    matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}')
}

/// 音节数：汉字逐字算，西文按词算。翻唱要求「词与旋律逐行对得上」，这个数给用户看个量级。
fn syllables(lines: &[String]) -> usize {
    let mut count = 0;
    for line in lines {
        let mut in_word = false;
        for c in line.chars() {
            if is_cjk(c) {
                count += 1;
                in_word = false;
            } else if c.is_alphabetic() {
                if !in_word {
                    count += 1;
                    in_word = true;
                }
            } else {
                in_word = false;
            }
        }
    }
    count
}

// ---------------------------------------------------------------------------
// 输入：可选的人声分离
// ---------------------------------------------------------------------------

/// 只取人声轨：6 分轨优先，没有就 4 分轨；都没有返回 `None`（退回整轨，别让按钮整体失败）。
fn vocals_only(samples: &[f32], channels: usize, sample_rate: f64) -> Option<Vec<f32>> {
    let stems = models::demucs6s::separate(samples, channels, sample_rate, &|_| true)
        .or_else(|_| models::demucs::separate(samples, channels, sample_rate, &|_| true))
        .ok()?;
    let (_, vocals) = stems.stems.into_iter().find(|(name, _)| *name == "vocals")?;
    if vocals.is_empty() {
        return None;
    }
    Some(vocals)
}

// ---------------------------------------------------------------------------
// 命令
// ---------------------------------------------------------------------------

/// 同步跑一次抓歌词（界面异步包装 / MCP 共用）。
pub fn lyrics_blocking(
    app: &tauri::AppHandle,
    args: LyricsArgs,
) -> Result<LyricsOutcome, String> {
    let _busy = crate::yue_transcribe::BusyGuard::acquire()?;
    cancel_flag().store(false, Ordering::Relaxed);
    let started = Instant::now();

    let id = args.model.clone().unwrap_or_else(|| DEFAULT_MODEL.to_string());
    let model = MODELS
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("没有这个 ASR 档位：{id}"))?;
    let entry = catalog::by_id(model.id).ok_or("清单里没有这条 ASR 条目")?;
    let Some(model_dir) = catalog::group_dir(entry) else {
        return Err(format!(
            "还没下载「{}」权重（管理器 → 模型）",
            entry.name
        ));
    };
    let Some(pack) = crate::yue::effective_pack() else {
        return Err("还没装 audio.cpp 引擎（下载管理器 → 运行时 → audio.cpp 引擎）".into());
    };
    if !engine::installed(pack) {
        return Err(format!("{} 没装完整，去下载管理器重装一次", pack.label));
    }

    let (mut samples, channels, sample_rate) = crate::yue_transcribe::load_input(
        app,
        args.audio.as_deref(),
        args.track,
        args.clip,
    )?;
    if samples.is_empty() {
        return Err("这段音频是空的".into());
    }
    let seconds = (samples.len() / channels.max(1)) as f32 / sample_rate.max(1.0) as f32;

    // 混音直接听词准确率差：默认先取人声轨（没有分离权重就照旧用整轨）。
    let mut separated = false;
    if args.separate.unwrap_or(true) {
        if let Some(vocals) = vocals_only(&samples, channels, sample_rate) {
            samples = vocals;
            separated = true;
        }
    }

    // 先停写歌引擎：两份权重同时占显存会 OOM。
    let _ = crate::yue_engine::ai_yue_engine_stop(app.clone());

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let dir = crate::paths::data_dir(app)?.join("local-ai");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let wav = dir.join(format!("lyrics-{stamp}.wav"));
    crate::yue_transcribe::write_wav(&wav, &samples, channels, sample_rate)?;

    let outcome = run_engine(pack, model.family, &model_dir, &wav);
    let _ = std::fs::remove_file(&wav);

    let lines = clean_lines(&outcome?);
    if lines.is_empty() {
        return Err("这段音频里没听出词（纯音乐？或者换一档模型试试）".into());
    }
    Ok(LyricsOutcome {
        text: lines.join("\n"),
        syllables: syllables(&lines),
        lines,
        seconds,
        elapsed: started.elapsed().as_secs_f32(),
        model: model.id.to_string(),
        separated,
    })
}

/// 抓歌词（界面；分钟级，异步跑在阻塞线程池里）。
#[tauri::command]
pub async fn ai_yue_lyrics(
    app: tauri::AppHandle,
    args: LyricsArgs,
) -> Result<LyricsOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || lyrics_blocking(&app, args))
        .await
        .map_err(|e| format!("抓歌词线程崩了：{e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_body_forces_fixed_chunking() {
        // 回归：不给 `audio_chunk_mode` 时引擎会去加载没随包发布的 Silero VAD，直接 500。
        let body = task_body("qwen3_asr", std::path::Path::new("D:/tmp/a.wav"));
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["model"], "qwen3_asr");
        assert_eq!(value["request"]["audio"], "D:/tmp/a.wav");
        assert_eq!(value["request"]["options"]["audio_chunk_mode"], "fixed");
        assert_eq!(value["request"]["options"]["audio_chunk_duration_sec"], 30.0);
    }

    #[test]
    fn tags_and_markers_are_stripped() {
        let lines = clean_lines("<|zh|><|NEUTRAL|><|Speech|>月亮代表我的心。[Music]♪");
        assert_eq!(lines, vec!["月亮代表我的心"]);
    }

    #[test]
    fn sentences_become_lines() {
        let lines = clean_lines("第一句。第二句！第三句？");
        assert_eq!(lines, vec!["第一句", "第二句", "第三句"]);
    }

    #[test]
    fn long_lines_are_wrapped() {
        let raw = "这一句特别长没有标点也没有逗号所以只能按宽度硬切开才能当成一行一句来唱下去";
        let lines = clean_lines(raw);
        assert!(lines.len() > 1, "{lines:?}");
        for line in &lines {
            assert!(line.chars().count() <= 24, "{line}");
        }
    }

    #[test]
    fn syllables_count_cjk_per_char_and_latin_per_word() {
        let lines = vec!["月亮代表我的心".to_string(), "my heart will go on".to_string()];
        assert_eq!(syllables(&lines), 7 + 5);
    }

    #[test]
    fn empty_noise_is_not_lyrics() {
        assert!(clean_lines("♪ ♪ [Applause]").is_empty());
    }
}
