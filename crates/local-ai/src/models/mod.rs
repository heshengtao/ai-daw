//! **ONNX 模型层**（`ort`）：把真正的权重接进来，替掉内置 DSP 的近似实现。
//!
//! 四个模型：
//! - [`basic_pitch`] —— Spotify Basic Pitch（ICASSP 2022）多音音高转录，音频 → MIDI 音符。
//! - [`localvqe`] —— LocalVQE v1.2（DeepVQE 的流式 CPU 派生版）联合降噪/去混响。
//! - [`beat_this`] —— Beat This!（ISMIR 2024）节拍/重拍检测，推出全局 BPM 与拍号。
//! - [`demucs`] —— HTDemucs（Demucs v4）音源分离 4 分轨（复数谱 "cac" 输入）。
//! - [`demucs6s`] —— HTDemucs 6s 音源分离 6 分轨（自足 ONNX：波形进、分轨出）。
//!
//! **小模型随安装包分发**（Basic Pitch 230 KB / LocalVQE 6 MB，放进 `src-tauri/resources/models/`
//! 并由 `bundle.resources` 打进包内）；**大模型（如音源分离 209 MB）不随包**，由应用内下载器
//! 下到应用配置目录的 `models/`（见 `src-tauri/src/models.rs`）。开发时也可用
//! `tools/fetch-models.sh` 灌进仓库根 `models/`。缺失时宿主退回内置 DSP（音源分离没有兜底）。
//!
//! 模型目录按顺序查找：
//! 1. 环境变量 `AIDAW_MODELS`（用户/测试覆盖）
//! 2. 宿主注册的额外目录（[`set_extra_dirs`]）：应用配置目录的 `models/`（下载落点）
//!    与打包后的资源目录 `models/`（随包小模型）
//! 3. 可执行文件同级的 `models/`
//! 4. macOS 应用包里的 `../Resources/models/`
//! 5. 开发时仓库根目录的 `models/`（`crate` 的上两级）
//! 6. 当前工作目录的 `models/`
//!
//! **多文件模型**（「一张图 + 一个 `.onnx.data` 外部权重」这类）不散落在 `models/` 下，而是
//! 整组待在子目录里——`ort` 靠 `.onnx` 里记录的相对路径去找外部权重，图和权重分处两个搜索
//! 目录就会加载失败。所以这类模型要求**同一个搜索目录**里齐备全部文件才算可用（见
//! [`catalog::group_dir`]）。

pub mod basic_pitch;
pub mod beat_this;
pub mod catalog;
pub mod demucs;
pub mod demucs6s;
pub mod localvqe;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Basic Pitch 权重文件名。
pub const BASIC_PITCH_FILE: &str = "basic-pitch-nmp.onnx";
/// LocalVQE 降噪权重文件名。
pub const LOCALVQE_FILE: &str = "localvqe-ns.onnx";
/// HTDemucs（4 分轨）音源分离权重文件名（ORT 格式）。
pub const HTDEMUCS_FILE: &str = "htdemucs.ort";
/// HTDemucs 6s（6 分轨）音源分离权重文件名（自足 ONNX 导出）。
pub const HTDEMUCS_6S_FILE: &str = "htdemucs_6s.onnx";
/// Beat This! 的 log-mel 前端权重文件名（264 KB，**随包**，同 Basic Pitch）。
pub const BEAT_THIS_MEL_FILE: &str = "mel_spectrogram.onnx";
/// Beat This! 的主模型（`final0`，FP32，约 83 MB）——**不随包**，用户按需下载。
pub const BEAT_THIS_FILE: &str = "beat_this.onnx";

/// 一次音源分离的结果：按模型输出顺序的 `(stem 名, 交错样本)`。
///
/// `stem 名` 是稳定标识（`drums` / `bass` / `other` / `vocals` / `guitar` / `piano`），
/// 宿主据此映射成各语言的显示名。样本是**交错**的，采样率与声道数与输入一致。
#[derive(Debug, Clone, Default)]
pub struct Stems {
    /// 各分轨。
    pub stems: Vec<(&'static str, Vec<f32>)>,
}

/// 把模型采样率下的**交错 L/R** 分轨重采样回源采样率，并按 `channels` 拼装。
///
/// 4 分轨与 6 分轨两个模块共用（它们都产出「交错立体声 @44.1 kHz」）。
pub(crate) fn finish_stem(
    interleaved: &[f32],
    channels: usize,
    sample_rate: f64,
    target_frames: usize,
) -> Vec<f32> {
    let n = interleaved.len() / 2;
    let mut l = Vec::with_capacity(n);
    let mut r = Vec::with_capacity(n);
    for i in 0..n {
        l.push(interleaved[2 * i]);
        r.push(interleaved[2 * i + 1]);
    }
    let l = crate::resample::resample(&l, 44_100.0, sample_rate);
    let r = crate::resample::resample(&r, 44_100.0, sample_rate);
    let frames = l.len().min(r.len()).min(target_frames);
    let mut out = vec![0.0f32; target_frames * channels];
    if channels == 1 {
        for i in 0..frames {
            out[i] = 0.5 * (l[i] + r[i]);
        }
    } else {
        for i in 0..frames {
            out[i * channels] = l[i];
            if channels >= 2 {
                out[i * channels + 1] = r[i];
            }
        }
    }
    out
}

/// 宿主注册的额外搜索目录（下载落点 / 打包资源目录）。
static EXTRA_DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();

/// 注册额外搜索目录（首次调用生效；宿主在启动时调用一次）。
///
/// 本 crate 不依赖 Tauri，因此由宿主把「应用配置目录的 `models/`」与「打包资源目录的
/// `models/`」交给它——两处分别是**下载落点**与**随包小模型**。
pub fn set_extra_dirs(dirs: Vec<PathBuf>) {
    let _ = EXTRA_DIRS.set(dirs);
}

/// 按优先级返回候选目录。
pub fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("AIDAW_MODELS") {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(extra) = EXTRA_DIRS.get() {
        dirs.extend(extra.iter().cloned());
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join("models"));
            dirs.push(parent.join("../Resources/models"));
        }
    }
    // 开发路径：`crates/local-ai` 的上两级 = 仓库根。
    dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models"));
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join("models"));
    }
    dirs
}

/// 在候选目录里找一个模型文件。
pub fn find(file: &str) -> Option<PathBuf> {
    search_dirs()
        .into_iter()
        .map(|d| d.join(file))
        .find(|p| p.is_file())
}

/// 在候选目录的**子目录**里找一个模型文件（多文件模型组用，见 [`catalog::group_dir`]）。
pub fn find_in(dir: &str, file: &str) -> Option<PathBuf> {
    search_dirs()
        .into_iter()
        .map(|d| d.join(dir).join(file))
        .find(|p| p.is_file())
}

/// 各模型各自的可用情况（给界面/日志用）。
#[derive(Debug, Clone, Default)]
pub struct Availability {
    /// Basic Pitch 权重路径。
    pub basic_pitch: Option<PathBuf>,
    /// LocalVQE 权重路径。
    pub localvqe: Option<PathBuf>,
    /// HTDemucs 音源分离权重路径。
    pub htdemucs: Option<PathBuf>,
    /// Beat This! 主模型（`final0`）路径 —— **仅当 log-mel 前端也在位时**才算可用。
    pub beat_this: Option<PathBuf>,
    /// 歌声合成（SoulX-Singer 9 图整组）的目录 —— 整组齐备才算可用。
    pub singing: Option<PathBuf>,
}

impl Availability {
    /// 至少有一个模型可用。
    pub fn any(&self) -> bool {
        self.basic_pitch.is_some()
            || self.localvqe.is_some()
            || self.htdemucs.is_some()
            || self.beat_this.is_some()
            || self.singing.is_some()
    }
}

/// 探测各模型是否存在。
pub fn availability() -> Availability {
    // Beat This! 要**两个**权重才能跑：mel 前端（随包）与主模型（按需下载）。
    let beat_this = match (find(BEAT_THIS_MEL_FILE), find(BEAT_THIS_FILE)) {
        (Some(_), Some(beat)) => Some(beat),
        _ => None,
    };
    Availability {
        basic_pitch: find(BASIC_PITCH_FILE),
        localvqe: find(LOCALVQE_FILE),
        htdemucs: find(HTDEMUCS_FILE),
        beat_this,
        singing: crate::singing::model_dir(),
    }
}

/// `ort` 的错误统一转成 `String`（宿主只透传）。
pub(crate) fn err<E: std::fmt::Display>(what: &str, e: E) -> String {
    format!("{what}：{e}")
}
