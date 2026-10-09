//! 离线渲染一个预设参数集，写成 16-bit WAV。
//!
//! 用途：给预设做**客观测量**（频谱重心、起振时间、衰减、混叠量），而不是靠耳朵猜。
//! 用法：`cargo run -p daw-party-synth --example render_preset -- <presets.json> <outdir>`
//!
//! presets.json 形如 `[{ "id": "...", "params": { "cutoff": 0.7, ... } }, ...]`，
//! 参数是**完整**的（调用方负责把补丁摊平）。

use daw_party_plugin_api::{MidiEvent, ParamId, Plugin, ProcessContext, TimedMidiEvent};
use daw_party_synth::SynthPlugin;
use std::io::Write;

const SR: f64 = 48_000.0;
const BLOCK: usize = 256;
/// 按住 1.5 秒，松开后再录 2.0 秒（够听到释音/衰减尾巴）。
const HOLD_SECS: f64 = 1.5;
const TAIL_SECS: f64 = 2.0;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(input), Some(outdir)) = (args.get(1), args.get(2)) else {
        eprintln!("用法: render_preset <presets.json> <outdir> [notes 逗号分隔, 默认 60]");
        std::process::exit(2);
    };
    // 音高可选：混叠与音高强相关，高音区才是「刺耳」的现场，所以几个八度都录一遍。
    let notes: Vec<u8> = args
        .get(3)
        .map(|s| s.split(',').filter_map(|n| n.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![60]);
    let json = std::fs::read_to_string(input).expect("读不到 presets.json");
    let presets = parse_presets(&json);
    std::fs::create_dir_all(outdir).expect("建不了输出目录");

    for preset in &presets {
        for note in &notes {
            let samples = render(preset, *note);
            let path = format!("{outdir}/{}@{}.wav", preset.id, note);
            write_wav(&path, &samples);
        }
        println!("{}: 峰值 {:.3}", preset.id, peak(&render(preset, notes[0])));
    }
    println!("共 {} 条 × {} 个音高", presets.len(), notes.len());
}

struct Preset {
    id: String,
    params: Vec<(String, f32)>,
}

/// 手写一个够用的 JSON 解析：只需要 `[{ "id": "...", "params": { "k": v, ... } }]`。
fn parse_presets(json: &str) -> Vec<Preset> {
    let mut out = Vec::new();
    let mut rest = json;
    while let Some(at) = rest.find("\"id\":") {
        let after = &rest[at + 5..];
        let id = match between(after, '"', '"') {
            Some(s) => s.to_string(),
            None => break,
        };
        let params_start = match after.find("\"params\":") {
            Some(i) => i,
            None => break,
        };
        let brace = match after[params_start..].find('{') {
            Some(i) => params_start + i,
            None => break,
        };
        let end = match after[brace..].find('}') {
            Some(i) => brace + i,
            None => break,
        };
        let body = &after[brace + 1..end];
        let mut params = Vec::new();
        for pair in body.split(',') {
            let mut kv = pair.splitn(2, ':');
            let (Some(k), Some(v)) = (kv.next(), kv.next()) else { continue };
            let key = k.trim().trim_matches('"');
            if key.is_empty() {
                continue;
            }
            if let Ok(value) = v.trim().parse::<f32>() {
                params.push((key.to_string(), value));
            }
        }
        out.push(Preset { id, params });
        rest = &after[end..];
    }
    out
}

fn between(s: &str, open: char, close: char) -> Option<&str> {
    let start = s.find(open)? + open.len_utf8();
    let end = s[start..].find(close)? + start;
    Some(&s[start..end])
}

fn render(preset: &Preset, note: u8) -> Vec<f32> {
    let mut synth = SynthPlugin::new();
    for (id, value) in &preset.params {
        synth.set_param(&ParamId::from(id.as_str()), *value);
    }
    synth.prepare(SR, BLOCK);

    let hold_blocks = (HOLD_SECS * SR / BLOCK as f64).round() as usize;
    let tail_blocks = (TAIL_SECS * SR / BLOCK as f64).round() as usize;
    let mut out = Vec::with_capacity((hold_blocks + tail_blocks) * BLOCK);

    let on = [TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note, velocity: 100 } }];
    let off = [TimedMidiEvent { frame: 0, event: MidiEvent::NoteOff { note } }];

    let ctx = ProcessContext { sample_rate: SR, block_size: BLOCK, tempo: 120.0, playing: true };
    for block in 0..(hold_blocks + tail_blocks) {
        let mut l = [0.0f32; BLOCK];
        let mut r = [0.0f32; BLOCK];
        let events: &[TimedMidiEvent] = if block == 0 {
            &on
        } else if block == hold_blocks {
            &off
        } else {
            &[]
        };
        synth.process(&ctx, &mut [&mut l, &mut r], events);
        out.extend_from_slice(&l);
    }
    out
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |a, b| a.max(b.abs()))
}

fn write_wav(path: &str, samples: &[f32]) {
    let mut file = std::fs::File::create(path).expect("建不了 wav");
    let data_len = (samples.len() * 2) as u32;
    let mut header = Vec::with_capacity(44);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&(36 + data_len).to_le_bytes());
    header.extend_from_slice(b"WAVEfmt ");
    header.extend_from_slice(&16u32.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes()); // PCM
    header.extend_from_slice(&1u16.to_le_bytes()); // 单声道
    header.extend_from_slice(&(SR as u32).to_le_bytes());
    header.extend_from_slice(&((SR as u32) * 2).to_le_bytes());
    header.extend_from_slice(&2u16.to_le_bytes());
    header.extend_from_slice(&16u16.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&data_len.to_le_bytes());
    file.write_all(&header).expect("写 wav 头失败");
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        pcm.extend_from_slice(&v.to_le_bytes());
    }
    file.write_all(&pcm).expect("写 wav 数据失败");
}
