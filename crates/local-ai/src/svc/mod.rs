//! **音色替换（SVC）**：把一段人声的**音色**换成参考声线，旋律 / 歌词 / 节奏保留。
//!
//! 输入是两段 24 kHz 单声道音频：
//! - `target` —— 要换声的那段人声（决定唱什么、怎么唱）；
//! - `reference` —— 参考音色（决定换成谁的声音）。
//!
//! 三层：
//! - [`f0`] —— YIN 基频跟踪（50 fps），给目标音频提供旋律走向。
//! - [`soulx`] —— 7 张 ONNX 图（mel / F0 嵌入 / whisper 前端 / 条件 / 扩散 / 声码器）。
//! - 这里只做编排：裁参考、拼条件、按长音频分段、拼接输出。
//!
//! **长音频**：目标超过 30 s 时按参考实现的 `build_vocal_segments` 切块（优先在清音段切、
//! 每段 15–30 s、块间留 1 个切点重叠），逐块推理后只把「核心段」写回，接缝处 20 ms 交叉淡化。
//!
//! **电平**：推理前把目标归一化到固定电平（[`normalize_input`]），输出端**逐帧**把包络对齐到
//! 目标（[`match_dynamics`]），最后用 −1 dBFS 起的**前视限幅**兜峰值——不是整轨乘一个标量，
//! 也不是 0.65 起的 tanh 软饱和（那会把持续的大音量唱段压出谐波失真）。
//!
//! **素材要求**：目标与参考都应该是**单声部人声**。混音里的鼓 / 和弦会让 YIN 判错周期，
//! 直接把整首歌当目标会得到跑调的 F0；先做一次「音源分离 → 人声」再替换是推荐流程。

pub mod f0;
pub mod soulx;

pub use soulx::{Precision, SAMPLE_RATE};

use crate::Progress;

/// 参考音色最多取多少秒（与歌声合成一致：10 秒足够表达音色，也让 prompt 段短一些）。
pub const PROMPT_SECONDS: f64 = 10.0;
/// 分段下限（秒）。
pub const MIN_SEGMENT_SECONDS: f64 = 15.0;
/// 分段上限（秒）。目标超过它才需要切块。
pub const MAX_SEGMENT_SECONDS: f64 = 30.0;
/// 清音连续多少帧就当成可切的空档（50 fps → 200 ms）。
pub const UV_FRAMES_THRESHOLD: usize = 10;
/// 峰值上限：前视限幅器的绝对上界（−0.45 dBFS）。
const PEAK_CEILING: f32 = 0.95;
/// 限幅起点（−1 dBFS）：低于它**完全不动**，超过才压——是限幅器，不是饱和器。
const LIMIT_THRESHOLD: f32 = 0.891_251;
/// 限幅器的前视时长：提前看清峰值，给增益下降留出时间（不会把瞬态削平）。
const LIMIT_LOOKAHEAD_SECONDS: f32 = 0.005;
/// 限幅器的释放时间：增益回到 1 的速度。
const LIMIT_RELEASE_SECONDS: f32 = 0.08;
/// 包络匹配的帧长（50 ms @ 24 kHz）。
const ENVELOPE_FRAME_SECONDS: f32 = 0.05;
/// 包络匹配的跳步（25 ms）：比帧长短一半，增益曲线才够平滑。
const ENVELOPE_HOP_SECONDS: f32 = 0.025;
/// 逐帧增益的上下限（约 ±24 dB）：再大就不是「对齐」而是「编造」了。
const MAX_FRAME_GAIN: f32 = 16.0;
/// 逐帧增益的平滑时间常数：压下去要快（10 ms），抬起来要慢（60 ms），避免抽吸。
const GAIN_ATTACK_SECONDS: f32 = 0.01;
/// 见 [`GAIN_ATTACK_SECONDS`]。
const GAIN_RELEASE_SECONDS: f32 = 0.06;
/// 包络比值里给静音留的底（−80 dBFS），避免 0/0。
const ENVELOPE_EPS: f32 = 1e-4;
/// 段与段之间的交叉淡化（20 ms）。
const SEAM_FADE_SECONDS: f32 = 0.02;
/// 整轨电平微调的上限（±6 dB）。
const TRIM_LIMIT: f32 = 2.0;
/// 推理前目标的有声帧 RMS 目标（−20 dBFS）：让 mel / whisper 前端留在训练分布内。
const INPUT_LEVEL: f32 = 0.1;
/// 输入归一化的峰值保护。
const INPUT_PEAK: f32 = 0.95;
/// 输入归一化的增益上限（±24 dB）。
const INPUT_MAX_GAIN: f32 = 16.0;
/// 响度对齐用的帧长（50 ms @ 24 kHz）。
const LOUDNESS_FRAME: usize = 1200;
/// 只统计目标里高于这个 RMS 的帧（约 −40 dBFS）；静音段不参与响度比较。
const ACTIVE_THRESHOLD: f32 = 0.01;

/// 一次替换的结果。
#[derive(Debug, Clone)]
pub struct Take {
    /// 单声道采样（24 kHz）。
    pub samples: Vec<f32>,
    /// 采样率。
    pub sample_rate: f64,
    /// 时长（秒）。
    pub seconds: f64,
    /// 实际推理了几个区段。
    pub segments: usize,
    /// 这次实际用的重量精度。
    pub precision: soulx::Precision,
    /// 实际使用的移调（半音）。
    pub pitch_shift: i32,
    /// 参考声线实际取自原音频的第几秒（自动裁剪）；没有裁剪时为 `None`。
    pub reference_offset: Option<f64>,
    /// 目标音频里有声帧的占比（0..=1，给界面/摘要判断素材是否合适）。
    pub voiced_ratio: f32,
}

/// 一个推理区段（秒）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    /// 实际送进模型的区间（含与前一段的重叠）。
    pub infer_start: f64,
    /// 实际送进模型的区间终点。
    pub infer_end: f64,
    /// 只把这段写回输出（避免重叠处重复）。
    pub core_start: f64,
    /// 核心段终点。
    pub core_end: f64,
}

/// 模型是否就位。
pub fn installed() -> bool {
    soulx::installed()
}

/// 模型所在目录。
pub fn model_dir() -> Option<std::path::PathBuf> {
    soulx::model_dir()
}

/// 按参考实现的 `build_vocal_segments` 切块。
///
/// `f0` 是 50 fps 的 F0 曲线（清音 0）。返回空表示整段都没有足够的有声内容。
pub fn build_segments(f0: &[f32]) -> Vec<Segment> {
    if f0.is_empty() {
        return Vec::new();
    }
    let total = f0.len();
    let min_frames = (MIN_SEGMENT_SECONDS * f0::FPS).round().max(1.0) as usize;
    let max_frames = (MAX_SEGMENT_SECONDS * f0::FPS).round().max(1.0) as usize;
    let mut split_points: Vec<usize> = vec![0];
    {
        let mut append_split_point = |point: usize| {
            let point = point.min(total);
            while point.saturating_sub(*split_points.last().unwrap()) > max_frames {
                let next = split_points.last().unwrap() + max_frames;
                split_points.push(next);
            }
            if point > *split_points.last().unwrap() {
                split_points.push(point);
            }
        };

        // 在足够长的清音段里切：切点放在清音段后 5 帧，给下一段留出起音。
        let mut idx = 0usize;
        while idx < total {
            if f0[idx] == 0.0 {
                let run_start = idx;
                while idx < total && f0[idx] == 0.0 {
                    idx += 1;
                }
                let run_end = idx;
                if run_end - run_start >= UV_FRAMES_THRESHOLD {
                    append_split_point((run_end - 5).max((run_start + run_end) / 2));
                }
            } else {
                idx += 1;
            }
        }
        append_split_point(total);
    }

    let rate = f0::FPS;
    let mut cores: Vec<(f64, f64)> = Vec::new();
    let mut inferred: Vec<(f64, f64)> = Vec::new();
    {
        let mut append_segment = |start_idx: usize, end_idx: usize| {
            cores.push((
                split_points[start_idx] as f64 / rate,
                split_points[end_idx] as f64 / rate,
            ));
            let overlap_start = if start_idx > 0
                && split_points[end_idx] - split_points[start_idx - 1] <= max_frames
            {
                start_idx - 1
            } else {
                start_idx
            };
            inferred.push((
                split_points[overlap_start] as f64 / rate,
                split_points[end_idx] as f64 / rate,
            ));
        };

        let mut segment_start = 0usize;
        let mut segment_end = 1usize;
        while segment_start < split_points.len() - 1 {
            while segment_end < split_points.len()
                && split_points[segment_end] - split_points[segment_start] < min_frames
            {
                segment_end += 1;
            }
            if segment_end >= split_points.len() {
                append_segment(segment_start, split_points.len() - 1);
                break;
            }
            append_segment(segment_start, segment_end);
            segment_start = segment_end;
            segment_end = segment_start + 1;
        }
    }

    // 丢掉几乎全静音的块（与参考实现一致：有声占比 > 5% 且至少 10 帧）。
    let mut out = Vec::with_capacity(cores.len());
    for (core, infer) in cores.iter().zip(inferred.iter()) {
        let start = (infer.0 * rate) as usize;
        let end = ((infer.1 * rate) as usize).min(total);
        if end <= start {
            continue;
        }
        let span = &f0[start..end];
        let voiced = span.iter().filter(|v| **v > 0.0).count();
        if voiced * 20 > span.len() && voiced >= 10 {
            out.push(Segment {
                infer_start: infer.0,
                infer_end: infer.1,
                core_start: core.0,
                core_end: core.1,
            });
        }
    }
    out
}

/// 把一段短特征补齐 / 截断到 `frames × EMB_DIM`。
fn fit(values: &[f32], frames: usize) -> Vec<f32> {
    let dim = soulx::EMB_DIM;
    let mut out = vec![0.0f32; frames * dim];
    let n = values.len().min(out.len());
    out[..n].copy_from_slice(&values[..n]);
    out
}

/// 逐元素相加（`target += values`）。
fn add_in_place(target: &mut [f32], values: &[f32]) {
    for (t, v) in target.iter_mut().zip(values.iter()) {
        *t += *v;
    }
}

/// 整段 RMS（空信号为 0）。
fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|v| v * v).sum::<f32>() / samples.len() as f32).sqrt()
}

/// 目标 / 输出在**两边都有声**的帧上的 RMS（同一批帧）。
///
/// 两边都要有声：SVC 在清音 / 气声 / 换声失败的段落常常输出**纯数字静音**（实测一段
/// 4 分钟素材里 27% 的采样是零）。只看目标是否有声，这些帧会把输出的分母撑大，整轨
/// 于是被硬抬上去——大的地方更容易撞限幅，正是「越补越糊」的来源。
fn active_rms_pair(target: &[f32], output: &[f32]) -> (f32, f32) {
    let n = target.len().min(output.len());
    let mut target_energy = 0.0f64;
    let mut output_energy = 0.0f64;
    let mut count = 0usize;
    let mut start = 0usize;
    while start < n {
        let end = (start + LOUDNESS_FRAME).min(n);
        let t = &target[start..end];
        let o = &output[start..end];
        let ms = |v: &[f32]| {
            (v.iter().map(|s| f64::from(*s) * f64::from(*s)).sum::<f64>() / v.len() as f64).sqrt()
        };
        if ms(t) >= f64::from(ACTIVE_THRESHOLD) && ms(o) >= f64::from(ACTIVE_THRESHOLD) {
            target_energy += t.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>();
            output_energy += o.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>();
            count += t.len();
        }
        start = end;
    }
    if count == 0 {
        return (0.0, 0.0);
    }
    (
        (target_energy / count as f64).sqrt() as f32,
        (output_energy / count as f64).sqrt() as f32,
    )
}

/// 逐帧 RMS 包络（帧长 `frame`、跳步 `hop`；末尾不足一帧按实际长度算）。
fn envelope(samples: &[f32], frame: usize, hop: usize) -> Vec<f32> {
    let frame = frame.max(1);
    let hop = hop.max(1);
    let mut out = Vec::with_capacity(samples.len() / hop + 1);
    let mut start = 0usize;
    while start < samples.len() {
        let end = (start + frame).min(samples.len());
        out.push(rms(&samples[start..end]));
        start += hop;
    }
    out
}

/// 逐帧增益：目标包络 ÷ 输出包络（压下去快、抬起来慢）。
///
/// 这就是「大的压下来、小的地方抬上去」那一步。模型的绝对电平与动态都不受控，
/// 但**目标素材**的相对动态正是用户要听的东西，所以逐帧对齐到目标包络——
/// 只乘一个全局标量（旧做法）对齐的是整轨平均响度，段内动态一点没动。
fn envelope_gains(target: &[f32], output: &[f32]) -> Vec<f32> {
    let frame = (ENVELOPE_FRAME_SECONDS * SAMPLE_RATE as f32).round() as usize;
    let hop = (ENVELOPE_HOP_SECONDS * SAMPLE_RATE as f32).round() as usize;
    let n = target.len().min(output.len());
    if n == 0 {
        return Vec::new();
    }
    let target_env = envelope(&target[..n], frame, hop);
    let output_env = envelope(&output[..n], frame, hop);
    let hop_seconds = hop as f32 / SAMPLE_RATE as f32;
    let attack = 1.0 - (-hop_seconds / GAIN_ATTACK_SECONDS).exp();
    let release = 1.0 - (-hop_seconds / GAIN_RELEASE_SECONDS).exp();
    let mut gains = Vec::with_capacity(target_env.len());
    let mut gain = 1.0f32;
    for (i, (t, o)) in target_env.iter().zip(output_env.iter()).enumerate() {
        let want =
            ((*t + ENVELOPE_EPS) / (*o + ENVELOPE_EPS)).clamp(1.0 / MAX_FRAME_GAIN, MAX_FRAME_GAIN);
        // 第一帧直接落到目标值（否则文件开头会有一段增益爬升，听起来就是「渐强」）；
        // 之后压下去走 attack、抬起来走 release。
        gain = if i == 0 {
            want
        } else if want < gain {
            gain + (want - gain) * attack
        } else {
            gain + (want - gain) * release
        };
        gains.push(gain);
    }
    gains
}

/// 把逐帧增益线性插值到每个采样（帧心对齐）再乘到 `out` 上，避免增益本身产生拉链声。
fn apply_envelope_gains(out: &mut [f32], gains: &[f32], hop: usize) {
    if gains.is_empty() || out.is_empty() {
        return;
    }
    let hop = hop.max(1) as f32;
    let last = gains.len() - 1;
    for (i, v) in out.iter_mut().enumerate() {
        // 第 k 帧的「帧心」在 (k + 0.5) × hop。
        let pos = (i as f32 + 0.5) / hop - 0.5;
        let gain = if pos <= 0.0 {
            gains[0]
        } else {
            let k = pos as usize;
            if k >= last {
                gains[last]
            } else {
                let frac = pos - k as f32;
                gains[k] * (1.0 - frac) + gains[k + 1] * frac
            }
        };
        *v *= gain;
    }
}

/// 前视限幅器：−1 dBFS 以下**一点都不动**，超过才把增益压下来。
///
/// 旧实现是 0.65 起的 `tanh` 软饱和——那是「大音量唱段整句被压出谐波失真」的直接
/// 来源。这里换成标准前视限幅：先向后看 5 ms 求出每个采样需要的增益，再用 80 ms
/// 释放把增益慢慢放回去；波形只在峰值处被压，中低电平的音色不受影响。
fn lookahead_limit(out: &mut [f32]) {
    if out.is_empty() {
        return;
    }
    let look = ((LIMIT_LOOKAHEAD_SECONDS * SAMPLE_RATE as f32).round() as usize).max(1);
    let release = 1.0 - (-1.0 / (LIMIT_RELEASE_SECONDS * SAMPLE_RATE as f32)).exp();

    // 每个采样需要的增益（只有超过阈值才 < 1）。
    let need: Vec<f32> = out
        .iter()
        .map(|v| {
            let a = v.abs();
            if a > LIMIT_THRESHOLD {
                LIMIT_THRESHOLD / a
            } else {
                1.0
            }
        })
        .collect();

    // 向后 `look` 个采样取最小值：增益提前 `look` 个采样就开始降，峰值不会被削平。
    let mut ahead = vec![1.0f32; need.len()];
    let mut window: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
    for i in (0..need.len()).rev() {
        while let Some(&back) = window.back() {
            if need[back] >= need[i] {
                window.pop_back();
            } else {
                break;
            }
        }
        window.push_back(i);
        while let Some(&front) = window.front() {
            if front > i + look {
                window.pop_front();
            } else {
                break;
            }
        }
        ahead[i] = need[*window.front().expect("刚 push 过 i")];
    }

    // 压下去立刻生效（`ahead` 已经提前），放回来走 80 ms 释放。
    let mut gain = 1.0f32;
    for (v, want) in out.iter_mut().zip(ahead.iter()) {
        gain = if *want < gain {
            *want
        } else {
            gain + (*want - gain) * release
        };
        // 阈值已经保证不会过界；`PEAK_CEILING` 是不依赖算术的绝对上界。
        *v = (*v * gain).clamp(-PEAK_CEILING, PEAK_CEILING);
    }
}

/// 按「目标有声帧」的 RMS 把整轨电平拉一次；`limit` 是增益上限（1.0 = 不动）。
fn trim_to_active_rms(target: &[f32], out: &mut [f32], limit: f32) -> f32 {
    let (t, o) = active_rms_pair(target, out);
    if !(t > 1e-4 && o > 1e-4) || limit <= 1.0 {
        return 1.0;
    }
    let trim = (t / o).clamp(1.0 / limit, limit);
    if (trim - 1.0).abs() < 1e-3 {
        return 1.0;
    }
    for v in out.iter_mut() {
        *v *= trim;
    }
    trim
}

/// 输出端的动态与响度：逐帧对齐目标包络 → 整轨兜底 → 前视限幅。
///
/// 顺序是有讲究的：**先**把电平和动态对齐到目标，**再**限幅。限幅带走的那点 RMS
/// 正是「超过 −1 dBFS 的峰值」，不该再靠抬整体增益补回来——那正是旧实现「越补越糊」
/// 的来源。
fn match_dynamics(target: &[f32], out: &mut [f32]) {
    let hop = (ENVELOPE_HOP_SECONDS * SAMPLE_RATE as f32).round() as usize;
    let gains = envelope_gains(target, out);
    apply_envelope_gains(out, &gains, hop);
    // 兜底：逐帧增益被 ±24 dB 夹住的极端素材（整段几乎静音）由整轨再拉一次。
    trim_to_active_rms(target, out, TRIM_LIMIT);
    lookahead_limit(out);
}

/// 目标音频在**有声帧**上的 RMS（`f0` 是 50 fps 的基频曲线，清音为 0）。
fn voiced_rms(target: &[f32], f0: &[f32]) -> f32 {
    let hop = (SAMPLE_RATE / f0::FPS).round() as usize;
    if hop == 0 {
        return rms(target);
    }
    let mut energy = 0.0f64;
    let mut count = 0usize;
    for (i, pitch) in f0.iter().enumerate() {
        if *pitch <= 0.0 {
            continue;
        }
        let start = i * hop;
        if start >= target.len() {
            break;
        }
        let end = (start + hop).min(target.len());
        energy += target[start..end]
            .iter()
            .map(|v| f64::from(*v) * f64::from(*v))
            .sum::<f64>();
        count += end - start;
    }
    if count == 0 {
        return rms(target);
    }
    (energy / count as f64).sqrt() as f32
}

/// 推理前的电平归一化：有声帧 RMS 拉到 [`INPUT_LEVEL`]（只乘一个标量，不动动态）。
///
/// mel / whisper 前端用训练集**固定**的均值方差做标准化，输入电平会直接决定条件的
/// 尺度：素材比训练集响太多就跑到分布外（输出更毛、更容易破），太轻则输出轻、噪声
/// 占比高。只调 ±24 dB，并先做峰值保护，避免把安静素材抬到削顶。
fn normalize_input(target: &[f32], f0: &[f32]) -> Vec<f32> {
    let level = voiced_rms(target, f0);
    if !level.is_finite() || level <= 1e-4 {
        return target.to_vec();
    }
    let mut gain = (INPUT_LEVEL / level).clamp(1.0 / INPUT_MAX_GAIN, INPUT_MAX_GAIN);
    let peak = target.iter().fold(0.0f32, |m, v| m.max(v.abs())) * gain;
    if peak > INPUT_PEAK {
        gain *= INPUT_PEAK / peak;
    }
    if (gain - 1.0).abs() < 1e-3 {
        return target.to_vec();
    }
    target.iter().map(|v| v * gain).collect()
}

/// 把 `piece` 的核心段写回 `out`，开头 `fade` 个采样与前一段已写入的内容交叉淡化。
///
/// 后一段的推理段本来就覆盖前一段的**整段核心**（重叠 = 前一个切点），所以它的
/// `piece` 在接缝前 20 ms 有同一段时间的另一份渲染：把两者混合，电平台阶和咔哒
/// 一起消掉，而不是硬拼。
fn write_core(
    out: &mut [f32],
    dst: usize,
    piece: &[f32],
    core_start: usize,
    len: usize,
    fade: usize,
) {
    if len == 0 || dst >= out.len() {
        return;
    }
    let len = len.min(out.len() - dst);
    let fade = fade.min(core_start).min(len / 2);
    for k in 0..fade {
        let r = (k as f32 + 0.5) / fade as f32;
        out[dst + k] = out[dst + k] * (1.0 - r) + piece[core_start + k] * r;
    }
    let from = core_start + fade;
    let to = core_start + len;
    out[dst + fade..dst + len].copy_from_slice(&piece[from..to]);
}

/// 把参考音色的 F0 对齐到 mel 帧数（多截少补 0）。
fn fit_f0(f0: &[f32], frames: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; frames];
    let n = f0.len().min(frames);
    out[..n].copy_from_slice(&f0[..n]);
    out
}

/// 音色替换。
///
/// - `target`：24 kHz 单声道目标人声。
/// - `reference`：参考音色的单声道采样与采样率（内部重采样到 24 kHz，自动挑最像人声的
///   10 秒）。**必需**——没有参考就无所谓「换成谁的音色」。
/// - `pitch_shift`：整体移调（半音，-24..=24）；男女声对唱时按音域差调整。
/// - `progress`：0..1 进度；返回 `false` 取消。
#[allow(clippy::too_many_arguments)]
pub fn convert(
    target: &[f32],
    reference: Option<(&[f32], f64)>,
    steps: u32,
    seed: u64,
    pitch_shift: i32,
    model: Option<&str>,
    progress: Progress<'_>,
) -> Result<Take, String> {
    let (precision, dir) = soulx::resolve(model)?;
    if target.len() < soulx::HOP {
        return Err("目标音频太短（不足 20 毫秒）".into());
    }
    let Some((ref_samples, ref_rate)) = reference else {
        return Err("音色替换需要一条参考声线（参考轨 / 音色库 / 音频文件）".into());
    };
    if ref_samples.is_empty() || !(ref_rate.is_finite() && ref_rate > 0.0) {
        return Err("参考音色是空音频".into());
    }
    let pitch_shift = pitch_shift.clamp(-24, 24);
    let steps = steps.clamp(4, 64);

    let mut runner = soulx::Runner::load(&dir, &|p| progress(p * 0.20))?;
    if !progress(0.20) {
        return Err("已取消".into());
    }

    // ---- 参考音色：重采样 → 挑 10 秒 → mel / F0 / whisper ----
    let resampled = crate::resample::resample(ref_samples, ref_rate, soulx::SAMPLE_RATE);
    let (picked, choice) =
        crate::singing::reference::trim_to_best(&resampled, soulx::SAMPLE_RATE, PROMPT_SECONDS);
    if picked.len() < soulx::HOP {
        return Err("参考音色太短（不足 20 毫秒）".into());
    }
    let prompt_mel = runner.mel(&picked)?;
    let prompt_frames = prompt_mel.len() / soulx::MEL_DIM;
    if prompt_frames == 0 {
        return Err("参考音色太短，提不出 mel 帧".into());
    }
    let prompt_f0 = fit_f0(&f0::track(&picked, soulx::SAMPLE_RATE), prompt_frames);
    let prompt_emb = runner.f0_embed(&prompt_f0, 0)?;
    let prompt_content = runner.whisper(&picked)?;
    let mut feat_prompt = fit(&prompt_content, prompt_frames);
    add_in_place(&mut feat_prompt, &prompt_emb);
    if !progress(0.28) {
        return Err("已取消".into());
    }

    // ---- 目标：F0 曲线决定旋律，也决定分段 ----
    let target_f0 = f0::track(target, soulx::SAMPLE_RATE);
    if target_f0.is_empty() {
        return Err("目标音频太短，提不出 F0".into());
    }
    let voiced = target_f0.iter().filter(|v| **v > 0.0).count();
    let voiced_ratio = voiced as f32 / target_f0.len() as f32;
    if voiced_ratio < 0.02 {
        return Err("目标音频里没有检测到人声：请换一条人声轨，或先做「音源分离 → 人声」".into());
    }

    let segments = build_segments(&target_f0);
    if segments.is_empty() {
        return Err("目标音频里没有可替换的人声段（几乎全是清音）".into());
    }

    // 推理输入做一次电平归一化：模型的 mel / whisper 前端只认训练集的电平尺度。
    // 目标原始电平（`target`）不参与推理，只用于最终的电平 / 动态对齐。
    let model_input = normalize_input(target, &target_f0);

    let mut out = vec![0.0f32; target.len()];
    let total_segments = segments.len();

    for (index, segment) in segments.iter().enumerate() {
        let base = 0.28 + 0.72 * (index as f32 / total_segments as f32);
        let span = 0.72 / total_segments as f32;
        if !progress(base) {
            return Err("已取消".into());
        }

        let wav_start =
            ((segment.infer_start * soulx::SAMPLE_RATE).round() as usize).min(target.len());
        let wav_end = ((segment.infer_end * soulx::SAMPLE_RATE).round() as usize).min(target.len());
        let f0_start = ((segment.infer_start * f0::FPS).round() as usize).min(target_f0.len());
        let f0_end = ((segment.infer_end * f0::FPS).round() as usize).min(target_f0.len());
        if wav_end <= wav_start || f0_end <= f0_start {
            continue;
        }
        let gt_wav = &model_input[wav_start..wav_end];
        let gt_f0 = &target_f0[f0_start..f0_end];
        let frames = gt_f0.len();

        let gt_emb = runner.f0_embed(gt_f0, pitch_shift)?;
        let gt_content = runner.whisper(gt_wav)?;
        let mut feat_target = fit(&gt_content, frames);
        add_in_place(&mut feat_target, &gt_emb);

        let total_frames = prompt_frames + frames;
        let mut features = Vec::with_capacity(total_frames * soulx::EMB_DIM);
        features.extend_from_slice(&feat_prompt);
        features.extend_from_slice(&feat_target);
        let cond = runner.condition(&features, total_frames)?;

        let seed_segment = seed.wrapping_add(index as u64 * 7919);
        let mel = runner.diffuse(
            &prompt_mel,
            prompt_frames,
            &cond,
            frames,
            steps,
            seed_segment,
            &|p| progress(base + span * (0.02 + 0.83 * p)),
        )?;
        let audio = runner.vocode(&mel, frames)?;
        if !progress(base + span * 0.90) {
            return Err("已取消".into());
        }

        // 模型输出 `frames × 480`；裁 / 补到这一段的原始长度。
        let mut piece = audio;
        if piece.len() > gt_wav.len() {
            piece.truncate(gt_wav.len());
        } else if piece.len() < gt_wav.len() {
            // mel 帧数向下取整，最多短 480 个采样（20 ms）：补 0，并给尾巴做个短淡化，
            // 免得留一个台阶；非末段还会被下一段的接缝交叉淡化盖掉。
            let fade = (gt_wav.len() - piece.len()).min(piece.len());
            let base = piece.len();
            for k in 0..fade {
                piece[base - fade + k] *= 1.0 - (k as f32 + 0.5) / fade as f32;
            }
            piece.resize(gt_wav.len(), 0.0);
        }

        // 只把核心段写回（推理段含有与前一块的重叠）。
        let core_start = ((segment.core_start * soulx::SAMPLE_RATE).round() as usize)
            .saturating_sub(wav_start)
            .min(piece.len());
        let core_end = ((segment.core_end * soulx::SAMPLE_RATE).round() as usize)
            .saturating_sub(wav_start)
            .min(piece.len());
        if core_end <= core_start {
            continue;
        }
        let dst = wav_start + core_start;
        let len = (core_end - core_start).min(out.len().saturating_sub(dst));
        let fade = ((SEAM_FADE_SECONDS * soulx::SAMPLE_RATE as f32).round() as usize)
            .min(piece.len().saturating_sub(core_start));
        write_core(&mut out, dst, &piece, core_start, len, fade);

        if !progress(base + span) {
            return Err("已取消".into());
        }
    }

    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if !(peak.is_finite() && peak > 1e-5) {
        return Err("生成结果是静音（检查参考音色，或换一条目标人声）".into());
    }
    // 电平 / 动态：模型输出的绝对电平不受控、段与段之间还会跳，而用户要听的是目标素材
    // 的相对动态。逐帧把包络对齐到目标，再整轨微调、前视限幅兜峰值（真正的限幅，
    // 不再用 0.65 起的 tanh 软饱和——那会把持续的大音量唱段压出谐波失真）。
    match_dynamics(target, &mut out);

    Ok(Take {
        seconds: out.len() as f64 / soulx::SAMPLE_RATE,
        samples: out,
        sample_rate: soulx::SAMPLE_RATE,
        segments: total_segments,
        precision,
        pitch_shift,
        reference_offset: Some(choice.start_sec),
        voiced_ratio,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一段（含清音空档的）F0 曲线。
    fn curve(voiced_frames: usize, silent_frames: usize) -> Vec<f32> {
        let mut f0 = vec![220.0f32; voiced_frames];
        f0.extend(std::iter::repeat_n(0.0f32, silent_frames));
        f0
    }

    #[test]
    fn short_audio_is_one_segment() {
        let segments = build_segments(&curve(500, 0)); // 10 秒
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].infer_start, 0.0);
        assert!((segments[0].infer_end - 10.0).abs() < 1e-6);
        assert_eq!(segments[0].core_start, 0.0);
        assert!((segments[0].core_end - 10.0).abs() < 1e-6);
    }

    #[test]
    fn long_audio_is_split_within_the_window() {
        let segments = build_segments(&curve(2000, 0)); // 40 秒：必须切
        assert!(segments.len() >= 2, "40 秒应切成多段");
        for s in &segments {
            let len = s.core_end - s.core_start;
            assert!(
                len <= MAX_SEGMENT_SECONDS + 1e-6,
                "核心段不得超过 30 秒：{len}"
            );
            assert!(len > 0.0);
        }
        // 核心段应首尾相接，覆盖整段。
        assert_eq!(segments[0].core_start, 0.0);
        assert!((segments.last().unwrap().core_end - 40.0).abs() < 1e-6);
    }

    #[test]
    fn splits_prefer_the_unvoiced_gap() {
        // 20 秒有声 + 1 秒静音 + 20 秒有声：切点应落在静音附近。
        let mut f0 = curve(1000, 0);
        f0.extend(std::iter::repeat_n(0.0f32, 50));
        f0.extend(curve(1000, 0));
        let segments = build_segments(&f0);
        assert!(segments.len() >= 2);
        let boundary = segments[1].core_start;
        assert!(
            (19.0..=21.5).contains(&boundary),
            "切点应在 20 秒的清音附近：{boundary}"
        );
    }

    #[test]
    fn all_silence_yields_no_segments() {
        assert!(build_segments(&vec![0.0f32; 2000]).is_empty());
    }

    #[test]
    fn fit_pads_and_truncates() {
        let up = fit(&[1.0, 2.0], 2);
        assert_eq!(up.len(), 2 * soulx::EMB_DIM);
        assert_eq!(up[0], 1.0);
        assert_eq!(up[1], 2.0);
        assert_eq!(up[2], 0.0);
        let down = fit(&vec![1.0f32; 4 * soulx::EMB_DIM], 1);
        assert_eq!(down.len(), soulx::EMB_DIM);
        assert!(down.iter().all(|v| *v == 1.0));
    }

    #[test]
    fn rms_is_the_quadratic_mean() {
        assert_eq!(rms(&[]), 0.0);
        let v = [0.5f32, -0.5, 0.5, -0.5];
        assert!((rms(&v) - 0.5).abs() < 1e-6);
        // 幅度减半 → RMS 减半（响度对齐按这个比例缩放）。
        let half: Vec<f32> = v.iter().map(|x| x * 0.5).collect();
        assert!((rms(&half) / rms(&v) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn active_rms_pair_skips_frames_that_are_quiet_on_either_side() {
        // 第 1 帧两边都有声 → 计入；第 2 帧目标有声但输出是数字静音（SVC 很常见）→
        // 不计入，否则整轨会被硬抬；第 3 帧目标静音、输出有噪声 → 同样不计入。
        let f = LOUDNESS_FRAME;
        let mut target = vec![0.1f32; f];
        target.extend(std::iter::repeat_n(0.1f32, f));
        target.extend(std::iter::repeat_n(0.0f32, f));
        let mut output = vec![0.05f32; f];
        output.extend(std::iter::repeat_n(0.0f32, f));
        output.extend(std::iter::repeat_n(0.9f32, f));
        let (t, o) = active_rms_pair(&target, &output);
        assert!((t - 0.1).abs() < 1e-5, "目标取同一批帧：{t}");
        assert!((o - 0.05).abs() < 1e-5, "输出只取两边都有声的帧：{o}");
    }

    #[test]
    fn limiter_is_transparent_below_the_threshold() {
        // 0.5 的方波比阈值低 5 dB：应当原封不动地过去（旧实现 0.65 起就压）。
        let mut signal: Vec<f32> = (0..4000)
            .map(|i| if i % 100 < 50 { 0.5 } else { -0.5 })
            .collect();
        let before = signal.clone();
        lookahead_limit(&mut signal);
        assert_eq!(signal, before);
    }

    #[test]
    fn limiter_caps_a_spike_without_touching_the_body() {
        let mut out = vec![0.2f32; 8000];
        out[4000] = 10.0;
        lookahead_limit(&mut out);
        let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak <= PEAK_CEILING + 1e-4, "尖峰必须被限幅：{peak}");
        assert!(
            (out[100] - 0.2).abs() < 1e-5,
            "远端本体不该被压：{}",
            out[100]
        );
    }

    #[test]
    fn limiter_scales_a_loud_sine_instead_of_saturating_it() {
        // 0.95 的正弦只比阈值响 0.55 dB：限幅器应整体缩一点（波形不变），
        // 而不是像 tanh 那样把持续的大音量压出谐波（峰均比会掉）。
        let sr = SAMPLE_RATE as f32;
        let mut signal: Vec<f32> = (0..4800)
            .map(|i| 0.95 * (std::f32::consts::TAU * 220.0 * i as f32 / sr).sin())
            .collect();
        lookahead_limit(&mut signal);
        let peak = signal.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak <= PEAK_CEILING + 1e-4, "峰值：{peak}");
        let crest = peak / rms(&signal);
        assert!(
            (crest - std::f32::consts::SQRT_2).abs() < 0.05,
            "波形不能变形（峰均比 {crest}）"
        );
    }

    #[test]
    fn dynamics_follow_the_target_instead_of_one_global_gain() {
        // 目标：前半 −20 dBFS、后半 −8 dBFS（12 dB 动态）。模型输出：前半小了 20 dB、
        // 后半基本对。旧做法乘一个标量只能二选一：抬前半就把后半推爆，压后半就让
        // 前半更小。逐帧对齐应当两段都对上，同时保住 12 dB 的相对动态。
        let n = LOUDNESS_FRAME * 8;
        let mut target = vec![0.1f32; n / 2];
        target.extend(std::iter::repeat_n(0.4f32, n / 2));
        let mut out = vec![0.01f32; n / 2];
        out.extend(std::iter::repeat_n(0.4f32, n / 2));
        match_dynamics(&target, &mut out);

        let quiet_t = rms(&target[..n / 2]);
        let quiet_o = rms(&out[..n / 2]);
        let loud_t = rms(&target[n / 2..]);
        let loud_o = rms(&out[n / 2..]);
        let quiet_db = 20.0 * (quiet_o / quiet_t).log10();
        let loud_db = 20.0 * (loud_o / loud_t).log10();
        assert!(
            quiet_db.abs() < 1.5,
            "小声段应被抬到目标电平：{quiet_db} dB"
        );
        assert!(loud_db.abs() < 1.5, "大声段不该被一起推爆：{loud_db} dB");
        let dynamics = 20.0 * (loud_o / quiet_o).log10();
        let want = 20.0 * (loud_t / quiet_t).log10();
        assert!(
            (dynamics - want).abs() < 1.5,
            "相对动态被改变了：{dynamics} dB vs {want} dB"
        );
    }

    #[test]
    fn a_single_spike_no_longer_squashes_the_whole_take() {
        // 目标是一段稳定有声素材；输出整体小 14 dB，另外有一个离谱的尖峰。
        // 旧实现按绝对峰值缩放，这个尖峰会把整轨压到几乎听不见。
        let target = vec![0.1f32; LOUDNESS_FRAME * 16];
        let mut out = vec![0.02f32; LOUDNESS_FRAME * 16];
        out[1000] = 10.0;
        match_dynamics(&target, &mut out);
        let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak <= PEAK_CEILING + 1e-4, "尖峰必须被限幅：{peak}");
        // 尖峰之后有 80 ms 释放（会短暂压低）；等释放走完，主体必须回到目标电平。
        let tail = rms(&out[LOUDNESS_FRAME * 15..]);
        assert!((tail - 0.1).abs() < 0.005, "主体应被抬到目标电平：{tail}");
    }

    #[test]
    fn seam_crossfade_removes_the_level_step() {
        // 前一段在接缝处是 0.2，后一段对同一时刻渲染出 0.4：直接硬拼是一个台阶
        // （听见「忽然变大」+ 咔哒）。交叉淡化之后，相邻采样差必须很小。
        let mut out = vec![0.2f32; 4000];
        let piece = vec![0.4f32; 4000];
        write_core(&mut out, 2000, &piece, 1000, 1000, 200);
        let step = out[2000..3000]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(step < 0.005, "接缝处不该有台阶：{step}");
        assert!(
            (out[2100] - 0.3).abs() < 0.01,
            "淡化的中点应是两段的平均：{}",
            out[2100]
        );
        assert!(
            (out[2999] - 0.4).abs() < 1e-6,
            "核心段内应是新渲染：{}",
            out[2999]
        );
    }

    #[test]
    fn input_normalization_lands_on_the_target_level() {
        // 同一段素材轻 34 dB / 响 14 dB，归一化后都应落回训练分布的电平附近。
        let sr = SAMPLE_RATE as f32;
        let f0 = vec![220.0f32; 20];
        let quiet: Vec<f32> = (0..9600)
            .map(|i| 0.01 * (std::f32::consts::TAU * 220.0 * i as f32 / sr).sin())
            .collect();
        let loud: Vec<f32> = quiet.iter().map(|v| *v * 50.0).collect();
        let lifted = normalize_input(&quiet, &f0);
        let tamed = normalize_input(&loud, &f0);
        let l = voiced_rms(&lifted, &f0);
        let t = voiced_rms(&tamed, &f0);
        assert!(
            (l / INPUT_LEVEL - 1.0).abs() < 0.15,
            "轻素材应被抬起来：{l}"
        );
        assert!(
            (t / INPUT_LEVEL - 1.0).abs() < 0.15,
            "响素材应被压下来：{t}"
        );
    }

    #[test]
    fn input_normalization_only_counts_voiced_frames() {
        // 前面 0.5 秒静音（f0 = 0）不该把电平估计拉低：有声段才是基准。
        let sr = SAMPLE_RATE as f32;
        let mut audio = vec![0.0f32; 4800];
        audio.extend(
            (0..4800).map(|i| 0.05 * (std::f32::consts::TAU * 220.0 * i as f32 / sr).sin()),
        );
        let f0: Vec<f32> = std::iter::repeat_n(0.0f32, 10)
            .chain(std::iter::repeat_n(220.0f32, 10))
            .collect();
        let normalized = normalize_input(&audio, &f0);
        let level = voiced_rms(&normalized, &f0);
        assert!(
            (level / INPUT_LEVEL - 1.0).abs() < 0.15,
            "有声段应落到目标电平：{level}"
        );
        assert!(
            normalized[..4800].iter().all(|v| *v == 0.0),
            "静音段仍是静音"
        );
    }
}
