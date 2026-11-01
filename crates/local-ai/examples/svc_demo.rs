//! 音色替换的手动试听工具（开发用）：
//!
//! ```bash
//! AIDAW_MODELS=/path/to/models cargo run -p ai-daw-local-ai --example svc_demo -- \
//!   --target target.wav --ref voice.wav --out /tmp/svc.wav --steps 32 --shift 0
//! ```
//!
//! 两个输入都读 16-bit PCM WAV（会自动重采样到 24 kHz 单声道）。目标应该是单声部人声；
//! 参考可以是整首歌（会先挑一段最像人声的 10 秒），也可以是 3–10 秒干声。
//! 权重放在 `$AIDAW_MODELS/soulx-svc-fp16hq/`（或 `soulx-svc-fp32/`）。

use ai_daw_local_ai::svc;

fn read_wav16(path: &str) -> (Vec<f32>, f64) {
    let bytes = std::fs::read(path).expect("读不到音频");
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
    let mut target_path = String::new();
    let mut ref_path = String::new();
    let mut out_path = "/tmp/svc-demo.wav".to_string();
    let mut steps = 32u32;
    let mut shift = 0i32;
    let mut seed = 0u64;
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--target" => {
                target_path = args[i + 1].clone();
                i += 2;
            }
            "--ref" => {
                ref_path = args[i + 1].clone();
                i += 2;
            }
            "--out" => {
                out_path = args[i + 1].clone();
                i += 2;
            }
            "--steps" => {
                steps = args[i + 1].parse().unwrap_or(32);
                i += 2;
            }
            "--shift" => {
                shift = args[i + 1].parse().unwrap_or(0);
                i += 2;
            }
            "--seed" => {
                seed = args[i + 1].parse().unwrap_or(0);
                i += 2;
            }
            other => panic!("未知参数：{other}"),
        }
    }
    if target_path.is_empty() || ref_path.is_empty() {
        panic!("需要 --target <目标人声> 与 --ref <参考音色>");
    }
    assert!(
        svc::installed(),
        "SoulX-Singer-SVC 权重不在 $AIDAW_MODELS/soulx-svc-fp16hq/"
    );

    let (target, target_rate) = read_wav16(&target_path);
    let (reference, reference_rate) = read_wav16(&ref_path);
    let target = ai_daw_local_ai::resample::resample(&target, target_rate, 24_000.0);
    println!(
        "目标：{:.1} 秒；参考：{:.1} 秒",
        target.len() as f64 / 24_000.0,
        reference.len() as f64 / reference_rate
    );

    let take = svc::convert(
        &target,
        Some((&reference, reference_rate)),
        steps,
        seed,
        shift,
        None,
        &|_| true,
    )
    .expect("替换失败");
    write_wav(&out_path, &take.samples, take.sample_rate);
    println!(
        "写出 {out_path}：{:.1} 秒、{} 段、{}、移调 {:+}、有声占比 {:.0}%、参考起点 {:?} 秒",
        take.seconds,
        take.segments,
        take.precision.label(),
        take.pitch_shift,
        take.voiced_ratio * 100.0,
        take.reference_offset,
    );
}
