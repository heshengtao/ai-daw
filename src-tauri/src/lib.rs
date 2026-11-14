#![forbid(unsafe_code)]

mod tauri_host;
mod web;


use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use tauri::{Emitter, Manager};

use ai_daw_engine::registry::PluginSlot;
use ai_daw_host_vst3::EditorHandle;

use ai_daw_daemon::audio::{AudioEngine, TransportSnapshot};
use ai_daw_daemon::daemon::{Config as DaemonConfig, Daemon};
use ai_daw_daemon::state::{AppState, EVENT_TRANSPORT};

/// 传输位置的推送周期：跟屏幕刷新对齐（约 60 Hz）。
///
/// 原来是 33 ms（30 Hz），注释写着「看起来连续」——在 60 Hz 屏上并不成立：播放头每秒只走
/// 30 步，放大后一步好几像素，滚起来能明显看出台阶。快照本身只是几个数字，一次
/// `emit` 的开销远小于一帧，翻倍到 60 Hz 比「前端插值猜位置」简单，也不会漂。
/// 暂停后位置不再变化，`last == snapshot` 会直接跳过，所以静止时没有任何额外推送。
const TRANSPORT_TICK: Duration = Duration::from_millis(16);

/// 插件窗口的轮询周期：把插件发起的窗口事件送进去，并清理被用户关掉的窗口。
const EDITOR_TICK: Duration = Duration::from_millis(80);

/// 进程入口时刻：所有「启动到哪一步花了多久」都以此为基准写进日志。
static BOOT: OnceLock<Instant> = OnceLock::new();

/// 距进程入口（`run()` 第一行）过了多少毫秒。
pub(crate) fn boot_ms() -> u128 {
    BOOT.get().map(|t| t.elapsed().as_millis()).unwrap_or(0)
}

/// 创建主窗口，去掉系统自绘的标题栏：
/// - macOS：Overlay 风格（保留原生红黄绿，去掉系统标题栏，内容延伸到顶部）
/// - Windows / Linux：完全无边框，窗口按钮由前端自绘
fn build_main_window(app: &tauri::AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    let config = &app.config().app.windows[0];
    let mut builder =
        tauri::WebviewWindowBuilder::from_config(app, config)?.on_page_load(|window, payload| {
            let stage = match payload.event() {
                tauri::webview::PageLoadEvent::Started => "开始加载",
                tauri::webview::PageLoadEvent::Finished => "加载完成",
            };
            ai_daw_daemon::logging::info(format!(
                "[startup] webview {stage}（进程入口 +{} ms）",
                boot_ms()
            ));
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                // 页面内容已经就绪，窗口可以出来了（`visible: false` 见 tauri.conf.json：
                // 建窗到这一刻之前窗口是隐藏的，用户看不到那块「只有底色」的黑屏）。
                // **音频宿主不在这里启动**：装载要解码工程里的全部素材（实测 4–21 秒），
                // 和首帧抢 CPU/磁盘会把「窗口出来」再推后十几秒。改由前端的
                // [`app_ready`] 在挂载之后触发。
                let _ = window.show();
            }
        });

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
            // 先等首次装载把宿主建起来：扫描会起一串探针子进程、大量读盘，和首屏 /
            // 首次发声抢 CPU 只会把启动拖得更长。目录已经用缓存装好了，晚几秒扫描
            // 不影响可用性；最多等 5 秒（没有音频设备时也照常往下走）。
            for _ in 0..50 {
                if app.state::<Arc<AppState>>().audio.snapshot().available {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
            ai_daw_daemon::manager::rescan_with_managed(&app);
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
    let Some(key) = ai_daw_daemon::commands::host_window_track(window.label()) else {
        return;
    };
    let state = window.app_handle().state::<AppState>();
    if let Some(plugin_window) = ai_daw_daemon::state::with_editor_windows(|windows| windows.remove(&key)) {
        plugin_window.close();
    }
    state.open_editors.lock().unwrap().remove(&key);
    state.host_windows.lock().unwrap().remove(&key);
}

/// 启动插件窗口轮询线程：窗口级的事件（缩放、DPI）必须在**开窗口的那条线程**上泵进插件。
///
/// VST3 的窗口按**模块**分线程（见 `host_vst3::plugin_thread`），CLAP / AU 的窗口住在主线程。
/// 每拍给「有实例的模块」各排一次服务任务——**不等待**：某个模块的插件卡住时，它那条线程上的
/// 任务会排队/跳过，别的模块与主线程照常跑。
fn spawn_editor_ticker(app: tauri::AppHandle) {
    thread::Builder::new()
        .name("ai-daw-editors".into())
        .spawn(move || loop {
            thread::sleep(EDITOR_TICK);

            let modules: Vec<std::path::PathBuf> = {
                let state = app.state::<Arc<AppState>>();
                let handles = state.editor_handles.lock().unwrap();
                let mut modules: Vec<std::path::PathBuf> = handles
                    .values()
                    .filter_map(|slot| slot.module.clone())
                    .collect();
                modules.sort();
                modules.dedup();
                modules
            };
            for module in modules {
                let app_for_plugin = app.clone();
                ai_daw_host_vst3::post_service(&module, move || {
                    service_editor_windows(&app_for_plugin, true);
                });
            }

            let app_for_main = app.clone();
            let dispatched =
                app.run_on_main_thread(move || service_editor_windows(&app_for_main, false));
            if dispatched.is_err() {
                // 主窗口已销毁，收工。
                break;
            }
        })
        .expect("failed to spawn editor ticker");
}

/// 在**当前线程**上服务归它管的那批插件窗口。
///
/// `plugin_thread` 只用来区分「这一趟是不是插件线程那趟」：CLAP 的 `request_callback`
/// 必须在主线程上服务，与窗口是否打开无关。
fn service_editor_windows(app: &tauri::AppHandle, plugin_thread: bool) {
    let state = app.state::<Arc<AppState>>();
    let hosts = state.host_windows.clone();
    // 需要连带关掉的宿主窗口，等到借用结束之后再关（见下面的说明）。
    let mut close_hosts: Vec<(u64, PluginSlot)> = Vec::new();
    let mut doomed: Vec<(u64, PluginSlot)> = Vec::new();

    ai_daw_daemon::state::with_editor_windows(|windows| {
        for window in windows.values() {
            window.service();
        }
        // 结束的两种情况：插件自己关了窗口；或者我们给「嵌入模式」插件开的那扇
        // 宿主窗口被用户关掉了（那时插件不会通知我们）。
        doomed = windows
            .iter()
            .filter(|(key, w)| {
                if w.closed_by_user() {
                    return true;
                }
                let is_host = hosts.lock().unwrap().contains(*key);
                is_host
                    && app
                        .get_webview_window(&ai_daw_daemon::commands::host_window_label(key.0, key.1))
                        .is_none()
            })
            .map(|(key, _)| *key)
            .collect();
        for key in &doomed {
            // 必须走 `close()` 而不是直接丢：它先 hide + destroy 插件的界面。
            // 直接丢会把插件的原生视图留成悬空指针，之后一碰就崩。
            if let Some(window) = windows.remove(key) {
                window.close();
            }
            // 界面住在宿主窗口里的话，那扇窗也要收掉；插件自己关了界面时它
            // 更会剩在那里变成一扇空窗。
            if hosts.lock().unwrap().remove(key) {
                close_hosts.push(*key);
            }
        }
    });

    // 关宿主窗口必须在 `with_editor_windows` 的借用**之外**做：关窗会触发
    // `CloseRequested`，我们的处理器又要去借同一个 RefCell——嵌套借用会 panic。
    for key in close_hosts {
        if let Some(window) =
            app.get_webview_window(&ai_daw_daemon::commands::host_window_label(key.0, key.1))
        {
            let _ = window.close();
        }
    }

    // 镜像只清这一趟负责的 key——另一个模块线程的窗口不归这里管。
    {
        let mut open = state.open_editors.lock().unwrap();
        let mut meta = state.editor_plugins.lock().unwrap();
        let mut native = state.editor_windows.lock().unwrap();
        for key in &doomed {
            open.remove(key);
            meta.remove(key);
            native.remove(key);
        }
    }

    if !plugin_thread {
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
    }
}

/// 预设文件的位置：应用配置目录下的 `presets.json`。
///
/// 拿不到目录时返回 `None`（预设仍能用，只是重启后不保留），不因此启动失败。
fn preset_file(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    ai_daw_daemon::paths::config_dir(app)
        .ok()
        .map(|dir| dir.join("presets.json"))
}

/// 音色记忆库文件的位置：应用配置目录下的 `sound-memories.json`。
fn sound_memory_file(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    ai_daw_daemon::paths::config_dir(app)
        .ok()
        .map(|dir| dir.join("sound-memories.json"))
}

/// MCP 服务器配置文件的位置：**系统默认**配置目录下的 `mcp.json`。
///
/// `mcp-url` 与 `cli-path` 都是给外部工具按 bundle id 找的发现文件，必须留在默认位置，
/// 不能跟着数据目录搬走（它们只有几十字节，留在系统盘无所谓）。
fn mcp_file(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    ai_daw_daemon::paths::default_config_dir(app).map(|dir| dir.join("mcp.json"))
}

/// 写歌智能体配置文件的位置：应用配置目录下的 `ai.json`。
fn agent_file(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    ai_daw_daemon::paths::config_dir(app)
        .ok()
        .map(|dir| dir.join("ai.json"))
}

/// 写歌智能体会话目录：应用配置目录下的 `agent/sessions/`。
fn agent_sessions_dir(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    ai_daw_daemon::paths::config_dir(app)
        .ok()
        .map(|dir| dir.join("agent").join("sessions"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = BOOT.set(Instant::now());
    // 数据目录必须在**任何路径解析之前**定下来：日志、会话、插件缓存都按它落盘。
    ai_daw_daemon::paths::init_from_args();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        // 自动更新：`updater` 负责检查 / 下载 / 验签安装，`process` 负责装完重启。
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .on_window_event(|window, event| on_host_window_closing(window, event))
        .setup(|app| {
            // 启动耗时：`setup` 跑完才进事件循环，这一段越久，首屏越像「黑屏卡住」。
            let setup_started = std::time::Instant::now();
            // 日志要最先装上：启动阶段的崩溃也得留下线索。
            ai_daw_daemon::logging::init(app.handle());
            // 数据目录迁移后，旧工程文件里的素材引用还要能找到新位置。
            ai_daw_daemon::paths::init_remap(app.handle());
            ai_daw_daemon::logging::info(format!(
                "[startup] setup 开始（进程入口 +{} ms）",
                boot_ms()
            ));
            // 宿主 crate 的日志（哪个插件超时、哪条模块线程卡住）一并进同一份日志。
            ai_daw_host_vst3::set_log_sink(|line| ai_daw_daemon::logging::info(format!("[host] {line}")));
            // 「加载未响应」的插件隔离名单：下次启动不再为它等一次超时。
            ai_daw_daemon::quarantine::init(
                ai_daw_daemon::paths::config_dir(app.handle())
                    .ok()
                    .map(|dir| dir.join("plugin-quarantine.json")),
            );
            // CLAP 插件会问宿主「这是主线程吗」——必须在真正的主线程上登记一次。
            ai_daw_host_vst3::mark_main_thread();
            // 还得给它一个「把活派到主线程」的手：CLAP 的 init / 参数枚举 / activate 都只能
            // 在主线程做，而这些调用发生在音频线程构建 Player 的时候。
            ai_daw_daemon::main_thread::TauriMainThread::install(app.handle());
            // 会话缓存与插件目录都要在**建状态之前**读出来：`AppState::bootstrap` 会让音频线程
            // 一次性装载「最终工程 + 已缓存目录」。以前是常驻的默认工程先装一遍、会话再装一遍、
            // 目录到位又各重建一次——默认模板的采样音色解码就白白吃掉启动时最显眼的几秒。
            let session = ai_daw_daemon::paths::session_file(app.handle()).and_then(|file| ai_daw_daemon::session::read_session(&file));
            let cached = ai_daw_daemon::manager::cached_catalog(app.handle());
            // Web 访问配置：设置页可切换局域网绑定；端口默认 28430。
            // 与 `aidawd` 共用同一份配置；服务本身不鉴权（取决于部署侧的网关/网络）。
            let web_config = crate::web::load_config(app.handle());
            app.manage(crate::web::WebState::default());
            // 后端从此刻起就是"daemon"：状态 + 宿主（Tauri 实现）+ 配置。
            // 网页客户端连的是同一个 `Daemon`，所以两边看到的是同一份工程、同一个音频宿主。
            let host = Arc::new(crate::tauri_host::TauriHost::new(app.handle().clone()));
            let state = AppState::bootstrap_with_host(session, cached, host.clone());
            let daemon = Daemon::new(
                Arc::new(state),
                host,
                DaemonConfig {
                    data_dir: app.path().app_data_dir().unwrap_or_default(),
                    lan: web_config.lan,
                    port: Some(web_config.port),
                    // 桌面端托管的是同一份 `dist/`：浏览器打开端口就是同一个界面。
                    web_dir: Some(app.path().resource_dir().unwrap_or_default().join("web")),
                },
            );
            app.manage(Arc::clone(&daemon));
            app.manage(Arc::clone(daemon.state_arc()));
            ai_daw_daemon::logging::info(format!(
                "[startup] 会话与插件缓存装载完成（进程入口 +{} ms）",
                boot_ms()
            ));
            // 插件线程不需要预先启动：每个 VST3 模块在第一次用到时各自起一条（带消息循环），
            // 于是一个插件卡住只影响它自己。见 `host_vst3::plugin_thread`。
            build_main_window(app.handle())?;
            ai_daw_daemon::logging::info(format!(
                "[startup] 主窗口创建完成（进程入口 +{} ms）",
                boot_ms()
            ));
            let state = app.state::<Arc<AppState>>();
            state.init_presets(preset_file(app.handle()));
            state.init_sound_memories(sound_memory_file(app.handle()));
            drop(state);
            // 插件源的网络配置（镜像 / 下载代理）：要在预热线程之前装上。
            app.manage(ai_daw_daemon::manager::StoreRuntime::load(app.handle()));
            // 执行后端：CoreML 编译缓存目录 + 按 store.json 的偏好落地 + 校验并打印一行。
            // **必须在任何 ort 调用之前**（只决定「用哪个 EP」，不触发加载，所以放这儿安全）。
            ai_daw_daemon::backends::init_runtime(app.handle());
            // 传输状态推送已搬进 daemon（浏览器客户端也要收），这里不再重复起线程。
            // 正常的装载由前端的 `app_ready`（挂载之后的第一帧）触发。
            // 页面万一根本没起来（dev server 没起、页面脚本报错），也不能永远不出窗口、
            // 永远没声音：留一个 6 秒的兜底，超时就直接显示窗口并装载。
            // （以前这里是 3 秒就装载音频，而 WebView 起得比 3 秒慢时它几乎每次都会抢跑，
            //  正是「打开应用黑屏十几秒」的来源之一。）
            {
                let handle = app.handle().clone();
                let _ = thread::Builder::new()
                    .name("ai-daw-first-load-fallback".into())
                    .spawn(move || {
                        thread::sleep(Duration::from_secs(6));
                        if let Some(window) = handle.get_webview_window("main") {
                            let _ = window.show();
                        }
                        handle.state::<Arc<AppState>>().start_audio();
                    });
            }
            // 插件目录已随 `AppState::bootstrap` 交给音频线程了；这里只启动后台扫描，
            // 扫描结果与缓存一致时 `install_catalog` 不会再重建宿主（见那里的说明）。
            spawn_plugin_scan(app.handle().clone());
            spawn_editor_ticker(app.handle().clone());
            // 后台预热插件注册表缓存：用户点开「插件管理器 → 可下载」时通常已命中缓存、秒开。
            {
                let handle = app.handle().clone();
                let _ = thread::Builder::new()
                    .name("ai-daw-store-warm".into())
                    .spawn(move || ai_daw_daemon::manager::warm_registry_cache(&handle));
            }
            // 内嵌 MCP 服务器：把 DAW 能力暴露给 AI 代理（只监听回环）。按设置启动。
            let mcp = ai_daw_daemon::mcp::McpRuntime::load(app.handle().clone(), mcp_file(app.handle()));
            mcp.autostart();
            app.manage(mcp);
            // 写歌智能体：OpenAI 兼容端点配置（`ai.json`）+ 会话运行时（`agent/sessions/`）。
            app.manage(ai_daw_daemon::agent::AgentConfigRuntime::load(agent_file(
                app.handle(),
            )));
            app.manage(ai_daw_daemon::agent::AgentRuntime::new(agent_sessions_dir(
                app.handle(),
            )));
            // 注意：`aidaw` CLI 的挂载**不做任何自动动作**——它是设置页里的一个显式开关
            // （见 `cli.rs`）。启动时碰 PATH 会平白弹一次密码框，体验很差。
            // 但要**登记自己在哪**：别的 AI harness 按 bundle id 推导配置目录、读 `cli-path`
            // 就能拿到全路径，不必搜索、也不必用户开挂载开关。
            if let Some(dir) = ai_daw_daemon::paths::default_config_dir(app.handle()) {
                ai_daw_daemon::cli::publish_path(&dir);
            }
            // 让模型层知道去哪找权重：**下载落点**（应用配置目录的 models/）与**随包小模型**
            // （打包后的资源目录 models/）。本 crate 不依赖 Tauri，所以由宿主在这里登记。
            {
                let mut extra: Vec<std::path::PathBuf> = Vec::new();
                if let Ok(dir) = app.path().resource_dir() {
                    extra.push(dir.join("models"));
                }
                if let Ok(dir) = ai_daw_daemon::paths::config_dir(app.handle()) {
                    extra.push(dir.join("models"));
                }
                ai_daw_local_ai::models::set_extra_dirs(extra);
            }
            // 已下线模型（清单里没有、权重可能还在盘上）：顺手收掉，别让它们变成
            // 管理器里找不到入口的无主文件（如抓歌词的 SenseVoice-Small）。
            ai_daw_daemon::models::purge_retired(app.handle());
            // 诊断开关：`AIDAW_EDITOR_SELFTEST` 驱动一条不依赖鼠标的插件界面回归链路。
            //
            // - `1`      启动 12 秒后给 1 号轨道开一次界面（默认）；
            // - `cycle`  开 → 等 2 秒 → 关；
            // - `switch` RdPiano ↔ ACE 反复切换，**每次先关界面再切**（旧路径）；
            // - `stress` RdPiano ↔ ACE 反复切换，**界面一直开着**——用户报的那条路径：
            //            换乐器的同时要收旧窗、建新实例、开新窗。
            //
            // 「打开插件界面卡死 / 一个坏插件拖垮全部」在无头环境里很难复现，用它可以一键走完
            // 同一条链路：Tauri 命令 → 插件线程 → 插件的 `IPlugView::attached`。
            if let Ok(mode) = std::env::var("AIDAW_EDITOR_SELFTEST") {
                let handle = app.handle().clone();
                let _ = thread::Builder::new()
                    .name("editor-selftest".into())
                    .spawn(move || {
                        std::thread::sleep(Duration::from_secs(12));

                        let open = |title: &str| {
                            let state = handle.state::<Arc<AppState>>();
                            ai_daw_daemon::commands::open_plugin_window(
                                handle.clone(),
                                state,
                                1,
                                title.to_string(),
                            )
                        };
                        let close = || ai_daw_daemon::commands::close_plugin_window(handle.clone(), 1);
                        let select = |plugin_id: &str| {
                            let state = handle.state::<Arc<AppState>>();
                            ai_daw_daemon::commands::set_track_instrument(
                                handle.clone(),
                                state,
                                1,
                                plugin_id.to_string(),
                            );
                        };

                        if mode == "switch" || mode == "stress" {
                            let keep_open = mode == "stress";
                            let plugins = [
                                ("RdPiano", "vst3:ABCDEF019182FAEB476C5A735244504E"),
                                ("ACE", "vst3:ABCDEF019182FAEB546D646D42726467"),
                            ];
                            for round in 0..4 {
                                for (name, id) in plugins {
                                    let at = std::time::Instant::now();
                                    select(id);
                                    eprintln!(
                                        "[{mode} r{round}] {name} select in {:?}",
                                        at.elapsed()
                                    );
                                    std::thread::sleep(Duration::from_secs(1));
                                    let at = std::time::Instant::now();
                                    let opened = open(name);
                                    eprintln!(
                                        "[{mode} r{round}] {name} open -> {opened:?} in {:?}",
                                        at.elapsed()
                                    );
                                    if !keep_open {
                                        std::thread::sleep(Duration::from_millis(1500));
                                        let closed = close();
                                        eprintln!("[{mode} r{round}] {name} close -> {closed:?}");
                                    }
                                }
                            }
                            return;
                        }

                        let at = std::time::Instant::now();
                        let result = open("selftest");
                        eprintln!("[selftest] open -> {result:?} in {:?}", at.elapsed());
                        if mode == "cycle" {
                            std::thread::sleep(Duration::from_secs(2));
                            let closed = close();
                            eprintln!("[selftest] close -> {closed:?}");
                        }
                    });
            }
            // 这一段决定了「窗口出来之前黑了多久」：超过 ~300 ms 就值得查是谁拖的。
            ai_daw_daemon::logging::info(format!(
                "[startup] setup 完成，用时 {} ms（进程入口 +{} ms）",
                setup_started.elapsed().as_millis(),
                boot_ms()
            ));
            // 浏览器客户端：与桌面共用同一个 Daemon（同一份工程、同一个音频宿主）。
            // 绑定默认只回环；设置页允许局域网访问时才对内网开放。
            {
                let daemon_for_web = std::sync::Arc::clone(&daemon);
                let handle = app.handle().clone();
                // 浏览器发来的命令要在本进程里执行，所以要带上真实句柄。
                let rpc_handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    match ai_daw_daemon::boot::serve_with_app(daemon_for_web, Some(rpc_handle))
                        .await
                    {
                        Ok((addr, fut)) => {
                            let mut running =
                                crate::web::info_for(&crate::web::load_config(&handle));
                            running.port = addr.port();
                            running.running = true;
                            crate::web::mark_running(&handle, running);
                            ai_daw_daemon::logging::info(format!("[web] 监听 http://{addr}"));
                            fut.await;
                        }
                        Err(err) => {
                            ai_daw_daemon::logging::info(format!("[web] 启动失败：{err}"))
                        }
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ai_daw_daemon::commands::app_ready,
            crate::web::web_access_info,
            crate::web::web_access_set_lan,
            ai_daw_daemon::paths::data_dir_info,
            ai_daw_daemon::paths::data_dir_migrate,
            ai_daw_daemon::paths::data_dir_cleanup_previous,
            ai_daw_daemon::commands::get_project,
            ai_daw_daemon::commands::project_revision,
            ai_daw_daemon::commands::add_track,
            ai_daw_daemon::commands::remove_track,
            ai_daw_daemon::commands::rename_track,
            ai_daw_daemon::commands::set_track_instrument,
            ai_daw_daemon::commands::set_param,
            ai_daw_daemon::commands::set_params,
            ai_daw_daemon::commands::set_tempo,
            ai_daw_daemon::commands::set_time_signature,
            ai_daw_daemon::commands::set_keyframe,
            ai_daw_daemon::commands::remove_keyframe,
            ai_daw_daemon::commands::new_project,
            ai_daw_daemon::commands::rename_project,
            ai_daw_daemon::commands::localize_starter_tracks,
            ai_daw_daemon::commands::current_project_path,
            ai_daw_daemon::commands::session_restored,
            ai_daw_daemon::commands::cache_session,
            ai_daw_daemon::commands::project_history,
            ai_daw_daemon::commands::delete_project_history,
            ai_daw_daemon::commands::restore_project_history,
            ai_daw_daemon::commands::add_clip,
            ai_daw_daemon::commands::remove_clip,
            ai_daw_daemon::commands::move_clip,
            ai_daw_daemon::commands::move_clips,
            ai_daw_daemon::commands::resize_clip,
            ai_daw_daemon::commands::set_clip_rate,
            ai_daw_daemon::commands::trim_clip,
            ai_daw_daemon::commands::split_clip,
            ai_daw_daemon::commands::add_note,
            ai_daw_daemon::commands::set_note,
            ai_daw_daemon::commands::set_notes,
            ai_daw_daemon::commands::remove_note,
            ai_daw_daemon::commands::remove_notes,
            ai_daw_daemon::commands::remove_clips,
            ai_daw_daemon::commands::add_notes,
            ai_daw_daemon::commands::replace_notes,
            ai_daw_daemon::commands::update_notes,
            ai_daw_daemon::commands::remove_notes_by_id,
            ai_daw_daemon::commands::transpose_notes,
            ai_daw_daemon::commands::set_note_velocity,
            ai_daw_daemon::commands::quantize_notes,
            ai_daw_daemon::commands::strum_notes,
            ai_daw_daemon::commands::apply_swing,
            ai_daw_daemon::commands::humanize_notes,
            ai_daw_daemon::commands::fit_notes_to_range,
            ai_daw_daemon::commands::duplicate_clip,
            ai_daw_daemon::commands::merge_clips,
            ai_daw_daemon::commands::get_track_note_stats,
            ai_daw_daemon::commands::analyze_track_overlap,
            ai_daw_daemon::commands::analyze_harmony,
            ai_daw_daemon::commands::set_project_harmony,
            ai_daw_daemon::commands::set_project_sections,
            ai_daw_daemon::commands::set_track_meta,
            ai_daw_daemon::commands::set_mpe,
            ai_daw_daemon::commands::set_midi_events,
            ai_daw_daemon::commands::set_articulations,
            ai_daw_daemon::commands::list_articulation_maps,
            ai_daw_daemon::commands::import_articulation_map,
            ai_daw_daemon::commands::set_note_tag,
            ai_daw_daemon::commands::set_note_tags,
            ai_daw_daemon::commands::track_midi_capabilities,
            ai_daw_daemon::commands::set_gain,
            ai_daw_daemon::commands::set_pan,
            ai_daw_daemon::commands::set_mute,
            ai_daw_daemon::commands::set_solo,
            ai_daw_daemon::commands::set_master_gain,
            ai_daw_daemon::commands::set_sends,
            ai_daw_daemon::commands::set_automation,
            ai_daw_daemon::commands::monitor_track_mix,
            ai_daw_daemon::commands::history_depths,
            ai_daw_daemon::commands::undo,
            ai_daw_daemon::commands::redo,
            ai_daw_daemon::commands::ops_list,
            ai_daw_daemon::commands::ops_revert_to,
            ai_daw_daemon::commands::ops_reapply_to,
            ai_daw_daemon::commands::transport_status,
            ai_daw_daemon::commands::retry_audio,
            ai_daw_daemon::commands::list_output_devices,
            ai_daw_daemon::commands::set_output_device,
            ai_daw_daemon::commands::transport_play,
            ai_daw_daemon::commands::transport_pause,
            ai_daw_daemon::commands::transport_stop,
            ai_daw_daemon::commands::transport_seek,
            ai_daw_daemon::commands::transport_set_loop,
            ai_daw_daemon::commands::transport_set_loop_range,
            ai_daw_daemon::commands::transport_set_metronome,
            ai_daw_daemon::commands::save_project,
            ai_daw_daemon::commands::export_score,
            ai_daw_daemon::commands::export_midi,
            ai_daw_daemon::commands::import_project,
            ai_daw_daemon::commands::read_import_file,
            ai_daw_daemon::commands::open_project,
            ai_daw_daemon::commands::render_preview,
            ai_daw_daemon::commands::analyze_project,
            ai_daw_daemon::commands::export_audio,
            ai_daw_daemon::commands::import_audio,
            ai_daw_daemon::commands::add_audio_clip,
            ai_daw_daemon::commands::audio_waveform,
            ai_daw_daemon::commands::clip_bounds,
            ai_daw_daemon::commands::list_input_devices,
            ai_daw_daemon::commands::set_input_device,
            ai_daw_daemon::commands::input_level,
            ai_daw_daemon::commands::list_midi_inputs,
            ai_daw_daemon::commands::set_midi_input,
            ai_daw_daemon::commands::midi_input_device,
            ai_daw_daemon::commands::add_midi_clip,
            ai_daw_daemon::commands::record_start,
            ai_daw_daemon::commands::record_stop,
            ai_daw_daemon::commands::list_plugins,
            ai_daw_daemon::commands::plugin_scan_status,
            ai_daw_daemon::plugin_dirs::plugin_scan_dirs,
            ai_daw_daemon::plugin_dirs::plugin_set_scan_dirs,
            ai_daw_daemon::commands::rescan_plugins,
            ai_daw_daemon::commands::plugin_params,
            ai_daw_daemon::commands::add_effect,
            ai_daw_daemon::commands::remove_effect,
            ai_daw_daemon::commands::move_effect,
            ai_daw_daemon::commands::set_effect_param,
            ai_daw_daemon::commands::open_plugin_window,
            ai_daw_daemon::commands::close_plugin_window,
            ai_daw_daemon::commands::plugin_window_open,
            ai_daw_daemon::commands::open_synth_window,
            ai_daw_daemon::commands::open_effect_window,
            ai_daw_daemon::commands::open_effect_plugin_window,
            ai_daw_daemon::commands::close_effect_plugin_window,
            ai_daw_daemon::commands::effect_plugin_window_open,
            ai_daw_daemon::commands::set_effect_params,
            ai_daw_daemon::commands::set_effect_enabled,
            ai_daw_daemon::commands::set_effect_sidechain,
            ai_daw_daemon::commands::add_master_effect,
            ai_daw_daemon::commands::remove_master_effect,
            ai_daw_daemon::commands::move_master_effect,
            ai_daw_daemon::commands::set_master_effect_param,
            ai_daw_daemon::commands::set_master_effect_enabled,
            ai_daw_daemon::commands::set_master_effect_sidechain,
            ai_daw_daemon::commands::mixer_meters,
            ai_daw_daemon::commands::clear_meter_clip,
            ai_daw_daemon::commands::scope_data,
            ai_daw_daemon::commands::audition_note,
            ai_daw_daemon::commands::audition_midi_events,
            ai_daw_daemon::commands::list_presets,
            ai_daw_daemon::commands::save_preset,
            ai_daw_daemon::commands::update_preset,
            ai_daw_daemon::commands::delete_preset,
            ai_daw_daemon::commands::list_sound_memories,
            ai_daw_daemon::commands::save_sound_memory,
            ai_daw_daemon::commands::update_sound_memory,
            ai_daw_daemon::commands::delete_sound_memory,
            ai_daw_daemon::commands::apply_sound_memory,
            ai_daw_daemon::mcp::mcp_status,
            ai_daw_daemon::mcp::set_mcp_config,
            ai_daw_daemon::agent::agent_config,
            ai_daw_daemon::agent::agent_set_config,
            ai_daw_daemon::agent::agent_models,
            ai_daw_daemon::agent::agent_test,
            ai_daw_daemon::agent::agent_sessions,
            ai_daw_daemon::agent::agent_session,
            ai_daw_daemon::agent::agent_delete_session,
            ai_daw_daemon::agent::agent_set_head,
            ai_daw_daemon::agent::agent_send,
            ai_daw_daemon::agent::agent_cancel,
            ai_daw_daemon::agent::agent_running,
            ai_daw_daemon::cli::cli_status,
            ai_daw_daemon::cli::install_cli,
            ai_daw_daemon::cli::uninstall_cli,
            ai_daw_daemon::manager::plugin_manager_registry,
            ai_daw_daemon::manager::plugin_manager_installed,
            ai_daw_daemon::manager::plugin_manager_foreign,
            ai_daw_daemon::manager::plugin_manager_install,
            ai_daw_daemon::manager::plugin_manager_resolve_url,
            ai_daw_daemon::manager::plugin_manager_import,
            ai_daw_daemon::manager::plugin_manager_remove,
            ai_daw_daemon::manager::plugin_manager_open,
            ai_daw_daemon::manager::plugin_manager_network,
            ai_daw_daemon::manager::plugin_manager_set_network,
            ai_daw_daemon::manager::plugin_manager_probe_proxy,
            ai_daw_daemon::manager::plugin_manager_proxy_status,
            ai_daw_daemon::ai::ai_status,
            ai_daw_daemon::ai::ai_models,
            ai_daw_daemon::ai::ai_start,
            ai_daw_daemon::ai::ai_cancel,
            ai_daw_daemon::voices::voice_list,
            ai_daw_daemon::voices::voice_save,
            ai_daw_daemon::voices::voice_update,
            ai_daw_daemon::voices::voice_delete,
            ai_daw_daemon::singing::singing_models,
            ai_daw_daemon::singing::singing_status,
            ai_daw_daemon::singing::singing_render,
            ai_daw_daemon::singing::singing_cancel,
            ai_daw_daemon::svc::svc_models,
            ai_daw_daemon::svc::svc_status,
            ai_daw_daemon::svc::svc_render,
            ai_daw_daemon::svc::svc_cancel,
            ai_daw_daemon::models::model_catalog,
            ai_daw_daemon::models::model_download,
            ai_daw_daemon::manager::model_save_hf_token,
            ai_daw_daemon::manager::model_hf_token_status,
            ai_daw_daemon::models::model_cancel,
            ai_daw_daemon::models::model_remove,
            ai_daw_daemon::models::model_reveal,
            ai_daw_daemon::models::model_resolve_url,
            ai_daw_daemon::models::model_probe,
            ai_daw_daemon::paths::disk_free_bytes,
            ai_daw_daemon::backends::ai_backend_info,
            ai_daw_daemon::backends::ai_set_backend,
            ai_daw_daemon::runtimes::ai_runtime_catalog,
            ai_daw_daemon::runtimes::ai_runtime_discover,
            ai_daw_daemon::runtimes::ai_runtime_import,
            ai_daw_daemon::runtimes::ai_runtime_install,
            ai_daw_daemon::runtimes::ai_runtime_cancel,
            ai_daw_daemon::runtimes::ai_runtime_busy,
            ai_daw_daemon::runtimes::ai_runtime_remove,
            ai_daw_daemon::runtimes::ai_runtime_reveal,
            ai_daw_daemon::yue::ai_yue_catalog,
            ai_daw_daemon::yue::ai_yue_install,
            ai_daw_daemon::yue::ai_yue_import,
            ai_daw_daemon::yue::ai_yue_cancel,
            ai_daw_daemon::yue::ai_yue_remove,
            ai_daw_daemon::yue::ai_yue_reveal,
            ai_daw_daemon::yue::ai_yue_select,
            ai_daw_daemon::yue_engine::ai_yue_engine_status,
            ai_daw_daemon::yue_engine::ai_yue_engine_log,
            ai_daw_daemon::yue_engine::ai_yue_cuda_perf,
            ai_daw_daemon::yue_engine::ai_yue_set_cuda_perf,
            ai_daw_daemon::yue_engine::ai_yue_engine_start,
            ai_daw_daemon::yue_engine::ai_yue_engine_stop,
            ai_daw_daemon::yue_engine::ai_yue_compose,
            ai_daw_daemon::yue_engine::ai_yue_compose_cancel,
            ai_daw_daemon::yue_engine::ai_yue_import_abc,
            ai_daw_daemon::yue_engine::ai_yue_score,
            ai_daw_daemon::yue_engine::ai_yue_record,
            ai_daw_daemon::yue_engine::ai_yue_continuable,
            ai_daw_daemon::yue_engine::ai_yue_lora,
            ai_daw_daemon::yue_engine::ai_yue_set_lora,
            ai_daw_daemon::yue_transcribe::ai_yue_transcribe,
            ai_daw_daemon::yue_transcribe::ai_yue_transcribe_cancel,
            ai_daw_daemon::yue_midi::ai_yue_midi_abc,
            ai_daw_daemon::yue_lyrics::ai_yue_lyrics,
            ai_daw_daemon::yue_lyrics::ai_yue_lyrics_cancel,
        ])
        .build(tauri::generate_context!())
        .expect("error while building ai-daw");

    app.run(|app_handle, event| {
        // 进程级退出（程序坞「退出」/ Cmd+Q）走的是 `app.exit`，前端那道 `onCloseRequested`
        // 守卫不会触发——只靠 10 秒一次的自动保存，最近一次保存之后的改动（比如刚添加的
        // 效果器）会整段丢失。这里由后端兜底再存一次会话缓存。
        if let tauri::RunEvent::ExitRequested { .. } = event {
            if let Err(err) = ai_daw_daemon::commands::write_session(app_handle) {
                eprintln!("退出时写会话缓存失败：{err}");
            }
            // 会话缓存之外再留一份历史：下次打开即使点了「新建工程」，也能从历史回去。
            ai_daw_daemon::commands::remember_before_exit(app_handle);
            // 别把 yue-server 留在后台占着几个 GB 显存。
            ai_daw_daemon::yue_engine::shutdown_on_exit();
        }
    });
}

/// 桌面端要从哪里托管前端产物。
///
/// 两种来源，按优先级：
/// 1. **打包后**：资源目录里的 `web/`（`tauri.conf.json` 把 `../dist` 映射到它）；
/// 2. **开发期**：仓库根目录的 `dist/`（`pnpm build` 的产物）。
///
/// 都找不到就返回 `None`——服务只提供 `/rpc`，浏览器打开会明确提示没有界面，
/// 而不是给一个 404。
fn resolve_web_dir(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    if let Ok(dir) = app.path().resource_dir() {
        let bundled = dir.join("web");
        if bundled.join("index.html").is_file() {
            return Some(bundled);
        }
    }
    let dev = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist");
    if dev.join("index.html").is_file() {
        return Some(dev);
    }
    None
}
