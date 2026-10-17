//! 诊断用：对一个 **16-bit PCM WAV** 跑一遍 Beat This!，打印原始拍点 / 重拍 / BPM / 拍号估计。
//!
//! 用法（权重放 `AIDAW_MODELS` 指向的目录，或仓库根 `models/`）：
//!
//! ```text
//! AIDAW_MODELS=$PWD/models cargo run -p ai-daw-local-ai --example beat_probe -- song.wav
//! ```
//!
//! 只做 16-bit PCM（本仓库测试曲子的格式）；要读别的格式请先用 `ffmpeg` 转一下。

use std::path::Path;

use beat_this::{BeatThis, RtenRuntime};

fn read_wav16(path: &Path) -> Option<(Vec<f32>, usize, u32)> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" {
        return None;
    }
    let mut pos = 12;
    let mut channels = 0usize;
    let mut bits = 0u16;
    let mut rate = 0u32;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = pos + 8;
        if id == b"fmt " && body + 16 <= bytes.len() {
            channels = u16::from_le_bytes(bytes[body + 2..body + 4].try_into().unwrap()) as usize;
            rate = u32::from_le_bytes(bytes[body + 4..body + 8].try_into().unwrap());
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
            return Some((out, channels, rate));
        }
        pos = body + size + (size & 1);
    }
    None
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("用法：beat_probe <file.wav>");
        std::process::exit(2);
    };
    let (mel, beat) = ai_daw_local_ai::models::beat_this::model_paths()
        .expect("未找到 Beat This! 权重（mel 前端 + final0）");
    let (samples, channels, rate) = read_wav16(Path::new(&path)).expect("读不了这个 WAV（需 16-bit PCM）");
    // 下混单声道。
    let mono: Vec<f32> = if channels <= 1 {
        samples.clone()
    } else {
        (0..samples.len() / channels)
            .map(|i| samples[i * channels..(i + 1) * channels].iter().sum::<f32>() / channels as f32)
            .collect()
    };
    println!("素材：{path}  {} 声道  {} Hz  {:.1} s", channels, rate, mono.len() as f64 / rate as f64);

    let mut tracker = BeatThis::new(&RtenRuntime, &mel, &beat).expect("加载权重失败");
    let t = std::time::Instant::now();
    let timed = tracker.analyze_audio_timed(&mono, rate).expect("推理失败");
    println!("推理耗时：{:.1} s", t.elapsed().as_secs_f64());
    println!(
        "  分阶段：mel {:.1} s / predict {:.1} s / decode {:.3} s",
        timed.timing.mel.as_secs_f64(),
        timed.timing.predict.as_secs_f64(),
        timed.timing.decode.as_secs_f64(),
    );
    let analysis = timed.analysis;
    println!("拍点 {} 个，重拍 {} 个", analysis.beats.len(), analysis.downbeats.len());
    println!("BPM（crate 估计）：{:?}", beat_this::calculate_bpm(&analysis));
    println!("拍号估计（本仓库逻辑）：{}", ai_daw_local_ai::models::beat_this::time_signature_num(&analysis));
    println!("重拍时刻：{:?}", analysis.downbeats);
    println!("拍点时刻：{:?}", analysis.beats);
    // 相邻重拍之间夹了多少拍（= 小节拍数的直接观察）。
    let idx: Vec<usize> = analysis
        .downbeats
        .iter()
        .filter_map(|d| {
            let at = analysis.beats.partition_point(|b| *b < *d - 1e-3);
            analysis.beats.get(at).filter(|b| (**b - *d).abs() < 1e-3).map(|_| at)
        })
        .collect();
    let spans: Vec<usize> = idx.windows(2).map(|p| p[1] - p[0]).collect();
    println!("重拍间隔（拍）：{spans:?}");
}
