//! 契约薄壳：事件的定义已下沉到 `ai-daw-ipc`（引擎 ↔ Tauri ↔ MCP 唯一的 schema 来源）。
//!
//! 这里保留模块路径，让引擎内部既有的 `crate::event::…` 与对外部的
//! `ai_daw_engine::event::…` 全部照旧可用。
pub use ai_daw_ipc::event::*;
