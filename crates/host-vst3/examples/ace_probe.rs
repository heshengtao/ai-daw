//! 复现探针：ACE Bridge 的加载 / 开界面 / 关界面在「专用插件线程」上到底卡在哪一步。
//!
//! 复刻应用的真实时序：加载与开界面都在 `plugin_thread` 上，主线程只负责下指令。
//! 每一轮都重复「加载 → 开 → 等 → 关 → 释放」，因为现场日志显示前几轮正常、第 5 轮左右才卡。
//!
//! ```text
//! cargo run -p ai-daw-host-vst3 --example ace_probe
//! cargo run -p ai-daw-host-vst3 --example ace_probe -- "ACE Bridge" 6
//! ```
//!
//! 输出全部走 stderr 并 flush；另有看门狗线程，主流程卡死时它照常打点（那一行就是卡住的位置）。

use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ai_daw_host_vst3::{CatalogPlugin, PluginEditorWindow};

thread_local! {
    /// 打开着的窗口只能活在开它的那条线程上（和宿主里的做法一致）。
    static WINDOW: std::cell::RefCell<Option<PluginEditorWindow>> =
        const { std::cell::RefCell::new(None) };
}

/// 当前进行到哪一步（看门狗读它）。
static STEP: AtomicUsize = AtomicUsize::new(0);
const STEP_NAMES: &[&str] = &[
    "准备",
    "加载（构造实例）",
    "打开界面",
    "等待（界面开着）",
    "关闭界面",
    "释放实例",
    "下一轮",
];

fn say(message: impl AsRef<str>) {
    let mut err = std::io::stderr();
    let _ = writeln!(err, "{}", message.as_ref());
    let _ = err.flush();
}

fn step(name: &str) {
    STEP.store(
        STEP_NAMES.iter().position(|n| *n == name).unwrap_or(0),
        Ordering::SeqCst,
    );
    say(format!("  >> {name}"));
}

#[cfg(target_os = "windows")]
fn list_process_windows(tag: &str) {
    use winapi::shared::minwindef::{BOOL, LPARAM, TRUE};
    use winapi::shared::windef::HWND;
    use winapi::um::processthreadsapi::GetCurrentProcessId;
    use winapi::um::winuser::{
        EnumWindows, GetClassNameW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
        IsWindowVisible,
    };

    struct Found {
        entries: Vec<String>,
    }

    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let found = &mut *(lparam as *mut Found);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid != GetCurrentProcessId() {
            return TRUE;
        }
        let mut class = [0u16; 256];
        let class_len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
        let mut title = [0u16; 512];
        let title_len = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
        let mut rect = std::mem::zeroed();
        GetWindowRect(hwnd, &mut rect);
        found.entries.push(format!(
            "{:?} class={:?} title={:?} visible={} rect=({},{} {}x{})",
            hwnd,
            String::from_utf16_lossy(&class[..class_len.max(0) as usize]),
            String::from_utf16_lossy(&title[..title_len.max(0) as usize]),
            IsWindowVisible(hwnd) != 0,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
        ));
        TRUE
    }

    let mut found = Found {
        entries: Vec::new(),
    };
    unsafe {
        EnumWindows(Some(callback), &mut found as *mut Found as LPARAM);
    }
    say(format!(
        "  [{tag}] 本进程顶层窗口 {} 个：",
        found.entries.len()
    ));
    for entry in &found.entries {
        say(format!("    {entry}"));
    }
}

#[cfg(not(target_os = "windows"))]
fn list_process_windows(_tag: &str) {}

/// 把本进程的插件窗口强行抬到最前（验证「抬窗」这套 API 在真实插件上有效）。
#[cfg(target_os = "windows")]
fn raise_plugin_windows() {
    use winapi::shared::minwindef::{BOOL, LPARAM, TRUE};
    use winapi::shared::windef::HWND;
    use winapi::um::processthreadsapi::GetCurrentProcessId;
    use winapi::um::winuser::{
        BringWindowToTop, EnumWindows, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible,
        SetForegroundWindow, SetWindowPos, ShowWindow, HWND_TOP, SWP_NOMOVE, SWP_NOSIZE,
        SWP_SHOWWINDOW, SW_RESTORE,
    };

    struct Ctx {
        raised: usize,
    }

    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam as *mut Ctx);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid != GetCurrentProcessId() || IsWindowVisible(hwnd) == 0 {
            return TRUE;
        }
        let mut class = [0u16; 128];
        let len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
        let class = String::from_utf16_lossy(&class[..len.max(0) as usize]);
        if class != "VST3PluginWindow" {
            return TRUE;
        }
        ShowWindow(hwnd, SW_RESTORE);
        SetWindowPos(
            hwnd,
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        );
        BringWindowToTop(hwnd);
        let ok = SetForegroundWindow(hwnd);
        say(format!("  [抬窗] {hwnd:?} SetForegroundWindow={ok}"));
        ctx.raised += 1;
        TRUE
    }

    let mut ctx = Ctx { raised: 0 };
    unsafe {
        EnumWindows(Some(callback), &mut ctx as *mut Ctx as LPARAM);
    }
    say(format!("  [抬窗] 共 {} 个", ctx.raised));
}

#[cfg(not(target_os = "windows"))]
fn raise_plugin_windows() {}

fn main() {
    let wanted = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "ACE Bridge".to_string());
    let rounds: usize = std::env::args()
        .nth(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(8);

    ai_daw_host_vst3::mark_main_thread();
    say(format!(
        "main = {:?}，目标插件「{wanted}」",
        std::thread::current().id()
    ));

    // 看门狗：主流程卡住时，它打出当前步骤（这就是卡死的位置）。
    std::thread::Builder::new()
        .name("watchdog".into())
        .spawn(|| {
            let mut tick = 0;
            loop {
                std::thread::sleep(Duration::from_secs(3));
                tick += 1;
                let step = STEP.load(Ordering::SeqCst);
                say(format!(
                    "  [看门狗 {tick}] 停在步骤「{}」（进程还活着）",
                    STEP_NAMES.get(step).copied().unwrap_or("?")
                ));
                // 卡在插件的模态对话框上时，这里能看见那扇窗。
                list_process_windows(&format!("看门狗 {tick}"));
            }
        })
        .expect("起看门狗");

    step("准备");
    let managed = std::path::PathBuf::from(std::env::var("APPDATA").unwrap_or_default())
        .join("com.aidaw.app/plugins/vst3");
    let catalog = ai_daw_host_vst3::scan_catalog_with(
        &[
            managed,
            std::path::PathBuf::from(r"C:\Program Files\Common Files\VST3"),
        ],
        &[],
        &[],
    );
    let Some(entry) = catalog.iter().find(|e| e.name.contains(&wanted)) else {
        say(format!("扫不到「{wanted}」，已扫到："));
        for e in &catalog {
            say(format!("  {} / {} / {}", e.name, e.id, e.path.display()));
        }
        return;
    };
    let entry: CatalogPlugin = entry.clone();
    say(format!(
        "entry = {} / {} / has_editor={}",
        entry.name, entry.id, entry.has_editor
    ));
    list_process_windows("扫描后");

    for round in 0..rounds {
        say(format!("===== 第 {round} 轮 ====="));

        step("加载（构造实例）");
        let started = Instant::now();
        let module = entry.path.clone();
        let loaded = ai_daw_host_vst3::run_on(&module, ai_daw_host_vst3::LOAD_TIMEOUT, {
            let entry = entry.clone();
            move || ai_daw_host_vst3::load_catalog_plugin_with_editor(&entry)
        });
        let (plugin, handle) = match loaded {
            Ok(Ok((plugin, Some(handle)))) => {
                say(format!("  加载完成，耗时 {:?}", started.elapsed()));
                (plugin, handle)
            }
            Ok(Ok((_, None))) => {
                say("  加载完成，但没有编辑器句柄（插件没有界面）");
                return;
            }
            Ok(Err(error)) => {
                say(format!("  加载失败：{error}"));
                continue;
            }
            Err(error) => {
                say(format!("  加载派发失败：{error}"));
                continue;
            }
        };

        step("打开界面");
        let module = entry.path.clone();
        let opened = ai_daw_host_vst3::run_on(&module, ai_daw_host_vst3::EDITOR_TIMEOUT, {
            let handle = handle.clone();
            move || {
                let started = Instant::now();
                match handle.open("ace probe", None, None) {
                    Ok(window) => {
                        WINDOW.with(|slot| *slot.borrow_mut() = Some(window));
                        Ok(started.elapsed())
                    }
                    Err(error) => Err(format!("{error}（耗时 {:?}）", started.elapsed())),
                }
            }
        });
        match opened {
            Ok(Ok(elapsed)) => {
                say(format!("  open() -> Ok，耗时 {elapsed:?}"));
                list_process_windows("开界面后");
                if std::env::var_os("ACE_PROBE_RAISE").is_some() {
                    raise_plugin_windows();
                    list_process_windows("抬窗后");
                }

                step("等待（界面开着）");
                let hold = std::env::var("ACE_PROBE_HOLD")
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(2);
                std::thread::sleep(Duration::from_secs(hold));

                step("关闭界面");
                let module = entry.path.clone();
                let closed =
                    ai_daw_host_vst3::run_on(&module, ai_daw_host_vst3::EDITOR_TIMEOUT, || {
                        let started = Instant::now();
                        let window = WINDOW.with(|slot| slot.borrow_mut().take());
                        if let Some(window) = window {
                            window.close();
                        }
                        started.elapsed()
                    });
                match closed {
                    Ok(elapsed) => say(format!("  close() 完成，耗时 {elapsed:?}")),
                    Err(error) => say(format!("  close() 派发失败：{error}")),
                }
                list_process_windows("关界面后");
            }
            Ok(Err(error)) => say(format!("  open() -> Err({error})")),
            Err(error) => say(format!("  open() 派发失败：{error}")),
        }

        step("释放实例");
        let module = entry.path.clone();
        let released =
            ai_daw_host_vst3::run_on(&module, ai_daw_host_vst3::EDITOR_TIMEOUT, move || {
                let started = Instant::now();
                drop(plugin);
                started.elapsed()
            });
        match released {
            Ok(elapsed) => say(format!("  释放完成，耗时 {elapsed:?}")),
            Err(error) => say(format!("  释放派发失败：{error}")),
        }
        step("下一轮");
    }

    say("done");
}
