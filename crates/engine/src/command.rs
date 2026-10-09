//! 契约薄壳：命令的定义已下沉到 `daw-party-ipc`（引擎 ↔ Tauri ↔ MCP 唯一的 schema 来源）。
//!
//! 这里保留模块路径，让引擎内部既有的 `crate::command::…` 与对外部的
//! `daw_party_engine::command::…` 全部照旧可用。
pub use daw_party_ipc::command::*;
