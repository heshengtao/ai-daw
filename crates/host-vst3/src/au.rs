//! AU（Audio Units）后端，适配成本项目统一的 [`ai_daw_plugin_api::Plugin`] trait。
//!
//! AU 只在 macOS 上存在：本文件是**跨平台门面**，真正的实现在 [`macos`] 子模块里（只在 macOS 编译）。
//! 非 macOS 上 [`scan`] 返回空、[`load`] 直接报错，调用方（[`crate::scan`]）无需写 `#[cfg]`。

use std::path::PathBuf;

use ai_daw_plugin_api::Plugin;

use crate::editor::EditorHandle;
use crate::error::HostError;
use crate::scan::ScannedPlugin;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{AuEditorHandle, AuEditorWindow, AuHostedPlugin};

/// 扫描本机 AU 插件（`.component`），额外扫描给定目录。
///
/// 只读 `<bundle>/Contents/Info.plist` 的 `AudioComponents` 数组与系统 `AudioComponent`
/// 注册表，**不实例化插件**（`has_editor` 恒为 true），所以扫描阶段不会因为坏插件崩溃。
/// 非 macOS 恒返回空。
#[cfg(target_os = "macos")]
pub fn scan(extra: &[PathBuf]) -> Vec<ScannedPlugin> {
    macos::scan(extra)
}

/// 扫描本机 AU 插件（非 macOS 平台没有 AU）。
#[cfg(not(target_os = "macos"))]
pub fn scan(_extra: &[PathBuf]) -> Vec<ScannedPlugin> {
    Vec::new()
}

/// 按插件 id（`au:<type>:<subtype>:<manufacturer>`）加载一个 AU 实例（含编辑器句柄）。
#[cfg(target_os = "macos")]
pub fn load(id: &str) -> Result<(Box<dyn Plugin>, Option<EditorHandle>), HostError> {
    macos::load(id)
}

/// AU 仅在 macOS 上可用。
#[cfg(not(target_os = "macos"))]
pub fn load(_id: &str) -> Result<(Box<dyn Plugin>, Option<EditorHandle>), HostError> {
    Err(HostError::Load("AU 插件仅在 macOS 上受支持".into()))
}
