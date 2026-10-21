//! **Basic Pitch**（Spotify, ICASSP 2022）：多音音高转录，音频 → MIDI 音符。
//!
//! 权重是官方 `nmp.onnx`（TF SavedModel 的 ONNX 导出，Apache-2.0）。图里已经包含了谐波 CQT
//! 前端，所以喂进去的是**原始波形**：输入 `[batch, 43844, 1]`，输出三个头 ——
//! `StatefulPartitionedCall:1` = note（172×88）、`:2` = onset（172×88）、`:0` = contour（172×264）。
//!
//! 这里把官方 `inference.py` / `note_creation.py` 的窗口化与音符解码**逐行照搬**（含
//! 半重叠补零、每窗掐掉首尾 15 帧、由帧差补 onset、邻频能量抑制、以及 melodia 后处理），
//! 只省略了 pitch bend（转录结果暂不写音符的 `expr`）。参数与官方默认值一致：
//! onset 0.5 / frame 0.3 / 最短音符 127.7 ms / 频率范围不限。

use std::path::PathBuf;

use ort::value::Tensor;

use super::err;
use crate::pitch::DetectedNote;
use crate::resample;
use crate::Progress;

/// 模型要求的采样率。
const SR: f64 = 22_050.0;
/// 输入窗口长度（样本）。
const N_SAMPLES: usize = 43_844;
/// 模型内部帧移。
const FFT_HOP: usize = 256;
/// 每个窗口的输出帧数（模型固定输出 172 帧）。
const ANNOT_N_FRAMES: usize = 172;
/// 相邻窗口重叠的帧数（官方默认 30）。
const OVERLAP_FRAMES: usize = 30;
/// 窗口之间的样本步进。
const HOP: usize = N_SAMPLES - OVERLAP_FRAMES * FFT_HOP;
/// 音高 bin 数。
const N_SEMITONES: usize = 88;
/// 最低 bin 对应的 MIDI 号。
const MIDI_OFFSET: usize = 21;
/// 送进模型的最大窗口数（一批）。
const BATCH: usize = 16;

// 官方默认阈值。
const ONSET_THRESH: f32 = 0.5;
const FRAME_THRESH: f32 = 0.3;
/// `int(round(127.7ms * fps))`。
const MIN_NOTE_LEN: usize = 11;
const ENERGY_TOL: usize = 11;
const MAX_FREQ_IDX: usize = 87;
/// `2.0 * ANNOTATIONS_FPS - n_overlapping_frames`（浮点，官方就是浮点参与截断）。
const FRAMES_PER_WINDOW: f64 = 2.0 * (SR / FFT_HOP as f64) - OVERLAP_FRAMES as f64;
const MAGIC_ALIGNMENT_OFFSET: f64 = 0.0018;

const IN_NOTE: &str = "StatefulPartitionedCall:1";
const IN_ONSET: &str = "StatefulPartitionedCall:2";
const MODEL_INPUT: &str = "serving_default_input_2:0";

/// 模型文件路径（没下载时为 `None`）。
pub fn model_path() -> Option<PathBuf> {
    super::find(super::BASIC_PITCH_FILE)
}

/// 把交错多声道音频转成音符事件（时间相对素材开头，单位秒）。
///
/// 内部会下混成单声道并重采样到 22 050 Hz。`progress` 返回 `false` 即取消，返回已完成的部分。
pub fn transcribe(
    samples: &[f32],
    channels: usize,
    sample_rate: f64,
    progress: Progress,
) -> Result<Vec<DetectedNote>, String> {
    let path = model_path().ok_or_else(|| format!("未找到 Basic Pitch 权重（{}）", super::BASIC_PITCH_FILE))?;
    let mut session = crate::backend::session::build(&path, crate::backend::session::GraphRole::BasicPitch)?;

    let mono = crate::to_mono(samples, channels);
    if mono.is_empty() || sample_rate <= 0.0 {
        return Ok(Vec::new());
    }
    let audio = resample::resample(&mono, sample_rate, SR);
    if audio.len() < FFT_HOP {
        return Ok(Vec::new());
    }
    if !progress(0.02) {
        return Ok(Vec::new());
    }

    // 前面补 `overlap/2` 个零帧，然后按 HOP 取窗（末尾不足一窗的补零）。
    let mut padded = vec![0.0f32; OVERLAP_FRAMES * FFT_HOP / 2];
    padded.extend_from_slice(&audio);
    let mut starts: Vec<usize> = Vec::new();
    let mut i = 0usize;
    while i < padded.len() {
        starts.push(i);
        i += HOP;
    }

    let n_frames = ((audio.len() as f64 / HOP as f64) * FRAMES_PER_WINDOW) as usize;
    let mut notes_mat = vec![0.0f32; n_frames * N_SEMITONES];
    let mut onsets_mat = vec![0.0f32; n_frames * N_SEMITONES];
    let trim = OVERLAP_FRAMES / 2;
    let mut written = 0usize;

    let windows = starts.len();
    for b in (0..windows).step_by(BATCH) {
        if written >= n_frames {
            break;
        }
        // 0.02..0.60 是推理阶段。
        if !progress(0.02 + 0.58 * b as f32 / windows.max(1) as f32) {
            break;
        }
        let end = (b + BATCH).min(windows);
        let batch = end - b;
        let mut flat = vec![0.0f32; batch * N_SAMPLES];
        for (k, w) in (b..end).enumerate() {
            let s = starts[w];
            let e = (s + N_SAMPLES).min(padded.len());
            flat[k * N_SAMPLES..k * N_SAMPLES + (e - s)].copy_from_slice(&padded[s..e]);
        }
        let input = Tensor::from_array(([batch, N_SAMPLES, 1], flat))
            .map_err(|e| err("构造模型输入失败", e))?;
        let outputs = session
            .run(ort::inputs![MODEL_INPUT => input])
            .map_err(|e| err("Basic Pitch 推理失败", e))?;
        let (_, note) = outputs[IN_NOTE]
            .try_extract_tensor::<f32>()
            .map_err(|e| err("读取 note 输出失败", e))?;
        let (_, onset) = outputs[IN_ONSET]
            .try_extract_tensor::<f32>()
            .map_err(|e| err("读取 onset 输出失败", e))?;

        // 掐掉每个窗口首尾各 15 帧，其余按时间顺序拼成一条帧序列。
        'windows: for k in 0..batch {
            for f in trim..ANNOT_N_FRAMES - trim {
                if written >= n_frames {
                    break 'windows;
                }
                let src = (k * ANNOT_N_FRAMES + f) * N_SEMITONES;
                let dst = written * N_SEMITONES;
                for c in 0..N_SEMITONES {
                    notes_mat[dst + c] = note[src + c];
                    onsets_mat[dst + c] = onset[src + c];
                }
                written += 1;
            }
        }
    }

    let frames = written.min(n_frames);
    if frames == 0 {
        return Ok(Vec::new());
    }
    let events = decode(
        &notes_mat[..frames * N_SEMITONES],
        &onsets_mat[..frames * N_SEMITONES],
        frames,
        progress,
    );
    Ok(events
        .into_iter()
        .map(|e| {
            let start_sec = frames_to_time(e.start);
            let end_sec = frames_to_time(e.end);
            DetectedNote {
                pitch: (e.pitch as i32).clamp(0, 127) as u8,
                start_sec,
                duration_sec: (end_sec - start_sec).max(0.0),
                velocity: (e.amplitude * 127.0).round().clamp(1.0, 127.0) as u8,
            }
        })
        .collect())
}

/// 第 `idx` 帧对应的时间（秒）。官方 `model_frames_to_time`：每个窗口再减掉一个固定偏移。
fn frames_to_time(idx: usize) -> f64 {
    let t = idx as f64 * FFT_HOP as f64 / SR;
    let window = (idx as f64 / ANNOT_N_FRAMES as f64).floor();
    let offset = (FFT_HOP as f64 / SR) * (ANNOT_N_FRAMES as f64 - (N_SAMPLES as f64 / FFT_HOP as f64))
        + MAGIC_ALIGNMENT_OFFSET;
    t - offset * window
}

/// 一个音符事件（帧为单位）。
#[derive(Debug, Clone, Copy, PartialEq)]
struct NoteEvent {
    start: usize,
    end: usize,
    pitch: usize,
    amplitude: f32,
}

/// 帧激活 + onset 激活 → 音符事件（官方 `output_to_notes_polyphonic` 的移植）。
fn decode(frames: &[f32], onsets: &[f32], n_frames: usize, progress: Progress) -> Vec<NoteEvent> {
    let bins = N_SEMITONES;
    let onsets = infer_onsets(onsets, frames, n_frames, bins);
    let peaks = local_maxima(&onsets, n_frames, bins);
    let mut remaining = frames.to_vec();

    // 候选 onset：取局部极大且过阈值的点，**按时间倒序**（官方用 np.where 后整体反转：
    // 时间降序，同一帧内频率降序），顺序会影响能量抑制的结果。
    let mut seeds: Vec<(usize, usize)> = Vec::new();
    for t in 0..n_frames {
        for f in 0..bins {
            if peaks[t * bins + f] && onsets[t * bins + f] >= ONSET_THRESH {
                seeds.push((t, f));
            }
        }
    }
    seeds.reverse();

    let mut out: Vec<NoteEvent> = Vec::new();
    // 第一遍：以 onset 为起点向后找能量落点。
    for (t0, f) in seeds {
        if t0 + 1 >= n_frames {
            continue;
        }
        let mut i = t0 + 1;
        let mut k = 0usize;
        while i < n_frames - 1 && k < ENERGY_TOL {
            if remaining[i * bins + f] < FRAME_THRESH {
                k += 1;
            } else {
                k = 0;
            }
            i += 1;
        }
        i -= k;
        if i <= t0 + MIN_NOTE_LEN {
            continue;
        }
        suppress(&mut remaining, n_frames, bins, t0, i, f);
        out.push(NoteEvent {
            start: t0,
            end: i,
            pitch: f + MIDI_OFFSET,
            amplitude: column_mean(frames, n_frames, bins, t0, i, f),
        });
    }

    // 第二遍：melodia 后处理——反复取剩余能量最大的点，向两侧扩展。
    // 维护「每行的最大值」以免每次迭代都全量扫描（长素材下快一个数量级）。
    let mut row_peaks: Vec<f32> = (0..n_frames).map(|t| row_max(&remaining, t, bins)).collect();
    let mut guard = 0usize;
    let max_iters = n_frames * bins + 1;
    loop {
        guard += 1;
        if guard > max_iters {
            break;
        }
        if guard % 64 == 0 && !progress(0.80 + 0.20 * (guard as f32 / max_iters as f32).min(1.0)) {
            break;
        }
        let (mx, t_mid) = match global_max(&row_peaks) {
            Some(v) => v,
            None => break,
        };
        if !(mx > FRAME_THRESH) {
            break;
        }
        let Some(f) = (0..bins).find(|f| remaining[t_mid * bins + f] >= mx) else {
            break;
        };
        remaining[t_mid * bins + f] = 0.0;
        row_peaks[t_mid] = row_max(&remaining, t_mid, bins);

        // 前向。
        let mut i = t_mid + 1;
        let mut k = 0usize;
        while i < n_frames - 1 && k < ENERGY_TOL {
            if remaining[i * bins + f] < FRAME_THRESH {
                k += 1;
            } else {
                k = 0;
            }
            clear_cross(&mut remaining, n_frames, bins, i, f);
            row_peaks[i] = row_max(&remaining, i, bins);
            i += 1;
        }
        let i_end = i.saturating_sub(1).saturating_sub(k);

        // 后向。
        let mut i = t_mid as i64 - 1;
        let mut k = 0i64;
        while i > 0 && k < ENERGY_TOL as i64 {
            let row = i as usize;
            if remaining[row * bins + f] < FRAME_THRESH {
                k += 1;
            } else {
                k = 0;
            }
            clear_cross(&mut remaining, n_frames, bins, row, f);
            row_peaks[row] = row_max(&remaining, row, bins);
            i -= 1;
        }
        let i_start = ((i + 1 + k).max(0)) as usize;

        if i_end <= i_start || i_end - i_start <= MIN_NOTE_LEN {
            continue;
        }
        out.push(NoteEvent {
            start: i_start,
            end: i_end,
            pitch: f + MIDI_OFFSET,
            amplitude: column_mean(frames, n_frames, bins, i_start, i_end, f),
        });
    }
    out.sort_by(|a, b| (a.start, a.pitch).cmp(&(b.start, b.pitch)));
    out
}

/// 把 `[start, end)` 行内 `f` 与相邻两列清零（官方抑制邻频）。
fn suppress(buf: &mut [f32], n_frames: usize, bins: usize, start: usize, end: usize, f: usize) {
    for t in start..end.min(n_frames) {
        clear_cross(buf, n_frames, bins, t, f);
    }
}

/// 单行里 `f` 与相邻两列清零。
fn clear_cross(buf: &mut [f32], n_frames: usize, bins: usize, t: usize, f: usize) {
    if t >= n_frames {
        return;
    }
    let base = t * bins;
    buf[base + f] = 0.0;
    if f < MAX_FREQ_IDX {
        buf[base + f + 1] = 0.0;
    }
    if f > 0 {
        buf[base + f - 1] = 0.0;
    }
}

fn column_mean(frames: &[f32], n_frames: usize, bins: usize, start: usize, end: usize, f: usize) -> f32 {
    let end = end.min(n_frames);
    if start >= end {
        return 0.0;
    }
    let mut sum = 0.0f32;
    for t in start..end {
        sum += frames[t * bins + f];
    }
    sum / (end - start) as f32
}

fn row_max(buf: &[f32], t: usize, bins: usize) -> f32 {
    buf[t * bins..(t + 1) * bins].iter().copied().fold(f32::NEG_INFINITY, f32::max)
}

fn global_max(row_max: &[f32]) -> Option<(f32, usize)> {
    let mut best: Option<(f32, usize)> = None;
    for (t, v) in row_max.iter().enumerate() {
        if v.is_finite() && best.map(|(b, _)| *v > b).unwrap_or(true) {
            best = Some((*v, t));
        }
    }
    best
}

/// 官方 `get_infered_onsets`：把帧激活的「上升沿」也算成 onset。
fn infer_onsets(onsets: &[f32], frames: &[f32], n_frames: usize, bins: usize) -> Vec<f32> {
    let n_diff = 2usize;
    if n_frames <= n_diff {
        return onsets.to_vec();
    }
    let mut diff = vec![0.0f32; n_frames * bins];
    // diff = min over n in 1..=n_diff of (frames_appended[n:] - frames_appended[:-n])
    for n in 1..=n_diff {
        for t in 0..n_frames {
            let cur = frames[t * bins..(t + 1) * bins].as_ref();
            let prev = if t >= n { Some(&frames[(t - n) * bins..(t - n + 1) * bins]) } else { None };
            for f in 0..bins {
                let d = cur[f] - prev.map(|p| p[f]).unwrap_or(0.0);
                let v = if n == 1 { d } else { d.min(diff[t * bins + f]) };
                diff[t * bins + f] = v;
            }
        }
    }
    for v in diff.iter_mut() {
        if *v < 0.0 {
            *v = 0.0;
        }
    }
    for t in 0..n_diff.min(n_frames) {
        for f in 0..bins {
            diff[t * bins + f] = 0.0;
        }
    }
    let max_onset = onsets.iter().copied().fold(0.0f32, f32::max);
    let max_diff = diff.iter().copied().fold(0.0f32, f32::max);
    let mut out = onsets.to_vec();
    if max_diff > 0.0 {
        let scale = max_onset / max_diff;
        for i in 0..out.len() {
            out[i] = out[i].max(diff[i] * scale);
        }
    }
    out
}

/// 沿时间轴的局部极大（等价 `scipy.signal.argrelmax(..., axis=0, order=1, mode="clip")`）。
///
/// `clip` 模式下首尾样本会和自己比较，因此**第一帧与最后一帧永远不是峰值**。
fn local_maxima(buf: &[f32], n_frames: usize, bins: usize) -> Vec<bool> {
    let mut peaks = vec![false; n_frames * bins];
    if n_frames < 3 {
        return peaks;
    }
    for t in 1..n_frames - 1 {
        for f in 0..bins {
            let v = buf[t * bins + f];
            if v > buf[(t - 1) * bins + f] && v > buf[(t + 1) * bins + f] {
                peaks[t * bins + f] = true;
            }
        }
    }
    peaks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_to_time_matches_the_reference_shape() {
        // 第 0 帧是 0；时间随帧号单调增（每帧 ≈ FFT_HOP/SR − 每窗口修正）。
        assert_eq!(frames_to_time(0), 0.0);
        let mut prev = -1.0;
        for t in 0..400 {
            let v = frames_to_time(t);
            assert!(v > prev, "时间应单调递增：t={t} {v} <= {prev}");
            prev = v;
        }
    }

    #[test]
    fn local_maxima_never_picks_the_edges() {
        // 一个真正的单峰：t=2 比两侧都高。
        let bins = 2usize;
        let n = 5usize;
        let values = [0.0f32, 1.0, 3.0, 1.0, 0.0];
        let mut buf = vec![0.0f32; n * bins];
        for t in 0..n {
            buf[t * bins] = values[t];
        }
        let peaks = local_maxima(&buf, n, bins);
        assert!(!peaks[0], "第一帧不可能是峰（clip 模式会和自己比较）");
        assert!(!peaks[(n - 1) * bins], "最后一帧不可能是峰");
        assert!(peaks[2 * bins], "单峰应被找到");
        assert_eq!(peaks.iter().filter(|p| **p).count(), 1);
    }

    #[test]
    fn infer_onsets_zeroes_the_first_frames_and_rescales() {
        let bins = 2usize;
        let n = 6usize;
        // 帧激活：在 t=3 处跳起来。
        let mut frames = vec![0.0f32; n * bins];
        for t in 3..n {
            frames[t * bins] = 0.5;
        }
        // 已有一个 onset 作为缩放基准（官方用 max(onsets) 归一化差分）。
        let mut onsets = vec![0.0f32; n * bins];
        onsets[5 * bins + 1] = 0.4;
        let out = infer_onsets(&onsets, &frames, n, bins);
        for t in 0..2 {
            assert_eq!(out[t * bins], 0.0, "前 n_diff 帧的差分要清零");
        }
        assert!(out[3 * bins] > 0.0, "上升沿应补出 onset：{}", out[3 * bins]);
        assert_eq!(out[5 * bins + 1], 0.4, "原有 onset 应保留");
    }

    /// 端到端：只有拿到权重才跑（没下载就跳过）。
    #[test]
    fn transcribes_a_synthetic_tone_when_the_model_is_present() {
        if model_path().is_none() {
            eprintln!("跳过：未找到 {}（先跑 tools/fetch-models.sh）", super::super::BASIC_PITCH_FILE);
            return;
        }
        let sr = 22_050.0;
        let audio: Vec<f32> = (0..(sr * 1.0) as usize)
            .map(|i| 0.7 * (std::f64::consts::TAU * 440.0 * i as f64 / sr).sin() as f32)
            .collect();
        let notes = transcribe(&audio, 1, sr, &|_| true).expect("转写应成功");
        assert!(!notes.is_empty(), "440 Hz 正弦应至少得到一个音符");
        assert!(
            notes.iter().any(|n| n.pitch == 69),
            "应检测到 A4(69)：{:?}",
            notes.iter().map(|n| n.pitch).collect::<Vec<_>>()
        );
    }

    /// 复音：C 大三和弦应还原出 60 / 64 / 67。
    #[test]
    fn transcribes_a_polyphonic_chord_when_the_model_is_present() {
        if model_path().is_none() {
            return;
        }
        let sr = 22_050.0;
        let n = (sr * 1.5) as usize;
        let audio: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f64 / sr;
                let f = |hz: f64| (std::f64::consts::TAU * hz * t).sin();
                0.25 * (f(261.63) + f(329.63) + f(392.0)) as f32
            })
            .collect();
        let notes = transcribe(&audio, 1, sr, &|_| true).expect("转写应成功");
        let pitches: Vec<u8> = notes.iter().map(|n| n.pitch).collect();
        for want in [60u8, 64, 67] {
            assert!(pitches.contains(&want), "缺少 MIDI {want}：{pitches:?}");
        }
    }
}
