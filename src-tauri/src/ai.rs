//! 本地 AI 任务：后台线程里跑 DSP，带**进度**与**取消**。
//!
//! 两条能力（`crates/local-ai`）：`to_midi`（音频转 MIDI）、`enhance`（降噪）。
//! 都不依赖外部模型，因此**随应用即可用**；重活放在后台线程，界面显示进度条并随时可停。
//!
//! 结果落地都走**同一条命令管线**（可撤销）：转 MIDI → `AddNoteTrack`（一次事务）；
//! 降噪 → 写 WAV + 素材缓存 + `AddAudioClip`。

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{Emitter, Manager};

use ai_daw_engine::audio::DecodedAudio;
use ai_daw_engine::command::Command;
use ai_daw_engine::model::{AudioSource, Note, TrackId};

use crate::state::AppState;

/// AI 状态广播的前端事件名。
pub const EVENT_AI: &str = "engine://ai";

/// 任务类型。
pub const KIND_TO_MIDI: &str = "to_midi";
/// 人声降噪 / 去混响：优先用 LocalVQE 模型（16 kHz 语音模型）。
pub const KIND_ENHANCE: &str = "enhance";
/// 乐器 / 素材底噪降噪：**始终走内置 DSP**（全带宽，只在接近底噪处衰减）。
pub const KIND_ENHANCE_DSP: &str = "enhance_dsp";

/// 降噪的过减系数（越大越激进）。
const ENHANCE_STRENGTH: f32 = 2.0;

/// 广播给界面的任务状态。
#[derive(Debug, Clone, serde::Serialize)]
pub struct AiStatus {
    /// 是否有任务在跑。
    pub running: bool,
    /// 正在跑的任务类型（`to_midi` / `enhance`）。
    pub kind: Option<String>,
    /// 进度（0.0..=1.0）。
    pub progress: f32,
    /// 结束后的一句话（成功摘要或错误原因）。
    pub message: Option<String>,
}

/// 任务运行时（共享句柄；克隆很便宜）。
#[derive(Clone)]
pub struct AiRuntime {
    running: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU32>,
    kind: Arc<Mutex<Option<String>>>,
    message: Arc<Mutex<Option<String>>>,
}

impl Default for AiRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl AiRuntime {
    /// 空状态。
    pub fn new() -> Self {
        AiRuntime {
            running: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(AtomicU32::new(0)),
            kind: Arc::new(Mutex::new(None)),
            message: Arc::new(Mutex::new(None)),
        }
    }

    /// 当前状态快照。
    pub fn status(&self) -> AiStatus {
        AiStatus {
            running: self.running.load(Ordering::Relaxed),
            kind: self.kind.lock().unwrap().clone(),
            progress: self.progress.load(Ordering::Relaxed) as f32 / 1000.0,
            message: self.message.lock().unwrap().clone(),
        }
    }
}

/// 取当前任务状态。
#[tauri::command]
pub fn ai_status(state: tauri::State<'_, AppState>) -> AiStatus {
    state.ai.status()
}

/// 两个本地 AI 模型是否已经就位（界面据此显示「模型 / 内置 DSP」）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct AiModels {
    /// Basic Pitch 权重是否已下载。
    pub basic_pitch: bool,
    /// LocalVQE 权重是否已下载。
    pub localvqe: bool,
}

/// 查询本地 AI 模型的可用情况（权重放在 `models/` 目录，见 `tools/fetch-models.sh`）。
#[tauri::command]
pub fn ai_models() -> AiModels {
    let available = ai_daw_local_ai::models::availability();
    AiModels {
        basic_pitch: available.basic_pitch.is_some(),
        localvqe: available.localvqe.is_some(),
    }
}

/// 取消当前任务（下一次进度回调时停下，已处理的部分保留/丢弃由任务自己决定）。
#[tauri::command]
pub fn ai_cancel(state: tauri::State<'_, AppState>) {
    state.ai.cancel.store(true, Ordering::Relaxed);
}

/// 开始一个本地 AI 任务：作用于 `track` 上**第一个音频片段**。
#[tauri::command]
pub fn ai_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    kind: String,
    track: u64,
) -> Result<(), String> {
    if kind != KIND_TO_MIDI && kind != KIND_ENHANCE && kind != KIND_ENHANCE_DSP {
        return Err(format!("未知的 AI 任务：{kind}"));
    }
    if state.ai.running.swap(true, Ordering::SeqCst) {
        return Err("已有 AI 任务在跑".into());
    }
    // 先把要用的输入取出来（锁只在此刻短暂持有）。
    // 除了素材本身，还要记下**源轨道在工程里的行号**：两条能力都要把结果轨道插在它下面。
    let (source, clip_start, clip_length, track_name, track_index) = {
        let engine = state.engine.lock().unwrap();
        let project = engine.project();
        let index = project
            .tracks
            .iter()
            .position(|t| t.id == TrackId(track))
            .ok_or("轨道不存在")?;
        let t = &project.tracks[index];
        let clip = t
            .clips
            .iter()
            .find(|c| c.audio.is_some())
            .ok_or("这条轨道没有音频片段（先录音或导入音频）")?;
        (
            clip.audio.clone().expect("已确认是音频片段"),
            clip.start,
            clip.length,
            t.name.clone(),
            index,
        )
    };

    let runtime = state.ai.clone();
    runtime.cancel.store(false, Ordering::Relaxed);
    runtime.progress.store(0, Ordering::Relaxed);
    *runtime.kind.lock().unwrap() = Some(kind.clone());
    *runtime.message.lock().unwrap() = None;

    let app2 = app.clone();
    let runtime_thread = runtime.clone();
    std::thread::Builder::new()
        .name("ai-daw-ai".into())
        .spawn(move || {
            let outcome = run_job(
                &app2,
                &runtime_thread,
                &kind,
                track_index,
                &source,
                clip_start,
                clip_length,
                &track_name,
            );
            {
                let mut msg = runtime_thread.message.lock().unwrap();
                *msg = Some(match &outcome {
                    Ok(text) => text.clone(),
                    Err(err) => format!("失败：{err}"),
                });
            }
            runtime_thread.running.store(false, Ordering::SeqCst);
            runtime_thread.progress.store(0, Ordering::Relaxed);
            let _ = app2.emit(EVENT_AI, runtime_thread.status());
        })
        .map_err(|e| e.to_string())?;

    let _ = app.emit(EVENT_AI, runtime.status());
    Ok(())
}

/// 后台线程里跑的实际工作。
#[allow(clippy::too_many_arguments)]
fn run_job(
    app: &tauri::AppHandle,
    runtime: &AiRuntime,
    kind: &str,
    track_index: usize,
    source: &AudioSource,
    clip_start: f64,
    clip_length: f64,
    track_name: &str,
) -> Result<String, String> {
    let state = app.state::<AppState>();
    let audio = state.assets.load(&source.path)?;
    let channels = audio.channels;
    let sample_rate = audio.sample_rate;
    let frames = audio.frames();
    let samples: Vec<f32> = audio.samples.to_vec();
    if frames == 0 || sample_rate <= 0.0 {
        return Err("素材是空的".into());
    }

    // 进度回调：更新原子量 + 广播 + 检查取消。
    let rt = runtime.clone();
    let app_cb = app.clone();
    let progress = move |p: f32| -> bool {
        rt.progress.store((p.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
        let _ = app_cb.emit(EVENT_AI, rt.status());
        !rt.cancel.load(Ordering::Relaxed)
    };

    if kind == KIND_TO_MIDI {
        let (detected, backend) = detect_notes(&samples, channels, sample_rate, &progress);
        if detected.is_empty() {
            return Ok("没有检测到音符（这段素材可能是静音或复音）".into());
        }
        // 素材秒 → 片段内的拍：素材铺满片段跨度，因此按比例换算。
        let source_seconds = frames as f64 / sample_rate;
        let beats_per_second = clip_length / source_seconds;
        let notes: Vec<Note> = detected
            .iter()
            .map(|n| Note {
                pitch: n.pitch,
                start: n.start_sec * beats_per_second,
                duration: (n.duration_sec * beats_per_second).max(Note::MIN_DURATION_BEATS),
                velocity: n.velocity,
                volume: Note::MAX_VOLUME,
            })
            .collect();
        let count = notes.len();
        let name = format!("{track_name} · MIDI");
        // 插到**源轨道的下一行**（而不是最末尾）：转 MIDI 的直觉是「结果就贴在原轨下面」。
        let _ = commands_run(
            app,
            Command::AddNoteTrack {
                name,
                start: clip_start,
                length: clip_length,
                notes,
                at: Some(track_index + 1),
            },
        );
        return Ok(format!("已生成 {count} 个音符（{backend}）"));
    }

    // 降噪：产出新素材，再把它作为**下面一条新轨道**放进去（原轨不动，便于对比）。
    let (processed, backend) = enhance_audio(
        &samples,
        channels,
        sample_rate,
        kind == KIND_ENHANCE_DSP,
        &progress,
    );
    if runtime.cancel.load(Ordering::Relaxed) {
        return Ok("已取消".into());
    }
    let path = ai_output_path(app, "denoise")?;
    {
        let mut buffer = ai_daw_engine::audio::AudioBuffer::silence(channels, frames);
        let len = buffer.data().len();
        buffer.data_mut().copy_from_slice(&processed[..len]);
        let rate = sample_rate.round().max(1.0) as u32;
        ai_daw_engine::wav::write_wav(&path, &buffer, rate).map_err(|e| e.to_string())?;
    }
    state.assets.insert(
        &path,
        Arc::new(DecodedAudio { samples: processed.into(), channels, sample_rate }),
    );
    // 降噪：把处理结果放在**源轨道下面**的一条新轨道上（与转 MIDI 同一套做法），
    // 原轨保持不动，方便两条轨道对比试听。
    //
    // 素材窗口原样沿用源片段（`offset_frames` / `frames`）：处理的是整个文件，新文件的
    // 长度与采样率与旧文件逐帧一致，所以同样的窗口 = 同样的时间跨度与速度。
    let suffix = if kind == KIND_ENHANCE_DSP { "底噪降噪" } else { "人声降噪" };
    let _ = commands_run(
        app,
        Command::AddAudioTrack {
            name: format!("{track_name} · {suffix}"),
            start: clip_start,
            length: clip_length,
            source: AudioSource {
                path,
                offset_frames: source.offset_frames,
                frames: source.frames,
            },
            at: Some(track_index + 1),
        },
    );
    Ok(format!("已生成{suffix}轨道（{backend}）"))
}

/// 音频 → MIDI：**有 Basic Pitch 权重就优先用模型**，没有或加载失败则退回内置 DSP。
///
/// 返回 `(音符, 实际使用的后端名字)`；名字会写进任务摘要，方便确认到底走了哪条路径。
fn detect_notes(
    samples: &[f32],
    channels: usize,
    sample_rate: f64,
    progress: ai_daw_local_ai::Progress<'_>,
) -> (Vec<ai_daw_local_ai::pitch::DetectedNote>, &'static str) {
    use ai_daw_local_ai::models;
    if models::basic_pitch::model_path().is_some() {
        match models::basic_pitch::transcribe(samples, channels, sample_rate, progress) {
            Ok(notes) => return (notes, "Basic Pitch 模型"),
            Err(e) => eprintln!("[ai] Basic Pitch 不可用，退回内置 DSP：{e}"),
        }
    }
    (
        ai_daw_local_ai::pitch::detect_notes(samples, channels, sample_rate, progress),
        "内置 DSP",
    )
}

/// 降噪：**人声走模型、乐器走内置 DSP**，两者都放到源轨道下面的一条新轨道上。
///
/// 为什么分两条：LocalVQE 是 16 kHz 的**语音**模型（DNS/AEC 训练），两条副作用叠加起来会
/// 「把乐器都去掉」——① 16 kHz 意味着 8 kHz 以上的内容直接没了；② 它的掩码判的是
/// 语音/非语音，持续的音乐（pad、弦乐、失真吉他）会被当成噪声压掉。
/// 内置 DSP 是全带宽的宽带扩展器，只在能量接近底噪的位置衰减，对音乐温和得多。
fn enhance_audio(
    samples: &[f32],
    channels: usize,
    sample_rate: f64,
    force_dsp: bool,
    progress: ai_daw_local_ai::Progress<'_>,
) -> (Vec<f32>, &'static str) {
    use ai_daw_local_ai::models;
    if !force_dsp && models::localvqe::model_path().is_some() {
        match models::localvqe::enhance(samples, channels, sample_rate, progress) {
            Ok(out) => return (out, "LocalVQE 模型"),
            Err(e) => eprintln!("[ai] LocalVQE 不可用，退回内置 DSP：{e}"),
        }
    }
    (
        ai_daw_local_ai::denoise::denoise(samples, channels, sample_rate, ENHANCE_STRENGTH, progress),
        "内置 DSP",
    )
}

/// 走与界面/代理同一条命令管线（会广播事件、进撤销历史）。
fn commands_run(app: &tauri::AppHandle, command: Command) -> Result<(), String> {
    // `run_agent` 署名 `Agent("mcp")`；这里也用代理署名——它是「AI 动作」，会点亮 AI 指示。
    let _ = crate::commands::run_agent(app, command);
    Ok(())
}

/// AI 产出的文件路径：应用数据目录 `local-ai/<kind>-<epochms>.wav`。
fn ai_output_path(app: &tauri::AppHandle, kind: &str) -> Result<String, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("拿不到应用数据目录：{e}"))?
        .join("local-ai");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    Ok(dir.join(format!("{kind}-{stamp}.wav")).to_string_lossy().into_owned())
}
