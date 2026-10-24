//! 复现「插件原生界面开着时，音频线程还出不出声、能连响几下」。
//!
//! 结构照搬真实宿主：
//! - **主线程**（这里就是 `main`）：开界面窗口，并按帧泵 `service_platform_events`；
//! - **音频线程**（另起一条）：按块 `process`，统计峰值。
//!
//! 只要本机装了 KR-106（VST3 / CLAP），就能对比两种格式的表现：
//!
//! ```text
//! cargo run -p ai-daw-host-vst3 --example editor_audio -- vst3
//! cargo run -p ai-daw-host-vst3 --example editor_audio -- clap
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ai_daw_plugin_api::{MidiEvent, ProcessContext, TimedMidiEvent};

const SR: f64 = 48_000.0;
const BLOCK: usize = 512;
/// 音频线程跑多久（秒）。
const SECONDS: f64 = 6.0;

fn main() {
    let format = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "vst3".to_string());
    ai_daw_host_vst3::mark_main_thread();

    let Some(scanned) = ai_daw_host_vst3::scan()
        .into_iter()
        .find(|p| p.name.contains("KR-106") && p.format == format)
    else {
        eprintln!("本机未安装 KR-106 的 {format} 版本");
        std::process::exit(2);
    };
    let entry = scanned.catalog();

    let (plugin, handle) = match ai_daw_host_vst3::load_catalog_plugin_with_editor(&entry) {
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
    eprintln!(
        "{format}: 已加载，界面 {} ",
        if entry.has_editor { "有" } else { "无" }
    );

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
                let ctx = ProcessContext {
                    sample_rate: SR,
                    block_size: BLOCK,
                    tempo: 120.0,
                    playing: true,
                };
                let mut left = vec![0.0f32; BLOCK];
                let mut right = vec![0.0f32; BLOCK];
                let mut blocks = 0u32;
                let total = (SECONDS * SR / BLOCK as f64) as u32;
                // **按真实时间推进**：否则整段音频会在几十毫秒内渲染完——界面还没开就结束了，
                // 测出来的「界面开着还有声」是假的。
                let started = std::time::Instant::now();
                // 每 0.4 秒一个音：按下 → 0.2 秒后松开。逐个音记它的峰值。
                let per_note = (0.4 * SR / BLOCK as f64) as u32;
                let hold = (0.2 * SR / BLOCK as f64) as u32;
                let mut note_peak = 0.0f32;
                let mut note_index = 0u32;
                while !stop.load(Ordering::Relaxed) && blocks < total {
                    let in_note = blocks % per_note;
                    let mut evs: Vec<TimedMidiEvent> = Vec::new();
                    if in_note == 0 {
                        note_index += 1;
                    }
                    if in_note == 0 {
                        evs.push(TimedMidiEvent {
                            frame: 0,
                            channel: 0,
                            event: MidiEvent::NoteOn {
                                note: 60,
                                velocity: 100,
                            },
                        });
                    } else if in_note == hold {
                        evs.push(TimedMidiEvent {
                            frame: 0,
                            channel: 0,
                            event: MidiEvent::NoteOff { note: 60 },
                        });
                    }
                    // 等这一块的真实时长过去（**放在最前面**：`continue` 也要按时间走，
                    // 否则拿不到锁时空转会一路把整段跑完，测出来的东西全是假的）。
                    let due = started + Duration::from_secs_f64(blocks as f64 * BLOCK as f64 / SR);
                    if let Some(rest) = due.checked_duration_since(std::time::Instant::now()) {
                        std::thread::sleep(rest);
                    }
                    let Ok(mut plugin) = audio_plugin.try_lock() else {
                        // 与真实宿主一致：拿不到锁这一块就静音跳过。
                        blocks += 1;
                        continue;
                    };
                    plugin.process(&ctx, &mut [&mut left, &mut right], &evs);
                    let mut block_peak = 0.0f32;
                    for s in left.iter().chain(right.iter()) {
                        block_peak = block_peak.max(s.abs());
                    }
                    note_peak = note_peak.max(block_peak);
                    blocks += 1;
                    // 一个音走完，记下这个音的峰值。
                    if blocks % per_note == 0 {
                        eprintln!("  第 {note_index} 个音：峰值 {note_peak:.3}");
                        peaks.lock().unwrap().push(note_peak);
                        note_peak = 0.0;
                    }
                }
            })
            .expect("起音频线程");
    }

    // 先让它响几个音，再开界面，然后继续响——这样能看出「开界面之后第几个音开始没声」。
    std::thread::sleep(Duration::from_millis(1500));
    let before: Vec<f32> = peaks.lock().unwrap().clone();
    eprintln!("  开界面前已响 {} 个音", before.len());

    eprintln!("  打开界面…");
    let window = match handle.open("KR-106", None) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("  开界面失败（无显示环境时会这样）：{e}");
            stop.store(true, Ordering::Relaxed);
            return;
        }
    };

    // 界面开着跑一会儿之后，插一次 `reset()`——真实宿主里 `Player::silence()`（暂停/停止）
    // 对乐器做的就是这件事。这一步是「第二下播放不出来」的嫌疑点。
    let reset_at = std::time::Instant::now();
    let mut did_reset = false;
    // 界面开着的同时按帧泵事件，直到音频线程跑完。
    while !stop.load(Ordering::Relaxed) {
        if !did_reset && reset_at.elapsed() > Duration::from_millis(1500) {
            let reset_note = peaks.lock().unwrap().len();
            {
                let mut plugin = audio_plugin.lock().unwrap();
                plugin.reset();
            }
            did_reset = true;
            eprintln!("  ⟳ 在第 {reset_note} 个音之后调用了 reset()（等价于暂停/停止）");
        }
        window.service();
        std::thread::sleep(Duration::from_millis(16));
        if peaks.lock().unwrap().len() >= (SECONDS / 0.4) as usize {
            break;
        }
    }

    let all: Vec<f32> = peaks.lock().unwrap().clone();
    eprintln!("\n  每个音的峰值：");
    let mut first_silent: Option<usize> = None;
    for (i, peak) in all.iter().enumerate() {
        let mark = if *peak > 0.01 { "✓" } else { "✗ 没声" };
        eprintln!("    第 {} 个音：{peak:.3} {mark}", i + 1);
        if *peak <= 0.01 && first_silent.is_none() {
            first_silent = Some(i + 1);
        }
    }
    match first_silent {
        None => eprintln!("\n✅ 每个音都出声（界面开着）"),
        Some(n) => eprintln!(
            "\n❌ 第 {n} 个音开始没声（开界面在第 {} 个音之后）",
            before.len()
        ),
    }

    stop.store(true, Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(200));
    window.close();
}
