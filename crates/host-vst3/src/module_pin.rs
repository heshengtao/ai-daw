//! 让插件模块在整个进程生命周期内常驻。
//!
//! # 为什么需要
//!
//! JUCE 系插件会在**创建它的那条线程**上建一个隐藏的消息窗口（窗口类名 `JUCEWindow`），
//! 它的窗口过程指在插件 DLL 里。最后一个实例被释放时，`vst3-host` 会 `FreeLibrary` 把 DLL
//! 卸载掉，可那个窗口还在——下一条消息（定时器、`WM_NULL`、鼠标）一投递过去，Windows 就会
//! 跳到已经解除映射的代码上：`0xC0000005`，崩溃模块显示为 `xxx.vst3_unloaded`。
//!
//! 所以这里多拿一次引用、**永不释放**：`vst3-host` 的 `FreeLibrary` 只会把计数减回去，
//! 模块仍然映射着，窗口过程依旧有效。
//!
//! 这不是权宜之计——多数 DAW 同样选择「插件模块装一次就不再卸」，正是为了避开这类
//! 「卸载后还有回调」的崩溃。代价是插件占用的镜像内存会一直留着（可执行页被共享，实际
//! 新增的常驻内存远小于 DLL 文件大小）。

use std::path::Path;

/// 把插件模块钉在内存里（Windows）。其他平台是空操作。
#[cfg(target_os = "windows")]
pub fn pin(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use winapi::um::libloaderapi::LoadLibraryW;

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `wide` 是以 NUL 结尾的 UTF-16 路径，生命周期覆盖本次调用。
    // 拿到句柄后**故意不调用 `FreeLibrary`**——这就是本模块的目的。
    unsafe {
        LoadLibraryW(wide.as_ptr());
    }
}

/// 把插件模块钉在内存里（非 Windows：不需要）。
#[cfg(not(target_os = "windows"))]
pub fn pin(_path: &Path) {}
