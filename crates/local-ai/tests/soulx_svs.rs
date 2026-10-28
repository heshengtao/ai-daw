//! **歌声合成的端到端验证**（需要 1.4 GB 的 SoulX-Singer 权重，默认 `#[ignore]`）。
//!
//! ```bash
//! cargo test -p ai-daw-local-ai --test soulx_svs -- --ignored --nocapture
//! ```
//!
//! 它做三件事：渲染 4 个音、把结果写成 `/tmp/ai-daw-svs.wav`（人工试听）、
//! 检查时长 / 峰值 / RMS / 基频是否说得过去。没有权重时**跳过**而不是失败。

use ai_daw_local_ai::singing::SAMPLE_RATE;
use ai_daw_local_ai::singing::{assign_lyrics, synthesize, SingNote};

/// 极简 WAV 读取：只支持 16-bit PCM（测试素材就是这种）。
fn read_wav_16(path: &str) -> (Vec<f32>, f64) {
    let bytes = std::fs::read(path).expect("读不到参考音频");
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    let mut pos = 12usize;
    let mut channels = 1usize;
    let mut rate = 44_100f64;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = &bytes[pos + 8..(pos + 8 + size).min(bytes.len())];
        if id == b"fmt " {
            channels = u16::from_le_bytes(body[2..4].try_into().unwrap()) as usize;
            rate = u32::from_le_bytes(body[4..8].try_into().unwrap()) as f64;
        } else if id == b"data" {
            let samples: Vec<f32> = body
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
                .collect();
            let mono: Vec<f32> = if channels > 1 {
                samples
                    .chunks_exact(channels)
                    .map(|frame| frame.iter().sum::<f32>() / channels as f32)
                    .collect()
            } else {
                samples
            };
            return (mono, rate);
        }
        pos += 8 + size + (size & 1);
    }
    panic!("WAV 里没有 data 块");
}

fn write_wav(path: &str, samples: &[f32], rate: f64) {
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    let data_len = (samples.len() * 2) as u32;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&(rate as u32).to_le_bytes());
    out.extend_from_slice(&((rate as u32) * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, out).expect("写不出 WAV");
}

/// 自相关基频估计（只看低频，避开八度误判）。
fn estimate_f0(segment: &[f32], rate: f64) -> f64 {
    let mean = segment.iter().sum::<f32>() / segment.len() as f32;
    let x: Vec<f32> = segment.iter().map(|v| v - mean).collect();
    let lo = (rate / 700.0) as usize;
    let hi = ((rate / 80.0) as usize).min(x.len() / 2);
    let mut corr = vec![0.0f64; hi];
    for lag in lo..hi {
        let mut acc = 0.0f64;
        for i in 0..x.len() - lag {
            acc += f64::from(x[i] * x[i + lag]);
        }
        corr[lag] = acc;
    }
    let peak = corr[lo..hi].iter().cloned().fold(0.0f64, f64::max);
    if peak <= 0.0 {
        return 0.0;
    }
    // 取**第一个**足够高的局部极大值：直接取全局最大会在两倍周期处选中八度低音。
    for lag in (lo + 1)..hi - 1 {
        if corr[lag] > corr[lag - 1] && corr[lag] >= corr[lag + 1] && corr[lag] >= 0.72 * peak {
            return rate / lag as f64;
        }
    }
    rate / (corr
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .unwrap()
        .0 as f64)
}

#[test]
#[ignore = "需要 SoulX-Singer 权重（约 1.4 GB）"]
fn renders_four_notes_with_a_reference_timbre() {
    assert!(
        ai_daw_local_ai::singing::installed(),
        "模型没装：先跑 tools/fetch-models.sh 或到「管理器 → 模型」下载"
    );

    let reference_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../test-files/ace-step-10s-lyric-zh.wav"
    );
    let (reference, reference_rate) = read_wav_16(reference_path);

    let mut notes: Vec<SingNote> = [(0.0, 60), (1.0, 62), (2.0, 64), (3.0, 65)]
        .iter()
        .map(|(start, pitch)| SingNote {
            start: *start,
            duration: 1.0,
            pitch: *pitch,
            syllable: None,
            tone: None,
            slur: false,
        })
        .collect();
    let report = assign_lyrics(&mut notes, "小河流水");
    assert_eq!(report.assigned, 4);

    let take = synthesize(
        &notes,
        120.0,
        32,
        7,
        Some((&reference, reference_rate)),
        &|_| true,
    )
    .expect("合成失败");

    let out = "/tmp/ai-daw-svs.wav";
    write_wav(out, &take.samples, take.sample_rate);
    assert_eq!(take.sample_rate, SAMPLE_RATE);
    // 4 拍 @120 BPM = 2 秒。
    assert!(
        (take.seconds - 2.0).abs() < 0.05,
        "时长应该是 2 秒，实际 {}",
        take.seconds
    );
    assert!(
        take.samples.iter().all(|s| s.is_finite()),
        "输出里有 NaN/Inf"
    );
    let peak = take.samples.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(
        (peak - 0.891).abs() < 0.01,
        "峰值应归一到 -1 dBFS，实际 {peak}"
    );
    let rms = (take.samples.iter().map(|v| v * v).sum::<f32>() / take.samples.len() as f32).sqrt();
    assert!(rms > 0.02, "输出太安静（RMS {rms}），可能是静音");

    // 基频跟踪：第 1、2 个音分别是 C4(261.6) / D4(293.7)。
    let rate = take.sample_rate;
    for (index, expected) in [(0usize, 261.63f64), (1, 293.66)] {
        let start = (index as f64 * 0.5 * rate) as usize + (rate * 0.15) as usize;
        let window = &take.samples[start..(start + rate as usize / 4).min(take.samples.len())];
        let f0 = estimate_f0(window, rate);
        // 允许八度：模型的激励偶尔让二次谐波比基频更强（参考实现也这样），
        // 自相关/ YIN 都会把它当成 2×f0。音级对就够了。
        let ratio = f0 / expected;
        let octave_ok = [0.5f64, 1.0, 2.0]
            .iter()
            .any(|k| (ratio / k - 1.0).abs() < 0.06);
        assert!(
            octave_ok,
            "第 {} 个音的目标是 {expected:.1} Hz，估计 {f0:.1} Hz（比值 {ratio:.2}）",
            index + 1
        );
    }

    println!(
        "写好了 {out}：{} 秒、{} 段、{} 个音、RMS {rms:.3}",
        take.seconds, take.segments, take.notes
    );
}

/// 没有参考音色也能跑通（32 帧零 mel 当 prompt），但**音色会明显偏噪**——所以界面上
/// 参考声线是必填的，这条用例只保证底层不会崩、不会静音。
#[test]
#[ignore = "需要 SoulX-Singer 权重（约 1.4 GB）"]
fn renders_without_a_reference_voice_without_crashing() {
    assert!(ai_daw_local_ai::singing::installed());
    let mut notes: Vec<SingNote> = [(0.0, 60), (1.0, 62)]
        .iter()
        .map(|(start, pitch)| SingNote {
            start: *start,
            duration: 1.0,
            pitch: *pitch,
            syllable: None,
            tone: None,
            slur: false,
        })
        .collect();
    assign_lyrics(&mut notes, "小 河");
    let take = synthesize(&notes, 120.0, 16, 3, None, &|_| true).expect("合成失败");
    assert!(take.samples.iter().all(|s| s.is_finite()));
    let rms = (take.samples.iter().map(|v| v * v).sum::<f32>() / take.samples.len() as f32).sqrt();
    assert!(rms > 0.02, "输出太安静：RMS {rms}");
    write_wav("/tmp/ai-daw-svs-noref.wav", &take.samples, take.sample_rate);
}
