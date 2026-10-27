//! 插件原生窗口的「抬到最前 / 摆到用户眼前 / 改标题」。
//!
//! # 为什么需要
//!
//! VST3 的编辑器窗口是 `vst3-host` 用 `CreateWindowExW` + `ShowWindow(SW_SHOW)` 开的：
//! 它**不会**激活窗口，也不会调整 Z 序。在 Windows 上，一个非前台线程新开的窗口就这样
//! 老老实实待在已经最大化的主窗口后面——用户点「打开界面」看不到任何东西，还以为插件坏了。
//! CLAP 的浮动窗口同理，那扇窗完全归插件自己管，宿主只能事后把它抬起来。
//!
//! 这里做三件事，全部是「尽力而为」，失败也不影响插件本身：
//! 1. 开窗前后各拍一张「本进程顶层窗口」的快照，差集里认出插件刚开的那扇；
//! 2. 把它摆到主窗口所在显示器的中央（否则 `CW_USEDEFAULT` 会把它叠在左上角、被大窗口盖住）；
//! 3. 抬到最前并抢前台（含 `AttachThreadInput` 兜底——后台线程直接 `SetForegroundWindow`
//!    通常会被前台锁拦下）。
//!
//! 只有 Windows 有实现；其他平台是空操作（macOS 的编辑器由 AppKit 自己 `makeKeyAndOrderFront`）。

/// 一个原生窗口句柄（`HWND` 的 `isize` 形式，便于跨线程传递）。
pub type NativeWindow = isize;

/// 拍一张「本进程当前所有顶层窗口」的快照，用于开窗后做差集。
#[cfg(target_os = "windows")]
pub fn snapshot() -> Vec<NativeWindow> {
    use winapi::shared::minwindef::{BOOL, LPARAM, TRUE};
    use winapi::shared::windef::HWND;
    use winapi::um::processthreadsapi::GetCurrentProcessId;
    use winapi::um::winuser::{EnumWindows, GetWindowThreadProcessId};

    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let windows = &mut *(lparam as *mut Vec<NativeWindow>);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == GetCurrentProcessId() {
            windows.push(hwnd as NativeWindow);
        }
        TRUE
    }

    let mut windows: Vec<NativeWindow> = Vec::new();
    // SAFETY: 回调只往 `windows` 里 push，生命周期覆盖本次 `EnumWindows`。
    unsafe {
        EnumWindows(
            Some(collect),
            &mut windows as *mut Vec<NativeWindow> as LPARAM,
        );
    }
    windows
}

#[cfg(not(target_os = "windows"))]
pub fn snapshot() -> Vec<NativeWindow> {
    Vec::new()
}

/// 在快照差集里认出插件刚开的那扇窗。
///
/// 优先认 `vst3-host` 的窗口类（`VST3PluginWindow`）；CLAP 浮动窗口的类名由插件决定，
/// 这时退回「可见、尺寸像一扇界面」的最大一扇。输入法、隐藏的消息窗一律排除。
#[cfg(target_os = "windows")]
pub fn find_opened(before: &[NativeWindow]) -> Option<NativeWindow> {
    use winapi::shared::windef::RECT;
    use winapi::um::winuser::{GetClassNameW, GetWindowRect, IsWindowVisible};

    let mut best: Option<(NativeWindow, i64, bool)> = None;
    for window in snapshot() {
        if before.contains(&window) {
            continue;
        }
        // SAFETY: 句柄来自刚拍的快照；只读查询，不会改变窗口状态。
        unsafe {
            if IsWindowVisible(window as _) == 0 {
                continue;
            }
            let mut class = [0u16; 256];
            let len = GetClassNameW(window as _, class.as_mut_ptr(), class.len() as i32);
            let class = String::from_utf16_lossy(&class[..len.max(0) as usize]);
            if is_helper_window(&class) {
                continue;
            }
            let mut rect: RECT = std::mem::zeroed();
            if GetWindowRect(window as _, &mut rect) == 0 {
                continue;
            }
            let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
            if width < 100 || height < 80 {
                continue; // 隐藏消息窗 / 工具条之类
            }
            let preferred = class == "VST3PluginWindow";
            let area = i64::from(width) * i64::from(height);
            if best.map_or(true, |(_, best_area, best_preferred)| {
                (preferred && !best_preferred) || (preferred == best_preferred && area > best_area)
            }) {
                best = Some((window, area, preferred));
            }
        }
    }
    best.map(|(window, _, _)| window)
}

#[cfg(not(target_os = "windows"))]
pub fn find_opened(_before: &[NativeWindow]) -> Option<NativeWindow> {
    None
}

/// 这些类名不是「插件界面」，认窗时要跳过。
///
/// **不要**按 `JUCE_*` 过滤：JUCE 插件的浮动界面本身就是 `JUCE_<hash>` 类名的顶层窗口
/// （ACE 那些 136x39 的隐藏消息窗也是这个类，但它们 `IsWindowVisible == 0`，靠可见性就能筛掉）。
#[cfg(target_os = "windows")]
fn is_helper_window(class: &str) -> bool {
    class == "IME"
        || class == "MSCTFIME UI"
        || class.starts_with("Sogou")
        || class.starts_with("SoPY")
        || class == "VST3EmbeddedEditor"
}

/// 改窗口标题（`vst3-host` 只会写「插件名 - VST3」，宿主换成「轨道名 - 插件名」）。
#[cfg(target_os = "windows")]
pub fn set_title(window: NativeWindow, title: &str) {
    use winapi::um::winuser::SetWindowTextW;

    let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    // SAFETY: `wide` 以 NUL 结尾，生命周期覆盖本次调用。
    unsafe {
        SetWindowTextW(window as _, wide.as_ptr());
    }
}

#[cfg(not(target_os = "windows"))]
pub fn set_title(_window: NativeWindow, _title: &str) {}

/// 摆位置 + 改标题 + 抬到最前（开窗成功后的统一收尾）。
pub fn present(window: NativeWindow, title: &str, anchor: Option<NativeWindow>) {
    set_title(window, title);
    place(window, anchor);
    raise(window);
}

/// 把窗口摆到 `anchor` 所在显示器的中央。
///
/// 不摆的话，`CW_USEDEFAULT` 会把它放在左上角（约 (150,150)）——主窗口一最大化就整扇被盖住，
/// 这正是「点了打开界面却什么也没出现」的直接原因。
#[cfg(target_os = "windows")]
pub fn place(window: NativeWindow, anchor: Option<NativeWindow>) {
    use winapi::shared::windef::{HWND, RECT};
    use winapi::um::winuser::{
        GetMonitorInfoW, GetWindowRect, MonitorFromWindow, SetWindowPos, MONITORINFO,
        MONITOR_DEFAULTTONEAREST, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    };

    // SAFETY: 句柄都来自本进程；只读查询 + 一次移动窗口。
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        if GetWindowRect(window as _, &mut rect) == 0 {
            return;
        }
        let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
        if width <= 0 || height <= 0 {
            return;
        }

        let reference: HWND = anchor
            .map(|handle| handle as HWND)
            .unwrap_or(std::ptr::null_mut());
        let monitor = MonitorFromWindow(reference, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return;
        }
        let work = info.rcWork;
        let (work_width, work_height) = (work.right - work.left, work.bottom - work.top);

        // 比工作区还大的界面贴左上角，别让标题栏跑到屏幕外。
        let x = work.left + ((work_width - width) / 2).max(0);
        let y = work.top + ((work_height - height) / 2).max(0);

        // 已经在目标位置就别动：用户把窗口拖走后再开同一插件，不该被强行拉回来。
        if (rect.left - x).abs() > 4 || (rect.top - y).abs() > 4 {
            SetWindowPos(
                window as _,
                std::ptr::null_mut(),
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn place(_window: NativeWindow, _anchor: Option<NativeWindow>) {}

/// 把窗口抬到最前并尽量抢到前台。
///
/// `SetForegroundWindow` 在前台锁生效时会被忽略（调用方不是前台进程）；此时把当前线程临时
/// 接到前台线程的输入队列上再试一次——这是宿主替插件窗口抢焦点时的标准做法。
#[cfg(target_os = "windows")]
pub fn raise(window: NativeWindow) {
    use winapi::um::processthreadsapi::GetCurrentThreadId;
    use winapi::um::winuser::{
        AttachThreadInput, BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId,
        IsIconic, SetForegroundWindow, SetWindowPos, ShowWindow, HWND_TOP, SWP_NOMOVE, SWP_NOSIZE,
        SWP_SHOWWINDOW, SW_RESTORE,
    };

    // SAFETY: 句柄来自本进程；全是窗口管理调用，没有内存所有权转移。
    unsafe {
        if IsIconic(window as _) != 0 {
            ShowWindow(window as _, SW_RESTORE);
        }
        SetWindowPos(
            window as _,
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        );
        BringWindowToTop(window as _);
        if SetForegroundWindow(window as _) != 0 {
            return;
        }

        let foreground = GetForegroundWindow();
        if foreground.is_null() {
            return;
        }
        let foreground_thread = GetWindowThreadProcessId(foreground, std::ptr::null_mut());
        let this_thread = GetCurrentThreadId();
        if foreground_thread == 0 || foreground_thread == this_thread {
            return;
        }
        if AttachThreadInput(foreground_thread, this_thread, 1) != 0 {
            SetForegroundWindow(window as _);
            SetWindowPos(
                window as _,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );
            AttachThreadInput(foreground_thread, this_thread, 0);
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn raise(_window: NativeWindow) {}
