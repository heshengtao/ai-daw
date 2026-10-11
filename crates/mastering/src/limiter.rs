//! **真峰值限幅器**（前瞻 + 4× 过采样）。
//!
//! 工作区里没有任何限幅器 / 过采样器（唯一的饱和原语是 `builtin.drive` 的 `tanh`），所以这里
//! 从零实现。母带是**离线**处理——整段信号一次性在内存里，因此不需要环形缓冲、不需要延迟线、
//! 也不需要「实时安全」的零分配约束；前瞻直接用「前向滑动窗口最小值」表达。
//!
//! 步骤：
//! 1. **4× 过采样**估算真峰值（采样间峰值是采样峰值测不到的），通道间取最大值 → 立体声联动；
//! 2. 每个基频帧算目标增益 `min(1, ceiling / peak)`；
//! 3. 前瞻：对目标增益取**前向**滑动窗口最小值（窗口 = 前瞻时长），这样增益在峰值**之前**就压下去；
//! 4. 释放平滑（一阶），避免增益抖动；
//! 5. 应用增益。
//!
//! 因为第 3 步已经看到了未来，所以**不需要额外的延迟线**——输出第 `n` 个样本用的就是考虑过
//! 后 `lookahead` 个样本之后得到的增益。

use std::collections::VecDeque;
use std::f64::consts::PI;

/// 过采样倍数。
const OVERSAMPLE: usize = 4;
/// 每个相位的滤波器抽头数（`OVERSAMPLE * TAPS_PER_PHASE` = 原型滤波器长度）。
const TAPS_PER_PHASE: usize = 12;
/// 默认前瞻时长（毫秒）。
const LOOKAHEAD_MS: f64 = 5.0;
/// 默认释放时长（毫秒）。
const RELEASE_MS: f64 = 120.0;

/// 限幅结果。
pub(crate) struct Limited {
    /// 实际施加的最大衰减（dB，`0.0` 表示完全没动）。
    pub reduction_db: f64,
}

/// Blackman 窗。
fn blackman(i: f64, n: f64) -> f64 {
    if n <= 1.0 {
        return 1.0;
    }
    let t = i / (n - 1.0);
    0.42 - 0.5 * (2.0 * PI * t).cos() + 0.08 * (4.0 * PI * t).cos()
}

/// 4 倍插值原型滤波器（多相分解，**每个相位各自归一到直流增益 1**）。
///
/// 截止频率取新采样率下的 `0.5 / OVERSAMPLE`——原始信号只占新奈奎斯特的前 `1/OVERSAMPLE`，
/// 再宽的截止会把镜像放进来。
fn interpolator() -> Vec<f32> {
    let len = OVERSAMPLE * TAPS_PER_PHASE;
    let mid = (len - 1) as f64 / 2.0;
    let fc = 0.5 / OVERSAMPLE as f64;
    let mut h = vec![0.0f32; len];
    for (j, slot) in h.iter_mut().enumerate() {
        let t = j as f64 - mid;
        let x = 2.0 * fc * t;
        let sinc = if x.abs() < 1e-9 {
            1.0
        } else {
            (PI * x).sin() / (PI * x)
        };
        *slot = (sinc * blackman(j as f64, len as f64)) as f32;
    }
    for p in 0..OVERSAMPLE {
        let sum: f64 = (0..TAPS_PER_PHASE)
            .map(|k| h[k * OVERSAMPLE + p] as f64)
            .sum();
        if sum.abs() > 1e-12 {
            for k in 0..TAPS_PER_PHASE {
                h[k * OVERSAMPLE + p] = (h[k * OVERSAMPLE + p] as f64 / sum) as f32;
            }
        }
    }
    h
}

/// 用第 `p` 个相位、在第 `n` 个基频帧上求一个插值样本。
///
/// 相位滤波器中心对齐到 `n`，越界按 0 处理（只在信号开头几帧发生）。
fn phase_sample(h: &[f32], x: &[f32], n: usize, p: usize) -> f32 {
    let center = TAPS_PER_PHASE as isize / 2;
    let mut acc = 0.0f32;
    for k in 0..TAPS_PER_PHASE {
        let idx = n as isize - k as isize + center;
        if idx < 0 || idx as usize >= x.len() {
            continue;
        }
        acc += h[k * OVERSAMPLE + p] * x[idx as usize];
    }
    acc
}

/// **前向**滑动窗口最小值：`out[i] = min(x[i .. i + window])`（末尾窗口自动缩短）。
///
/// 单调双端队列，`O(n)`。队列里**值从前到后递增**，所以最小值在队首。
fn sliding_min_forward(x: &[f32], window: usize) -> Vec<f32> {
    let n = x.len();
    let window = window.max(1);
    let mut out = vec![1.0f32; n];
    let mut dq: VecDeque<usize> = VecDeque::new();
    for i in (0..n).rev() {
        let expire = i + window;
        while let Some(&front) = dq.front() {
            if front >= expire {
                dq.pop_front();
            } else {
                break;
            }
        }
        while let Some(&back) = dq.back() {
            if x[back] >= x[i] {
                dq.pop_back();
            } else {
                break;
            }
        }
        dq.push_back(i);
        if let Some(&front) = dq.front() {
            out[i] = x[front];
        }
    }
    out
}

/// 把 `interleaved`（`channels` 声道）的真峰值压到 `ceiling`（线性）以下。
///
/// 就地修改。`ceiling >= 1.0` 时不会做任何事（不可能超）。
pub(crate) fn limit_true_peak(
    interleaved: &mut [f32],
    channels: usize,
    sample_rate: f64,
    ceiling: f32,
) -> Limited {
    let frames = interleaved.len() / channels;
    if frames == 0 || !ceiling.is_finite() || ceiling <= 0.0 {
        return Limited { reduction_db: 0.0 };
    }

    let h = interpolator();
    let mut channel = vec![0.0f32; frames];
    let mut peak = vec![0.0f32; frames];
    for c in 0..channels {
        for n in 0..frames {
            channel[n] = interleaved[n * channels + c];
        }
        for (n, slot) in peak.iter_mut().enumerate() {
            for p in 0..OVERSAMPLE {
                let v = phase_sample(&h, &channel, n, p).abs();
                if v > *slot {
                    *slot = v;
                }
            }
        }
    }

    let mut target = vec![1.0f32; frames];
    for (slot, &p) in target.iter_mut().zip(peak.iter()) {
        if p > ceiling {
            *slot = ceiling / p;
        }
    }

    let lookahead = ((LOOKAHEAD_MS / 1000.0) * sample_rate).round().max(1.0) as usize;
    let z = sliding_min_forward(&target, lookahead);
    let release = ((RELEASE_MS / 1000.0) * sample_rate).max(1.0);
    let rel = (1.0 - (-1.0 / release).exp()) as f32;

    let mut gain = vec![1.0f32; frames];
    let mut g = 1.0f32;
    let mut lowest = 1.0f32;
    for n in 0..frames {
        g = if z[n] < g { z[n] } else { g + (1.0 - g) * rel };
        gain[n] = g;
        if g < lowest {
            lowest = g;
        }
    }

    if lowest >= 1.0 {
        return Limited { reduction_db: 0.0 };
    }

    for n in 0..frames {
        let g = gain[n];
        if g >= 1.0 {
            continue;
        }
        for c in 0..channels {
            interleaved[n * channels + c] *= g;
        }
    }

    let reduction_db = if lowest <= 0.0 {
        f64::INFINITY
    } else {
        -20.0 * (lowest as f64).log10()
    };
    Limited { reduction_db }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 4 倍插值后，一个直流信号应当还是同一个直流（每个相位都归一到 1）。
    #[test]
    fn interpolation_has_unity_gain_per_phase() {
        let h = interpolator();
        for p in 0..OVERSAMPLE {
            let sum: f64 = (0..TAPS_PER_PHASE)
                .map(|k| h[k * OVERSAMPLE + p] as f64)
                .sum();
            assert!((sum - 1.0).abs() < 1e-5, "相位 {p} 直流增益 = {sum}");
        }
    }

    /// 前向窗口最小值。
    #[test]
    fn sliding_min_looks_forward() {
        let x = [1.0f32, 0.5, 1.0, 1.0];
        let out = sliding_min_forward(&x, 2);
        assert_eq!(out, vec![0.5, 0.5, 1.0, 1.0]);
    }

    /// 前向窗口的末尾会自然缩短（不能借到窗口外的值）。
    #[test]
    fn sliding_min_uses_the_remaining_tail() {
        let x = [1.0f32, 1.0, 1.0, 0.25];
        let out = sliding_min_forward(&x, 8);
        assert_eq!(out, vec![0.25, 0.25, 0.25, 0.25]);
    }

    /// 全幅正弦被压到 ceiling 以下，且没有压过头（增益是「刚好够」）。
    #[test]
    fn limiter_holds_a_full_scale_sine_below_the_ceiling() {
        let sr = 48_000.0;
        let frames = 24_000;
        let mut buf = Vec::with_capacity(frames * 2);
        for n in 0..frames {
            let v = (2.0 * PI * 997.0 * n as f64 / sr).sin() as f32;
            buf.push(v);
            buf.push(v);
        }
        let ceiling = 10f32.powf(-1.0 / 20.0);
        let out = limit_true_peak(&mut buf, 2, sr, ceiling);
        let peak = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(
            peak <= ceiling + 1e-4,
            "限幅后峰值 {peak} 超过 ceiling {ceiling}"
        );
        assert!(
            out.reduction_db > 0.5,
            "应当真的压了：{} dB",
            out.reduction_db
        );
    }

    /// 采样间峰值：`fs/4` 且相移 45°，取样恒为 ±0.7071，**真峰值却是 1.0**（+3 dB）。
    /// 只看采样值的限幅器会完全放过它——这条断言就是「为什么必须 4× 过采样」。
    #[test]
    fn limiter_sees_inter_sample_peaks() {
        let sr = 48_000.0;
        let frames = 24_000;
        // sin(πn/2 + π/4)：幅值 1.0 的正弦只被采到 ±0.7071。
        let mut buf = Vec::with_capacity(frames * 2);
        for n in 0..frames {
            let v = ((PI / 2.0) * n as f64 + PI / 4.0).sin() as f32;
            buf.push(v);
            buf.push(v);
        }
        let sample_peak = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let ceiling = 10f32.powf(-1.0 / 20.0); // ≈ 0.891
        assert!(
            sample_peak < ceiling,
            "前提：采样峰值本应在 ceiling 之下（{sample_peak} < {ceiling}）"
        );

        let out = limit_true_peak(&mut buf, 2, sr, ceiling);
        assert!(
            out.reduction_db > 0.5,
            "过采样应当看到采样间峰值并限幅：{} dB",
            out.reduction_db
        );
        let peak = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(
            peak <= ceiling + 1e-3,
            "限幅后峰值 {peak} 超过 ceiling {ceiling}"
        );
    }

    /// 已经在 ceiling 以下的信号原样通过，不做任何衰减。
    #[test]
    fn limiter_leaves_quiet_material_untouched() {
        let mut buf = vec![0.1f32; 4800];
        let before = buf.clone();
        let out = limit_true_peak(&mut buf, 1, 48_000.0, 0.9);
        assert_eq!(buf, before);
        assert_eq!(out.reduction_db, 0.0);
    }
}
