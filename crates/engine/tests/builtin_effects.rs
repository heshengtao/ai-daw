//! 内置效果器（延迟 / 混响）走**真实渲染路径**的验证。
//!
//! 注册表里有条目不等于真的有声音：这里把效果器挂进轨道效果链，用 `render` 出整段音频，
//! 再从能量的时间分布上断言「尾巴确实变长了 / 回声确实出现了」。
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use ai_daw_engine::model::{Clip, ClipId, Note, PluginInstance, Project, Track, TrackId};
use ai_daw_engine::registry::{PluginRegistry, BUILTIN_DELAY, BUILTIN_REVERB};
use ai_daw_engine::render::{render, RenderOptions};

const SR: f64 = 48_000.0;

/// 效果器实例（参数按名字给）。
fn fx(id: &str, params: &[(&str, f32)]) -> PluginInstance {
    PluginInstance {
        plugin_id: id.into(),
        params: params.iter().map(|(k, v)| ((*k).to_string(), *v)).collect::<BTreeMap<_, _>>(),
        state: Vec::new(),
    }
}

/// 一条挂着内置合成器的轨道 + 一个 0.25 s 的短音（尾巴好不好听，全靠效果器）。
fn project(effects: Vec<PluginInstance>) -> Project {
    let mut p = Project::new("fx");
    p.transport.tempo = 120.0;
    p.tracks.push(Track {
        meta: None,
        id: TrackId(1),
        name: "Synth".into(),
        instrument: Some(PluginInstance {
            plugin_id: "builtin.synth".into(),
            params: [("attack".to_string(), 0.001), ("release".to_string(), 0.05)]
                .into_iter()
                .collect(),
            state: Vec::new(),
        }),
        effects,
        clips: vec![Clip {
            id: ClipId(1),
            start: 0.0,
            length: 0.5,
            notes: vec![Note {
                id: 0,
                pitch: 60,
                start: 0.0,
                duration: 0.25,
                velocity: 110,
                volume: 127,
                offset: 0.0,
                legato: false, expr: ai_daw_engine::model::PitchExpression::NONE,
            }],
            audio: None,
        }],
        mixer: Default::default(),
        sends: vec![],
        automation: vec![],
    });
    p
}

/// 渲染成单声道（左右取平均）：多给 2 s 尾音，才看得到效果器的尾巴。
fn render_mono(effects: Vec<PluginInstance>) -> Vec<f32> {
    let registry = PluginRegistry::with_builtins();
    let options = RenderOptions { sample_rate: SR, tail_seconds: 2.0, settle_seconds: 0.0 };
    let buffer = render(&project(effects), &registry, &options);
    let data = buffer.data();
    data.chunks(2).map(|c| (c[0] + c[1]) * 0.5).collect()
}

/// 某段区间的均方根。
fn rms(data: &[f32], from: usize, to: usize) -> f64 {
    let slice = &data[from.min(data.len())..to.min(data.len())];
    if slice.is_empty() {
        return 0.0;
    }
    (slice.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / slice.len() as f64).sqrt()
}

/// 混响：干声早已静音的那一段，必须还有能量。
#[test]
fn reverb_extends_the_tail() {
    let dry = render_mono(vec![]);
    let wet = render_mono(vec![fx(BUILTIN_REVERB, &[("size", 0.9), ("damp", 0.2), ("mix", 0.8)])]);

    // 音符 0.25 s + 0.05 s 释音 ≈ 0.3 s；这里看 0.6..1.6 s 这一段。
    let from = (0.6 * SR) as usize;
    let to = (1.6 * SR) as usize;
    let (d, w) = (rms(&dry, from, to), rms(&wet, from, to));
    assert!(d < 1e-4, "干声在这段里应当已经静音：{d}");
    assert!(w > 1e-3, "混响应当时尾巴仍有能量：{w}");
}

/// 混响 `mix = 0` 不该改变干声（效果器「关掉」时不能悄悄动信号）。
#[test]
fn reverb_at_zero_mix_matches_the_dry_render() {
    let dry = render_mono(vec![]);
    let bypassed = render_mono(vec![fx(BUILTIN_REVERB, &[("mix", 0.0)])]);
    assert_eq!(dry, bypassed);
}

/// 延迟：回声落在音符结束**之后**，且反馈越大回声越多。
#[test]
fn delay_echoes_after_the_note() {
    let dry = render_mono(vec![]);
    // 0.51 s 的延迟：音符 0.3 s 就收干净了。
    let echo = render_mono(vec![fx(BUILTIN_DELAY, &[("time", 0.5), ("feedback", 0.6), ("mix", 0.9)])]);

    let from = (0.45 * SR) as usize;
    let to = (0.7 * SR) as usize;
    assert!(rms(&dry, from, to) < 1e-4, "干声在这里应当已经静音");
    assert!(rms(&echo, from, to) > 1e-3, "这里应当有第一次回声");

    // 第二次回声（约 1.02 s）应当比第一次弱，但仍在。
    let (first, second) = (rms(&echo, from, to), rms(&echo, (0.95 * SR) as usize, (1.1 * SR) as usize));
    assert!(second > 1e-4, "应当还有第二次回声：{second}");
    assert!(second < first, "回声应当逐次衰减：{first} -> {second}");
}
