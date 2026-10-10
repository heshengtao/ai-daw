# 内置合成器界面重设计 + 编排网格适配 — 设计文档

- 日期：2026-10-08
- 状态：**已完成（2026-10-10 校准）；预设部分已被 `2026-10-09-synth-overhaul-design.md` 取代**（24 条 → 32 条、类别重排；默认正弦 / 固定尺寸窗口 / 一个周期示波器 / MD3 下拉均已落地）
- 主题：内置合成器（`builtin.synth`）的默认音色、插件界面、示波器与预设；编排界面的左侧留白与网格随拍号适配

## 1. 背景

内置合成器目前是「一台老式减法合成器的面板」：18 个参数平铺成 6 张卡片，示波器显示的是**真实音频**。

面向的是新手小白，现在的问题是：

1. **初始音色不是正弦**。`synth.note` 里已经写着「16 复音正弦」，但引擎 `PARAM_TABLE` 的 `wave` 默认值是 `0.0`（锯齿）——文案与行为不一致。
2. **插件窗口尺寸不固定**。窗口初始 1060×720、可自由缩放，面板是流式布局，窗口一拉键盘就被拉伸变形。
3. **参数太多、术语太硬**。18 个参数一次性铺开，`PWM / Glide / Key Follow / LFO` 对小白没有意义。
4. **示波器对小白没有教学价值**。显示真实音频，转旋钮时屏幕只是抖动，看不出「波形本身变了」。
5. **预设太少**（4 条），且选择器用的是**原生 `<select>`**，和 MD3 圆角语言对不上。
6. **编排界面左侧留白突兀**：时间轴在 T=0 左侧预留了 26px（`TIMELINE_INSET_PX`），但那里是一片空白，与左侧音轨栏的轨道色没有任何呼应。
7. **网格没有按拍号呈现**：网格除了拍线、小节线，还叠了一层固定的 1/16 细线；用户期望「一个小节就是分子个格子」。
8. **底部位置读数在 /8 拍号下与网格对不上**：`BottomBar` 用 `beats / 分子` 计算小节，而网格/标尺用 `barBeats = 分子 × 4/分母`。

## 2. 目标与非目标

**目标**

- 新建轨道的内置合成器默认音色为**正弦**。
- 插件界面**尺寸固定**，键盘位置/比例不变形；面板分为**基础**与**高级**两层，默认只露基础。
- 示波器显示**一个周期的波形**，随旋钮**解析地**变化，用于教学。
- 提供**大量预设音色**；所有下拉框改用 MD3 组件，**不出现任何原生 `<select>`**。
- 编排视图左侧留白放**对应轨道色的竖线**，与左栏呼应。
- 切换拍号后，编排与编辑两个视图的网格**每小节格子数 = 分子**；修掉底部位置读数的对不齐问题。

**非目标（本次不做）**

- 不改引擎 DSP、插件 ABI（`plugin-api`）、VST3/CLAP 宿主、工程序列化格式。
- 不改外部插件的参数表（`PluginParamList`）。
- 不动 `BUILTIN_PRESETS`（引擎侧老工程兼容用的插件 id 与补丁）。
- 不删后端实时示波器管线（`scope_data` 命令、`crates/engine/src/scope.rs`）——只让合成器界面不再轮询它。

## 3. 决策记录

| 决策点 | 结论 |
| --- | --- |
| 每小节格子数 | 每拍一格（4/4→4，3/4→3，7/8→7）；去掉 1/16 细线层 |
| 吸附精度 | **保持 1/16 拍不变**（卷帘仍可放十六分音符），只是可见格线变粗 |
| 预设数量 | 24 条出厂预设（6 类 × 4） |
| 侧边栏窄容器 | 整块面板**等比缩放**（键盘比例不变形） |
| 基础面板参数 | 波形 / 亮度(Cutoff) / 尖锐度(Resonance) / 起音 / 衰减 / 延音 / 释音 / 音量（共 8 个）；共振留在基础层 |
| 窗口行为 | 固定内尺寸、禁用缩放（`.resizable(false)`） |

## 4. 详细设计

### 4.1 默认音色 = 正弦

- `crates/synth/src/lib.rs`
  - `PARAM_TABLE` 第 4 行 `("wave", "Wave", 0.0, 3.0, 2.0)`（默认 `2.0` = Sine）。
  - `default_params()` 里 `wave: 2.0`。
  - 保留 `Wave::from_param` 的兜底（越界→Saw）不动。
  - 新增测试：`default_wave_is_sine`，断言 `SynthPlugin::new().get_param("wave") == Some(2.0)`。
- `crates/engine/src/registry.rs`
  - `BUILTIN_PRESETS` 里 `builtin.synth` 的补丁为空 → 自然变成正弦。
  - 为保住 lead / bass / pad 的原有手感，在它们的补丁里**显式补 `("wave", 0.0)`**（锯齿）；pluck 已是 `1.0`（方波），不动。
- `src/data/plugins.ts`
  - `SYNTH_PARAMS` 里 `wave.default` 改 `2`。
  - `SYNTH_FACTORY_PRESETS` 的 lead / bass / pad 补 `wave: 0`（与引擎补丁一致）。
- 兼容性：状态序列化按 `PARAM_TABLE` 顺序，顺序不变；老工程的 12 字节状态仍可读。**只有新建轨道/新建预设未指定 `wave` 时**才是正弦。

### 4.2 插件界面：固定尺寸 + 基础/高级两层

**尺寸与缩放**

- `src-tauri/src/commands.rs::open_synth_window`
  - `.inner_size(960.0, 700.0)`、`.resizable(false)`（去掉 560×460 的最小尺寸）。
- `SynthPanel.vue` 改为**固定设计宽 900px** 的设备面板：
  - 外层 `.fit`（`width:100%`）用 `ResizeObserver` 计算 `scale = min(1, 容器宽 / 900)`，对内层 `.device { width:900px; height:560px }` 施加 `transform: scale(scale); transform-origin: top center;`，并把 `.fit` 高度设为 `560*scale`。
  - 窗口里容器宽 ≥ 900 → `scale=1`，与设计稿 1:1；侧边栏窄时等比缩小，**键盘比例永不变形**。
- `SynthWindowView.vue`：固定头部（标题 + 「在主窗口选中此轨道」）+ 设备区，整体不出现滚动条。

**布局（参考 AKAI MPK mini 的排布）**

```
┌───────────────────────────────────────────────┐
│ 预设垫子（当前类别 4 个，大圆角方块，点一下即切）   │  ← 顶部
│ [类别 chips]      [预设 ▾ Md3Select] [另存为][删] │
├───────────────────────┬───────────────────────┤
│ 屏幕：一个周期示波器    │  基础旋钮组 / 高级面板   │
│ + ADSR 曲线            │  （高级按钮切换）        │
├───────────────────────┴───────────────────────┤
│ 试听键盘（固定高度，2 个八度）                    │  ← 底部
└───────────────────────────────────────────────┘
```

- **基础面板**：波形（MD3 分段按钮，正弦/锯齿/方波/三角，配波形小图标）、亮度(Cutoff)、尖锐度(Resonance)、起音/衰减/延音/释音 + ADSR 曲线、音量（一个偏大的旋钮，呼应参考图右侧大旋钮）。
- **「高级」按钮**：MD3 填充/tonal 按钮，位于面板右上；点击展开高级区（弹簧动画）。状态存组件内 `ref`（不持久化）。
- **高级面板**：Sub、PWM、滑音(Glide)、HPF、滤波包络量(FilterEnv)、键跟随(KeyFollow)、LFO 速率/延迟/音高/滤波。按老式减法合成器分区（DCO / HPF / VCF / VCA / LFO），复用 `Md3Knob`。
- 显示名做小白化 i18n：Cutoff→亮度、Resonance→尖锐度、Attack→起音、Decay→衰减、Sustain→延音、Release→释音、FilterEnv→滤波包络、KeyFollow→键跟随、Gain→音量（其余保留技术名，因为只在高级区）。

**共用性**：`SynthPanel` 仍被 `PluginEditorPane.vue`（侧边栏兜底）复用；固定尺寸 + 缩放逻辑放在 `SynthPanel` 内部，侧边栏无需改动即可工作。

### 4.3 一个周期的示波器

- 新组件 `src/components/common/Oscilloscope.vue`：
  - props：`params: Record<string, number>`、`height?`；用 `<canvas>` 绘制。
  - 在一个基频周期上取 `N=256` 点，按真实 DSP 的算法解析生成：
    1. `base = wave(phase, pwm)`（`wave` 0..3 → 锯齿/方波/正弦/三角；方波占空比 `0.5 + (pwm-0.5)*0.9`）。
    2. `sub > 0` 时叠加 `sub * square(phase * 0.5)`（低八度，画面上呈现「半个周期」的偏移）。
    3. 通过一个由 `cutoff` / `resonance` 推导截止频率的**一阶低通**（对显示波形做平滑/过冲），让「亮度/尖锐度」也能看出变化。
    4. 归一化到 `[-1,1]` 并留边距。
  - 结果：转波形 / 脉宽 / 副振 / 亮度 / 尖锐度，屏幕上的波形都会**真实地变形**。
  - 空闲时也一直显示（无 NOTE 也画一个周期），与小白预期一致。
- `SynthPanel.vue` 不再使用 `WaveformDisplay`；`src/components/common/WaveformDisplay.vue` 仅被它使用，**删除**。
- 后端 `scope_data` / `scope.rs` / `api.tauri.scopeData` **保留不动**（非目标）。

### 4.4 预设音色与 MD3 下拉

- `src/data/plugins.ts`
  - `SYNTH_FACTORY_PRESETS` 扩到 **24 条**，6 类各 4：贝斯 Bass / 主音 Lead / 键盘 Keys / 铺底 Pad / 拨弦 Pluck / 音效 FX。
  - `FactoryPreset` 增加可选 `category: string`，取值为固定 slug（`bass|lead|keys|pad|pluck|fx`）；界面把 slug 映射为 i18n 键（`presetCategory.<slug>`）后显示，供分组与筛选。
  - 名称/说明沿用 i18n key 约定（`preset.<slug>.name` / `.note`），在 `zh.ts` / `en.ts` 各补齐 24 条。
  - 新增预设的 `id` 只用于界面匹配（形如 `builtin.synth.<slug>`），**不进** `LEGACY_SYNTH_IDS`，不参与 `canonicalPluginId`。
- `src/stores/presets.ts`
  - `PresetOption` 增加 `category`；`forPlugin` 透传。
  - `match` 逻辑不变（仍按参数完全相等匹配，取最后一条）。
- `src/components/common/PresetBar.vue`（重构）
  - 删除原生 `<select>`，改用 `Md3Select`（选项带类别 `hint` 或分组）。
  - 保留「另存为… / 删除」与命名输入框。
- `src/components/common/Md3Select.vue`
  - 增加可选的**分组**支持：`SelectOption` 增加可选 `group`，菜单里渲染分组标题（不改现有调用方行为）。
- 预设垫子：`SynthPanel` 顶部渲染当前类别下的预设方块（MD3 大圆角），点按即 `@apply` 整组参数；类别用 MD3 filter chip 切换。

### 4.5 编排左侧留白 → 轨道色竖线

- `src/views/ArrangeView.vue`
  - 每条 `.lane` 内新增 `.lane-accent`：绝对定位，`left: -${TIMELINE_INSET_PX}px`、宽 3px、`inset-block:0`、`border-radius`，背景 `hsl(var(--track-hue) 55% 55%)`。
  - 同一条 lane 再叠一层**很淡的同色底**覆盖这 26px 留白，弱化空白：`background: hsl(var(--track-hue) 55% 55% / 0.10)`。
  - 因为 `.arrange` 的 `padding-left` 就是留白区，子元素负偏移落在 padding box 内，不会被裁掉。
  - `TrackHeaderList.vue` 不动（它已有 `.grip` 同色）。

### 4.6 网格：一个小节 = 分子个格子

- `src/stores/timeline.ts::gridBackground`
  - 去掉 `fine`（`zoom*snap`）那一层，只保留：拍线（`zoom*meterBeat`，即每个格子）+ 小节线（`zoom*barBeats`）。
  - `snap`（`meterBeat/4`）**保持不变**——吸附精度不变，只改可见格线。
  - 由于 `meterBeat`/`barBeats` 本就由拍号推导，改完后：4/4→每小节 4 格、3/4→3、6/8→6、7/8→7；编排、编辑、控制器泳道共用同一个背景，自动一致。
- `src/components/layout/BottomBar.vue`
  - `bars` 与 `positionText` 改用 `timeline.barBeats`（`分子 × 4/分母`）而不是裸 `分子`，使底部读数与网格/标尺对齐。

## 5. 改动文件清单

**Rust**

- `crates/synth/src/lib.rs`：默认 `wave=2.0`；新增默认波形测试。
- `crates/engine/src/registry.rs`：lead/bass/pad 补丁显式 `wave=0.0`。
- `src-tauri/src/commands.rs`：`open_synth_window` 固定尺寸 + 禁用缩放。

**前端（新增）**

- `src/components/common/Oscilloscope.vue`。

**前端（修改）**

- `src/data/plugins.ts`：`wave` 默认；24 条出厂预设（带 `category`）。
- `src/stores/presets.ts`：`PresetOption.category`。
- `src/i18n/zh.ts` / `src/i18n/en.ts`：预设文案 + 小白化参数名。
- `src/components/workspace/panes/SynthPanel.vue`：重写（固定尺寸 + 基础/高级 + 示波器 + 预设垫子）。
- `src/components/common/PresetBar.vue`：原生 `<select>` → `Md3Select`。
- `src/components/common/Md3Select.vue`：可选分组。
- `src/views/SynthWindowView.vue`：固定尺寸布局。
- `src/stores/timeline.ts`：`gridBackground` 去掉细线层。
- `src/views/ArrangeView.vue`：`.lane-accent` 轨道色竖线。
- `src/components/layout/BottomBar.vue`：位置读数用 `barBeats`。

**前端（删除）**

- `src/components/common/WaveformDisplay.vue`（仅被 `SynthPanel` 使用）。

## 6. 验收标准

1. 新建工程/新建轨道，内置合成器默认发声为**正弦**；示波器显示的也是一个周期正弦。
2. 合成器窗口不能缩放；在有缩放兜底的面板里，键盘比例不变形。
3. 默认只显示基础参数；点「高级」后出现 Sub/PWM/Glide/HPF/FilterEnv/KeyFollow/LFO。
4. 合成器界面内**没有任何原生 `<select>`**；预设选择器是 MD3 下拉。
5. 示波器只画**一个周期**；拖动 波形/脉宽/副振/亮度/尖锐度 时波形实时变形。
6. 预设数量为 24，全部可选中并整组写回（一次撤销步骤）。
7. 编排界面左侧留白里，每条轨道有一条**对应颜色**的竖线。
8. 改拍号后，编排与编辑网格的**每小节格子数 = 分子**（4/4→4，3/4→3，7/8→7）；编排/编辑/控制器泳道一致。
9. 底部位置读数在 6/8、7/8 等拍号下与网格小节线对齐。

## 7. 验证方式

- Rust：`cargo test -p ai-daw-synth`（默认波形 + 现有用例）、`cargo test -p ai-daw-engine`（预设补丁不破）。
- 前端：`npm run build`（含 `vue-tsc` 类型检查）必须通过；`npm run dev` 浏览器预览核对布局与示波器（`openSynthWindow` 在预览里会回退到侧边栏面板，可验证缩放兜底）。
- 手动：改拍号核对两个视图网格与底部读数；拖旋钮核对示波器；点「高级」核对展开。

## 8. 风险与兼容

- **默认波形变化会影响「没写 wave 的预设」**：已在 lead/bass/pad 补丁显式 `wave=0.0` 兜底，保持原有听感。
- **老工程**：参数存在工程里，加载后按存值恢复，不受默认值影响；12 字节老状态仍可读。
- **可调窗口变小**：由固定尺寸替代，用户不能再拉大窗口——这是本次的需求本身。
- **侧边栏缩放**：`transform: scale` 会让 canvas 略糊，但比例正确；侧边栏本就是浏览器预览的兜底路径。
- **删除 `WaveformDisplay`**：确认无其他引用后再删（已确认仅 `SynthPanel` 使用）。
