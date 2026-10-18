//! 本地 AI 任务：后台线程里跑 DSP / 模型，带**进度**与**取消**。
//!
//! 能力（`crates/local-ai`）：`to_midi`（音频转 MIDI）、`enhance`（降噪）、`separate`（音源分离）、
//! `master`（母带）、`detect_tempo`（节拍检测）。重活放在后台线程，界面显示进度条并随时可停。
//!
//! 结果落地都走**同一条命令管线**（可撤销）：转 MIDI → `AddNoteTrack`（一次事务）；
//! 降噪 / 音源分离 → 写 WAV + 素材缓存 + `AddAudioTrack`（分离一次建四条轨，仍是一条撤销记录）；
//! 母带 → 整个工程离线渲染后落到一条新音频轨。

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{Emitter, Manager};

use ai_daw_engine::audio::{AudioBuffer, DecodedAudio};
use ai_daw_engine::command::Command;
use ai_daw_engine::event::Event;
use ai_daw_engine::model::{AudioSource, ClipId, Note, TrackId};

use crate::state::AppState;

/// AI 状态广播的前端事件名。
pub const EVENT_AI: &str = "engine://ai";

/// 任务类型。
pub const KIND_TO_MIDI: &str = "to_midi";
/// 人声降噪 / 去混响：优先用 LocalVQE 模型（16 kHz 语音模型）。
pub const KIND_ENHANCE: &str = "enhance";
/// 乐器 / 素材底噪降噪：**始终走内置 DSP**（全带宽，只在接近底噪处衰减）。
pub const KIND_ENHANCE_DSP: &str = "enhance_dsp";
/// 母带处理：**作用于整个工程的离线混音**（不针对某条轨道），纯 DSP、无需模型。
pub const KIND_MASTER: &str = "master";
/// 音源分离：把所选轨道的音频拆成**鼓 / 贝斯 / 其他 / 人声**四条分轨（HTDemucs 模型，无 DSP 兜底）。
pub const KIND_SEPARATE: &str = "separate";
/// 节拍检测：用 **Beat This!** 推出**单一全局 BPM + 拍号**，一次事务写进工程。
///
/// 为什么是「全局一个速度」而不是逐小节速度图：时间轴按「拍」**线性绘制**，只有恒定速度下
/// 网格 / 波形才稳定对齐（上一版写速度图，结果永远对不上——见 `models/beat_this.rs`）。
pub const KIND_TEMPO: &str = "detect_tempo";

/// 降噪的过减系数（越大越激进）。
const ENHANCE_STRENGTH: f32 = 2.0;

/// 母带渲染用的采样率（与其余离线路径一致）。
const MASTER_SAMPLE_RATE: u32 = 44_100;
/// 母带渲染保留的尾音秒数（混响 / 延迟的尾巴不能被切掉）。
const MASTER_TAIL_SECONDS: f64 = 0.5;

/// 母带参数（来自界面；都省略时用 crate 的默认值）。
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct MasterArgs {
    /// 目标积分响度（LUFS）。
    pub target_lufs: Option<f64>,
    /// 真峰值上限（dBTP）。
    pub ceiling_dbtp: Option<f64>,
    /// 立体声宽度（1.0 = 原样，0 = 单声道）。
    pub stereo_width: Option<f32>,
}

impl MasterArgs {
    /// 缺省项补上默认值（范围由 `MasterOptions::sanitized` 兜底）。
    fn into_options(self) -> ai_daw_mastering::MasterOptions {
        let defaults = ai_daw_mastering::MasterOptions::default();
        ai_daw_mastering::MasterOptions {
            target_lufs: self.target_lufs.unwrap_or(defaults.target_lufs),
            ceiling_dbtp: self.ceiling_dbtp.unwrap_or(defaults.ceiling_dbtp),
            stereo_width: self.stereo_width.unwrap_or(defaults.stereo_width),
        }
    }
}

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

/// 本地 AI 模型是否已经就位（界面据此显示「模型 / 内置 DSP」）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct AiModels {
    /// Basic Pitch 权重是否已下载。
    pub basic_pitch: bool,
    /// LocalVQE 权重是否已下载。
    pub localvqe: bool,
    /// HTDemucs 音源分离权重是否已下载。
    pub separate: bool,
    /// Beat This! 节拍 / 速度检测权重是否已下载（mel 前端随包 + 主模型按需）。
    pub beat_this: bool,
}

/// 查询本地 AI 模型的可用情况（权重放在 `models/` 目录，见 `tools/fetch-models.sh`）。
#[tauri::command]
pub fn ai_models() -> AiModels {
    let available = ai_daw_local_ai::models::availability();
    AiModels {
        basic_pitch: available.basic_pitch.is_some(),
        localvqe: available.localvqe.is_some(),
        separate: available.htdemucs.is_some(),
        beat_this: available.beat_this.is_some(),
    }
}

/// 取消当前任务（下一次进度回调时停下，已处理的部分保留/丢弃由任务自己决定）。

#[tauri::command]
pub fn ai_cancel(state: tauri::State<'_, AppState>) {
    state.ai.cancel.store(true, Ordering::Relaxed);
}

/// 开始一个本地任务。
///
/// `to_midi` / `enhance` / `enhance_dsp` / `separate` / `detect_tempo` 作用于 `track` 上的**第一个音频片段**；
/// `master` 作用于**整个工程的离线混音**（不看 `track`，`master` 参数只在它身上有意义）。
/// `model` 只在 `separate` 上有意义：选择用哪个分离模型（缺省 = 4 分轨的 HTDemucs）。
#[tauri::command]
pub fn ai_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    kind: String,
    track: u64,
    master: Option<MasterArgs>,
    model: Option<String>,
) -> Result<(), String> {
    if !matches!(
        kind.as_str(),
        KIND_TO_MIDI
            | KIND_ENHANCE
            | KIND_ENHANCE_DSP
            | KIND_MASTER
            | KIND_SEPARATE
            | KIND_TEMPO
    ) {
        return Err(format!("未知的 AI 任务：{kind}"));
    }
    if state.ai.running.swap(true, Ordering::SeqCst) {
        return Err("已有 AI 任务在跑".into());
    }

    let runtime = state.ai.clone();
    runtime.cancel.store(false, Ordering::Relaxed);
    runtime.progress.store(0, Ordering::Relaxed);
    *runtime.kind.lock().unwrap() = Some(kind.clone());
    *runtime.message.lock().unwrap() = None;

    // 母带：输入是整个工程，与轨道无关，走单独一条线程路径。
    if kind == KIND_MASTER {
        let options = master.unwrap_or_default().into_options();
        let thread_app = app.clone();
        let thread_runtime = runtime.clone();
        std::thread::Builder::new()
            .name("ai-daw-master".into())
            .spawn(move || {
                let outcome = run_master_job(&thread_app, &thread_runtime, &options);
                finish(&thread_app, &thread_runtime, outcome);
            })
            .map_err(|e| e.to_string())?;
        let _ = app.emit(EVENT_AI, runtime.status());
        return Ok(());
    }

    // 先把要用的输入取出来（锁只在此刻短暂持有）。找不到轨道/音频片段就报错。
    let input = track_input(&state, track)?;

    let thread_app = app.clone();
    let thread_runtime = runtime.clone();
    std::thread::Builder::new()
        .name("ai-daw-ai".into())
        .spawn(move || {
            let outcome = run_job(&thread_app, &thread_runtime, &kind, &input, model.as_deref());
            finish(&thread_app, &thread_runtime, outcome);
        })
        .map_err(|e| e.to_string())?;

    let _ = app.emit(EVENT_AI, runtime.status());
    Ok(())
}

/// 一条轨道的「第一个音频片段」输入，以及结果轨道要插入的行号。
pub(crate) struct TrackInput {
    /// 源轨道 id（结果新轨插在它下面；节拍检测要按它改片段）。
    pub track: TrackId,
    /// 源音频片段 id（节拍检测要对齐的就是它）。
    pub clip: ClipId,
    /// 源音频片段。
    pub source: AudioSource,
    /// 源片段起始拍。
    pub clip_start: f64,
    /// 源片段长度（拍）。
    pub clip_length: f64,
    /// 源轨道名。
    pub track_name: String,
    /// 源轨道在工程里的行号（结果插在它下面）。
    pub track_index: usize,
}

/// 取一条轨道上的第一个音频片段（`track` 不存在或没有音频片段都会报错）。
pub(crate) fn track_input(state: &AppState, track: u64) -> Result<TrackInput, String> {
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
    Ok(TrackInput {
        track: t.id,
        clip: clip.id,
        source: clip.audio.clone().expect("已确认是音频片段"),
        clip_start: clip.start,
        clip_length: clip.length,
        track_name: t.name.clone(),
        track_index: index,
    })
}

/// 任务收尾：写摘要、清运行标志、广播一次终态。
fn finish(app: &tauri::AppHandle, runtime: &AiRuntime, outcome: Result<String, String>) {
    {
        let mut message = runtime.message.lock().unwrap();
        *message = Some(match outcome {
            Ok(text) => text,
            Err(err) => format!("失败：{err}"),
        });
    }
    runtime.running.store(false, Ordering::SeqCst);
    runtime.progress.store(0, Ordering::Relaxed);
    let _ = app.emit(EVENT_AI, runtime.status());
}

/// 后台线程里跑的实际工作。
fn run_job(
    app: &tauri::AppHandle,
    runtime: &AiRuntime,
    kind: &str,
    input: &TrackInput,
    model: Option<&str>,
) -> Result<String, String> {
    let state = app.state::<AppState>();
    let audio = state.assets.load(&input.source.path)?;
    let channels = audio.channels;
    let sample_rate = audio.sample_rate;
    let frames = audio.frames();
    let samples: Vec<f32> = audio.samples.to_vec();
    if frames == 0 || sample_rate <= 0.0 {
        return Err("素材是空的".into());
    }
    let track_name = input.track_name.as_str();
    let track_index = input.track_index;
    let clip_start = input.clip_start;
    let clip_length = input.clip_length;
    let source = &input.source;

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
                id: 0,
                pitch: n.pitch,
                start: n.start_sec * beats_per_second,
                duration: (n.duration_sec * beats_per_second).max(Note::MIN_DURATION_BEATS),
                velocity: n.velocity,
                volume: Note::MAX_VOLUME,
                offset: 0.0,
                legato: false,
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

    // 节拍检测：推出全局 BPM + 拍号，并把片段对齐到小节线（一次事务 = 一条撤销记录）。
    if kind == KIND_TEMPO {
        return apply_tempo_detection(app, input, &samples, channels, sample_rate, &progress);
    }

    // 音源分离：按选定的模型把音频拆成 N 条新轨（4 分轨或 6 分轨），一次事务。
    // 没有模型权重时给明确错误——分离没有内置 DSP 近似。
    if kind == KIND_SEPARATE {
        let stems = separate_with_model(model, &samples, channels, sample_rate, &progress)?;
        if runtime.cancel.load(Ordering::Relaxed) {
            return Ok("已取消".into());
        }
        let count = place_stems(app, input, &stems, channels, sample_rate, frames)?;
        return Ok(format!("已生成 {count} 条分轨（HTDemucs）"));
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
                rate: 1.0,
                pitch_semitones: 0.0,
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

/// stem 名（模型输出里的稳定标识）→ 显示名。
fn stem_title(name: &str) -> &'static str {
    match name {
        "drums" => "鼓",
        "bass" => "贝斯",
        "other" => "其他",
        "vocals" => "人声",
        "guitar" => "吉他",
        "piano" => "钢琴",
        _ => "分轨",
    }
}

/// 按模型 id 选一个分离后端（缺省 = 4 分轨；未知 id 直接报错）。
fn separate_with_model(
    model: Option<&str>,
    samples: &[f32],
    channels: usize,
    sample_rate: f64,
    progress: ai_daw_local_ai::Progress<'_>,
) -> Result<ai_daw_local_ai::models::Stems, String> {
    use ai_daw_local_ai::models;
    match model.unwrap_or("htdemucs-ort-v1") {
        "" | "htdemucs-ort-v1" => models::demucs::separate(samples, channels, sample_rate, progress),
        "htdemucs-6s" => models::demucs6s::separate(samples, channels, sample_rate, progress),
        other => Err(format!("未知的音源分离模型：{other}")),
    }
}

/// 把分离出的各条分轨写成 WAV（各自缓存）并作为**源轨道下面**的新轨插入。
///
/// 一次事务提交全部 `AddAudioTrack`，因此一次撤销即可整体回退。返回新建轨道数。
fn place_stems(
    app: &tauri::AppHandle,
    input: &TrackInput,
    stems: &ai_daw_local_ai::models::Stems,
    channels: usize,
    sample_rate: f64,
    frames: usize,
) -> Result<usize, String> {
    let rate = sample_rate.round().max(1.0) as u32;

    let state = app.state::<AppState>();
    let mut commands: Vec<Command> = Vec::with_capacity(stems.stems.len());
    for (i, (name, part)) in stems.stems.iter().enumerate() {
        let file = ai_output_path(app, &format!("separate-{name}"))?;
        {
            let mut buffer = AudioBuffer::silence(channels, frames);
            let len = buffer.data().len();
            buffer.data_mut().copy_from_slice(&part[..len]);
            ai_daw_engine::wav::write_wav(&file, &buffer, rate).map_err(|e| e.to_string())?;
        }
        state.assets.insert(
            &file,
            Arc::new(DecodedAudio {
                samples: part.to_vec().into(),
                channels,
                sample_rate,
            }),
        );
        commands.push(Command::AddAudioTrack {
            name: format!("{} · {}", input.track_name, stem_title(name)),
            start: input.clip_start,
            length: input.clip_length,
            source: AudioSource {
                path: file,
                offset_frames: input.source.offset_frames,
                frames: input.source.frames,
                rate: 1.0,
                pitch_semitones: 0.0,
            },
            at: Some(input.track_index + 1 + i),
        });
    }
    let (_, events) = crate::commands::run_agent_all(app, commands);
    Ok(events
        .iter()
        .filter(|e| matches!(e, Event::TrackAdded { .. }))
        .count())
}

/// 同步分离一条轨道（供 MCP 工具用；无进度回调，整体阻塞在调用线程）。
///
/// `model` 选择模型（缺省 4 分轨）。返回新建轨道数。行为与界面一致（同一条命令管线、一次事务）。
pub(crate) fn separate_track(
    app: &tauri::AppHandle,
    track: u64,
    model: Option<String>,
) -> Result<usize, String> {
    let state = app.state::<AppState>();
    let input = track_input(&state, track)?;
    let audio = state.assets.load(&input.source.path)?;
    let channels = audio.channels;
    let sample_rate = audio.sample_rate;
    let frames = audio.frames();
    let samples: Vec<f32> = audio.samples.to_vec();
    if frames == 0 || sample_rate <= 0.0 {
        return Err("素材是空的".into());
    }
    let stems =
        separate_with_model(model.as_deref(), &samples, channels, sample_rate, &|_| true)?;
    place_stems(app, &input, &stems, channels, sample_rate, frames)
}

/// 一条音频片段在**整段素材采样**里的可见窗口（交错）。
///
/// 片段可能只取素材的一段（`offset_frames` / `frames`）；节拍检测与对齐都只该看这一段。
fn clip_window<'a>(samples: &'a [f32], channels: usize, source: &AudioSource) -> &'a [f32] {
    let channels = channels.max(1);
    let total = samples.len() / channels;
    let offset = (source.offset_frames as usize).min(total);
    let take = source
        .frames
        .map(|f| f as usize)
        .unwrap_or(total - offset)
        .min(total - offset);
    &samples[offset * channels..(offset + take) * channels]
}

/// 节拍检测并写回：**一次事务**里设速度 + 拍号 + 把片段对齐到小节线。
///
/// 对齐做三件事，全部落在同一笔撤销记录里：
/// - 速度 → 检出的 BPM，拍号 → 检出的分子 / 4（**只设全局一个值**，不写速度图）；
/// - `start` → 让**第一个重拍**落在一条小节线上（片段起点不能为负，所以吸附到**下一条**
///   小节线；没有重拍就退回 0）；
/// - `length` → 素材窗口在新速度下的自然拍长（方框与素材等长），`rate` 复位到 1.0（原速）。
fn apply_tempo_detection(
    app: &tauri::AppHandle,
    input: &TrackInput,
    samples: &[f32],
    channels: usize,
    sample_rate: f64,
    progress: ai_daw_local_ai::Progress<'_>,
) -> Result<String, String> {
    use ai_daw_local_ai::models;

    let window = clip_window(samples, channels, &input.source);
    if window.is_empty() {
        return Err("素材是空的".into());
    }
    let detection = match models::beat_this::detect(window, channels, sample_rate, progress) {
        Ok(d) => d,
        // 取消走 `progress` 回调，回来时是「已取消」而不是失败。
        Err(e) if e == "已取消" => return Ok("已取消".into()),
        Err(e) => return Err(e),
    };

    let (bpm, num, den) = (detection.bpm, detection.time_sig_num, detection.time_sig_den);
    // 一小节多少拍（四分音符为基准）：4/4 → 4，3/4 → 3。
    let bar_beats = num as f64 * (4.0 / den as f64);
    // 第一个重拍相对片段起点的拍数。
    let phase_beats = detection.first_downbeat_sec.unwrap_or(0.0) * bpm / 60.0;
    // 片段起点不能为负（会被夹到 0），所以把首重拍吸附到**下一条**小节线。
    let start = (bar_beats - phase_beats.rem_euclid(bar_beats)) % bar_beats;
    // 素材在新速度下的自然拍长：方框与素材等长，播放速率因此回到 1.0。
    let length = detection.duration_sec * bpm / 60.0;

    let _ = crate::commands::run_agent_all(
        app,
        vec![
            Command::SetTempo(bpm),
            Command::SetTimeSignature { num, den },
            Command::MoveClip { track: input.track, clip: input.clip, start },
            Command::ResizeClip { track: input.track, clip: input.clip, length },
            Command::SetClipRate { track: input.track, clip: input.clip, rate: 1.0, pitch_semitones: 0.0 },
        ],
    );

    let first_bar = ((start + phase_beats) / bar_beats).round() as i64 + 1;
    Ok(format!(
        "速度 {bpm:.1} BPM · {num}/{den}（{} 个拍点 / {} 个重拍）；首重拍对齐到第 {first_bar} 小节",
        detection.beats, detection.downbeats,
    ))
}

/// 同步跑一遍节拍检测（供 MCP 工具用；无进度回调，整体阻塞在调用线程）。
pub(crate) fn detect_tempo_track(app: &tauri::AppHandle, track: u64) -> Result<String, String> {
    let state = app.state::<AppState>();
    let input = track_input(&state, track)?;
    let audio = state.assets.load(&input.source.path)?;
    let channels = audio.channels;
    let sample_rate = audio.sample_rate;
    let samples: Vec<f32> = audio.samples.to_vec();
    if samples.is_empty() || sample_rate <= 0.0 {
        return Err("素材是空的".into());
    }
    apply_tempo_detection(app, &input, &samples, channels, sample_rate, &|_| true)
}

/// 走与界面/代理同一条命令管线（会广播事件、进撤销历史）。
fn commands_run(app: &tauri::AppHandle, command: Command) -> Result<(), String> {
    // `run_agent` 署名 `Agent("mcp")`；这里也用代理署名——它是「AI 动作」，会点亮 AI 指示。
    let _ = crate::commands::run_agent(app, command);
    Ok(())
}

/// 粗粒度进度：渲染与母带都是一整段不可打断的调用，只能报里程碑。
fn step(runtime: &AiRuntime, app: &tauri::AppHandle, progress: f32) {
    runtime
        .progress
        .store((progress.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
    let _ = app.emit(EVENT_AI, runtime.status());
}

/// **母带**：把整个工程离线渲染成混音 → 跑一遍母带链 → 落到**末尾的一条新音频轨**。
///
/// 为什么是「新轨道」而不是直接改文件：与降噪同一套理由——原混音不动，两条轨道可以 A/B
/// 试听，而且这是一次**可撤销的事务**。要交付文件就用「导出」。
///
/// 母带链本身是纯 DSP（`crates/mastering`）：响度归一 + 真峰值限幅 + 立体声宽度。
fn run_master_job(
    app: &tauri::AppHandle,
    runtime: &AiRuntime,
    options: &ai_daw_mastering::MasterOptions,
) -> Result<String, String> {
    let state = app.state::<AppState>();

    step(runtime, app, 0.05);
    if runtime.cancel.load(Ordering::Relaxed) {
        return Ok("已取消".into());
    }

    // 渲染整个工程。渲染本身不支持取消，所以这一段只能等它跑完。
    let inputs = crate::commands::render_inputs(&state);
    let project = inputs.project.clone();
    let buffer = crate::commands::render_offline(inputs, MASTER_SAMPLE_RATE, MASTER_TAIL_SECONDS);
    let channels = buffer.channels();
    step(runtime, app, 0.6);
    if runtime.cancel.load(Ordering::Relaxed) {
        return Ok("已取消".into());
    }

    let (processed, report) =
        ai_daw_mastering::master(buffer.data(), channels, MASTER_SAMPLE_RATE as f64, options)
            .map_err(|e| e.to_string())?;
    drop(buffer);
    step(runtime, app, 0.85);
    if runtime.cancel.load(Ordering::Relaxed) {
        return Ok("已取消".into());
    }

    let frames = processed.len() / channels;
    let seconds = frames as f64 / MASTER_SAMPLE_RATE as f64;
    let path = ai_output_path(app, "master")?;
    {
        let mut out = ai_daw_engine::audio::AudioBuffer::silence(channels, frames);
        out.data_mut().copy_from_slice(&processed);
        ai_daw_engine::wav::write_wav(&path, &out, MASTER_SAMPLE_RATE).map_err(|e| e.to_string())?;
    }
    // 先把要用的数字与长度算出来，再交出 `processed` 的所有权。
    let length = crate::commands::natural_length_beats(&project, 0.0, seconds);
    state.assets.insert(
        &path,
        Arc::new(DecodedAudio {
            samples: processed.into(),
            channels,
            sample_rate: MASTER_SAMPLE_RATE as f64,
        }),
    );

    let _ = commands_run(
        app,
        Command::AddAudioTrack {
            name: "母带".into(),
            start: 0.0,
            length,
            source: AudioSource {
                path,
                offset_frames: 0,
                frames: Some(frames as u64),
                rate: 1.0,
                pitch_semitones: 0.0,
            },
            at: None,
        },
    );

    Ok(format!(
        "母带完成：{:.1} → {:.1} LUFS，真峰值 {:.1} dBTP{}",
        report.input_lufs,
        report.output_lufs,
        report.output_true_peak_dbtp,
        if report.reduced_db > 0.05 {
            format!("，限幅 {:.1} dB", report.reduced_db)
        } else {
            String::new()
        }
    ))
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

