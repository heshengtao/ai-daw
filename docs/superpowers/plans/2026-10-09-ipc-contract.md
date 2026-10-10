# `crates/ipc` 契约下沉 Implementation Plan

> **状态（2026-10-09）**：✅ 全部完成（Task 1–4）。设计见
> `docs/superpowers/specs/2026-10-09-ipc-contract-design.md`。
>
> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans。

**Goal：** 把 `Command` / `Event` / `Project` 的 serde schema 抽成独立 crate `ai-daw-ipc`，
成为「引擎 ↔ Tauri ↔ 未来 MCP」的唯一契约，且**零行为变化**。

**Architecture：** 新 crate 只放纯数据与纯函数；engine 保留同名薄壳模块（`pub use`），
使内部与既有外部路径全部不变。

**Tech Stack：** Rust edition 2021、serde 1。

## Global Constraints

- 许可 Apache-2.0-only；允许清单见 ROADMAP §3（本轮加入 MPL-2.0，仅限未修改依赖）。
- `#![forbid(unsafe_code)]`（新 crate 同样）。
- 新增模型字段一律 `#[serde(default)]`（本轮**没有**新增字段）。
- 每个 Task 末尾 `cargo test --workspace` 全绿。
- 提交结尾 `Co-authored-by: CommandCodeBot <noreply@commandcode.ai>`。

## 文件结构

```
crates/ipc/
  Cargo.toml
  src/lib.rs
  src/command.rs
  src/event.rs
  src/model/{mod,clip,track,transport,project}.rs
crates/engine/src/
  command.rs      → 薄壳（pub use）
  event.rs        → 薄壳（pub use）
  model/mod.rs    → 薄壳（pub use；model/{clip,track,transport,project}.rs 已删除）
deny.toml         → 新增
NOTICE            → 增补 symphonia（MPL-2.0）
```

## Task 1: 新建 `crates/ipc` 并搬运 schema

**Files:** Create `crates/ipc/{Cargo.toml,src/lib.rs,src/command.rs,src/event.rs,src/model/*.rs}`；
Modify 根 `Cargo.toml`（members）。

- [ ] **Step 1**：`crates/ipc/Cargo.toml` —— `serde`（dep）、`serde_json`（dev-dep）。
- [ ] **Step 2**：逐字搬 `model/{clip,track,transport,project}.rs`、`command.rs`、`event.rs`。
- [ ] **Step 3**：`command.rs` 里指向引擎的 intra-doc 链接（`crate::STARTER_TRACKS`）改成普通文字。

## Task 2: engine 侧薄壳

**Files:** Modify `crates/engine/Cargo.toml`、`crates/engine/src/{command.rs,event.rs,model/mod.rs}`；
Delete `crates/engine/src/model/{clip,project,track,transport}.rs`。

- [ ] **Step 1**：engine 依赖 `ai-daw-ipc`。
- [ ] **Step 2**：三个模块改为 `pub use ai_daw_ipc::…::*;`。
- [ ] **Step 3**：删除旧文件，`cargo test --workspace`。

## Task 3: 契约测试

- [ ] **Step 1**：wire-shape 测试随 `project.rs` 搬入 `crates/ipc`。
- [ ] **Step 2**：新增 `event_wire_shape_is_externally_tagged`（外部标记 + 裸数字 id + `Agent` 携带名字）。

## Task 4: 许可基座

**Files:** Create `deny.toml`；Modify `NOTICE`、`docs/superpowers/ROADMAP.md` §3。

- [ ] **Step 1**：ROADMAP §3 允许清单加入 MPL-2.0（含条件与首个用例）。
- [ ] **Step 2**：`deny.toml` 落 allow-list。
- [ ] **Step 3**：`NOTICE` 增补 symphonia 段落（声明未修改）。

## 自检 / Self-Review

- **Spec coverage**：对照 `2026-10-09-ipc-contract-design.md` §4 的 5 条验收 —— 1（逐字一致，人工比对）、
  2（通过数守恒，实测）、3（src-tauri 未改却编译通过，实测）、4（子模块路径，由编译证明）、
  5（新测试通过）。全部覆盖。
- **Placeholder 扫描**：无 TODO / 未实现分支。
- **类型一致性**：`crates/ipc` 与 `crates/engine` 无重复定义，薄壳是唯一来源。
- **偏差**：计划里写「新增 `Command` 的 serde 形状测试」——实际 `Command` **不派生** `Serialize`，
  它不作线上格式（Tauri 逐个传参、MCP 逐个映射工具），故只给 `Event` 加了形状测试。这是有意保留的现状。

## 执行记录（随进度更新）

### 全部完成（4/4）

| 任务 | 内容 |
| --- | --- |
| 1 | `crates/ipc` 建立；`model/{clip,track,transport,project}.rs` + `command.rs` + `event.rs` 逐字搬入 |
| 2 | engine 三个模块改为 `pub use` 薄壳；删除旧的 4 个模型文件；加 `ai-daw-ipc` 依赖 |
| 3 | wire-shape 测试随类型搬入；新增 `event_wire_shape_is_externally_tagged` |
| 4 | ROADMAP §3 加 MPL-2.0；新增 `deny.toml`；`NOTICE` 增补 symphonia |

**验证**：
- `cargo test --workspace` → 全绿。engine 库 106 → **90**，新增 `ai-daw-ipc` **17**（16 搬移 + 1 新增）。
- `cargo test -p ai-daw-engine --features realtime` → **107**（原 123 − 16 搬移）。
  两组数字与「恰好搬走 16 个用例」完全吻合。
- `cargo check --manifest-path src-tauri/Cargo.toml` → 通过；`src-tauri` **一行未改**，
  证明 `pub use` 薄壳对既有外部路径（`ai_daw_engine::model::…` 等，含子模块）完全兼容。

**说明**：
- 薄壳用 glob `pub use ai_daw_ipc::model::*;` 能连**子模块**一起带出（`engine::model::clip` 可用），
  这一点由 src-tauri 与 engine 的全量编译通过直接证明，无需逐个子模块写薄壳。
- 本轮**零行为变化**：没有新增/删除字段，没有改归一化，没有改 serde 属性。
- 未做：CI 接入 `cargo-deny`（本机也未安装该工具，`deny.toml` 只保证配置就位）。
