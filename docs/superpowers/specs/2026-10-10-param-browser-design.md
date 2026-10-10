# 参数浏览器（自动化目标绑定）设计

> 状态：设计完成（2026-10-10），待实现。

## 1 背景与问题

自动化车道的目标下拉现在等于「该轨道全部可自动化参数」：`gain_db` / `pan` + 乐器的全部 + **每个效果器的全部**。
后果是列表随插件数量线性膨胀，而且**外部插件**（VST3 / CLAP）动辄几百个参数——列表长到无法使用。
效果链越长越糟：链上每个效果器都要重复摊开自己的一整段参数。

这不是我们独有的问题，所有 DAW 的解法都是**避免摊开全量**：

| 做法 | 例子 |
| --- | --- |
| 入口是**控件右键**（"显示自动化"），列表是次要入口 | Ableton、Bitwig、Logic、Studio One、Reaper |
| 列表只装**已经加进来的参数** | Reaper 的包络列表、Ableton 的 clip 包络选择器（只列已有包络的） |
| 需要"添加"时给一个**可搜索的挑选器** | Cubase 的参数列表、Studio One 的 Add Automation |
| **分页 / 参数 bank**（源自硬件） | Live + Push 的蓝手、Bitwig 的 Remote Controls |
| 用**元数据**先筛（厂商声明的可自动化标记） | VST3 `kCanAutomate`、CLAP 只读标记 |
| 让**用户自己收窄**（宏 / 收藏） | Live 的 Macro、Reaper 的轨道别名 |

本设计取其中三条：**列表只装已绑定/已用的**、**一个可搜索/可排序/可筛选/可翻页的挑选器**、**用 `automatable` 元数据兜底**。

## 2 目标 / 非目标

**目标**

- 车道下拉默认只列 `增益` / `声像` + **已绑定的参数** + **已经画过曲线的参数**。
- 新增一个**独立参数页**（点击打开）：翻页显示该轨道**全部插件的全部参数**，支持查找 / 排序 / 筛选。
- 每行一个**绑定 / 解绑**按钮，绑定结果立即反映到车道下拉。
- 参数页与"插件界面"（`PluginEditorPane` / 原生编辑器）是**两个不同的东西**：参数页只做"挑参数给自动化用"，不做编辑。

**非目标（本期不做）**

- **右键"自动化这个参数"**（在旋钮/滑块上右键）。这是 DAW 的主力手势，但它需要先修 `Md3Knob`/`Md3Slider` 的按键门控（见 §8），另开一期。
- 不改工程模型 / IPC / MCP（绑定集合是 UI 偏好，见 §3.1）。
- 不做虚拟滚动（用翻页）；不做参数别名/重命名；不做用户自定义分组。

## 3 关键决策

### 3.1 绑定集合存**哪里**：UI 偏好（localStorage）

- 新增 `src/stores/paramBindings.ts`，照 `src/stores/settings.ts` 的写法（KEY `ai-daw-param-bindings`，初始化读一次 + `watch` 写回）。
- 形状：`Record<scopeKey, string[]>`，值是**目标键**（`automationTargetsFor` 产出的那种：`inst:<id>` / `fx<i>:<id>`），**数组顺序即列表顺序**（后绑的在后面）。
- `scopeKey = (当前工程路径 ?? "untitled") + "::" + trackId`。轨道 id 是后端按 1..N 发的，**每个工程都一样**，所以必须带工程标识，否则换工程就串。
- **`undefined` 与 `[]` 必须区分**：`undefined` = 「没有记录，用默认（§3.2）」；`[]` = 「用户显式全部解绑」。JSON 里这靠"键不存在"表达，读的时候用 `in` 判断，别用 `?? []` 一把梭。
- 工程**另存/首次保存**时要做 **re-key**：把 `untitled::<trackId>` 的记录搬到 `<path>::<trackId>`，否则用户一保存、列表就"变回默认"。

被否决的备选：写进工程模型（`Track.automation_targets?: string[]`）。好处是跟随工程、AI 可读可写；代价是模型 + IPC + 一条新命令（+ 可能要一个 MCP 工具），而它的价值只是"换个机器也记得我绑了哪些参数"。既然是视图偏好，就用视图的存储。

### 3.2 默认绑定 = 「精选」集，而不是"全部"

- 没有记录的插件：**内置合成器**默认绑 `COMMON_INSTRUMENT_PARAMS` 的那 8 个（`inst:<id>`）；其他插件默认**空**。
- 这个默认是**派生**出来的（`curateTargets` 里现算），**不写进 localStorage**。好处：以后改进精选表，没动过手的用户自动跟上。
- 于是开箱状态：内置合成器轨道下拉 = 增益 / 声像 + 8 个常用参数（可接受），外部插件轨道 = 增益 / 声像（需要显式绑定）。

### 3.3 车道下拉的可见集合

`可见 = [增益, 声像] + 已绑定的 + 已经有曲线的（即使已解绑）`。

最后一条很重要：**解绑不能把已有曲线"藏没"**——否则用户解绑后曲线还在（数据没删），但下拉里找不到它，看起来像丢了。所以"有曲线的参数"始终可见，只是排在绑定项之后。

**解绑不删数据**：`Track.automation` 里的曲线原样保留。

### 3.4 参数页 = 模态，作用域 = 整条轨道

- 形态：**模态**（克隆 `src/components/common/PluginBrowser.vue` 的外壳：`Teleport` → `.scrim` / `.dialog` + `.head` / `.toolbar` / `.body` / `.foot`），因为它已经有搜索药丸、筛选 chips、排序分段控件、`.list` / `.row` 的现成样式。
- 作用域：**整条轨道**，按插件分组（`乐器` / `效果器 1` / `效果器 2` …），行里带上组名——这样"我到底在绑谁"一眼可见。
- 两个入口，同一模态：
  1. **车道下拉的 action 行**「＋ 添加参数…」：`Md3Select` 的 `action` prop + `@action`（`InspectorPanel.vue:353` 已有同样的用法）→ 作用域 = 该车道所属轨道。
  2. **检查器每个插件卡片**的「参数…」按钮（乐器卡与效果器行各一个，样式沿用 `.open-editor.small`）→ 作用域 = 该轨道，并把插件筛选**预选**到该插件。
- 打开时对该轨道的乐器 + 全部效果器调 `plugins.ensureParams`（外部插件要异步取参数表），列表显示加载态。

### 3.5 翻页

房子里的既有风格是"一屏全渲染 + 滚动"，**没有翻页先例**。本期按要求做**真翻页**：

- 页大小默认 **16**，可切 `16 / 32 / 64 / 全部`。
- 底部：`上一页` / `第 x / y 页` / `下一页`，右侧 `共 N 个参数（筛出 M 个）`。
- **搜索 / 筛选 / 排序一变，回到第 1 页**（否则会停在越界的页上）。
- 每页最多 64 行，DOM 压力可控；外部插件几百个参数靠翻页 + 筛选兜底。

## 4 数据与接口

### 4.1 `src/stores/paramBindings.ts`（新）

```ts
/** 绑定集合：哪些目标键显示在自动化车道下拉里。UI 偏好，不进工程。 */
export const useParamBindingsStore = defineStore("paramBindings", () => {
  /** scopeKey -> 目标键数组（顺序即列表顺序）；键不存在 = 用默认精选。 */
  const bindings = ref<Record<string, string[]>>(load());

  /** 当前作用域键：工程路径（未保存时用 `untitled`）+ 轨道 id。 */
  function scopeKey(trackId: number): string;
  /** 该作用域显式绑定的键；**没有记录时返回 null**（调用方据此走默认）。 */
  function explicit(trackId: number): string[] | null;
  /** 已绑定 = 显式记录；没有记录则由调用方（curate）用默认精选补齐。 */
  function isBound(trackId: number, key: string): boolean;
  /** 绑定 / 解绑（幂等）；首次绑定会把默认精选"实体化"（见 §3.2）。 */
  function toggle(trackId: number, key: string): void;
  /** 保存工程后把 `untitled::<id>` 的记录搬到 `<path>::<id>`。 */
  function rekey(oldScope: string, newScope: string): void;
});
```

**"实体化"细节**：当某插件还是"没有记录"状态时，用户第一次点绑定/解绑，必须先把**默认精选**写进记录再改，否则"解绑一个精选参数"会变成"整组精选都消失"。

### 4.2 `src/utils/automation.ts`（改）

- `automationTargetsFor(track, paramsOf, t)` **保持"全量"**语义（参数页要用它列出一切）。
- 新增 `curateTargets(all, { bound, fallbackBound, usedKeys })`：按 §3.3 的规则过滤 + 排序，返回给车道下拉用的列表。
- 新增 `laneHasPoints(track, key): boolean`（"已用"判定）。
- 目标对象加 `hint`（见 §4.4）。

### 4.3 组件

- `src/components/common/ParameterBrowser.vue`（新）：模态。props `{ open, trackId, focusPluginId?, scopeLabel? }`，emit `update:open`。
  - 内部：`projectStore.track(trackId)` 取轨道 → `ensureParams` → `automationTargetsFor` 拿全量 → 应用搜索/筛选/排序 → 切片翻页 → 行。
  - 行 = `<div class="row">`（**不要**把按钮嵌在 button 里，照 `InspectorPanel.vue:378-405` 的效果器行写法）：显示名 + 原始 id + 单位/范围 + 当前值 + 绑定按钮（已绑 = 主色实心，未绑 = 描边）。
  - `automatable === false` 的行：可见但**禁用**，并注明"该插件声明此参数不可自动化"。
- `src/components/AutomationLane.vue` / `ControllerLane`（卷帘车道）：下拉改喂 `curateTargets`，并加 `:action="{ label: t('auto.browse') }"` + `@action` 打开参数页。
- `src/components/workspace/InspectorPanel.vue`：乐器卡与每个效果器行加「参数…」按钮 → 打开参数页（预选该插件）。
- 参数页的挂载点：`App.vue` 根部（像 `ProjectDialog` / `ShortcutsDialog` 那样，`App.vue:487-499`），打开状态放在一个小 store 里（`paramBrowser.open({ trackId, pluginId? })`），这样两个入口跨视图都能用。**不要**像 `PluginBrowser` 那样挂在 `InspectorPanel` 内部——车道下拉也需要它。

### 4.4 目标 hint（顺手做掉）

`AutomationTarget` 加 `hint?: string`（构建时已本地化，和 `label` 一样）：

- 内置/默认目标：手写文案，照 `lane.velocityHint` 的做法。`gain_db`、`pan`、`velocity` 各一条；合成器那 8 个精选参数可各配一句（和 `SYNTH_PARAM_LABELS` 并列一张表）。
- 插件参数：**不写散文**，自动拼「名称 · 单位 · 范围（默认 x）」——这正是真实 DAW 的做法（插件只给名字、单位、范围）。
- 车道头部在曲线模式下也渲染 `hint`（现在只渲染 `label`）。

## 5 与引擎 / AI 的关系

- **无 Rust 改动**：`Track.automation` 不变、`set_automation` 不变；被绑定的参数只是"下拉里能选到"，曲线仍旧走原来的命令。
- **MCP 不改**：绑定集合是 UI 偏好，不暴露给 agent（agent 本来就能对任意键 `set_automation`，不受绑定限制）。
- `skills/*.md` 不动。

## 6 验收

1. 打开参数页：看到该轨道**全部**插件的全部参数，按插件分组，组名带槽位号。
2. 搜索（显示名 / 原始 id / 单位）、排序、筛选（全部 / 已绑定 / 未绑定 / 已有曲线 + 仅可自动化 + 插件）都生效；翻页正确，页数随筛选变化，且**筛选一变回到第 1 页**。
3. 绑定 / 解绑**立即**反映到车道下拉；解绑**不删**已有曲线；有曲线的参数即使解绑仍在下拉里出现。
4. 默认状态：内置合成器轨道下拉 = 增益 / 声像 + 精选 8 个；外部插件轨道 = 增益 / 声像。
5. 刷新页面绑定保持；**换工程不串**（同一 trackId、不同工程各显示各自绑定）。
6. 参数页与插件界面是两个不同的东西：检查器里不再直接摊开通用参数列表，只有一个「参数…」按钮。
7. `npm run build` 通过；中英词典键数相等。

## 7 风险

| 风险 | 缓解 |
| --- | --- |
| 未保存的工程共用一个 `untitled` 桶（v1 限制） | 保存时 re-key；文档写明。彻底解决要等"绑定进工程模型"那一版 |
| 保存工程后绑定"变回默认" | re-key（§3.1）；这是本设计里最容易漏的一步，验收第 5 条盯着它 |
| `undefined` 被当成 `[]` → 默认精选整组消失 | §3.1 明确用 `in` 判断；§4.1 的"实体化" |
| 外部插件参数表要异步加载 | 打开时 `ensureParams`，列表加载态；加载完再算筛选/翻页 |
| 绑定的是**槽位下标键**（`fx<i>:…`），重排效果链后绑定项指向别的效果器 | 与曲线同一个 v1 取舍（设计 §9 已记）；参数页的插件筛选按**当前**槽位计算，不做缓存 |
| 库里已有缺陷：`Md3Knob` / `Md3Slider` 不判断按键，右键会把值跳到光标并提交 | 本期不做右键手势，但**这个 bug 该单独修**（见下一条非目标） |
