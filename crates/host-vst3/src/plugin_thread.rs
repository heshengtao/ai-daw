//! 每个插件模块一条专属「插件线程」：实例化和它的原生界面都住在这里。
//!
//! # 为什么必须同一条线程
//!
//! JUCE 系插件（RdPiano、ACE Bridge 都是）在 `IPlugView::attached()` 时，会把「把界面挂上去」
//! 这件事派发给它认为的**消息线程**——在 Windows 上就是**第一次碰到这个 DLL 的那条线程**。
//! 那条线程若没有消息循环，派发出去的任务永远不会执行，`attached()` 也就永不返回：整个应用
//! 无响应（Windows 事件日志里留下的是 `AppHangB1`，不是崩溃）。
//!
//! 所以同一件插件的**实例化与界面必须落在同一条线程上**。曾经的实现在「参数面板」「插件
//! 描述」这些只读路径上直接在主线程池里实例化插件，抢先把这个「消息线程」绑到了一条**短命
//! 的工作线程**上；之后真正加载 / 开界面时，插件就在等一条已经没人跑消息的线程——表现正是
//! 「ACE Bridge 打不开界面，重启后连别的插件也打不开」。
//!
//! # 为什么是「每个模块一条」而不是「全局一条」
//!
//! 全局一条线程意味着**一个插件卡住，所有插件一起完蛋**：任务队列是串行的，卡住的那条永远
//! 不会回到循环里。按模块分线程之后，坏插件只拖死它自己那一条；别的插件照常加载、照常开界面。
//! 同一个模块的多个实例仍共用一条线程（JUCE 的静态状态按 DLL 各一份，换线程反而会错位）。
//!
//! # 超时
//!
//! 线程卡住时宿主不能跟着一起等：所有跨线程操作都带超时（见 [`LOAD_TIMEOUT`] /
//! [`EDITOR_TIMEOUT`]），超时后返回 [`HostError::Timeout`]，并把该模块标记为「卡住」，
//! 后续操作直接失败而不是排队等下去。插件自己恢复（任务最终跑完）后标记自动清除。
//!
//! # 其他平台
//!
//! - **macOS**：编辑器必须是 AppKit 主线程（`vst3-host` 会直接拒绝在非主线程建 `NSWindow`），
//!   所以 [`run_on`] 退化成「派发到主线程」，行为与改动前一致。
//! - **Linux**：没有实测过，同样退化成主线程派发。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::error::HostError;

/// 实例化一个插件最多等多久。
///
/// 采样器类插件冷启动要几秒（大音色库甚至十几秒），所以给得比较宽；超过这个时间基本可以断定
/// 它卡在模态框 / 死锁上——宿主此时必须放手，否则整条音频重建都跟着停住。
pub const LOAD_TIMEOUT: Duration = Duration::from_secs(20);

/// 开 / 关一次界面最多等多久。
///
/// 正常插件开界面是几十毫秒；要一秒以上基本就是卡在模态框 / 消息线程上了。
/// 不能给太长：界面里点「打开界面」是同步等结果的，用户不该为此干站十秒。
pub const EDITOR_TIMEOUT: Duration = Duration::from_secs(6);

/// 读一次插件内部状态（重建播放器前的抢救）最多等多久。
pub const STATE_TIMEOUT: Duration = Duration::from_secs(5);

/// 把模块路径规整成查表用的键（拿不到 canonical 就用原样）。
fn module_key(module: &Path) -> PathBuf {
    std::fs::canonicalize(module).unwrap_or_else(|_| module.to_path_buf())
}

/// 模块的短名字（日志 / 线程名用）。
fn module_label(module: &Path) -> String {
    module
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| module.display().to_string())
}

pub use imp::{is_wedged, post_service, run_on};

#[cfg(target_os = "windows")]
mod imp {
    use super::*;

    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
    use std::sync::mpsc::{self, Sender};
    use std::sync::Arc;
    use std::thread;
    use std::time::Instant;

    use winapi::um::processthreadsapi::GetCurrentThreadId;
    use winapi::um::winuser::{
        DispatchMessageW, GetMessageW, PeekMessageW, PostThreadMessageW, TranslateMessage, MSG,
        PM_NOREMOVE, WM_APP,
    };

    /// 唤醒消息：投递任务后用它把阻塞在 `GetMessageW` 上的线程叫醒。
    const WM_WAKE: u32 = WM_APP + 1;

    type Task = Box<dyn FnOnce() + Send + 'static>;

    /// 一条插件线程，以及它的任务队列与健康状态。
    pub struct HostThread {
        /// 模块路径（日志用）。
        label: String,
        /// 投递任务的队列。
        queue: Sender<Task>,
        /// 线程 id（`GetCurrentThreadId`），用于「已经在这条线程上」的判定与唤醒。
        id: AtomicU32,
        /// 已投递但还没跑完的任务数。
        outstanding: AtomicUsize,
        /// 上一次等待超时：线程可能卡在插件的某次调用里。
        wedged: AtomicBool,
        /// 是否已经排了一个「服务窗口」任务（避免 ticker 每 80ms 堆积）。
        service_queued: AtomicBool,
    }

    impl HostThread {
        fn is_current(&self) -> bool {
            let id = self.id.load(Ordering::Acquire);
            id != 0 && id == unsafe { GetCurrentThreadId() }
        }

        fn wake(&self) {
            let id = self.id.load(Ordering::Acquire);
            if id != 0 {
                // SAFETY: 线程 id 来自 `GetCurrentThreadId`，消息队列已在建线程时建好。
                unsafe {
                    PostThreadMessageW(id, WM_WAKE, 0, 0);
                }
            }
        }
    }

    fn registry() -> &'static Mutex<HashMap<PathBuf, Arc<HostThread>>> {
        static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Arc<HostThread>>>> = OnceLock::new();
        REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// 取（必要时启动）某个模块的插件线程。
    fn host_for(module: &Path) -> Result<Arc<HostThread>, HostError> {
        let key = module_key(module);
        if let Ok(map) = registry().lock() {
            if let Some(host) = map.get(&key) {
                return Ok(Arc::clone(host));
            }
        }

        let label = module_label(module);
        let (queue, receiver) = mpsc::channel::<Task>();
        let (ready_tx, ready_rx) = mpsc::channel::<u32>();
        let host = Arc::new(HostThread {
            label: label.clone(),
            queue,
            id: AtomicU32::new(0),
            outstanding: AtomicUsize::new(0),
            wedged: AtomicBool::new(false),
            service_queued: AtomicBool::new(false),
        });

        let for_thread = Arc::clone(&host);
        let thread_name = format!("ai-daw-vst3-{label}");
        thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                // 第一次 PeekMessage 会把线程消息队列建起来，后面 PostThreadMessage 才投得进来。
                // SAFETY: 标准 Win32 消息队列初始化。
                unsafe {
                    let mut msg: MSG = std::mem::zeroed();
                    PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
                }
                let id = unsafe { GetCurrentThreadId() };
                for_thread.id.store(id, Ordering::Release);
                let _ = ready_tx.send(id);

                loop {
                    // 先清空任务队列；闭包里可能又投递任务，所以循环到空为止。
                    while let Ok(task) = receiver.try_recv() {
                        task();
                    }
                    // 没有任务就等消息：插件的窗口消息、以及唤醒用的 WM_WAKE 都从这里进来。
                    // SAFETY: 标准 Win32 消息循环。
                    unsafe {
                        let mut msg: MSG = std::mem::zeroed();
                        let got = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
                        if got <= 0 {
                            break; // WM_QUIT 或出错：应用在退出
                        }
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            })
            .map_err(|error| HostError::Load(format!("无法为「{label}」启动插件线程：{error}")))?;

        // 等线程把消息队列建好，否则紧接着的第一次投递会「投了但没人醒」。
        if ready_rx.recv_timeout(Duration::from_secs(2)).is_err() {
            return Err(HostError::Load(format!(
                "插件线程「{label}」没能就绪（消息队列没建起来）"
            )));
        }

        registry()
            .lock()
            .map_err(|_| HostError::Load("插件线程表锁中毒".into()))?
            .insert(key, Arc::clone(&host));
        Ok(host)
    }

    /// 在 `module` 的插件线程上跑一段并等结果（带超时）。
    ///
    /// - **已经在这条线程上**：就地执行（自己投给自己会死锁）；
    /// - **该模块已被判定卡住**：立刻返回 [`HostError::Timeout`]，不再排队；
    /// - 否则投递过去，最多等 `timeout`。
    pub fn run_on<T: Send + 'static>(
        module: &Path,
        timeout: Duration,
        task: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, HostError> {
        let host = host_for(module)?;
        if host.is_current() {
            return Ok(task());
        }
        if host.wedged.load(Ordering::Acquire) {
            return Err(HostError::Timeout(format!(
                "{}：上一次调用仍未返回，不再排队等待",
                host.label
            )));
        }

        host.outstanding.fetch_add(1, Ordering::AcqRel);
        let (tx, rx) = mpsc::channel();
        let for_task = Arc::clone(&host);
        let queued = host.queue.send(Box::new(move || {
            let result = task();
            let _ = tx.send(result);
            // 队列清空说明线程又活过来了：清掉「卡住」标记，下次可以正常重试。
            if for_task.outstanding.fetch_sub(1, Ordering::AcqRel) == 1 {
                for_task.wedged.store(false, Ordering::Release);
            }
        }));
        if queued.is_err() {
            host.outstanding.fetch_sub(1, Ordering::AcqRel);
            return Err(HostError::Load(format!("{}：插件线程已退出", host.label)));
        }
        host.wake();

        let started = Instant::now();
        match rx.recv_timeout(timeout) {
            Ok(value) => Ok(value),
            Err(_) => {
                host.wedged.store(true, Ordering::Release);
                Err(HostError::Timeout(format!(
                    "{}：等了 {} 秒仍未返回（插件卡在构造 / 界面 / 模态框上）",
                    host.label,
                    started.elapsed().as_secs().max(1)
                )))
            }
        }
    }

    /// 往 `module` 的插件线程排一个「服务窗口」任务，**不等待**。
    ///
    /// 同一时刻最多排一个：插件线程卡住时不会堆积任务，ticker 也不会被拖住。
    pub fn post_service(module: &Path, task: impl FnOnce() + Send + 'static) {
        let Ok(host) = host_for(module) else {
            return;
        };
        if host.is_current() {
            task();
            return;
        }
        if host.wedged.load(Ordering::Acquire) {
            return;
        }
        if host.service_queued.swap(true, Ordering::AcqRel) {
            return; // 上一拍还没跑
        }
        let for_task = Arc::clone(&host);
        let queued = host.queue.send(Box::new(move || {
            for_task.service_queued.store(false, Ordering::Release);
            task();
        }));
        if queued.is_err() {
            host.service_queued.store(false, Ordering::Release);
            return;
        }
        host.wake();
    }

    /// 这个模块是不是已经被判定「卡住」（只看状态，不会启动线程）。
    pub fn is_wedged(module: &Path) -> bool {
        registry()
            .lock()
            .ok()
            .and_then(|map| map.get(&module_key(module)).map(Arc::clone))
            .is_some_and(|host| host.wedged.load(Ordering::Acquire))
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    //! 非 Windows：没有独立线程，插件线程就是主线程（macOS 的 AppKit 要求）。

    use super::*;

    pub fn run_on<T: Send + 'static>(
        _module: &Path,
        _timeout: Duration,
        task: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, HostError> {
        crate::dispatch::on_main_thread(task)
    }

    pub fn post_service(_module: &Path, task: impl FnOnce() + Send + 'static) {
        let _ = crate::dispatch::on_main_thread(task);
    }

    pub fn is_wedged(_module: &Path) -> bool {
        false
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    use std::time::Instant;

    fn fake(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    /// 一个插件卡住**只**拖死它自己的模块线程：同一个模块快速失败，别的模块照常干活。
    ///
    /// 这是「ACE Bridge 打不开界面之后，连别的插件都打不开」那条 bug 的护栏。
    #[test]
    fn a_stuck_module_does_not_block_other_modules() {
        let stuck = fake("aidaw-test-stuck.vst3");
        let healthy = fake("aidaw-test-healthy.vst3");

        // 任务要卡 800ms，但只等 100ms → 判定超时。
        let timed_out = run_on(&stuck, Duration::from_millis(100), || {
            std::thread::sleep(Duration::from_millis(800));
        });
        assert!(
            matches!(timed_out, Err(HostError::Timeout(_))),
            "卡住的任务必须超时返回，而不是一起挂死：{timed_out:?}"
        );

        // 卡住期间再找它：快速失败，不排队、不再等第二次超时。
        let at = Instant::now();
        let again = run_on(&stuck, Duration::from_secs(2), || 1usize);
        assert!(
            matches!(again, Err(HostError::Timeout(_))),
            "已判定卡住的模块应快速失败：{again:?}"
        );
        assert!(
            at.elapsed() < Duration::from_millis(200),
            "不该再等一轮超时"
        );

        // 别的模块完全不受影响。
        let at = Instant::now();
        let value = run_on(&healthy, Duration::from_secs(2), || 42usize);
        assert_eq!(value.expect("另一个模块应正常返回"), 42);
        assert!(
            at.elapsed() < Duration::from_secs(1),
            "另一个模块不该被卡住的模块拖住（用了 {:?}）",
            at.elapsed()
        );
    }

    /// 同一个模块始终落在同一条线程上（JUCE 的「消息线程」依赖这一点）。
    #[test]
    fn a_module_keeps_one_stable_thread() {
        let module = fake("aidaw-test-stable.vst3");
        let outer = std::thread::current().id();
        let first = run_on(&module, Duration::from_secs(2), || {
            std::thread::current().id()
        })
        .expect("任务应执行");
        assert_ne!(outer, first, "任务应该跑在模块自己的线程上");
        let second = run_on(&module, Duration::from_secs(2), || {
            std::thread::current().id()
        })
        .expect("任务应执行");
        assert_eq!(first, second, "同一个模块的两次调用必须在同一条线程上");
    }
}
