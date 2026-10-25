#![forbid(unsafe_code)]

mod agent;
mod ai;
mod articulation_map;
mod assets;
mod audio;
mod backends;
mod cli;
mod commands;
mod cue_sheet;
mod factory_presets;
mod history;
mod main_thread;
mod manager;
mod mcp;
mod models;
mod plugin_info;
mod plugins;
mod presets;
mod session;
mod sound_memories;
mod state;

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tauri::{Emitter, Manager};

use ai_daw_engine::registry::PluginSlot;
use ai_daw_host_vst3::EditorHandle;

use crate::audio::{AudioEngine, TransportSnapshot};
use crate::state::{AppState, EVENT_TRANSPORT};

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
        .name("ai-daw-transport".into())
        .spawn(move || {
            let mut last: Option<TransportSnapshot> = None;
            let mut warned = false;
            loop {
                thread::sleep(TRANSPORT_TICK);
                let snapshot = audio.snapshot();
                if last.as_ref() == Some(&snapshot) {
                    continue;
                }
                if app.emit(EVENT_TRANSPORT, &snapshot).is_err() {
                    // 建窗 / 关窗的那一瞬间，某个 webview 还没准备好，这一次广播会失败。
                    // **一次失败绝不能退出**：线程一退，所有窗口的传输状态就永久冻结了——
                    // 界面停在「正在播放」、播放头不动，实际早已没声，按播放键也没反应。
                    if !warned {
                        warned = true;
                        eprintln!("[transport] 状态广播失败（窗口正在建/销毁？），下一拍重试");
                    }
                    // 真的一个窗口都不剩了才收工。
                    if app.webview_windows().is_empty() {
                        break;
                    }
                    // 清掉基线，保证下一拍**重发**同一份快照，而不是被「没变化」跳过。
                    last = None;
                    continue;
                }
                warned = false;
                last = Some(snapshot);
            }
        })
        .expect("failed to spawn transport thread");
}

/// 启动插件扫描线程：结果写入共享状态（音频线程据此重建注册表）并广播给界面。
///
/// 扫描会 dlopen 每个插件，可能耗时数秒，所以放到后台，不阻塞窗口出现。
/// 一并扫描**插件管理器**的托管目录（它装/导入的插件在应用数据目录里）。
fn spawn_plugin_scan(app: tauri::AppHandle) {
    thread::Builder::new()
        .name("ai-daw-scan".into())
        .spawn(move || {
            crate::manager::rescan_with_managed(&app);
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
    let Some(key) = crate::commands::host_window_track(window.label()) else {
        return;
    };
    let state = window.app_handle().state::<AppState>();
    if let Some(plugin_window) = crate::state::with_editor_windows(|windows| windows.remove(&key)) {
        plugin_window.close();
    }
    state.open_editors.lock().unwrap().remove(&key);
    state.host_windows.lock().unwrap().remove(&key);
}

/// 启动插件窗口轮询线程：窗口级的事件（缩放、DPI）必须在主线程泵进插件。
fn spawn_editor_ticker(app: tauri::AppHandle) {
    thread::Builder::new()
        .name("ai-daw-editors".into())
        .spawn(move || loop {
            thread::sleep(EDITOR_TICK);
            let app_for_main = app.clone();
            let dispatched = app.run_on_main_thread(move || {
                let state = app_for_main.state::<AppState>();
                let mut open = state.open_editors.lock().unwrap();
                let hosts = state.host_windows.clone();
                // 需要连带关掉的宿主窗口，等到借用结束之后再关（见下面的说明）。
                let mut close_hosts: Vec<(u64, PluginSlot)> = Vec::new();
                crate::state::with_editor_windows(|windows| {
                    for window in windows.values() {
                        window.service();
                    }
                    // 结束的两种情况：插件自己关了窗口；或者我们给「嵌入模式」插件开的那扇
                    // 宿主窗口被用户关掉了（那时插件不会通知我们）。
                    let doomed: Vec<(u64, PluginSlot)> = windows
                        .iter()
                        .filter(|(key, w)| {
                            if w.closed_by_user() {
                                return true;
                            }
                            let is_host = hosts.lock().unwrap().contains(*key);
                            is_host
                                && app_for_main
                                    .get_webview_window(&crate::commands::host_window_label(
                                        key.0, key.1,
                                    ))
                                    .is_none()
                        })
                        .map(|(key, _)| *key)
                        .collect();
                    for key in doomed {
                        // 必须走 `close()` 而不是直接丢：它先 hide + destroy 插件的界面。
                        // 直接丢会把插件的原生视图留成悬空指针，之后一碰就崩。
                        if let Some(window) = windows.remove(&key) {
                            window.close();
                        }
                        // 界面住在宿主窗口里的话，那扇窗也要收掉；插件自己关了界面时它
                        // 更会剩在那里变成一扇空窗。
                        if hosts.lock().unwrap().remove(&key) {
                            close_hosts.push(key);
                        }
                    }
                    open.clear();
                    open.extend(windows.keys().copied());
                    // 窗口记录同样只保留活着的（用户自己关掉的那扇要一并忘掉）。
                    state
                        .editor_plugins
                        .lock()
                        .unwrap()
                        .retain(|key, _| windows.contains_key(key));
                });

                // 关宿主窗口必须在 `with_editor_windows` 的借用**之外**做：关窗会触发
                // `CloseRequested`，我们的处理器又要去借同一个 RefCell——嵌套借用会 panic。
                for key in close_hosts {
                    if let Some(window) = app_for_main
                        .get_webview_window(&crate::commands::host_window_label(key.0, key.1))
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
                    .map(|entry| entry.handle.clone())
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
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("presets.json"))
}

/// 音色记忆库文件的位置：应用配置目录下的 `sound-memories.json`。
fn sound_memory_file(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("sound-memories.json"))
}

/// MCP 服务器配置文件的位置：应用配置目录下的 `mcp.json`。
fn mcp_file(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("mcp.json"))
}

/// 会话缓存文件的位置：应用配置目录下的 `session.json`。
///
/// 它记录「上次退出时的工作状态」，供下次启动还原；拿不到配置目录时返回 `None`（不缓存）。
pub(crate) fn session_file(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("session.json"))
}

/// 历史工程目录：应用配置目录下的 `history/`。
///
/// 拿不到配置目录时返回 `None`（历史功能不可用，但不影响编辑）。
pub(crate) fn history_dir(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("history"))
}

/// 写歌智能体配置文件的位置：应用配置目录下的 `ai.json`。
fn agent_file(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("ai.json"))
}

/// 写歌智能体会话目录：应用配置目录下的 `agent/sessions/`。
fn agent_sessions_dir(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("agent").join("sessions"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        // 自动更新：`updater` 负责检查 / 下载 / 验签安装，`process` 负责装完重启。
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .on_window_event(|window, event| on_host_window_closing(window, event))
        .manage(AppState::new("untitled"))
        .setup(|app| {
            // CLAP 插件会问宿主「这是主线程吗」——必须在真正的主线程上登记一次。
            ai_daw_host_vst3::mark_main_thread();
            // 还得给它一个「把活派到主线程」的手：CLAP 的 init / 参数枚举 / activate 都只能
            // 在主线程做，而这些调用发生在音频线程构建 Player 的时候。
            crate::main_thread::TauriMainThread::install(app.handle());
            build_main_window(app.handle())?;
            // 会话缓存：把上次退出时的工作状态读回来（没有或损坏就保持默认新工程）。
            if let Some(file) = session_file(app.handle()) {
                if let Some(session) = session::read_session(&file) {
                    app.state::<AppState>().restore_session(session);
                }
            }
            let state = app.state::<AppState>();
            state.init_presets(preset_file(app.handle()));
            state.init_sound_memories(sound_memory_file(app.handle()));
            let audio = state.audio.clone();
            drop(state);
            // 插件源的网络配置（镜像 / 下载代理）：要在预热线程之前装上。
            app.manage(crate::manager::StoreRuntime::load(app.handle()));
            // 执行后端：CoreML 编译缓存目录 + 按 store.json 的偏好落地 + 校验并打印一行。
            // **必须在任何 ort 调用之前**（只决定「用哪个 EP」，不触发加载，所以放这儿安全）。
            crate::backends::init_runtime(app.handle());
            spawn_transport_ticker(app.handle().clone(), audio);
            // 先用上次成功扫描的缓存把外部插件装上：这一轮扫描要给每个 VST3 起一个探针
            // 子进程，可能很久，界面不该为此空着。后台扫描完成后会用最新结果覆盖。
            let cached = crate::manager::cached_catalog(app.handle());
            if !cached.is_empty() {
                app.state::<AppState>().install_catalog(cached);
            }
            spawn_plugin_scan(app.handle().clone());
            spawn_editor_ticker(app.handle().clone());
            // 后台预热插件注册表缓存：用户点开「插件管理器 → 可下载」时通常已命中缓存、秒开。
            {
                let handle = app.handle().clone();
                let _ = thread::Builder::new()
                    .name("ai-daw-store-warm".into())
                    .spawn(move || crate::manager::warm_registry_cache(&handle));
            }
            // 内嵌 MCP 服务器：把 DAW 能力暴露给 AI 代理（只监听回环）。按设置启动。
            let mcp = crate::mcp::McpRuntime::load(app.handle().clone(), mcp_file(app.handle()));
            mcp.autostart();
            app.manage(mcp);
            // 写歌智能体：OpenAI 兼容端点配置（`ai.json`）+ 会话运行时（`agent/sessions/`）。
            app.manage(crate::agent::AgentConfigRuntime::load(agent_file(
                app.handle(),
            )));
            app.manage(crate::agent::AgentRuntime::new(agent_sessions_dir(
                app.handle(),
            )));
            // 注意：`aidaw` CLI 的挂载**不做任何自动动作**——它是设置页里的一个显式开关
            // （见 `cli.rs`）。启动时碰 PATH 会平白弹一次密码框，体验很差。
            // 但要**登记自己在哪**：别的 AI harness 按 bundle id 推导配置目录、读 `cli-path`
            // 就能拿到全路径，不必搜索、也不必用户开挂载开关。
            if let Ok(dir) = app.path().app_config_dir() {
                crate::cli::publish_path(&dir);
            }
            // 让模型层知道去哪找权重：**下载落点**（应用配置目录的 models/）与**随包小模型**
            // （打包后的资源目录 models/）。本 crate 不依赖 Tauri，所以由宿主在这里登记。
            {
                let mut extra: Vec<std::path::PathBuf> = Vec::new();
                if let Ok(dir) = app.path().resource_dir() {
                    extra.push(dir.join("models"));
                }
                if let Ok(dir) = app.path().app_config_dir() {
                    extra.push(dir.join("models"));
                }
                ai_daw_local_ai::models::set_extra_dirs(extra);
            }
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
            commands::rename_project,
            commands::localize_starter_tracks,
            commands::current_project_path,
            commands::cache_session,
            commands::project_history,
            commands::delete_project_history,
            commands::restore_project_history,
            commands::add_clip,
            commands::remove_clip,
            commands::move_clip,
            commands::move_clips,
            commands::resize_clip,
            commands::set_clip_rate,
            commands::trim_clip,
            commands::split_clip,
            commands::add_note,
            commands::set_note,
            commands::set_notes,
            commands::remove_note,
            commands::remove_notes,
            commands::remove_clips,
            commands::add_notes,
            commands::replace_notes,
            commands::update_notes,
            commands::remove_notes_by_id,
            commands::transpose_notes,
            commands::set_note_velocity,
            commands::quantize_notes,
            commands::strum_notes,
            commands::apply_swing,
            commands::humanize_notes,
            commands::fit_notes_to_range,
            commands::duplicate_clip,
            commands::merge_clips,
            commands::get_track_note_stats,
            commands::analyze_track_overlap,
            commands::analyze_harmony,
            commands::set_project_harmony,
            commands::set_project_sections,
            commands::set_track_meta,
            commands::set_mpe,
            commands::set_midi_events,
            commands::set_articulations,
            commands::list_articulation_maps,
            commands::import_articulation_map,
            commands::set_note_tag,
            commands::set_note_tags,
            commands::track_midi_capabilities,
            commands::set_gain,
            commands::set_pan,
            commands::set_mute,
            commands::set_solo,
            commands::set_master_gain,
            commands::set_sends,
            commands::set_automation,
            commands::monitor_track_mix,
            commands::history_depths,
            commands::undo,
            commands::redo,
            commands::ops_list,
            commands::ops_revert_to,
            commands::ops_reapply_to,
            commands::transport_status,
            commands::retry_audio,
            commands::transport_play,
            commands::transport_pause,
            commands::transport_stop,
            commands::transport_seek,
            commands::transport_set_loop,
            commands::transport_set_loop_range,
            commands::transport_set_metronome,
            commands::save_project,
            commands::open_project,
            commands::render_preview,
            commands::analyze_project,
            commands::export_audio,
            commands::import_audio,
            commands::add_audio_clip,
            commands::audio_waveform,
            commands::clip_bounds,
            commands::list_input_devices,
            commands::set_input_device,
            commands::input_level,
            commands::record_start,
            commands::record_stop,
            commands::list_plugins,
            commands::plugin_scan_status,
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
            commands::open_effect_window,
            commands::open_effect_plugin_window,
            commands::close_effect_plugin_window,
            commands::effect_plugin_window_open,
            commands::set_effect_params,
            commands::scope_data,
            commands::audition_note,
            commands::audition_midi_events,
            commands::list_presets,
            commands::save_preset,
            commands::update_preset,
            commands::delete_preset,
            commands::list_sound_memories,
            commands::save_sound_memory,
            commands::update_sound_memory,
            commands::delete_sound_memory,
            commands::apply_sound_memory,
            mcp::mcp_status,
            mcp::set_mcp_config,
            agent::agent_config,
            agent::agent_set_config,
            agent::agent_models,
            agent::agent_test,
            agent::agent_sessions,
            agent::agent_session,
            agent::agent_delete_session,
            agent::agent_set_head,
            agent::agent_send,
            agent::agent_cancel,
            agent::agent_running,
            cli::cli_status,
            cli::install_cli,
            cli::uninstall_cli,
            manager::plugin_manager_registry,
            manager::plugin_manager_installed,
            manager::plugin_manager_foreign,
            manager::plugin_manager_install,
            manager::plugin_manager_resolve_url,
            manager::plugin_manager_import,
            manager::plugin_manager_remove,
            manager::plugin_manager_open,
            manager::plugin_manager_network,
            manager::plugin_manager_set_network,
            manager::plugin_manager_probe_proxy,
            manager::plugin_manager_proxy_status,
            ai::ai_status,
            ai::ai_models,
            ai::ai_start,
            ai::ai_cancel,
            models::model_catalog,
            models::model_download,
            models::model_cancel,
            models::model_remove,
            models::model_reveal,
            models::model_resolve_url,
            backends::ai_backend_info,
            backends::ai_set_backend,
        ])
        .build(tauri::generate_context!())
        .expect("error while building ai-daw");

    app.run(|app_handle, event| {
        // 进程级退出（程序坞「退出」/ Cmd+Q）走的是 `app.exit`，前端那道 `onCloseRequested`
        // 守卫不会触发——只靠 10 秒一次的自动保存，最近一次保存之后的改动（比如刚添加的
        // 效果器）会整段丢失。这里由后端兜底再存一次会话缓存。
        if let tauri::RunEvent::ExitRequested { .. } = event {
            if let Err(err) = commands::write_session(app_handle) {
                eprintln!("退出时写会话缓存失败：{err}");
            }
            // 会话缓存之外再留一份历史：下次打开即使点了「新建工程」，也能从历史回去。
            commands::remember_before_exit(app_handle);
        }
    });
}
