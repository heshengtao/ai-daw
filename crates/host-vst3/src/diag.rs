//! 诊断日志出口。
//!
//! 宿主 crate 不依赖 Tauri，平时只往 stderr 写；应用启动时用 [`set_sink`] 把出口接到
//! `<应用配置目录>/logs/aidaw.log`，于是「哪个插件超时、哪条线程卡住」都能在日志里查到。
//! 没装 sink 时（单测、命令行工具）退回 stderr，行为不变。

use std::sync::OnceLock;

type Sink = Box<dyn Fn(&str) + Send + Sync + 'static>;

static SINK: OnceLock<Sink> = OnceLock::new();

/// 装上日志出口（只生效一次；重复调用返回 `false`）。
pub fn set_sink(sink: impl Fn(&str) + Send + Sync + 'static) -> bool {
    SINK.set(Box::new(sink)).is_ok()
}

/// 记一行。
pub fn info(message: impl AsRef<str>) {
    let message = message.as_ref();
    match SINK.get() {
        Some(sink) => sink(message),
        // 没有 sink 时写 stderr：命令行工具与单测里仍能看到。
        None => eprintln!("{message}"),
    }
}
