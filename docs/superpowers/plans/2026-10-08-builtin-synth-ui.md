# 合成器默认正弦 + 插件界面重设计 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让内置合成器默认发声为正弦，并把它的插件界面重做成固定尺寸、MD3、小白友好的「基础/高级」两层界面，配一个周期示波器与 24 条预设。

**Architecture:** 引擎侧把 `PARAM_TABLE` 的 `wave` 默认值从 0（锯齿）改为 2（正弦）；出厂预设补丁显式钉住各自的波形以保听感。前端把 `SynthPanel.vue` 从「响应式参数卡片墙」重写为**固定设计宽 900×560 的设备面板**（外层按容器宽度等比缩放），基础层只露 8 个参数，高级层弹簧展开；示波器改为**按参数解析生成一个基频周期**（新 `src/utils/waveform.ts` + `Oscilloscope.vue`，删除轮询真实音频的 `WaveformDisplay.vue`）；预设选择器从原生 `<select>` 换成 `Md3Select`，并新增 24 条出厂预设（6 类 × 4）与预设垫子网格。

**Tech Stack:** Rust（`crates/synth`、`crates/engine`）、Tauri 2、Vue 3 `<script setup>` + Pinia + TypeScript、Tailwind + MD3 CSS 变量。

## Global Constraints

- 前端**没有测试框架**：前端任务的验证 = `npm run build`（`vue-tsc --noEmit` + `vite build`）通过 + 手动核对。不要引入 vitest/jest。
- Rust 任务的验证 = `cargo test -p daw-party-synth` / `cargo test -p daw-party-engine --features realtime`。
- 参数表必须与 `crates/synth` 的 `PARAM_TABLE` **逐项一致**（id、范围、默认值）；改动任何默认值必须两侧同步（`crates/synth/src/lib.rs` 与 `src/data/plugins.ts`）。
- 状态序列化顺序（`PARAM_TABLE` 顺序）不可变；老工程 12 字节状态仍须可读。
- 前端**不出现原生 `<select>`**；下拉一律用 `Md3Select`。
- 颜色/圆角一律走 `--md-*` token 与 `rgb(var(--md-*))` 写法；大圆角遵循 MD3（卡片 20–28px）。
- 注释与文案用中文；i18n 文案中英各一份。
- 提交信息格式：`feat(synth): …` / `fix(synth): …`。

---

## File Structure

**Rust**
- `crates/synth/src/lib.rs` — 改 `wave` 默认值；新增默认波形测试。
- `crates/engine/src/registry.rs` — 出厂预设补丁显式钉住 `wave`。

**前端 · 数据与逻辑**
- `src/data/plugins.ts` — `wave` 默认值；24 条出厂预设（带 `category`）。
- `src/i18n/zh.ts` / `src/i18n/en.ts` — 预设文案 + 类别文案 + 小白化参数名。
- `src/utils/waveform.ts`（新）— 一个周期波形的纯函数。
- `src/stores/presets.ts` — `PresetOption` 增加 `category`。

**前端 · 组件**
- `src/components/common/Md3Select.vue` — 支持可选分组。
- `src/components/common/PresetBar.vue` — 改用 `Md3Select`。
- `src/components/common/Oscilloscope.vue`（新）— 画一个周期。
- `src/components/common/WaveformDisplay.vue` — 删除。
- `src/components/workspace/panes/SynthPanel.vue` — 重写。
- `src/views/SynthWindowView.vue` — 固定尺寸布局。

**Tauri**
- `src-tauri/src/commands.rs` — `open_synth_window` 固定尺寸 + 禁缩放。

---

### Task 1: 引擎默认波形改为正弦

**Files:**
- Modify: `crates/synth/src/lib.rs`（`PARAM_TABLE` 第 4 行、`default_params()`、`mod tests`）

**Interfaces:**
- Produces: `SynthPlugin::new().get_param("wave") == Some(2.0)`（2 = Sine）。

- [ ] **Step 1: 写失败测试**

在 `crates/synth/src/lib.rs` 的 `mod tests` 末尾加：

```rust
    /// 默认音色必须是正弦（新手一上手就是一个干净的基础音）。
    #[test]
    fn default_wave_is_sine() {
        let s = SynthPlugin::new();
        assert_eq!(s.get_param(&ParamId::from("wave")), Some(2.0));
    }
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p daw-party-synth default_wave_is_sine`
Expected: FAIL（`assertion failed: Some(0.0) == Some(2.0)`）

- [ ] **Step 3: 改默认值**

`PARAM_TABLE` 里把 `("wave", "Wave", 0.0, 3.0, 0.0)` 改为：

```rust
    ("wave", "Wave", 0.0, 3.0, 2.0),
```

`default_params()` 里把 `wave: 0.0,` 改为：

```rust
        wave: 2.0,
```

- [ ] **Step 4: 运行全套 synth 测试**

Run: `cargo test -p daw-party-synth`
Expected: PASS（含 `waveforms_differ`、`renders_440hz_within_tolerance` 等；后者本就显式设 `wave=2.0`，不受影响）

- [ ] **Step 5: 提交**

```bash
git add crates/synth/src/lib.rs
git commit -m "fix(synth): default oscillator waveform is sine"
```

---

### Task 2: 出厂预设钉住各自波形（保听感）

**Files:**
- Modify: `crates/engine/src/registry.rs`（`BUILTIN_PRESETS`）

**Interfaces:**
- Consumes: Task 1 的默认正弦。
- Produces: `builtin.synth` 仍为默认（正弦）；`lead`/`bass`/`pad` 补丁含 `("wave", 0.0)`；`pluck` 含 `("wave", 1.0)`。

- [ ] **Step 1: 写失败测试**

在 `crates/engine/src/registry.rs` 的 `mod tests` 里加（若尚无 `mod tests` 则新建，`use super::*;` + `use daw_party_plugin_api::ParamId;`）：

```rust
    /// 默认改成正弦后，老预设必须仍保持原来的波形，否则听感会变。
    #[test]
    fn factory_presets_pin_their_waveform() {
        let reg = PluginRegistry::with_builtins();
        let wave_of = |id: &str| -> Option<f32> {
            let p = reg.create(id, crate::model::TrackId(0))?;
            p.get_param(&ParamId::from("wave"))
        };
        assert_eq!(wave_of("builtin.synth"), Some(2.0), "默认音色 = 正弦");
        assert_eq!(wave_of("builtin.synth.lead"), Some(0.0));
        assert_eq!(wave_of("builtin.synth.bass"), Some(0.0));
        assert_eq!(wave_of("builtin.synth.pad"), Some(0.0));
        assert_eq!(wave_of("builtin.synth.pluck"), Some(1.0));
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p daw-party-engine factory_presets_pin_their_waveform`
Expected: FAIL（lead/bass/pad 现在是 `Some(2.0)`）

- [ ] **Step 3: 在补丁里钉住波形**

编辑 `BUILTIN_PRESETS`：给 `builtin.synth.lead`、`builtin.synth.bass`、`builtin.synth.pad` 三条的 `patch` 开头各加 `("wave",0.0),`；`builtin.synth.pluck` 已含 `("wave",1.0)`，不动。例如 lead 变为：

```rust
    Preset { id: "builtin.synth.lead", name: "主音 Lead", patch: &[("gain",0.75),("wave",0.0),("attack",0.005),("release",0.25),("cutoff",0.80),("resonance",0.22),("filterEnv",0.45),("decay",0.35),("sustain",0.70),("lfoRate",0.80),("lfoDelay",0.35),("lfoPitch",0.10)] },
```

（bass、pad 同样在 `patch` 里加 `("wave",0.0),`。）

- [ ] **Step 4: 运行测试**

Run: `cargo test -p daw-party-engine --features realtime`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add crates/engine/src/registry.rs
git commit -m "fix(engine): pin factory preset waveforms after sine default"
```

---

### Task 3: 前端参数默认与出厂预设数据同步

**Files:**
- Modify: `src/data/plugins.ts`（`SYNTH_PARAMS`、`SYNTH_FACTORY_PRESETS`）

**Interfaces:**
- Produces: `SYNTH_PARAMS` 中 `wave.default === 2`；`SYNTH_FACTORY_PRESETS` 的 lead/bass/pad 含 `wave: 0`。

- [ ] **Step 1: 改 `SYNTH_PARAMS` 的 wave 默认**

`src/data/plugins.ts` 中把 `{ id: "wave", name: "波形", min: 0, max: 3, step: 1, default: 0, digits: 0 },` 改为 `default: 2`。

- [ ] **Step 2: 给出厂预设补 `wave`**

在 `SYNTH_FACTORY_PRESETS` 的 lead / bass / pad 三条 `params` 各加 `wave: 0,`（与 Task 2 的引擎补丁一致）。pluck 已含 `wave: 1`。

- [ ] **Step 3: 类型检查 + 构建**

Run: `npm run build`
Expected: 成功（无 TS 错误）

- [ ] **Step 4: 提交**

```bash
git add src/data/plugins.ts
git commit -m "feat(synth): mirror sine default and pinned preset waveforms"
```

---

### Task 4: `Md3Select` 支持可选分组

**Files:**
- Modify: `src/components/common/Md3Select.vue`

**Interfaces:**
- Produces: `SelectOption` 增加可选 `group?: string`；选项带 `group` 时，菜单按出现顺序渲染分组标题（标题为 `group` 原文，无 `group` 的选项归入无标题区块）。

- [ ] **Step 1: 扩展类型与渲染**

`SelectOption` 接口增加字段：

```ts
export interface SelectOption {
  id: string;
  label: string;
  /** 第二行小字（可选）。 */
  hint?: string;
  /** 分组标题（可选）：同 group 的选项渲染在同一个标题下。 */
  group?: string;
}
```

在 `<script setup>` 增加一个计算属性，把扁平的 `options` 折叠为「标题 + 选项」的区块序列（保持原顺序，空 group 归到 `""` 区块）：

```ts
/** 选项按 group 折叠成区块；keepOrder 保证与传入顺序一致。 */
const groups = computed<{ title: string; items: SelectOption[] }[]>(() => {
  const out: { title: string; items: SelectOption[] }[] = [];
  const index = new Map<string, number>();
  for (const o of props.options) {
    const key = o.group ?? "";
    let at = index.get(key);
    if (at === undefined) {
      at = out.length;
      index.set(key, at);
      out.push({ title: key, items: [] });
    }
    out[at]!.items.push(o);
  }
  return out;
});
```

把模板里的候选列表从 `v-for="o in options"` 改为按区块渲染（保留原有 `.item` 按钮与选中态），标题在 `title !== ""` 时渲染：

```html
        <p v-if="options.length === 0" class="empty">{{ emptyText }}</p>
        <template v-for="g in groups" :key="g.title">
          <p v-if="g.title" class="group-title">{{ g.title }}</p>
          <button
            v-for="o in g.items"
            :key="o.id"
            class="item"
            type="button"
            role="option"
            :aria-selected="o.id === modelValue"
            @click="pick(o.id)"
          >
            <span class="item-main">
              <span class="item-label">{{ o.label }}</span>
              <span v-if="o.hint" class="item-hint">{{ o.hint }}</span>
            </span>
            <CheckIcon v-if="o.id === modelValue" class="item-check" />
          </button>
        </template>
```

在非 scoped 的菜单样式块里加标题样式：

```css
.md3-select-menu .group-title {
  margin: 4px 0 2px;
  padding: 2px 10px;
  font-size: 10px;
  letter-spacing: 0.6px;
  color: rgb(var(--md-on-surface-muted));
}
```

- [ ] **Step 2: 类型检查 + 构建**

Run: `npm run build`
Expected: 成功

- [ ] **Step 3: 提交**

```bash
git add src/components/common/Md3Select.vue
git commit -m "feat(ui): support grouped options in Md3Select"
```

---

### Task 5: `PresetBar` 去掉原生 `<select>`

**Files:**
- Modify: `src/components/common/PresetBar.vue`

**Interfaces:**
- Consumes: Task 4 的 `Md3Select`（含 `group`）、`usePresetsStore().forPlugin()`。
- Produces: 无原生 `<select>` 的预设选择器；行为与现在一致（`apply` / 另存为 / 删除）。

- [ ] **Step 1: 替换选择器**

在 `<script setup>` 引入：`import Md3Select from "@/components/common/Md3Select.vue";`，并新增选项映射（把预设选项映射成 `Md3Select` 的 `SelectOption`，用平台/自存分组）：

```ts
const selectOptions = computed(() =>
  options.value.map((p) => ({
    id: p.id,
    label: p.name,
    hint: p.note || undefined,
    group: p.factory ? t("preset.factoryGroup") : t("preset.userGroup"),
  })),
);
const selectedId = computed(() => active.value?.id ?? "");
function onSelect(id: string): void {
  pick(id);
}
```

把模板里的 `<select class="select" …>…</select>` 整段替换为：

```html
    <Md3Select
      class="preset-select"
      :model-value="selectedId"
      :options="selectOptions"
      :label="t('preset.choose')"
      :placeholder="t('preset.unsaved')"
      @select="onSelect"
    />
```

（`Md3Select` 宽度是 `100%`，用 `.preset-select { flex: 1 1 auto; min-width: 0; }` 让它填满预设条。）

删除 `.select { … }` 样式块（已无用）。

- [ ] **Step 2: 确认没有原生 select 残留**

Run: `grep -rn "<select" src/`
Expected: 无输出

- [ ] **Step 3: 类型检查 + 构建**

Run: `npm run build`
Expected: 成功

- [ ] **Step 4: 提交**

```bash
git add src/components/common/PresetBar.vue
git commit -m "feat(synth): use MD3 dropdown for preset selection"
```

---

### Task 6: 扩到 24 条出厂预设 + 文案

**Files:**
- Modify: `src/data/plugins.ts`（`FactoryPreset` 增 `category`；`SYNTH_FACTORY_PRESETS` 扩到 24）
- Modify: `src/i18n/zh.ts`、`src/i18n/en.ts`
- Modify: `src/stores/presets.ts`（`PresetOption.category`）

**Interfaces:**
- Consumes: `fullSynthParams`（已存在）。
- Produces: `FactoryPreset { id; name; note; category; params }`，`category ∈ {"bass","lead","keys","pad","pluck","fx"}`；`PresetOption.category?: string`。

- [ ] **Step 1: `FactoryPreset` 加 `category`**

```ts
export interface FactoryPreset {
  id: string;
  name: string;
  note: string;
  /** 类别 slug：bass | lead | keys | pad | pluck | fx（界面映射为 i18n 键 presetCategory.<slug>）。 */
  category: string;
  params: Record<string, number>;
}
```

- [ ] **Step 2: 写 24 条预设**

保留现有 4 条并把 `category` 补上（lead→`lead`、pluck→`pluck`、bass→`bass`、pad→`pad`），再补 20 条。**每条的 `id` 用 `builtin.synth.<slug>`，`name`/`note` 用 i18n 键 `preset.<slug>.name` / `preset.<slug>.note`**。各条 `params`（未列的参数取 `SYNTH_DEFAULTS`；`wave` 已按 Task 3 显式给出处置）：

| slug | category | wave | 其余参数（覆盖默认） |
| --- | --- | --- | --- |
| lead | lead | 0 | gain .75, attack .005, release .25, cutoff .80, resonance .22, filterEnv .45, decay .35, sustain .70, lfoRate .80, lfoDelay .35, lfoPitch .10 |
| lead.soft | lead | 2 | gain .70, attack .02, release .30, cutoff .70, resonance .10, decay .40, sustain .75 |
| lead.square | lead | 1 | gain .72, attack .004, release .20, cutoff .65, resonance .25, filterEnv .40, decay .30, sustain .70 |
| lead.sync | lead | 3 | gain .70, attack .003, release .18, cutoff .85, resonance .35, filterEnv .50, decay .25, sustain .65, lfoRate .85, lfoPitch .15 |
| pluck | pluck | 1 | gain .80, attack .001, release .08, cutoff .55, filterEnv .70, decay .12, sustain 0 |
| pluck.soft | pluck | 3 | gain .75, attack .002, release .20, cutoff .50, resonance .10, filterEnv .50, decay .20, sustain .10 |
| pluck.bell | pluck | 2 | gain .72, attack .001, release .35, cutoff .85, resonance .25, filterEnv .60, decay .45, sustain 0 |
| pluck.mute | pluck | 0 | gain .78, attack .001, release .06, cutoff .40, resonance .20, filterEnv .55, decay .10, sustain 0 |
| bass | bass | 0 | gain .85, attack .002, release .12, sub .60, cutoff .35, resonance .25, filterEnv .50, decay .25, sustain .50 |
| bass.sub | bass | 0 | gain .80, attack .004, release .20, sub .90, cutoff .28, decay .35, sustain .60 |
| bass.round | bass | 2 | gain .80, attack .006, release .18, sub .40, cutoff .30, resonance .10, decay .30, sustain .55 |
| bass.pick | bass | 1 | gain .80, attack .001, release .08, cutoff .55, filterEnv .70, decay .12, sustain .10 |
| pad | pad | 0 | gain .55, attack .35, release 1.20, cutoff .50, filterEnv .30, decay 1.00, sustain .80, lfoRate .60, lfoFilter .15 |
| pad.strings | pad | 0 | gain .60, attack .25, release 1.00, sub .20, cutoff .55, resonance .15, filterEnv .25, decay .90, sustain .85, lfoRate .50, lfoFilter .10 |
| pad.glass | pad | 2 | gain .50, attack .50, release 1.50, cutoff .70, resonance .18, filterEnv .20, decay 1.20, sustain .80, lfoRate .40, lfoPitch .05 |
| pad.choir | pad | 3 | gain .52, attack .40, release 1.40, sub .15, cutoff .45, filterEnv .35, decay 1.10, sustain .85, lfoRate .55, lfoFilter .20 |
| keys.ep | keys | 2 | gain .75, attack .008, release .45, cutoff .60, resonance .12, filterEnv .35, decay .50, sustain .50 |
| keys.bell | keys | 2 | gain .70, attack .001, release .80, cutoff .90, resonance .20, filterEnv .60, decay .90, sustain .20 |
| keys.organ | keys | 1 | gain .72, attack .003, release .12, sub .30, cutoff .75, sustain .95 |
| keys.pluck | keys | 3 | gain .78, attack .001, release .25, cutoff .55, filterEnv .65, decay .30, sustain .15 |
| fx.riser | fx | 0 | gain .60, attack .60, release .80, cutoff .40, resonance .40, filterEnv .80, decay 1.50, sustain .90, lfoRate .90, lfoFilter .35, lfoPitch .20 |
| fx.wobble | fx | 1 | gain .65, attack .01, release .30, sub .40, pwm .50, cutoff .35, resonance .50, filterEnv .50, decay .60, sustain .70, lfoRate .35, lfoFilter .80 |
| fx.drone | fx | 0 | gain .50, attack .80, release 2.00, sub .70, cutoff .35, resonance .30, decay 2.00, sustain .95, lfoRate .25, lfoFilter .40 |
| fx.blip | fx | 1 | gain .70, attack .001, release .05, pwm .80, cutoff .80, resonance .50, filterEnv .80, decay .06, sustain 0 |

> 注意：现有 4 条的 slug 已经是 `lead`/`pluck`/`bass`/`pad`，直接复用它们的键名与 id（`builtin.synth.lead` 等），只补 `category`。

- [ ] **Step 3: 补 i18n 文案**

`src/i18n/zh.ts` 与 `src/i18n/en.ts` 各加：6 条类别 `presetCategory.bass|lead|keys|pad|pluck|fx`、2 条分组标题 `preset.factoryGroup` / `preset.userGroup`、以及上表 24 组 `preset.<slug>.name` / `preset.<slug>.note`（中文诗意短名 + 一句话说明；英文同理）。已存在的中文键保留。

- [ ] **Step 4: `PresetOption` 带类别**

`src/stores/presets.ts` 的 `PresetOption` 增加 `category?: string`；`forPlugin` 里出厂映射时带上 `category: p.category`。用户自存预设 `category` 留空（归入「我的预设」分组）。

- [ ] **Step 5: 类型检查 + 构建**

Run: `npm run build`
Expected: 成功

- [ ] **Step 6: 提交**

```bash
git add src/data/plugins.ts src/i18n/zh.ts src/i18n/en.ts src/stores/presets.ts
git commit -m "feat(synth): 24 factory presets with categories and copy"
```

---

### Task 7: 一个周期示波器（纯函数 + 组件）

**Files:**
- Create: `src/utils/waveform.ts`
- Create: `src/components/common/Oscilloscope.vue`
- Delete: `src/components/common/WaveformDisplay.vue`

**Interfaces:**
- Produces: `sampleOnePeriod(params, points?): number[]`（长度 `points`、值域约 `[-1,1]`）；`<Oscilloscope :params :height />`。
- Note: `WaveformDisplay.vue` 仅被 `SynthPanel.vue` 使用（Task 8 会替换引用），删除前先确认无其它引用。

- [ ] **Step 1: 写纯函数**

`src/utils/waveform.ts`：

```ts
/** 画一个周期所需的参数子集（从合成器参数表里取）。 */
export interface WaveParams {
  wave: number;
  sub: number;
  pwm: number;
  cutoff: number;
  resonance: number;
}

function waveSample(wave: number, phase: number, pulseWidth: number): number {
  const p = phase - Math.floor(phase);
  switch (Math.round(wave)) {
    case 1: return p < pulseWidth ? 1 : -1;
    case 2: return Math.sin(p * Math.PI * 2);
    case 3: return 4 * Math.abs(p - 0.5) - 1;
    default: return 2 * p - 1;
  }
}

/**
 * 一个基频周期的波形采样（解析生成，不是真实音频）。
 *
 * 跟随真实 DSP 的形状：主振 + 低八度方波副振 + 由「亮度/尖锐度」推导的一阶低通。
 * 于是转这些旋钮时屏幕上的波形会真实地变形——这正是给小白看的教学意图。
 */
export function sampleOnePeriod(params: WaveParams, points = 256): number[] {
  const n = Math.max(16, Math.floor(points));
  const pwm = 0.5 + (params.pwm - 0.5) * 0.9;
  const raw: number[] = new Array(n);
  for (let i = 0; i < n; i += 1) {
    const phase = i / n;
    let s = waveSample(params.wave, phase, pwm);
    if (params.sub > 0) s += params.sub * waveSample(1, phase * 0.5, 0.5);
    raw[i] = s;
  }
  // 「亮度」：截止越低越平滑；「尖锐度」让峰更尖（视觉上的共振）。
  const k = Math.min(1, Math.max(0.02, 0.03 + params.cutoff * 0.97));
  let prev = raw[0]!;
  for (let i = 0; i < n; i += 1) {
    prev += k * (raw[i]! - prev);
    raw[i] = prev;
  }
  let peak = 1e-6;
  for (const v of raw) peak = Math.max(peak, Math.abs(v));
  const gain = 0.92 + 0.06 * (1 - params.resonance);
  return raw.map((v) => (v / peak) * gain);
}
```

- [ ] **Step 2: 写组件**

`src/components/common/Oscilloscope.vue`：

```vue
<script setup lang="ts">
/**
 * 一个周期的波形屏（教学用）。
 * 数据是按参数解析算出来的一个基频周期，不是真实音频——转旋钮能看到波形本身的变化。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { sampleOnePeriod, type WaveParams } from "@/utils/waveform";

const props = withDefaults(
  defineProps<{ params: WaveParams; points?: number; height?: number }>(),
  { points: 256, height: 150 },
);

const canvasEl = ref<HTMLCanvasElement | null>(null);

function themeColors(el: HTMLElement): { line: string; grid: string } {
  const styles = getComputedStyle(el);
  const read = (name: string, fallback: string) => {
    const v = styles.getPropertyValue(name).trim();
    return v ? `rgb(${v})` : fallback;
  };
  return { line: read("--md-primary", "#6750a4"), grid: read("--md-outline-variant", "#cac4d0") };
}

function draw(): void {
  const el = canvasEl.value;
  if (!el) return;
  const ctx = el.getContext("2d");
  if (!ctx) return;
  const dpr = window.devicePixelRatio || 1;
  const w = el.clientWidth;
  const h = el.clientHeight;
  if (el.width !== Math.round(w * dpr) || el.height !== Math.round(h * dpr)) {
    el.width = Math.round(w * dpr);
    el.height = Math.round(h * dpr);
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  const { line, grid } = themeColors(el);

  ctx.strokeStyle = grid;
  ctx.globalAlpha = 0.5;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, h / 2);
  ctx.lineTo(w, h / 2);
  ctx.stroke();
  ctx.globalAlpha = 1;

  const data = sampleOnePeriod(props.params, props.points);
  const mid = h / 2;
  const scale = h / 2 - 3;
  ctx.strokeStyle = line;
  ctx.lineWidth = 1.8;
  ctx.lineJoin = "round";
  ctx.beginPath();
  for (let i = 0; i < data.length; i += 1) {
    const x = (i / (data.length - 1)) * w;
    const y = mid - Math.max(-1, Math.min(1, data[i]!)) * scale;
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.stroke();
}

let raf = 0;
function loop(): void {
  draw();
  raf = requestAnimationFrame(loop);
}
onMounted(() => {
  raf = requestAnimationFrame(loop);
});
onUnmounted(() => cancelAnimationFrame(raf));
watch(() => JSON.stringify(props.params), draw);
</script>

<template>
  <div class="scope" :style="{ height: `${height}px` }">
    <canvas ref="canvasEl" aria-label="waveform (one period)" />
  </div>
</template>

<style scoped>
.scope {
  position: relative;
  width: 100%;
  border-radius: 20px;
  background-color: rgb(var(--md-scrim) / 0.55);
  border: 1px solid rgb(var(--md-outline-variant) / 0.45);
  overflow: hidden;
}
canvas {
  display: block;
  width: 100%;
  height: 100%;
}
</style>
```

- [ ] **Step 3: 暂不删除旧组件**

`src/components/common/WaveformDisplay.vue` 仍被 `SynthPanel.vue` 引用，**留到 Task 8**（替换引用之后）再删——否则本任务提交后仓库通不过构建。

- [ ] **Step 4: 类型检查 + 构建**

Run: `npm run build`
Expected: 成功（`Oscilloscope.vue` 与 `waveform.ts` 已就位，但尚未被引用）

- [ ] **Step 5: 提交**

```bash
git add src/utils/waveform.ts src/components/common/Oscilloscope.vue
git commit -m "feat(synth): one-period analytic oscilloscope"
```

---

### Task 8: 重写 `SynthPanel`（固定尺寸设备 + 基础/高级 + 预设垫子）

**Files:**
- Modify: `src/components/workspace/panes/SynthPanel.vue`（整文件重写）

**Interfaces:**
- Consumes: `Oscilloscope.vue`、`Md3Knob.vue`、`PianoKeyboard.vue`、`PresetBar.vue`、`usePresetsStore().forPlugin()`、`SYNTH_PARAMS`。
- Produces: `SynthPanel` props `{ params: Record<string, number>; track?: number }`、emit `(e:"commit", id, value)`（**与现在一致**，调用方 `SynthWindowView` / `PluginEditorPane` 无需改）。设计尺寸常量 `DEVICE_W = 900`、`DEVICE_H = 560`。

- [ ] **Step 1: 重写组件**

结构要点（完整实现按此组织）：

- `<script setup>`：保留 `SPECS` / `drafts` / `shown(id)` / `onLive` / `onCommit` / `display(id,v)`（从现有文件迁移，逻辑不变）；新增：
  - `advanced = ref(false)`；
  - `BASIC_IDS = ["wave","cutoff","resonance","attack","decay","sustain","release","gain"]`（8 个），`ADVANCED_GROUPS`（DCO: sub/pwm/glide；HPF: hpf；VCF: filterEnv/keyFollow；LFO: lfoRate/lfoDelay/lfoPitch/lfoFilter）；
  - `scopeParams = computed<WaveParams>(() => ({ wave: shown("wave"), sub: shown("sub"), pwm: shown("pwm"), cutoff: shown("cutoff"), resonance: shown("resonance") }))`；
  - `ADSR` 包络点的现有 `envelope` computed 迁移过来（保留）；
  - 预设垫子：`const presets = usePresetsStore(); const coarse = ref<string>("lead");`，`import { SYNTH_ID } from "@/data/plugins";`，`const options = computed(() => presets.forPlugin(SYNTH_ID))`；`coarseOptions` = 去重后的 `category` 列表，渲染成 MD3 filter chip；`pads = computed(() => options.value.filter(o => (o.category ?? 'lead') === coarse.value))`；点垫子 emit `applyPreset`（见下）。
- `<template>`：`.fit > .device`：
  - 顶部：预设垫子网格 + 类别 chips + `<PresetBar>`。
  - 中部：左侧 `<Oscilloscope :params="scopeParams" :height="170" />` + ADSR 小曲线；右侧基础旋钮（wave 用 MD3 分段按钮 + 其余 `Md3Knob`），底部一个「高级」`btn-tonal` 切换 `advanced`。
  - 高级区：`v-if="advanced"`，按 `ADVANCED_GROUPS` 渲染分区卡片（复用 `Md3Knob`）。
  - 底部：`<PianoKeyboard v-if="props.track !== undefined" :track="props.track" />`（固定高度 108px，不参与 flex 压缩）。
- `<style scoped>`：`.fit { width: 100%; display: flex; justify-content: center; }`；`.device { width: 900px; height: 560px; display: flex; flex-direction: column; gap: 12px; }`；卡片圆角 20–28px；`.device` 内所有纵向区域用固定高度/`flex: 0 0 auto`，键盘行 `flex: 0 0 auto`。

- [ ] **Step 2: `SynthWindowView` 接上 `applyPreset`**

在 `src/views/SynthWindowView.vue` 的 `<SynthPanel …>` 上加 `@apply-preset="applyPreset"`（该组件的 `applyPreset` 已存在，签名 `(values: Record<string, number>) => Promise<void>`）。

- [ ] **Step 3: `PluginEditorPane` 接上 `applyPreset`**

在 `src/components/workspace/panes/PluginEditorPane.vue` 的 `<SynthPanel …>` 上加 `@apply-preset="applyPreset"`（该组件的 `applyPreset` 已存在）。

- [ ] **Step 4: 类型检查 + 构建**

Run: `npm run build`
Expected: 成功

- [ ] **Step 5: 手动核对（浏览器预览）**

Run: `npm run dev`，打开某轨道的内置合成器（浏览器预览会走侧边栏 `PluginEditorPane`）。
Expected: 默认显示基础 8 个控件 + 一个周期正弦；点「高级」展开高级区；拖波形/脉宽/副振/亮度/尖锐度，示波器波形跟着变；顶部预设垫子点击后整组参数写回；无原生 select。核对完停止 dev server。

- [ ] **Step 6: 删除旧组件**

确认 `WaveformDisplay` 已无任何引用后：

Run: `grep -rn "WaveformDisplay" src/`
Expected: 无输出

Run: `rm src/components/common/WaveformDisplay.vue`

- [ ] **Step 7: 提交**

```bash
git add src/components/workspace/panes/SynthPanel.vue src/views/SynthWindowView.vue src/components/workspace/panes/PluginEditorPane.vue src/components/common/WaveformDisplay.vue
git commit -m "feat(synth): fixed-size MD3 synth face with basic/advanced layers"
```

---

### Task 9: 窄容器等比缩放 + 窗口固定尺寸

**Files:**
- Modify: `src/components/workspace/panes/SynthPanel.vue`（`.fit` 加缩放逻辑）
- Modify: `src/views/SynthWindowView.vue`（固定尺寸布局）
- Modify: `src-tauri/src/commands.rs`（`open_synth_window`）

**Interfaces:**
- Consumes: Task 8 的 `DEVICE_W` / `DEVICE_H`。
- Produces: 面板在容器 < 900px 时整体等比缩小（`transform: scale`），键盘比例不变形。

- [ ] **Step 1: 面板加 scale-to-fit**

在 `SynthPanel.vue` 里加：

```ts
const fitEl = ref<HTMLElement | null>(null);
const scale = ref(1);
let ro: ResizeObserver | null = null;

function measure(): void {
  const el = fitEl.value;
  if (!el) return;
  scale.value = Math.min(1, el.clientWidth / 900);
}
onMounted(() => {
  measure();
  ro = new ResizeObserver(measure);
  if (fitEl.value) ro.observe(fitEl.value);
});
onUnmounted(() => ro?.disconnect());
```

模板：

```html
<div ref="fitEl" class="fit" :style="{ height: `${560 * scale}px` }">
  <div class="device" :style="{ transform: `scale(${scale})`, transformOrigin: 'top center' }">
    …
  </div>
</div>
```

（`.fit { width: 100%; display: flex; justify-content: center; overflow: hidden; }`）

- [ ] **Step 2: `SynthWindowView` 固定尺寸**

把 `.body` 的 `overflow-y: auto` 去掉（不再需要滚动），`.synth-window` 保持 `height: 100vh; overflow: hidden`；`.body` 改为 `display: flex; justify-content: center; padding: 10px 18px 18px;`。

- [ ] **Step 3: 窗口固定尺寸 + 禁缩放**

`src-tauri/src/commands.rs` 的 `open_synth_window` 里，把

```rust
        .inner_size(1060.0, 720.0)
        .min_inner_size(560.0, 460.0)
```

改为

```rust
        .inner_size(960.0, 700.0)
        .resizable(false)
```

- [ ] **Step 4: 构建（Rust + 前端）**

Run: `cargo build --manifest-path src-tauri/Cargo.toml && npm run build`
Expected: 均成功（`src-tauri` 是独立 workspace，必须用 manifest-path，或 `cd src-tauri && cargo build`）

- [ ] **Step 5: 手动核对**

Run: `npm run tauri dev`；打开内置合成器窗口：窗口不可缩放，键盘位置稳定；在侧边栏窄容器里面板整体缩小、键盘比例不变。

- [ ] **Step 6: 提交**

```bash
git add src/components/workspace/panes/SynthPanel.vue src/views/SynthWindowView.vue src-tauri/src/commands.rs
git commit -m "feat(synth): fixed-size window with scale-to-fit fallback"
```

---

## Self-Review

**Spec coverage（对照 `2026-10-08-builtin-synth-ui-redesign-design.md`）**
- 4.1 默认正弦 → Task 1、2、3。
- 4.2 固定尺寸 + 基础/高级 → Task 8、9。
- 4.3 一个周期示波器 → Task 7。
- 4.4 大量预设 + MD3 下拉 → Task 4、5、6。
- 4.5 编排留白竖线 / 4.6 网格每拍一格 / 底栏读数 → **不在本计划**，属计划 B。
- 删除 `WaveformDisplay.vue` → Task 8 Step 6（Task 7 只新增示波器组件，引用切换与删除都在 Task 8，保证每个任务提交时都能构建）。

**Placeholder 扫描**：无 TBD/TODO；数据均为具体值。

**类型一致性**：`WaveParams` 在 `waveform.ts` 定义、由 `SynthPanel` 的 `scopeParams` 与 `Oscilloscope` 的 props 共用；`SynthPanel` 的 emit 在 Task 8 增加 `applyPreset`，调用方在 Task 8 Step 2/3 同步接上；`FactoryPreset.category`（Task 6）与 `PresetOption.category`（Task 6 Step 4）命名一致。

---

## 执行后的遗留（follow-ups）

Plan A 已完成全部 9 个任务，并通过最终整支复审（复审结论「with fixes」→ 一轮修复后复审干净）。以下为**非阻塞**的保留项，留作后续跟进：

- `Oscilloscope.vue` 的 `.scope` 仍是硬编码 20px 圆角（不在 MD3 shape 刻度上）。
- `SynthPanel.vue` 基础/高级两段旋钮的 `Md3Knob` 属性块重复（可抽薄封装，非必须）。
- `synth.cutoff` / `synth.resonance` / `synth.envAmount`（及 `synth.vca` / `synth.amp` / `synth.legacyNote`）已成孤立 i18n 键。
- 深色主题下 `.group` 以 `--md-surface-container-low` 取代 `--app-inset`，比原先略深（细微）。
- 窄侧边栏（scale≈0.31）下预设下拉菜单最小宽度 200px，比触发器宽（仅视觉）。
- 前端 ↔ 引擎参数表同步仍靠人工（无前端测试框架）；Rust 侧目前只断言了 5 个 `wave` 值。
- **需人工肉眼确认**：窗口运行时确实不可缩放；侧边栏 320px 下面板整体缩到约 0.31（字很小）；窗口高度 < ~740px 时键盘会被裁掉且无滚动条。
