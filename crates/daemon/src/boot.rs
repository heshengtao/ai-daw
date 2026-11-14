//! 无窗口启动：会话、预设、音色记忆库，然后交给调用方决定听哪里。
//!
//! 桌面端的 `setup()` 与 `ai-daw` 都调这里；**桌面独有的部分**（建窗、原生插件窗口、
//! 更新器）留在外壳里，不进来。

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use tokio::net::TcpListener;

use crate::daemon::{Config, Daemon};
use crate::host::{Host, UiHost};
use crate::state::AppState;

/// 无窗口进程的宿主：有数据目录、没有原生能力。
pub struct HeadlessHost {
    dir: PathBuf,
}

impl HeadlessHost {
    /// 指定数据目录。
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self { dir: dir.as_ref().to_path_buf() }
    }
}

impl Host for HeadlessHost {
    fn emit_value(&self, event: &'static str, payload: Value) {
        // 无窗口进程没有 webview，但浏览器客户端要通过 SSE 收到同一条事件。
        crate::bus::publish(event, payload);
    }

    fn data_dir(&self) -> PathBuf {
        self.dir.clone()
    }

    fn ui(&self) -> Option<&dyn UiHost> {
        None
    }
}

/// 装配后端。**不含任何窗口、事件循环**，可以在任意线程上调用。
pub fn bootstrap(host: Arc<dyn Host>, cfg: Config) -> Arc<Daemon> {
    crate::paths::init_with_data_dir(cfg.data_dir.clone());
    let session = crate::session::read_session(&cfg.data_dir.join("session.json"));
    let state = Arc::new(AppState::bootstrap_with_host(
        session,
        Vec::new(),
        Arc::clone(&host),
    ));
    // HTTP 命令层按 Tauri 签名写的，靠这张托管表取状态；桌面端由 `app.manage()` 填。
    crate::tauri_api::share_rpc_state(&state);
    crate::tauri_api::share_headless_runtimes();
    // 桌面外壳在 `setup()` 里注册的目录，无头进程必须自己补一遍（见函数注释）。
    #[cfg(not(feature = "desktop"))]
    init_headless_dirs(&cfg.data_dir);
    // 用户的预设与音色记忆库是跨工程资产，按数据目录加载（拿不到就空着，不影响启动）。
    state.init_presets(Some(cfg.data_dir.join("presets.json")));
    state.init_sound_memories(Some(cfg.data_dir.join("sound-memories.json")));
    // 无窗口进程也要出声：浏览器监听（/audio）与导出都依赖实时宿主。
    // 容器里没有声卡时 `start_audio` 会自己降级为 idle，不会 panic；
    // 想彻底跳过可以用 `AIDAW_NO_AUDIO=1`。
    if std::env::var_os("AIDAW_NO_AUDIO").is_none() {
        state.start_audio();
    }
    let daemon = Daemon::new(state, host, cfg);
    spawn_transport_ticker(Arc::clone(&daemon));
    daemon
}

/// 无头进程的资源目录注册。
///
/// 桌面外壳在 `src-tauri/src/lib.rs` 的 `setup()` 里做这三件事，无头进程没有外壳，
/// 漏掉任何一件都会让"桌面能用的东西在浏览器/Docker 里悄悄失效"：
///
/// 1. **模型搜索目录**：`<data>/models` 是用户下载模型的落点。不注册的话，下载成功
///    也永远显示"未安装"（`set_extra_dirs` 是进程级只写一次，`models::installed`
///    只认注册过的目录）。
/// 2. **ONNX Runtime 根目录**：不注册就落到 `$TMPDIR/ai-daw-runtime/ort`，
///    容器重启即丢、`/data` 卷白白空着。
/// 3. **audio.cpp（YuE）引擎根目录**：同上，落到 `$TMPDIR/ai-daw-audiocpp`。
///
/// 随包资源（镜像里的 `models/`、`ort/`、`articulation-maps/`）走环境变量
/// `AIDAW_MODELS` / `AIDAW_ORT_BUNDLED` / `AIDAW_ARTICULATION_MAPS`，不需要这里注册。
#[cfg(not(feature = "desktop"))]
fn init_headless_dirs(data_dir: &Path) {
    let app = crate::tauri_api::rpc_app_handle();
    // 下载落点排最前：用户在应用内下的模型要能覆盖镜像自带的那一份。
    // 随包目录由 `AIDAW_MODELS_EXTRA` 传入（`:` 分隔的路径表，可以直接挂多个卷）。
    let mut dirs = vec![data_dir.join("models")];
    if let Some(extra) = std::env::var_os("AIDAW_MODELS_EXTRA") {
        dirs.extend(std::env::split_paths(&extra));
    }
    ai_daw_local_ai::models::set_extra_dirs(dirs);
    // 运行时根目录 + 后端偏好 + YuE 引擎目录；必须在任何 dylib 被加载之前调用。
    crate::backends::init_runtime(&app);

}

/// 起 HTTP 服务。返回实际绑定的地址与可被 `select!` 的前台 future。
///
/// `token` 为空串时**拒绝启动**：局域网内的裸 `/rpc` 等于把整台机器的文件读写交出去。
pub async fn serve(
    daemon: Arc<Daemon>,
) -> std::io::Result<(SocketAddr, impl std::future::Future<Output = ()>)> {
    // 无头专属的"后台服务"：桌面壳在 `setup()` 里自己 manage + autostart。
    // 放在 serve 而不是 bootstrap：autostart 要 `tokio::spawn`，而 `bootstrap` 可能被
    // 同步调用（测试里就是），那时还没有运行时。
    #[cfg(not(feature = "desktop"))]
    init_headless_services(&daemon);
    serve_with_app(daemon, None).await
}

/// 无头进程要常驻的后台服务（目前只有内嵌 MCP server）。
///
/// 桌面壳用的是真实 `AppHandle` + `app.manage()`；无头进程用空壳句柄 + 托管表，
/// 于是 `mcp_status` / `set_mcp_config` 这两条命令在容器里也能真跑。
#[cfg(not(feature = "desktop"))]
fn init_headless_services(daemon: &Daemon) {
    let app = crate::tauri_api::rpc_app_handle();
    let data_dir = daemon.config().data_dir.clone();
    // `--lan` 时 MCP 也绑 0.0.0.0：容器里发布端口后外部 MCP 客户端才连得进来。
    let mcp = crate::mcp::McpRuntime::load(
        app.clone(),
        Some(data_dir.join("mcp.json")),
        daemon.config().lan,
    );
    // 先启动、再登记：HTTP 服务还没起来，中间不会有命令看到"半个服务器"。
    mcp.autostart();
    crate::tauri_api::install_state(mcp);

    // 写歌智能体：配置（`ai.json`）+ 会话运行时（`agent/sessions/`）。
    // 桌面壳在 `setup()` 里 `app.manage(...)`；无头进程要自己装进托管表，否则
    // AI 面板在浏览器里永远显示"未配置"、发送后一直转圈。
    // 工具调用走的是 `mcp::tools::dispatch`（与 MCP 同一份实现），不经过 webview。
    crate::tauri_api::install_state(crate::agent::AgentConfigRuntime::load(Some(
        data_dir.join("ai.json"),
    )));
    crate::tauri_api::install_state(crate::agent::AgentRuntime::new(Some(
        data_dir.join("agent").join("sessions"),
    )));
}

/// 起 HTTP 服务，并把宿主句柄交给命令层（桌面端用它替 web 客户端执行命令）。
///
/// 无头进程传 `None`：命令层改用空壳句柄 + 全局托管表（见 `tauri_api::share_rpc_state`）。
pub async fn serve_with_app(
    daemon: Arc<Daemon>,
    app: Option<crate::tauri_api::AppHandle>,
) -> std::io::Result<(SocketAddr, impl std::future::Future<Output = ()>)> {
    let cfg = daemon.config().clone();
    let port = cfg.port.unwrap_or(28430);
    let ip = if cfg.lan {
        std::net::Ipv4Addr::UNSPECIFIED
    } else {
        std::net::Ipv4Addr::LOCALHOST
    };
    let listener = TcpListener::bind((std::net::IpAddr::V4(ip), port)).await?;
    let addr = listener.local_addr()?;
    let static_dir = daemon.config().web_dir.clone();
    let router = crate::server::router(Arc::new(crate::server::ServerState {
        daemon,
        static_dir,
        app,
    }));
    Ok((addr, async move {
        let _ = axum::serve(listener, router).await;
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_builds_a_daemon_without_touching_tauri() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(HeadlessHost::new(dir.path()));
        let cfg = Config {
            data_dir: dir.path().to_path_buf(),
            lan: false,
            port: None,
            web_dir: None,
        };
        let daemon = bootstrap(host, cfg);
        assert!(!daemon.state().engine.lock().unwrap().project().tracks.is_empty());
        assert!(daemon.host().ui().is_none());
    }

    #[tokio::test]
    async fn serve_returns_bound_address_and_answers_health() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(HeadlessHost::new(dir.path()));
        let cfg = Config {
            data_dir: dir.path().to_path_buf(),
            lan: false,
            port: Some(0),
            web_dir: None,
        };
        let daemon = bootstrap(host, cfg);
        let (addr, fut) = serve(daemon).await.unwrap();
        assert!(addr.port() > 0);
        tokio::spawn(fut);
        let body = reqwest::get(format!("http://{addr}/health"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(body.contains("\"ok\""));
    }
}

/// 传输状态推送线程：状态变了才发（播放中约 60 Hz，暂停后完全静默）。
///
/// 放在 daemon 而不是桌面壳里，是为了让**浏览器客户端也能收到**：
/// 事件先经 `Host`，Tauri 实现转给 webview，SSE 客户端则从总线拿到同一份。
pub fn spawn_transport_ticker(daemon: Arc<Daemon>) {
    std::thread::Builder::new()
        .name("ai-daw-transport".into())
        .spawn(move || {
            let mut last = None;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(16));
                let snapshot = daemon.state().audio.snapshot();
                if last.as_ref() == Some(&snapshot) {
                    continue;
                }
                crate::host::emit(&**daemon.host(), crate::state::EVENT_TRANSPORT, &snapshot);
                last = Some(snapshot);
            }
        })
        .expect("failed to spawn transport thread");
}
