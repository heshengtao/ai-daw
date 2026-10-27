//! CLAP 扫描探针。
//!
//! 扫描阶段最危险的动作是 `dlopen` / `dlclose` 第三方插件：坏插件在加载或卸载（`DllMain`、
//! 静态析构）时崩溃会带走整个宿主。这里把「读一个 `.clap` 的描述符」放进独立子进程，
//! 插件崩了只死这个进程，宿主把该插件记进跳过名单，继续扫下一个。
//!
//! 协议：stdout 上输出一行 `<CLAP_PROBE_MARKER><JSON数组>`。带前缀是为了让插件自己打印的
//! 日志不会和宿主混淆。不做任何实例化（不需要主线程 run loop）。

// 打包后的宿主是 GUI 子系统、没有控制台。探针若按控制台子系统启动，Windows 会为它**另开一个
// 控制台窗口**——扫描时每个 CLAP 闪一个黑窗。探针只走 stdout 管道，不需要控制台。
// debug 构建保留控制台，方便在 `tauri dev` 的终端里看诊断。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;

use ai_daw_host_vst3::scan::{clap_bundle_probe_json, CLAP_PROBE_MARKER};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: clap-scan-probe <path-to-plugin.clap>");
        return ExitCode::from(2);
    };
    let path = PathBuf::from(path);

    match clap_bundle_probe_json(&path) {
        Ok(json) => {
            println!("{CLAP_PROBE_MARKER}{json}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("failed to introspect {}: {error}", path.display());
            ExitCode::FAILURE
        }
    }
}
