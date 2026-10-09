//! 复现「插件原生界面开着时，音频线程还出不出声」。
//!
//! 结构照搬真实宿主：
//! - **主线程**（这里就是 `main`）：开界面窗口，并按帧泵 `service_platform_events`；
//! - **音频线程**（另起一条）：按块 `process`，统计峰值。
//!
//! 只要本机装了 KR-106（VST3 / CLAP），就能对比两种格式的表现：
//!
//! ```text
//! cargo run -p daw-party-host-vst3 --example editor_audio -- vst3
//! cargo run -p daw-party-host-vst3 --example editor_audio -- clap
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use daw_party_plugin_api::{MidiEvent, ProcessContext, TimedMidiEvent};

const SR: f64 = 48_000.0;
const BLOCK: usize = 512;
/// 音频线程跑多久（秒）。
const SECONDS: f64 = 2.0;

fn main() {
    let format = std::env::args().nth(1).unwrap_or_else(|| "vst3".to_string());
    daw_party_host_vst3::mark_main_thread();

    let Some(scanned) = daw_party_host_vst3::scan()
        .into_iter()
        .find(|p| p.name.contains("KR-106") && p.format == format)
    else {
        eprintln!("本机未安装 KR-106 的 {format} 版本");
        std::process::exit(2);
    };
    let entry = scanned.catalog();

    let (plugin, handle) = match daw_party_host_vst3::load_catalog_plugin_with_editor(&entry) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("加载失败：{e}");
            std::process::exit(2);
        }
    };
    let Some(handle) = handle else {
        eprintln!("{format} 没能拿到编辑器句柄");
        std::process::exit(2);
    };
    println!("{format}: 已加载，界面 {} ", if entry.has_editor { "有" } else { "无" });

    // 音频线程：先按「没开界面」跑一段，再在界面打开后跑一段。
    let audio_plugin = Arc::new(Mutex::new(plugin));
    let stop = Arc::new(AtomicBool::new(false));
    let peaks = Arc::new(Mutex::new(Vec::<f32>::new()));

    {
        let audio_plugin = Arc::clone(&audio_plugin);
        let stop = Arc::clone(&stop);
        let peaks = Arc::clone(&peaks);
        let mut plugin = audio_plugin.lock().unwrap();
        plugin.prepare(SR, BLOCK);
        drop(plugin);

        std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || {
                let ctx = ProcessContext { sample_rate: SR, block_size: BLOCK, tempo: 120.0, playing: true };
                let evs = [TimedMidiEvent {
                    frame: 0,
                    event: MidiEvent::NoteOn { note: 60, velocity: 100 },
                }];
                let mut left = vec![0.0f32; BLOCK];
                let mut right = vec![0.0f32; BLOCK];
                let mut peak = 0.0f32;
                let mut blocks = 0u32;
                let total = (SECONDS * SR / BLOCK as f64) as u32;
                let mut sent = false;
                while !stop.load(Ordering::Relaxed) && blocks < total {
                    let Ok(mut plugin) = audio_plugin.try_lock() else {
                        // 与真实宿主一致：拿不到锁这一块就静音跳过。
                        blocks += 1;
                        continue;
                    };
                    let first = !sent;
                    plugin.process(&ctx, &mut [&mut left, &mut right], if first { &evs } else { &[] });
                    sent = true;
                    // **每块单独取峰值**再推进列表：累计最大值是单调的，永远测不出「中途变静音」。
                    let mut block_peak = 0.0f32;
                    for s in left.iter().chain(right.iter()) {
                        block_peak = block_peak.max(s.abs());
                    }
                    peak = peak.max(block_peak);
                    blocks += 1;
                    peaks.lock().unwrap().push(block_peak);
                }
                println!("  音频线程结束：跑了 {blocks} 块，峰值 {peak:.3}");
            })
            .expect("起音频线程");
    }

    // 「没开界面」阶段：等 0.5 秒，读一次峰值。
    std::thread::sleep(Duration::from_millis(700));
    let before = *peaks.lock().unwrap().last().unwrap_or(&0.0);
    println!("  开界面前：峰值 {before:.3}");

    // 打开界面，然后按帧泵事件（模拟宿主 ticker）。
    println!("  打开界面…");
    let window = match handle.open("KR-106", None) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("  开界面失败（无显示环境时会这样）：{e}");
            stop.store(true, Ordering::Relaxed);
            return;
        }
    };
    // 界面打开后，模拟宿主在「暂停 / 停止」时做的事情：对乐器调 `reset()`。
    // 真实宿主里 `Player::silence()` 就是这么干的（丢弃悬挂声部 + 重开处理）。
    std::thread::sleep(Duration::from_millis(300));
    let mid = *peaks.lock().unwrap().last().unwrap_or(&0.0);
    println!("  reset 之前：峰值 {mid:.3}");
    {
        let mut plugin = audio_plugin.lock().unwrap();
        plugin.reset();
    }
    println!("  已调用 reset()");
    let open_since = std::time::Instant::now();
    while open_since.elapsed() < Duration::from_millis(1300) {
        window.service();
        std::thread::sleep(Duration::from_millis(16));
    }
    let after = *peaks.lock().unwrap().last().unwrap_or(&0.0);
    // 只看界面打开之后那段有没有「动」：最后 0.5 秒的峰值增量。
    let len = peaks.lock().unwrap().len();
    let tail_peak = {
        let p = peaks.lock().unwrap();
        let from = len.saturating_sub((0.4 * SR / BLOCK as f64) as usize);
        p[from..].iter().cloned().fold(0.0f32, f32::max)
    };
    println!("  开界面后：峰值 {after:.3}");
    println!("  开界面后最后 0.4 秒的峰值：{tail_peak:.3}");

    if tail_peak > 0.01 {
        println!("✅ 界面开着时音频线程仍在出声音");
    } else {
        println!("❌ 界面开着时音频线程没有输出（复现成功）");
    }

    stop.store(true, Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(200));
    window.close();
}
