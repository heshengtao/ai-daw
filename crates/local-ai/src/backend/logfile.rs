//! 把 `[runtime]` / `[backend]` 的关键行**同时**写进一个文件。
//!
//! 为什么要落盘：这些行的天然去处是 stderr，也就是 `tauri dev` 的终端。可一旦应用是从
//! 桌面图标 / 打包版启动的，那行「CUDA 不可用，回落 CPU EP」就永远没人看见——排查
//! 「GPU 利用率怎么不涨」时只能靠猜。写到应用配置目录下的 `backend.log`（超过 256 KB 就
//! 从头覆盖），用户把窗口跑一遍，日志自己就在那里。
//!
//! 不引依赖：Windows 用 `%APPDATA%\com.aidaw.app\backend.log`，其它平台放临时目录。

use std::io::Write as _;
use std::path::PathBuf;

/// 单个日志文件的上限；超了就从空文件重写（这些日志只用于排最近一次问题）。
const MAX_BYTES: u64 = 256 * 1024;

/// 日志文件路径（拿不到配置目录时退回临时目录）。
fn path() -> Option<PathBuf> {
    let dir = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(|base| PathBuf::from(base).join("com.aidaw.app"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("com.aidaw.app")
        })
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(|base| PathBuf::from(base).join("com.aidaw.app"))
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| PathBuf::from(home).join(".config").join("com.aidaw.app"))
            })
    };
    dir.map(|dir| dir.join("backend.log"))
}

/// 写一行：stderr（终端里能立刻看到）+ 文件（事后能翻）。
pub fn note(line: &str) {
    // 带时间戳：同一个日志文件里会有多次启动 / 多次任务，没时间就分不清谁是谁。
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("[{stamp}] {line}");
    // 终端被关掉 / 管道断开时 `writeln!` 会失败，但**不会 panic**（`eprintln!` 会）。
    let _ = writeln!(std::io::stderr(), "{line}");
    let Some(file) = path() else { return };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let too_big = std::fs::metadata(&file)
        .map(|meta| meta.len() > MAX_BYTES)
        .unwrap_or(false);
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true);
    if too_big {
        options.truncate(true);
    } else {
        options.append(true);
    }
    if let Ok(mut handle) = options.open(&file) {
        let _ = writeln!(handle, "{line}");
    }
}
