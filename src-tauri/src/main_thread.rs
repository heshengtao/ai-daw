//! 把 `ai-daw-host-vst3` 的「主线程派发」接到 Tauri 的主线程上。
//!
//! CLAP 规定 `init` / 参数枚举 / `activate` / 原生界面都只能在**进程主线程**上调用，插件会
//! 真的去核对 `clap_host_thread_check`。宿主 crate 是库、不认识 Tauri，所以由这里注入实现。
//!
//! 实现方式就是项目里已经用了很多次的模式：`run_on_main_thread` 派发 + 回执 channel 阻塞等待
//! （见 `commands.rs` 的 `open_plugin_window`）。

use std::sync::mpsc;
use std::sync::Arc;

use ai_daw_host_vst3::MainThreadDispatch;
use tauri::AppHandle;

/// 用 `AppHandle::run_on_main_thread` 实现的主线程派发。
pub struct TauriMainThread {
    app: AppHandle,
}

impl TauriMainThread {
    /// 装上派发器。
    ///
    /// **必须在主线程上、且在主线程事件循环起来之前调用一次**（`setup` 正合适：它就在主线程
    /// 上，而且早于后台插件扫描与编辑器 ticker 上线）。
    ///
    /// 调用前必须先调过 `mark_main_thread()`：否则 `is_on_main_thread()` 恒为 false，主线程
    /// 自己发起的派发会退化成「自己等自己」而死锁。
    pub fn install(app: &AppHandle) {
        ai_daw_host_vst3::set_main_thread_dispatch(Arc::new(Self { app: app.clone() }));
    }
}

impl MainThreadDispatch for TauriMainThread {
    fn dispatch(&self, task: Box<dyn FnOnce() + Send + 'static>) -> bool {
        let (tx, rx) = mpsc::channel();
        let handed = self
            .app
            .run_on_main_thread(move || {
                task();
                let _ = tx.send(());
            })
            .is_ok();
        if !handed {
            // 主线程已经退出（通常是应用正在关闭）：调用方会退回就地执行。
            return false;
        }
        // 阻塞等主线程跑完——调用方（音频控制线程 / 渲染池线程）本来就不是实时线程。
        rx.recv().is_ok()
    }
}
