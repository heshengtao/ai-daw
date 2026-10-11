//! **HTDemucs 6s（Demucs v4 `htdemucs_6s`）—— 自足 ONNX 导出**：混音 → 6 条分轨
//! （鼓 / 贝斯 / 其他 / 人声 / 吉他 / 钢琴）。
//!
//! 与 [`super::demucs`]（需要喂复数谱 "cac"、我们自己算 STFT/iSTFT）不同，这份导出把 STFT
//! **折进了图里**：只有一个输入 `mix`（原始波形 `[1,2,T]`），输出 `stems`（`[1,6,2,T]`）。
//! 因此这里**不需要** `rustfft`——只是「切窗 → 前向 → 取每窗的前半个窗口拼回」。
//!
//! stem 顺序：`[drums, bass, other, vocals, guitar, piano]`。权重是 Meta Demucs v4 家族
//! 的 `htdemucs_6s`（**MIT**）。注意官方说明 `piano` 分轨质量偏弱、`guitar` 尚可。

use std::path::PathBuf;

use ort::session::Session;
use ort::value::Tensor;

use super::err;
use crate::resample;
use crate::Progress;

/// 模型要求的采样率。
const SR: f64 = 44_100.0;
/// 单个模型窗口的样本数（~7.8 秒 @44.1 kHz）。
const T: usize = 343_980;
/// 窗口步进（重叠一半；每窗只取前一半样本）。
const WINDOW_HOP: usize = T / 2;
/// 分轨数与顺序（与官方 `htdemucs_6s` 一致）。
const STEM_NAMES: [&str; 6] = ["drums", "bass", "other", "vocals", "guitar", "piano"];

const IN_MIX: &str = "mix";
const OUT_STEMS: &str = "stems";

/// 模型文件路径（没下载时为 `None`）。
pub fn model_path() -> Option<PathBuf> {
    super::find(super::HTDEMUCS_6S_FILE)
}

/// 把交错多声道音频分离成六条分轨（样本数与声道数与输入一致）。
///
/// 内部会上下混成 L/R 并重采样到 44 100 Hz，跑完再重采样回源采样率。`progress` 返回 `false`
/// 即取消，返回已写完的部分（宿主据此丢弃、报「已取消」）。
pub fn separate(
    samples: &[f32],
    channels: usize,
    sample_rate: f64,
    progress: Progress,
) -> Result<super::Stems, String> {
    let path =
        model_path().ok_or_else(|| format!("未找到 6 分轨权重（{}）", super::HTDEMUCS_6S_FILE))?;
    let mut session = Session::builder()
        .map_err(|e| err("初始化 ONNX Runtime 失败", e))?
        .commit_from_file(path)
        .map_err(|e| err("加载 6 分轨权重失败", e))?;

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

    // 六条分轨在模型采样率下的交错 L/R 缓冲（长度 2*n）。
    let sources = STEM_NAMES.len();
    let mut out: Vec<Vec<f32>> = (0..sources).map(|_| vec![0.0f32; 2 * n]).collect();

    let num_windows = if n <= WINDOW_HOP {
        1
    } else {
        (n - 1) / WINDOW_HOP + 1
    };
    let mut lwin = vec![0.0f32; T];
    let mut rwin = vec![0.0f32; T];
    let mut pos = 0usize;
    let mut done = 0usize;
    while pos < n {
        if !progress(done as f32 / num_windows as f32 * 0.97) {
            break;
        }
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

    Ok(super::Stems {
        stems: STEM_NAMES
            .iter()
            .enumerate()
            .map(|(s, name)| (*name, super::finish_stem(&out[s], channels, sample_rate, frames)))
            .collect(),
    })
}

/// 跑一个模型窗口：波形输入 → 六条分轨的时域结果（长度 `6*2*T`）。
fn run_window(session: &mut Session, left: &[f32], right: &[f32]) -> Result<Vec<f32>, String> {
    let t = left.len();
    let mut mix = Vec::with_capacity(2 * t);
    mix.extend_from_slice(left);
    mix.extend_from_slice(right);
    let input = Tensor::from_array(([1usize, 2, t], mix)).map_err(|e| err("构造输入失败", e))?;
    let outputs = session
        .run(ort::inputs![IN_MIX => input])
        .map_err(|e| err("6 分轨推理失败", e))?;
    let (shape, data) = outputs[OUT_STEMS]
        .try_extract_tensor::<f32>()
        .map_err(|e| err("读取 stems 输出失败", e))?;
    if shape.len() != 4
        || shape[1] != STEM_NAMES.len() as i64
        || shape[2] != 2
        || shape[3] != t as i64
    {
        return Err(err("输出形状不符", format!("{shape:?}")));
    }
    Ok(data.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_a_synth_mix_when_the_model_is_present() {
        if model_path().is_none() {
            eprintln!(
                "跳过：未找到 {}（先跑 tools/fetch-models.sh）",
                super::super::HTDEMUCS_6S_FILE
            );
            return;
        }
        let sr = 44_100.0;
        let n = 60_000usize;
        let mut interleaved = Vec::with_capacity(n * 2);
        for i in 0..n {
            let t = i as f64 / sr;
            let bass = 0.3 * (std::f64::consts::TAU * 80.0 * t).sin();
            let lead = 0.2 * (std::f64::consts::TAU * 880.0 * t).sin();
            interleaved.push((bass + lead) as f32);
            interleaved.push((bass * 0.9) as f32);
        }
        let stems = separate(&interleaved, 2, sr, &|_| true).expect("分离应成功");
        assert_eq!(stems.stems.len(), STEM_NAMES.len());
        let names: Vec<&str> = stems.stems.iter().map(|(n, _)| *n).collect();
        for want in ["drums", "bass", "other", "vocals", "guitar", "piano"] {
            assert!(names.contains(&want), "缺少分轨 {want}");
        }
        for (_, data) in &stems.stems {
            assert_eq!(data.len(), interleaved.len());
        }
    }

    #[test]
    fn cancellation_returns_a_same_length_buffer() {
        if model_path().is_none() {
            return;
        }
        let audio = vec![0.0f32; 1_000 * 2];
        let stems = separate(&audio, 2, 44_100.0, &|_| false).expect("取消也应正常返回");
        assert!(stems.stems.iter().all(|(_, d)| d.len() == audio.len()));
    }
}
