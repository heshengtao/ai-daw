//! `Host` / `UiHost` 的桌面实现：事件转给 webview，原生能力走 Tauri。
//!
//! 这是 shell 与后端之间唯一的接线层。后端不知道 Tauri 的存在，只认 trait。

use std::path::PathBuf;

use ai_daw_daemon::host::{Host, PickRequest, UiHost};
use serde_json::Value;
use tauri::{Emitter, Manager};

/// 桌面宿主。
#[derive(Clone)]
pub struct TauriHost {
    app: tauri::AppHandle,
}

impl TauriHost {
    /// 绑定到当前的 Tauri App。
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

impl Host for TauriHost {
    fn emit_value(&self, event: &'static str, payload: Value) {
        // 建窗 / 关窗那一瞬间 webview 可能还没准备好；广播失败只影响这一次推送。
        let _ = self.app.emit(event, payload.clone());
        // 同一条事件也要进总线：浏览器客户端（SSE）和 webview 看到的是同一份状态。
        ai_daw_daemon::bus::publish(event, payload);
    }

    fn data_dir(&self) -> PathBuf {
        self.app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
    }

    fn ui(&self) -> Option<&dyn UiHost> {
        Some(self)
    }
}

impl UiHost for TauriHost {
    fn pick_file(&self, _request: PickRequest) -> Result<Option<PathBuf>, String> {
        // 桌面端的文件选择走前端 `@tauri-apps/plugin-dialog`（见 src/api/dialogs.ts）：
        // 命令拿到的是前端选好的路径，后端不需要自己弹窗。
        Ok(None)
    }

    fn show_main_window(&self) {
        if let Some(window) = self.app.get_webview_window("main") {
            let _ = window.show();
        }
    }

    fn run_on_main_thread(&self, task: Box<dyn FnOnce() + Send>) {
        let _ = self.app.run_on_main_thread(task);
    }
}
