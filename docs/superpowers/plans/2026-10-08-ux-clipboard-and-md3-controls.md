# 交互与控件统一 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 音频块/音符支持单条复制粘贴（副本落到尾部）、单击空白按「上一次选中长度」快速创建、控件统一到一套 MD3 药丸样式、文案全 i18n、logo/默认色为薰衣草紫、标签页自动跳转、控制器默认音量、去掉空片段提示。

**Architecture:** 以 `Md3Select` 的触发框（34px / 999px 药丸）作为**唯一**的输入/下拉规范，抽成 `main.css` 的共享类 `.md3-field`，把 `InstrumentSelect` 合并进 `Md3Select` 后删除；复制粘贴用一个新的内存 store `clipboard`，把「上次选中长度」放进 `selection` store；其余都是小改。

**Tech Stack:** Vue 3 `<script setup>` + Pinia + TypeScript、Tailwind + MD3 CSS 变量。

## Global Constraints

- 前端**没有测试框架**：验证 = `npm run build`（`vue-tsc --noEmit` + `vite build`）通过 + `npm run dev` 手动核对。**不要**引入 vitest/jest。
- 规范控件样式（唯一）：`height:34px; border-radius:999px; padding:0 12px; border:1px solid rgb(var(--md-outline)/0.7); background-color:rgb(var(--md-surface-container-high)); color:rgb(var(--md-on-surface)); font-size:12px`；focus 态 `border-color:rgb(var(--md-primary))` + `box-shadow:0 0 0 2px rgb(var(--md-primary)/0.22)`。
- **全项目不得出现原生 `<select>`**；自定义输入组件外观必须走规范样式。
- 颜色/圆角走 `--md-*` token（`rgb(var(--md-*))`）。
- 注释与文案用中文；**所有新出现的界面文案都要 i18n**（`zh.ts` + `en.ts` 各一份）。
- 不改 `Md3Select` 的对外 props（`modelValue/options/label/placeholder/empty/disabled`）与 emits（`update:modelValue` / `select`）——只**新增**可选能力。
- 提交信息格式：`feat(ui): …`。

## 依赖顺序

本计划应在计划 B（编排留白/网格）与计划 D（动态 BPM/节拍）**之后**执行：B、D 会改 `ArrangeView.vue` / `EditView.vue` / `stores/timeline.ts`，本计划再在其上叠加。吸附一律用稳定的 `timeline.snapBeats(beat)`。

---

## File Structure

- `src/components/common/Md3Select.vue` — 新增可选底部动作行。
- `src/components/common/InstrumentSelect.vue` — 删除。
- `src/assets/main.css` — `.md3-field` 共享类；清理死类；显式 `[data-accent="lavender"]`。
- `src/components/workspace/InspectorPanel.vue` — 音色行改 `Md3Select`；输入框统一。
- `src/components/common/PresetBar.vue`、`NumberField.vue`、`InstrumentBrowser.vue` — 输入框统一。
- `src/components/common/Logo.vue`、`src/components/workspace/TrackHeaderList.vue` — logo 填充形态。
- `src/components/workspace/PanelTabs.vue` — `pick` → `openTab`。
- `src/stores/clipboard.ts`（新）、`src/stores/selection.ts` — 剪贴板与记忆长度。
- `src/views/ArrangeView.vue`、`src/views/EditView.vue` — 复制粘贴、点击创建、默认属性、去提示。
- `src/i18n/zh.ts`、`src/i18n/en.ts` — 文案。
- `src/views/SynthWindowView.vue` — 删除无效的「在主窗口选中这条轨道」按钮（Task 10）。

---

### Task 1: `Md3Select` 增加可选底部动作行

**Files:**
- Modify: `src/components/common/Md3Select.vue`

**Interfaces:**
- Produces: props 新增可选 `action?: { label: string }`；emits 新增 `(e: "action")`。未传 `action` 时行为与现在完全一致。

- [ ] **Step 1: 扩展 props / emits**

`SelectOption` 不动。`defineProps` 加：

```ts
    /** 菜单底部的一个动作按钮（例如「浏览全部音色」）。 */
    action?: { label: string };
```

`defineEmits` 加一支：

```ts
  (e: "action"): void;
```

`pick()` 之后新增：

```ts
function runAction(): void {
  close();
  emit("action");
}
```

- [ ] **Step 2: 渲染动作行**

在菜单模板里，`<template v-for="g in groups">…</template>` 之后（仍是 `.md3-select-menu` 内）加：

```html
        <template v-if="action">
          <span class="menu-sep" aria-hidden="true" />
          <button class="item action" type="button" @click="runAction">
            <span class="item-main"><span class="item-label">{{ action.label }}</span></span>
            <ArrowRightIcon class="item-check" />
          </button>
        </template>
```

并从 `@heroicons/vue/24/outline` 引入 `ArrowRightIcon`。

- [ ] **Step 3: 样式（非 scoped 的菜单块）**

```css
.md3-select-menu .menu-sep {
  height: 1px;
  margin: 4px 6px;
  background-color: rgb(var(--md-outline-variant) / 0.5);
}
.md3-select-menu .item.action .item-label {
  color: rgb(var(--md-primary));
}
```

- [ ] **Step 4: 验证**

Run: `npm run build`
Expected: 成功

- [ ] **Step 5: 提交**

```bash
git add src/components/common/Md3Select.vue
git commit -m "feat(ui): optional footer action in Md3Select"
```

---

### Task 2: 「音色」选择器合并进 `Md3Select`，删除 `InstrumentSelect`

**Files:**
- Modify: `src/components/workspace/InspectorPanel.vue`
- Delete: `src/components/common/InstrumentSelect.vue`

**Interfaces:**
- Consumes: Task 1 的 `action`/`@action`；`Md3Select` 的 `group`。
- Produces: 音色行用 `Md3Select`；`InstrumentBrowser` 仍由「浏览全部」打开。

- [ ] **Step 1: 读现有用法**

`src/components/workspace/InspectorPanel.vue` 里找到 `<InstrumentSelect …>` 的用法与 `instrumentOptions`（或等价）的构造，以及 `InstrumentSelect.vue` 的 props（`modelValue` / `disabled`）与它内置的「无乐器」「分类分组」「浏览全部」三项能力。

- [ ] **Step 2: 换成 `Md3Select`**

- 顶部改 import：去掉 `InstrumentSelect`，引入 `Md3Select` 与 `SelectOption` 类型。
- 选项映射为 `SelectOption[]`：每个音色 `{ id, label: name, hint: note, group: t(category) }`；在最前插入 `{ id: "", label: t("instrument.none"), group: "" }`（保留「无乐器」）。
- 模板替换为：

```html
          <Md3Select
            class="timbre-select"
            :model-value="track.instrument?.plugin_id ?? ''"
            :options="timbreOptions"
            :label="i18n.t('inspector.timbre')"
            :action="{ label: i18n.t('browser.open') }"
            @select="(id: string) => onPickTimbre(id)"
            @action="browserOpen = true"
          />
```

  `onPickTimbre(id)` = 现有的「设置乐器 / 卸载」逻辑（`id === ""` → 卸载）。
- `.timbre-select { flex: 1 1 auto; min-width: 0; }`（同 `.fx-select`）。

- [ ] **Step 3: 确认无残留并删除**

Run: `grep -rn "InstrumentSelect" src/`
Expected: 无输出（若 `InstrumentBrowser` 等仍引用则一并改掉）

Run: `rm src/components/common/InstrumentSelect.vue`

- [ ] **Step 4: 验证**

Run: `npm run build`
Expected: 成功

- [ ] **Step 5: 手动核对**

Run: `npm run dev`：检查器「音色」行能选音色、能选「无乐器」、能点底部「浏览全部」打开浏览器；外观是 34px 药丸。

- [ ] **Step 6: 提交**

```bash
git add src/components/workspace/InspectorPanel.vue src/components/common/InstrumentSelect.vue
git commit -m "feat(ui): merge instrument picker into Md3Select"
```

---

### Task 3: 输入框统一到 `.md3-field`

**Files:**
- Modify: `src/assets/main.css`
- Modify: `src/components/workspace/InspectorPanel.vue`、`src/components/common/PresetBar.vue`、`src/components/common/NumberField.vue`、`src/components/common/InstrumentBrowser.vue`

**Interfaces:**
- Produces: `main.css` 的 `.md3-field`（以及 `.md3-field:focus`），尺寸/形状与 `Md3Select` 触发框一致。

- [ ] **Step 1: 加共享类**

`src/assets/main.css` 的 `@layer components` 里加：

```css
  /* 统一的 MD3 输入/下拉外观：与 Md3Select 的触发框同尺寸同形状。
     五处输入框（轨道名 / 预设名 / 数值 / 搜索）都复用它，防止再次漂移。 */
  .md3-field {
    height: 34px;
    padding: 0 12px;
    border: 1px solid rgb(var(--md-outline) / 0.7);
    border-radius: 999px;
    background-color: rgb(var(--md-surface-container-high));
    color: rgb(var(--md-on-surface));
    font-size: 12px;
    outline: none;
    transition: border-color 0.15s ease, box-shadow 0.15s ease;
  }
  .md3-field::placeholder {
    color: rgb(var(--md-on-surface-muted));
  }
  .md3-field:focus {
    border-color: rgb(var(--md-primary));
    box-shadow: 0 0 0 2px rgb(var(--md-primary) / 0.22);
  }
```

删除已死且带 typo 的 `.input-filled` / `.input-outlined`（并把 `.no-spinner` 一并删掉，若确认无引用）。

- [ ] **Step 2: 四处改用 `.md3-field`**

- `InspectorPanel.vue` `.name-input`：把 `height/border/radius/bg/font-size` 收敛为 `@apply`-less 的 `.md3-field`——即元素加 `class="name-input md3-field"`，并删掉 `.name-input` 里与 `.md3-field` 重复的声明，只保留布局（`flex: 1 1 auto; min-width: 0;`）。
- `PresetBar.vue` `.name-input`：同上（去掉 28px/8px 那套）。
- `NumberField.vue` `.step`：把 `height: 26px` → `34px`，保持药丸；`.step-input`/`.step-btn` 高度随之调整（`20px` → `26px`），或直接给 `.step` 用 `.md3-field` 的尺寸值。
- `InstrumentBrowser.vue` `.search`：`height: 40px` → `34px`；其余不变（已是 999px 药丸）。

- [ ] **Step 3: 显式声明 lavender 强调色**

`main.css` 的 accent 段落里补一组与 `:root` 相同的 `[data-accent="lavender"]`（浅色 + `.dark[data-accent="lavender"]` 深色各一组），并写注释说明「默认色，显式声明以免隐式依赖」。

- [ ] **Step 4: 验证**

Run: `npm run build`（应成功）；`grep -rn "input-filled\|input-outlined" src/`（应无输出）

- [ ] **Step 5: 手动核对**

Run: `npm run dev`：轨道名输入框、预设命名框、BPM/数值步进器、浏览器搜索框——四者高度/圆角/边框/底色一致。

- [ ] **Step 6: 提交**

```bash
git add src/assets/main.css src/components/workspace/InspectorPanel.vue src/components/common/PresetBar.vue src/components/common/NumberField.vue src/components/common/InstrumentBrowser.vue
git commit -m "feat(ui): unify input and select styling on one MD3 field"
```

---

### Task 4: 硬编码文案 i18n 清扫

**Files:**
- Modify: `src/components/workspace/panes/PluginsPane.vue`、`src/components/workspace/TrackHeaderList.vue`、`src/components/workspace/RightPanel.vue`、`src/views/ArrangeView.vue`、`src/views/SynthWindowView.vue`、`src/components/common/PianoKeyboard.vue`、`src/views/SettingsView.vue`
- Modify: `src/i18n/zh.ts`、`src/i18n/en.ts`

**Interfaces:**
- Produces: 上述文件里不再有硬编码的中文界面文案；新键中英各一份。

- [ ] **Step 1: 逐个替换**

对每处硬编码文案引入 `i18n.t(...)`（`PluginsPane`/`TrackHeaderList`/`RightPanel`/`SettingsView` 已有 `i18n`；`PianoKeyboard` 需要引入 `useI18n`）。命名示例：`plugins.loadTo`、`plugins.selectTrackFirst`、`plugins.builtin`、`plugins.pending`、`plugins.loadNamed`、`tracks.newName`（`{n}` 占位）、`tracks.noteCount`（`{n}`）、`right.emptyTitle`、`right.emptyHint`、`synth.windowTitle`、`synth.windowSubtitle`、`synth.focusTrack`、`keyboard.group`、`keyboard.audition`（`{note}`）、`settings.showTrackBar`、`settings.trackBarWidth` 等。

- [ ] **Step 2: 补齐 zh/en**

两边都加。跑 `npm run build`。

- [ ] **Step 3: 校验中英键集一致**

Run: `node -e "const z=require('fs').readFileSync('src/i18n/zh.ts','utf8'),e=require('fs').readFileSync('src/i18n/en.ts','utf8');const k=s=>[...s.matchAll(/^\s*\"([^\"]+)\":/gm)].map(m=>m[1]).sort();const A=k(z),B=k(e);const only=A.filter(x=>!B.includes(x)).concat(B.filter(x=>!A.includes(x)));console.log(only.length?('MISMATCH '+JSON.stringify(only)):'OK '+A.length+' keys')"`
Expected: `OK <n> keys`

- [ ] **Step 4: 提交**

```bash
git add src/i18n/zh.ts src/i18n/en.ts src/components/workspace/panes/PluginsPane.vue src/components/workspace/TrackHeaderList.vue src/components/workspace/RightPanel.vue src/views/ArrangeView.vue src/views/SynthWindowView.vue src/components/common/PianoKeyboard.vue src/views/SettingsView.vue
git commit -m "feat(i18n): translate remaining hardcoded UI copy"
```

---

### Task 5: logo 填充形态 + 默认强调色

**Files:**
- Modify: `src/components/common/Logo.vue`、`src/components/workspace/TrackHeaderList.vue`

**Interfaces:**
- Produces: `Logo` 新增可选 `filled?: boolean`（默认 `false`，保持现状）；`filled` 时渲染薰衣草紫药丸底 + `on-primary` 图形。

- [ ] **Step 1: 给 Logo 加 filled 形态**

```vue
<script setup lang="ts">
/** 应用标志（波形条）；`filled` 时是一枚薰衣草紫的圆角底。 */
withDefaults(defineProps<{ filled?: boolean }>(), { filled: false });
</script>

<template>
  <span class="logo" :class="{ filled }">
    <svg viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
      <rect x="1" y="6" width="3" height="8" rx="1.5" />
      <rect x="6" y="3" width="3" height="14" rx="1.5" />
      <rect x="11" y="4.5" width="3" height="11" rx="1.5" />
      <rect x="16" y="7" width="3" height="6" rx="1.5" />
    </svg>
  </span>
</template>

<style scoped>
.logo { display: inline-flex; color: inherit; }
.logo.filled {
  width: 24px;
  height: 24px;
  align-items: center;
  justify-content: center;
  border-radius: 8px;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}
.logo.filled svg { width: 15px; height: 15px; }
</style>
```

- [ ] **Step 2: 品牌区用它**

`TrackHeaderList.vue` 的品牌区把 `<Logo class="brand-logo" />` 改为 `<Logo filled />`，并把 `.brand-logo` 的固定颜色去掉（颜色由 `filled` 决定）。

- [ ] **Step 3: 验证**

Run: `npm run build`；`npm run dev` 看左上角 logo 是否薰衣草紫底、深色主题下也可读。

- [ ] **Step 4: 提交**

```bash
git add src/components/common/Logo.vue src/components/workspace/TrackHeaderList.vue
git commit -m "feat(ui): lavender filled app logo"
```

---

### Task 6: 「+」选择面板后自动跳转

**Files:**
- Modify: `src/components/workspace/PanelTabs.vue`

**Interfaces:**
- Produces: `pick(id)` 选中后当前标签页切换为该面板。

- [ ] **Step 1: 改 pick**

```ts
function pick(id: PaneId): void {
  // 用户是从列表里「选中」了这个面板——按习惯应当直接切过去。
  layout.openTab(id);
  pickerOpen.value = false;
}
```

（`layout.openTab` 会 `push` 标签、设为 `activeTab` 并展开右侧面板。）

- [ ] **Step 2: 验证**

Run: `npm run build`；`npm run dev`：点「+」选一个未打开的面板 → 新标签出现**且**立刻成为当前页。

- [ ] **Step 3: 提交**

```bash
git add src/components/workspace/PanelTabs.vue
git commit -m "feat(ui): switch to a pane when picked from the add-pane menu"
```

---

### Task 7: 控制器默认「音量」+ 去掉空片段文字提示

**Files:**
- Modify: `src/views/EditView.vue`

**Interfaces:**
- Produces: `controllerProperty` 初值 `"volume"`；钢琴卷帘不再渲染空片段提示。

- [ ] **Step 1: 默认属性**

把

```ts
const controllerProperty = ref<ControllerProperty>("velocity");
```

改为

```ts
const controllerProperty = ref<ControllerProperty>("volume");
```

- [ ] **Step 2: 删掉空片段提示**

删除模板里这一段（及其样式 `.roll-empty`）：

```html
          <p v-if="clip && clip.notes.length === 0" class="roll-empty text-body-md">
            这个片段还没有音符——在网格上拖动即可画出音符
          </p>
```

保留「没有片段」时的 `.roll-blank`（那是另一种情形，仍需要入口）。

Run: `grep -n "roll-empty" src/views/EditView.vue`
Expected: 无输出

- [ ] **Step 3: 验证**

Run: `npm run build`；`npm run dev`：卷帘下方默认选中「音量」；空片段里没有文字。

- [ ] **Step 4: 提交**

```bash
git add src/views/EditView.vue
git commit -m "feat(edit): default controller lane to volume, drop empty-clip hint"
```

---

### Task 8: 复制粘贴（单片段 / 单音符）

**Files:**
- Create: `src/stores/clipboard.ts`
- Modify: `src/views/ArrangeView.vue`、`src/views/EditView.vue`

**Interfaces:**
- Produces: `useClipboardStore()` → `{ payload, copyClip(clip: Clip), copyNote(note: Note), clear() }`；`payload` 为 `{ kind: "clip"; notes: Note[]; length: number } | { kind: "note"; note: Note } | null`。

- [ ] **Step 1: 写 store**

```ts
/**
 * 内存剪贴板：只服务「复制/粘贴音频块与音符」这一件事，不落盘、不进撤销历史。
 * v1 只承载单条内容（单个片段 / 单个音符）。
 */
import { defineStore } from "pinia";
import { ref } from "vue";
import type { Clip, Note } from "@/api/types";

export type ClipboardPayload =
  | { kind: "clip"; notes: Note[]; length: number }
  | { kind: "note"; note: Note };

export const useClipboardStore = defineStore("clipboard", () => {
  const payload = ref<ClipboardPayload | null>(null);

  function copyClip(clip: Clip): void {
    payload.value = { kind: "clip", notes: clip.notes.map((n) => ({ ...n })), length: clip.length };
  }

  function copyNote(note: Note): void {
    payload.value = { kind: "note", note: { ...note } };
  }

  function clear(): void {
    payload.value = null;
  }

  return { payload, copyClip, copyNote, clear };
});
```

- [ ] **Step 2: 编排视图复制粘贴**

在 `ArrangeView.vue`：引入 `useClipboardStore`。在现有 `onKey` 里加分支（并从 `selection.clipRefs` 取选中项）：

```ts
function onKey(e: KeyboardEvent): void {
  if (e.target instanceof HTMLInputElement) return;
  const mod = e.metaKey || e.ctrlKey;
  if (mod && e.key.toLowerCase() === "c") {
    const ref = selection.clipRefs;
    if (ref.length !== 1) return;
    const t = tracks.value.find((x) => x.id === ref[0].track);
    const c = t?.clips.find((x) => x.id === ref[0].clip);
    if (c) clipboard.copyClip(c);
    return;
  }
  if (mod && e.key.toLowerCase() === "v") {
    void pasteClip();
    return;
  }
  if (e.key !== "Delete" && e.key !== "Backspace") return;
  // …原有删除逻辑不变
}

/** 粘贴：副本落在该轨道最后一个片段之后。 */
async function pasteClip(): Promise<void> {
  const p = clipboard.payload;
  if (!p || p.kind !== "clip") return;
  const ref = selection.clipRefs;
  const trackId = ref.length > 0 ? ref[0].track : selection.trackId;
  if (trackId === null) return;
  const t = tracks.value.find((x) => x.id === trackId);
  const start = (t?.clips ?? []).reduce((m, c) => Math.max(m, c.start + c.length), 0);
  try {
    await store.addClip(trackId, start, p.length);
    const track = store.project?.tracks.find((x) => x.id === trackId);
    const created = track?.clips[track.clips.length - 1];
    if (!created) return;
    for (const n of p.notes) await store.addNote(trackId, created.id, { ...n });
    selection.selectClip(trackId, created.id);
  } catch (err) {
    layout.notify(i18n.t("error.addClip", { error: String(err) }));
  }
}
```

（`Ctrl/Cmd+C` 与 `Ctrl/Cmd+V` 要 `e.preventDefault()`，避免浏览器默认行为。）

- [ ] **Step 3: 卷帘复制粘贴**

在 `EditView.vue`：同样引入 `useClipboardStore`，在 `onKey` 加分支——`copy` 取 `selection.noteIndices`（长度为 1 时）对应的 `clip.notes[i]`；`paste` 落在 `max(note.start + note.duration)` 处：

```ts
async function pasteNote(): Promise<void> {
  const p = clipboard.payload;
  const c = clip.value;
  const t = trackId.value;
  if (!p || p.kind !== "note" || !c || t === null) return;
  const start = c.notes.reduce((m, n) => Math.max(m, n.start + n.duration), 0);
  try {
    const index = c.notes.length;
    await store.addNote(t, c.id, { ...p.note, start });
    selection.selectNote(index);
  } catch (err) {
    layout.notify(i18n.t("error.addNote", { error: String(err) }));
  }
}
```

- [ ] **Step 4: 验证**

Run: `npm run build`；`npm run dev`：选中一个片段 → Ctrl/Cmd+C → Ctrl/Cmd+V，同轨道尾部出现等长副本（音符一并带过去）；卷帘里同理。

- [ ] **Step 5: 提交**

```bash
git add src/stores/clipboard.ts src/views/ArrangeView.vue src/views/EditView.vue
git commit -m "feat: copy/paste for a selected clip or note"
```

---

### Task 9: 记忆长度 + 单击空白即创建

**Files:**
- Modify: `src/stores/selection.ts`、`src/views/ArrangeView.vue`、`src/views/EditView.vue`

**Interfaces:**
- Consumes: `timeline.snapBeats(beat)`。
- Produces: `selection.lastClipLength` / `selection.lastNoteLength`（ref，默认 1 / 0.5）+ `rememberClipLength(n)` / `rememberNoteLength(n)`。

- [ ] **Step 1: selection store**

在 `src/stores/selection.ts` 里加：

```ts
  /** 上一次选中的片段长度（拍）：单击空白新建片段时用它。 */
  const lastClipLength = ref(1);
  /** 上一次选中的音符时长（拍）：单击空白新建音符时用它。 */
  const lastNoteLength = ref(0.5);

  function rememberClipLength(n: number): void {
    if (Number.isFinite(n) && n > 0) lastClipLength.value = n;
  }
  function rememberNoteLength(n: number): void {
    if (Number.isFinite(n) && n > 0) lastNoteLength.value = n;
  }
```

并加入 return 导出。

- [ ] **Step 2: 编排视图：记录 + 点击创建**

- 选中片段的地方（`onClipPointerDown` / marquee 结果 / 新建后）调用 `selection.rememberClipLength(clip.length)`。
- `onPointerUp` 的 `create` 分支：当 `length < MIN_CREATE_BEATS`（即原地点击）时，用记忆长度创建：

```ts
  if (current === "create") {
    const clicked = Math.abs(create.to - create.from) < MIN_CREATE_BEATS;
    const start = clicked ? create.from : Math.min(create.from, create.to);
    const length = clicked ? selection.lastClipLength : Math.abs(create.to - create.from);
    try {
      await store.addClip(create.trackId, start, length);
      const t = store.project?.tracks.find((x) => x.id === create.trackId);
      const created = t?.clips[t.clips.length - 1];
      if (created) {
        selection.selectClip(create.trackId, created.id);
        selection.rememberClipLength(created.length);
      }
      layout.notify(i18n.t("arrange.clipAdded"));
    } catch (err) {
      layout.notify(i18n.t("error.addClip", { error: String(err) }));
    }
    return;
  }
```

- [ ] **Step 3: 卷帘：记录 + 点击创建**

- 音符被选中（`onNotePointerDown` / marquee / `selectNote` 后）调用 `selection.rememberNoteLength(note.duration)`。
- `onPointerUp` 的 `create` 分支：`duration` 改为

```ts
    const span = Math.abs(create.to - create.from);
    const duration = span >= MIN_CREATE_BEATS ? span : selection.lastNoteLength;
```

  并在创建后 `selection.rememberNoteLength(duration)`。

- [ ] **Step 4: 验证**

Run: `npm run build`；`npm run dev`：选中一个 2 拍片段 → 单击编排空白 → 立刻得到一个 2 拍的片段；连续点击得到一串等长片段。卷帘同理（音符按上次选中的时长）。

- [ ] **Step 5: 提交**

```bash
git add src/stores/selection.ts src/views/ArrangeView.vue src/views/EditView.vue
git commit -m "feat: click-to-create uses the last selected clip/note length"
```

---

### Task 10: 删除合成器窗口里无效的「在主窗口选中这条轨道」按钮

**Files:**
- Modify: `src/views/SynthWindowView.vue`

**背景（已核实）**：这个按钮是**空操作**。`SynthWindowView.vue:80` 的按钮调用 `selectTrack()`（`:58-60`），后者只改**本窗口自己的** Pinia `selection`（`:21`）。合成器窗口是**独立的 Vue 实例**（`SynthWindowView.vue:5-7` 自己写了这一点），它自己的 store 没有任何渲染依赖，也影响不到主窗口（跨窗口只有后端广播 `engine://event` / `engine://transport`）。既不发命令、也不发事件、也不抢焦点——所以点了什么都不会发生。而且每个能打开它的入口传入的都是主窗口**已经选中**的那条轨道，`SynthWindowView.vue:33-36` 那条「退回第一台内置合成器轨道」的兜底也救不了这个按钮。

**Interfaces:**
- Produces: 该窗口不再渲染按钮，且不留下任何孤立代码。

- [ ] **Step 1: 删按钮**

删除模板里的这一行（及其上方可能存在的容器/分隔元素）：

```html
        <button v-if="track" class="focus-btn" @click="selectTrack">在主窗口选中这条轨道</button>
```

- [ ] **Step 2: 删孤立代码**

删除现在只被它使用的：

```ts
import { useSelectionStore } from "@/stores/selection";
```
```ts
const selection = useSelectionStore();
```
```ts
function selectTrack(): void {
  if (trackId.value !== null) selection.selectTrack(trackId.value);
}
```

以及 `<style scoped>` 里的 `.focus-btn` / `.focus-btn:hover` 规则（`:131-144` 附近）。

Run: `grep -n "selection\|focus-btn\|selectTrack" src/views/SynthWindowView.vue`
Expected: 无输出

- [ ] **Step 3: 修正过时注释**

同文件 `:45` 附近的注释声称「在这个窗口里改参数，也会同步选中主窗口的那条轨道（两边状态一致更不容易懵）」，但 `commit()`（`:46-50`）只调用 `store.setParam`，从不选轨道。把这句改成真实行为（参数改动只写后端工程状态，两个窗口靠 `engine://` 广播保持一致）。

- [ ] **Step 4: 验证**

Run: `npm run build`
Run: `npm run dev`，打开内置合成器窗口：标题/轨道名/面板/键盘都正常，只是不再有那个按钮。

- [ ] **Step 5: 提交**

```bash
git add src/views/SynthWindowView.vue
git commit -m "fix(synth): drop the no-op 'select this track' button from the synth window"
```

---

## Self-Review

**Spec coverage（对照 `2026-10-08-ux-clipboard-and-md3-controls-design.md`）**
- 3.1 控件统一（`Md3Select` 动作行 / 合并 `InstrumentSelect` / `.md3-field`）→ Task 1、2、3。
- 3.2 i18n 清扫 → Task 4。
- 3.3 复制粘贴 → Task 8。
- 3.4 记忆长度点击创建 → Task 9。
- 3.5 去空片段提示 / 控制器默认音量 / 标签自动跳转 / logo 与默认色 → Task 5、6、7。
- 合成器窗口的死按钮（来自 `2026-10-09-audio-device-resilience-design.md` §4.6）→ Task 10。

**Placeholder 扫描**：无 TBD/TODO；代码与类名均为具体值。

**类型一致性**：`ClipboardPayload` / `useClipboardStore` 在 Task 8 定义并使用；`lastClipLength` / `lastNoteLength` / `remember*` 在 Task 9 Step 1 定义、Step 2/3 使用；`Md3Select` 的 `action`/`@action` 在 Task 1 定义、Task 2 使用；`.md3-field` 在 Task 3 定义并在同任务四处使用。

**注意**：Task 8 与 Task 9 都改 `ArrangeView.vue` / `EditView.vue`，按顺序执行即可；Task 9 依赖 Task 8 之后的文件状态。
