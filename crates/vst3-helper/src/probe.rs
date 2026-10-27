// Copied from the `vst3-host` crate 0.9.0 (`src/bin/vst3-host-probe.rs`),
// Copyright Helge Sverre, MIT License. See NOTICE.
//
// 为什么把它抄进来：`vst3-host` 的「安全扫描」（`discover_plugins_safe`）会为**每个**
// 插件起一个 `vst3-host-probe` 子进程来读元数据；插件在探测中崩溃（abort / 访问违例）
// 或弹模态框卡住时，死的是这个子进程，宿主只把它记成「跳过」。Cargo 不会构建依赖
// crate 的 bin target，Tauri 又需要它作为一个可打包的 sidecar，所以这里保持与上游逐字
// 一致，仅用于扫描隔离。
//
//! VST3 discovery probe process.
//!
//! A deliberately tiny, single-shot binary used by the crate's crash-resistant discovery
//! path ([`vst3_host::discovery::discover_plugins_safe`]). It takes a single `.vst3` path
//! as its only argument, runs the library's normal in-process introspection
//! ([`vst3_host::get_detailed_plugin_info`]) on it, prints the result as one line of JSON
//! to stdout on success, and exits non-zero on any failure.
//!
//! The whole point is process isolation for the *introspection* step: some installed
//! plugins call `abort()` or trigger a pure-virtual call while being instantiated, which
//! terminates the process. By doing that work here, in a child process, a crash kills
//! *this* process — the parent scanner sees a non-zero exit / signal death and skips the
//! plugin instead of dying itself. A Rust `catch_unwind` in the parent cannot achieve this
//! (an `abort()` does not unwind), which is why a separate process is required.
//!
//! This binary is intentionally independent of the run-time isolation IPC
//! (`vst3-host-helper`): it speaks no protocol, holds no state, and exits immediately.

// 打包后的宿主是 **GUI 子系统**、自己没有控制台。探针若按控制台子系统启动，Windows 会给它
// **另开一个控制台窗口**——扫描时每个插件闪一个黑窗。探针只走 stdin/stdout 管道，不需要控制台。
// debug 构建保留控制台，方便在 `tauri dev` 的终端里看诊断输出。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    // argv[1] is the plugin path. Anything else is a usage error.
    let mut args = std::env::args_os().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: vst3-host-probe <path-to-plugin.vst3>");
        return ExitCode::from(2);
    };
    let path = PathBuf::from(path);

    match vst3_host::get_detailed_plugin_info(&path) {
        Ok(info) => match serde_json::to_string(&info) {
            Ok(json) => {
                // One JSON object on one line — the parent reads exactly this.
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("failed to serialize plugin info: {e}");
                ExitCode::from(3)
            }
        },
        Err(e) => {
            eprintln!("failed to introspect {}: {e}", path.display());
            ExitCode::FAILURE
        }
    }
}
