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

/// 起 HTTP 服务。返回实际绑定的地址与可被 `select!` 的前台 future。
///
/// `token` 为空串时**拒绝启动**：局域网内的裸 `/rpc` 等于把整台机器的文件读写交出去。
pub async fn serve(
    daemon: Arc<Daemon>,
) -> std::io::Result<(SocketAddr, impl std::future::Future<Output = ()>)> {
    serve_with_app(daemon, None).await
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
