//! 阻塞点探针：单独加载 RdPiano，并在不同条件下开它的原生界面。
//!
//! 目的：把「打开界面卡死」从 Tauri 里剥出来，逐个变量复刻宿主环境。
//!
//! ```text
//! cargo run -p ai-daw-host-vst3 --example rdpiano_probe            # 基线
//! PROBE_STA=1 cargo run ... --example rdpiano_probe                # 主线程 STA（WebView2 那样）
//! PROBE_DPI=1 cargo run ... --example rdpiano_probe                # Per-Monitor V2
//! PROBE_OFFTHREAD=1 cargo run ... --example rdpiano_probe          # 插件在别的线程上加载并 process
//! ```
//!
//! 打点全走 stderr 并 flush；另有心跳线程，主线程卡死时它照常打点。

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ai_daw_plugin_api::ProcessContext;

const SR: f64 = 48_000.0;
const BLOCK: usize = 512;

fn say(message: impl AsRef<str>) {
    let mut err = std::io::stderr();
    let _ = writeln!(err, "{}", message.as_ref());
    let _ = err.flush();
}

/// 加载插件（可选在后台线程上加载并 prepare/process）。
fn load(
    entry: &ai_daw_host_vst3::CatalogPlugin,
) -> Option<(
    Box<dyn ai_daw_plugin_api::Plugin>,
    ai_daw_host_vst3::EditorHandle,
)> {
    if std::env::var_os("PROBE_OFFTHREAD").is_some() {
        let entry = entry.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("loader".into())
            .spawn(move || {
                match ai_daw_host_vst3::load_catalog_plugin_with_editor(&entry) {
                    Ok((mut plugin, Some(handle))) => {
                        // 真实宿主：插件在音频线程上第一次被 prepare / process。
                        // PROBE_LOAD_ONLY=1 时只加载、不 prepare，用来区分是「加载线程」还是
                        // 「第一次 prepare/process 的线程」把插件绑住了。
                        if std::env::var_os("PROBE_LOAD_ONLY").is_none() {
                            plugin.prepare(SR, BLOCK);
                            let ctx = ProcessContext {
                                sample_rate: SR,
                                block_size: BLOCK,
                                tempo: 120.0,
                                playing: true,
                            };
                            let mut left = vec![0.0f32; BLOCK];
                            let mut right = vec![0.0f32; BLOCK];
                            for _ in 0..80 {
                                plugin.process(&ctx, &mut [&mut left, &mut right], &[]);
                                std::thread::sleep(Duration::from_millis(11));
                            }
                        }
                        let _ = tx.send(Some((plugin, handle)));
                    }
                    _ => {
                        let _ = tx.send(None);
                    }
                }
            })
            .expect("起加载线程");
        rx.recv().ok().flatten()
    } else {
        match ai_daw_host_vst3::load_catalog_plugin_with_editor(entry) {
            Ok((plugin, Some(handle))) => Some((plugin, handle)),
            Ok(_) => {
                say("没有编辑器句柄");
                None
            }
            Err(error) => {
                say(format!("加载失败：{error}"));
                None
            }
        }
    }
}

fn main() {
    // Tauri 的进程是 Per-Monitor V2 DPI 感知。
    #[cfg(target_os = "windows")]
    if std::env::var_os("PROBE_DPI").is_some() {
        let ok = unsafe {
            winapi::um::winuser::SetProcessDpiAwarenessContext(
                winapi::shared::windef::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            )
        };
        say(format!("SetProcessDpiAwarenessContext(PMv2) -> {ok:?}"));
    }

    // Tauri 的主线程是 STA（WebView2 要求）。
    #[cfg(target_os = "windows")]
    if std::env::var_os("PROBE_STA").is_some() {
        let hr = unsafe {
            winapi::um::combaseapi::CoInitializeEx(
                std::ptr::null_mut(),
                winapi::um::objbase::COINIT_APARTMENTTHREADED,
            )
        };
        say(format!("CoInitializeEx(STA) -> {hr:#x}"));
    }

    ai_daw_host_vst3::mark_main_thread();
    say(format!("main thread = {:?}", std::thread::current().id()));

    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| {
            // 默认看应用托管目录里的 RdPiano（插件管理器装的就在这儿）。
            std::env::var_os("APPDATA").map(|base| {
                PathBuf::from(base)
                    .join("com.aidaw.app")
                    .join("plugins")
                    .join("vst3")
                    .join("rdpiano_juce.vst3")
            })
        })
        .unwrap_or_default();
    if path.as_os_str().is_empty() {
        say("用法：rdpiano_probe <插件路径>");
        return;
    }
    let dir = path.parent().map(PathBuf::from).unwrap_or_default();
    let scan_at = Instant::now();
    let catalog = ai_daw_host_vst3::scan_catalog_with(&[dir], &[], &[]);
    say(format!(
        "scan: {} 个，耗时 {:?}",
        catalog.len(),
        scan_at.elapsed()
    ));
    let wanted = path.to_string_lossy().to_lowercase();
    let Some(entry) = catalog
        .iter()
        .find(|e| e.path.to_string_lossy().to_lowercase() == wanted)
    else {
        say("扫不到插件");
        return;
    };
    say(format!(
        "entry = {} / has_editor={}",
        entry.name, entry.has_editor
    ));

    let load_at = Instant::now();
    let Some((plugin, handle)) = load(entry) else {
        return;
    };
    say(format!("加载完成，耗时 {:?}", load_at.elapsed()));

    // 音频线程：和真实宿主一样，插件实例一直被 process。
    let audio = Arc::new(Mutex::new(plugin));
    let stop = Arc::new(AtomicBool::new(false));
    if std::env::var_os("PROBE_LOAD_ONLY").is_none() {
        let audio = Arc::clone(&audio);
        let stop = Arc::clone(&stop);
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
                let mut blocks = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    if let Ok(mut plugin) = audio.try_lock() {
                        plugin.process(&ctx, &mut [&mut left, &mut right], &[]);
                        blocks += 1;
                        if blocks % 94 == 0 {
                            say(format!("  [音频] {blocks} 块"));
                        }
                    }
                    std::thread::sleep(Duration::from_millis(11));
                }
            })
            .expect("起音频线程");
    }
    std::thread::sleep(Duration::from_millis(600));

    // 心跳：主线程卡死时它还在打点。
    {
        let stop = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("heartbeat".into())
            .spawn(move || {
                let mut tick = 0;
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(500));
                    tick += 1;
                    say(format!("  [心跳 {tick}] 进程还活着"));
                }
            })
            .expect("起心跳线程");
    }

    say("→ 主线程：handle.open() 之前");
    let open_at = Instant::now();
    match handle.open("RdPiano probe", None, None) {
        Ok(window) => {
            say(format!(
                "← 主线程：open() 返回 Ok，耗时 {:?}",
                open_at.elapsed()
            ));
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                window.service();
                std::thread::sleep(Duration::from_millis(16));
            }
            say("关闭界面");
            window.close();
        }
        Err(error) => say(format!("← 主线程：open() 返回 Err({error})")),
    }
    stop.store(true, Ordering::Relaxed);
    say("done");
}
