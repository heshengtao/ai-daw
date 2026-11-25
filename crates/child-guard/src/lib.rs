//! **子进程生命周期守卫**：父进程一死，子进程跟着死。
//!
//! 为什么不用退出钩子：`RunEvent::ExitRequested` / `atexit` 只覆盖「正常退出」。控制台
//! Ctrl+C、崩溃、任务管理器结束进程都会绕过它，而外部推理引擎（`audiocpp_server`）带着
//! 几 GB 显存活下来——用户看到的就是「应用关了，显存没放」。这里不靠钩子，让**内核**收尸：
//!
//! - **Windows**：把每个子进程塞进一个 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 的 Job。
//!   父进程无论怎么死，它持有的 Job 句柄都会关闭，内核随即杀掉组里所有进程。
//! - **Unix**：spawn 前给子进程设 `PR_SET_PDEATHSIG = SIGKILL`，父进程消失时内核直接送达。
//!
//! ```no_run
//! # fn main() -> std::io::Result<()> {
//! let mut command = std::process::Command::new("audiocpp_server");
//! ai_daw_child_guard::configure(&mut command);
//! let child = command.spawn()?;
//! ai_daw_child_guard::adopt(&child);
//! # Ok(())
//! # }
//! ```
//!
//! 单独一个 crate 是因为主 crate 有 `#![forbid(unsafe_code)]`，而 Job 对象与 `prctl(2)`
//! 都要 FFI；把 unsafe 圈在这几十行里，别处的约束不受影响。

/// spawn 之前调用：给子进程挂上「父死子死」。
///
/// Unix 要写进 `Command::pre_exec`，所以必须发生在 spawn 之前；Windows 上是空操作。
pub fn configure(command: &mut std::process::Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        // SAFETY: `pre_exec` 在 fork 之后、exec 之前执行，只能调 async-signal-safe 的东西。
        // 这里只调 `prctl(2)`，符合要求。
        unsafe {
            command.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    #[cfg(not(unix))]
    {
        let _ = command;
    }
}

/// spawn 之后调用：把子进程收进「父死子死」的组。
///
/// Windows 上失败不报错：Job 建不出来（极老的系统 / 沙箱限制）时退回旧行为，
/// 总比让调用方起不来进程强。
pub fn adopt(child: &std::process::Child) {
    #[cfg(windows)]
    windows_job::adopt(child);

    #[cfg(not(windows))]
    {
        let _ = child;
    }
}

#[cfg(windows)]
mod windows_job {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    /// Job 句柄：本进程活着就一直开着（这也是「父死子死」的触发条件）。
    /// 用 `usize` 存是因为裸指针不是 `Send + Sync`，而句柄要放进 `OnceLock`。
    fn job() -> Option<HANDLE> {
        static JOB: OnceLock<usize> = OnceLock::new();
        let raw = *JOB.get_or_init(|| {
            // SAFETY: 只传空属性与空名字，句柄由我们自己持有。
            unsafe {
                let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if handle.is_null() {
                    return 0;
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    std::ptr::addr_of!(info).cast::<c_void>(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if ok == 0 {
                    let _ = CloseHandle(handle);
                    return 0;
                }
                handle as usize
            }
        });
        (raw != 0).then_some(raw as HANDLE)
    }

    pub(super) fn adopt(child: &std::process::Child) {
        let Some(job) = job() else { return };
        // SAFETY: 两个句柄都有效且由调用方 / 静态持有；assign 只读它们。
        unsafe {
            let _ = AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE);
        }
    }
}
