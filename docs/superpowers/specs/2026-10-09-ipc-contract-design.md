# `crates/ipc` 契约下沉 — 设计

> **状态（2026-10-09）**：已完成（见 `docs/superpowers/plans/2026-10-09-ipc-contract.md` 的执行记录）。

## 1 背景

设计文档 §5 把 `crates/ipc` 列为「命令/事件/工程 schema（serde），API 契约，前后端与未来 AI 共用」，
ROADMAP 的 A1 把它定为「解锁后续一切接口的前置」。此前 `Command` / `Event` / `Project` 的 schema 定义在
`ai-daw-engine` 内部，MCP 与 Tauri 只能间接依赖引擎，契约没有单一出处。

## 2 目标与非目标

**目标**

- 让 `crates/ipc`（`ai-daw-ipc`）成为工程 / 命令 / 事件 schema 的**唯一来源**。
- **零行为变化**：纯搬家 + 薄壳，测试通过数守恒。

**非目标**

- 不改任何类型、字段、serde 属性或归一化行为。
- 不把有状态/带 IO 的东西（`Engine`、`apply`、调度、播放、渲染、`realtime`）搬过去。
- 不把 `Command` 变成可序列化（当前它不作线上格式；Tauri 命令逐个传参、MCP 逐个映射工具）。
- 不接 CI（仅落 `deny.toml`）。

## 3 方案

- 新 crate 只依赖 `serde`（dev-dep `serde_json`），`#![forbid(unsafe_code)]`。
- 搬运清单：`model/{clip,track,transport,project}.rs`、`command.rs`、`event.rs`
  （含 `Actor`、`Event::actor()`、`Event::is_mix_only()` 与全部 `normalize_*` / `clamp_*`）。
- `ai-daw-engine` 保留**同名薄壳模块**：`pub use ai_daw_ipc::{model,command,event}::*;`，
  于是引擎内部既有的 `crate::model::…` / `crate::command::…` / `crate::event::…` 与外部既有的
  `ai_daw_engine::model::…` 全部照旧可用 —— diff 被限制在「搬家 + 三行薄壳」。
- 契约测试随类型一起搬到 `crates/ipc`（wire-shape 测试属于契约，本就该住在契约 crate），
  并新增 `Event` 的**外部标记**形状测试（`stores/engineEvents.ts` 依赖该形状）。

## 4 验收标准

1. 类型/字段/serde 属性与搬移前**逐字一致**（除文档链接措辞）。
2. `cargo test --workspace` 通过数守恒：engine 库 106 → 90，新增 `ai-daw-ipc` 17（16 搬移 + 1 新增）；
   `--features realtime` 123 → 107。
3. `cargo check --manifest-path src-tauri/Cargo.toml` 通过（src-tauri 未改任何代码）。
4. `ai_daw_engine::model::clip` 这类**子模块路径**仍然可用（薄壳要能带出子模块）。
5. 新增的 `event_wire_shape_is_externally_tagged` 通过。

## 5 验证方式

```
cargo test --workspace
cargo test -p ai-daw-engine --features realtime
cargo check --manifest-path src-tauri/Cargo.toml
```

## 6 风险

| 风险 | 缓解 |
| --- | --- |
| 薄壳用 glob 是否真能带出**子模块**（`model::clip`） | 由「src-tauri 与 engine 全量编译通过」直接验证；若不行则改为逐个子模块薄壳 |
| 搬家漏改内部路径 | 依赖编译器穷举；全量测试即回归 |
| 契约测试搬家后重复/丢失 | 通过数守恒逐项核对（见验收 2） |
