//! 无窗口启动：会话、预设、音色记忆库，然后交给调用方决定听哪里。
//!
//! 桌面端的 `setup()` 与 `aidawd` 都调这里；**桌面独有的部分**（建窗、原生插件窗口、
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
    fn emit_value(&self, _event: &'static str, _payload: Value) {
        // 事件目前没有订阅者：M3 接上 SSE 后这里改成广播。
        // 注意**不要**因此跳过 emit —— 未来的订阅者只依赖这一条通道。
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
    // 用户的预设与音色记忆库是跨工程资产，按数据目录加载（拿不到就空着，不影响启动）。
    state.init_presets(Some(cfg.data_dir.join("presets.json")));
    state.init_sound_memories(Some(cfg.data_dir.join("sound-memories.json")));
    Daemon::new(state, host, cfg)
}

/// 起 HTTP 服务。返回实际绑定的地址与可被 `select!` 的前台 future。
///
/// `token` 为空串时**拒绝启动**：局域网内的裸 `/rpc` 等于把整台机器的文件读写交出去。
pub async fn serve(
    daemon: Arc<Daemon>,
    token: String,
) -> std::io::Result<(SocketAddr, impl std::future::Future<Output = ()>)> {
    assert!(!token.is_empty(), "拒绝在没有访问令牌的情况下启动 HTTP 服务");
    let cfg = daemon.config().clone();
    let port = cfg.port.unwrap_or(28430);
    let ip = if cfg.lan {
        std::net::Ipv4Addr::UNSPECIFIED
    } else {
        std::net::Ipv4Addr::LOCALHOST
    };
    let listener = TcpListener::bind((std::net::IpAddr::V4(ip), port)).await?;
    let addr = listener.local_addr()?;
    let app = crate::server::router(Arc::new(crate::server::ServerState { daemon, token }));
    Ok((addr, async move {
        let _ = axum::serve(listener, app).await;
    }))
}

/// 访问令牌：读 `<data_dir>/web-token`，没有就生成一份并落盘。
///
/// 桌面端与 `aidawd` 共用同一个文件，所以浏览器在两种模式下填的是同一串 token。
pub fn web_token(data_dir: &Path) -> String {
    let path = data_dir.join("web-token");
    if let Ok(text) = std::fs::read_to_string(&path) {
        let trimmed = text.trim();
        if trimmed.len() == 64 {
            return trimmed.to_string();
        }
    }
    let token = fresh_token();
    let _ = std::fs::create_dir_all(data_dir);
    let _ = std::fs::write(&path, &token);
    token
}

/// 生成一个新的 32 字节令牌（十六进制）。
fn fresh_token() -> String {
    use sha2::{Digest, Sha256};
    let seed = format!(
        "{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        std::env::temp_dir().display()
    );
    Sha256::digest(seed.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
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
        };
        let daemon = bootstrap(host, cfg);
        assert!(!daemon.state().engine.lock().unwrap().project().tracks.is_empty());
        assert!(daemon.host().ui().is_none());
    }

    #[test]
    fn web_token_is_stable_and_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let first = web_token(dir.path());
        let second = web_token(dir.path());
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(dir.path().join("web-token").exists());
    }

    #[tokio::test]
    async fn serve_returns_bound_address_and_answers_health() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(HeadlessHost::new(dir.path()));
        let cfg = Config {
            data_dir: dir.path().to_path_buf(),
            lan: false,
            port: Some(0),
        };
        let daemon = bootstrap(host, cfg);
        let (addr, fut) = serve(daemon, "t".into()).await.unwrap();
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
