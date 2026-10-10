//! 重采样：**多相窗函数 sinc**（Blackman 窗），只处理单声道 `f32`。
//!
//! 两个模型对采样率很挑：Basic Pitch 要 22 050 Hz、LocalVQE 要 16 000 Hz，而工程里的素材
//! 通常是 44.1 / 48 kHz，所以进出模型都要重采样。线性插值在做**降采样**时会把镜像折回来
//! （音高检测会被高频镜像干扰），因此这里用带限 sinc：截止频率取两个采样率中较低的那个
//! 奈奎斯特，再乘 Blackman 窗抑制旁瓣。

/// 相位表精度：把每个输出点的分数延迟量化成 1/PHASES。
const PHASES: usize = 512;
/// 每侧抽头数（总共 2*TAPS 个抽头）。
const TAPS: usize = 16;

/// 把单声道 `x` 从 `from` Hz 重采样到 `to` Hz。
///
/// 采样率相同（或非法）时直接返回副本。输出长度 = `floor(len * to / from)`。
pub fn resample(x: &[f32], from: f64, to: f64) -> Vec<f32> {
    if x.is_empty() {
        return Vec::new();
    }
    if !(from > 0.0) || !(to > 0.0) || (from - to).abs() < 1e-9 {
        return x.to_vec();
    }
    let ratio = to / from;
    let table = coefficient_table(ratio);
    let n_out = ((x.len() as f64) * ratio).floor().max(0.0) as usize;
    let mut out = Vec::with_capacity(n_out);
    for i in 0..n_out {
        let t = i as f64 / ratio; // 输出点在输入轴上的位置
        let k = t.floor() as i64;
        let frac = t - k as f64;
        let phase = ((frac * PHASES as f64).round() as usize).min(PHASES - 1);
        let row = &table[phase * 2 * TAPS..(phase + 1) * 2 * TAPS];
        let mut acc = 0.0f32;
        for (j, c) in row.iter().enumerate() {
            // 抽头 j 对应的输入下标：k + j - (TAPS-1)（正负两侧各 TAPS 个）
            let idx = k + j as i64 - (TAPS as i64 - 1);
            if idx >= 0 && (idx as usize) < x.len() {
                acc += x[idx as usize] * c;
            }
        }
        out.push(acc);
    }
    out
}

/// 预计算相位表：`PHASES * 2 * TAPS`，每行（一个相位）归一化到直流增益 1。
fn coefficient_table(ratio: f64) -> Vec<f32> {
    // 截止频率（相对输入采样率的归一化频率，0.5 = 输入奈奎斯特）。
    let cutoff = 0.5 * ratio.min(1.0);
    let mut table = vec![0.0f32; PHASES * 2 * TAPS];
    for p in 0..PHASES {
        let frac = p as f64 / PHASES as f64;
        let row = &mut table[p * 2 * TAPS..(p + 1) * 2 * TAPS];
        let mut sum = 0.0f64;
        for j in 0..2 * TAPS {
            // 相对当前输出点的整数偏移（负 = 过去）。
            let d = frac - (j as f64 - (TAPS as f64 - 1.0));
            let v = 2.0 * cutoff * sinc(2.0 * cutoff * d) * blackman(d / TAPS as f64);
            row[j] = v as f32;
            sum += v;
        }
        if sum.abs() > 1e-12 {
            for v in row.iter_mut() {
                *v = (*v as f64 / sum) as f32;
            }
        }
    }
    table
}

fn sinc(u: f64) -> f64 {
    if u.abs() < 1e-9 {
        1.0
    } else {
        let p = std::f64::consts::PI * u;
        p.sin() / p
    }
}

/// Blackman 窗，定义域 `-1..=1` 之外为 0。
fn blackman(t: f64) -> f64 {
    if t.abs() > 1.0 {
        return 0.0;
    }
    let x = (t + 1.0) * 0.5; // 0..1
    0.42 - 0.5 * (std::f64::consts::TAU * x).cos() + 0.08 * (2.0 * std::f64::consts::TAU * x).cos()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, seconds: f64, sr: f64) -> Vec<f32> {
        let n = (seconds * sr) as usize;
        (0..n)
            .map(|i| (std::f64::consts::TAU * freq * i as f64 / sr).sin() as f32 * 0.5)
            .collect()
    }

    /// 某个频率上的幅度（跳过两端各 10% 的边界）。
    fn tone_amplitude(x: &[f32], freq: f64, sr: f64) -> f32 {
        let skip = x.len() / 10;
        let body = &x[skip..x.len().saturating_sub(skip)];
        let mut re = 0.0f64;
        let mut im = 0.0f64;
        for (i, v) in body.iter().enumerate() {
            let phase = std::f64::consts::TAU * freq * (i + skip) as f64 / sr;
            re += *v as f64 * phase.cos();
            im -= *v as f64 * phase.sin();
        }
        2.0 * ((re * re + im * im).sqrt() / body.len() as f64) as f32
    }

    #[test]
    fn same_rate_is_a_copy() {
        let x = vec![0.1f32, -0.2, 0.3];
        assert_eq!(resample(&x, 48_000.0, 48_000.0), x);
    }

    #[test]
    fn dc_is_preserved() {
        // 直流信号重采样后仍应是同一个常数（检验相位表归一化）。
        // 两端各 64 个样本会有正常的边界下垂（FIR 抽头超出信号范围），只检查中间。
        let x = vec![0.25f32; 48_000];
        let y = resample(&x, 48_000.0, 22_050.0);
        for (i, v) in y.iter().enumerate().take(y.len() - 64).skip(64) {
            assert!((v - 0.25).abs() < 1e-3, "第 {i} 个样本 = {v}");
        }
    }

    #[test]
    fn a_midband_tone_survives_a_round_trip() {
        let sr_in = 48_000.0;
        let x = sine(1_000.0, 1.0, sr_in);
        let before = tone_amplitude(&x, 1_000.0, sr_in);
        let down = resample(&x, sr_in, 22_050.0);
        assert_eq!(down.len(), (48_000f64 * 22_050.0 / 48_000.0) as usize);
        let after = tone_amplitude(&down, 1_000.0, 22_050.0);
        assert!(
            (after - before).abs() < before * 0.1,
            "1 kHz 通过 48k→22.05k 应基本无损：{before} -> {after}"
        );
        let back = resample(&down, 22_050.0, sr_in);
        let round = tone_amplitude(&back, 1_000.0, sr_in);
        assert!(
            (round - before).abs() < before * 0.15,
            "往返后 1 kHz 应保持：{before} -> {round}"
        );
    }

    #[test]
    fn out_of_band_tone_is_attenuated_on_downsample() {
        // 20 kHz 在 48 k 里合法，但高于 22.05 k 的奈奎斯特 → 必须被抗镜像滤波器压下去。
        let x = sine(20_000.0, 0.5, 48_000.0);
        let before = tone_amplitude(&x, 20_000.0, 48_000.0);
        let y = resample(&x, 48_000.0, 22_050.0);
        let leaked = tone_amplitude(&y, 48_000.0 - 20_000.0, 22_050.0); // 折回 4 kHz 的镜像
        assert!(leaked < before * 0.05, "镜像应被压掉：{before} -> {leaked}");
    }

    #[test]
    fn upsampling_keeps_the_tone() {
        let x = sine(440.0, 0.5, 16_000.0);
        let before = tone_amplitude(&x, 440.0, 16_000.0);
        let y = resample(&x, 16_000.0, 48_000.0);
        let after = tone_amplitude(&y, 440.0, 48_000.0);
        assert!((after - before).abs() < before * 0.05, "16k→48k：{before} -> {after}");
    }
}
