//! 单音音高检测：把一段单声源音频变成 MIDI 音符事件。
//!
//! 做法：按帧做 **FFT 自相关**（Wiener–Khinchin），取音高范围内归一化自相关的峰值，
//! 抛物线细化后换成 MIDI 号；把连续、音高相近的帧并成一个音符。适合「人声 / 单音乐器
//! 录音 → MIDI」这类素材；复音素材需要模型版实现（见 crate 文档）。

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::{hann, Progress};

/// 检测到的音符（时间以秒计，音高是 MIDI 号）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectedNote {
    /// MIDI 音高（0..=127）。
    pub pitch: u8,
    /// 起点（秒，相对素材开头）。
    pub start_sec: f64,
    /// 时长（秒）。
    pub duration_sec: f64,
    /// 力度（1..=127）。
    pub velocity: u8,
}

/// 分析帧长（约 46 ms @44.1 kHz）。
const FRAME: usize = 2048;
/// 帧移（75% 重叠）。
const HOP: usize = 512;
/// 音高搜索范围（Hz）。
const MIN_FREQ: f32 = 55.0;
const MAX_FREQ: f32 = 1000.0;
/// 归一化自相关达到这个值才算「有声」。
const VOICING: f32 = 0.30;
/// 帧 RMS 低于这个值视为静音。
const MIN_ENERGY: f32 = 1e-4;
/// 短于这个时长的音符丢弃（避免颤音碎成一堆）。
const MIN_NOTE_SEC: f64 = 0.06;

/// 把交错多声道音频转成音符事件。
///
/// `progress` 每若干帧调用一次；返回 `false` 即取消（返回已完成的部分）。
pub fn detect_notes(
    samples: &[f32],
    channels: usize,
    sample_rate: f64,
    progress: Progress,
) -> Vec<DetectedNote> {
    let mono = crate::to_mono(samples, channels);
    if mono.len() < FRAME || sample_rate <= 0.0 {
        return Vec::new();
    }
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FRAME);
    let ifft = planner.plan_fft_inverse(FRAME);
    let window = hann(FRAME);

    let frame_count = (mono.len() - FRAME) / HOP + 1;
    let mut notes: Vec<DetectedNote> = Vec::new();
    // 正在累积的一段：(pitch, 起始帧, 结束帧(不含), 峰值 RMS)
    let mut cur: Option<(u8, usize, usize, f32)> = None;

    for f in 0..frame_count {
        // 每 16 帧报一次进度，并检查取消。
        if f % 16 == 0 && !progress(f as f32 / frame_count as f32) {
            break;
        }
        let start = f * HOP;
        let frame = &mono[start..start + FRAME];
        let rms = (frame.iter().map(|x| x * x).sum::<f32>() / FRAME as f32).sqrt();
        let pitch = if rms < MIN_ENERGY {
            None
        } else {
            estimate_pitch(frame, &window, fft.as_ref(), ifft.as_ref(), sample_rate)
        };

        match (cur, pitch) {
            (Some((p, s, _e, peak)), Some(np)) if near_semitone(p, np) => {
                cur = Some((p, s, f + 1, peak.max(rms)));
            }
            (Some((p, s, e, peak)), next) => {
                push_note(&mut notes, p, s, e, peak, sample_rate);
                cur = next.map(|np| (np, f, f + 1, rms));
            }
            (None, Some(np)) => cur = Some((np, f, f + 1, rms)),
            (None, None) => {}
        }
    }
    if let Some((p, s, e, peak)) = cur {
        push_note(&mut notes, p, s, e, peak, sample_rate);
    }
    notes
}

/// 两个 MIDI 号是否算「同一个音」（同一个半音内）。
fn near_semitone(a: u8, b: u8) -> bool {
    a == b
}

fn push_note(
    notes: &mut Vec<DetectedNote>,
    pitch: u8,
    start_frame: usize,
    end_frame: usize,
    peak_rms: f32,
    sample_rate: f64,
) {
    let start_sec = start_frame as f64 * HOP as f64 / sample_rate;
    let duration_sec = (end_frame - start_frame) as f64 * HOP as f64 / sample_rate;
    if duration_sec < MIN_NOTE_SEC {
        return;
    }
    let velocity = ((peak_rms * 2.0).min(1.0) * 127.0).round().clamp(1.0, 127.0) as u8;
    notes.push(DetectedNote { pitch, start_sec, duration_sec, velocity });
}

/// 单帧基频 → MIDI 号；无声或不可信时 `None`。
fn estimate_pitch(
    frame: &[f32],
    window: &[f32],
    fft: &dyn Fft<f32>,
    ifft: &dyn Fft<f32>,
    sample_rate: f64,
) -> Option<u8> {
    let mut buf: Vec<Complex<f32>> =
        frame.iter().zip(window).map(|(x, w)| Complex::new(x * w, 0.0)).collect();
    fft.process(&mut buf);
    // 功率谱 → 逆变换 = 自相关。
    for c in buf.iter_mut() {
        *c = Complex::new(c.norm_sqr(), 0.0);
    }
    ifft.process(&mut buf);
    let r0 = buf[0].re;
    if r0 <= 0.0 {
        return None;
    }

    let min_lag = ((sample_rate / MAX_FREQ as f64).floor() as usize).max(1);
    let max_lag = ((sample_rate / MIN_FREQ as f64).ceil() as usize).min(FRAME - 2);
    if min_lag >= max_lag {
        return None;
    }
    let mut best_lag = min_lag;
    let mut best = 0.0f32;
    for lag in min_lag..=max_lag {
        let v = buf[lag].re / r0;
        if v > best {
            best = v;
            best_lag = lag;
        }
    }
    if best < VOICING {
        return None;
    }
    // 抛物线细化（用峰值两侧的样本）。
    let refined = if best_lag > min_lag && best_lag < max_lag {
        let y0 = buf[best_lag - 1].re / r0;
        let y1 = best;
        let y2 = buf[best_lag + 1].re / r0;
        let denom = y0 - 2.0 * y1 + y2;
        if denom.abs() > 1e-9 {
            best_lag as f32 + 0.5 * (y0 - y2) / denom
        } else {
            best_lag as f32
        }
    } else {
        best_lag as f32
    };
    let freq = sample_rate as f32 / refined;
    if !(MIN_FREQ..=MAX_FREQ).contains(&freq) {
        return None;
    }
    let midi = 69.0 + 12.0 * (freq / 440.0).log2();
    Some(midi.round().clamp(0.0, 127.0) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, seconds: f64, sr: f64) -> Vec<f32> {
        let n = (seconds * sr) as usize;
        (0..n)
            .map(|i| (std::f64::consts::TAU * freq * i as f64 / sr).sin() as f32 * 0.6)
            .collect()
    }

    #[test]
    fn detects_a_single_sustained_tone() {
        let sr = 44_100.0;
        let audio = sine(440.0, 1.0, sr);
        let notes = detect_notes(&audio, 1, sr, &|_| true);
        assert_eq!(notes.len(), 1, "一段稳定正弦应得到一条音符：{notes:?}");
        assert_eq!(notes[0].pitch, 69, "440 Hz = A4 = MIDI 69");
        assert!(notes[0].duration_sec > 0.8, "时长应接近 1 秒：{}", notes[0].duration_sec);
        assert!(notes[0].velocity > 0);
    }

    #[test]
    fn detects_two_separated_tones_as_two_notes() {
        let sr = 22_050.0;
        let mut audio = sine(220.0, 0.4, sr); // A3 = 57
        audio.extend(std::iter::repeat(0.0).take((0.2 * sr) as usize));
        audio.extend(sine(660.0, 0.4, sr)); // E5 = 76
        let notes = detect_notes(&audio, 1, sr, &|_| true);
        let pitches: Vec<u8> = notes.iter().map(|n| n.pitch).collect();
        assert_eq!(pitches, vec![57, 76], "两个音应各自成一条音符：{notes:?}");
    }

    #[test]
    fn silence_yields_no_notes() {
        let sr = 44_100.0;
        let audio = vec![0.0f32; 44_100];
        assert!(detect_notes(&audio, 1, sr, &|_| true).is_empty());
    }

    #[test]
    fn cancellation_stops_early() {
        let sr = 44_100.0;
        let audio = sine(440.0, 2.0, sr);
        // 第一次回调就取消：应立刻停下（不会有完整音符）。
        let notes = detect_notes(&audio, 1, sr, &|_| false);
        assert!(notes.is_empty(), "取消后不应产出音符");
    }

    #[test]
    fn mono_downmix_averages_channels() {
        // 左右分别为 1.0 / 0.0 → 单声道 0.5。
        let stereo = [1.0f32, 0.0, 1.0, 0.0];
        let mono = crate::to_mono(&stereo, 2);
        assert_eq!(mono, vec![0.5, 0.5]);
    }
}
