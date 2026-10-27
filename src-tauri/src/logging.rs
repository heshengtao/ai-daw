//! 极简运行日志。
//!
//! 目的只有一个：出事时能知道**最后走到了哪一步**。之前的几次崩溃（卡死、`0xC0000005`、
//! 启动即退出）都只能靠 Windows 事件日志倒推，因为应用自己什么都不写。
//!
//! 落点：`<应用配置目录>/logs/aidaw.log`（超过 2 MB 轮转成 `aidaw.1.log`）。
//! 同时接管 panic：panic 的文字也进日志，不依赖用户从终端里看 stderr。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

use tauri::Manager;

/// 日志文件路径。没装成（拿不到配置目录）时所有写入都是空操作。
static LOG: OnceLock<PathBuf> = OnceLock::new();

/// 装上日志：定文件位置、轮转、接管 panic。`setup` 一开始调一次。
pub fn init(app: &tauri::AppHandle) {
    let Ok(dir) = app.path().app_config_dir() else {
        return;
    };
    let dir = dir.join("logs");
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("aidaw.log");
    // 超过 2 MB 就轮转，只留上一份——日志是用来查最近一次事故的，不是存档。
    let too_big = fs::metadata(&path)
        .map(|meta| meta.len() > 2 * 1024 * 1024)
        .unwrap_or(false);
    if too_big {
        let _ = fs::rename(&path, dir.join("aidaw.1.log"));
    }
    let _ = LOG.set(path);

    // panic 也写进来：崩溃前最后一条正常日志 + panic 文字，足够定位。
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        info(format!("PANIC {panic}"));
        default_hook(panic);
    }));

    info(format!(
        "==== 启动 AI DAW {} ====",
        env!("CARGO_PKG_VERSION")
    ));
}

/// 记一行。写失败就算了——日志不该影响主流程。
pub fn info(message: impl AsRef<str>) {
    let Some(path) = LOG.get() else {
        return;
    };
    let stamp = chrono::Local::now().format("%m-%d %H:%M:%S%.3f");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "[{stamp}] {}", message.as_ref());
    }
}
