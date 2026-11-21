#![forbid(unsafe_code)]
//! AI DAW 的**扩展插件**运行时。
//!
//! 这里的东西与 VST3 / CLAP（DSP 插件）无关：扩展是 npm 包，给工作室加 MCP 工具、右栏面板、
//! 供应商与内容包。设计见 `docs/superpowers/specs/2026-11-18-extensions-architecture-design.md`。
//!
//! 分层：
//!
//! - [`manifest`]：包格式（`package.json` 的 `aidaw` 字段 + `aidaw.json`）；
//! - [`state`]：已装清单与设置（`<配置目录>/extensions.json`）；
//! - [`registry`]：工具 / 面板 / 内容包的注册表（唯一真相，MCP 与界面都读它）；
//! - [`host`]：一个扩展一个线程的 QuickJS 宿主；
//! - [`market`]：市场索引（`index.json`）的拉取、校验、缓存与搜索；
//! - [`npm`] / [`installer`]：从 npm registry 装包、校验、解压、落盘；
//! - [`manager`]：把上面几层缝在一起的入口。
//!
//! **不依赖 daemon**：需要执行宿主命令时通过 [`bridge::Bridge`] 回调，由 daemon 注入。

pub mod bridge;
pub mod error;
pub mod host;
pub mod index_format;
pub mod installer;
pub mod manifest;
pub mod manager;
pub mod market;
pub mod npm;
pub mod range;
pub mod registry;
pub mod search;
pub mod state;

pub use error::{ExtError, ExtResult};
pub use manager::{Manager, ManagerConfig};
