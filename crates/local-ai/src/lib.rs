#![forbid(unsafe_code)]

//! 本地 AI 能力：**两个真实模型（ONNX）+ 一套内置 DSP 兜底**。
//!
//! 两个能力：
//! - **音频转 MIDI**：有 `models/basic-pitch-nmp.onnx` 时走 [`models::basic_pitch`]
//!   （Spotify Basic Pitch，复音转录）；没有权重时退回 [`pitch::detect_notes`]
//!   （单音音高检测，FFT 自相关 + 抛物线细化）。
//! - **音频增强 / 降噪**：有 `models/localvqe-ns.onnx` 时走 [`models::localvqe`]
//!   （LocalVQE v1.2，联合降噪/去混响）；没有权重时退回 [`denoise::denoise`]
//!   （STFT 域宽带向下扩展器）。
//!
//! 权重不进仓库：跑 `tools/fetch-models.sh` 把它们放进 `models/` 即可生效
//! （[`models::availability`] 可查当前状态）。采样率转换见 [`resample`]。
//!
//! 两件事都跑在**后台工作线程**里（宿主用 `spawn_blocking`），带**进度回调**与**取消**
//! （回调返回 `false` 即尽快中止）。**不在音频回调路径上**，因此可以自由分配。

pub mod backend;
pub mod denoise;
pub mod models;
pub mod pitch;
pub mod resample;

/// 处理进度回调：参数为 0.0..=1.0；返回 `false` 表示**取消**，调用方应尽快返回已完成的部分。
pub type Progress<'a> = &'a dyn Fn(f32) -> bool;

/// 流式文本回调：模型每多吐出一小段就调一次，参数是**这一段的增量**。
///
/// 只有「AI 补全」这类**给人看的文本生成**才需要它（界面上那行「思考中」）；
/// 产语义码那条路传空实现即可——码是给 DiT 的，不是给人读的。
pub type Delta<'a> = &'a dyn Fn(&str);

/// 把交错多声道下混成单声道（取平均）。
pub(crate) fn to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    let channels = channels.max(1);
    if channels == 1 {
        return samples.to_vec();
    }
    let frames = samples.len() / channels;
    (0..frames)
        .map(|i| {
            let base = i * channels;
            samples[base..base + channels].iter().sum::<f32>() / channels as f32
        })
        .collect()
}

/// 周期性 Hann 窗（长度 `n`）。
pub(crate) fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos())
        .collect()
}
