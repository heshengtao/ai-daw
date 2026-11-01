//! **音色替换（SVC，宿主侧）**：把一条人声轨的音色换成参考声线，旋律 / 歌词 / 节奏不变。
//!
//! 与其它 AI 任务同一套做法：
//! - 重活在**后台线程**里跑，界面显示进度条、随时可取消；
//! - 结果走**同一条命令管线**（`AddAudioTrack`，一次事务 = 一条撤销记录）；
//! - 原轨不动，替换结果落在**它下面一行**，方便 A/B。
//!
//! 与歌声合成（[`crate::singing`]）的区别是输入：那边是 MIDI + 歌词「凭空唱」，这边是
//! **已有的一段人声**——F0 / 节奏 / 咬字都从原音频里提，只把音色换成参考的那条。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{Emitter, Manager};

use ai_daw_engine::audio::{AudioBuffer, DecodedAudio};
use ai_daw_engine::command::Command;
use ai_daw_engine::model::{AudioSource, TrackId};
use ai_daw_local_ai::svc;

use crate::state::AppState;

/// 进度广播事件名。
pub const EVENT_SVC: &str = "engine://svc";

/// 任务状态（前端据此画进度条）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SvcStatus {
    /// 是否有任务在跑。
    pub running: bool,
    /// 进度（0.0..=1.0）。
    pub progress: f32,
    /// 结束后的结论（成功摘要或错误原因）。
    pub message: Option<String>,
}

/// 共享的任务状态。
#[derive(Clone)]
pub struct SvcRuntime {
    running: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU32>,
    message: Arc<Mutex<Option<String>>>,
}

impl Default for SvcRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl SvcRuntime {
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
    pub fn status(&self) -> SvcStatus {
        SvcStatus {
            running: self.running.load(Ordering::Relaxed),
            progress: self.progress.load(Ordering::Relaxed) as f32 / 1000.0,
            message: self.message.lock().unwrap().clone(),
        }
    }

    /// 请求取消（下一次进度回调时停下，什么都不落地）。与 `svc_cancel` 命令同一语义。
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
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
        let _ = app.emit(EVENT_SVC, self.status());
    }
}

/// 模型是否就位（界面据此决定「替换」按钮是否可用）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SvcModels {
    /// 是否有任一档已就位。
    pub installed: bool,
    /// 默认档的目录（没装时是 `None`）。
    pub dir: Option<String>,
    /// 需要下载的字节数（默认档的体积，界面提示用）。
    pub approx_bytes: u64,
}

/// 查询模型状态。
#[tauri::command]
pub fn svc_models() -> SvcModels {
    use ai_daw_local_ai::models::catalog;
    let default = svc::soulx::default_precision();
    SvcModels {
        installed: default.is_some(),
        dir: svc::model_dir().map(|p| p.to_string_lossy().into_owned()),
        approx_bytes: catalog::by_id(svc::soulx::Precision::Fp16Hq.id())
            .map(|e| e.total_bytes())
            .unwrap_or(0),
    }
}

/// 查询任务状态。
#[tauri::command]
pub fn svc_status(state: tauri::State<'_, AppState>) -> SvcStatus {
    state.svc.status()
}

/// 取消当前任务（下一次进度回调时停下，什么都不落地）。
#[tauri::command]
pub fn svc_cancel(state: tauri::State<'_, AppState>) {
    state.svc.cancel.store(true, Ordering::Relaxed);
}

/// 替换参数（界面与 MCP 共用）。
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct SwapArgs {
    /// 目标音频轨（要换音色的那条人声）。
    pub track: u64,
    /// 指定片段 id；省略 = 这条轨上第一个音频片段。
    pub clip: Option<u64>,
    /// 参考音色：直接给一个音频文件路径。
    pub reference: Option<String>,
    /// 参考音色：用这条轨道的第一个音频片段（与 `reference` 二选一）。
    pub reference_track: Option<u64>,
    /// 参考音色：人声音色库条目 id（优先级最高）。
    pub reference_voice: Option<String>,
    /// 模型：`soulx-svc-fp16hq` / `soulx-svc-fp32`；省略 = 已安装的默认档。
    pub model: Option<String>,
    /// 采样步数（32 默认；16 更快、质量略降）。
    pub steps: Option<u32>,
    /// 噪声种子（同一份输入 + 同一个种子 = 同一条输出）。
    pub seed: Option<u64>,
    /// 整体移调（半音，-24..=24）。
    pub pitch_shift: Option<i32>,
    /// 新轨道的名字；省略 = `<源轨名> · 换声`。
    pub name: Option<String>,
}

impl SwapArgs {
    /// 采样步数（夹到 8..=64）。
    pub fn steps(&self) -> u32 {
        self.steps.unwrap_or(svc::soulx::DEFAULT_STEPS).clamp(8, 64)
    }

    /// 种子（缺省 0：可复现）。
    pub fn seed(&self) -> u64 {
        self.seed.unwrap_or(0)
    }

    /// 移调（缺省 0；夹到 −24..=24）。
    pub fn pitch_shift(&self) -> i32 {
        self.pitch_shift.unwrap_or(0).clamp(-24, 24)
    }

    /// 选中的模型 id（没指定时返回 `None`，由后端取默认已安装档）。
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref().filter(|m| !m.trim().is_empty())
    }
}

/// 开始一次替换（后台线程）。
#[tauri::command]
pub fn svc_render(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    args: SwapArgs,
) -> Result<(), String> {
    if state.svc.running.swap(true, Ordering::SeqCst) {
        return Err("已有一条音色替换在跑".into());
    }
    if !svc::installed() {
        state.svc.running.store(false, Ordering::SeqCst);
        return Err("未安装音色替换模型（管理器 → 模型 → SoulX-Singer-SVC，约 2.4 GB）".into());
    }

    let runtime = state.svc.clone();
    runtime.cancel.store(false, Ordering::Relaxed);
    runtime.set_progress(0.0);
    *runtime.message.lock().unwrap() = None;
    let _ = app.emit(EVENT_SVC, runtime.status());

    let thread_app = app.clone();
    let thread_runtime = runtime.clone();
    let spawn = std::thread::Builder::new()
        .name("ai-daw-svc".into())
        .spawn(move || {
            // 线程里 panic 也要收尾：否则 `running` 永远为真、进度条卡住（见 crate::job）。
            let outcome = crate::job::guard(|| render(&thread_app, &thread_runtime, &args));
            thread_runtime.finish(&thread_app, outcome);
        });
    if let Err(e) = spawn {
        runtime.running.store(false, Ordering::SeqCst);
        return Err(e.to_string());
    }
    Ok(())
}

/// 同步替换（MCP 工具用：整体阻塞在调用线程，返回一句摘要）。
pub(crate) fn render_blocking(app: &tauri::AppHandle, args: &SwapArgs) -> Result<String, String> {
    let state = app.state::<AppState>();
    let runtime = state.svc.clone();
    if runtime.running.swap(true, Ordering::SeqCst) {
        return Err("已有一条音色替换在跑".into());
    }
    runtime.cancel.store(false, Ordering::Relaxed);
    *runtime.message.lock().unwrap() = None;
    let outcome = render(app, &runtime, args);
    runtime.running.store(false, Ordering::SeqCst);
    outcome
}

/// 替换的共用实现（进度 / 取消 / 落地都在这里）。
fn render(app: &tauri::AppHandle, runtime: &SvcRuntime, args: &SwapArgs) -> Result<String, String> {
    let state = app.state::<AppState>();

    // ---- 1. 目标片段（锁只在这里短暂持有）----
    let target = {
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
            .filter(|c| c.audio.is_some())
            .find(|c| args.clip.is_none_or(|id| c.id.0 == id))
            .ok_or("这条轨道上没有音频片段（先录音或导入音频）")?;
        Target {
            track_index: index,
            track_name: track.name.clone(),
            clip_start: clip.start,
            clip_length: clip.length,
            source: clip.audio.clone().expect("已确认是音频片段"),
        }
    };

    // ---- 2. 参考音色（必需）----
    let reference = crate::voices::load_reference(
        &state,
        args.reference_voice.as_deref(),
        args.reference_track,
        args.reference.as_deref(),
    )?
    .ok_or("需要一条参考声线：选一条参考轨、音色库条目，或给一个音频文件（换音色就是换这一段）")?;

    // ---- 3. 目标音频 → 单声道 24 kHz（只取片段窗口，整首歌的工程不必整首推理）----
    let audio = state.assets.load(&target.source.path)?;
    let file_frames = audio.frames();
    if file_frames == 0 {
        return Err("目标音频是空的".into());
    }
    let channels = audio.channels.max(1);
    let start = (target.source.offset_frames as usize).min(file_frames);
    let end = match target.source.frames {
        Some(frames) => (start + frames as usize).min(file_frames),
        None => file_frames,
    };
    if end <= start {
        return Err("目标片段的时间窗口是空的".into());
    }
    let mono = to_mono(&audio.samples[start * channels..end * channels], channels);
    let resampled = ai_daw_local_ai::resample::resample(
        &mono,
        audio.sample_rate,
        ai_daw_local_ai::svc::SAMPLE_RATE,
    );

    // ---- 4. 模型先解析：没装的那一档立刻报错，别等加载完 2.4 GB 才说 ----
    let model = args.model();
    svc::soulx::resolve(model)?;
    let runtime_cb = runtime.clone();
    let app_cb = app.clone();
    let progress = move |p: f32| -> bool {
        runtime_cb.set_progress(p);
        let _ = app_cb.emit(EVENT_SVC, runtime_cb.status());
        !runtime_cb.cancelled()
    };
    let started = std::time::Instant::now();
    let take = match svc::convert(
        &resampled,
        Some((&reference.0, reference.1)),
        args.steps(),
        args.seed(),
        args.pitch_shift(),
        model,
        &progress,
    ) {
        Ok(take) => take,
        Err(err) if err == "已取消" || runtime.cancelled() => return Ok("已取消".into()),
        Err(err) => return Err(err),
    };

    // ---- 5. 落盘 + 建轨 ----
    let path = output_path(app, "voice-swap")?;
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

    let name = args
        .name
        .clone()
        .unwrap_or_else(|| format!("{} · 换声", target.track_name));

    // 输出固定是 24 kHz，且只包含片段窗口：新文件的起点就是窗口起点。
    // 源片段有 `frames` 时新文件要等长覆盖；只有 `offset` 的（到文件尾）保持 `None`。
    let total_out = take.samples.len() as u64;
    let out_offset = 0u64;
    let out_frames = target.source.frames.map(|_| total_out);

    crate::commands::run_agent(
        app,
        Command::AddAudioTrack {
            name: name.clone(),
            start: target.clip_start,
            length: target.clip_length,
            source: AudioSource {
                path: path.clone(),
                offset_frames: out_offset,
                frames: out_frames,
                // 与源片段保持同一套「时间拉伸 / 变速变调」参数：新音色在时间轴上的
                // 位置与听感才和原片段一致（窗口本身已经按源窗口截好了）。
                rate: target.source.rate,
                pitch_semitones: target.source.pitch_semitones,
            },
            at: Some(target.track_index + 1),
        },
    );

    let seconds = started.elapsed().as_secs_f32();
    let reference = take
        .reference_offset
        .filter(|s| *s >= 1.0)
        .map(|s| format!("，参考取自 {s:.0} 秒"))
        .unwrap_or_default();
    let shift = if take.pitch_shift == 0 {
        String::new()
    } else {
        format!("，移调 {:+}", take.pitch_shift)
    };
    // 图到底建在显卡上还是 CPU 上——「GPU 利用率没涨」时先看这一行。
    let (gpu, cpu) = ai_daw_local_ai::backend::session::take_session_stats();
    if gpu + cpu > 0 {
        use std::io::Write as _;
        let _ = writeln!(
            std::io::stderr(),
            "[backend] 音色替换：GPU {gpu} 张图 / CPU {cpu} 张"
        );
    }
    let ep = if gpu + cpu == 0 {
        String::new()
    } else if let (0, Some(reason)) = (gpu, ai_daw_local_ai::backend::session::last_fallback()) {
        // 全在 CPU 上：把第一张图失败的原因直接写进摘要，不用去翻日志。
        format!("，GPU {gpu} / CPU {cpu} 张图（{reason}）")
    } else {
        format!("，GPU {gpu} / CPU {cpu} 张图")
    };
    Ok(format!(
        "已生成「{name}」：{:.1} 秒、{} 段、{}（{shift}{reference}，用时 {seconds:.0} 秒{ep}）",
        take.seconds,
        take.segments,
        take.precision.label(),
    ))
}

/// 目标片段快照（锁外使用）。
struct Target {
    track_index: usize,
    track_name: String,
    clip_start: f64,
    clip_length: f64,
    source: AudioSource,
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

/// 模型目录（设置页「打开模型目录」用）。
#[allow(dead_code)]
pub fn model_dir() -> Option<PathBuf> {
    svc::model_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_seed_and_shift_are_clamped() {
        let slow = SwapArgs {
            steps: Some(1),
            ..Default::default()
        };
        assert_eq!(slow.steps(), 8);
        let fast = SwapArgs {
            steps: Some(999),
            ..Default::default()
        };
        assert_eq!(fast.steps(), 64);
        assert_eq!(SwapArgs::default().seed(), 0);
        let high = SwapArgs {
            pitch_shift: Some(99),
            ..Default::default()
        };
        assert_eq!(high.pitch_shift(), 24);
        let low = SwapArgs {
            pitch_shift: Some(-99),
            ..Default::default()
        };
        assert_eq!(low.pitch_shift(), -24);
    }

    #[test]
    fn model_id_is_empty_string_aware() {
        let args = SwapArgs {
            model: Some("  ".into()),
            ..Default::default()
        };
        assert_eq!(args.model(), None);
        let args = SwapArgs {
            model: Some(" soulx-svc-fp32 ".into()),
            ..Default::default()
        };
        assert_eq!(args.model(), Some(" soulx-svc-fp32 "));
    }

    #[test]
    fn mono_downmix_averages_channels() {
        let stereo = vec![1.0, 0.0, 0.5, -0.5];
        assert_eq!(to_mono(&stereo, 2), vec![0.5, 0.0]);
        assert_eq!(to_mono(&stereo, 1), stereo);
    }
}
