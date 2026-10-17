//! **节拍 / 重拍检测**：Beat This!（ISMIR 2024，JKU Linz，MIT）——一次推理直接给出
//! 拍点与重拍的时间戳，据此推出**单一全局 BPM + 拍号**。
//!
//! 与上一版（自实现 log-mel 前端 + 把结果写成**逐小节速度图**）的关键区别：这次**不写速度图**。
//! 本仓库的时间轴是按「拍」**线性绘制**的（`px = 拍 × zoom`），全局恒定速度下网格与波形天然
//! 对齐；一旦引入逐小节变化的速度，网格 / 波形 / 节拍器就再也不可能稳定对上——这正是上一版
//! 撤掉的原因。所以这里只产出「一个 BPM + 一个拍号」，由宿主一次事务写进工程。
//!
//! 两个权重缺一不可：
//! - `mel_spectrogram.onnx`（264 KB 的 log-mel 前端）—— **随安装包分发**（同 Basic Pitch）；
//! - `beat_this.onnx`（`final0`，FP32，约 83 MB）—— **不随包**，用户在「管理器 → 模型」按需下载
//!   （见 `src-tauri/src/models.rs` 与 `catalog.rs`）。
//!
//! 推理走 crate 默认的纯 Rust `rten` 后端，不需要 `libonnxruntime`。重采样到 22.05 kHz、log-mel、
//! 30 秒重叠分块推理、峰值拾取全都由 crate 内部完成，这里只负责「喂单声道 f32 → 读拍点」。

use std::path::PathBuf;

use beat_this::{BeatAnalysis, BeatThis, RtenRuntime};

use crate::Progress;

/// 一次节拍检测的结果：全局 BPM + 拍号 + 对齐用的相位。
#[derive(Debug, Clone, PartialEq)]
pub struct TempoDetection {
    /// 速度（BPM，以四分音符计）。
    pub bpm: f64,
    /// 拍号分子。
    pub time_sig_num: u8,
    /// 拍号分母（Beat This! 不区分分母，恒取 4）。
    pub time_sig_den: u8,
    /// 检测到的拍点数。
    pub beats: usize,
    /// 检测到的重拍数。
    pub downbeats: usize,
    /// 第一个重拍相对素材起点的秒数；没有重拍时为 `None`（宿主据此算对齐相位）。
    pub first_downbeat_sec: Option<f64>,
    /// 参与检测的素材时长（秒）。
    pub duration_sec: f64,
}

/// 两个权重都就位时返回 `(mel 前端, 主模型)` 路径。
pub fn model_paths() -> Option<(PathBuf, PathBuf)> {
    Some((super::find(super::BEAT_THIS_MEL_FILE)?, super::find(super::BEAT_THIS_FILE)?))
}

/// 对素材跑一遍 Beat This!，推出全局 BPM 与拍号。
///
/// `samples` 是**交错**多声道，内部先下混成单声道；采样率由 crate 重采样到 22.05 kHz。
pub fn detect(
    samples: &[f32],
    channels: usize,
    sample_rate: f64,
    progress: Progress<'_>,
) -> Result<TempoDetection, String> {
    let (mel_path, beat_path) = model_paths().ok_or("Beat This! 权重未安装")?;
    if !(sample_rate.is_finite() && sample_rate >= 8_000.0) {
        return Err(format!("采样率不可用：{sample_rate}"));
    }
    let mono = crate::to_mono(samples, channels);
    let duration_sec = mono.len() as f64 / sample_rate;
    // 太短的素材连一个像样的节拍周期都装不下；也给模型留一点上下文。
    if duration_sec < 2.0 {
        return Err("素材太短（至少 2 秒）".into());
    }
    if !progress(0.05) {
        return Err("已取消".into());
    }

    let mut tracker = BeatThis::new(&RtenRuntime, &mel_path, &beat_path)
        .map_err(|e| format!("加载 Beat This! 权重失败：{e}"))?;
    // 推理是一整段不可打断的调用，只能在两端检查取消。
    let analysis = tracker
        .analyze_audio(&mono, sample_rate.round() as u32)
        .map_err(|e| format!("Beat This! 推理失败：{e}"))?;
    if !progress(0.9) {
        return Err("已取消".into());
    }

    let bpm = beat_this::calculate_bpm(&analysis)
        .ok_or("没检测到足够的拍点（素材可能没有稳定节拍）")? as f64;
    if !(bpm.is_finite() && bpm > 0.0) {
        return Err("速度估计不可用".into());
    }
    let time_sig_num = time_signature_num(&analysis);
    progress(1.0);

    Ok(TempoDetection {
        bpm,
        time_sig_num,
        time_sig_den: 4,
        beats: analysis.beats.len(),
        downbeats: analysis.downbeats.len(),
        first_downbeat_sec: analysis.downbeats.first().map(|&t| t as f64),
        duration_sec,
    })
}

/// 拍号分子 = **相邻重拍之间夹着的拍数**的众数（重拍本身算 1 拍）。
///
/// Beat This! 只输出「哪些拍是重拍」，不直接给拍号，也不会猜分母。常见拍号（4/4、3/4）下
/// 这个众数就是分子；分母统一取 4（24/8 那类只在谱面上、听着仍是每小节若干拍）。
/// 重拍不足 2 个（推不出间隔）就退回 4/4。
pub fn time_signature_num(analysis: &BeatAnalysis) -> u8 {
    if analysis.downbeats.len() < 2 {
        return 4;
    }
    // 每个重拍在拍点序列里的下标；相邻下标之差 = 一个小节的拍数。
    let idx: Vec<usize> = analysis
        .downbeats
        .iter()
        .filter_map(|d| {
            let at = analysis.beats.partition_point(|b| *b < *d - 1e-3);
            analysis.beats.get(at).filter(|b| (**b - *d).abs() < 1e-3).map(|_| at)
        })
        .collect();
    let mut counts: std::collections::HashMap<u8, usize> = std::collections::HashMap::new();
    for pair in idx.windows(2) {
        let span = pair[1] - pair[0];
        if (2..=32).contains(&span) {
            *counts.entry(span as u8).or_default() += 1;
        }
    }
    // 频次最高者；并列时取较小者（3/4 比 6/4 更常见）。
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(num, _)| num)
        .unwrap_or(4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use beat_this::Tensor;

    fn analysis(beats: Vec<f32>, downbeats: Vec<f32>) -> BeatAnalysis {
        BeatAnalysis {
            beats,
            downbeats,
            mel: Tensor { shape: vec![1, 0, 128], data: vec![] },
            beat_logits: vec![],
            downbeat_logits: vec![],
        }
    }

    #[test]
    fn four_four_is_detected_from_downbeat_spacing() {
        // 8 拍、每 4 拍一个重拍 → 4/4。
        let a = analysis(
            vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5],
            vec![0.0, 2.0],
        );
        assert_eq!(time_signature_num(&a), 4);
    }

    #[test]
    fn three_four_is_detected_from_downbeat_spacing() {
        // 6 拍、每 3 拍一个重拍 → 3/4。
        let a = analysis(
            vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5],
            vec![0.0, 1.5],
        );
        assert_eq!(time_signature_num(&a), 3);
    }

    #[test]
    fn falls_back_to_four_four_without_enough_downbeats() {
        assert_eq!(time_signature_num(&analysis(vec![0.0, 0.5, 1.0], vec![])), 4);
        assert_eq!(time_signature_num(&analysis(vec![0.0, 0.5, 1.0], vec![0.0])), 4);
    }

    #[test]
    fn mode_wins_over_outliers() {
        // 三个 4 拍小节 + 一个 2 拍的小节：众数应是 4。
        let beats: Vec<f32> = (0..14).map(|i| i as f32 * 0.5).collect();
        let a = analysis(beats, vec![0.0, 2.0, 4.0, 6.0]);
        assert_eq!(time_signature_num(&a), 4);
    }
}
