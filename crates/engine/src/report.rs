//! 契约薄壳：分析结果类型已下沉到 `ai-daw-ipc`（MCP / CLI / 界面共用的唯一形状）。
//!
//! 保留模块路径，让引擎内部的 `crate::report::…` 与对外的
//! `ai_daw_engine::report::…` 都能用。
pub use ai_daw_ipc::report::*;
