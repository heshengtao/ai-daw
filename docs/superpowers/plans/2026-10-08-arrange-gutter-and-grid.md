# 编排留白竖线 + 网格每拍一格 + 底栏读数修正 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 编排视图左侧留白放对应轨道色的竖线；两个视图的网格改成「每拍一格」（去掉固定 1/16 细线层）；修正底栏位置读数使其与网格小节线对齐。

**Architecture:** 网格是 `stores/timeline.ts` 里由拍号推导的一串 CSS 背景层，编排、编辑、控制器泳道共用同一个 `gridBackground`——去掉「细线」那一层即可让一个小节显示「分子个格子」。留白竖线是 `ArrangeView.vue` 每条 lane 里一个落在左侧 `TIMELINE_INSET_PX` 区的绝对定位元素，颜色取 `trackHue(index)`（与左栏 `.grip` 一致）。底栏读数把「按分子除」改成按 `barBeats = 分子 × 4/分母` 计算。

**Tech Stack:** Vue 3 `<script setup>` + Pinia + TypeScript、Tailwind + MD3 CSS 变量。

## Global Constraints

- 前端**没有测试框架**：验证 = `npm run build`（`vue-tsc --noEmit` + `vite build`）通过 + 浏览器 `npm run dev` 手动核对。不要引入 vitest/jest。
- **吸附精度不变**：`snap = meterBeat / 4` 保持原样；本计划只改**可见格线**，不碰吸附。
- 缩放（`MIN_ZOOM`/`MAX_ZOOM`/`zoomAt`）与滚动（`scrollLeft`）逻辑不动。
- 颜色/圆角走 `--md-*` token；轨道色统一用 `hsl(var(--track-hue) …)`。
- 注释与文案用中文；i18n 文案中英各一份（若新增文案）。
- 提交信息格式：`feat(timeline): …` / `fix(timeline): …`。

---

## File Structure

- `src/stores/timeline.ts` — `gridBackground` 去掉细线层。
- `src/views/ArrangeView.vue` — 每条 lane 加轨道色竖线。
- `src/components/layout/BottomBar.vue` — 小节/拍读数改用 `barBeats`。

---

### Task 1: 网格改成「每拍一格」

**Files:**
- Modify: `src/stores/timeline.ts`（`gridBackground` computed；文件头注释也更新）

**Interfaces:**
- Consumes: 现有的 `zoom`、`meterBeat`、`barBeats` computed。
- Produces: `gridBackground` 只含两层（拍线格 + 小节线）；`snap` / `snapBeats` 语义不变。

- [ ] **Step 1: 改 `gridBackground`**

把 `src/stores/timeline.ts` 里的 `gridBackground` computed 整段替换为：

```ts
  /**
   * 网格背景（编排与卷帘共用同一串）。
   *
   * 只画两层：**拍线**（一个小节 = 分子个格子）+ **小节线**。
   * 不再叠细分的吸附线——那会让一个小节看起来有很多格，和「一个小节几拍」对不上。
   * 吸附精度（`snap`）不受影响：拖动仍按 `snap = meterBeat / 4` 吸附，只是格线更干净。
   *
   * **顺序有讲究**：CSS 把 `background-image` 里**靠前**的那层画在最上面。
   * 小节线必然与拍线重合（`barBeats = sigNum × meterBeat`），所以小节线要排在前。
   */
  const gridBackground = computed(() => {
    const layers: string[] = [];
    layers.push(
      `repeating-linear-gradient(to right, var(--grid-bar) 0 1px, transparent 1px ${zoom.value * barBeats.value}px)`,
    );
    layers.push(
      `repeating-linear-gradient(to right, var(--grid-line) 0 1px, transparent 1px ${zoom.value * meterBeat.value}px)`,
    );
    return layers.join(", ");
  });
```

同时把文件头注释里「`snap = meterBeat / 4`：吸附网格」那段保留，但删掉/改写任何暗示「细分网格线会画出来」的描述（注释里若提到细线网格，改成「吸附用 `meterBeat / 4`，但格线只画到拍」）。

> 注意（执行后修正）：这里的层顺序是**小节线在前、拍线在后**。见文末「执行后的遗留 H4」——反过来会让小节线被拍线盖掉。

- [ ] **Step 2: 类型检查 + 构建**

Run: `npm run build`
Expected: 成功

- [ ] **Step 3: 手动核对**

Run: `npm run dev`，在编排视图与编辑视图分别改底栏拍号：
Expected:
- 4/4 → 一个小节 4 个格子；3/4 → 3 个；6/8 → 6 个；7/8 → 7 个；
- 编排、编辑、下方控制器泳道的格线一致；
- 拖动片段/音符仍能吸附到比格线更细的位置（1/16 拍）。
核对完停止 dev server。

- [ ] **Step 4: 提交**

```bash
git add src/stores/timeline.ts
git commit -m "feat(timeline): one grid cell per beat, drop the fixed 16th line layer"
```

---

### Task 2: 编排左侧留白放轨道色竖线（可点选该音轨）

**Files:**
- Modify: `src/views/ArrangeView.vue`（模板里的 `.lane`、`.arrange` 内联样式、`<style scoped>`）

**Interfaces:**
- Consumes: `trackHue(index)`（已 import）、`TIMELINE_INSET_PX`（本文件常量）、`tracks` 的 `ti` 索引、`selection.selectTrack(id)`、`i18n.t(...)`。
- Produces: 每条 lane 在左侧留白区有一条 `--track-hue` 竖线 + 一层同色淡底；**点击这条竖线即选中该音轨**。

- [ ] **Step 1: 暴露留白宽度为 CSS 变量**

`.arrange` 的内联样式现在是 `{ paddingLeft: `${TIMELINE_INSET_PX}px` }`。改成：

```html
    :style="{ paddingLeft: `${TIMELINE_INSET_PX}px`, '--timeline-inset': `${TIMELINE_INSET_PX}px` }"
```

- [ ] **Step 2: lane 里加竖线元素（可点击选中音轨）**

在 `.lane` 模板内、**第一个子元素位置**（`.clip-ghost` 之前）加：

```html
        <button
          class="lane-accent"
          type="button"
          :style="{ '--track-hue': String(trackHue(ti)) }"
          :aria-label="i18n.t('arrange.selectTrack')"
          :aria-pressed="selection.trackId === track.id"
          @click.stop="selection.selectTrack(track.id)"
        />
```

并在 `src/i18n/zh.ts` / `src/i18n/en.ts` 各加一个键 `arrange.selectTrack`（zh：`选中这条音轨`，en：`Select this track`）。

- [ ] **Step 3: 加样式**

在 `<style scoped>` 的 `.lane` 规则之后加：

```css
/* 左侧留白里的轨道色竖线：与左栏 .grip 同色，留白不再突兀 */
.lane-accent {
  position: absolute;
  top: 0;
  bottom: 0;
  left: calc(-1 * var(--timeline-inset, 26px));
  width: var(--timeline-inset, 26px);
  padding: 0;
  border: 0;
  pointer-events: auto;
  cursor: pointer;
  background-color: hsl(var(--track-hue) 55% 55% / 0.1);
}
.lane-accent:hover {
  background-color: hsl(var(--track-hue) 55% 55% / 0.18);
}
.lane-accent:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: -2px;
}
.lane-accent::before {
  content: "";
  position: absolute;
  top: 0;
  bottom: 0;
  left: 0;
  width: 3px;
  border-radius: 0 3px 3px 0;
  background-color: hsl(var(--track-hue) 55% 55%);
}
```

- [ ] **Step 4: 类型检查 + 构建**

Run: `npm run build`
Expected: 成功

- [ ] **Step 5: 手动核对**

Run: `npm run dev`，编排视图：
Expected: 每条轨道左侧留白里有一条与左栏把柄同色的竖线；悬停时略微加深；**点击竖线会选中该音轨**（左栏该行高亮、检查器跟随）。

关于滚动：留白竖线是 lane 内 `left: calc(-1 * var(--timeline-inset))` 的绝对定位元素，也就是**滚动内容自己的左内边距**，不是固定在视口左缘的覆盖层。因此它**随内容一起横向滚动**——滚到后面时留白与竖线会移出视口左缘。这是实现后接受的既有行为（见文末「执行后的遗留 H2」），不是缺陷。

手动核对清单：
- 竖线颜色与左栏把柄同色，悬停时加深；
- 点击竖线选中该音轨（左栏该行高亮、检查器跟随）；
- **滚动到第 8 小节附近再检查留白**：留白与竖线应随内容一起滚出视口左缘，而不是钉在左缘。

- [ ] **Step 6: 提交**

```bash
git add src/views/ArrangeView.vue
git commit -m "feat(arrange): per-track color accent in the left gutter"
```

---

### Task 3: 底栏位置读数改用 `barBeats`

**Files:**
- Modify: `src/components/layout/BottomBar.vue`（`bars`、`positionText` computed）

**Interfaces:**
- Consumes: 现有 `timeSigNum` / `timeSigDen` computed、`transport.positionBeats`。
- Produces: 小节数与小节.拍读数与网格小节线一致（6/8、7/8 等拍号下也对齐）。

- [ ] **Step 1: 改两个 computed**

把 `src/components/layout/BottomBar.vue` 里的 `bars` 与 `positionText` 两个 computed 替换为：

```ts
/** 工程内容长度（小节）。与网格/标尺共用 timeline store 的同一个公式（spec §4.6）。 */
const bars = computed(() => {
  const beats = timeline.contentBeats;
  if (beats <= 0) return 0;
  return Math.ceil(beats / timeline.barBeats);
});

/** 当前位置（小节.拍），随实时位置推进。小节/拍长度同样取自 timeline store。 */
const positionText = computed(() => {
  const beats = transport.positionBeats;
  const bar = Math.floor(beats / timeline.barBeats) + 1;
  const beat = Math.floor((beats % timeline.barBeats) / timeline.meterBeat) + 1;
  return `${bar}.${beat}`;
});
```

并在文件顶部加 `import { useTimelineStore } from "@/stores/timeline"` 与 `const timeline = useTimelineStore()`。

> 注意（执行后修正）：**不要**在 `BottomBar.vue` 里重复定义 `barBeats` / `meterBeat`，也不要再自己扁平化所有 clip 算内容拍数——spec §4.6 要求这两处都走 timeline store（`timeline.barBeats` / `timeline.meterBeat` / `timeline.contentBeats`）。原先的 `Math.max(1, …)` 也去掉了：内容为 0 拍时显示 0 小节。依赖方向是 BottomBar → timeline/project/transport，无循环。

- [ ] **Step 2: 类型检查 + 构建**

Run: `npm run build`
Expected: 成功

- [ ] **Step 3: 手动核对**

Run: `npm run dev`，分别设 4/4、6/8、7/8，按播放：
Expected: 底栏「小节」计数与标尺上的小节线数量一致；「小节.拍」读数在每小节线处归 1。

- [ ] **Step 4: 提交**

```bash
git add src/components/layout/BottomBar.vue
git commit -m "fix(bottom): bar/beat readout uses barBeats so it matches the grid"
```

---

## Self-Review

**Spec coverage（对照 `2026-10-08-builtin-synth-ui-redesign-design.md`）**
- 4.5 编排左侧留白竖线（含点击选中音轨）→ Task 2。
- 4.6 网格每拍一格 → Task 1。
- 4.6（底栏读数修正）→ Task 3。

**Placeholder 扫描**：无 TBD/TODO；样式与代码均为具体值。

**类型一致性**：`barBeats`/`meterBeat`/`contentBeats` 由 `timeline.ts` 单一来源提供，`BottomBar.vue` 不再各算一份（见 Task 3 Step 1 的执行后修正）。

**依赖提示**：本计划的 `gridBackground` 会在计划 D（动态 BPM/节拍）里被推广为**分段叠层**——届时「无拍号关键点」的情形必须仍等价于本计划的结果。

---

## 执行后的遗留

本计划执行完、并通过最终整支复审后，记录几项**已接受**的收尾（均已落地或明确不修）：

- **H1（已修）`--grid-line-fine` 死 token 已删除**：可见格线只剩拍线 / 小节线两级，`main.css` 亮色与深色块里的 `--grid-line-fine` 已移除，相关注释同步改成两级。`timeline.ts` 里无消费方的 `snapPx` / `beatPx` 两个导出也一并删掉（只留 `barPx`，由 `TimelineRuler.vue` 使用）。
- **H2（接受）留白竖线随内容滚动**：竖线落在滚动内容的左内边距里，所以滚到后面会随内容滚出视口。固定不动的覆盖层会带来额外的层级与命中区域问题，本计划不修；Task 2 Step 5 的期望文案已按此改正，并加入「滚动到第 8 小节附近再检查留白」。
- **H3（接受）`aria-pressed` 用单选用法**：竖线按钮用 `aria-pressed` 表达「当前选中的轨道」。多选轨道时它表达不了全貌，但当前选择模型本就是单选；等真正支持多选时再换成合适的语义。`aria-label` 现为「选中这条音轨 · <轨道名>」，便于区分同名/同色轨道。
- **H4（已修）背景层顺序**：小节线与拍线必然重合（`barBeats = 分子 × meterBeat`），而 CSS 把 `background-image` 里靠前的一层画在最上面。原来的顺序让`--grid-bar` 被更淡的 `--grid-line` 盖住，小节线实际不可见；现改为**先 push 小节线层**，Task 1 Step 1 的代码块已同步。
