//! Tauri 类型的唯一入口：桌面构建转真实 Tauri，无头构建用空壳。
//!
//! 为什么要这一层：`crates/daemon` 里 239 个命令的实现体仍然按 Tauri 的签名写
//! （`AppHandle` / `State<'_, T>`），但**无头二进制不该为这些签名付出 612 MB 的
//! WebKit 代价**。于是把"用到的 Tauri API"收进这一层：
//!
//! - `feature = "desktop"`（默认）：`pub use tauri::*`，行为与以前完全一致；
//! - 否则：下面这组空壳类型，只保证**编译**通过。无头进程从不构造 `AppHandle`，
//!   所以空壳里的方法永远不会被调用——它们的存在只是让签名成立。
//!
//! 这是过渡脚手架：M3 把命令层搬回 `src-tauri` 之后，本模块与所有 `AppHandle`
//! 参数一起删除。

#[cfg(feature = "desktop")]
pub use tauri::*;

#[cfg(not(feature = "desktop"))]
mod stub {
    use std::any::{Any, TypeId};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::RwLock;

    /// 无头进程的托管状态表：`TypeId → &'static dyn Any`。
    ///
    /// Tauri 的 `AppHandle::state::<T>()` 背后就是一张"类型 → 单例"的表，桌面端由
    /// `app.manage()` 填。无头进程没有 AppHandle，于是这张表由 [`install_state`] 填，
    /// 命令层里 `app.state::<T>()` 照常写、照常取——**同一份命令代码两种构建都能跑**。
    ///
    /// 存 `&'static` 是刻意泄漏：这里只装进程级的少量单例（`Arc<AppState>` 等），
    /// 生命周期本来就等于进程；泄漏换来的是 `State<'static, T>` 这种零成本的返回。
    static MANAGED: RwLock<Option<HashMap<TypeId, &'static (dyn Any + Send + Sync)>>> = RwLock::new(None);

    /// 装一份托管状态（可重复安装同类型，后写的生效）。
    ///
    /// 传什么类型就以什么类型登记：`install_state(Arc::clone(&state))` 对应
    /// `app.state::<Arc<AppState>>()`，`install_state(runtime)` 对应
    /// `app.state::<StoreRuntime>()`——与桌面端 `app.manage(x)` 的键完全一致。
    pub fn install_state<T: Any + Send + Sync>(value: T) {
        let leaked: &'static T = Box::leak(Box::new(value));
        let any: &'static (dyn Any + Send + Sync) = leaked;
        let mut guard = MANAGED.write().unwrap();
        guard
            .get_or_insert_with(HashMap::new)
            .insert(TypeId::of::<T>(), any);
    }

    /// 空壳应用句柄：无头进程里没有任何东西会构造它。
    #[derive(Clone, Default)]
    pub struct AppHandle;

    /// 空壳状态句柄：只提供 `Deref`，让 `state.engine` 这类写法成立。
    pub struct State<'a, T: ?Sized>(&'a T);

    impl<T: ?Sized> std::ops::Deref for State<'_, T> {
        type Target = T;
        fn deref(&self) -> &T {
            self.0
        }
    }

    impl<'a, T: ?Sized> State<'a, T> {
        /// 取出内部引用（与 Tauri 的 `State::inner` 同名）。
        pub fn inner(&self) -> &'a T {
            self.0
        }
    }

    impl AppHandle {
        /// 空壳：把任务派到主线程（无头进程没有主线程泵）。
        pub fn run_on_main_thread<F: FnOnce() + Send + 'static>(&self, _task: F) -> Result<(), String> {
            Ok(())
        }

        /// 取托管状态（与 Tauri 的 `AppHandle::state` 同义，数据来自 [`install_state`]）。
        pub fn state<T: Send + Sync + 'static>(&self) -> State<'static, T> {
            let guard = MANAGED.read().unwrap();
            let table = guard.as_ref().expect("[tauri_api] 无头进程还没有安装任何托管状态");
            let any = *table
                .get(&TypeId::of::<T>())
                .unwrap_or_else(|| panic!("[tauri_api] 托管状态里没有 {:?}", std::any::type_name::<T>()));
            State(
                any.downcast_ref::<T>()
                    .unwrap_or_else(|| panic!("[tauri_api] 托管状态类型不匹配：{}", std::any::type_name::<T>())),
            )
        }

        /// 空壳：路径解析。
        pub fn path(&self) -> PathResolver {
            PathResolver
        }

        /// 空壳：广播事件。
        ///
        /// 转发给进程内事件总线，SSE 客户端与 `/audio` 元数据都能收到——桌面端
        /// `app.emit` 发给 webview，这里发给浏览器，二者是同一层语义。
        pub fn emit<S: serde::Serialize>(&self, event: &str, payload: S) -> Result<(), String> {
            match serde_json::to_value(payload) {
                Ok(value) => crate::bus::publish(event, value),
                Err(err) => eprintln!("[emit] 事件 {event} 序列化失败：{err}"),
            }
            Ok(())
        }

        /// 空壳：取窗口。
        pub fn get_webview_window(&self, _label: &str) -> Option<WebviewWindow> {
            None
        }

        /// 空壳：配置。
        pub fn config(&self) -> Config {
            Config
        }
    }

    /// 路径解析器（`app.path().app_data_dir()` 这类调用）。
    #[derive(Clone, Copy, Default)]
    pub struct PathResolver;

    impl PathResolver {
        pub fn app_data_dir(&self) -> Result<PathBuf, String> {
            Err("无头构建没有 Tauri 路径".into())
        }
        pub fn app_config_dir(&self) -> Result<PathBuf, String> {
            Err("无头构建没有 Tauri 路径".into())
        }
        pub fn app_cache_dir(&self) -> Result<PathBuf, String> {
            Err("无头构建没有 Tauri 路径".into())
        }
        pub fn resource_dir(&self) -> Result<PathBuf, String> {
            Err("无头构建没有 Tauri 路径".into())
        }
    }

    /// 空壳配置。
    #[derive(Clone, Copy, Default)]
    pub struct Config;

    /// 空壳窗口（只出现在桌面专用分支里）。
    #[derive(Clone, Default)]
    pub struct WebviewWindow;

    impl WebviewWindow {
        /// 空壳：窗口标签。
        pub fn label(&self) -> &str {
            "main"
        }

        /// 空壳：窗口所属的应用句柄。
        pub fn app_handle(&self) -> AppHandle {
            AppHandle
        }

        /// 空壳：从最小化还原。
        pub fn unminimize(&self) -> Result<(), String> {
            Ok(())
        }

        /// 空壳：macOS 的原生视图句柄（插件 `set_parent` 用）。
        #[cfg(target_os = "macos")]
        pub fn ns_view(&self) -> Result<usize, String> {
            Ok(0)
        }

        pub fn show(&self) -> Result<(), String> {
            Ok(())
        }
        pub fn hide(&self) -> Result<(), String> {
            Ok(())
        }
        pub fn set_focus(&self) -> Result<(), String> {
            Ok(())
        }
        pub fn close(&self) -> Result<(), String> {
            Ok(())
        }
        pub fn set_size<S>(&self, _size: S) -> Result<(), String> {
            Ok(())
        }
        pub fn set_title(&self, _title: &str) -> Result<(), String> {
            Ok(())
        }
        pub fn emit<S: serde::Serialize>(&self, _event: &str, _payload: S) -> Result<(), String> {
            Ok(())
        }
    }

    /// 空壳窗口构造器。
    pub struct WebviewWindowBuilder;

    impl WebviewWindowBuilder {
        pub fn new<M>(_manager: &M, _label: &str, _url: WebviewUrl) -> Self {
            Self
        }

        pub fn decorations(self, _decorations: bool) -> Self {
            self
        }

        pub fn hidden_title(self, _hidden: bool) -> Self {
            self
        }
        pub fn inner_size(self, _width: f64, _height: f64) -> Self {
            self
        }
        pub fn title(self, _title: impl AsRef<str>) -> Self {
            self
        }
        pub fn title_bar_style(self, _style: TitleBarStyle) -> Self {
            self
        }
        pub fn resizable(self, _resizable: bool) -> Self {
            self
        }
        pub fn min_inner_size(self, _width: f64, _height: f64) -> Self {
            self
        }
        pub fn visible(self, _visible: bool) -> Self {
            self
        }
        pub fn build(self) -> Result<WebviewWindow, String> {
            Ok(WebviewWindow)
        }
    }

    /// 空壳窗口 URL。
    pub enum WebviewUrl {
        App(String),
        External(String),
    }

    /// 空壳标题栏样式。
    #[derive(Clone, Copy, Default)]
    pub enum TitleBarStyle {
        #[default]
        Visible,
        Transparent,
        Overlay,
    }

    /// 空壳尺寸。
    pub struct LogicalSize<T> {
        pub width: T,
        pub height: T,
    }

    impl<T> LogicalSize<T> {
        pub fn new(width: T, height: T) -> Self {
            Self { width, height }
        }
    }

    /// 空壳 IPC 响应（二进制回传）。
    pub struct Response(Vec<u8>);

    impl Response {
        pub fn new(bytes: Vec<u8>) -> Self {
            Self(bytes)
        }
    }

    /// 空壳的 IPC 模块。
    pub mod ipc {
        pub use super::Response;
    }

    /// 空壳的异步运行时。
    pub mod async_runtime {
        use std::future::Future;

        /// 后台线程执行阻塞任务。
        pub fn spawn_blocking<F, T>(task: F) -> impl Future<Output = Result<T, String>> + Send
        where
            F: FnOnce() -> T + Send + 'static,
            T: Send + 'static,
        {
            async move { Ok(tokio::task::spawn_blocking(task).await.map_err(|e| e.to_string())?) }
        }

        /// 起一个异步任务。
        pub fn spawn<F>(task: F)
        where
            F: Future<Output = ()> + Send + 'static,
        {
            tokio::spawn(task);
        }
    }

    /// 空壳的 `Manager` / `Emitter`：方法都写成 `AppHandle` 的固有方法了，
    /// 这里只需要让 `use crate::tauri_api::{Emitter, Manager};` 有意义。
    pub trait Manager {}
    pub trait Emitter {}

    impl Manager for AppHandle {}
    impl Emitter for AppHandle {}

    // 空壳里没有真正的 generic 运行时类型；保留别名让签名照常书写。
    pub struct Wry;
    pub trait Runtime {}
    impl Runtime for Wry {}
}

#[cfg(not(feature = "desktop"))]
pub use stub::*;

/* -------------------------------------------------------------------------- */
/* HTTP 命令层用的桥：让 `*_tauri_command` 在两种构建里都能被直接调用            */
/* -------------------------------------------------------------------------- */

/// 把引擎状态装进无头进程的托管表（桌面端由 `app.manage()` 负责，这里不需要做事）。
#[cfg(not(feature = "desktop"))]
pub fn share_rpc_state(state: &std::sync::Arc<crate::state::AppState>) {
    stub::install_state(std::sync::Arc::clone(state));
}

/// 无头进程里桌面端会 `app.manage()` 的那些运行时（插件商店设置等）。
///
/// 这几样只依赖数据目录，浏览器客户端也该能用（改镜像源、下载模型、装插件）。
#[cfg(not(feature = "desktop"))]
pub fn share_headless_runtimes() {
    let app = rpc_app_handle();
    stub::install_state(crate::manager::StoreRuntime::load(&app));
}

/// 桌面端由 `app.manage()` 负责，这里空实现。
#[cfg(feature = "desktop")]
pub fn share_headless_runtimes() {}

/// 把引擎状态装进无头进程的托管表（桌面端已托管，空实现）。
#[cfg(feature = "desktop")]
pub fn share_rpc_state(_state: &std::sync::Arc<crate::state::AppState>) {}

/// `/rpc` 里拿 `AppHandle` 的地方用它。
///
/// 桌面端由外壳把真实句柄交给 HTTP 服务（`serve_with_app`），无头进程则返回空壳——
/// 空壳的 `state()` / `emit()` 已经接上全局托管表与事件总线，命令照常执行。
#[cfg(not(feature = "desktop"))]
pub fn rpc_app_handle() -> AppHandle {
    AppHandle::default()
}

/// `/rpc` 里拿 `AppHandle` 的地方用它（桌面端必须由外壳传入，见 `serve_with_app`）。
#[cfg(feature = "desktop")]
pub fn rpc_app_handle() -> AppHandle {
    panic!("桌面构建的 HTTP 服务没有拿到 AppHandle：请用 serve_with_app 启动")
}

/// 从 `AppHandle` 取引擎状态（两种构建同形，桌面端走 `app.state`）。
pub fn app_state(app: &AppHandle) -> State<'_, std::sync::Arc<crate::state::AppState>> {
    app.state::<std::sync::Arc<crate::state::AppState>>()
}
