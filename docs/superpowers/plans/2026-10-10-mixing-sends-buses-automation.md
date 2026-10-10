# 混音完善（C3）Implementation Plan

> **状态（2026-10-10）**：✅ Task 1–6 全部完成（引擎 + 宿主 API + 数据层 + 编辑器 UI：
> 检查器发送区、编排视图的自动化曲线编辑器）。
> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans.

**Goal：** 给引擎加「发送 → 总线」路由与参数自动化（`gain_db` / `pan`），全链路可撤销、可被 AI 驱动。

**Architecture：** `Track.sends` + `Track.automation` 进模型；调度表把发送解析成轨道下标并算出拓扑处理顺序；
`Player` 按顺序逐轨处理、把推子后信号复制进目标的总线缓冲；自动化按拍逐块求值覆盖推子。

**Tech Stack：** Rust 2021、serde、`crates/ipc` 契约、Tauri、Vue 3。

设计见 `../specs/2026-10-10-mixing-sends-buses-automation-design.md`。

## Global Constraints

- 许可 Apache-2.0-only；**不新增依赖**。
- 引擎 `#![forbid(unsafe_code)]`。
- **实时回调零分配**：`tests/realtime_safety.rs` 是硬门禁（本轮已扩展到发送 + 自动化）。
- 新增模型字段一律 `#[serde(default)]` 且空值 `skip_serializing_if`，**旧工程输出形状不变**。
- 每个 Task 末尾全绿；涉及 src-tauri 跑 `cargo check`；前端跑 `npm run build` 且 i18n 中英键数一致。

## 文件结构

```
crates/ipc/src/model/track.rs     Send / AutomationPoint / AutomationLane / lane_value_at / Track 字段
crates/ipc/src/{command,event}.rs SetSends / SetAutomation / TrackSendsChanged / AutomationChanged
crates/engine/src/{apply,bus}.rs  应用与 translate（归一化、自发送丢弃、点排序过滤）
crates/engine/src/schedule.rs     LaneParam / ScheduledLane / ScheduledSend / routing_order / order / tempo_map
crates/engine/src/player.rs       bus_in / order / 车道求值 / 路由 loop / SetAutomation
crates/engine/src/transport.rs    TransportCommand::SetAutomation
src-tauri/src/{audio,commands,lib,mcp}.rs  仅混音同步 + set_sends/set_automation 命令与 MCP 工具
src/api/* src/stores/project.ts   前端数据层
```

## Task 1: 模型（`crates/ipc`）

- [x] `Send` / `AutomationPoint` / `AutomationLane` + `lane_value_at`（线性插值、两端夹住、不分配）。
- [x] `Track.sends` / `Track.automation`（`#[serde(default, skip_serializing_if = "Vec::is_empty")]`）。
- [x] 测试：插值/夹住；空字段不写入 JSON；带值往返。

## Task 2: 命令 / 事件 / apply

- [x] `Command::{SetSends, SetAutomation}`；`Event::{TrackSendsChanged, AutomationChanged}` + `actor()` + `is_mix_only()`。
- [x] `translate`：自发送丢弃、增益归一化、点过滤/排序/封顶、空参数名不产事件。
- [x] `apply`：整体替换发送/车道；空曲线删车道。
- [x] 测试：`bus.rs` 的 `set_sends_*` / `set_automation_*`（含撤销与边界）。

## Task 3: 调度（`schedule.rs`）

- [x] `LaneParam` / `ScheduledLane` / `ScheduledSend`；`ScheduledEvents.order` + `tempo_map`。
- [x] `routing_order(edges)`（Kahn，零入度按升序；环残余按原顺序追加）。
- [x] `from_project_with_assets` 解析发送（悬空/自发送丢弃）与车道（未知参数丢弃）。
- [x] 测试：`routing_order_*`、`sends_and_lanes_are_resolved_into_the_schedule`。

## Task 4: 播放器（`player.rs`）核心

- [x] `bus_in` / `order` / `tempo_map` / `has_automation`；`TrackVoice.lanes/.sends`。
- [x] `process_block`：并入总线输入 → 乐器/音频 → 效果链 → 推子 → master + 发送复制。
- [x] `refresh_mix`：按拍求值车道覆盖 gain/pan；不可听轨道清空自己的总线输入。
- [x] `TransportCommand::SetAutomation`：就地过滤/排序/替换（零分配，预留容量）。
- [x] 测试：`a_send_copies_the_post_fader_signal_into_the_bus`、`gain_automation_overrides_the_track_fader`、
      `set_automation_applies_live_without_a_rebuild`。

## Task 5: 宿主 + 前端数据层

- [x] `audio.rs`：`AutomationChanged` → `SetAutomation`（仅混音）。
- [x] `commands.rs`：`set_sends` / `set_automation`；`lib.rs` 注册。
- [x] `mcp.rs`：`set_sends` / `set_automation` 工具 + 工具表断言（30 → 32）。
- [x] 前端：类型、`api.setSends/setAutomation`、mock、`project.setSends/setAutomation`。

## Task 6: 编辑器 UI

- [x] 检查器的**发送区**（每轨：列出现有发送 = 目标名 + 增益滑块 + 移除；下拉新增发送）。
- [x] 编排视图的自动化曲线编辑器（`gain_db` / `pan`），拖动编辑点 → `setAutomation`：
      `components/workspace/AutomationLane.vue`（底部车道，与网格共用同一条时间轴）+ 顶栏开关
      （`AutomationIcon`）+ `layout.showAutomation` / `automationHeight`（持久化、可拖拽改高）。
- [x] i18n 键（中英 `inspector.sends*` / `error.setSends` / `auto.*` / `error.setAutomation`）。

## 执行记录（2026-10-10）

- `cargo test -p ai-daw-engine --features realtime` → **141 通过**（含本轮新增：模型 2、调度 3、播放器 3、命令 2）。
- `tests/realtime_safety.rs`：门禁已覆盖**发送 + 自动化**（含 `SetAutomation` 的 `apply_command` 零分配断言）。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib` → 11 通过（MCP 工具表含 `set_sends`/`set_automation`）。
- `npm run build` 通过。

**执行记录（Task 6，编辑器 UI）**：新增 `components/workspace/AutomationLane.vue`（编排视图底部车道：
在空白处点击/拖动加点、拖动改拍与值、右键删点、清空整条曲线、表头读数显示播放头处的曲线值），
`components/common/icons/AutomationIcon.vue`，`ArrangeView.vue` 包一层纵向容器并把车道钉在底部，
`TopBar.vue` 加开关（只在编排视图出现）。`npm run build` 通过；i18n 中英各 654 键、完全一致；
浏览器预览（mock 后端）逐项验证：开关/加点/拖动/右键删除/清空/参数切换/撤销、曲线两端抽头与引擎
`lane_value_at` 的夹住语义一致。**修掉一个真实缺陷**：拖动草稿若直接持有工程数据的 Vue 响应式代理，
拖动会就地改写工程并绕过撤销，且 `structuredClone`（mock）会抛 `DataCloneError`——进出车道的点
一律先摊平成普通对象。

**后续修正（同日）**：① 顶栏开关不再随视图消失——编排与钢琴卷帘两个视图都有该按钮，
各自开关本视图的底部窗格（`showAutomation` / 新增的 `showControllerLane`），**两个窗格默认都打开**；
② `PanelResizer` 的 `top`/`bottom` 增量符号写反（抓住上边缘向上拖反而变矮），已修。

**已知遗留**：自动化点上限 4096；仅线性曲线；无 pre-fader send；画布未做纵向缩放（值域固定 -60..12 dB / -1..1）；
**自动化车道目前只认 `gain_db` / `pan`，插件参数还不能自动化**（见 ROADMAP C3 与设计 §7）。
