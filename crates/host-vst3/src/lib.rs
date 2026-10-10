//! VST3 / CLAP 插件宿主。
//!
//! 把 [`truce_rack`] 的插件实例适配成本项目统一的 [`ai_daw_plugin_api::Plugin`] trait：
//! 引擎其余部分不认识 VST3 / CLAP，只认那一套 ABI。
//!
//! 本 crate 是工作区里唯一允许 `unsafe` 的地方（原生窗口 / 裸指针），对上层只暴露安全 API。

pub mod clap;
pub mod dispatch;
pub mod editor;
pub mod error;
pub mod scan;
pub mod vst3;

pub use clap::{ClapHostedPlugin, GuiMode};
pub use dispatch::{
    mark_main_thread, on_main_thread, set_main_thread_dispatch, MainThreadDispatch,
};
pub use editor::{EditorHandle, PluginEditorWindow};
pub use error::HostError;
pub use scan::{
    load_catalog_plugin, load_catalog_plugin_with_editor, scan, scan_catalog, CatalogPlugin,
    ScannedPlugin,
};
pub use vst3::{Vst3EditorHandle, Vst3HostedPlugin};
