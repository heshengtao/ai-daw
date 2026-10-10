//! 端到端：按宿主的真实用法调模型 —— **48 kHz 立体声**、带底噪、走完整重采样链路。
//!
//! 两个模型都需要下载权重（`tools/fetch-models.sh`）；没下载时这些测试直接跳过而不是失败。

use ai_daw_local_ai::models;
use ai_daw_local_ai::resample;

/// 48 kHz 立体声，左右相同：C 大三和弦 + 嘶声。
fn chord_with_hiss(seconds: f64, sr: f64) -> Vec<f32> {
    let n = (seconds * sr) as usize;
    let mut state = 0x2545_F491u32;
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f64 / sr;
        let tone: f64 = [261.63, 329.63, 392.0]
            .iter()
            .map(|hz| (std::f64::consts::TAU * hz * t).sin())
            .sum::<f64>()
            * 0.22;
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let hiss = ((state >> 9) as f32 / (1 << 23) as f32 - 1.0) * 0.05;
        let v = tone as f32 + hiss;
        out.push(v);
        out.push(v);
    }
    out
}

fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

#[test]
fn transcribes_a_chord_recorded_at_48k_stereo() {
    if models::basic_pitch::model_path().is_none() {
        eprintln!("跳过：未找到 Basic Pitch 权重");
        return;
    }
    let sr = 48_000.0;
    let audio = chord_with_hiss(1.5, sr);
    let notes = models::basic_pitch::transcribe(&audio, 2, sr, &|_| true).expect("转写应成功");
    let pitches: Vec<u8> = notes.iter().map(|n| n.pitch).collect();
    for want in [60u8, 64, 67] {
        assert!(pitches.contains(&want), "48k 立体声下应检测到 MIDI {want}：{pitches:?}");
    }
    assert!(notes.iter().all(|n| n.duration_sec > 0.0 && n.velocity > 0));
}

#[test]
fn enhance_keeps_length_and_lowers_the_noise_floor() {
    if models::localvqe::model_path().is_none() {
        eprintln!("跳过：未找到 LocalVQE 权重");
        return;
    }
    let sr = 48_000.0;
    let audio = chord_with_hiss(2.0, sr);
    let out = models::localvqe::enhance(&audio, 2, sr, &|_| true).expect("降噪应成功");
    assert_eq!(out.len(), audio.len(), "输出必须与输入同长度（时间轴不能变）");
    assert!(out.iter().all(|v| v.is_finite()), "输出不应出现 NaN/inf");
    // 静音帧仍然静音；整体能量应被压低（模型是语音降噪，对音乐也会压）。
    assert!(rms(&out) < rms(&audio), "降噪后总能量应更低：{} -> {}", rms(&audio), rms(&out));
}

#[test]
fn resampling_to_model_rates_round_trips_a_tone() {
    // 这条不依赖权重：确认 48k → 16k → 48k 不会把 440 Hz 弄丢（模型前的准备工作）。
    let sr = 48_000.0;
    let n = (sr * 0.5) as usize;
    let x: Vec<f32> = (0..n).map(|i| (std::f64::consts::TAU * 440.0 * i as f64 / sr).sin() as f32 * 0.5).collect();
    let down = resample::resample(&x, sr, 16_000.0);
    let back = resample::resample(&down, 16_000.0, sr);
    assert!((back.len() as i64 - n as i64).abs() <= 1);
    let a = rms(&x);
    let b = rms(&back);
    assert!((a - b).abs() < a * 0.05, "往返后整体能量应保持：{a} -> {b}");
}
