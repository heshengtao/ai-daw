//! **HTDemucs（Demucs v4 混合 Transformer）**：音源分离,把一段混音拆成
//! **鼓 / 贝斯 / 其他 / 人声**四条分轨。
//!
//! 权重是 `gentij/htdemucs-ort` 的 `htdemucs.ort`（**ORT 格式**，源模型 Meta Demucs v4，
//! Apache-2.0）。图里有**两个输入**：原始波形 `input`（`[1,2,T]`，左||右）与 Demucs 的
//! **复数谱 "cac"** `x`（`[1,4,F,Frames]`）；**两个输出**：频域 `output` 与时域 `add_67`。
//! 最终时域结果 = `add_67` + iSTFT(`output`)。
//!
//! 窗口化与 STFT/iSTFT **逐行移植自** `stem-splitter-core`（MIT OR Apache-2.0，见 `NOTICE`），
//! 但改用本仓库自己的 `ort`（rc.13）与 `rustfft`——因此**不引入第二套 ONNX 运行时**。
//!
//! 常量取自模型 manifest：44 100 Hz，窗口 `T = 343 980`，步进 `T/2`；STFT `nfft = 4096`、
//! `hop = 1024` → 谱维 `F = 2048`、`Frames = 336`。权重缺失时**没有内置 DSP 兜底**
//! （分离没有「近似实现」可言），宿主会提示去下载模型。

use std::path::PathBuf;

use ort::session::Session;
use ort::value::Tensor;
use rustfft::num_complex::Complex32;
use rustfft::FftPlanner;

use super::err;
use crate::resample;
use crate::Progress;

/// 模型要求的采样率。
const SR: f64 = 44_100.0;
/// 单个模型窗口的样本数（~7.8 秒 @44.1 kHz）。
const T: usize = 343_980;
/// 谱频点数（`nfft/2`）。
const F_BINS: usize = 2_048;
/// 谱时间帧数。
const FRAMES: usize = 336;
/// STFT 的 FFT 长度。
const NFFT: usize = 4_096;
/// STFT 的帧移。
const STFT_HOP: usize = 1_024;
/// 模型窗口之间的步进（重叠一半；每窗只取前一半样本）。
const WINDOW_HOP: usize = T / 2;

const IN_TIME: &str = "input";
const IN_SPEC: &str = "x";
const OUT_TIME: &str = "add_67";
const OUT_FREQ: &str = "output";

/// 模型文件路径（没下载时为 `None`）。
pub fn model_path() -> Option<PathBuf> {
    super::find(super::HTDEMUCS_FILE)
}

/// 把交错多声道音频分离成四条分轨（样本数与声道数与输入一致）。
///
/// 内部会上下混成 L/R 并重采样到 44 100 Hz，跑完再重采样回源采样率。`progress` 返回 `false`
/// 即取消，返回已写完的部分（宿主据此丢弃、报「已取消」）。
pub fn separate(
    samples: &[f32],
    channels: usize,
    sample_rate: f64,
    progress: Progress,
) -> Result<super::Stems, String> {
    let path = model_path()
        .ok_or_else(|| format!("未找到音源分离权重（{}）", super::HTDEMUCS_FILE))?;
    let mut session =
        crate::backend::session::build(&path, crate::backend::session::GraphRole::Demucs)?;

    let channels = channels.max(1);
    let frames = samples.len() / channels;
    if frames == 0 || sample_rate <= 0.0 {
        return Ok(super::Stems::default());
    }

    // 取 L/R 单声道，再重采样到模型采样率。
    let mut left: Vec<f32> = Vec::with_capacity(frames);
    let mut right: Vec<f32> = Vec::with_capacity(frames);
    for i in 0..frames {
        let base = i * channels;
        let l = samples[base];
        left.push(l);
        right.push(if channels == 1 {
            l
        } else {
            samples.get(base + 1).copied().unwrap_or(l)
        });
    }
    let left = resample::resample(&left, sample_rate, SR);
    let right = resample::resample(&right, sample_rate, SR);
    let n = left.len().min(right.len());
    if n == 0 {
        return Ok(super::Stems::default());
    }

    // 四条分轨在模型采样率下的交错 L/R 缓冲（长度 2*n）。
    let mut out: [Vec<f32>; 4] = std::array::from_fn(|_| vec![0.0f32; 2 * n]);

    let num_windows = if n <= WINDOW_HOP { 1 } else { (n - 1) / WINDOW_HOP + 1 };
    let mut lwin = vec![0.0f32; T];
    let mut rwin = vec![0.0f32; T];
    let mut pos = 0usize;
    let mut done = 0usize;
    while pos < n {
        // 前 97% 是推理；剩下给重采样与落盘。
        if !progress(done as f32 / num_windows as f32 * 0.97) {
            break;
        }
        // 填窗（越界补零）。
        lwin.fill(0.0);
        rwin.fill(0.0);
        let take = T.min(n - pos);
        lwin[..take].copy_from_slice(&left[pos..pos + take]);
        rwin[..take].copy_from_slice(&right[pos..pos + take]);

        let window_out = run_window(&mut session, &lwin, &rwin)?;
        let copy = WINDOW_HOP.min(T).min(n - pos);
        for (s, stem) in out.iter_mut().enumerate() {
            let base = s * 2 * T;
            for i in 0..copy {
                stem[2 * (pos + i)] = window_out[base + i];
                stem[2 * (pos + i) + 1] = window_out[base + T + i];
            }
        }

        done += 1;
        if pos + WINDOW_HOP >= n {
            break;
        }
        pos += WINDOW_HOP;
    }

    // 重采样回源采样率，并按输入的声道数拼装。
    let finish = |interleaved: &[f32]| super::finish_stem(interleaved, channels, sample_rate, frames);
    let [drums, bass, other, vocals] = out;
    Ok(super::Stems {
        stems: vec![
            ("drums", finish(&drums)),
            ("bass", finish(&bass)),
            ("other", finish(&other)),
            ("vocals", finish(&vocals)),
        ],
    })
}

/// 跑一个模型窗口：时域 + 复数谱输入 → 四条分轨的时域结果（长度 `sources*2*T`）。
fn run_window(session: &mut Session, left: &[f32], right: &[f32]) -> Result<Vec<f32>, String> {
    let t = left.len();
    let mut time = Vec::with_capacity(2 * t);
    time.extend_from_slice(left);
    time.extend_from_slice(right);

    let mut spec = Vec::new();
    let (f_bins, frames) = stft_cac_stereo_centered_into(left, right, NFFT, STFT_HOP, &mut spec);
    if f_bins != F_BINS || frames != FRAMES {
        return Err(err(
            "频谱维度不符",
            format!("F={f_bins}, Frames={frames}，应为 F={F_BINS}, Frames={FRAMES}"),
        ));
    }

    let time_in =
        Tensor::from_array(([1usize, 2, t], time)).map_err(|e| err("构造时域输入失败", e))?;
    let spec_in = Tensor::from_array(([1usize, 4, f_bins, frames], spec))
        .map_err(|e| err("构造频谱输入失败", e))?;
    let outputs = session
        .run(ort::inputs![IN_TIME => time_in, IN_SPEC => spec_in])
        .map_err(|e| err("音源分离推理失败", e))?;

    let (shape_time, time_data) = outputs[OUT_TIME]
        .try_extract_tensor::<f32>()
        .map_err(|e| err("读取时域输出失败", e))?;
    let (shape_freq, freq_data) = outputs[OUT_FREQ]
        .try_extract_tensor::<f32>()
        .map_err(|e| err("读取频域输出失败", e))?;
    if shape_time.len() != 4 || shape_time[2] != 2 || shape_time[3] != t as i64 {
        return Err(err("时域输出形状不符", format!("{shape_time:?}")));
    }
    let num_sources = shape_time[1] as usize;
    if shape_freq.len() != 5
        || shape_freq[1] != num_sources as i64
        || shape_freq[2] != 4
        || shape_freq[3] != F_BINS as i64
        || shape_freq[4] != FRAMES as i64
    {
        return Err(err("频域输出形状不符", format!("{shape_freq:?}")));
    }

    // 时域头 + 频域头 iSTFT 相加。
    let mut out = time_data.to_vec();
    for s in 0..num_sources {
        let off = s * 4 * f_bins * frames;
        let (l, r) = istft_cac_stereo(
            &freq_data[off..off + 4 * f_bins * frames],
            f_bins,
            frames,
            NFFT,
            STFT_HOP,
            t,
        );
        let base = s * 2 * t;
        for i in 0..t {
            out[base + i] += l[i];
            out[base + t + i] += r[i];
        }
    }
    Ok(out)
}

/// 对称 Hann 窗（分母 `n-1`），与上游移植一致（区别于 `crate::hann` 的周期窗）。
fn symmetric_hann(n: usize) -> Vec<f32> {
    if n <= 1 {
        return vec![1.0];
    }
    let denom = (n - 1) as f32;
    (0..n)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / denom).cos())
        .collect()
}

/// 立体声「复数谱（cac）+ 居中补零」STFT。布局 `[4, F, frames]`：
/// L.re、L.im、R.re、R.im，每个平面按 `fi*frames + fr` 展平。返回 `(F, frames)`。
fn stft_cac_stereo_centered_into(
    left: &[f32],
    right: &[f32],
    n_fft: usize,
    hop: usize,
    out: &mut Vec<f32>,
) -> (usize, usize) {
    let t = left.len();
    let pad = n_fft / 2;
    let padded_len = pad + t + pad;
    let mut l_sig = vec![0.0f32; padded_len];
    let mut r_sig = vec![0.0f32; padded_len];
    l_sig[pad..pad + t].copy_from_slice(left);
    r_sig[pad..pad + t].copy_from_slice(right);

    let frames = 1 + (t / hop);
    let f_bins = n_fft / 2;

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n_fft);
    let window = symmetric_hann(n_fft);

    out.clear();
    out.resize(4 * f_bins * frames, 0.0);

    let mut buf_l = vec![Complex32::new(0.0, 0.0); n_fft];
    let mut buf_r = vec![Complex32::new(0.0, 0.0); n_fft];
    for fr in 0..frames {
        let start = fr * hop;
        let li = &l_sig[start..start + n_fft];
        let ri = &r_sig[start..start + n_fft];
        for i in 0..n_fft {
            let w = window[i];
            buf_l[i] = Complex32::new(li[i] * w, 0.0);
            buf_r[i] = Complex32::new(ri[i] * w, 0.0);
        }
        fft.process(&mut buf_l);
        fft.process(&mut buf_r);
        for fi in 0..f_bins {
            let base_fr = fi * frames + fr;
            out[base_fr] = buf_l[fi].re;
            out[f_bins * frames + base_fr] = buf_l[fi].im;
            out[2 * f_bins * frames + base_fr] = buf_r[fi].re;
            out[3 * f_bins * frames + base_fr] = buf_r[fi].im;
        }
    }
    (f_bins, frames)
}

/// 从「复数谱 + 居中补零」STFT 重建一串立体声信号（长度 `target_length`）。
/// 用于把模型频域头还原到时域。
fn istft_cac_stereo(
    spec: &[f32],
    f_bins: usize,
    frames: usize,
    n_fft: usize,
    hop: usize,
    target_length: usize,
) -> (Vec<f32>, Vec<f32>) {
    let pad = n_fft / 2;
    let padded_length = target_length + 2 * pad;
    let mut left = vec![0.0f32; padded_length];
    let mut right = vec![0.0f32; padded_length];
    let mut norm = vec![0.0f32; padded_length];

    let mut planner = FftPlanner::<f32>::new();
    let ifft = planner.plan_fft_inverse(n_fft);
    let window = symmetric_hann(n_fft);
    let scale = 1.0 / n_fft as f32;

    let mut buf_l = vec![Complex32::new(0.0, 0.0); n_fft];
    let mut buf_r = vec![Complex32::new(0.0, 0.0); n_fft];
    for fr in 0..frames {
        buf_l.fill(Complex32::new(0.0, 0.0));
        buf_r.fill(Complex32::new(0.0, 0.0));
        for fi in 0..f_bins {
            let b = fi * frames + fr;
            buf_l[fi] = Complex32::new(spec[b], spec[f_bins * frames + b]);
            buf_r[fi] = Complex32::new(
                spec[2 * f_bins * frames + b],
                spec[3 * f_bins * frames + b],
            );
        }
        for fi in 1..f_bins {
            buf_l[n_fft - fi] = buf_l[fi].conj();
            buf_r[n_fft - fi] = buf_r[fi].conj();
        }
        buf_l[0].im = 0.0;
        buf_r[0].im = 0.0;
        if f_bins < n_fft && n_fft == 2 * f_bins {
            buf_l[n_fft / 2].im = 0.0;
            buf_r[n_fft / 2].im = 0.0;
        }
        ifft.process(&mut buf_l);
        ifft.process(&mut buf_r);
        let start = fr * hop;
        for i in 0..n_fft {
            let pos = start + i;
            if pos >= padded_length {
                break;
            }
            let w = window[i];
            left[pos] += buf_l[i].re * w * scale;
            right[pos] += buf_r[i].re * w * scale;
            norm[pos] += w * w;
        }
    }
    for i in 0..padded_length {
        let s = norm[i];
        if s > 1e-10 {
            left[i] /= s;
            right[i] /= s;
        }
    }

    let start = pad.min(padded_length);
    let end = (pad + target_length).min(padded_length);
    let mut lo = vec![0.0f32; target_length];
    let mut ro = vec![0.0f32; target_length];
    let copy = end.saturating_sub(start);
    lo[..copy].copy_from_slice(&left[start..end]);
    ro[..copy].copy_from_slice(&right[start..end]);
    (lo, ro)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f64, seconds: f64, sr: f64) -> Vec<f32> {
        let n = (seconds * sr) as usize;
        (0..n)
            .map(|i| (std::f64::consts::TAU * freq * i as f64 / sr).sin() as f32 * 0.3)
            .collect()
    }

    /// 居中补零的 STFT → iSTFT 应能重建原信号（Hann² + 4× 重叠，中间段误差极小）。
    #[test]
    fn stft_istft_round_trips() {
        let n_fft = 1024usize;
        let hop = 256usize;
        let t = 4096usize;
        let left: Vec<f32> = (0..t)
            .map(|i| 0.4 * (i as f32 * 0.031).sin() + 0.2 * (i as f32 * 0.11).cos())
            .collect();
        let right: Vec<f32> = (0..t).map(|i| 0.3 * (i as f32 * 0.017).sin()).collect();

        let mut spec = Vec::new();
        let (f_bins, frames) = stft_cac_stereo_centered_into(&left, &right, n_fft, hop, &mut spec);
        assert_eq!(f_bins, n_fft / 2);
        assert_eq!(frames, 1 + t / hop);

        let (l2, r2) = istft_cac_stereo(&spec, f_bins, frames, n_fft, hop, t);
        // 两端各 1/8 是边界（窗不足），只比较中间。
        let skip = t / 8;
        let mut max_err = 0.0f32;
        for i in skip..t - skip {
            max_err = max_err.max((l2[i] - left[i]).abs()).max((r2[i] - right[i]).abs());
        }
        assert!(max_err < 1e-3, "重建误差过大：{max_err}");
    }

    /// 权重缺失时给出明确错误；存在时跑一小段并检查长度与非静音。
    #[test]
    fn separates_a_synth_mix_when_the_model_is_present() {
        if model_path().is_none() {
            eprintln!(
                "跳过：未找到 {}（先跑 tools/fetch-models.sh）",
                super::super::HTDEMUCS_FILE
            );
            return;
        }
        let sr = 44_100.0;
        let n = 60_000usize; // 短于一个窗口 → 单窗
        let mut interleaved = Vec::with_capacity(n * 2);
        for i in 0..n {
            let t = i as f64 / sr;
            let bass = 0.3 * (std::f64::consts::TAU * 80.0 * t).sin();
            let lead = 0.2 * (std::f64::consts::TAU * 880.0 * t).sin();
            interleaved.push((bass + lead) as f32);
            interleaved.push((bass * 0.9) as f32);
        }
        let stems = separate(&interleaved, 2, sr, &|_| true).expect("分离应成功");
        let by = |name: &str| {
            stems
                .stems
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v)
                .expect("缺分轨")
        };
        for name in ["drums", "bass", "other", "vocals"] {
            assert_eq!(by(name).len(), interleaved.len());
        }
        // 低频更可能在 bass，高频更可能在 other / vocals，此处只要求「有内容」。
        let energy = by("bass")
            .iter()
            .map(|v| v * v)
            .sum::<f32>()
            .max(by("other").iter().map(|v| v * v).sum());
        assert!(energy > 1e-6, "分轨不该全静音");
    }

    /// 取消应尽早返回（同长度、不 panic）。
    #[test]
    fn cancellation_returns_a_same_length_buffer() {
        if model_path().is_none() {
            return;
        }
        let audio = vec![0.0f32; 1_000 * 2];
        let _ = tone(440.0, 0.02, 44_100.0);
        let stems = separate(&audio, 2, 44_100.0, &|_| false).expect("取消也应正常返回");
        let drums = stems.stems.iter().find(|(n, _)| *n == "drums").map(|(_, v)| v);
        assert_eq!(drums.map(Vec::len).unwrap_or(audio.len()), audio.len());
    }
}
