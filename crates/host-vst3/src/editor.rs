//! 插件原生界面（独立窗口）。
//!
//! 原生视图（macOS 的 NSView、Windows 的 HWND）没法嵌进网页视图里的某个 DOM 标签页，所以这里
//! 给插件开它**自己的顶层窗口**。窗口附着在**正在发声的那个实例**上，因此界面里的改动会实时
//! 作用到声音。
//!
//! 两种格式走不同路径：
//! - **VST3**：`vst3-host` 的 `PluginWindow` 负责建原生窗口并把插件视图挂进去。
//! - **CLAP**：插件的 GUI 扩展支持「浮动窗口」——窗口完全由插件自己创建与管理，宿主只需
//!   `create` + `show`，然后定期泵它的主线程回调。

use crate::clap::{ClapEditorHandle, ClapGuiWindow};
use crate::error::HostError;
use crate::vst3::Vst3EditorHandle;

/// 与音频线程共享的插件实例句柄（供界面打开原生窗口）。
#[derive(Clone)]
pub enum EditorHandle {
    Vst3(Vst3EditorHandle),
    Clap(ClapEditorHandle),
    /// AU（仅 macOS）。
    #[cfg(target_os = "macos")]
    Au(crate::au::AuEditorHandle),
}

impl EditorHandle {
    /// 这个界面需要**宿主提供原生窗口**吗？需要的话给出插件建议的尺寸。
    ///
    /// - VST3：`vst3-host` 的 `PluginWindow` 自带建窗，宿主不用管 → `None`；
    /// - CLAP：只有「嵌入模式」的插件（比如 KR-106 的 CLAP）才需要 → `Some(size)`；
    /// - AU：宿主自建 `NSWindow` 承载插件自带视图 → `None`。
    ///
    /// 会去问插件实例，**必须在主 / UI 线程调用**。
    pub fn host_window_size(&self) -> Option<(u32, u32)> {
        match self {
            EditorHandle::Vst3(_) => None,
            EditorHandle::Clap(handle) => handle.host_view_size(),
            #[cfg(target_os = "macos")]
            EditorHandle::Au(_) => None,
        }
    }

    /// 装上「插件要求改窗口大小」的回调（只有嵌入模式的插件会用到）。
    pub fn set_resize_handler(&self, handler: crate::clap::ResizeFn) {
        if let EditorHandle::Clap(handle) = self {
            handle.set_resize_handler(handler);
        }
    }

    /// 读插件的完整状态（供宿主在**重建播放器前**抢救、重建后 `load_state` 还原）。
    ///
    /// 在控制侧线程调用即可：CLAP 会自行派发到主线程，VST3 直接在当前线程取。
    /// 插件不支持状态存取（或读取失败）时返回空——空状态在恢复时会被忽略。
    pub fn save_state(&self) -> Vec<u8> {
        match self {
            EditorHandle::Vst3(handle) => handle
                .lock()
                .map(|plugin| plugin.save_state().unwrap_or_default())
                .unwrap_or_default(),
            EditorHandle::Clap(handle) => handle.save_state(),
            #[cfg(target_os = "macos")]
            EditorHandle::Au(handle) => handle.save_state(),
        }
    }

    /// 服务插件发起的宿主回调请求（CLAP 的 `request_callback` → `on_main_thread`）。
    ///
    /// 由编辑器 ticker 定期调用，**与窗口是否打开无关**：插件可能在还没开界面时就请求
    /// 回调，不服务它就会让插件那边的线程一直等下去。**必须在主 / UI 线程调用。**
    pub fn service_requests(&self) {
        if let EditorHandle::Clap(handle) = self {
            handle.service_requests();
        }
    }

    /// 打开界面。
    ///
    /// `host_view` 是宿主原生视图的句柄（macOS 的 NSView / Windows 的 HWND），
    /// 只有需要嵌入的插件用得上——见 [`Self::host_window_size`]。
    ///
    /// **必须在主 / UI 线程调用。**
    pub fn open(&self, title: &str, host_view: Option<usize>) -> Result<PluginEditorWindow, HostError> {
        let kind = match self {
            EditorHandle::Vst3(handle) => {
                if !handle.lock().map(|p| p.has_editor()).unwrap_or(false) {
                    return Err(HostError::NoEditor);
                }
                let mut window = vst3_host::PluginWindow::new(handle.clone());
                window.open().map_err(|e| HostError::Editor(e.to_string()))?;
                WindowKind::Vst3(window)
            }
            EditorHandle::Clap(handle) => {
                WindowKind::Clap(handle.open_window(title, host_view)?)
            }
            #[cfg(target_os = "macos")]
            EditorHandle::Au(handle) => WindowKind::Au(handle.open(title)?),
        };
        Ok(PluginEditorWindow { kind })
    }
}

/// 打开的窗口内部是哪种。
enum WindowKind {
    Vst3(vst3_host::PluginWindow),
    Clap(ClapGuiWindow),
    #[cfg(target_os = "macos")]
    Au(crate::au::AuEditorWindow),
}

/// 一个已打开的插件窗口。
pub struct PluginEditorWindow {
    kind: WindowKind,
}

impl PluginEditorWindow {
    /// 每帧调用：把插件发起的窗口级事件（缩放、DPI、主线程回调）送进插件。
    ///
    /// 非阻塞：插件正被音频线程加锁处理时它会直接返回，下一帧再补。
    pub fn service(&self) {
        match &self.kind {
            WindowKind::Vst3(window) => {
                let _ = window.service_platform_events();
            }
            WindowKind::Clap(window) => window.service(),
            // AU 的窗口事件由应用自己的 NSApplication 事件循环处理，无需每帧泵。
            #[cfg(target_os = "macos")]
            WindowKind::Au(_) => {}
        }
    }

    /// 窗口是否还开着。
    pub fn is_open(&self) -> bool {
        match &self.kind {
            WindowKind::Vst3(window) => window.is_open(),
            WindowKind::Clap(window) => window.is_open(),
            #[cfg(target_os = "macos")]
            WindowKind::Au(window) => window.is_open(),
        }
    }

    /// 用户自己把窗口关了（宿主不会被通知，需要每帧轮询）。
    pub fn closed_by_user(&self) -> bool {
        !self.is_open()
    }

    /// 关闭窗口（`Drop` 也会做）。
    pub fn close(self) {
        match self.kind {
            WindowKind::Vst3(mut window) => window.close(),
            WindowKind::Clap(window) => window.close(),
            #[cfg(target_os = "macos")]
            WindowKind::Au(window) => window.close(),
        }
    }
}
