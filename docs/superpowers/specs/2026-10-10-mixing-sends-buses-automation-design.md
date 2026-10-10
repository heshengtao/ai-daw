# 混音完善（C3）：发送 / 总线 / 自动化 — 设计

> **状态（2026-10-10）**：引擎 + 宿主 API + 前端数据层已完成并有测试；**编辑器 UI 已完成**
> （检查器发送区 + 编排视图的自动化曲线编辑器，见 §7）。
> 计划见 `../plans/2026-10-10-mixing-sends-buses-automation.md`。

## 1 背景

Roadmap Stage C 的 C3：`AutomationLane`、send / bus（设计文档 §6 已列，模型尚未实现）。C1/C2 之后，
音频链路已有「轨道 → 效果链 → 轨道增益/声像 → master」；本轮补上**发送（aux send）→ 总线（bus）**
的路由与**参数自动化**。

## 2 目标与非目标

**目标**

- 轨道可把**推子后**信号按增益发送到另一条轨道（总线）；总线照常送往 master。
- 轨道参数（v1：`gain_db` / `pan`）可挂**自动化折线**，按拍线性插值、逐块生效。
- 两类变更都走命令/事件总线、可撤销；自动化是**仅混音**变更（不重建播放器）。

**非目标（v1）**

- pre-fader send、send 的自动化、多级延迟补偿。
- 自动化曲线类型（仅线性；无指数/S 形）、曲线编辑器里的纵向值域缩放。
- 侧链、多总线并行分组、反馈环（检测到环时忽略环上的发送）。

## 3 关键决策

| 决策 | 结论 | 理由 |
| --- | --- | --- |
| 发送放哪 | `Track.sends: Vec<Send>`（**不是** `MixerChannel`） | `MixerChannel` 是 `Copy`，塞 `Vec` 会波及 master/schedule/宿主一大片；`Track` 本就非 `Copy` |
| 路由顺序 | 由发送图**拓扑排序**（Kahn），目标排在来源之后 | 总线才能在本块看到已汇总的输入；有环时残余节点按原顺序追加（环上的发送被忽略，绝不死循环） |
| 总线语义 | 总线输入**并入本轨信号**（乐器/音频片段之后、效果链之前） | 总线能用自己的效果链处理汇总信号；总线本身也照常去 master |
| 自动化覆盖 | 有曲线的参数**覆盖**推子值（读取模式） | 标准 DAW 行为；文档写明 |
| 自动化实时性 | 事件 `is_mix_only`，经 `TransportCommand::SetAutomation` 推给音频线程**逐块求值** | 拖曲线不重建播放器、不打断音频 |
| 发送实时性 | `TrackSendsChanged` 是**结构性**变更（重建） | 改路由会改变处理顺序，重建最稳妥；发送不常改 |
| 空字段 | `sends` / `automation` 空时**不写进 JSON**（`skip_serializing_if`） | 老工程 JSON 形状逐字不变 |

## 4 模型与契约

- `Send { target: TrackId, gain_db: f32 }`、`AutomationPoint { beat: f64, value: f32 }`、
  `AutomationLane { param: String, points: Vec<AutomationPoint> }`（`crates/ipc/src/model/track.rs`）。
- `AutomationLane::value_at(beat)` / `lane_value_at(points, beat)`：线性插值、两端夹住，**不分配**。
- `Command::{SetSends, SetAutomation}`（整体替换）；`Event::{TrackSendsChanged, AutomationChanged}`。
- `Event::is_mix_only()`：`AutomationChanged` 为真；`TrackSendsChanged` 为假（结构性）。
- `translate` 归一化：send 增益夹紧、**丢弃自发送**；自动化点滤非有限值、夹非负拍、按拍排序、封顶 4096 点。

## 5 引擎改动

- `schedule.rs`：
  - `LaneParam::{GainDb, Pan}`；`ScheduledLane`（保拍坐标，求值时用速度表）；`ScheduledSend { target_index, gain }`
    （目标解析成**轨道下标**，播放时 O(1)）。
  - `ScheduledEvents.order`（拓扑顺序）与 `ScheduledEvents.tempo_map`；`routing_order(edges)` 纯函数 + 测试。
  - `from_project_with_assets` 解析发送/车道（悬空目标、未知参数、自发送一律丢弃）。
- `player.rs`：
  - `TrackVoice.lanes/.sends`；`Player.bus_in`（每轨一条交错立体声累加缓冲，长度 `2*max_block`）、`order`、`tempo_map`、`has_automation`。
  - `process_block` 按 `order` 逐轨：并入 `bus_in[idx]` → 乐器/音频 → 效果链 → 推子 → master **且**按发送复制进目标 `bus_in`。
  - `refresh_mix` 逐块按 `beats = tempo_map.seconds_to_beats(position/rate)` 求值车道，覆盖 gain/pan。
  - `SetAutomation` 命令：就地 `retain`/`sort_unstable_by` 后整体替换（**零分配**；`lanes` 预留容量）。
  - 不可听轨道 → 清掉自己的 `bus_in`（静音的总线不透传上游）。
- `transport.rs`：`TransportCommand::SetAutomation`。

## 6 宿主与前端

- `audio.rs`：`Event::AutomationChanged` → `TransportCommand::SetAutomation`（仅混音通道）；发送走重建。
- 命令：`set_sends` / `set_automation`；MCP 工具同名（工具表 30 → 32）。
- 前端数据层：`Send`/`AutomationPoint`/`AutomationLane` 类型、`api.setSends/setAutomation`、mock、store 封装。

## 7 编辑器 UI（已完成）

- **发送 UI**：检查器（`InspectorPanel.vue`）新增「发送」一组——列出本轨发送（目标名 + 增益滑块 +
  移除），并从下拉里挑一条轨道新增发送。
- **自动化曲线编辑器**：编排视图底部一条车道（`components/workspace/AutomationLane.vue`，对标
  `ControllerLane.vue`）——参数下拉（`gain_db` / `pan`）、点/拖加点、拖动改拍与值、右键删点、
  清空整条曲线；表头读数显示点数与**播放头处的曲线值**。横向与编排网格共用同一条时间轴
  （同一缩放、同一滚动位置、同一套由拍号推导的网格，画布左边缘与网格原点一样留出
  `TIMELINE_INSET_PX`），所以曲线的小节线严格压在网格上。渲染对折线**两端各补一个抽头**，
  与引擎 `lane_value_at` 的「首点前 / 末点后按最近点夹住」语义一致。
  面板开关在顶栏——**编排与钢琴卷帘两个视图都有**（编排控自动化车道，卷帘控下方控制器泳道，
  按钮不再随视图消失）；两个底部窗格**默认都打开**，可见性各自持久化
  （`layout.showAutomation` / `showControllerLane`），高度可拖拽调整
  （`automationHeight` / `laneHeight`）。拖拽方向与鼠标直觉一致：抓住上边缘**往上拖是变高**
  （`PanelResizer` 的 `top`/`bottom` 增量符号修过一次）。
  - 交互全部走 `setAutomation` 命令 → 可撤销，AI 走 MCP 时与人类编辑同一条管线。
  - 进出车道的每个点都先摊平成**普通对象**：工程数据是 Vue 响应式代理，直接放进拖动草稿会
    就地改写工程（绕过撤销），且会被浏览器预览 mock 的 `structuredClone` 拒绝。

## 8 验收标准

1. **引擎单测**（`cargo test -p ai-daw-engine --features realtime`）：
   - `routing_order` 把目标排在来源之后；有环不挂死；
   - 发送把推子后信号复制进总线（master = 直连 + 总线，峰值 ≈ 2×）；
   - 增益自动化覆盖推子；`SetAutomation` 实时生效（推入/移除）；
   - 命令层：自发送丢弃、增益归一化、点排序/过滤、空曲线删车道、空参数名不产事件；
   - 模型：`lane_value_at` 插值与两端夹住；空 `sends`/`automation` 不写进 JSON。
2. **实时安全**：`tests/realtime_safety.rs` 已把**发送 + 自动化**纳入零分配门禁，并单独断言
   `SetAutomation` 的 `apply_command` 零分配。
3. 宿主 `cargo check` + 前端 `npm run build` 通过；MCP 工具表含 `set_sends` / `set_automation`。

## 9 风险

| 风险 | 缓解 |
| --- | --- |
| 总线路由破坏既有播放 | 全量引擎测试保持全绿；`order` 缺省回退顺序处理 |
| 自动化求值引入分配 | 求值走 `lane_value_at`（纯算术）；`SetAutomation` 就地排序/替换并预留容量；门禁覆盖 |
| 反馈环 | 拓扑排序忽略环上发送（不死循环）；自发送在命令层丢弃 |
| 老工程 JSON 变形 | 空 `sends`/`automation` 不序列化；wire-shape 测试断言 |
