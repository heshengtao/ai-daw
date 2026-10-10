# 插件参数自动化（含自适应参数车道）— 设计

> **状态（2026-10-10）**：✅ 完成（2026-10-10）。本设计把自动化从「轨道 `gain_db` / `pan`」扩展到
> **任意插件参数**（乐器与效果器），并把钢琴卷帘下方的控制器泳道改造成**按插件自适应的参数车道**。
> 实现计划见 `../plans/2026-10-10-plugin-param-automation.md`。
>
> 前置：`specs/2026-10-10-mixing-sends-buses-automation-design.md`（C3：发送 / 总线 / `gain_db`·`pan` 自动化）。

## 1 背景

C3 已经落地「发送 → 总线」路由与**轨道参数**的自动化：`Track.automation` 里存若干条 `AutomationLane`，
调度表把能识别的参数解析进播放器，`Player::refresh_mix` **逐块**求值并**覆盖**推子值。但能自动化的
参数是一个**封闭的两值枚举**（`LaneParam::GainDb | Pan`）——插件自己的参数（滤波截止、混响混合……）
一律进不来。

用户要求：

1. 自动化要能到**插件参数**（乐器 + 效果器）；
2. 钢琴卷帘下方的**控制器泳道不要写死**，按选中轨道的插件自适应——内置合成器 = 力度 + 一组常用参数；
   并**去掉与力度重复的「音量」**；
3. 卷帘里选的插件参数与编排视图的自动化是**同一份数据**（`Track.automation`），只是位置不同。

## 2 目标与非目标

**目标**

- 自动化目标扩展到三类：轨道混音参数（`gain_db` / `pan`）、**轨道乐器**的任意参数、**效果链上任意
  效果器**的任意参数。
- 目标沿用 `Track.automation[].param` 这个**字符串键**，带命名空间；**工程 JSON / IPC / MCP 的 wire 形状
  一字不改**，旧工程零迁移。
- 逐块求值（块率）、有曲线就**覆盖**该参数的静态值（读取模式）、可撤销、AI 可经 MCP 驱动。
- 钢琴卷帘下方泳道：`力度`（逐音符，维持现状）+ 上述全部目标（曲线）；**去掉「音量」选项**。
- 内置合成器只列**精选常用参数**；外部插件列全部 `automatable` 参数。

**非目标（v1）**

- 采样级 / 音频速率自动化（块率足够，且与现状一致）。
- 曲线形状（指数 / S 形）、LFO / 调制矩阵、自动化写模式（touch / latch）、多参数批量编辑。
- 效果器按**插件身份**绑定（v1 用槽位下标，见 §3）。
- `send` 的增益自动化、跨轨调制。
- **逐音符的插件参数**：插件参数是轨道级的，不会给 `Note` 加字段（见 §3 的泳道语义决策）。

## 3 关键决策

| 决策 | 结论 | 理由 |
| --- | --- | --- |
| 目标怎么表示 | 沿用 `param: String` 键 + 命名空间（`gain_db` / `pan` / `inst:<id>` / `fx<i>:<id>`），引擎内解析成 `LaneTarget` | 工程 JSON / IPC / MCP 形状一字不改，旧工程零迁移；把它升成结构化字段要改 schema、迁移旧车道、前端/命令/MCP 全跟着动 |
| 键在哪解析 | **控制侧**（`src-tauri/src/audio.rs` 从 `Event::AutomationChanged` 解析），`TransportCommand::SetAutomation` 携带**已解析好的** `LaneTarget` | 音频线程解析键就要给 `ParamId` 分配 `String`，会破 `tests/realtime_safety.rs` 的零分配门禁 |
| 求值时机 | 逐块（块率），与 `gain_db` / `pan` 完全一致 | 与现状一致；滤波截止这类参数块率足够 |
| 覆盖语义 | 有曲线即覆盖该参数的静态值（读取模式） | 与现有 `gain_db` / `pan` 一致 |
| 结构校验 | 调度构建时校验：`inst:` 要求该轨有乐器；`fx<i>:` 要求下标在链内；不满足即丢弃 | 与既有的「丢弃未知参数、丢弃悬空发送」同一套做法 |
| 效果器寻址 | 槽位下标 `fx<i>:<param>` | 与既有 `set_effect_param(track, index, …)` 一致；代价是重排效果链后曲线跟槽位走（§9） |
| 内置合成器的可选项 | **精选常用子集**（一张可编辑的表） | 26 个参数全塞进下拉不可用；用户明确要「常用的那些」 |
| 卷帘泳道语义 | `力度` = 逐音符直角条；其余目标 = **曲线**（与编排**同一份数据**） | 用户确认；避免给 `Note` 加插件参数字段、避免扩插件 ABI |
| 控制器泳道的选项来源 | 与编排的自动化车道**共用同一个目标列表** | 两处一致，不会一边能选、一边不能 |

## 4 键与模型

自动化键（`Track.automation[].param`）：

| 键 | 目标 | 值域 | 说明 |
| --- | --- | --- | --- |
| `gain_db` | 轨道推子增益 | −60 … +12 dB | 原样不变 |
| `pan` | 轨道声像 | −1 … 1 | 原样不变 |
| `inst:<param_id>` | 该轨**乐器**的参数 | 由插件 `ParamInfo` 给出 | 如 `inst:cutoff` |
| `fx<i>:<param_id>` | 效果链**第 i 个**效果器的参数 | 由插件 `ParamInfo` 给出 | 如 `fx0:mix`；`i` 从 0 起 |

- 空键、空 `param_id`、以及既不匹配 `inst:` / `fx<i>:` 前缀也不是 `gain_db` / `pan` 的键 → 解析失败，
  该车道在调度构建时被丢弃（工程里仍保留，与现状「未知参数保留但求值时忽略」一致）。
- `param_id` 里**不支持** `:`（内置参数 id 是普通标识符，VST3 / CLAP 的参数 id 是十进制序号，均不含 `:`）。

引擎侧新增（`crates/engine/src/schedule.rs`）：

```rust
/// 一条自动化车道的**已解析目标**（参数 id 在这里就分配好，随车道带进播放器）。
#[derive(Debug, Clone, PartialEq)]
pub enum LaneTarget {
    /// 轨道推子增益（dB）。
    GainDb,
    /// 轨道声像（-1..=1）。
    Pan,
    /// 轨道的乐器参数。
    Instrument(ParamId),
    /// 效果链第 `index` 个效果器的参数。
    Effect { index: usize, param: ParamId },
}

impl LaneTarget {
    /// 从键解析。**会分配**（`ParamId` 拥有自己的 `String`）——只在控制侧 / 调度构建时调用，
    /// 绝不在音频线程调用。不认识的形式返回 `None`。
    pub fn from_key(key: &str) -> Option<LaneTarget>;
}
```

`Track` / `AutomationLane` / `AutomationPoint` 模型、`Command::SetAutomation`、`Event::AutomationChanged`
（`param: String`）**全部不变**。`Event::is_mix_only()` 里 `AutomationChanged` 仍为真 → 改插件参数曲线
**不重建播放器**。

## 5 引擎改动

- `schedule.rs`：`LaneParam` 由 `LaneTarget` 取代；`ScheduledLane { param: LaneTarget, points }`。
  `from_project_with_assets` 解析 + **结构性校验**（`Instrument` 需 `track.instrument.is_some()`、
  `Effect { index }` 需 `index < track.effects.len()`），不满足即 `filter_map` 丢弃。
- `player.rs`：
  - `refresh_mix` 逐块求值后 `match &lane.param`：`GainDb` / `Pan` 走原路；`Instrument(pid)` →
    `track.plugin.as_mut()?.set_param(pid, value)`；`Effect { index, param }` →
    `track.effects.get_mut(index)?.set_param(param, value)`。仍是**块率一个值**。
  - `TransportCommand::SetAutomation` 的 `param: String` 换成 `target: LaneTarget`；
    `retain` / `push` 逻辑不变（`lanes.reserve(4)` 的预留继续覆盖 `push`）。
  - `LaneTarget` 非 `Copy`：匹配改用 `match &lane.param`，`retain(|lane| lane.param != target)` 依旧可用。
- `transport.rs`：`TransportCommand::SetAutomation { track, target: LaneTarget, points }`。
- `lib.rs`：导出 `LaneTarget`（宿主与测试要用）。
- 离线渲染：`ScheduledEvents` 构造路径相同 → **导出的 WAV 与实时一致**地带上插件参数自动化。

## 6 宿主与 MCP

- `src-tauri/src/audio.rs`：`Event::AutomationChanged { param, .. }` → `LaneTarget::from_key(param)`，
  拿到才 `queue_if_available(TransportCommand::SetAutomation { target, .. })`；解析失败就跳过（等价于现在的忽略）。
- `commands.rs` / `mcp.rs`：**签名不变**（`param` 仍是 `String`），`set_automation` 的入参与
  `skills/mix.md` 的说明补一句「`param` 可以是 `inst:<id>` / `fx<i>:<id>`」。
- MCP 工具表 **33 个不变**（`every_tool_is_registered` 的断言无需改）。
- **不需要**新命令：前端能从 `plugin_params(plugin_id)` + 轨道快照自己拼出可用目标。

## 7 前端

**共用目标列表**（新 `src/utils/automation.ts`）

- `AutomationTarget { key, label, group, min, max, step, digits, unit? }`。
- `automationTargetsFor(track, paramsOf, t)`：先给 `gain_db` / `pan`（组「轨道」），再给乐器的可自动化参数
  （组「乐器」），再按效果链顺序给每个效果器的可自动化参数（组「效果器 1」「效果器 2」…）。
  **键里的 `i` 从 0 起（`fx0:` = 链上第 1 个），分组标题从 1 起显示**——只在这一处做换算。
- 只收 `automatable !== false` 的参数。**内置合成器用精选子集**：`data/plugins.ts` 里一张
  `COMMON_INSTRUMENT_PARAMS` 表，v1 给 `builtin.synth` 定这 8 个（就是合成器面板「基础」那一组）：
  `cutoff / resonance / attack / decay / sustain / release / detune / lfoRate`。表是**唯一**要改的地方，
  以后加参数只动它；没有精选条目的插件（外部插件）列全部可自动化参数。
- 参数显示名：内置合成器走 i18n（把 `SynthPanel` 里那张 id → `synth.*` 键的表提到 `data/plugins.ts` 共用，
  英文界面才不会是中文名）；其余用 `ParamSpec.name`。
- `laneValueAt(points, beat)` 也放这里（镜下镜像引擎，供读数用）。

**曲线单元**（新 `src/components/workspace/AutomationCurve.vue`）

- 从现有 `AutomationLane.vue` 整块搬出：刻度列、网格、折线（两端补抽头）、关键点、加点 / 拖点 / 右键删、
  播放头。
- 接口：props `{ points, min, max, step, digits, unit?, height, insetPx }`，emit `apply(points)`。
  「清空」「参数下拉」「读数」留在各自的车道外壳里。
- `insetPx` 是**画布左边缘相对时间轴原点（第 0 拍）的偏移**，两个宿主不一样，必须传：
  编排视图的网格从 `TIMELINE_INSET_PX`(26) 起算，传 **26**；卷帘下方的车道左侧已有 60px 的刻度列
  （与琴键列对齐、且已显示该目标的 min/max），传 **0**。这样两处的网格线各自与所在视图的网格严格对齐。
  `insetPx` 为 0 时组件不画自带的刻度列。

**两个车道外壳**

- `AutomationLane.vue`（编排）：下拉 = 全部目标（分组）→ `AutomationCurve`（`setAutomation`）。
- `ControllerLane.vue`（卷帘下方，改成**参数车道**）：
  - 下拉 = `力度` + 全部目标（分组）；
  - `力度` → 现有逐音符直角条（继续走 `set_notes`）；其余 → `AutomationCurve`（走 `setAutomation`）；
  - **去掉「音量」**（与力度重复）。`Note.volume` 字段保留，旧工程听感不变。
  - 组件改为接收 `targets` / `curvePoints` 并 emit `applyCurve`，由 `EditView`（已持有 store）落实提交。

**顶栏底部窗格开关**（本轮已实现，这里补记）

- 按钮**常驻**：编排 / 编辑能用；其他视图（混音 / 设置）**显示为禁用**（将来会用得上）。
- 图标统一用 `PanelIcon side="bottom"`（与侧边栏开关同一套视觉），去掉临时的自绘图标。

**i18n**：补 `auto.group.*`、`topbar.bottomLaneUnavailable` 等；**删掉** `lane.volume` / `lane.volumeHint`。
中英键数保持一致。

## 8 验收标准

1. **引擎单测**（`cargo test -p ai-daw-engine --features realtime`）：
   - `LaneTarget::from_key` 四种形式；空键 / 空 id / 未知前缀 → `None`；
   - 调度构建丢弃「无乐器」的 `inst:` 与「越界」的 `fx<i>:`；
   - `refresh_mix` 对**乐器参数**与**效果器参数**逐块生效，并**覆盖**该参数的静态值；
   - 旧工程（只有 `gain_db` / `pan`）行为逐位不变。
2. **实时安全**：`tests/realtime_safety.rs` 的零分配门禁把**插件参数车道**也覆盖进去
   （`SetAutomation` 携带已解析 `LaneTarget` 时 `apply_command` 仍 0 分配）。
3. **宿主**：`cargo check` 通过；MCP 工具表仍是 33 个。
4. **前端**：`npm run build` 通过；中英键数一致；浏览器预览里
   「卷帘下方选 `inst:cutoff` 画曲线 → 切到编排看到**同一条**」「效果器参数同理」「力度仍是逐音符条」
   「下拉里没有『音量』」「顶栏开关在混音 / 设置页禁用」。

## 9 风险

| 风险 | 缓解 |
| --- | --- |
| 音频线程解析键会分配，破坏零分配门禁 | 解析固定在控制侧；`TransportCommand::SetAutomation` 携带已解析的 `LaneTarget` |
| `LaneTarget` 由 `Copy` 变非 `Copy`，波及匹配/`retain` 写法 | 编译期暴露；改 `match &lane.param` 即可 |
| 重排效果链后 `fx<i>` 曲线跟槽位走 | 明说为 v1 取舍；参数不存在时插件忽略（不炸） |
| 效果器被删 / 乐器被卸载 → 车道变「孤儿」 | 结构校验丢弃（不生效）；工程里保留，装回即恢复 |
| 外部插件 `set_param` 自身可能分配 | 门禁只覆盖内置插件；外部插件本来就在音频线程里跑 `process`，量级相同 |
| 外部插件参数极多，下拉很长 | v1 只做分组 + 滚动；精选表可按插件补充 |
| 参数 id 含 `:` 无法表达 | 内置与 VST3 / CLAP 的 id 均不含 `:`；文档写明不支持 |
