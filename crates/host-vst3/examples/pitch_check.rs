//! VST3 乐器的**音高**回归验证：同一个音（A4 = MIDI 69）在不同采样率下都该是 440 Hz。
//!
//! 为什么需要这个用例：VST3 后端早先**从不把宿主的采样率交给插件**——实例是加载时按
//! `vst3-host` 的默认 44100 配置的，而宿主按设备采样率消费插件的样本。后果是插件内部一切
//! 与采样率有关的计算（振荡器相位、包络/LFO 时间、延迟线）都被拉伸 设备采样率/44100 倍：
//! 48 kHz 的设备上正好 **+146.6 音分**（≈一个半音，听上去像「这个乐器比别人高一个多半音」）
//! 并且快 8.8%。内置音源、CLAP、AU 都不受影响，所以只有某些 VST3 插件「对不上琴键」。
//!
//! 用法（需要本机装了对应插件；名字片段匹配）：
//!
//! ```text
//! cargo run -p ai-daw-host-vst3 --example pitch_check -- JC303
//! ```
//!
//! 退出码非 0 表示有采样率下音高不对。

use std::path::PathBuf;

use ai_daw_plugin_api::{MidiEvent, Plugin, PluginKind, ProcessContext, TimedMidiEvent};

/// 每个采样率渲染多久（秒）。
const SECONDS: f64 = 1.2;
/// 块大小（帧）。
const BLOCK: usize = 512;
/// 参考音：写下去的 A4。
const A4: f64 = 440.0;
/// 只接受 ±5 音分以内的偏差。
const TOLERANCE_CENTS: f64 = 5.0;

fn main() {
    // 扫描与加载都必须在主线程（`vst3-host` 的要求），example 的 main 就是。
    ai_daw_host_vst3::mark_main_thread();

    let want = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "JC303".to_string());

    // 插件管理器把插件装在应用数据目录里，扫描要把它带上（系统默认路径之外）。
    let managed: Vec<PathBuf> = std::env::var("HOME")
        .ok()
        .map(|home| home + "/Library/Application Support/com.aidaw.app/plugins/vst3")
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
        .into_iter()
        .collect();

    let entry = ai_daw_host_vst3::scan_catalog_with(&managed, &[], &[])
        .into_iter()
        .find(|p| p.format == "vst3" && p.kind == PluginKind::Instrument && p.name.contains(&want));
    let Some(entry) = entry else {
        eprintln!(
            "没找到名字里含「{want}」的 VST3 乐器。\n用法：cargo run -p ai-daw-host-vst3 --example pitch_check -- <名字片段>"
        );
        std::process::exit(2);
    };
    println!(
        "插件：{}（{}）\n写 MIDI 69（A4），期望 440.00 Hz：",
        entry.name, entry.id
    );

    let mut failures = 0usize;
    for rate in [44_100.0f64, 48_000.0, 96_000.0] {
        let plugin = ai_daw_host_vst3::load_catalog_plugin(&entry);
        let measured = match plugin {
            Ok(mut plugin) => measure_pitch(&mut *plugin, rate),
            Err(err) => {
                eprintln!("  {rate:>7.0} Hz：加载失败：{err}");
                failures += 1;
                continue;
            }
        };
        match measured {
            Some(hz) => {
                let cents = 1200.0 * (hz / A4).log2();
                let ok = cents.abs() <= TOLERANCE_CENTS;
                println!(
                    "  {rate:>7.0} Hz：实测 {hz:7.2} Hz（{cents:+6.1} 音分）{}",
                    if ok {
                        "✓"
                    } else {
                        "✗ 音高被采样率拉伸"
                    }
                );
                if !ok {
                    failures += 1;
                }
            }
            None => {
                println!("  {rate:>7.0} Hz：✗ 没渲染出声音");
                failures += 1;
            }
        }
    }

    if failures > 0 {
        eprintln!("\n{failures} 个采样率下音高不对——`prepare` 很可能没把采样率交给插件。");
        std::process::exit(1);
    }
    println!("\n全部通过：乐器在任何采样率下都按写下的音高发声。");
}

/// 在给定采样率下渲染一个 A4，返回测得的基频（Hz）；没出声则 `None`。
fn measure_pitch(plugin: &mut dyn Plugin, rate: f64) -> Option<f64> {
    plugin.prepare(rate, BLOCK);
    let ctx = ProcessContext {
        sample_rate: rate,
        block_size: BLOCK,
        tempo: 120.0,
        playing: true,
    };

    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];
    let mut mono: Vec<f64> = Vec::with_capacity((SECONDS * rate) as usize + BLOCK);
    let blocks = (SECONDS * rate / BLOCK as f64).ceil() as usize;
    for block in 0..blocks {
        let events = if block == 0 {
            vec![TimedMidiEvent {
                frame: 0,
                channel: 0,
                event: MidiEvent::NoteOn {
                    note: 69,
                    velocity: 90,
                },
            }]
        } else {
            Vec::new()
        };
        plugin.process(&ctx, &mut [&mut left, &mut right], &events);
        mono.extend(left.iter().map(|v| f64::from(*v)));
    }

    // 跳过起音，取 0.35 秒加 Hann 窗——窗长够分辨 0.25 Hz 的扫描步长。
    let skip = (0.25 * rate) as usize;
    let len = (0.35 * rate) as usize;
    if mono.len() < skip + len {
        return None;
    }
    let tau = std::f64::consts::TAU;
    let window: Vec<f64> = mono[skip..skip + len]
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let hann = 0.5 - 0.5 * (tau * i as f64 / (len - 1) as f64).cos();
            v * hann
        })
        .collect();
    let rms = (window.iter().map(|v| v * v).sum::<f64>() / len as f64).sqrt();
    if rms < 1e-4 {
        return None;
    }

    // 基频搜索：只扫 A4 附近（±20%）。本用例只回答「同一个音有没有被拉伸」，
    // 不做通用音高检测——那需要自相关/谐波求和，没有必要。
    let (mut best_hz, mut best_mag) = (0.0f64, -1.0f64);
    let mut hz = A4 * 0.8;
    while hz <= A4 * 1.2 {
        let w = tau * hz / rate;
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (n, x) in window.iter().enumerate() {
            let phase = w * n as f64;
            re += x * phase.cos();
            im -= x * phase.sin();
        }
        let mag = re * re + im * im;
        if mag > best_mag {
            best_mag = mag;
            best_hz = hz;
        }
        hz += 0.25;
    }
    Some(best_hz)
}
