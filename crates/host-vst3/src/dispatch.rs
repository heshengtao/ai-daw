//! 主线程派发：把**必须在进程主线程上做**的工作（CLAP 的实例化 / 参数枚举 / `activate` /
//! 原生界面）交给宿主去执行。
//!
//! host-vst3 是库，不认识 Tauri；应用启动时装一个 [`MainThreadDispatch`] 实现进来。
//! 没装实现时（单测、命令行 example）所有派发都**就地执行**，功能不受影响——宿主的存在
//! 只影响「在哪个线程上跑」，不影响「跑不跑得起来」。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::ThreadId;

use crate::error::HostError;

/// 进程主线程 id（GUI 所在）。
static MAIN_THREAD: OnceLock<ThreadId> = OnceLock::new();

/// 登记进程主线程。
///
/// 两处用途：回答插件的 `clap_host_thread_check.is_main_thread`，以及判断「我是不是已经在
/// 主线程上了」（已在主线程时派发必须就地执行，否则自己等自己会死锁）。
///
/// 必须在应用启动时、于主线程调用一次（Tauri 的 `setup` 就在主线程上）。
pub fn mark_main_thread() {
    let _ = MAIN_THREAD.set(std::thread::current().id());
}

/// 当前线程是不是登记过的主线程。
pub fn is_on_main_thread() -> bool {
    MAIN_THREAD.get() == Some(&std::thread::current().id())
}

/// 宿主提供的「把活丢到进程主线程」的能力。
pub trait MainThreadDispatch: Send + Sync + 'static {
    /// 把 `task` 派发到主线程执行，并**阻塞到它跑完**。
    ///
    /// 返回 `false` 表示没能派发（通常是主线程已退出），调用方会退回就地执行。
    fn dispatch(&self, task: Box<dyn FnOnce() + Send + 'static>) -> bool;
}

static DISPATCH: OnceLock<Arc<dyn MainThreadDispatch>> = OnceLock::new();

/// 装上主线程派发实现。重复调用只保留第一次，返回是否**本次真的装上**。
pub fn set_main_thread_dispatch(dispatch: Arc<dyn MainThreadDispatch>) -> bool {
    DISPATCH.set(dispatch).is_ok()
}

/// 统计「因为没装派发实现而就地执行」的次数。
///
/// 排查用：应用里这个数应当恒为 0——不为 0 说明有 CLAP 调用落在了非主线程上。
pub static INLINE_FALLBACKS: AtomicUsize = AtomicUsize::new(0);

/// 在主线程上跑完 `f` 并取回结果。
///
/// - **已经在主线程上**：直接跑。自己投给自己会死锁。
/// - **没装派发实现**：就地跑，只记一笔 [`INLINE_FALLBACKS`]。单测与命令行工具走这条。
/// - **装了**：派发并阻塞等结果；主线程已退出则返回 `Err`，调用方可退回就地执行。
pub fn on_main_thread<T, F>(f: F) -> Result<T, HostError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    if is_on_main_thread() {
        return Ok(f());
    }
    let Some(dispatch) = DISPATCH.get() else {
        INLINE_FALLBACKS.fetch_add(1, Ordering::Relaxed);
        return Ok(f());
    };

    let (tx, rx) = std::sync::mpsc::channel();
    let handed_over = dispatch.dispatch(Box::new(move || {
        // 主线程可能已经不等结果了（发送失败也无所谓）。
        let _ = tx.send(f());
    }));
    if !handed_over {
        return Err(HostError::Load("主线程已退出，无法在它上面执行".into()));
    }
    rx.recv()
        .map_err(|_| HostError::Load("主线程没有把执行结果送回来".into()))
}
