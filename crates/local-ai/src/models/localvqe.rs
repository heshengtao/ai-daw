//! **LocalVQE v1.2**（DeepVQE 的流式 CPU 派生版，Apache-2.0）：联合降噪 / 去混响。
//!
//! 权重是我们从官方 `localvqe-v1.2-1.3M.pt`（sha256 与上游 manifest 一致）导出的 ONNX，
//! 见 `tools/export-localvqe-onnx.py`。导出时做了两件事：
//!
//! 1. **去掉 `AlignBlock`**：它是给 AEC 用的远端参考对齐，需要第二个输入 `ref`；降噪场景没有
//!    远端信号（传零），此时该分支对输出的影响只有 ~1e-4 相对量级，实测与官方图（ref=0）
//!    的最大偏差 4.5e-5。去掉之后图里只剩单输入 `mic`，也顺带绕开了该模块在 torch→ONNX
//!    导出时的数值不稳。
//! 2. **固定长度**：官方的 PyTorch 参考是**整段**前向（`(B, 128, T, 65)` 这类中间张量随 T 线性
//!    增长，几分钟的素材会吃掉几百 MB），且动态长度导出会踩到 `Unfold`/`Pad` 的形状特化。
//!    这里按固定 2 秒（32 000 样本 @16 kHz）导出，由本模块做 50% 重叠的 Hann 交叠相加。
//!
//! 模型是 **16 kHz 语音**降噪（DNS / AEC Challenge 训练）：对人声很有效，但对**纯音乐**会
//! 当成噪声压掉（实测 220 Hz 正弦 + 噪声的 RMS 从 0.218 掉到 0.012，与官方 PyTorch 参考的
//! 0.012 一致，即模型本身的行为）。用它处理音乐素材前请先试听。

use std::path::PathBuf;

use ort::session::Session;
use ort::value::Tensor;

use crate::resample;
use crate::{hann, Progress};

/// 模型要求的采样率。
const SR: f64 = 16_000.0;
/// 导出时的固定输入长度（2 秒）。
const CHUNK: usize = 32_000;
/// 分块步进（1 秒 → 50% 重叠）。
const HOP: usize = CHUNK / 2;

const MODEL_INPUT: &str = "mic";
const MODEL_OUTPUT: &str = "out";

/// 模型文件路径（没下载时为 `None`）。
pub fn model_path() -> Option<PathBuf> {
    super::find(super::LOCALVQE_FILE)
}

/// 对交错多声道音频降噪，返回同长度、同声道数的样本。
///
/// 每个声道独立处理（保住立体声像）。`progress` 返回 `false` 即取消。
pub fn enhance(
    samples: &[f32],
    channels: usize,
    sample_rate: f64,
    progress: Progress,
) -> Result<Vec<f32>, String> {
    let path = model_path().ok_or_else(|| format!("未找到 LocalVQE 权重（{}）", super::LOCALVQE_FILE))?;
    let mut session =
        crate::backend::session::build(&path, crate::backend::session::GraphRole::LocalVqe)?;

    let channels = channels.max(1);
    if samples.is_empty() || sample_rate <= 0.0 {
        return Ok(vec![0.0; samples.len()]);
    }
    let frames = samples.len() / channels;
    let mut out = vec![0.0f32; samples.len()];
    for ch in 0..channels {
        let mono: Vec<f32> = (0..frames).map(|i| samples[i * channels + ch]).collect();
        let base = ch as f32 / channels as f32;
        let span = 1.0 / channels as f32;
        let Some(clean) = enhance_channel(&mut session, &mono, sample_rate, &|p| progress(base + p * span))
        else {
            return Ok(out); // 取消：返回已完成的声道
        };
        let n = frames.min(clean.len());
        for i in 0..n {
            out[i * channels + ch] = clean[i];
        }
    }
    Ok(out)
}

/// 单声道降噪；取消时返回 `None`。
fn enhance_channel(
    session: &mut Session,
    x: &[f32],
    sample_rate: f64,
    progress: Progress,
) -> Option<Vec<f32>> {
    if x.is_empty() {
        return Some(Vec::new());
    }
    let audio = resample::resample(x, sample_rate, SR);
    let window = hann(CHUNK);
    let mut acc = vec![0.0f32; audio.len()];
    let mut norm = vec![0.0f32; audio.len()];

    let chunks = if audio.len() <= CHUNK { 1 } else { (audio.len() - 1) / HOP + 1 };
    let mut chunk = vec![0.0f32; CHUNK];
    for c in 0..chunks {
        if !progress(c as f32 / chunks as f32) {
            return None;
        }
        let start = c * HOP;
        chunk.fill(0.0);
        let end = (start + CHUNK).min(audio.len());
        chunk[..end - start].copy_from_slice(&audio[start..end]);

        let input = Tensor::from_array(([1usize, CHUNK], chunk.clone())).ok()?;
        let outputs = session.run(ort::inputs![MODEL_INPUT => input]).ok()?;
        let (_, clean) = outputs[MODEL_OUTPUT].try_extract_tensor::<f32>().ok()?;

        for i in 0..CHUNK {
            let t = start + i;
            if t >= acc.len() {
                break;
            }
            let w = window[i];
            acc[t] += clean[i] * w;
            norm[t] += w * w;
        }
    }
    for (i, v) in acc.iter_mut().enumerate() {
        if norm[i] > 1e-6 {
            *v /= norm[i];
        }
    }
    let back = resample::resample(&acc, SR, sample_rate);
    let mut back = back;
    back.resize(x.len(), 0.0);
    Some(back)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_stays_silent_when_the_model_is_present() {
        if model_path().is_none() {
            eprintln!("跳过：未找到 {}（先跑 tools/fetch-models.sh）", super::super::LOCALVQE_FILE);
            return;
        }
        let n = 32_000;
        let silence = vec![0.0f32; n];
        let out = enhance(&silence, 1, 16_000.0, &|_| true).expect("降噪应成功");
        assert_eq!(out.len(), n);
        let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak < 1e-3, "静音不该被放大出声音：峰值 {peak}");
    }

    #[test]
    fn white_noise_is_suppressed_when_the_model_is_present() {
        if model_path().is_none() {
            return;
        }
        let sr = 16_000.0;
        let n = 32_000;
        let mut state = 0x1234_5678u32;
        let noise: Vec<f32> = (0..n)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 9) as f32 / (1 << 23) as f32 - 1.0) * 0.2
            })
            .collect();
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        let before = rms(&noise);
        let out = enhance(&noise, 1, sr, &|_| true).expect("降噪应成功");
        let after = rms(&out);
        assert!(after < before * 0.2, "纯噪声应被显著压低：{before} -> {after}");
    }

    #[test]
    fn stereo_is_processed_per_channel_when_the_model_is_present() {
        if model_path().is_none() {
            return;
        }
        let n = 16_000;
        let mut interleaved = Vec::with_capacity(n * 2);
        for i in 0..n {
            let t = i as f32 / 16_000.0;
            interleaved.push((std::f32::consts::TAU * 220.0 * t).sin() * 0.3);
            interleaved.push(0.0);
        }
        let out = enhance(&interleaved, 2, 16_000.0, &|_| true).expect("降噪应成功");
        assert_eq!(out.len(), interleaved.len());
        let right: Vec<f32> = out.iter().skip(1).step_by(2).copied().collect();
        let peak = right.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak < 1e-3, "静音声道不该被注入声音：峰值 {peak}");
    }

    #[test]
    fn cancellation_returns_a_same_length_buffer() {
        if model_path().is_none() {
            return;
        }
        let audio = vec![0.1f32; 48_000];
        let out = enhance(&audio, 1, 16_000.0, &|_| false).expect("取消也应正常返回");
        assert_eq!(out.len(), audio.len());
    }
}
