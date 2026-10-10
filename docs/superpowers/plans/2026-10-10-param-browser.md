# 参数浏览器（自动化目标绑定）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal：** 修掉"自动化车道下拉把插件全部参数摊开、长到没法用"的问题：新增一个可点击打开的**参数页**（翻页 + 查找 + 排序 + 筛选，每行可绑定/解绑），车道下拉只列 `增益` / `声像` + 已绑定的 + 已经有曲线的参数。

**Architecture：** 绑定集合是**纯前端 UI 偏好**（新 Pinia store + localStorage，按「工程 + 轨道」隔离），不进工程模型、不动 IPC/MCP、零 Rust 改动。`utils/automation.ts` 保持"全量"取数，新增 `curateTargets` 做"给车道看"的收窄；参数页是一个模态，外壳克隆 `PluginBrowser.vue`，挂到 `App.vue` 根部，由两个入口（车道下拉的 action 行、检查器的「参数…」按钮）打开。

**Tech Stack：** Vue 3 + Pinia + vue-router、MD3 自绘组件、`localStorage` 偏好存储。无新依赖。

设计见 `../specs/2026-10-10-param-browser-design.md`。

## Global Constraints

- **许可 Apache-2.0-only；不新增任何依赖**（`package.json` 不动）。
- **不改工程模型 / IPC / MCP**：`Track` / `Command` / `Event` / `src-tauri/**` / `crates/**` 一律不动。绑定集合只活在 localStorage。
- `npm run build`（`vue-tsc --noEmit` + `vite build`）必须通过；每个任务结束时**中英词典键数必须相等**。
- **`undefined` ≠ `[]`**：绑定记录"键不存在"表示"用默认精选"，`[]` 表示"用户显式全解绑"。读的时候用 `in` / `hasOwnProperty` 判断，**不要** `?? []`。
- **不要把 `<button>` 嵌进 `<button>`**：列表行照 `InspectorPanel.vue:378-405` 的写法，用 `<div class="row">` 承载行内的多个按钮。
- 复用既有 CSS 语言（`rgb(var(--md-…))` token、`.scrim`/`.dialog`、`.row`/`.chip`/`.seg`），不要另起一套视觉。
- 本期**不做**右键手势（旋钮/滑块上右键"自动化这个参数"），见设计 §2；也**不要**顺手去改 `Md3Knob`/`Md3Slider`（那是另一期，且要先修按键门控）。

## 文件清单

- 新建：`src/stores/paramBindings.ts`、`src/stores/paramBrowser.ts`、`src/components/common/ModalShell.vue`、`src/components/common/ListToolbar.vue`、`src/components/common/ParameterBrowser.vue`
- 修改：`src/utils/automation.ts`、`src/data/plugins.ts`、`src/components/common/PluginBrowser.vue`（改用共享件）、`src/components/workspace/AutomationLane.vue`、`src/components/workspace/ControllerLane.vue`、`src/views/EditView.vue`、`src/components/workspace/InspectorPanel.vue`、`src/App.vue`、`src/stores/project.ts`（保存后 re-key）、`src/i18n/zh.ts`、`src/i18n/en.ts`、`docs/superpowers/ROADMAP.md`

---

## Task 1: 绑定数据层（store + 收窄逻辑 + 默认精选）

**Files:**
- Create: `src/stores/paramBindings.ts`
- Modify: `src/utils/automation.ts`、`src/data/plugins.ts`、`src/i18n/zh.ts`、`src/i18n/en.ts`

**Interfaces:**
- Produces: `useParamBindingsStore()`（`explicit(trackId)` / `isBound(trackId, key)` / `toggle(trackId, key)` / `rekey(oldScope, newScope)` / `scopeKey(trackId)`），
  `curateTargets(all, { bound, usedKeys, fallbackBound })`，`laneHasPoints(track, key)`，
  `AutomationTarget` 新增可选 `hint?: string`。

- [ ] **Step 1: 建 `stores/paramBindings.ts`**

照 `src/stores/settings.ts:22-43` 的写法（KEY `"ai-daw-param-bindings"`，"初始化读一次 + `watch` 写回"）：

```ts
/** 绑定集合：哪些自动化目标键显示在车道下拉里。纯 UI 偏好，不进工程。 */
const KEY = "ai-daw-param-bindings";
/** 工程路径未知（未保存）时用的作用域前缀。 */
const UNTITLED = "untitled";

type Bindings = Record<string, string[]>;

function load(): Bindings {
  try {
    const raw = localStorage.getItem(KEY);
    return raw ? (JSON.parse(raw) as Bindings) : {};
  } catch {
    return {};
  }
}
```

store 内部：`const bindings = ref<Bindings>(load())`；`watch(bindings, persist, { deep: true })`。

- `scopeKey(trackId: number)`：`` `${project.currentPath ?? UNTITLED}::${trackId}` ``（工程标识来自 `useProjectStore().currentPath`）。
- `explicit(trackId)`：**用 `Object.hasOwn(bindings.value, key)` 判断**有没有记录；有则返回数组（可能是 `[]`），没有返回 `null`。
- `toggle(trackId, key, fallbackBound: string[])`：没有记录时先 `bindings.value[scope] = [...fallbackBound]`（**实体化默认精选**，见设计 §4.1），再增/删 `key`；`[]` 时保留空数组（不要删键）。
- `rekey(oldScope, newScope)`：把旧键数组搬到新键（新键已有记录则不覆盖），删旧键。

- [ ] **Step 2: `utils/automation.ts` 加 `hint`、`laneHasPoints`、`curateTargets`**

```ts
/** 该参数在这条轨道上是否已经有曲线（"已用"判定）。 */
export function laneHasPoints(track: Track | null | undefined, key: string): boolean {
  return (track?.automation ?? []).some((l) => l.param === key && l.points.length > 0);
}

/**
 * 收窄给车道下拉用的目标列表：增益 / 声像 + 已绑定 + **已经有曲线的**。
 *
 * `bound === null` 表示"没有显式记录" → 用 `fallbackBound`（默认精选）。
 * 已经有曲线的即使没绑定也留在列表里（否则曲线会"看不见"），排在绑定项之后。
 */
export function curateTargets(
  all: AutomationTarget[],
  opts: { bound: string[] | null; fallbackBound: string[]; usedKeys: string[] },
): AutomationTarget[];
```

规则：
1. `gain_db` / `pan` 永远在最前（按 `all` 里的原始顺序）。
2. 然后 `bound ?? fallbackBound` 中能在 `all` 里找到的键（**保持绑定顺序**；`fallbackBound` 里不存在的键忽略）。
3. 最后 `usedKeys` 里还没出现的键（已用但已解绑）——同样要在 `all` 里存在。
4. 目标对象原样透传（含 `hint`）。

- [ ] **Step 3: `data/plugins.ts` 补默认精选键与 hint 表**

```ts
/** 某插件"默认绑定"的目标键（`inst:` / `fx<i>:` 只给参数 id，前缀由调用方拼）。 */
export function defaultBoundParamIds(pluginId: string): string[] {
  return COMMON_INSTRUMENT_PARAMS[canonicalPluginId(pluginId)] ?? [];
}
```

并在 `SYNTH_PARAM_LABELS` 旁加一张 `SYNTH_PARAM_HINTS: Record<string, string>`（至少覆盖那 8 个精选参数；文案走 i18n 的 `param.hint.*` 键，见 Step 4）。

- [ ] **Step 4: i18n 加文案（中英各一遍，键数必须相等）**

- `param.hint.gain_db` / `param.hint.pan` / `param.hint.velocity`：各一句解释。
  例（zh）：`增益` →「轨道推子：整体音量（dB）。自动化**覆盖**推子值，曲线拿掉就回到推子。」
- `param.hint.inst.<id>`（8 个精选参数各一条；或统一落到 `param.hint.<paramId>`）。
- 插件参数的 hint 不写文案，由 `automationTargetsFor` 自动拼「名称 · 单位 · 范围（默认 x）」。

- [ ] **Step 5: 跑构建**

Run: `npm run build`
Expected: 通过（本任务只有纯 TS，`vue-tsc` 会盯类型）。

- [ ] **Step 6: 给 `curateTargets` 一条真能跑的断言（本仓库没有前端测试运行器）**

`curateTargets` 是本期唯一有真实分支的纯函数，只靠"看起来对"不够。用已在 `node_modules` 里的 esbuild 把它打成 ESM，再用一次性脚本断言（脚本放会话 scratchpad，**不进仓库**；输出贴进报告）：

```bash
node_modules/.bin/esbuild src/utils/automation.ts --bundle --format=esm \
  --alias:@=./src --outfile=<scratchpad>/automation.mjs
node <scratchpad>/check-curate.mjs
```

必须断言的行为（对应设计 §3.3）：

1. `增益` / `声像` 永远在最前，且保持 `all` 里的原始顺序。
2. `bound` 的顺序被保留（后绑的在后面）。
3. `bound === null` → 用 `fallbackBound`。
4. `bound === []` → **只**剩 `增益` / `声像`（**不是**回落默认精选——这是最容易写错的一条）。
5. `usedKeys` 里"已用但已解绑"的追加在后面；`all` 里不存在的键一律忽略。

- [ ] **Step 7: 提交**

```bash
git add src/stores/paramBindings.ts src/utils/automation.ts src/data/plugins.ts src/i18n/zh.ts src/i18n/en.ts
git commit -m "feat(ui): 自动化目标绑定的数据层（UI 偏好存储 + 收窄 + 默认精选 + hint）"
```

---

## Task 2: 参数页模态 + 打开状态（含抽取共享外壳与工具条）

**Files:**
- Create: `src/components/common/ModalShell.vue`、`src/components/common/ListToolbar.vue`、`src/stores/paramBrowser.ts`、`src/components/common/ParameterBrowser.vue`
- Modify: `src/components/common/PluginBrowser.vue`、`src/App.vue`、`src/i18n/zh.ts`、`src/i18n/en.ts`

**为什么先抽壳**：`PluginBrowser.vue:141-204` 的外壳（scrim/dialog/head/foot）与工具条（搜索药丸/chips/排序分段）加起来约 170 行 CSS，"克隆一份"就是**逐字复制一个逻辑块**——评审会（正确地）把它记成 Important。所以先抽成两个共享组件，新模态只组合它们，`PluginBrowser` 也改用。**抽取必须行为不变。**

**Interfaces:**
- Consumes: Task 1 的 store / `curateTargets` / `automationTargetsFor` / `ensureParams`。
- Produces:
  `ModalShell.vue` — props `{ open: boolean; title: string }`，emits `update:open`；默认 slot = body，具名 slot `toolbar`、`foot`；自己管 Escape、点 scrim 关闭、`role="dialog"` `aria-modal`、`z-index:120`、`width:620px`、`max-height:78vh`。
  `ListToolbar.vue` — props `{ query: string; placeholder: string; chips: {id:string;label:string}[]; activeChip: string; sorts: {id:string;label:string}[]; activeSort: string }`，emits `update:query` / `update:activeChip` / `update:activeSort`。
  `useParamBrowserStore()` — `open({ trackId, pluginId? })` / `close()` / `isOpen` / `trackId` / `pluginId`（**不持久化**）。
- 依赖顺序：Step 1/2（抽件）→ Step 3/4（新模态）→ Step 5+ 才有意义。

- [ ] **Step 1: 抽 `ModalShell.vue`，并让 `PluginBrowser` 改用**

把 `PluginBrowser.vue:141-151` 的 `Teleport` → `.scrim`（`@pointerdown.self` 关闭）→ `.dialog`（`role="dialog"` `aria-modal="true"` `tabindex="-1"` `@keydown`）→ `.head`（`.large .title` + `.icon-btn`）搬进 `ModalShell.vue`，CSS（`:254-309`）**连 class 名一起搬**。`PluginBrowser` 删掉自己那份、改成组合。
**行为逐项不变**（这是本步的验收）：Escape 关闭、点 scrim 关闭、打开时聚焦搜索框（`:128-137`）、`z-index:120`、`width:620px`、`max-height:78vh`、标题与关闭按钮位置一致。

- [ ] **Step 2: 抽 `ListToolbar.vue`，并让 `PluginBrowser` 改用**

把 `PluginBrowser.vue:159-204`（搜索药丸 + `.chips` + `.sorts` 分段）与 CSS `:310-426` 搬进 `ListToolbar.vue`，改成 props 驱动 + emit 回写。`PluginBrowser` 改用后，**筛选项与交互逐项不变**（同样的 chip 集合、同样的排序项、同样的清空按钮）。

- [ ] **Step 3: `stores/paramBrowser.ts`**

一个极轻的 store：`isOpen = ref(false)`、`trackId = ref<number | null>(null)`、`pluginId = ref<string | null>(null)`，`open(...)` / `close()`。**不持久化。**

- [ ] **Step 4: `ParameterBrowser.vue` 用两个共享件拼出外壳**

`ModalShell`（标题「参数（用于自动化）」）+ `ListToolbar` 放 `toolbar` slot + body 放行列表 + `foot` 放翻页条。**不要**再复制一份 scrim/dialog/toolbar 的 CSS。

- [ ] **Step 5: 取数 + 分组**

`trackId` → `project.track(trackId)`；打开时对该轨道的 `instrument.plugin_id` 与**所有** `effects[i].plugin_id` 调 `plugins.ensureParams(id)`（外部插件异步），加载中显示占位。
然后 `automationTargetsFor(track, plugins.paramsOf, i18n.t)` 拿**全量**目标，每个目标带 `key` / `label` / `group` / `hint`。行里额外显示：组名（乐器 / 效果器 i）、原始参数 id、单位与范围。

- [ ] **Step 6: 搜索 / 筛选 / 排序的**取值逻辑**（样式与交互已在 `ListToolbar`）**

- 搜索：匹配 **显示名 / 原始 id / 单位**（大小写不敏感）。
- 筛选 chips：`全部` / `已绑定` / `未绑定` / `已有曲线`；另一个 chips：`仅可自动化`（**默认开**）。
- 插件筛选：`Md3Select`，选项 `全部` / `乐器` / `效果器 1` / …；`pluginId` 传入时**预选**该插件。
- 排序：`默认（插件顺序）` / `参数名` / `已绑定在前`。
- **任一筛选 / 搜索 / 排序变化 → `page.value = 1`。**

- [ ] **Step 7: 翻页 + 行 + 绑定按钮**

- 页大小 `pageSize` 默认 16，可切 `16 / 32 / 64 / 全部`；底部 `上一页` / `第 x / y 页` / `下一页`，右侧 `共 N 个（筛出 M 个）`。
- 行：`<div class="row">` + 名称 span + id/单位 span + 当前值 span + 绑定按钮（`已绑定` = 主色实心，`未绑定` = 描边）；按钮 `@click` → `bindings.toggle(trackId, key, fallback)`。**不要**把按钮嵌进 button。
- `automatable === false` 的目标：行可见但按钮禁用，并注明"该插件声明此参数不可自动化"。

- [ ] **Step 8: 挂到 `App.vue`**

在 `App.vue:487-499` 那几个 dialog 旁边挂 `<ParameterBrowser v-if="browser.isOpen" />`（**不要**挂进 `InspectorPanel`，车道下拉也要用它）。

- [ ] **Step 9: i18n + 构建 + 浏览器回归 + 提交**

新增键：`browser.title`、`browser.search`、`browser.filter.*`、`browser.sort.*`、`browser.page.prev/next/of`、`browser.count`、`browser.bind`、`browser.unbind`、`browser.notAutomatable`、`browser.loading`。

Run: `npm run build` + i18n 键数相等。
**浏览器回归由编排者执行**（端口 1420 被用户的 dev server 占着，别自己起）：打开插件浏览器，确认搜索/筛选/排序/选中/关闭/Escape 与改动前一致（Step 1/2 是行为不变的抽取，这条回归就是它的验收）。

```bash
git add src/components/common/ModalShell.vue src/components/common/ListToolbar.vue src/stores/paramBrowser.ts src/components/common/ParameterBrowser.vue src/components/common/PluginBrowser.vue src/App.vue src/i18n/zh.ts src/i18n/en.ts
git commit -m "feat(ui): 抽出共享的 ModalShell/ListToolbar，新增参数浏览器模态（翻页+查找+排序+筛选+绑定/解绑）"
```

---

## Task 3: 车道下拉改用收窄列表 + 「＋ 添加参数…」入口

**Files:**
- Modify: `src/components/workspace/AutomationLane.vue`、`src/components/workspace/ControllerLane.vue`、`src/views/EditView.vue`、`src/i18n/zh.ts`、`src/i18n/en.ts`

**Interfaces:**
- Consumes: Task 1 的 `curateTargets` / `laneHasPoints` / `useParamBindingsStore`；Task 2 的 `useParamBrowserStore`。

- [ ] **Step 1: `AutomationLane.vue` 换喂数**

`const all = computed(() => automationTargetsFor(...))` 保持全量取数，新增：

```ts
const bound = computed(() => bindings.explicit(trackId.value));       // null = 用默认
const fallback = computed(() => defaultBoundKeys(track.value));        // 精选集（拼上 inst:/fx< i>: 前缀）
const used = computed(() => (track.value?.automation ?? [])
  .filter((l) => l.points.length > 0).map((l) => l.param));
const targets = computed(() => curateTargets(all.value, {
  bound: bound.value, fallbackBound: fallback.value, usedKeys: used.value,
}));
```

`selection.track` 变化 / 插件参数表加载后要重算（现有 `watch` 上加依赖即可）。
**保留**已有的"选中的目标消失了就回落"逻辑。

- [ ] **Step 2: 下拉加 action 行**

`Md3Select` 加 `:action="{ label: i18n.t('auto.browse') }"` 与 `@action="browser.open({ trackId })"`（`Md3Select.vue:34,42,197-201` 已支持；`InspectorPanel.vue:353` 是同款用法）。

- [ ] **Step 3: 卷帘车道同样处理**

`ControllerLane.vue` 的 `chooserOptions` 里 `力度` 之前的部分改用收窄结果（`力度` 永远第一项），并把「＋ 添加参数…」的 action 透传给 `EditView`（新增 emit），`EditView.vue:513-521` 的 `controllerTargets` 改成 `curateTargets(...)`，并接住 emit 调 `browser.open`。

- [ ] **Step 4: 构建 + 浏览器回归 + 提交**

Run: `npm run build`
手工（mock 后端，单页内用 hash 导航，别用会重置状态的整页跳转）：
1. 内置合成器轨道：车道下拉 = `增益` / `声像` + 精选 8 个（**不再**是全部 26 个）。
2. 加一个延迟：下拉出现 `效果器 1` 组，**只**列绑定的那些（默认空 → 该组不出现，除非去参数页绑）。
3. 点「＋ 添加参数…」→ 参数页打开、作用域是该轨道。
4. 在参数页绑一个延迟参数 → 关掉 → 下拉里出现 `效果器 1 · <参数>`。
5. 解绑它 → 下拉里消失；若它已有曲线，则**仍在**列表里（只是排在后面）。

```bash
git add src/components/workspace/AutomationLane.vue src/components/workspace/ControllerLane.vue src/views/EditView.vue src/i18n/zh.ts src/i18n/en.ts
git commit -m "feat(ui): 车道下拉改用收窄列表，并加“＋ 添加参数…”入口"
```

---

## Task 4: 检查器的「参数…」按钮

**Files:**
- Modify: `src/components/workspace/InspectorPanel.vue`、`src/i18n/zh.ts`、`src/i18n/en.ts`

- [ ] **Step 1: 乐器卡加按钮**

在「打开界面」旁加一个「参数…」pill（沿用 `.open-editor` / `.open-editor.small`，`InspectorPanel.vue:735-755`），`@click="browser.open({ trackId: track.id, pluginId: track.instrument?.plugin_id })"`。**不要**把通用参数列表直接摊在卡片里（那是本设计要避免的）。

- [ ] **Step 2: 每个效果器行加按钮**

效果器行（`InspectorPanel.vue:378-406`）加一个 `.mini` 或 `.open-editor.small` 的「参数…」，`pluginId` 传该效果的 `plugin_id`。行内已有 `.strip-buttons`，注意**不要**嵌套 button。

- [ ] **Step 3: 构建 + 浏览器回归 + 提交**

Run: `npm run build`
手工：点乐器卡的「参数…」→ 参数页打开且**插件筛选预选到该乐器**；点效果器行的 → 预选到该效果器。

```bash
git add src/components/workspace/InspectorPanel.vue src/i18n/zh.ts src/i18n/en.ts
git commit -m "feat(ui): 检查器加“参数…”入口（与插件界面区分开）"
```

---

## Task 5: 文档 + 端到端验收

**Files:**
- Modify: `docs/superpowers/specs/2026-10-10-param-browser-design.md`
- **本分支不动 `docs/superpowers/ROADMAP.md`**：工作区里那份 ROADMAP 有用户未提交的 Stage F 改向（`aidaw` CLI 改 MCP 薄客户端、F1 降级 F5），另有他新写的 `plans/2026-10-10-aidaw-cli-mcp-client.md`。整目录 `git add` 会把它们卷进本分支的提交，所以这里的每一步都必须**逐文件 add**。ROADMAP 里"参数浏览器"那一行等他的改向落到 main 之后再补。

- [ ] **Step 1: 走一遍设计 §6 的 7 条验收**，逐条记录结果（尤其是第 5 条"换工程不串"与"保存后 re-key"）。
  保存 re-key 若还没接上，**在这里接**：在 `stores/project.ts` 的保存/另存路径里，`currentPath` 由 `null` 变成路径时调用 `bindings.rekey(untitledScope, newScope)`。

- [ ] **Step 2: ROADMAP 那一行 —— 本分支不做**（理由见 Files）。记进设计文档 §7 的"待补"即可，等用户的 Stage F 改向提交到 main 之后再补那一行。

- [ ] **Step 3: 设计文档状态行**改成 `✅ 完成（2026-10-10）`，并把实现中偏离设计的地方记进 §7。

- [ ] **Step 4: 提交**

```bash
git add docs/superpowers/specs/2026-10-10-param-browser-design.md docs/superpowers/plans/2026-10-10-param-browser.md
git commit -m "docs: 参数浏览器的执行结果与验收记录"
```

**逐文件 add，永远不要 `git add docs/`**：工作区里有用户未提交的 ROADMAP 改向和一份他新写的 plan，整目录 add 会把它们卷进本分支的提交。

---

## 自查记录

- **Spec 覆盖**：§3.1/§3.2→Task 1；§3.4/§3.5→Task 2；§3.3→Task 3；§4.4→Task 1/2；§6 验收→Task 5。
- **占位符扫描**：无 TBD；Task 1/2 给了可直接粘贴的关键代码，Task 2 的样式要求是"抽共享件"而不是"重写一份 CSS"。
- **类型一致性**：`explicit()` 返回 `string[] | null` 在 Task 1/3 之间一致；`curateTargets` 的 `bound: string[] | null` 与之一致；`browse` action 的 emit 名与 `Md3Select.vue:42` 一致。
- **风险最高的两处**（实现时盯紧）：`undefined` ≠ `[]`；保存工程后的 re-key。
- **开工前改掉的三处**（原稿会被评审正确地记成问题）：① 原稿让新模态"克隆 `PluginBrowser` 的外壳与工具条"——那是逐字复制约 170 行 CSS，改成先抽 `ModalShell`/`ListToolbar` 并让 `PluginBrowser` 改用（Task 2 Step 1/2）；② 原稿对 `curateTargets` 只有 `npm run build`，现在补了 esbuild + 一次性脚本的断言（Task 1 Step 6）；③ 原稿漏了 `src/stores/project.ts`（re-key 挂在保存路径上）。
- **仍然存在的缺口（记录在案，不在本期解决）**：本仓库没有前端测试运行器，Task 1 的断言是**一次性脚本**而非可持续的回归套件；`curateTargets` 之外的 UI 逻辑仍只靠构建 + 浏览器回归。要根治得先决定引入 vitest——那是独立的一期。
