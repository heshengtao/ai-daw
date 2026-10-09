# 交互与控件统一（复制粘贴 · 记忆长度 · MD3 控件归一 · 配色 · 标签跳转）— 设计文档

- 日期：2026-10-08
- 状态：待评审
- 主题：音频块/音符的复制粘贴与「记忆长度」点击创建；控件样式统一到一套 MD3；logo 与默认配色；标签页自动跳转；控制器泳道默认属性；去掉空片段文字提示

## 1. 背景与目标

一系列面向「新手快速出活儿」的交互与一致性诉求：

1. **音频块 / 音符支持快捷键复制粘贴**，粘贴出来的副本落在**尾部**（同一轨道最后一个片段之后 / 同一片段最后一个音符之后）。
2. **单击空白即创建**：点一下空白就得到一个音频块 / 音符，长度**等于上一次选中的片段 / 音符**，方便「啪啪点出一堆同样长度」。
3. **片段内没有音符时不要文字提示**（钢琴卷帘里的空片段提示去掉）。
4. **控制器泳道默认选中「音量」**（当前默认是力度）。
5. **控件风格统一**：以「效果器链那个大圆角输入框」为准（它其实就是 `Md3Select` 药丸：34px / 999px 圆角），把所有同类下拉、文字输入统一过去；**不得使用原生非 MD3 组件**；内置合成器界面同样遵守，并且文案要 i18n。
6. **logo 底色改成 MD3 薰衣草紫**；应用**默认强调色**也定为 MD3 薰衣草紫。
7. **「+」打开面板：从列表里选中后，新加的标签页要自动跳转过去**。

**非目标**

- 复制粘贴 v1 只处理**单个**片段 / **单个**音符（多选批量复制留作后续）。这样无需新增引擎命令。
- 不做系统剪贴板互通（不写 OS clipboard）。

## 2. 决策记录

| 决策点 | 结论 |
| --- | --- |
| 控件规范样式 | `Md3Select` 的触发框：`height:34px; border-radius:999px; padding:0 12px; border:1px solid rgb(var(--md-outline)/0.7); background:rgb(var(--md-surface-container-high)); font-size:12px` |
| 两套下拉 | `InstrumentSelect` 合并进 `Md3Select`（给它加一个可选的底部动作行），删除 `InstrumentSelect.vue` |
| 文字输入框 | 同一套药丸：高 34px、圆角 999px、同边框/底色（重命名、预设命名、搜索、BPM 等） |
| 复制粘贴范围 | v1 单个片段 / 单个音符 |
| 粘贴落点 | 片段：该轨道**最后一个片段的结束拍**；音符：该片段**最后一个音符的结束（相对）拍** |
| 记忆长度来源 | 选区变化时记录被选中片段/音符的 `length`/`duration` |
| 默认强调色 | `lavender`（已是默认；只补 logo 底色与一处兜底校验） |

## 3. 详细设计

### 3.1 控件统一（`Md3Select` 为唯一原语）

- `src/components/common/Md3Select.vue`
  - 触发框样式保持现状（药丸 34px），作为**规范样式**；不改视觉。
  - 新增可选 **底部动作行**：`action?: { label: string }`，渲染成菜单底部一个带分隔线的按钮，点击 `emit("action")`（并关闭菜单）。供「浏览全部音色」使用。
- 用 `Md3Select` 替换 `InstrumentSelect`：
  - `src/components/workspace/InspectorPanel.vue` 的「音色」行改用 `Md3Select`（选项带 `group` = 分类；「无乐器」作为一个选项；底部 `action` = 浏览全部 → 打开 `InstrumentBrowser`）。
  - 删除 `src/components/common/InstrumentSelect.vue`。
- 文字输入框统一：
  - `InspectorPanel.vue` 的 `.name-input`（轨道名，26px/999px）、`PresetBar.vue` 的 `.name-input`（28px/8px）、`NumberField.vue` 的 `.step`（26px/999px）、`InstrumentBrowser.vue` 的 `.search`（40px/999px）→ 统一到 **34px / 999px / `--md-outline` 0.7 / `--md-surface-container-high` / 12px**；`focus` 态统一为 `border-color: --md-primary` + `box-shadow: 0 0 0 2px rgb(var(--md-primary)/0.22)`。
  - 在 `main.css` 里加一个共享类（例如 `.md3-field`）承载这套尺寸/形状，各组件复用它，避免五处再次漂移。删除或修正已死且带 typo 的 `.input-filled` / `.input-outlined`（`--md-on-surface-muted` 拼错成 `--md-on-surface-muted` 那处）。
- **不使用原生非 MD3 组件**：现状已无原生 `<select>`；保留 `NumberField` 的 `input type="text" inputmode="numeric"`（它自身是 MD3 外壳，无原生外观）。
- **内置合成器界面**：`SynthPanel.vue` 只用 `Md3Select`（经 `PresetBar`）、`Md3Knob`、自定义分段/垫子/开关——本来就合规；本任务只保证它引用的 `PresetBar` 输入框统一到新规范。

### 3.2 i18n 清扫

把探查到的硬编码文案改成 i18n 键（中英各一份），至少覆盖：

- `workspace/panes/PluginsPane.vue`：`加载到：` / `先选中一条轨道，再加载插件。` / `内置` / `待接入` / `` `加载 ${name}` ``。
- `workspace/TrackHeaderList.vue`：`` `轨道 ${next}` ``（新增轨道默认名，改为 i18n 模板）、`… 音符`、空态文案。
- `workspace/RightPanel.vue`：`尚未打开任何面板` / `从下面选择一个面板开始`。
- `views/ArrangeView.vue` / `views/EditView.vue`：空态文案（其中 EditView 的空片段提示按 §3.5 **删除**）。
- `views/SynthWindowView.vue`：`内置合成器`（标题/副标题）、`在主窗口选中这条轨道`。
- `components/common/PianoKeyboard.vue`：`试听键盘` / `` `试听 ${note}` ``。
- `views/SettingsView.vue`：几处 `aria-label` / `label`（`显示轨道栏` 等）。
- `views/MixView.vue`：空态与 `PAN`（`PAN` 是通用术语，可保留）。

### 3.3 复制粘贴（单个片段 / 单个音符）

- 新增轻量模块 `src/stores/clipboard.ts`（Pinia store，仅内存，不持久化）：
  ```ts
  type Payload =
    | { kind: "clip"; notes: Note[]; length: number }
    | { kind: "note"; note: Note };
  ```
  暴露 `copyClip(clip)` / `copyNote(note)` / `payload` / `clear()`。
- **编排视图**（`ArrangeView.vue`）：
  - `Ctrl/Cmd+C`：若恰好选中一个片段，拷贝其 `notes` 与 `length`。
  - `Ctrl/Cmd+V`：在**该片段所属轨道**、`max(片段结束拍)` 处 `store.addClip(track, start, length)`；随后把拷贝的音符用 `store.addNote` 逐个写入新片段（v1 允许逐条写，撤销会分多步；多音符批量写留后续）。完成选中新片段。
  - 与现有 `Delete/Backspace` 处理共存（`onKey` 里加分支；输入框聚焦时不拦截）。
- **钢琴卷帘**（`EditView.vue`）：
  - `Ctrl/Cmd+C`：若恰好选中一个音符，拷贝它。
  - `Ctrl/Cmd+V`：在当前片段、`max(音符 start+duration)` 处 `store.addNote(...)`（音高、力度、音量沿用副本）。完成选中新音符。

### 3.4 「记忆长度」点击创建

- 在 `src/stores/selection.ts` 增加：
  ```ts
  const lastClipLength = ref(1);   // 上一次选中的片段长度（拍）
  const lastNoteLength = ref(0.5); // 上一次选中的音符时长（拍）
  ```
  并提供 `rememberClipLength(n)` / `rememberNoteLength(n)`（有限且 > 0 才写）。
- 记录时机：
  - `ArrangeView`：片段被选中时 `rememberClipLength(clip.length)`；新建片段后也记。
  - `EditView`：音符被选中时 `rememberNoteLength(note.duration)`；新建音符后也记。
- **单击空白即创建**：
  - `ArrangeView.onLanePointerDown`（左键、非框选模式）：按下仍进入 `create`；**松手时若长度 < `MIN_CREATE_BEATS`（即「原地点击」）**，改为用 `selection.lastClipLength` 在该（吸附后的）拍创建一个片段（而不是只选中轨道）。
  - `EditView`：`CLICK_NOTE_BEATS` 常量改为 `selection.lastNoteLength`（原地点击新建音符的时长）。
- 二者都吸附到 `timeline.snap`（计划 D 之后为 `snapAt`）。

### 3.5 其它小改

- **去掉空片段文字提示**：删除 `EditView.vue` 里 `.roll-empty` 元素与其样式（保留 `roll-blank`：「没有片段」时的入口，那是另一种情形）。
- **控制器泳道默认属性**：`EditView.vue` 的 `controllerProperty` 初值 `"velocity"` → `"volume"`。
- **标签自动跳转**：`PanelTabs.vue` 的 `pick(id)` 由 `layout.addTab(id)` 改为 `layout.openTab(id)`（`openTab` 会设为当前标签并展开右侧面板）。
- **logo 底色与默认配色**：
  - `Logo.vue` 增加一个「填充药丸底」形态（或用外层包裹）：底色 `rgb(var(--md-primary))`，图形用 `rgb(var(--md-on-primary))`；`TrackHeaderList` 的品牌区用这个形态。
  - 默认强调色保持 `lavender`（`useTheme.loadAccent()` 已默认 lavender）；确认首启无保存值时 `data-accent="lavender"`，并让 `main.css` 显式声明 `[data-accent="lavender"]` 一组与 `:root` 相同的值，避免「没写就等于默认」的隐式依赖。

## 4. 改动文件清单

- `src/components/common/Md3Select.vue`：可选底部动作行。
- `src/components/common/InstrumentSelect.vue`：**删除**。
- `src/components/workspace/InspectorPanel.vue`：音色行改 `Md3Select`；输入框统一。
- `src/components/common/PresetBar.vue`、`NumberField.vue`、`InstrumentBrowser.vue`：输入框统一到 `.md3-field`。
- `src/assets/main.css`：加 `.md3-field`；清理死的/带 typo 的旧输入类；补 `[data-accent="lavender"]`。
- `src/components/common/Logo.vue`、`src/components/workspace/TrackHeaderList.vue`：logo 填充形态。
- `src/components/workspace/PanelTabs.vue`：`pick` → `openTab`。
- `src/stores/clipboard.ts`（新）、`src/stores/selection.ts`（记忆长度）。
- `src/views/ArrangeView.vue`、`src/views/EditView.vue`：复制粘贴、点击创建、默认属性、去提示。
- 大量 `src/i18n/{zh,en}.ts` 文案键。

## 5. 验收标准

1. 选中一个片段按 Ctrl/Cmd+C，再按 Ctrl/Cmd+V：同轨道尾部出现等长副本（含音符）；音符同理（片段尾部出现副本）。
2. 单击编排空白 → 按上一次选中片段的长度创建；单击卷帘空白 → 按上一次选中音符的时长创建；连续点击可快速铺出等长内容。
3. 钢琴卷帘里空片段不再有文字提示。
4. 控制器泳道默认显示「音量」。
5. 所有下拉/输入框外观一致（34px 药丸）；全项目无原生 `<select>`；`InstrumentSelect.vue` 不再存在；新出现的文案均可中英切换。
6. 品牌区 logo 为薰衣草紫底；首启默认强调色为薰衣草紫。
7. 从「+」列表选一个面板后，界面自动切到该标签页。

## 6. 验证方式

- 前端无测试框架：`npm run build` 通过 + `npm run dev` 手动核对（浏览器预览即可覆盖上述全部交互）。
- Rust 无改动，无需跑引擎测试。

## 7. 风险

- **点击创建 vs 选中轨道**：改后「点空白」变成创建，选中轨道改由左侧音轨栏 / 留白竖线 / 顶栏承担（留白竖线可点选，见计划 B）。
- **复制粘贴的撤销粒度**：v1 单片段/单音符，逐条写入；多音符片段粘贴会产生多条撤销记录（已记为后续项）。
- **控件统一样式**：`InstrumentSelect` 合并进 `Md3Select` 时不要丢掉「无乐器」「分类分组」「浏览全部」三个能力。
