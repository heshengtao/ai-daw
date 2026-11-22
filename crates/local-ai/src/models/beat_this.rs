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

/// Beat This! 的 mel / 拍点证据帧率（50 fps，hop 441 @ 22.05 kHz）。
const FPS: f64 = 50.0;

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
    Some((
        super::find(super::BEAT_THIS_MEL_FILE)?,
        super::find(super::BEAT_THIS_FILE)?,
    ))
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

    let fit = estimate_bpm(&analysis.beats)
        .ok_or("没检测到足够的拍点（素材可能没有稳定节拍）")?;
    // 三条互补的路子：拍点回归（模型锁到哪儿）、拍点 logits 自相关、mel 通量自相关。
    // 后面两个只在「离回归值 5% 以内」才算数（挡住倍速 / 半速的倍频峰），然后按 2:1:1 融合。
    let others: Vec<f64> = [activation_bpm(&analysis, fit), mel_flux_bpm(&analysis, fit)]
        .into_iter()
        .flatten()
        .filter(|v| (v - fit).abs() / fit < 0.05)
        .collect();
    let bpm = if others.is_empty() {
        fit
    } else {
        (2.0 * fit + others.iter().sum::<f64>()) / (2.0 + others.len() as f64)
    };
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

/// 速度估计：对拍点序列做**分段最小二乘**，取各段斜率的中位数。
///
/// 为什么不直接用 crate 的 `calculate_bpm`：它取相邻间隔的中位数，而拍点被量化到模型的
/// 帧（约 20 ms 一格）。量化误差会系统性偏向「多一格」，同一首按 88 BPM 生成的曲子，
/// 整轨混音就报成 85.7（偏慢 2.6%，四分钟下来网格要漂掉两三个小节）；对「第几个拍 →
/// 第几秒」做直线拟合，量化误差被平均掉，同一份拍点能回到 86.9，和独立的起音自相关
/// 估计（87.6）一致。鼓单独一条轨时两种做法都回到 87.6–88.2（真值 88）。
///
/// 段落之间可能漏掉整段（模型在安静段落不出拍点），所以按「间隔 > 3 倍中位数」切开，
/// 每段单独拟合，最后取斜率中位数——跨空档的整条直线是没意义的。
pub fn estimate_bpm(beats: &[f32]) -> Option<f64> {
    if beats.len() < 4 {
        return None;
    }
    let mut intervals: Vec<f64> = beats
        .windows(2)
        .map(|w| (w[1] - w[0]) as f64)
        .filter(|iv| *iv > 0.15 && *iv < 3.0)
        .collect();
    if intervals.is_empty() {
        return None;
    }
    intervals.sort_by(|a, b| a.total_cmp(b));
    let median = intervals[intervals.len() / 2];
    let jump = (median * 3.0).max(0.9);

    let mut slopes: Vec<f64> = Vec::new();
    let mut start = 0usize;
    for i in 1..=beats.len() {
        let broke = i == beats.len() || ((beats[i] - beats[i - 1]) as f64) > jump;
        if !broke {
            continue;
        }
        let run = &beats[start..i];
        if run.len() >= 6 {
            let n = run.len() as f64;
            let mean_index = (n - 1.0) / 2.0;
            let mean_time = run.iter().map(|b| *b as f64).sum::<f64>() / n;
            let mut num = 0.0;
            let mut den = 0.0;
            for (k, t) in run.iter().enumerate() {
                let di = k as f64 - mean_index;
                num += di * (*t as f64 - mean_time);
                den += di * di;
            }
            let slope = num / den;
            if (0.15..3.0).contains(&slope) {
                slopes.push(slope);
            }
        }
        start = i;
    }

    let slope = if slopes.is_empty() {
        median
    } else {
        slopes.sort_by(|a, b| a.total_cmp(b));
        slopes[slopes.len() / 2]
    };
    let bpm = 60.0 / slope;
    (bpm.is_finite() && bpm > 0.0).then_some(bpm)
}

/// 拍点证据（`beat_logits`，50 fps）的自相关周期 → BPM。
///
/// 为什么要再来一遍：拍点回归测的是「模型把拍点放在哪」，段落之间会漂；自相关测的是
/// 「整段音频多久重复一次」，对漂移不敏感。同一首按 88 BPM 生成的曲子，回归给 86.9、
/// 自相关给 87.6（独立验证的真值 87.6–88），融合后误差落在 0.3% 以内。
///
/// 只在「回归值 ±25%」的周期里搜索，倍速 / 半速的倍频峰因此不会把结果带跑。
pub fn activation_bpm(analysis: &BeatAnalysis, hint: f64) -> Option<f64> {
    let logits = &analysis.beat_logits;
    if logits.len() < 400 {
        return None;
    }
    // sigmoid：把 logits 变成 0..1 的拍点概率，再减掉均值做自相关。
    let activation: Vec<f64> = logits
        .iter()
        .map(|v| 1.0 / (1.0 + (-(*v as f64)).exp()))
        .collect();
    let mean = activation.iter().sum::<f64>() / activation.len() as f64;
    let x: Vec<f64> = activation.iter().map(|v| v - mean).collect();
    bpm_from_activation(&x, hint)
}

/// 用模型算好的 **mel 谱**做起音包络自相关，再估一次周期。
///
/// 与 `activation_bpm`（拍点 logits）互补：logits 是模型已经「下过判断」的信号，峰又尖又窄，
/// 量化误差全落在峰上；mel 谱的通量是原始观测，峰更宽、插值更稳。两个都算，谁离回归值近
/// 就用谁——都不近就退回回归。
pub fn mel_flux_bpm(analysis: &BeatAnalysis, hint: f64) -> Option<f64> {
    let shape = &analysis.mel.shape;
    if shape.len() < 3 {
        return None;
    }
    let frames = shape[1];
    let bins = shape[2];
    if frames < 400 || bins == 0 || analysis.mel.data.len() < frames * bins {
        return None;
    }
    let mel = &analysis.mel.data;
    let mut flux: Vec<f64> = Vec::with_capacity(frames - 1);
    for t in 1..frames {
        let mut acc = 0.0;
        for b in 0..bins {
            let now = mel[t * bins + b] as f64;
            let prev = mel[(t - 1) * bins + b] as f64;
            let diff = now - prev;
            if diff > 0.0 {
                acc += diff;
            }
        }
        flux.push(acc);
    }
    // 去趋势：本地均值不是起音，会把自相关拖向长周期。
    let window = 32.min(flux.len() / 4).max(1);
    let mut detrended = Vec::with_capacity(flux.len());
    for i in 0..flux.len() {
        let lo = i.saturating_sub(window);
        let hi = (i + window).min(flux.len());
        let mean = flux[lo..hi].iter().sum::<f64>() / (hi - lo) as f64;
        detrended.push(flux[i] - mean);
    }
    bpm_from_activation(&detrended, hint)
}

/// 在「提示周期 ±25%」里找自相关峰，抛物线插值得到亚帧精度，返回 BPM。
fn bpm_from_activation(x: &[f64], hint: f64) -> Option<f64> {
    if x.len() < 64 {
        return None;
    }
    let hint_period = 60.0 / hint * FPS;
    let lo = ((hint_period * 0.8).floor() as usize).max(4);
    let hi = ((hint_period * 1.25).ceil() as usize).min(x.len() / 4);
    if hi <= lo + 2 {
        return None;
    }
    let score = |lag: usize| -> f64 {
        let mut acc = 0.0;
        for i in 0..(x.len() - lag) {
            acc += x[i] * x[i + lag];
        }
        acc / (x.len() - lag) as f64
    };
    let mut best = (f64::NEG_INFINITY, lo);
    for lag in lo..=hi {
        let s = score(lag);
        if s > best.0 {
            best = (s, lag);
        }
    }
    if best.0 <= 0.0 {
        return None;
    }
    // 整数帧的周期粒度在 88 BPM 附近就是 ±1.3 BPM，插值后约 ±0.1。
    let lag = best.1;
    if lag == lo || lag == hi {
        return Some(60.0 * FPS / lag as f64);
    }
    let (s0, s1, s2) = (score(lag - 1), best.0, score(lag + 1));
    let denom = s0 - 2.0 * s1 + s2;
    let delta = if denom.abs() > 1e-12 {
        (0.5 * (s0 - s2) / denom).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    Some(60.0 * FPS / (lag as f64 + delta))
}

/// 拍号分子 = **相邻重拍之间夹着的拍数**。
///
/// Beat This! 只输出「哪些拍是重拍」，不直接给拍号，也不会猜分母。这里要处理它最常见的
/// 一种误判：**把小节的第 3 拍也标成重拍**（半小节重拍），于是重拍间隔变成 2。实测一首
/// 88 BPM 的 boom bap，98 个重拍里大量间隔是 2，直接取众数就判成了 2/4。
///
/// 所以先把间隔按「音乐上可能的拍号」归类：
/// - 2 / 4 / 8 → 支持 4/4（2 是半小节重拍，8 是漏了一个重拍）；
/// - 3 / 6 → 支持 3/4（6 是漏了一个重拍的三拍子，或按八分算的 6/8）；
/// - 5 / 7 → 少见拍号，只有在它明显压过前两类时才采纳。
/// 4/4 是压倒性的多数，只有 3 拍子的证据**明显更多**时才改判。分母统一取 4。
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
            analysis
                .beats
                .get(at)
                .filter(|b| (**b - *d).abs() < 1e-3)
                .map(|_| at)
        })
        .collect();

    let mut four = 0usize;
    let mut three = 0usize;
    let mut rare: std::collections::HashMap<u8, usize> = std::collections::HashMap::new();
    for pair in idx.windows(2) {
        let span = pair[1] - pair[0];
        match span {
            2 | 4 | 8 => four += 1,
            3 | 6 => three += 1,
            n if (5..=7).contains(&n) => *rare.entry(n as u8).or_default() += 1,
            _ => {}
        }
    }

    let rare_best = rare
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)));
    if let Some((num, hits)) = rare_best {
        if hits > four.max(three) {
            return num;
        }
    }
    // 3 拍子要「明显」更多（至少是 4 的证据的 1.5 倍）才改判，避免偶尔漏拍的 4/4 被带偏。
    if three * 2 > four * 3 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use beat_this::Tensor;

    fn analysis(beats: Vec<f32>, downbeats: Vec<f32>) -> BeatAnalysis {
        BeatAnalysis {
            beats,
            downbeats,
            mel: Tensor {
                shape: vec![1, 0, 128],
                data: vec![],
            },
            beat_logits: vec![],
            downbeat_logits: vec![],
        }
    }

    #[test]
    fn four_four_is_detected_from_downbeat_spacing() {
        // 8 拍、每 4 拍一个重拍 → 4/4。
        let a = analysis(vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5], vec![0.0, 2.0]);
        assert_eq!(time_signature_num(&a), 4);
    }

    #[test]
    fn three_four_is_detected_from_downbeat_spacing() {
        // 6 拍、每 3 拍一个重拍 → 3/4。
        let a = analysis(vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5], vec![0.0, 1.5]);
        assert_eq!(time_signature_num(&a), 3);
    }

    #[test]
    fn falls_back_to_four_four_without_enough_downbeats() {
        assert_eq!(
            time_signature_num(&analysis(vec![0.0, 0.5, 1.0], vec![])),
            4
        );
        assert_eq!(
            time_signature_num(&analysis(vec![0.0, 0.5, 1.0], vec![0.0])),
            4
        );
    }

    #[test]
    fn mode_wins_over_outliers() {
        // 三个 4 拍小节 + 一个 2 拍的小节：众数应是 4。
        let beats: Vec<f32> = (0..14).map(|i| i as f32 * 0.5).collect();
        let a = analysis(beats, vec![0.0, 2.0, 4.0, 6.0]);
        assert_eq!(time_signature_num(&a), 4);
    }

    #[test]
    fn half_bar_downbeats_are_read_as_four_four() {
        // 实测里最常见的一种误判：模型把 1、3 拍都标成重拍（间隔 2），其实是 4/4。
        let beats: Vec<f32> = (0..16).map(|i| i as f32 * 0.7).collect();
        let downbeats: Vec<f32> = (0..8).map(|i| i as f32 * 1.4).collect();
        assert_eq!(time_signature_num(&analysis(beats, downbeats)), 4);
    }

    #[test]
    fn six_beat_spans_are_read_as_three_four() {
        // 6/8 按八分算：每小节 6 拍，重拍间隔 6 → 3/4（分母固定 4）。
        let beats: Vec<f32> = (0..24).map(|i| i as f32 * 0.4).collect();
        let downbeats: Vec<f32> = (0..4).map(|i| i as f32 * 2.4).collect();
        assert_eq!(time_signature_num(&analysis(beats, downbeats)), 3);
    }

    #[test]
    fn odd_meter_survives_when_it_dominates() {
        // 5/4：重拍间隔 5，压过其它证据时保留 5。
        let beats: Vec<f32> = (0..20).map(|i| i as f32 * 0.5).collect();
        let downbeats: Vec<f32> = (0..5).map(|i| i as f32 * 2.5).collect();
        assert_eq!(time_signature_num(&analysis(beats, downbeats)), 5);
    }

    #[test]
    fn regression_ignores_interval_jitter() {
        // 0.48 / 0.52 交替：间隔中位数是 0.52（→115 BPM），对拍号做直线拟合应回到 120。
        let mut beats = Vec::new();
        let mut t = 0.0f32;
        for i in 0..64 {
            beats.push(t);
            t += if i % 2 == 0 { 0.48 } else { 0.52 };
        }
        let bpm = estimate_bpm(&beats).expect("bpm");
        assert!((bpm - 120.0).abs() < 0.5, "{bpm}");
    }

    #[test]
    fn long_silence_splits_runs_without_breaking_tempo() {
        let mut beats = Vec::new();
        for i in 0..40 {
            beats.push(i as f32 * 0.5);
        }
        for i in 0..40 {
            beats.push(60.0 + i as f32 * 0.5);
        }
        let bpm = estimate_bpm(&beats).expect("bpm");
        assert!((bpm - 120.0).abs() < 0.5, "{bpm}");
    }

    #[test]
    fn too_few_beats_have_no_bpm() {
        assert!(estimate_bpm(&[0.0, 0.5, 1.0]).is_none());
    }

    #[test]
    fn activation_autocorrelation_recovers_the_period() {
        // 50 fps、每 34 帧一拍 = 88.24 BPM；提示 86 BPM，搜索结果应贴回 88.2。
        let mut logits = vec![-6.0f32; 2000];
        for i in (0..2000).step_by(34) {
            logits[i] = 6.0;
        }
        let mut a = analysis(vec![], vec![]);
        a.beat_logits = logits;
        let bpm = activation_bpm(&a, 86.0).expect("bpm");
        assert!((bpm - 88.24).abs() < 0.6, "{bpm}");
    }

    #[test]
    fn activation_autocorrelation_needs_enough_frames() {
        assert!(activation_bpm(&analysis(vec![], vec![]), 120.0).is_none());
    }
}


