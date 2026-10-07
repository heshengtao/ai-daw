use std::sync::Mutex;

use daw_party_engine::bus::Engine;

/// 应用共享状态：进程内持有唯一的引擎实例。
pub struct AppState {
    pub engine: Mutex<Engine>,
}

impl AppState {
    /// 以给定工程名新建共享状态。
    pub fn new(name: &str) -> Self {
        AppState { engine: Mutex::new(Engine::new(name)) }
    }
}

/// 引擎事件广播的前端事件名。
pub const EVENT_ENGINE: &str = "engine://event";
