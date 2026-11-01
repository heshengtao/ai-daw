//! **50 fps 基频（F0）跟踪**：YIN 算法的纯 DSP 实现，给音色替换（SVC）当输入。
//!
//! 为什么需要它：SVC 的条件不只来自参考音色，**目标音频的 F0 曲线**才决定旋律 / 音高走向。
//! 官方流程用 RMVPE（一个神经网络）出 F0，但那是另一个模型、另一份权重；本应用选择**不引
//! 额外模型**——SVC 本身已经 2.4 GB，再挂一个 100 MB 级的 F0 模型只为了 20 ms 一帧的音高，
//! 不划算。代价是：**它只适合单声部人声**，混音里的伴奏会干扰周期判断（见模块文档末尾）。
//!
//! 做法（de Cheveigné & Kawahara, *YIN*, 2002）：
//! 1. 按 50 fps（24 kHz 下 hop = 480）分帧，帧长 2048（约 85 ms，覆盖 55 Hz 的 4 个周期）。
//! 2. 用 FFT 算**差分函数** `d(τ) = Σ(x_j − x_{j+τ})²`（Wiener–Khinchin：能量前缀和 +
//!    互相关），再取累积均值归一化 `d'(τ) = d(τ)·τ / Σ_{k≤τ} d(k)`。
//! 3. 取第一个低于阈值 `0.15` 的局部极小；没有就用全局最小（仍须 < 0.4 才算有声）。
//! 4. 抛物线插值细化周期，得到 Hz；清音 / 静音为 0。
//! 5. 只修**八度跳变**（与局部中值差超过 600 音分才用中值替换），保留颤音等正常波动。
//!
//! 为什么不用现成的 [`crate::pitch`]：那是「音频转 MIDI」的量化检测（帧长 2048 / hop 512、
//! 输出 MIDI 号、把相邻帧并成音符），会抹掉颤音与滑音；SVC 要的是逐帧连续曲线。
//!
//! **素材要求**：单人人声（清唱、干声、分离后的人声轨）效果最好；直接把整首带伴奏的歌
//! 丢进来，周期检测会被鼓 / 和弦带偏，得到的 F0 基本不可用。界面上的说明与 MCP 工具的描述
//! 都写明了这一点。

use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

/// 帧率（与 SVC 的 mel 帧率一致）。
pub const FPS: f64 = 50.0;
/// 搜索下限（Hz）。SoulX-Singer 的 F0 档位从 32.7 Hz 起，但人声不会低到这里。
pub const MIN_HZ: f32 = 55.0;
/// 搜索上限（Hz）。
pub const MAX_HZ: f32 = 1100.0;
/// 分析帧长（采样点，24 kHz 下约 85 ms）。
const FRAME: usize = 2048;
/// YIN 的绝对阈值：第一个低于它的谷就算周期。
const YIN_THRESHOLD: f32 = 0.15;
/// 累积均值归一化差的上限：超过它一律视为清音。
const MAX_CMND: f32 = 0.40;
/// 低于这个 RMS 视为静音（不再做周期判断）。
const MIN_RMS: f32 = 1e-4;
/// 八度修正的容差（音分）：与局部中值差距小于它的帧原样保留。
const OCTAVE_TOLERANCE_CENTS: f32 = 600.0;

/// 一段单声道音频 → 50 fps 的 F0（Hz；清音 / 静音为 0）。
///
/// 返回长度是 `ceil(samples / hop)`，与 SVC 的 mel 帧数（`floor`）兼容——调用方按 mel 长度
/// 截断或补零即可。`samples` 建议先重采样到 24 kHz（[`crate::resample`]）。
pub fn track(samples: &[f32], sample_rate: f64) -> Vec<f32> {
    if samples.is_empty() || !(sample_rate.is_finite() && sample_rate > 0.0) {
        return Vec::new();
    }
    let sr = sample_rate as f32;
    let hop = ((sample_rate / FPS).round() as usize).max(1);
    let frames = samples.len().div_ceil(hop);
    let frame = FRAME.max(hop * 2);
    if frame < 4 {
        return vec![0.0; frames];
    }

    let lag_min = ((sr / MAX_HZ).floor() as usize).max(2);
    let lag_max = ((sr / MIN_HZ).ceil() as usize).min(frame - 2);
    if lag_min + 1 >= lag_max {
        return vec![0.0; frames];
    }

    let fft_len = (frame * 2).next_power_of_two();
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(fft_len);
    let ifft = planner.plan_fft_inverse(fft_len);

    // 两端各补半帧，让第 f 帧的中心落在 `(f + 0.5) * hop`（与 mel / F0 的帧格对齐）。
    let pad = frame / 2;
    let mut padded = vec![0.0f32; pad + samples.len() + frame];
    padded[pad..pad + samples.len()].copy_from_slice(samples);

    let mut buf: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); fft_len];
    let mut prefix = vec![0.0f32; frame + 1];
    let mut diff = vec![0.0f32; lag_max + 1];
    let mut cmnd = vec![1.0f32; lag_max + 1];
    let mut raw = vec![0.0f32; frames];

    for (f, slot) in raw.iter_mut().enumerate() {
        let base = f * hop;
        let seg = &padded[base..base + frame];
        let rms = (seg.iter().map(|v| v * v).sum::<f32>() / frame as f32).sqrt();
        if rms < MIN_RMS {
            continue;
        }

        // 自相关（线性；零填充到 fft_len 避免循环卷绕）。
        // YIN 不加窗：差分函数要求自相关与能量前缀都基于同一段**原始**采样，加了窗两者
        // 就对不上，`d(τ)` 永远压不到阈值以下。
        buf.iter_mut().for_each(|c| *c = Complex::new(0.0, 0.0));
        for (i, v) in seg.iter().enumerate() {
            buf[i] = Complex::new(*v, 0.0);
        }
        fft.process(&mut buf);
        for c in buf.iter_mut() {
            *c = Complex::new(c.norm_sqr(), 0.0);
        }
        ifft.process(&mut buf);
        // rustfft 的逆变换不归一：除以 fft_len 才是真正的线性自相关。
        let inv = 1.0 / fft_len as f32;

        // 能量前缀和：d(τ) = e0(τ) + e1(τ) − 2·r(τ)。
        prefix[0] = 0.0;
        for (i, v) in seg.iter().enumerate() {
            prefix[i + 1] = prefix[i] + v * v;
        }
        let total = prefix[frame];
        diff[0] = 0.0;
        for tau in 1..=lag_max {
            let e0 = prefix[frame - tau];
            let e1 = total - prefix[tau];
            let r = buf[tau].re * inv;
            diff[tau] = (e0 + e1 - 2.0 * r).max(0.0);
        }

        // 累积均值归一化差分。
        let mut running = 0.0f32;
        cmnd[0] = 1.0;
        for tau in 1..=lag_max {
            running += diff[tau];
            cmnd[tau] = diff[tau] * tau as f32 / running.max(1e-12);
        }

        // 第一个低于阈值的谷；找不到就用全局最小。
        let mut found = None;
        let mut tau = lag_min;
        while tau < lag_max {
            if cmnd[tau] < YIN_THRESHOLD {
                while tau + 1 < lag_max && cmnd[tau + 1] < cmnd[tau] {
                    tau += 1;
                }
                found = Some(tau);
                break;
            }
            tau += 1;
        }
        let tau = found.unwrap_or_else(|| {
            let mut best = lag_min;
            for t in lag_min..lag_max {
                if cmnd[t] < cmnd[best] {
                    best = t;
                }
            }
            best
        });
        if cmnd[tau] > MAX_CMND {
            continue;
        }

        // 抛物线细化（用谷底两侧的 CMND 值）。
        let mut period = tau as f32;
        if tau > lag_min && tau + 1 < lag_max {
            let y0 = cmnd[tau - 1];
            let y1 = cmnd[tau];
            let y2 = cmnd[tau + 1];
            let denom = y0 - 2.0 * y1 + y2;
            if denom.abs() > 1e-9 {
                period += 0.5 * (y0 - y2) / denom;
            }
        }
        let hz = sr / period.max(1.0);
        if (MIN_HZ..=MAX_HZ).contains(&hz) {
            *slot = hz;
        }
    }

    fix_octave_jumps(&mut raw);
    raw
}

/// 只修**八度跳变**：与 ±2 帧内有声帧的中值相差超过 600 音分时改用中值。
///
/// 不做整体中值滤波：那会把颤音（4–7 Hz，±50 音分左右）一并抹平。YIN 的八度错误通常
/// 只出现在换声 / 气声的几帧上，局部中值足够了。
fn fix_octave_jumps(values: &mut [f32]) {
    let original = values.to_vec();
    let mut neighbors: Vec<f32> = Vec::with_capacity(5);
    for i in 0..original.len() {
        if original[i] <= 0.0 {
            continue;
        }
        neighbors.clear();
        let lo = i.saturating_sub(2);
        let hi = (i + 3).min(original.len());
        neighbors.extend(original[lo..hi].iter().copied().filter(|v| *v > 0.0));
        neighbors.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = neighbors[neighbors.len() / 2];
        let cents = 1200.0 * (original[i] / median).log2().abs();
        if cents > OCTAVE_TOLERANCE_CENTS {
            values[i] = median;
        }
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

    fn median_voiced(v: &[f32]) -> f32 {
        let mut voiced: Vec<f32> = v.iter().copied().filter(|x| *x > 0.0).collect();
        voiced.sort_by(|a, b| a.partial_cmp(b).unwrap());
        voiced[voiced.len() / 2]
    }

    #[test]
    fn frame_count_matches_the_20ms_grid() {
        let audio = vec![0.0f32; 24_000 + 1];
        assert_eq!(track(&audio, 24_000.0).len(), 51, "ceil(24001 / 480) = 51");
    }

    #[test]
    fn tracks_a_sustained_tone() {
        let audio = tone(220.0, 1.0, 24_000.0);
        let f0 = track(&audio, 24_000.0);
        assert_eq!(f0.len(), 50);
        assert!(f0.iter().all(|v| *v > 0.0), "稳定正弦应全程有声");
        let med = median_voiced(&f0);
        assert!((med - 220.0).abs() < 4.0, "220 Hz 应落在 ±2% 内：{med}");
    }

    #[test]
    fn tracks_a_high_voice() {
        let audio = tone(660.0, 0.5, 24_000.0);
        let f0 = track(&audio, 24_000.0);
        let med = median_voiced(&f0);
        assert!((med - 660.0).abs() < 12.0, "660 Hz 应落在 ±2% 内：{med}");
    }

    #[test]
    fn silence_is_all_unvoiced() {
        let audio = vec![0.0f32; 48_000];
        assert!(track(&audio, 24_000.0).iter().all(|v| *v == 0.0));
    }

    #[test]
    fn silence_before_and_after_a_tone_keeps_zeros() {
        let mut audio = vec![0.0f32; 12_000];
        audio.extend(tone(220.0, 1.0, 24_000.0));
        audio.extend(std::iter::repeat_n(0.0f32, 12_000));
        let f0 = track(&audio, 24_000.0);
        assert_eq!(f0[0], 0.0);
        assert_eq!(*f0.last().unwrap(), 0.0);
        let voiced = f0.iter().filter(|v| **v > 0.0).count();
        assert!(voiced >= 40, "中间 1 秒应基本有声：{voiced}");
    }

    #[test]
    fn octave_jumps_are_pulled_back_to_the_local_median() {
        let mut values = vec![220.0f32; 21];
        values[10] = 440.0; // 单帧跳八度
        fix_octave_jumps(&mut values);
        assert!((values[10] - 220.0).abs() < 1.0);
        // 正常颤音（±50 音分）不被抹平。
        let mut vibrato = vec![220.0f32; 21];
        vibrato[10] = 226.0;
        fix_octave_jumps(&mut vibrato);
        assert!((vibrato[10] - 226.0).abs() < 0.01);
    }
}
