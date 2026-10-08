//! CLAP 后端：用 `clack` 托管 CLAP 实例，适配成本项目的 [`Plugin`] trait。
//!
//! 为什么不用 truce-rack 的 CLAP 后端：实测它在加载 KR-106 时**段错误**。`clack` 把
//! CLAP 的 FFI 全部安全封装，这里只写宿主策略。
//!
//! 三个 CLAP 特有的约束：
//! - **参数只能走事件**：CLAP 没有「设置参数」这个调用，改参数必须作为 `ParamValueEvent`
//!   在下一个 `process` 的输入事件里送达，因此 `set_param` 只是入队。
//! - **状态**走 `PluginState` 扩展（`save(Write)` / `load(Read)`），字节流原样交给工程保存。
//! - `PluginInstance` 被 clack 显式标记为 `!Send`（CLAP 有自己的线程模型），见文件末尾的
//!   `unsafe impl Send` 及其安全论证。
//!
//! # 线程契约（这是本文件最容易搞错的地方）
//!
//! CLAP 规定每个入口点只能在特定线程调用，插件（如 KR-106）会**真的去核对**，并在违规时
//! 报警甚至挂住。对照表：
//!
//! | 调用 | 线程 | 这里在哪 |
//! |---|---|---|
//! | `init` / `params.count` / `activate` / `deactivate` | 主线程 | `prepare` 里经 [`crate::dispatch::on_main_thread`] 派发 |
//! | `start_processing` / `stop_processing` / `reset` / `process` | 音频线程 | 第一次 `process`（`ensure_processing_started`）与 `reset` |
//! | `clap_plugin_gui.*` / `on_main_thread` / `destroy` | 主线程 | GUI 命令与编辑器 ticker；`Drop` 也把释放搬回主线程 |
//!
//! 关键点：`activate` 返回的 `StoppedPluginAudioProcessor` 是 `Send` 的句柄，所以可以
//! **在主线程 `activate`、把句柄交给音频线程再 `start_processing`**——这正是 clack 的用法。
//!
//! 这样拆分之后，**音频线程完全不碰任何 Mutex**（`process` / `reset` 只用处理器句柄）。
//! 旧实现让音频回调去锁插件实例，而主线程 `open_window` 又长时间持有同一把锁，一开插件
//! 界面就会把实时线程卡住（`buffer underrun`）甚至双向死锁——这是「开自带界面就卡死」的根因。

use std::ffi::CStr;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use clack_extensions::gui::{GuiConfiguration, GuiError, GuiSize, HostGui, HostGuiImpl, PluginGui};
use clack_extensions::params::{
    HostParams, HostParamsImplMainThread, HostParamsImplShared, ParamClearFlags, ParamInfoBuffer,
    ParamInfoFlags, ParamRescanFlags, PluginParams,
};
use clack_extensions::state::{HostState, HostStateImpl, PluginState};
use clack_extensions::thread_check::{HostThreadCheck, HostThreadCheckImpl};
use clack_host::events::event_types::{NoteOffEvent, NoteOnEvent, ParamValueEvent};
use clack_host::prelude::*;

use daw_party_plugin_api::{
    IoLayout, MidiEvent as HostEvent, ParamId, ParamInfo, Plugin, PluginCapabilities,
    PluginDescriptor, PluginError, PluginKind, ProcessContext, TimedMidiEvent,
};

use crate::error::HostError;

/// 引擎侧恒为立体声（`Player` 每轨固定送左右两条平面缓冲）。
const ENGINE_CHANNELS: usize = 2;
/// 缓冲的初始块大小（`prepare` 会按实际值调整）。
const DEFAULT_BLOCK: usize = 512;

// ---------------------------------------------------------------------------
// 宿主实现
// ---------------------------------------------------------------------------

/// 嵌入模式下，宿主给插件的**兜底**初始窗口尺寸。
///
/// 正常情况下用不上：宿主会问插件自报的尺寸（见 [`ClapEditorHandle::host_view_size`]），
/// 并在 `open_window` 里把窗口改成那个尺寸。只有插件不肯报尺寸时才退回它。
const EMBEDDED_WINDOW_HINT: (u32, u32) = (1000, 700);

/// 问出嵌入模式下插件**自报的尺寸**（拿不到就返回 `hint`）。
///
/// `clap_plugin_gui.get_size()` 只有在 `create()` 之后才合法，所以这里真建一次、问完立刻销毁。
/// 宿主就是照这个尺寸开窗口的——尺寸对不上时，先看这里。
fn probe_embedded_size(instance: &mut PluginInstance<ClapHost>, hint: (u32, u32)) -> (u32, u32) {
    let handle = instance.plugin_handle();
    let Some(gui) = handle.get_extension::<PluginGui>() else {
        return hint;
    };
    let config = GuiConfiguration { api_type: default_api_type(), is_floating: false };
    if gui.create(&handle, config).is_err() {
        return hint;
    }
    let size = gui.get_size(&handle).map_or(hint, |s| (s.width, s.height));
    gui.destroy(&handle);
    size
}

/// 插件要求改宿主窗口大小时的回调（由宿主装进来）。
pub type ResizeFn = Box<dyn Fn(u32, u32) + Send + Sync>;

/// 宿主共享数据（三个线程共用）。
struct ClapHostShared {
    gui_closed: Arc<AtomicBool>,
    /// 音频线程 id。
    ///
    /// **不能在建实例时猜**：实例是在主线程上建的，而 `process` 跑在音频后端的回调线程上
    /// ——猜错的话插件每次 `process` 都会认为「宿主把我叫到了错误的线程」，于是每个音频块
    /// 都往 stderr 打一行日志，把实时线程堵死。所以由第一次 `process` 记下来（且必须在
    /// `start_processing` **之前**记，插件会在那里问）。
    audio_thread: Arc<OnceLock<std::thread::ThreadId>>,
    /// 宿主窗口的缩放回调（嵌入模式下插件会要求改窗口大小）。
    resize: Arc<Mutex<Option<ResizeFn>>>,
    /// 插件请求宿主在主线程上叫它一次（`request_callback`）。
    callback_requested: Arc<AtomicBool>,
    /// 插件请求重开处理（`request_restart`）。
    restart_requested: Arc<AtomicBool>,
}

/// 诊断计数：插件问过宿主几次「这是主线程吗 / 这是音频线程吗」。
///
/// 有些插件的 GUI 会靠 `clap_host_thread_check` 判断自己能不能干活；留下这两个计数是为了
/// 排查「插件的原生界面不出声」这类问题（例子见 `examples/clap_probe.rs`）。
pub static THREAD_CHECK_CALLS: [std::sync::atomic::AtomicUsize; 2] = [
    std::sync::atomic::AtomicUsize::new(0),
    std::sync::atomic::AtomicUsize::new(0),
];

impl HostThreadCheckImpl for ClapHostShared {
    fn is_main_thread(&self) -> bool {
        THREAD_CHECK_CALLS[0].fetch_add(1, Ordering::Relaxed);
        crate::dispatch::is_on_main_thread()
    }

    fn is_audio_thread(&self) -> bool {
        THREAD_CHECK_CALLS[1].fetch_add(1, Ordering::Relaxed);
        self.audio_thread
            .get()
            .is_some_and(|id| *id == std::thread::current().id())
    }
}

impl<'a> SharedHandler<'a> for ClapHostShared {
    fn request_restart(&self) {
        // 插件要求宿主重新激活它（CLAP 里这是为了**更换音频配置**）。
        // 我们的配置（采样率 / 块大小）在实例生命周期内从不变化，所以不需要
        // deactivate/activate 那一轮；只做音频线程合法的 stop/start processing，
        // 由音频回调在下一个块开头执行（见 `ClapHostedPlugin::process`）。
        self.restart_requested.store(true, Ordering::Release);
    }

    fn request_process(&self) {
        // 无需实现：音频流一直在跑，下一个 `process` 立刻就到。
    }

    fn request_callback(&self) {
        // 插件（通常来自它自己的线程）要宿主在主线程上叫它一次。这里只置旗子，
        // 由编辑器 ticker 在主线程调 `clap_plugin.on_main_thread()`。
        // 不实现它的话，正在等回调的插件线程会一直等下去。
        self.callback_requested.store(true, Ordering::Release);
    }
}

impl HostParamsImplShared for ClapHostShared {
    fn request_flush(&self) {}
}

impl HostGuiImpl for ClapHostShared {
    fn resize_hints_changed(&self) {}

    fn request_resize(&self, new_size: GuiSize) -> Result<(), clack_host::host::HostError> {
        // 嵌入模式下这是**必须**实现的：插件的界面尺寸由它自己决定，宿主得改窗口跟上。
        if let Ok(slot) = self.resize.lock() {
            if let Some(handler) = slot.as_ref() {
                handler(new_size.width, new_size.height);
            }
        }
        Ok(())
    }

    fn request_show(&self) -> Result<(), clack_host::host::HostError> {
        Ok(())
    }

    fn request_hide(&self) -> Result<(), clack_host::host::HostError> {
        Ok(())
    }

    fn closed(&self, _was_destroyed: bool) {
        self.gui_closed.store(true, Ordering::Relaxed);
    }
}

/// 主线程处理器。
struct ClapHostMainThread<'a> {
    _shared: &'a ClapHostShared,
}

impl<'a> MainThreadHandler<'a> for ClapHostMainThread<'a> {}

impl HostParamsImplMainThread for ClapHostMainThread<'_> {
    fn rescan(&self, _flags: ParamRescanFlags) {}
    fn clear(&self, _param_id: ClapId, _flags: ParamClearFlags) {}
}

impl HostStateImpl for ClapHostMainThread<'_> {
    fn mark_dirty(&self) {}
}

/// 宿主句柄。
struct ClapHost;

impl HostHandlers for ClapHost {
    type Shared<'a> = ClapHostShared;
    type MainThread<'a> = ClapHostMainThread<'a>;
    type AudioProcessor<'a> = ();

    fn declare_extensions(builder: &mut HostExtensions<Self>, _shared: &Self::Shared<'_>) {
        builder
            .register::<HostParams>()
            .register::<HostState>()
            .register::<HostGui>()
            .register::<HostThreadCheck>();
    }
}

fn host_info() -> HostInfo {
    // 失败只可能是字符串里有 NUL，这里全是常量。
    HostInfo::new("daw-party", "daw-party", "https://example.com", env!("CARGO_PKG_VERSION"))
        .expect("host info 常量应当合法")
}

/// CLAP 的界面有两种托管方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiMode {
    /// 浮动：窗口由插件自己创建，宿主只要 `create` + `show`。
    Floating,
    /// 嵌入：**宿主必须提供一个原生视图**，插件把自己的视图塞进去（`set_parent`）。
    Embedded,
}

/// 问出这个插件的 GUI 用哪种方式托管；不支持界面返回 `None`。
///
/// 两种都支持时优先浮动（宿主少做一层窗口管理）。
pub fn gui_mode(entry: &PluginEntry, plugin_id: &CStr) -> Option<GuiMode> {
    let mut instance = PluginInstance::<ClapHost>::new(
        |_| ClapHostShared {
            gui_closed: Arc::new(AtomicBool::new(false)),
            audio_thread: Arc::new(OnceLock::new()),
            resize: Arc::new(Mutex::new(None)),
            callback_requested: Arc::new(AtomicBool::new(false)),
            restart_requested: Arc::new(AtomicBool::new(false)),
        },
        |shared| ClapHostMainThread { _shared: shared },
        entry,
        plugin_id,
        &host_info(),
    )
    .ok()?;
    let handle = instance.plugin_handle();
    let gui = handle.get_extension::<PluginGui>()?;
    let api_type = default_api_type();
    if gui.is_api_supported(&handle, GuiConfiguration { api_type, is_floating: true }) {
        return Some(GuiMode::Floating);
    }
    if gui.is_api_supported(&handle, GuiConfiguration { api_type, is_floating: false }) {
        return Some(GuiMode::Embedded);
    }
    None
}

/// 该插件是否有原生界面（浮动或嵌入都算——两种宿主都能显示，只是方式不同）。
pub fn has_gui(entry: &PluginEntry, plugin_id: &CStr) -> bool {
    gui_mode(entry, plugin_id).is_some()
}

/// 诊断：问出这个 CLAP 的 GUI 支持哪些 API / 浮动模式。
///
/// 给 `examples/clap_probe.rs` 用。宿主能不能显示插件界面，完全取决于这里的结果：
/// 只支持「嵌入」的插件，本宿主（只能提供浮动窗口）就显示不出来。
pub fn probe_gui(entry: &PluginEntry, plugin_id: &CStr) -> Result<String, HostError> {
    use clack_extensions::gui::GuiApiType;

    let mut instance = PluginInstance::<ClapHost>::new(
        |_| ClapHostShared {
            gui_closed: Arc::new(AtomicBool::new(false)),
            audio_thread: Arc::new(OnceLock::new()),
            resize: Arc::new(Mutex::new(None)),
            callback_requested: Arc::new(AtomicBool::new(false)),
            restart_requested: Arc::new(AtomicBool::new(false)),
        },
        |shared| ClapHostMainThread { _shared: shared },
        entry,
        plugin_id,
        &host_info(),
    )
    .map_err(|e| HostError::Load(e.to_string()))?;

    let handle = instance.plugin_handle();
    let Some(gui) = handle.get_extension::<PluginGui>() else {
        return Ok("没有 GUI 扩展（插件不自带界面）".into());
    };

    let mut report = String::new();
    match gui.get_preferred_api(&handle) {
        Some(config) => report.push_str(&format!(
            "偏好 api={} floating={} | ",
            config.api_type.0.to_string_lossy(),
            config.is_floating
        )),
        None => report.push_str("没有偏好 api | "),
    }
    let apis = [
        ("cocoa", GuiApiType::COCOA),
        ("win32", GuiApiType::WIN32),
        ("x11", GuiApiType::X11),
        ("wayland", GuiApiType::WAYLAND),
    ];
    for (name, api_type) in apis {
        for is_floating in [true, false] {
            let ok = gui.is_api_supported(&handle, GuiConfiguration { api_type, is_floating });
            report.push_str(&format!("{name}/floating={is_floating}:{ok} "));
        }
    }

    // 嵌入模式下插件**自报的窗口尺寸**：宿主就是照它开窗口的。
    // 尺寸只有在 `create()` 之后才合法，所以这里真建一次再问。
    let embedded = GuiConfiguration { api_type: default_api_type(), is_floating: false };
    if gui.is_api_supported(&handle, embedded) {
        match gui.create(&handle, embedded) {
            Ok(()) => {
                match gui.get_size(&handle) {
                    Some(size) => report.push_str(&format!("| 嵌入尺寸 {}x{}", size.width, size.height)),
                    None => report.push_str("| 嵌入尺寸：插件没给"),
                }
                gui.destroy(&handle);
            }
            Err(e) => report.push_str(&format!("| 嵌入 create 失败：{e}")),
        }
    }
    Ok(report)
}

// ---------------------------------------------------------------------------
// 参数元信息// ---------------------------------------------------------------------------

struct ClapParamMeta {
    clap_id: ClapId,
    id: ParamId,
    name: String,
    min: f64,
    max: f64,
    default: f64,
    automatable: bool,
}

// ---------------------------------------------------------------------------
// 适配器
// ---------------------------------------------------------------------------

/// CLAP 编辑器句柄：与界面线程**共享同一个插件实例**。
///
/// CLAP 的线程模型里，主线程（GUI / 参数）与音频线程（`process`）本就是并行的，
/// 因此这里共享实例是模型允许的；音频线程用的是另一个对象（processor），不会互相抢锁。
///
/// `instance` 用 `Option` 是为了让 `Drop` 能把最后一份 Arc 移出去交给主线程释放。
#[derive(Clone)]
pub struct ClapEditorHandle {
    instance: Option<Arc<Mutex<PluginInstance<ClapHost>>>>,
    gui_closed: Arc<AtomicBool>,
    /// 宿主窗口的缩放回调（`set_resize_handler` 装上）。
    resize: Arc<Mutex<Option<ResizeFn>>>,
    /// 插件的 `request_callback` 旗子。
    callback_requested: Arc<AtomicBool>,
}

/// 释放：句柄可能正好是实例的**最后一个引用**，而释放会触发 `deactivate` + `destroy`
/// ——两者都只能在主线程调，所以最后一份引用要搬到主线程去丢。
impl Drop for ClapEditorHandle {
    fn drop(&mut self) {
        let Some(instance) = self.instance.take() else { return };
        // 还有别的持有者（map、ticker 的临时克隆…）时释放不会发生在这里，
        // 就不必白跑一趟主线程。
        if Arc::strong_count(&instance) > 1 {
            return;
        }
        // 派发失败时（主线程已退出）`instance` 会在派发内部就地释放。
        let instance = SharedInstance(instance);
        let _ = crate::dispatch::on_main_thread(move || drop(instance));
    }
}

// SAFETY: `PluginInstance` 被 clack 标记为 `!Send`，但我们的用法是：
// - 实例只在**主线程**（界面）加锁使用；
// - 音频线程只碰独立的 `StartedPluginAudioProcessor`；
// CLAP 规范本身就允许主线程调用与音频处理并行，所以跨线程搬运这个句柄是安全的。
unsafe impl Send for ClapEditorHandle {}
unsafe impl Sync for ClapEditorHandle {}

/// 把实例 Arc 搬过线程边界的包装（用于「派发到主线程」的闭包）。
///
/// `Arc<Mutex<PluginInstance>>` 本身不是 `Send`（clack 故意把实例标成 `!Send`），而派发
/// 要求闭包 `Send`。安全论证与 [`ClapEditorHandle`] 相同：实例只在主线程上加锁使用，
/// 音频线程只碰独立的 processor 句柄。本 crate 是工作区里唯一允许 `unsafe` 的地方。
struct SharedInstance(Arc<Mutex<PluginInstance<ClapHost>>>);

// SAFETY: 见上方说明。
unsafe impl Send for SharedInstance {}
unsafe impl Sync for SharedInstance {}

/// 一个被托管的 CLAP 实例。
pub struct ClapHostedPlugin {
    id: String,
    name: String,
    kind: PluginKind,
    io: IoLayout,
    params: Vec<ClapParamMeta>,
    /// 是否自带原生界面（构造时探测一次）。
    has_editor: bool,
    /// 插件实例。
    ///
    /// 用 `ManuallyDrop` 是为了让 `Drop` 能把**最后一份** Arc 移出去、交到主线程释放：
    /// `clap_plugin.destroy()`（以及尚未 `deactivate` 时的 `deactivate`）只能在主线程调。
    instance: ManuallyDrop<Arc<Mutex<PluginInstance<ClapHost>>>>,
    gui_closed: Arc<AtomicBool>,
    /// 音频线程 id（第一次 `process` 时记下，见 [`ClapHostShared::audio_thread`]）。
    audio_thread: Arc<OnceLock<std::thread::ThreadId>>,
    /// 宿主窗口的缩放回调（嵌入模式下插件会要求改窗口大小）。
    resize: Arc<Mutex<Option<ResizeFn>>>,
    /// 插件请求的主线程回调 / 重开处理旗子（与 `ClapHostShared` 共享）。
    callback_requested: Arc<AtomicBool>,
    restart_requested: Arc<AtomicBool>,
    /// 音频处理器句柄。
    ///
    /// `prepare` 在主线程 `activate` 后得到 `Stopped`；第一次 `process`（音频线程）才
    /// `start_processing` 变成 `Started`。`reset` 也只用它——所以音频线程不碰实例锁。
    processor: Option<PluginAudioProcessor<ClapHost>>,
    sample_rate: f64,
    block_size: usize,
    in_ports: AudioPorts,
    out_ports: AudioPorts,
    /// 音频输入端口的副本（CLAP 的输入是只读的，不能与输出别名）。
    in_buf: Vec<Vec<f32>>,
    /// 处理输出暂存（处理完再拷回引擎的平面缓冲）。
    out_buf: Vec<Vec<f32>>,
    /// 输入事件缓冲：本块 MIDI + 待送达的参数变更。
    in_events: EventBuffer,
    /// 插件写回的出站事件。
    out_events: EventBuffer,
    /// 待送达的参数变更（下一个 `process` 会作为事件发出）。
    param_queue: Vec<(ClapId, f64)>,
    /// 插件侧产生的参数变化，等待宿主抽干。
    pending: Vec<(ParamId, f32)>,
}

// SAFETY: clack 故意把 `PluginInstance` 标记为 `!Send`，因为 CLAP 有自己的线程模型
// （主线程 / 音频线程 / 共享）。在我们的用法里，一个实例在音频线程上创建、使用并销毁，
// 从不跨线程移动——`Player` 在音频线程内构造，插件也在那里创建。本 crate 是工作区里
// 唯一允许 `unsafe` 的地方，这条保证由它承担。
unsafe impl Send for ClapHostedPlugin {}

impl ClapHostedPlugin {
    /// 从已加载的 CLAP entry 构造适配器。
    ///
    /// `plugin_id` 是该 entry 里的插件 id（一个 CLAP 文件可能导出多个插件）。
    ///
    /// **必须在进程主线程上调用**：`clap_plugin.init()` 与参数枚举都是主线程专属。
    /// `crate::scan` 里的加载入口都经 [`crate::dispatch::on_main_thread`] 派发。
    pub fn from_loaded(id: String, entry: &PluginEntry, plugin_id: &CStr) -> Result<Self, HostError> {
        let factory = entry
            .get_plugin_factory()
            .ok_or_else(|| HostError::Load("该 .clap 没有插件工厂".into()))?;
        let descriptor = factory
            .plugin_descriptors()
            .find(|d| d.id() == Some(plugin_id))
            .ok_or_else(|| HostError::Load("该 .clap 里没有这个插件 id".into()))?;

        let name = descriptor
            .name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| id.clone());
        // CLAP 用 features 标记插件类别；只有显式声明 instrument 的才当音源。
        let kind = if descriptor.features().any(|f| f.to_bytes() == b"instrument") {
            PluginKind::Instrument
        } else {
            PluginKind::Effect
        };
        let inputs = if kind == PluginKind::Effect { ENGINE_CHANNELS } else { 0 };

        let gui_closed = Arc::new(AtomicBool::new(false));
        let shared_flag = Arc::clone(&gui_closed);
        let audio_thread: Arc<OnceLock<std::thread::ThreadId>> = Arc::new(OnceLock::new());
        let audio_thread_shared = Arc::clone(&audio_thread);
        let resize: Arc<Mutex<Option<ResizeFn>>> = Arc::new(Mutex::new(None));
        let resize_shared = Arc::clone(&resize);
        let callback_requested = Arc::new(AtomicBool::new(false));
        let callback_shared = Arc::clone(&callback_requested);
        let restart_requested = Arc::new(AtomicBool::new(false));
        let restart_shared = Arc::clone(&restart_requested);
        let mut instance = PluginInstance::<ClapHost>::new(
            move |_| ClapHostShared {
                gui_closed: Arc::clone(&shared_flag),
                audio_thread: Arc::clone(&audio_thread_shared),
                resize: Arc::clone(&resize_shared),
                callback_requested: Arc::clone(&callback_shared),
                restart_requested: Arc::clone(&restart_shared),
            },
            |shared| ClapHostMainThread { _shared: shared },
            entry,
            plugin_id,
            &host_info(),
        )
        .map_err(|e| HostError::Load(e.to_string()))?;

        let params = collect_params(&mut instance);
        let has_editor = instance
            .plugin_handle()
            .get_extension::<PluginGui>()
            .is_some();

        Ok(ClapHostedPlugin {
            id,
            name,
            kind,
            io: IoLayout { inputs: inputs as u16, outputs: ENGINE_CHANNELS as u16 },
            params,
            has_editor,
            instance: ManuallyDrop::new(Arc::new(Mutex::new(instance))),
            gui_closed,
            audio_thread,
            resize,
            callback_requested,
            restart_requested,
            processor: None,
            sample_rate: 48_000.0,
            block_size: DEFAULT_BLOCK,
            in_ports: AudioPorts::with_capacity(ENGINE_CHANNELS, 1),
            out_ports: AudioPorts::with_capacity(ENGINE_CHANNELS, 1),
            in_buf: vec![vec![0.0; DEFAULT_BLOCK]; ENGINE_CHANNELS],
            out_buf: vec![vec![0.0; DEFAULT_BLOCK]; ENGINE_CHANNELS],
            in_events: EventBuffer::new(),
            out_events: EventBuffer::new(),
            param_queue: Vec::new(),
            pending: Vec::new(),
        })
    }

    /// 给界面用的编辑器句柄（与音频线程共享同一实例）。
    pub fn editor_handle(&self) -> ClapEditorHandle {
        ClapEditorHandle {
            instance: Some(Arc::clone(&*self.instance)),
            gui_closed: Arc::clone(&self.gui_closed),
            resize: Arc::clone(&self.resize),
            callback_requested: Arc::clone(&self.callback_requested),
        }
    }

    /// 诊断：宿主记下来的「音频线程」是不是当前这条线程。
    ///
    /// 给测试用：「在 A 线程建实例、在 B 线程 `process`」时正确答案是 B。
    pub fn audio_thread_is_current(&self) -> bool {
        self.audio_thread
            .get()
            .is_some_and(|id| *id == std::thread::current().id())
    }

    /// 测试用：实例是否已激活（主线程的 `activate` 是否跑过）。
    pub fn is_active(&self) -> bool {
        self.instance.lock().is_ok_and(|i| i.is_active())
    }

    /// 测试用：是否已经在 `start_processing`（即真的在音频线程上跑起来了）。
    pub fn is_processing(&self) -> bool {
        self.processor.as_ref().is_some_and(PluginAudioProcessor::is_started)
    }

    /// 把实例交给 `activate` 用的音频配置。
    fn audio_config(&self) -> PluginAudioConfiguration {
        PluginAudioConfiguration {
            sample_rate: self.sample_rate,
            min_frames_count: 1,
            max_frames_count: self.block_size.max(1) as u32,
        }
    }
}

/// 释放：必须在主线程上跑完 `deactivate` + `destroy`。
impl Drop for ClapHostedPlugin {
    fn drop(&mut self) {
        // SAFETY: 只在 `drop` 里取出一次，之后字段不会被再用到。
        let instance = SharedInstance(unsafe { ManuallyDrop::take(&mut self.instance) });
        // 派发失败时（主线程已退出）`instance` 会在派发内部就地释放——那时已经没有
        // 合规的选择了，只能退而求其次。
        //
        // 已知残留：若此时处理器还处于 `Started`（宿主被销毁时没有机会在音频线程上
        // `stop_processing`），clack 会在释放路径里替我们停它一次，于是插件会看到一次
        // 「stop_processing on wrong thread」——**只在销毁时报一次，不影响实时线程**。
        // 要做到零告警，需要「先让音频回调停处理、再暂停流」的停机握手（见 README/计划）。
        let _ = crate::dispatch::on_main_thread(move || drop(instance));
    }
}

/// 在**当前线程**上激活实例。抽成函数是为了「主线程派发失败时退回就地执行」能重试。
///
/// 参数取 [`SharedInstance`]（而不是裸 `Arc`）是**故意的**：闭包捕获会按「最小捕获」拆分
/// 字段，传裸 `Arc` 时闭包只捕获那个 `!Send` 的 Arc，`Send` 约束就过不了。
fn activate_here(
    instance: &SharedInstance,
    config: PluginAudioConfiguration,
) -> Result<StoppedPluginAudioProcessor<ClapHost>, HostError> {
    let mut instance = instance
        .0
        .lock()
        .map_err(|_| HostError::Load("插件实例锁中毒".into()))?;
    instance
        .activate(|_, _| (), config)
        .map_err(|e| HostError::Load(e.to_string()))
}

impl ClapEditorHandle {
    /// 服务插件发起的 `request_callback`：在主线程上叫一次 `clap_plugin.on_main_thread()`。
    ///
    /// 由前端的编辑器 ticker 定期调用（无窗口的实例也要服务）。**必须在主线程调用。**
    pub fn service_requests(&self) {
        if self.callback_requested.swap(false, Ordering::AcqRel) {
            if let Some(instance) = self.instance.as_deref() {
                if let Ok(mut instance) = instance.lock() {
                    instance.call_on_main_thread_callback();
                }
            }
        }
    }

    /// 这个界面需要宿主提供原生视图吗？需要的话给一个**窗口尺寸**。
    ///
    /// 只支持「嵌入」的插件（如 KR-106 的 CLAP）返回 `Some(尺寸)`；支持浮动的返回 `None`
    /// （窗口归插件自己管）。尺寸取**插件自报值**，不是拍脑袋的常量——宿主窗口就照它开，
    /// 两边不一致用户一眼就能看出来。
    ///
    /// **必须在主 / UI 线程调用。**
    pub fn host_view_size(&self) -> Option<(u32, u32)> {
        let mut instance = self.instance.as_deref()?.lock().ok()?;
        {
            let handle = instance.plugin_handle();
            let gui = handle.get_extension::<PluginGui>()?;
            let api_type = default_api_type();
            // 支持浮动就不用宿主操心窗口。
            if gui.is_api_supported(&handle, GuiConfiguration { api_type, is_floating: true }) {
                return None;
            }
            if !gui.is_api_supported(&handle, GuiConfiguration { api_type, is_floating: false }) {
                return None;
            }
        }
        Some(probe_embedded_size(&mut instance, EMBEDDED_WINDOW_HINT))
    }

    /// 装上「插件要求改宿主窗口大小」的回调（宿主用它在 `request_resize` 里改窗口）。
    pub fn set_resize_handler(&self, handler: ResizeFn) {
        if let Ok(mut slot) = self.resize.lock() {
            *slot = Some(handler);
        }
    }

    /// 打开插件的原生界面。
    ///
    /// - 插件支持**浮动**：窗口由它自己创建，`host_view` 忽略；
    /// - 插件只支持**嵌入**：必须给 `host_view`（宿主窗口的原生视图句柄），
    ///   插件把自己的界面塞进那个视图里。
    ///
    /// **必须在主 / UI 线程调用。**
    pub fn open_window(
        &self,
        title: &str,
        host_view: Option<usize>,
    ) -> Result<ClapGuiWindow, HostError> {
        let mut instance = self
            .instance
            .as_deref()
            .ok_or_else(|| HostError::Editor("编辑器句柄已释放".into()))?
            .lock()
            .map_err(|_| HostError::Editor("插件实例锁中毒".into()))?;
        let handle = instance.plugin_handle();
        let gui = handle.get_extension::<PluginGui>().ok_or(HostError::NoEditor)?;

        let api_type = default_api_type();
        let floating = GuiConfiguration { api_type, is_floating: true };
        let embedded = GuiConfiguration { api_type, is_floating: false };
        // 优先浮动（宿主少管一层窗口）；不行再走嵌入。
        let config = if gui.is_api_supported(&handle, floating) {
            floating
        } else if gui.is_api_supported(&handle, embedded) {
            embedded
        } else {
            return Err(HostError::EditorUnsupported);
        };

        gui.create(&handle, config)
            .map_err(|e: GuiError| HostError::Editor(e.to_string()))?;

        if config.is_floating {
            let title = std::ffi::CString::new(title).unwrap_or_default();
            gui.suggest_title(&handle, &title);
        } else {
            // 嵌入模式：把插件的视图挂到宿主提供的原生视图上。
            let Some(view) = host_view else {
                return Err(HostError::EditorNeedsHostWindow);
            };
            // SAFETY: 指针来自宿主自己的窗口（内容视图），只要那个窗口还活着就一直有效；
            // 宿主保证在窗口销毁前调用 `close()`。
            unsafe {
                gui.set_parent(
                    &handle,
                    clack_extensions::gui::Window::from_generic_ptr(
                        config.api_type,
                        view as *mut std::ffi::c_void,
                    ),
                )
            }
            .map_err(|e: GuiError| HostError::Editor(e.to_string()))?;

            // 插件在自己的视图里画多大，由它自己说了算；**宿主窗口必须跟着它走**，
            // 否则就是「插件界面一个尺寸、窗口另一个尺寸」。旧代码只把尺寸原样
            // `set_size` 回插件，窗口却还停在初始尺寸上——两边自然对不上。
            let size = gui.get_size(&handle).unwrap_or(GuiSize {
                width: EMBEDDED_WINDOW_HINT.0,
                height: EMBEDDED_WINDOW_HINT.1,
            });
            if let Ok(slot) = self.resize.lock() {
                if let Some(handler) = slot.as_ref() {
                    handler(size.width, size.height);
                }
            }
            let _ = gui.set_size(&handle, size);
        }

        gui.show(&handle)
            .map_err(|e: GuiError| HostError::Editor(e.to_string()))?;

        drop(instance);
        Ok(ClapGuiWindow { handle: self.clone(), gui })
    }
}

/// 本平台最常用的 GUI API。
fn default_api_type() -> clack_extensions::gui::GuiApiType<'static> {
    use clack_extensions::gui::GuiApiType;
    #[cfg(target_os = "macos")]
    return GuiApiType::COCOA;
    #[cfg(target_os = "windows")]
    return GuiApiType::WIN32;
    #[cfg(all(unix, not(target_os = "macos")))]
    return GuiApiType::X11;
}

/// 一个打开着的 CLAP 插件窗口。
pub struct ClapGuiWindow {
    handle: ClapEditorHandle,
    gui: PluginGui,
}

impl ClapGuiWindow {
    /// 每帧调用：让插件有机会跑它的「主线程回调」（部分插件的 GUI 依赖它）。
    pub fn service(&self) {
        if let Some(instance) = self.handle.instance.as_deref() {
            if let Ok(mut instance) = instance.lock() {
                instance.call_on_main_thread_callback();
            }
        }
    }

    /// 插件是否已经关掉了自己的窗口。
    pub fn closed_by_user(&self) -> bool {
        self.handle.gui_closed.load(Ordering::Relaxed)
    }

    pub fn is_open(&self) -> bool {
        !self.closed_by_user()
    }

    /// 收起并销毁窗口（所有关闭路径都必须走它，别直接丢）。
    ///
    /// 顺序按 CLAP 的期望来：先 `hide()` 让插件把界面收起来，再 `destroy()` 释放。
    /// **销毁必须发生在宿主窗口消失之前**：插件在 `destroy()` 之前一直持有
    /// `set_parent` 给它的原生视图指针，窗口先没了它就成了悬空指针，之后一碰就崩。
    pub fn close(self) {
        if let Some(instance) = self.handle.instance.as_deref() {
            if let Ok(mut instance) = instance.lock() {
                let handle = instance.plugin_handle();
                let _ = self.gui.hide(&handle);
                self.gui.destroy(&handle);
            }
        }
    }
}

/// 读取插件自报的参数表。
fn collect_params(instance: &mut PluginInstance<ClapHost>) -> Vec<ClapParamMeta> {
    let handle = instance.plugin_handle();
    let Some(params) = handle.get_extension::<PluginParams>() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for index in 0..params.count(&handle) {
        let mut buffer = ParamInfoBuffer::new();
        let Some(info) = params.get_info(&handle, index, &mut buffer) else {
            continue;
        };
        out.push(ClapParamMeta {
            clap_id: info.id,
            // CLAP 的参数 id 是整数（非 0）；字符串形式只是给工程文件用的稳定键。
            id: ParamId(info.id.get().to_string()),
            name: String::from_utf8_lossy(info.name).into_owned(),
            min: info.min_value,
            max: info.max_value,
            default: info.default_value,
            automatable: !info.flags.contains(ParamInfoFlags::IS_READONLY),
        });
    }
    out
}

/// 在**当前线程**上读插件状态。**必须在主线程调用。**
fn save_state_here(instance: &SharedInstance) -> Vec<u8> {
    let Ok(mut instance) = instance.0.lock() else { return Vec::new() };
    let handle = instance.plugin_handle();
    let Some(state) = handle.get_extension::<PluginState>() else {
        return Vec::new();
    };
    let mut bytes = Vec::new();
    if state.save(&handle, &mut bytes).is_err() {
        return Vec::new();
    }
    bytes
}

/// 在**当前线程**上写插件状态。**必须在主线程调用。**
fn load_state_here(instance: &SharedInstance, state: &[u8]) -> Result<(), PluginError> {
    let mut instance = instance
        .0
        .lock()
        .map_err(|_| PluginError::InvalidState("插件实例锁中毒".into()))?;
    let handle = instance.plugin_handle();
    let Some(ext) = handle.get_extension::<PluginState>() else {
        return Err(PluginError::InvalidState("该插件不支持状态存取".into()));
    };
    let mut reader = state;
    ext.load(&handle, &mut reader)
        .map_err(|e| PluginError::InvalidState(e.to_string()))
}

impl Plugin for ClapHostedPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: self.id.clone(),
            name: self.name.clone(),
            kind: self.kind,
            io: self.io,
            capabilities: PluginCapabilities { per_note_volume: false, has_editor: self.has_editor },
        }
    }

    fn params(&self) -> Vec<ParamInfo> {
        self.params
            .iter()
            .map(|p| ParamInfo {
                id: p.id.clone(),
                name: p.name.clone(),
                min: p.min as f32,
                max: p.max as f32,
                default: p.default as f32,
                automatable: p.automatable,
            })
            .collect()
    }

    fn get_param(&self, id: &ParamId) -> Option<f32> {
        let meta = self.params.iter().find(|p| &p.id == id)?;
        let mut instance = self.instance.lock().ok()?;
        let handle = instance.plugin_handle();
        let params = handle.get_extension::<PluginParams>()?;
        // 已入队但还没送达的变更优先（否则界面会「弹回去」）。
        if let Some((_, v)) = self.param_queue.iter().find(|(cid, _)| *cid == meta.clap_id) {
            return Some(*v as f32);
        }
        params.get_value(&handle, meta.clap_id).map(|v| v as f32)
    }

    fn set_param(&mut self, id: &ParamId, value: f32) {
        if !value.is_finite() {
            return;
        }
        if let Some(meta) = self.params.iter().find(|p| &p.id == id) {
            // 入队即可：CLAP 的参数变更必须在下一个 process 里作为事件送达。
            self.param_queue.push((meta.clap_id, value as f64));
        }
    }

    fn prepare(&mut self, sample_rate: f64, block_size: usize) {
        self.sample_rate = sample_rate;
        self.block_size = block_size.max(1);
        for ch in self.in_buf.iter_mut() {
            ch.resize(self.block_size, 0.0);
        }
        for ch in self.out_buf.iter_mut() {
            ch.resize(self.block_size, 0.0);
        }

        // `activate` 只能在主线程调（CLAP 规定），所以派发过去执行。
        // 拿回来的是 `Send` 的 `StoppedPluginAudioProcessor` 句柄——**这里不
        // `start_processing`**：那是音频线程专属，留给第一次 `process`。
        //
        // 每次重建 Player 都是全新的插件实例，所以不存在「先停掉上一次的处理」。
        let config = self.audio_config();
        let instance = SharedInstance(Arc::clone(&*self.instance));
        let fallback = SharedInstance(Arc::clone(&*self.instance));
        let activated = match crate::dispatch::on_main_thread(move || activate_here(&instance, config))
        {
            Ok(result) => result,
            // 主线程已退出：退回就地激活。线程不理想，但总比插件整条加载不出来强。
            Err(_) => activate_here(&fallback, config),
        };
        match activated {
            Ok(stopped) => self.processor = Some(PluginAudioProcessor::Stopped(stopped)),
            // 只在这里报一次（非实时线程）；失败意味着这条轨会静音。
            Err(err) => eprintln!("CLAP 插件激活失败：{err}"),
        }
    }

    fn reset(&mut self) {
        // `clap_plugin.reset()` 是**音频线程**专属，而本方法正是被音频回调调用的
        // （换位置 / 静音 / 独奏都会走到）。用它即可，**不碰实例锁**。
        //
        // 旧实现用 stop + deactivate + activate + start_processing 冒充 reset：
        // 既在错线程上调了主线程专属的 deactivate/activate，又会在实时线程上抢一把
        // 主线程正持有的锁（开插件界面时锁被持有很久）→ 卡住实时线程。
        if let Some(processor) = self.processor.as_mut() {
            processor.reset();
        }
    }

    fn process(&mut self, _ctx: &ProcessContext, audio: &mut [&mut [f32]], events: &[TimedMidiEvent]) {
        let frames = audio.first().map_or(0, |c| c.len());
        if frames == 0 || audio.len() < ENGINE_CHANNELS {
            return;
        }
        // 这一句是「我（音频后端）在哪个线程调用 process」的权威答案，必须在下面
        // `start_processing` **之前**落笔：插件会在 `start_processing` 里问
        // `is_audio_thread`，答错它就会判定宿主乱来。只在第一次真正落笔。
        let _ = self.audio_thread.set(std::thread::current().id());

        let Some(processor) = self.processor.as_mut() else { return };

        // 插件用 `request_restart` 要求重开处理：stop/start processing 都只能在音频
        // 线程调，而这里正是那条线程。
        if self.restart_requested.swap(false, Ordering::AcqRel) {
            let _ = processor.stop_processing();
        }

        // 第一次 `process` 就地开始处理。`start_processing` 是音频线程专属，而 `prepare`
        // 里的 `activate` 跑在主线程上，所以这里才是它的合法落点。
        // 失败只可能是插件拒绝开工——静音即可，**不能在实时线程上打日志**。
        if processor.ensure_processing_started().is_err() {
            return;
        }
        let Ok(started) = processor.as_started_mut() else { return };

        // ---- 输入事件 ----
        self.in_events.clear();
        for (id, value) in self.param_queue.drain(..) {
            // 全局参数变更：Pckn 用 match_all（不针对某个 port / 通道 / 音符）。
            self.in_events
                .push(&ParamValueEvent::new(0, id, Pckn::match_all(), value));
        }
        for e in events {
            match e.event {
                HostEvent::NoteOn { note, velocity } => {
                    let pckn = Pckn::new(0u16, 0u16, u16::from(note), 0u32);
                    self.in_events.push(&NoteOnEvent::new(
                        e.frame,
                        pckn,
                        f64::from(velocity.min(127)) / 127.0,
                    ));
                }
                HostEvent::NoteOff { note } => {
                    let pckn = Pckn::new(0u16, 0u16, u16::from(note), 0u32);
                    self.in_events.push(&NoteOffEvent::new(e.frame, pckn, 0.0));
                }
                // 折叠路径已在 `Player` 里完成；本适配器声明不支持 per-note 音量。
                HostEvent::NoteVolume { .. } | HostEvent::ControlChange { .. } => {}
            }
        }
        self.in_events.sort();

        // ---- 音频缓冲 ----
        if frames > self.in_buf[0].len() {
            for ch in self.in_buf.iter_mut() {
                ch.resize(frames, 0.0);
            }
        }
        if frames > self.out_buf[0].len() {
            for ch in self.out_buf.iter_mut() {
                ch.resize(frames, 0.0);
            }
        }
        let in_channels = self.io.inputs as usize;
        if in_channels > 0 {
            for i in 0..in_channels.min(ENGINE_CHANNELS) {
                self.in_buf[i][..frames].copy_from_slice(&audio[i][..frames]);
            }
        }
        for ch in self.out_buf.iter_mut() {
            ch[..frames].fill(0.0);
        }
        self.out_events.clear();

        // ---- 处理（字段级借位拆分，避免与 `self` 整体借用的冲突）----
        let Self { in_ports, out_ports, in_buf, out_buf, in_events, out_events, pending, params, .. } =
            self;

        let in_audio = in_ports.with_input_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_input_only(
                in_buf.iter_mut().map(|b| InputChannel::constant(b)),
            ),
        }]);
        let mut out_audio = out_ports.with_output_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_output_only(
                out_buf.iter_mut().map(|b| b.as_mut_slice()),
            ),
        }]);

        let input_events = in_events.as_input();
        let mut output_events = out_events.as_output();

        let result = started.process(
            &in_audio,
            &mut out_audio,
            &input_events,
            &mut output_events,
            None,
            None,
        );

        // ---- 写回 ----
        if result.is_ok() {
            for i in 0..ENGINE_CHANNELS {
                let n = frames.min(out_buf[i].len());
                audio[i][..n].copy_from_slice(&out_buf[i][..n]);
            }
        } else {
            for ch in audio.iter_mut() {
                ch[..frames].fill(0.0);
            }
        }

        // ---- 插件侧参数变化（界面拧旋钮）----
        pending.clear();
        for event in out_events.iter() {
            let Some(e) = event.as_event::<ParamValueEvent>() else { continue };
            let Some(clap_id) = e.param_id() else { continue };
            if let Some(meta) = params.iter().find(|p| p.clap_id == clap_id) {
                pending.push((meta.id.clone(), e.value() as f32));
            }
        }
    }

    fn save_state(&self) -> Vec<u8> {
        // 状态存取同样是主线程专属（KR-106 会核对），派发过去执行。
        let instance = SharedInstance(Arc::clone(&*self.instance));
        crate::dispatch::on_main_thread(move || save_state_here(&instance)).unwrap_or_default()
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError> {
        let instance = SharedInstance(Arc::clone(&*self.instance));
        let state = state.to_vec();
        crate::dispatch::on_main_thread(move || load_state_here(&instance, &state))
            .unwrap_or_else(|_| Err(PluginError::InvalidState("主线程已退出".into())))
    }

    fn take_param_changes(&mut self) -> Vec<(ParamId, f32)> {
        std::mem::take(&mut self.pending)
    }
}
