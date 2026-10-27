//! 诊断本机 CLAP 插件的「宿主接口使用情况」。
//!
//! 用途：排查「插件的原生界面不出声」这类问题——有些插件会先问宿主
//! `clap_host_thread_check`、`clap_host_gui` 之类，宿主答得不对它就不干活。
//!
//! ```sh
//! cargo run -p ai-daw-host-vst3 --example clap_probe
//! ```

use std::sync::atomic::Ordering;

use ai_daw_plugin_api::{MidiEvent, TimedMidiEvent};

fn main() {
    // 例子跑在主线程上，先把自己登记成主线程（真实应用在 Tauri 的 setup 里做这件事）。
    ai_daw_host_vst3::mark_main_thread();

    let catalog = ai_daw_host_vst3::scan_catalog();
    let claps: Vec<_> = catalog.iter().filter(|e| e.format == "clap").collect();
    if claps.is_empty() {
        println!("本机没有扫描到 CLAP 插件。");
        return;
    }

    for entry in claps {
        println!("== {} ==", entry.name);
        let mut plugin = match ai_daw_host_vst3::load_catalog_plugin_on_host(entry)
            .map(|(plugin, _)| plugin)
        {
            Ok(p) => p,
            Err(e) => {
                println!("  加载失败：{e}");
                continue;
            }
        };

        let before = [
            ai_daw_host_vst3::clap::THREAD_CHECK_CALLS[0].load(Ordering::Relaxed),
            ai_daw_host_vst3::clap::THREAD_CHECK_CALLS[1].load(Ordering::Relaxed),
        ];

        plugin.prepare(48_000.0, 512);
        let mut left = [0.0f32; 512];
        let mut right = [0.0f32; 512];
        let ctx = ai_daw_plugin_api::ProcessContext {
            sample_rate: 48_000.0,
            block_size: 512,
            tempo: 120.0,
            playing: true,
        };
        let events = [TimedMidiEvent {
            frame: 0,
            channel: 0,
            event: MidiEvent::NoteOn {
                note: 60,
                velocity: 100,
            },
        }];
        // 先跑一块空的（有些插件在首块才初始化内部引擎），再跑一块带音符的。
        plugin.process(&ctx, &mut [&mut left, &mut right], &[]);
        plugin.process(&ctx, &mut [&mut left, &mut right], &events);

        let after = [
            ai_daw_host_vst3::clap::THREAD_CHECK_CALLS[0].load(Ordering::Relaxed),
            ai_daw_host_vst3::clap::THREAD_CHECK_CALLS[1].load(Ordering::Relaxed),
        ];
        let peak = left.iter().fold(0.0f32, |a, b| a.max(b.abs()));
        println!("  is_main_thread 被问了 {} 次", after[0] - before[0]);
        println!("  is_audio_thread 被问了 {} 次", after[1] - before[1]);
        println!("  渲染峰值：{peak:.4}");
        println!("  参数数：{}", plugin.params().len());
        print_gui_caps(entry);
    }
}

/// 问出这个 CLAP 的 GUI 到底支持什么（这决定了宿主能不能把它显示出来）。
fn print_gui_caps(entry: &ai_daw_host_vst3::CatalogPlugin) {
    let path = match unsafe { clack_host::prelude::PluginEntry::load(&entry.path) } {
        Ok(p) => p,
        Err(e) => {
            println!("  GUI 探测：加载入口失败 {e}");
            return;
        }
    };
    let Some(id) = entry.id.strip_prefix("clap:") else {
        return;
    };
    let Ok(id) = std::ffi::CString::new(id) else {
        return;
    };
    match ai_daw_host_vst3::clap::probe_gui(&path, &id) {
        Ok(report) => println!("  GUI：{report}"),
        Err(e) => println!("  GUI 探测失败：{e}"),
    }
}
