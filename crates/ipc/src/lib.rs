#![forbid(unsafe_code)]

//! 引擎 ↔ Tauri ↔ MCP 的**唯一契约**：工程 / 命令 / 事件的 serde schema。
//!
//! 这里只有纯数据与它们之上的纯函数（归一化、遍历）——**没有**状态、IO 或设备代码。
//! `Engine`、调度、播放、渲染都留在 `daw-party-engine`；本 crate 只描述「线上长什么样」。
//! 前端（`src/api/types.ts`）与 MCP 工具都依赖这些形状，改这里必须同步它们。
//!
//! `daw-party-engine` 用 `pub use` 薄壳把这些类型再导出，所以引擎内部与既有调用点
//! （`daw_party_engine::model::…` / `::command::…` / `::event::…`）的路径完全不变。

pub mod command;
pub mod event;
pub mod model;
