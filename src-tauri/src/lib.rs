#![forbid(unsafe_code)]

mod audio;
mod commands;
mod main_thread;
mod mcp;
mod plugins;
mod presets;
mod state;

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tauri::{Emitter, Manager};

use daw_party_host_vst3::EditorHandle;

use crate::audio::{AudioEngine, TransportSnapshot};
use crate::state::{AppState, EVENT_PLUGINS, EVENT_TRANSPORT};

/// 传输位置的推送周期：约 30 Hz，足够让播放头看起来连续，又不会淹没前端。
const TRANSPORT_TICK: Duration = Duration::from_millis(33);

/// 插件窗口的轮询周期：把插件发起的窗口事件送进去，并清理被用户关掉的窗口。
const EDITOR_TICK: Duration = Duration::from_millis(80);

/// 创建主窗口，去掉系统自绘的标题栏：
/// - macOS：Overlay 风格（保留原生红黄绿，去掉系统标题栏，内容延伸到顶部）
/// - Windows / Linux：完全无边框，窗口按钮由前端自绘
fn build_main_window(app: &tauri::AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    let config = &app.config().app.windows[0];
    let mut builder = tauri::WebviewWindowBuilder::from_config(app, config)?;

    #[cfg(target_os = "macos")]
    {
        builder = builder
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true);
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        builder = builder.decorations(false);
    }

    builder.build()
}

/// 启动传输状态推送线程。
///
/// 只在状态真正变化时才发事件：播放中位置每帧都在变，因此约 30 Hz；暂停后位置不再
/// 变化，于是**不会**有任何无谓的推送。
fn spawn_transport_ticker(app: tauri::AppHandle, audio: Arc<AudioEngine>) {
    thread::Builder::new()
        .name("daw-party-transport".into())
        .spawn(move || {
            let mut last: Option<TransportSnapshot> = None;
            loop {
                thread::sleep(TRANSPORT_TICK);
                let snapshot = audio.snapshot();
                if last.as_ref() == Some(&snapshot) {
                    continue;
                }
                if app.emit(EVENT_TRANSPORT, &snapshot).is_err() {
                    // 窗口已关闭：没有订阅者了，线程可以退出。
                    break;
                }
                last = Some(snapshot);
            }
        })
        .expect("failed to spawn transport thread");
}

/// 启动插件扫描线程：结果写入共享状态（音频线程据此重建注册表）并广播给界面。
///
/// 扫描会 dlopen 每个插件，可能耗时数秒，所以放到后台，不阻塞窗口出现。
fn spawn_plugin_scan(app: tauri::AppHandle) {
    thread::Builder::new()
        .name("daw-party-scan".into())
        .spawn(move || {
            let catalog = daw_party_host_vst3::scan_catalog();
            {
                let state = app.state::<AppState>();
                state.install_catalog(catalog.clone());
            }
            let _ = app.emit(EVENT_PLUGINS, &catalog);
        })
        .expect("failed to spawn plugin scan thread");
}

/// 宿主窗口（嵌入模式的插件界面住在里面）要关闭时，**先把插件的界面收掉**。
///
/// CLAP 要求 `set_parent` 给它的那个原生视图一直有效到 `clap_plugin_gui.destroy()` 被调用为止。
/// 用户在宿主窗口上点红灯关窗时如果我们什么都不做，插件会抱着一个即将被释放的 NSView；之后
/// 任何一次 GUI 调用（我们每 80ms 的 `on_main_thread` 泵、或插件自己的界面定时器）都会踩到
/// 悬空指针，**直接崩溃**——而且这个坏状态会一直留到重启。
///
/// `CloseRequested` 正好在窗口销毁**之前**、于主线程触发，是收尾的唯一正确时机。
fn on_host_window_closing(window: &tauri::Window, event: &tauri::WindowEvent) {
    if !matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
        return;
    }
    let Some(track) = crate::commands::host_window_track(window.label()) else {
        return;
    };
    let state = window.app_handle().state::<AppState>();
    if let Some(plugin_window) = crate::state::with_editor_windows(|windows| windows.remove(&track)) {
        plugin_window.close();
    }
    state.open_editors.lock().unwrap().remove(&track);
    state.host_windows.lock().unwrap().remove(&track);
}

/// 启动插件窗口轮询线程：窗口级的事件（缩放、DPI）必须在主线程泵进插件。
fn spawn_editor_ticker(app: tauri::AppHandle) {
    thread::Builder::new()
        .name("daw-party-editors".into())
        .spawn(move || loop {
            thread::sleep(EDITOR_TICK);
            let app_for_main = app.clone();
            let dispatched = app.run_on_main_thread(move || {
                let state = app_for_main.state::<AppState>();
                let mut open = state.open_editors.lock().unwrap();
                let hosts = state.host_windows.clone();
                // 需要连带关掉的宿主窗口，等到借用结束之后再关（见下面的说明）。
                let mut close_hosts: Vec<u64> = Vec::new();
                crate::state::with_editor_windows(|windows| {
                    for window in windows.values() {
                        window.service();
                    }
                    // 结束的两种情况：插件自己关了窗口；或者我们给「嵌入模式」插件开的那扇
                    // 宿主窗口被用户关掉了（那时插件不会通知我们）。
                    let doomed: Vec<u64> = windows
                        .iter()
                        .filter(|(track, w)| {
                            if w.closed_by_user() {
                                return true;
                            }
                            let is_host = hosts.lock().unwrap().contains(track);
                            is_host
                                && app_for_main
                                    .get_webview_window(&crate::commands::host_window_label(**track))
                                    .is_none()
                        })
                        .map(|(track, _)| *track)
                        .collect();
                    for track in doomed {
                        // 必须走 `close()` 而不是直接丢：它先 hide + destroy 插件的界面。
                        // 直接丢会把插件的原生视图留成悬空指针，之后一碰就崩。
                        if let Some(window) = windows.remove(&track) {
                            window.close();
                        }
                        // 界面住在宿主窗口里的话，那扇窗也要收掉；插件自己关了界面时它
                        // 更会剩在那里变成一扇空窗。
                        if hosts.lock().unwrap().remove(&track) {
                            close_hosts.push(track);
                        }
                    }
                    open.clear();
                    open.extend(windows.keys().copied());
                });

                // 关宿主窗口必须在 `with_editor_windows` 的借用**之外**做：关窗会触发
                // `CloseRequested`，我们的处理器又要去借同一个 RefCell——嵌套借用会 panic。
                for track in close_hosts {
                    if let Some(window) =
                        app_for_main.get_webview_window(&crate::commands::host_window_label(track))
                    {
                        let _ = window.close();
                    }
                }

                // 插件可能在自己的线程上请求「叫我的主线程回调」（`request_callback`）。
                // 这件事**与窗口是否打开无关**，不服务它插件那边可能一直等下去，所以对每个
                // 登记过的实例都服务一次。先 clone 出来再锁插件实例，避免与音频线程登记
                // 句柄时的加锁顺序相反。
                let handles: Vec<EditorHandle> = state
                    .editor_handles
                    .lock()
                    .unwrap()
                    .values()
                    .cloned()
                    .collect();
                for handle in handles {
                    handle.service_requests();
                }
            });
            if dispatched.is_err() {
                // 主窗口已销毁，收工。
                break;
            }
        })
        .expect("failed to spawn editor ticker");
}

/// 预设文件的位置：应用配置目录下的 `presets.json`。
///
/// 拿不到目录时返回 `None`（预设仍能用，只是重启后不保留），不因此启动失败。
fn preset_file(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.path().app_config_dir().ok().map(|dir| dir.join("presets.json"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(|window, event| on_host_window_closing(window, event))
        .manage(AppState::new("untitled"))
        .setup(|app| {
            // CLAP 插件会问宿主「这是主线程吗」——必须在真正的主线程上登记一次。
            daw_party_host_vst3::mark_main_thread();
            // 还得给它一个「把活派到主线程」的手：CLAP 的 init / 参数枚举 / activate 都只能
            // 在主线程做，而这些调用发生在音频线程构建 Player 的时候。
            crate::main_thread::TauriMainThread::install(app.handle());
            build_main_window(app.handle())?;
            let state = app.state::<AppState>();
            state.init_presets(preset_file(app.handle()));
            let audio = state.audio.clone();
            drop(state);
            spawn_transport_ticker(app.handle().clone(), audio);
            spawn_plugin_scan(app.handle().clone());
            spawn_editor_ticker(app.handle().clone());
            // 内嵌 MCP 服务器：把 DAW 能力暴露给 AI 代理（只监听回环）。
            crate::mcp::spawn(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_project,
            commands::add_track,
            commands::remove_track,
            commands::rename_track,
            commands::set_track_instrument,
            commands::set_param,
            commands::set_params,
            commands::set_tempo,
            commands::set_time_signature,
            commands::set_keyframe,
            commands::remove_keyframe,
            commands::new_project,
            commands::localize_starter_tracks,
            commands::add_clip,
            commands::remove_clip,
            commands::move_clip,
            commands::move_clips,
            commands::resize_clip,
            commands::add_note,
            commands::set_note,
            commands::set_notes,
            commands::remove_note,
            commands::remove_notes,
            commands::remove_clips,
            commands::set_gain,
            commands::set_pan,
            commands::set_mute,
            commands::set_solo,
            commands::set_master_gain,
            commands::monitor_track_mix,
            commands::history_depths,
            commands::undo,
            commands::redo,
            commands::transport_status,
            commands::retry_audio,
            commands::transport_play,
            commands::transport_pause,
            commands::transport_stop,
            commands::transport_seek,
            commands::transport_set_loop,
            commands::save_project,
            commands::open_project,
            commands::render_preview,
            commands::analyze_project,
            commands::export_wav,
            commands::list_plugins,
            commands::rescan_plugins,
            commands::plugin_params,
            commands::add_effect,
            commands::remove_effect,
            commands::move_effect,
            commands::set_effect_param,
            commands::open_plugin_window,
            commands::close_plugin_window,
            commands::plugin_window_open,
            commands::open_synth_window,
            commands::scope_data,
            commands::audition_note,
            commands::list_presets,
            commands::save_preset,
            commands::update_preset,
            commands::delete_preset,
        ])
        .run(tauri::generate_context!())
        .expect("error while running daw-party");
}
