#![forbid(unsafe_code)]

//! AI DAW 的后端：工程模型、音频引擎、插件宿主、本地 AI、MCP 与 HTTP 服务。
//!
//! 这个 crate **不拥有窗口**：需要原生能力（对话框、插件编辑器窗口、主线程泵）时
//! 通过 [`host::Host`] 拿，桌面端与无窗口进程各给一份实现。所以同一套命令实现
//! 既服务 webview，也服务 HTTP 上的浏览器客户端。

pub mod abc;
pub mod agent;
pub mod ai;
pub mod articulation_map;
pub mod assets;
pub mod audio;
pub mod backends;
pub mod boot;
pub mod bus;
pub mod capabilities;
pub mod cli;
pub mod cloud_music;
pub mod commands;
pub mod cue_sheet;
pub mod daemon;
pub mod factory_presets;
pub mod history;
pub mod host;
pub mod job;
pub mod logging;
pub mod main_thread;
pub mod manager;
pub mod mcp;
pub mod midi_import;
pub mod midi_input;
pub mod models;
pub mod paths;
pub mod pick;
pub mod plugin_dirs;
pub mod plugin_gui;
pub mod plugin_info;
pub mod plugins;
pub mod presets;
pub mod quarantine;
pub mod runtimes;
pub mod server;
pub mod session;
pub mod singing;
pub mod sound_memories;
pub mod state;
pub mod svc;
pub mod tauri_api;
pub mod voices;
pub mod yue;
pub mod yue_engine;
pub mod yue_lyrics;
pub mod yue_midi;
pub mod yue_transcribe;

use std::sync::OnceLock;
use std::time::Instant;

/// 进程入口时刻：所有「启动到哪一步花了多久」都以此为基准写进日志。
static BOOT: OnceLock<Instant> = OnceLock::new();

/// 在进程入口调用一次（桌面壳的 `run()` 与 `ai-daw` 的 `main()` 各调一次）。
pub fn mark_boot() {
    let _ = BOOT.set(Instant::now());
}

/// 距进程入口过了多少毫秒。
pub fn boot_ms() -> u128 {
    BOOT.get().map(|t| t.elapsed().as_millis()).unwrap_or(0)
}
