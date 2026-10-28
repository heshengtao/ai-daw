//! 歌声合成的手动试听工具（开发用）：
//!
//! ```bash
//! cargo run -p ai-daw-local-ai --example svs_demo -- \
//!   --ref "/path/to/voice.wav" --out /tmp/svs.wav --bpm 100 --steps 32
//! ```
//!
//! 参考音频可以是整首歌（会先自动挑一段最像人声的 10 秒），也可以是 3–10 秒干声。
//! 输出 24 kHz 单声道 WAV；同时打印选段位置与合成摘要。

use ai_daw_local_ai::singing::{assign_lyrics, reference, synthesize, SingNote};

fn read_wav16(path: &str) -> (Vec<f32>, f64) {
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
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
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

fn main() {
    let mut ref_path = String::new();
    let mut out_path = "/tmp/svs-demo.wav".to_string();
    let mut bpm = 100.0f64;
    let mut steps = 32u32;
    let mut lyrics = "月亮代表我的心".to_string();
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--ref" => {
                ref_path = args[i + 1].clone();
                i += 2;
            }
            "--out" => {
                out_path = args[i + 1].clone();
                i += 2;
            }
            "--bpm" => {
                bpm = args[i + 1].parse().unwrap_or(100.0);
                i += 2;
            }
            "--steps" => {
                steps = args[i + 1].parse().unwrap_or(32);
                i += 2;
            }
            "--lyrics" => {
                lyrics = args[i + 1].clone();
                i += 2;
            }
            other => panic!("未知参数：{other}"),
        }
    }
    if ref_path.is_empty() {
        panic!("需要 --ref <参考音频>");
    }
    assert!(
        ai_daw_local_ai::singing::installed(),
        "SoulX-Singer 权重不在 models/soulx-singer-fp16/"
    );

    let (mono, rate) = read_wav16(&ref_path);
    let resampled = ai_daw_local_ai::resample::resample(&mono, rate, 24_000.0);
    let choice = reference::best_window(&resampled, 24_000.0, 10.0);
    println!(
        "参考：{} 秒，自动选中 {:.2} 秒起的 {:.1} 秒（有声占比 {:.0}%）",
        mono.len() as f64 / rate,
        choice.start_sec,
        choice.seconds,
        choice.voiced_ratio * 100.0
    );

    // 一句 8 个音的旋律（C 大调），最后一个音连音。
    let pitches = [60u8, 62, 64, 65, 67, 65, 64, 60];
    let marks = ["-", "-", "-", "-", "-", "-", "-", "-"];
    let mut notes: Vec<SingNote> = pitches
        .iter()
        .enumerate()
        .map(|(i, pitch)| SingNote {
            start: i as f64 * 1.25,
            duration: 1.25,
            pitch: *pitch,
            syllable: None,
            tone: None,
            slur: false,
        })
        .collect();
    let _ = marks;
    let report = assign_lyrics(&mut notes, &lyrics);
    println!("歌词：{}", report.summary());

    let take = synthesize(
        &notes,
        bpm,
        steps,
        7,
        Some((&mono, rate)),
        &|_| true,
    )
    .expect("合成失败");
    write_wav(&out_path, &take.samples, take.sample_rate);
    println!(
        "写出 {out_path}：{:.1} 秒、{} 个音、{} 段；实际参考起点 {:?} 秒",
        take.seconds, take.notes, take.segments, take.reference_offset
    );
}
