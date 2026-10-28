//! **参考声线的自动裁剪**：从一整首歌里挑出最适合当音色 prompt 的 10 秒。
//!
//! 直接把整首歌（或开头 10 秒）当参考有两个问题：开头经常是前奏 / 静音，整首又让
//! prompt 里混进大量非人声段落。这里做一个**轻量 VAD / 选段**，不引模型：
//!
//! 1. 按 50 fps 分帧，每帧算三个特征：
//!    - 能量（RMS）；
//!    - **周期性**（归一化自相关峰）——谐波信号（人声、乐器）高，噪声/鼓低；
//!    - **频谱平坦度**——噪声/镲片接近 1，有明确基频的人声很低。
//! 2. 每帧得分 = 能量 × 周期性 × 音调性，取「目标长度」内平均分最高的窗口，
//!    并给有声帧占比一点加成，避免选到一段全是长音效的间奏。
//! 3. 选中的窗口两端加 30 ms 淡入淡出，避免 prompt 里带爆音。
//!
//! 复杂度：FFT 自相关，4 分钟歌约几十毫秒；**不会**把它当完整 VAD 用，只是「挑一段
//! 更像人声的 10 秒」。

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

/// 选中的窗口。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowChoice {
    /// 起点（秒）。
    pub start_sec: f64,
    /// 长度（秒）。
    pub seconds: f64,
    /// 平均得分（0..=1，仅用于比较）。
    pub score: f32,
    /// 有声帧占比（0..=1）。
    pub voiced_ratio: f32,
}

/// 从整段音频里挑出最好的 `target_sec` 秒，返回（裁剪后的采样，选择信息）。
///
/// `samples` 必须是**单声道**；采样率任意（内部按 50 fps 分帧）。比目标短就整段返回。
pub fn trim_to_best(
    samples: &[f32],
    sample_rate: f64,
    target_sec: f64,
) -> (Vec<f32>, WindowChoice) {
    let choice = best_window(samples, sample_rate, target_sec);
    let sr = sample_rate.max(1.0);
    let start = (choice.start_sec * sr).round() as usize;
    let len = (choice.seconds * sr).round() as usize;
    let start = start.min(samples.len());
    let end = (start + len).min(samples.len());
    let mut out = samples[start..end].to_vec();
    fade_edges(&mut out, (sr * 0.03) as usize);
    (out, choice)
}

/// 只做选段（不复制音频）。
pub fn best_window(samples: &[f32], sample_rate: f64, target_sec: f64) -> WindowChoice {
    let sr = sample_rate.max(1.0);
    let total_sec = samples.len() as f64 / sr;
    if samples.is_empty() || target_sec <= 0.0 {
        return WindowChoice {
            start_sec: 0.0,
            seconds: total_sec.min(target_sec.max(0.0)),
            score: 0.0,
            voiced_ratio: 0.0,
        };
    }
    if total_sec <= target_sec {
        return WindowChoice {
            start_sec: 0.0,
            seconds: total_sec,
            score: 1.0,
            voiced_ratio: 1.0,
        };
    }

    let hop = ((sr / 50.0).round() as usize).max(1);
    let frame = (hop * 2).max(256);
    let fft_len = (frame * 2).next_power_of_two();
    let frames = samples.len().saturating_sub(frame) / hop + 1;
    if frames < 2 {
        return WindowChoice {
            start_sec: 0.0,
            seconds: total_sec,
            score: 0.0,
            voiced_ratio: 0.0,
        };
    }

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(fft_len);
    let ifft = planner.plan_fft_inverse(fft_len);
    let hann: Vec<f32> = (0..frame)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / frame as f32).cos())
        .collect();

    let mut buf: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); fft_len];
    let mut power: Vec<f32> = vec![0.0; fft_len];
    let lag_min = ((sr / 800.0).round() as usize).max(2);
    let lag_max = ((sr / 80.0).round() as usize)
        .min(frame - 1)
        .max(lag_min + 1);

    let mut scores = Vec::with_capacity(frames);
    let mut voiced = Vec::with_capacity(frames);
    for f in 0..frames {
        let base = f * hop;
        let slice = &samples[base..base + frame];
        let rms = (slice.iter().map(|v| v * v).sum::<f32>() / frame as f32).sqrt();
        let rms_db = 20.0 * (rms + 1e-9).log10();

        // 自相关（FFT）：线性相关靠零填充避免循环卷绕。
        buf.iter_mut().for_each(|c| *c = Complex::new(0.0, 0.0));
        for (i, v) in slice.iter().enumerate() {
            buf[i] = Complex::new(v * hann[i], 0.0);
        }
        fft.process(&mut buf);
        for (i, c) in buf.iter().enumerate() {
            power[i] = c.norm_sqr();
        }
        // 逆变换得到自相关；只关心 lag 范围。
        let mut ac: Vec<Complex<f32>> = power.iter().map(|p| Complex::new(*p, 0.0)).collect();
        ifft.process(&mut ac);
        let norm = ac[0].re.max(1e-12);
        let mut periodicity = 0.0f32;
        for lag in lag_min..lag_max {
            let r = ac[lag].re / norm;
            if r > periodicity {
                periodicity = r;
            }
        }
        periodicity = periodicity.clamp(0.0, 1.0);

        // 频谱平坦度：几何均值 / 算术均值（只取到 4 kHz，避开高频噪声主导）。
        let hi = ((4000.0 / sr) * fft_len as f64) as usize;
        let hi = hi.clamp(2, fft_len / 2);
        let mut sum = 0.0f64;
        let mut log_sum = 0.0f64;
        for p in power.iter().take(hi).skip(1) {
            let v = f64::from(*p) + 1e-12;
            sum += v;
            log_sum += v.ln();
        }
        let count = (hi - 1).max(1) as f64;
        let arithmetic = sum / count;
        let geometric = (log_sum / count).exp();
        let flatness = (geometric / arithmetic.max(1e-12)) as f32;
        let tonal = (1.0 - flatness).clamp(0.0, 1.0);

        let energy_norm = ((rms_db + 60.0) / 60.0).clamp(0.0, 1.0);
        let score = energy_norm * periodicity * (0.5 + 0.5 * tonal);
        scores.push(score);
        voiced.push(rms_db > -45.0 && periodicity > 0.45 && flatness < 0.6);
    }

    let target_frames = ((target_sec * 50.0).round() as usize).clamp(1, frames);
    let step = ((sr * 0.25) / hop as f64).round().max(1.0) as usize; // 0.25 秒步进
    let mut prefix = vec![0.0f64; frames + 1];
    let mut prefix_voiced = vec![0u32; frames + 1];
    for i in 0..frames {
        prefix[i + 1] = prefix[i] + f64::from(scores[i]);
        prefix_voiced[i + 1] = prefix_voiced[i] + u32::from(voiced[i]);
    }

    let mut best = (f64::NEG_INFINITY, 0usize, 0.0f32);
    let mut start = 0usize;
    while start + target_frames <= frames {
        let sum = prefix[start + target_frames] - prefix[start];
        let mean = sum / target_frames as f64;
        let ratio = (prefix_voiced[start + target_frames] - prefix_voiced[start]) as f64
            / target_frames as f64;
        let window_score = mean + 0.3 * ratio;
        if window_score > best.0 {
            best = (window_score, start, ratio as f32);
        }
        start += step;
    }
    // 尾部不足一个窗口时，补一个贴住结尾的候选。
    if frames >= target_frames {
        let start = frames - target_frames;
        let sum = prefix[frames] - prefix[start];
        let ratio = (prefix_voiced[frames] - prefix_voiced[start]) as f64 / target_frames as f64;
        let window_score = sum / target_frames as f64 + 0.3 * ratio;
        if window_score > best.0 {
            best = (window_score, start, ratio as f32);
        }
    }

    let start_sec = (best.1 * hop) as f64 / sr;
    WindowChoice {
        start_sec,
        seconds: target_sec.min(total_sec),
        score: (best.0 / 1.3).clamp(0.0, 1.0) as f32,
        voiced_ratio: best.2,
    }
}

/// 两端线性淡入淡出（`n` 个采样）。
fn fade_edges(samples: &mut [f32], n: usize) {
    if n == 0 || samples.len() < 2 * n {
        return;
    }
    let len = samples.len();
    for i in 0..n {
        let g = i as f32 / n as f32;
        samples[i] *= g;
        samples[len - 1 - i] *= g;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f64, seconds: f64, sr: f64) -> Vec<f32> {
        (0..(seconds * sr) as usize)
            .map(|i| (std::f64::consts::TAU * freq * i as f64 / sr).sin() as f32 * 0.5)
            .collect()
    }

    fn silence(seconds: f64, sr: f64) -> Vec<f32> {
        vec![0.0; (seconds * sr) as usize]
    }

    fn noise(seconds: f64, sr: f64) -> Vec<f32> {
        let mut state = 0x1234_5678u32;
        (0..(seconds * sr) as usize)
            .map(|_| {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                ((state >> 16) as f32 / 32768.0 - 1.0) * 0.4
            })
            .collect()
    }

    #[test]
    fn skips_silence_and_noise_to_find_the_tonal_part() {
        let sr = 24_000.0;
        let mut audio = silence(5.0, sr);
        audio.extend(noise(8.0, sr));
        audio.extend(tone(220.0, 12.0, sr));
        audio.extend(silence(4.0, sr));
        let (picked, choice) = trim_to_best(&audio, sr, 10.0);
        assert_eq!(picked.len(), (10.0 * sr) as usize);
        // 有声音的段从 13 秒开始，10 秒窗口应该落在 13..23 附近。
        assert!(
            (choice.start_sec - 13.0).abs() < 1.5,
            "窗口起点应落在有声段：{}",
            choice.start_sec
        );
        assert!(choice.voiced_ratio > 0.5, "应该基本是有声帧");
    }

    #[test]
    fn short_audio_is_returned_whole() {
        let sr = 24_000.0;
        let audio = tone(220.0, 3.0, sr);
        let (picked, choice) = trim_to_best(&audio, sr, 10.0);
        assert_eq!(picked.len(), audio.len());
        assert_eq!(choice.start_sec, 0.0);
        assert!((choice.seconds - 3.0).abs() < 0.01);
    }

    #[test]
    fn window_never_runs_past_the_end() {
        let sr = 24_000.0;
        let audio: Vec<f32> = (0..(12.0 * sr) as usize).map(|_| 0.0).collect();
        let (picked, choice) = trim_to_best(&audio, sr, 10.0);
        assert_eq!(picked.len(), (10.0 * sr) as usize);
        assert!(choice.start_sec + choice.seconds <= 12.0 + 1e-6);
    }
}
