//! 降噪 / 去底噪：STFT 域里的**宽带向下扩展器**（noise gate / expander）。
//!
//! 逐帧估计该帧的总功率，与「安静段的噪声底」比较得到一个**整帧共用**的增益：
//! `gain = sqrt(clamp((P − strength·N)/P, 下限², 1))`，再在对数域/帧间做平滑后乘到该帧的频谱上，
//! 逆变换并重叠相加。
//!
//! 为什么是「整帧一个增益」而不是「逐 bin 各自门控」：相邻帧重叠 75%、样本高度相关，
//! 若逐 bin 各按瞬时功率门控，同一段噪声会在重叠帧里被一帧放开、一帧压死，重叠相加反而
//! 产生失真（实测能量不降反升）。整帧增益的行为可预测（≈ 各帧增益的加权平均），衰减干净。
//!
//! **局限**：它压的是「安静段 / 句子之间的底噪」；垫在持续音乐**下面**的宽频嘶声需要模型版
//! 实现（见 crate 文档）。

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

use crate::{hann, Progress};

/// 分析帧长。
const FRAME: usize = 1024;
/// 帧移（75% 重叠，保证 COLA 重建）。
const HOP: usize = 256;
/// 噪声底取「帧功率」的这个分位数（安静段）。
const NOISE_PERCENTILE: f32 = 0.10;
/// 增益下限（约 −24 dB）：不做「全闭」的门。
const MIN_GAIN: f32 = 0.063;
/// 帧间增益平滑（越大越跟手，越小越平滑）。避免增益突变造成「抽吸」声。
const GAIN_SMOOTH: f32 = 0.35;

/// 对交错多声道音频降噪，返回同形状的样本。
///
/// `strength` 是**过减系数**（在噪声底之上再乘几倍）：`1.5..3` 是常用的去嘶声档位，越大越激进。
/// `progress` 返回 `false` 时取消并返回已处理的部分。
pub fn denoise(
    samples: &[f32],
    channels: usize,
    sample_rate: f64,
    strength: f32,
    progress: Progress,
) -> Vec<f32> {
    let channels = channels.max(1);
    let strength = if strength.is_finite() { strength.clamp(0.0, 8.0) } else { 2.0 };
    let frames = samples.len() / channels;
    let mut out = vec![0.0f32; samples.len()];
    for ch in 0..channels {
        let mono: Vec<f32> = (0..frames).map(|i| samples[i * channels + ch]).collect();
        // 每个声道的进度占 1/channels 的一段。
        let base = ch as f32 / channels as f32;
        let span = 1.0 / channels as f32;
        let Some(denoised) =
            denoise_channel(&mono, sample_rate, strength, &|p| progress(base + p * span))
        else {
            return out; // 取消：返回已完成的声道
        };
        let n = frames.min(denoised.len());
        for i in 0..n {
            out[i * channels + ch] = denoised[i];
        }
    }
    out
}

/// 单声道降噪；取消时返回 `None`。
fn denoise_channel(
    x: &[f32],
    sample_rate: f64,
    strength: f32,
    progress: Progress,
) -> Option<Vec<f32>> {
    let _ = sample_rate;
    if x.len() < FRAME {
        return Some(x.to_vec());
    }
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FRAME);
    let ifft = planner.plan_fft_inverse(FRAME);
    let window = hann(FRAME);
    let frame_count = (x.len() - FRAME) / HOP + 1;
    let bins = FRAME / 2 + 1;

    // 第一遍：每帧的总功率（所有 bin 之和）→ 噪声底 = 安静帧的分位数。
    let mut frame_power = vec![0.0f32; frame_count];
    let mut spec = vec![Complex::new(0.0f32, 0.0); FRAME];
    for f in 0..frame_count {
        if f % 32 == 0 && !progress(0.45 * f as f32 / frame_count as f32) {
            return None;
        }
        let start = f * HOP;
        for i in 0..FRAME {
            spec[i] = Complex::new(x[start + i] * window[i], 0.0);
        }
        fft.process(&mut spec);
        frame_power[f] = spec[..bins].iter().map(|c| c.norm_sqr()).sum();
    }
    let mut sorted = frame_power.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = (((sorted.len() - 1) as f32) * NOISE_PERCENTILE).round() as usize;
    let noise_floor = sorted[idx.min(sorted.len() - 1)].max(1e-12);

    // 第二遍：整帧增益 + 逆变换 + 重叠相加（用窗平方归一化）。
    let mut out = vec![0.0f32; x.len()];
    let mut norm = vec![0.0f32; x.len()];
    let min_gain_sq = MIN_GAIN * MIN_GAIN;
    let mut gain = 1.0f32;
    for f in 0..frame_count {
        if f % 32 == 0 && !progress(0.45 + 0.55 * f as f32 / frame_count as f32) {
            return None;
        }
        // 目标增益：帧功率高出噪声底多少。
        let p = frame_power[f].max(1e-18);
        let target = ((p - strength * noise_floor) / p).clamp(min_gain_sq, 1.0).sqrt();
        // 帧间平滑：避免增益突变造成抽吸。
        gain = GAIN_SMOOTH * target + (1.0 - GAIN_SMOOTH) * gain;

        let start = f * HOP;
        for i in 0..FRAME {
            spec[i] = Complex::new(x[start + i] * window[i], 0.0);
        }
        fft.process(&mut spec);
        for c in spec.iter_mut() {
            *c *= gain;
        }
        ifft.process(&mut spec);
        let scale = 1.0 / FRAME as f32; // rustfft 的逆变换未归一化
        for i in 0..FRAME {
            let w = window[i];
            out[start + i] += spec[i].re * scale * w;
            norm[start + i] += w * w;
        }
    }
    for i in 0..out.len() {
        if norm[i] > 1e-6 {
            out[i] /= norm[i];
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 确定性白噪声（LCG），避免依赖随机数库。
    fn noise(n: usize, amp: f32) -> Vec<f32> {
        let mut state = 0x1234_5678u32;
        (0..n)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 9) as f32 / (1 << 23) as f32 - 1.0) * amp
            })
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        if x.is_empty() {
            return 0.0;
        }
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    /// 某个频率上的幅度（DFT 投影）。
    fn tone_amplitude(x: &[f32], freq: f64, sr: f64) -> f32 {
        let mut re = 0.0f64;
        let mut im = 0.0f64;
        for (i, v) in x.iter().enumerate() {
            let phase = std::f64::consts::TAU * freq * i as f64 / sr;
            re += *v as f64 * phase.cos();
            im -= *v as f64 * phase.sin();
        }
        2.0 * ((re * re + im * im).sqrt() / x.len() as f64) as f32
    }

    #[test]
    fn zero_strength_reconstructs_the_input() {
        // 灵敏度检查：strength=0 → 增益处处为 1 → 输出应几乎等于输入（验证 STFT/iSTFT 正确）。
        let sr = 22_050.0;
        let x = noise((0.5 * sr) as usize, 0.2);
        let out = denoise(&x, 1, sr, 0.0, &|_| true);
        let diff = rms(&x.iter().zip(&out).map(|(a, b)| a - b).collect::<Vec<_>>());
        assert!(diff < 1e-2, "零强度应当近似恒等，实际残差 RMS = {diff}");
    }

    #[test]
    fn noise_only_is_strongly_attenuated() {
        let sr = 22_050.0;
        let audio = noise((1.0 * sr) as usize, 0.2);
        let before = rms(&audio);
        let after = rms(&denoise(&audio, 1, sr, 2.0, &|_| true));
        assert!(after < before * 0.5, "纯噪声应被明显压低：{before} -> {after}");
    }

    #[test]
    fn a_tone_with_gaps_survives_in_the_loud_part() {
        let sr = 22_050.0;
        let n = (1.0 * sr) as usize;
        let tone_frames = (0.35 * n as f64) as usize;
        // 前 35% 有音；全程都有嘶声。
        let hiss = noise(n, 0.08);
        let audio: Vec<f32> = (0..n)
            .map(|i| {
                let t = if i < tone_frames {
                    (std::f64::consts::TAU * 440.0 * i as f64 / sr).sin() as f32 * 0.5
                } else {
                    0.0
                };
                t + hiss[i]
            })
            .collect();

        let head = &audio[..tone_frames - 4096];
        let before = tone_amplitude(head, 440.0, sr);
        let out = denoise(&audio, 1, sr, 2.0, &|_| true);
        let after = tone_amplitude(&out[..tone_frames - 4096], 440.0, sr);
        assert!(after > before * 0.8, "有声段的 440 Hz 成分应当保留：{before} -> {after}");

        // 无声段（只有嘶声）应被压低。
        let tail_in = rms(&audio[tone_frames + 4096..]);
        let tail_out = rms(&out[tone_frames + 4096..]);
        assert!(tail_out < tail_in * 0.5, "无声段的底噪应被压低：{tail_in} -> {tail_out}");
    }

    #[test]
    fn cancellation_returns_partial_output_without_panicking() {
        let sr = 22_050.0;
        let audio = noise((1.0 * sr) as usize, 0.2);
        let out = denoise(&audio, 1, sr, 2.0, &|_| false);
        assert_eq!(out.len(), audio.len(), "取消也要返回同形状缓冲");
    }

    #[test]
    fn stereo_is_processed_per_channel() {
        let sr = 22_050.0;
        let n = (0.5 * sr) as usize;
        let left = noise(n, 0.2);
        let right = vec![0.0f32; n];
        let mut interleaved = Vec::with_capacity(n * 2);
        for i in 0..n {
            interleaved.push(left[i]);
            interleaved.push(right[i]);
        }
        let out = denoise(&interleaved, 2, sr, 2.0, &|_| true);
        assert_eq!(out.len(), interleaved.len());
        // 右声道是纯静音，处理完仍应接近 0。
        let quiet = rms(&out.iter().skip(1).step_by(2).copied().collect::<Vec<_>>());
        assert!(quiet < 1e-6, "静音声道不应被注入噪声：{quiet}");
    }
}
