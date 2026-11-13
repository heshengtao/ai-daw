//! 后端容器：一份共享状态 + 一个宿主实现 + 一份配置。
//!
//! 桌面端与 `aidawd` 都构造它，命令实现只认它——不认 Tauri，也不认 axum。
//! 这样同一批 `*_impl` 既能被 webview 的命令薄壳调用，也能被 HTTP 的 `/rpc` 调用。

use std::path::PathBuf;
use std::sync::Arc;

use crate::host::Host;
use crate::state::AppState;

/// 服务配置。
#[derive(Debug, Clone)]
pub struct Config {
    /// 应用数据目录（会话、插件缓存、token）。
    pub data_dir: PathBuf,
    /// 是否绑到 0.0.0.0（局域网）。默认只监听回环。
    pub lan: bool,
    /// HTTP 端口；`None` = 不启动 HTTP 服务。
    pub port: Option<u16>,
    /// 前端构建产物目录（`dist/`）。给了就由 daemon 托管，浏览器直接连它。
    pub web_dir: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self { data_dir: PathBuf::from("."), lan: false, port: None, web_dir: None }
    }
}

/// 后端实例。**唯一**持有 `AppState` 的地方。
pub struct Daemon {
    state: Arc<AppState>,
    host: Arc<dyn Host>,
    cfg: Config,
}

impl Daemon {
    /// 装配一个后端。`state` 与 `host` 都由外壳提供（桌面端或 `aidawd`）。
    pub fn new(state: Arc<AppState>, host: Arc<dyn Host>, cfg: Config) -> Arc<Self> {
        Arc::new(Self { state, host, cfg })
    }

    /// 共享状态；命令实现从这里读工程、锁引擎。
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// 状态的 `Arc` 句柄（外壳需要同时托管 `Arc<AppState>` 时用）。
    pub fn state_arc(&self) -> &Arc<AppState> {
        &self.state
    }

    /// 宿主实现。
    pub fn host(&self) -> &Arc<dyn Host> {
        &self.host
    }

    /// 配置。
    pub fn config(&self) -> &Config {
        &self.cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::FakeHost;

    #[test]
    fn daemon_shares_state_and_host() {
        let state = Arc::new(AppState::bootstrap(None, Vec::new()));
        let host = Arc::new(FakeHost::default());
        let daemon = Daemon::new(Arc::clone(&state), host, Config::default());
        assert!(Arc::ptr_eq(daemon.state_arc(), &state));
        assert!(daemon.host().ui().is_none());
    }
}
