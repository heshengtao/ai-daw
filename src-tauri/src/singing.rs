//! **歌声合成（宿主侧）**：把「MIDI 片段 + 一行歌词 + 参考音色」渲染成一条音频轨。
//!
//! 与其它 AI 任务（转 MIDI / 降噪 / 音源分离）同一套做法：
//! - 重活在**后台线程**里跑，界面显示进度条、随时可取消；
//! - 结果走**同一条命令管线**（`AddAudioTrack`，一次事务 = 一条撤销记录）；
//! - 原 MIDI 轨不动，歌声落在**它下面一行**，方便 A/B。
//!
//! 「换音色」不需要另一条链路：同一段 MIDI + 歌词、换一条参考声线重渲染即可
//! （SoulX-Singer 的音色本来就由参考音频决定，见设计文档 §7）。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{Emitter, Manager};

use ai_daw_engine::audio::{AudioBuffer, DecodedAudio};
use ai_daw_engine::command::Command;
use ai_daw_engine::model::{AudioSource, Note, TrackId};
use ai_daw_engine::tempo::TempoMap;
use ai_daw_local_ai::singing::{self, SingNote};

use crate::state::AppState;

/// 进度广播事件名。
pub const EVENT_SINGING: &str = "engine://singing";

/// 任务状态（前端据此画进度条）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SingingStatus {
    /// 是否有任务在跑。
    pub running: bool,
    /// 进度（0.0..=1.0）。
    pub progress: f32,
    /// 结束后的结论（成功摘要或错误原因）。
    pub message: Option<String>,
}

/// 共享的任务状态。
#[derive(Clone)]
pub struct SingingRuntime {
    running: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU32>,
    message: Arc<Mutex<Option<String>>>,
}

impl Default for SingingRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl SingingRuntime {
    /// 空状态。
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(AtomicU32::new(0)),
            message: Arc::new(Mutex::new(None)),
        }
    }

    /// 当前快照。
    pub fn status(&self) -> SingingStatus {
        SingingStatus {
            running: self.running.load(Ordering::Relaxed),
            progress: self.progress.load(Ordering::Relaxed) as f32 / 1000.0,
            message: self.message.lock().unwrap().clone(),
        }
    }

    fn set_progress(&self, p: f32) {
        self.progress
            .store((p.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn finish(&self, app: &tauri::AppHandle, outcome: Result<String, String>) {
        *self.message.lock().unwrap() = Some(match outcome {
            Ok(text) => text,
            Err(err) => format!("失败：{err}"),
        });
        self.running.store(false, Ordering::SeqCst);
        self.progress.store(0, Ordering::Relaxed);
        let _ = app.emit(EVENT_SINGING, self.status());
    }
}

/// 模型是否就位（界面据此决定「生成」按钮是否可用）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SingingModels {
    /// 9 张图整组是否齐备。
    pub installed: bool,
    /// 模型目录（没装时是 `None`）。
    pub dir: Option<String>,
    /// 需要下载的字节数（界面提示用）。
    pub approx_bytes: u64,
}

/// 查询模型状态。
#[tauri::command]
pub fn singing_models() -> SingingModels {
    let entry = ai_daw_local_ai::models::catalog::by_id(singing::soulx::DIR);
    SingingModels {
        installed: singing::installed(),
        dir: singing::model_dir().map(|p| p.to_string_lossy().into_owned()),
        approx_bytes: entry.map(|e| e.total_bytes()).unwrap_or(0),
    }
}

/// 查询任务状态。
#[tauri::command]
pub fn singing_status(state: tauri::State<'_, AppState>) -> SingingStatus {
    state.singing.status()
}

/// 取消当前任务（下一次进度回调时停下，什么都不落地）。
#[tauri::command]
pub fn singing_cancel(state: tauri::State<'_, AppState>) {
    state.singing.cancel.store(true, Ordering::Relaxed);
}

/// 渲染参数（界面与 MCP 共用）。
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct SingArgs {
    /// MIDI 轨道 id（用它的第一个 MIDI 片段；`clip` 可指定具体片段）。
    pub track: u64,
    /// 指定片段 id；省略 = 这条轨上第一个有音符的 MIDI 片段。
    pub clip: Option<u64>,
    /// 歌词（一字一音，`-` 连音，见 `singing::assign_lyrics`）。
    pub lyrics: String,
    /// 参考音色：直接给一个音频文件路径。
    pub reference: Option<String>,
    /// 参考音色：用这条轨道的第一个音频片段（与 `reference` 二选一）。
    pub reference_track: Option<u64>,
    /// 参考音色：人声音色库条目 id（优先级最高）。
    pub reference_voice: Option<String>,
    /// 采样步数（32 默认；16 更快、质量略降）。
    pub steps: Option<u32>,
    /// 噪声种子（同一份输入 + 同一个种子 = 同一条输出）。
    pub seed: Option<u64>,
    /// 新轨道的名字；省略 = `<源轨名> · 歌声`。
    pub name: Option<String>,
}

impl SingArgs {
    /// 采样步数（夹到 8..=64）。
    pub fn steps(&self) -> u32 {
        self.steps
            .unwrap_or(singing::soulx::DEFAULT_STEPS)
            .clamp(8, 64)
    }

    /// 种子（缺省 0：可复现）。
    pub fn seed(&self) -> u64 {
        self.seed.unwrap_or(0)
    }
}

/// 开始一次渲染（后台线程）。
#[tauri::command]
pub fn singing_render(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    args: SingArgs,
) -> Result<(), String> {
    if state.singing.running.swap(true, Ordering::SeqCst) {
        return Err("已有一条歌声在生成".into());
    }
    if !singing::installed() {
        state.singing.running.store(false, Ordering::SeqCst);
        return Err("未安装歌声合成模型（管理器 → 模型 → SoulX-Singer，约 1.4 GB）".into());
    }

    let runtime = state.singing.clone();
    runtime.cancel.store(false, Ordering::Relaxed);
    runtime.set_progress(0.0);
    *runtime.message.lock().unwrap() = None;
    let _ = app.emit(EVENT_SINGING, runtime.status());

    let thread_app = app.clone();
    let thread_runtime = runtime.clone();
    let spawn = std::thread::Builder::new()
        .name("ai-daw-singing".into())
        .spawn(move || {
            let outcome = render(&thread_app, &thread_runtime, &args);
            thread_runtime.finish(&thread_app, outcome);
        });
    if let Err(e) = spawn {
        runtime.running.store(false, Ordering::SeqCst);
        return Err(e.to_string());
    }
    Ok(())
}

/// 同步渲染（MCP 工具用：整体阻塞在调用线程，返回一句摘要）。
pub(crate) fn render_blocking(app: &tauri::AppHandle, args: &SingArgs) -> Result<String, String> {
    let state = app.state::<AppState>();
    let runtime = state.singing.clone();
    if runtime.running.swap(true, Ordering::SeqCst) {
        return Err("已有一条歌声在生成".into());
    }
    runtime.cancel.store(false, Ordering::Relaxed);
    *runtime.message.lock().unwrap() = None;
    let outcome = render(app, &runtime, args);
    runtime.running.store(false, Ordering::SeqCst);
    outcome
}

/// 渲染的共用实现（进度 / 取消 / 落地都在这里）。
fn render(
    app: &tauri::AppHandle,
    runtime: &SingingRuntime,
    args: &SingArgs,
) -> Result<String, String> {
    let state = app.state::<AppState>();

    // ---- 1. 取谱面（锁只在这里短暂持有）----
    let snapshot = {
        let engine = state.engine.lock().unwrap();
        let project = engine.project();
        let index = project
            .tracks
            .iter()
            .position(|t| t.id == TrackId(args.track))
            .ok_or("轨道不存在")?;
        let track = &project.tracks[index];
        let clip = track
            .clips
            .iter()
            .filter(|c| c.audio.is_none() && !c.notes.is_empty())
            .find(|c| args.clip.is_none_or(|id| c.id.0 == id))
            .ok_or("这条轨道上没有带音符的 MIDI 片段（先写旋律，或换一条轨道）")?;
        let bpm = TempoMap::from_transport(&project.transport).tempo_at(clip.start);
        Score {
            track_index: index,
            track_name: track.name.clone(),
            clip_id: clip.id.0,
            clip_start: clip.start,
            clip_length: clip.length,
            notes: clip.notes.clone(),
            bpm,
        }
    };

    if !(snapshot.bpm.is_finite() && snapshot.bpm > 0.0) {
        return Err("这段片段所在位置的速度不是有效值".into());
    }

    // ---- 2. 参考音色 ----
    let reference = load_reference(&state, args)?;

    // ---- 3. 音符 + 歌词 ----
    let first_beat = snapshot
        .notes
        .iter()
        .map(|n| n.start)
        .fold(f64::INFINITY, f64::min);
    if !first_beat.is_finite() {
        return Err("片段里没有音符".into());
    }
    let mut notes: Vec<SingNote> = snapshot
        .notes
        .iter()
        .map(|n| SingNote {
            start: n.start - first_beat,
            duration: n.duration,
            pitch: n.pitch,
            // 逐音歌词（`Note.lyric`）优先；没有的音符由下面的整串歌词补。
            syllable: n.lyric.clone(),
            tone: None,
            slur: false,
        })
        .collect();
    let report = singing::assign_lyrics(&mut notes, &args.lyrics);
    if report.assigned == 0 {
        return Err(
            "没有能唱的音节：在钢琴卷帘里给音符填歌词，或在「歌词」框里写中文（标点与空格会被跳过）"
                .into(),
        );
    }

    // ---- 4. 合成 ----
    let runtime_cb = runtime.clone();
    let app_cb = app.clone();
    let progress = move |p: f32| -> bool {
        runtime_cb.set_progress(p);
        let _ = app_cb.emit(EVENT_SINGING, runtime_cb.status());
        !runtime_cb.cancelled()
    };
    let started = std::time::Instant::now();
    let take = match reference {
        Some((samples, rate)) => singing::synthesize(
            &notes,
            snapshot.bpm,
            args.steps(),
            args.seed(),
            Some((&samples, rate)),
            &progress,
        ),
        None => singing::synthesize(
            &notes,
            snapshot.bpm,
            args.steps(),
            args.seed(),
            None,
            &progress,
        ),
    };
    let take = match take {
        Ok(take) => take,
        Err(err) if err == "已取消" || runtime.cancelled() => return Ok("已取消".into()),
        Err(err) => return Err(err),
    };

    // ---- 5. 落盘 + 建轨 ----
    let path = output_path(app, "sing")?;
    {
        let mut buffer = AudioBuffer::silence(1, take.samples.len());
        buffer.data_mut().copy_from_slice(&take.samples);
        ai_daw_engine::wav::write_wav(&path, &buffer, take.sample_rate as u32)
            .map_err(|e| format!("写 WAV 失败：{e}"))?;
    }
    state.assets.insert(
        &path,
        Arc::new(DecodedAudio {
            samples: take.samples.clone().into(),
            channels: 1,
            sample_rate: take.sample_rate,
        }),
    );

    let start = snapshot.clip_start + first_beat;
    let length = {
        let engine = state.engine.lock().unwrap();
        crate::commands::natural_length_beats(engine.project(), start, take.seconds)
    };
    let name = args
        .name
        .clone()
        .unwrap_or_else(|| format!("{} · 歌声", snapshot.track_name));
    crate::commands::run_agent(
        app,
        Command::AddAudioTrack {
            name: name.clone(),
            start,
            length,
            source: AudioSource {
                path: path.clone(),
                offset_frames: 0,
                frames: Some(take.samples.len() as u64),
                rate: 1.0,
                pitch_semitones: 0.0,
            },
            at: Some(snapshot.track_index + 1),
        },
    );

    let seconds = started.elapsed().as_secs_f32();
    let reference = take
        .reference_offset
        .filter(|s| *s >= 1.0)
        .map(|s| format!("，参考取自 {:.0} 秒", s))
        .unwrap_or_default();
    Ok(format!(
        "已生成「{name}」：{} 秒歌声、{} 个音、{} 段（{}{reference}，用时 {seconds:.0} 秒）",
        take.seconds.round(),
        take.notes,
        take.segments,
        report.summary(),
    ))
}

/// 谱面快照（锁外使用）。
struct Score {
    track_index: usize,
    track_name: String,
    #[allow(dead_code)]
    clip_id: u64,
    clip_start: f64,
    #[allow(dead_code)]
    clip_length: f64,
    notes: Vec<Note>,
    bpm: f64,
}

/// 参考音色：优先音色库（`reference_voice`），其次工程内轨道，最后文件路径。
fn load_reference(state: &AppState, args: &SingArgs) -> Result<Option<(Vec<f32>, f64)>, String> {
    if let Some(id) = args.reference_voice.as_deref().filter(|id| !id.is_empty()) {
        let (samples, rate) = crate::voices::samples_for(state, id)?;
        return Ok(Some((samples, rate)));
    }
    let source = if let Some(track) = args.reference_track {
        let engine = state.engine.lock().unwrap();
        let project = engine.project();
        let t = project
            .tracks
            .iter()
            .find(|t| t.id == TrackId(track))
            .ok_or("参考音色轨道不存在")?;
        let clip = t
            .clips
            .iter()
            .find(|c| c.audio.is_some())
            .ok_or("参考音色轨道上没有音频片段")?;
        Some(clip.audio.clone().expect("已确认是音频片段").path)
    } else {
        args.reference.clone()
    };
    let Some(path) = source else {
        return Ok(None);
    };
    let audio = state.assets.load(&path)?;
    if audio.frames() == 0 {
        return Err("参考音色是空音频".into());
    }
    let mono = to_mono(&audio.samples, audio.channels);
    Ok(Some((mono, audio.sample_rate)))
}

/// 交错多声道 → 单声道（平均）。
fn to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    let channels = channels.max(1);
    if channels == 1 {
        return samples.to_vec();
    }
    samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// 输出 WAV 路径（应用数据目录 `local-ai/`，与其它 AI 产物放一起）。
fn output_path(app: &tauri::AppHandle, kind: &str) -> Result<String, String> {
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
    Ok(dir
        .join(format!("{kind}-{stamp}.wav"))
        .to_string_lossy()
        .into_owned())
}

/// 模型是否就位（MCP 工具在开跑前先问一句，免得白等）。
pub fn model_ready() -> bool {
    singing::installed()
}

/// 模型目录（设置页「打开模型目录」用）。
#[allow(dead_code)]
pub fn model_dir() -> Option<PathBuf> {
    singing::model_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_downmix_averages_channels() {
        let stereo = vec![1.0, 0.0, 0.5, -0.5];
        assert_eq!(to_mono(&stereo, 2), vec![0.5, 0.0]);
        assert_eq!(to_mono(&stereo, 1), stereo);
    }

    #[test]
    fn steps_and_seed_are_clamped() {
        let slow = SingArgs {
            steps: Some(1),
            ..Default::default()
        };
        assert_eq!(slow.steps(), 8);
        let fast = SingArgs {
            steps: Some(999),
            ..Default::default()
        };
        assert_eq!(fast.steps(), 64);
        assert_eq!(SingArgs::default().seed(), 0);
    }
}
