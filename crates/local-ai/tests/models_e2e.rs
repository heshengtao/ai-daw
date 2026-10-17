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

// ---------------------------------------------------------------------------
// Beat This!（节拍 / 速度检测）
// ---------------------------------------------------------------------------

/// 无状态伪噪声（同一个 `i` 恒定给出同一个值；测试可复现）。
fn noise(i: usize) -> f64 {
    let mut x = (i as u32).wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    x ^= x >> 15;
    x = x.wrapping_mul(2_246_822_519);
    x ^= x >> 13;
    (x as f64 / u32::MAX as f64) * 2.0 - 1.0
}

/// 把 `f(i)` 生成的一段包络叠进 `buf`。
fn add(buf: &mut [f64], start_sec: f64, dur_sec: f64, sr: f64, f: impl Fn(usize) -> f64) {
    let start = (start_sec * sr) as usize;
    let n = (dur_sec * sr) as usize;
    for i in 0..n {
        let idx = start + i;
        if idx >= buf.len() {
            break;
        }
        buf[idx] += f(i);
    }
}

/// 合成一段「死准的 120 BPM、4/4」循环：四踩底鼓 + 2/4 军鼓 + 八分闭镲（立体声，左右相同）。
///
/// 与 `tools/make-test-song.py` 是同一种配方：简单、无速度漂移、第 0 拍就是重拍。
fn four_on_the_floor(bpm: f64, bars: usize, sr: f64) -> Vec<f32> {
    let beat = 60.0 / bpm;
    let total = ((bars as f64 * 4.0 * beat + 0.4) * sr) as usize;
    let mut mono = vec![0.0f64; total];
    for b in 0..bars * 4 {
        let t = b as f64 * beat;
        // 底鼓：频率 ~135→50 Hz 的正弦 + 指数衰减。
        add(&mut mono, t, 0.25, sr, |i| {
            let ii = i as f64;
            let env = (-ii / (0.09 * sr)).exp();
            let f = 50.0 + 85.0 * (-ii / (0.018 * sr)).exp();
            env * (std::f64::consts::TAU * f * ii / sr).sin()
        });
        if b % 4 == 1 || b % 4 == 3 {
            // 军鼓：190 Hz 音体 + 白噪声。
            add(&mut mono, t, 0.2, sr, |i| {
                let ii = i as f64;
                let env = (-ii / (0.055 * sr)).exp();
                env * (0.35 * (std::f64::consts::TAU * 190.0 * ii / sr).sin() + 0.65 * noise(i)) * 0.6
            });
        }
        // 闭镲：一阶差分噪声 ≈ 高通。
        for hat_t in [t, t + beat / 2.0] {
            add(&mut mono, hat_t, 0.05, sr, |i| {
                let ii = i as f64;
                let env = (-ii / (0.012 * sr)).exp();
                env * (noise(i) - noise(i.saturating_sub(1))) * 0.4
            });
        }
    }
    let peak = mono.iter().fold(1e-9f64, |m, v| m.max(v.abs()));
    let scale = 0.89 / peak;
    let mut stereo = Vec::with_capacity(mono.len() * 2);
    for v in &mono {
        let s = (v * scale).clamp(-1.0, 1.0) as f32;
        stereo.push(s);
        stereo.push(s);
    }
    stereo
}

/// 读 16-bit PCM WAV（单 / 双声道）→ 交错 f32。测试够用就行。
fn read_wav16(path: &std::path::Path) -> Option<Vec<f32>> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" {
        return None;
    }
    let mut pos = 12;
    let mut channels = 0usize;
    let mut bits = 0u16;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = pos + 8;
        if id == b"fmt " && body + 16 <= bytes.len() {
            channels = u16::from_le_bytes(bytes[body + 2..body + 4].try_into().unwrap()) as usize;
            bits = u16::from_le_bytes(bytes[body + 14..body + 16].try_into().unwrap());
        } else if id == b"data" {
            if bits != 16 || channels == 0 {
                return None;
            }
            let n = (size / 2).min((bytes.len() - body) / 2);
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let s = i16::from_le_bytes(bytes[body + i * 2..body + i * 2 + 2].try_into().unwrap());
                out.push(s as f32 / 32768.0);
            }
            return Some(out);
        }
        pos = body + size + (size & 1);
    }
    None
}

#[test]
fn detects_120_bpm_four_four_on_a_simple_loop() {
    if models::beat_this::model_paths().is_none() {
        eprintln!("跳过：未找到 Beat This! 权重（mel 前端 + final0）");
        return;
    }
    // 1) 现合成一段 120 BPM 4/4 的简单循环，走真实链路（44.1k 立体声 → 下混 → 22.05k 重采样）。
    let sr = 44_100.0;
    let synth = four_on_the_floor(120.0, 8, sr);
    let det = models::beat_this::detect(&synth, 2, sr, &|_| true).expect("节拍检测应成功");
    assert!((det.bpm - 120.0).abs() < 2.0, "BPM 应接近 120：{}", det.bpm);
    assert_eq!((det.time_sig_num, det.time_sig_den), (4, 4), "应是 4/4");
    assert!(det.downbeats >= 6, "8 小节应至少检出 6 个重拍：{}", det.downbeats);
    let first = det.first_downbeat_sec.expect("应检出重拍");
    assert!(first < 0.15, "首重拍应几乎落在起点：{first}");

    // 2) 若 `tools/make-test-song.py` 生成的曲子也在，直接验它——用户拿的就是这个文件。
    let song =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-files/beat-test-120bpm.wav");
    match read_wav16(&song) {
        Some(audio) => {
            let det = models::beat_this::detect(&audio, 2, sr, &|_| true).expect("脚本曲子的检测应成功");
            assert!((det.bpm - 120.0).abs() < 2.0, "脚本曲子的 BPM 应接近 120：{}", det.bpm);
            assert_eq!((det.time_sig_num, det.time_sig_den), (4, 4), "脚本曲子应是 4/4");
        }
        None => eprintln!("（未找到 test-files/beat-test-120bpm.wav，跳过脚本曲子的校验）"),
    }
}
