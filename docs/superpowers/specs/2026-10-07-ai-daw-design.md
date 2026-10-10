# AI DAW — 轻量级 AI 原生 DAW 设计文档

- 日期：2026-10-07（**2026-10-10 校准**）
- 状态：已评审通过，**v1 主体已实现**（进度见 `../ROADMAP.md`）
- 许可证：Apache-2.0（**纯 Apache-2.0-only，无 GPL**）

> **修订说明（2026-10-10）**：本文档的部分技术选型已随实现演进，权威口径以 `../ROADMAP.md` 为准。主要偏差：前端实际为 **Vue 3 + Pinia + Vue Router + Tailwind**（非 React + `@m3e/react`）；内置合成器为**手写正弦 + 线性 ADSR**（未引入 `fundsp`）；VST3 宿主**实装为 `vst3-host` + `clack-host`**（原首选 `truce-rack` 的 VST3 后端托管不了「无音频输入总线的音源」、CLAP 后端加载 KR-106 会段错误，见 ROADMAP §2）；**不使用 ASIO**（其 SDK 为 GPLv3/专有，破坏 Apache-2.0）。§3 与 §5 的状态列已按实现翻正。

## 1. 目标与非目标

### 目标
- 一个**极轻量、干净、好用的原生 DAW**：多轨 MIDI 编曲、插件乐器宿主、基础混音与导出、音频片段/采样器、音频录制。
- **插件优先**：所有音色都是插件；DAW 内置**唯一**一个音色插件——合成器。内置合成器与外部插件实现**同一套 ABI**。
- **可被 AI 驱动**：引擎的每条变更都走统一的命令/事件总线，外部 harness（MCP + 本地 API + skills 文档包）可以捏音色、编曲、导出。
- **AI 操作可视化 + 人类随时接管**：事件流带 actor 标签，前端动效呈现 AI 在做什么；人类可直接编辑、可暂停 agent、可全量撤销。

### 非目标（v1 不做）
- 插件进程隔离/沙箱（边界预留，后置）。
- 视频/记谱/乐谱打印、云协作、内置音频大模型。
- 移动端。

### "轻量"的定义
指**交互与体验的干净**（清爽 UI、快启动、低认知负担），不代表功能阉割。功能范围按 v1 明确列出。

## 2. 设计原则

1. **插件优先**：任何音源/效果都通过统一插件 ABI 接入，内置合成器是"第一个插件"。
2. **工程即唯一真相源**：单份工程状态，AI 与人类改同一份，UI 订阅变更流。
3. **接口先于功能**：引擎对外只暴露 Command/Event API；UI 与 AI 都只是客户端。
4. **逻辑分层、物理先合**：引擎为独立 crate，v1 同进程运行，接口即契约，将来无痛拆进程/无头。

## 3. 技术选型

| 层 | 选择 | 状态 |
| --- | --- | --- |
| 应用外壳 | Tauri v2 | ✅ |
| 前端 | **Vue 3 + Pinia + Vue Router + Tailwind**（Material You token，适配自 Exameow） | ✅ |
| 音频 IO | `cpal`（Windows 走 **WASAPI**，不用 ASIO——见下方许可证结论） | ✅ |
| 实时/离线渲染 | 自研 `Player` + `ScheduledEvents`（离线与实时逐位一致） | ✅ |
| 内置合成器 DSP | **手写正弦振荡器 + 线性 ADSR**（`fundsp` 未引入） | ✅ |
| WAV 读写 | `hound` | ✅ |
| MIDI 输入 | `midir` | ⬜ |
| 音频解码 | `symphonia`（wav / flac / mp3 / ogg / aac / m4a） | ✅ |
| 保音高变速 | `rubato` | ⬜（v1 明确不做） |
| VST3 / CLAP 宿主 | **`vst3-host` + `clack-host`**（未用 `truce-rack`，见上方「修订说明」） | ✅ |
| CLAP | `clack-host`（已随宿主一并落地，非「后置」） | ✅ |
| 响度/频谱分析 | `ebur128` / `rustfft` | ✅ |

### 许可证结论
- **VST 3.8 SDK（2025-10）已改为纯 MIT**，GPLv3/专有双授权已取消。MIT 与 Apache-2.0 兼容（保留 MIT 声明）。宿主 VST3 + Apache-2.0 开源可行。
- **ASIO SDK 同期转为 GPLv3（开源变体）/ Steinberg 专有**：在 Windows 启用 ASIO 后端会使整个应用受 GPLv3 约束，**与纯 Apache-2.0 不兼容**。故本项目**不使用 ASIO**，低延迟走 WASAPI。
- **依赖策略**：只放行 MIT / Apache-2.0 / BSD / ISC / CC0 / Zlib；**禁 GPL / AGPL / ASIO 专有件**（用 `cargo-deny` 在 CI 拦截）。据此，`Dhwani`（GPL-3.0-only）**不可用**。
- 商标：使用 "VST" 名称/Logo 可选；若使用需遵守 Steinberg 商标规范，不影响 MIT 授权。
- 注意：`openDAW`、`Strudel` 为 **AGPL**，仅可借鉴思路，不得直接抄代码。

## 4. 整体架构（路线 B）

引擎是**不依赖 UI 的独立 crate**，对外只暴露 Command/Event API。v1 运行在 Tauri 进程内，但内部模块边界即未来的进程边界；后续升级为独立进程（共享内存传音频 + IPC 传控制）或无头运行，均**零重构**。

```
┌──────────────────────────────────────────────────────────┐
│ Tauri App (src-tauri)                                     │
│   ├─ owns engine instance (v1, in-process)                │
│   ├─ bridges ipc Command/Event ↔ Tauri commands/events    │
│   └─ (later) MCP server + local HTTP/WS                    │
│   ┌────────────────────────────────────────────────────┐ │
│   │ Vue 3 + Pinia + Tailwind  (时间线/钢琴卷帘/混音台/可视化) │ │
│   └────────────────────────────────────────────────────┘ │
├──────────────────────────────────────────────────────────┤
│ crates/engine  (no UI deps)                                │
│   project model · command/event bus · transport ·          │
│   sequencer · audio graph · render (RT + offline) · mix    │
│        ▲                        │                          │
│        │ plugin ABI             │ analysis                 │
│   ┌────┴─────┐  ┌───────────┐  ┌┴──────────┐              │
│   │ synth    │  │ host-vst3 │  │ analysis  │              │
│   │ (内置)   │  │ (适配器)  │  │ listen    │              │
│   └──────────┘  └───────────┘  └───────────┘              │
└──────────────────────────────────────────────────────────┘
```

## 5. Workspace / Crate 划分

- `crates/engine` — ✅ **已实现**：工程模型、命令/事件总线、传输、调度、实时与离线渲染、混音。
- `crates/plugin-api` — ✅ **已实现**：统一插件 ABI（trait + 描述符 + 参数模型 + 状态）。
- `crates/synth` — ✅ **已实现**：唯一内置音色插件（**手写正弦 + 线性 ADSR**，非 `fundsp`），实现同一 ABI；另有 8 类 32 条出厂预设。
- `crates/ipc` — ✅ **已实现**：`Command` / `Event` / `Project` schema 的**唯一来源**（serde）；引擎以薄壳再导出，路径与行为不变。
- `crates/host-vst3` — ✅ **已实现**：VST3 用 `vst3-host`、CLAP 用 `clack-host`，适配到同一 ABI（含扫描 / 参数 / 状态 / 原生窗口）。
- `crates/asset` — ✅ **已实现**：音频解码（`symphonia`：wav / flac / mp3 / ogg / aac / m4a）+ 波形 `peaks`（MPL-2.0，见 `NOTICE`）。
- `crates/effects` — ✅ **已实现**：内置效果器（延迟 / 混响 / 合唱 / 过载 / 两段 EQ，共 5 个）。
- `crates/analysis` — ✅ **已实现**：渲染结果 → 响度（EBU R128：LUFS / LRA / 真峰值）+ 频谱指标（AI 的"听感"闭环；`ebur128` / `rustfft`）。
- `src-tauri` — ✅ **已实现**：Tauri App（命令层 + 实时传输 + 内嵌 MCP）。
- `src` — ✅ **已实现**：**Vue 3** 前端。
- `docs` — 文档与设计。

## 6. 核心领域模型

- `Project` — 聚合根，序列化为**人类可读 JSON**。
- `Track` — 承载 clips、插入效果、指向 MixerChannel。
- `Clip` — `MidiClip`（含 `Note`）或 `AudioClip`（含音频引用/波形/crop）。
- `Note` — pitch、start、duration、velocity、可选 articulation。
- `AutomationLane` — 参数自动化曲线。
- `PluginInstance` — 插件引用（format/id）+ 状态 blob + 参数现值。
- `MixerChannel` — gain / pan / sends。
- `Bus` / `Master` — 总线与主输出。
- `Transport` — tempo、time signature、loop region、播放位置。

### 变更模型（事件溯源）
所有修改 = `Command` → 产出 `Event` → 应用到状态 → 持久化。
- 人类与 AI 走同一条路 → 撤销/重做、回放、可视化天然可得。
- `Event` 带 `actor: Human | Agent` 与目标元数据，供前端标注"谁改了什么"。

## 7. 音频图与渲染

- 信号流：`Track → 插入效果链 → MixerChannel(gain/pan/send) → Bus → Master → 输出`。
- 块处理（256/512 帧），采样级 MIDI 音序。
- **实时（cpal）与离线渲染共用同一张图**，导出与播放**逐位一致**。
- 离线渲染用于：导出（WAV/分轨）、`analysis`（AI 听感）、确定性回归测试。

## 8. 插件 ABI（统一音色接口）

- 描述符：`id`、`name`、`kind: Instrument | Effect`、I/O 通道布局。
- 生命周期：`init(sample_rate, block_size)`、`reset`、`process`。
- 处理签名（已定形）：`process(&ProcessContext, audio: &mut [&mut [f32]], events: &[TimedMidiEvent])` —— **单一就地平面缓冲**，宿主调用前已把本块输入预填其中（效果器填上游信号，音源填静音）；插件就地读写，不得改通道数/帧数、不得分配。理由：Rust 下唯一无别名问题的表达，且与 nih-plug/JUCE 一致；VST3 适配时把同一块内存同时指给 `inputs`/`outputs`。参数不走 `process`，用 `get_param`/`set_param`。
- 参数模型：`id`、名称、范围、当前值、可自动化标志；支持通用参数 UI。
- 状态：`save_state` / `load_state`（存入工程）。
- 适配器：
  - `synth` 原生实现（内置第一个插件）。
  - `host-vst3` 适配 VST3（v1：扫描、加载、通用参数、状态存取；插件原生 GUI 后置）。
  - 后续：CLAP（`clack`）、AU。
- 插件发现/扫描 + 注册表；缺失插件加载时给占位与告警。

## 9. API 层（一鱼三吃）

同一份 Command/Event 契约三种出口：
1. 引擎 Rust API（进程内）。
2. Tauri commands/events → Vue 前端。
3. **外部**：MCP server + 本地 HTTP/WS + **skills 文档包**（后置）。

- harness 通过 skills 学会调用工具，完成**捏音色 / 编曲 / 导出**，无需改动引擎。
- `analysis` 提供 `listen` 式能力：渲染任意区间并返回响度/频谱等量化指标，构成 AI 的反馈闭环。
- AI 接入点后置，但 v1 的数据模型与 API 已为其预留。

## 10. 前端与 AI 可视化 + 人类接管

- 视图：传输条、轨道列表、编曲时间线、钢琴卷帘/步进器、混音台、插件浏览器 + 通用参数面板、音频片段编辑器（波形）、导出。
- **AI 可视化**：UI 订阅事件流，高亮被改动的轨道/音符/旋钮并做 MD3 Expressive 动效，附"AI 操作时间线"；AI 命令可节流播放以便人类观察。
- **人类接管**：人类编辑走同一总线即时生效；可暂停 agent；全量撤销/重做。

## 11. 错误处理

- v1 不做插件进程隔离（单进程），但边界已预留；对插件 `process` 尽量 catch panic。
- 缺失插件、渲染失败、音频设备丢失 → 统一以事件上报，UI 呈现。
- 系统边界（文件、用户输入、外部插件）做校验；内部代码信任不变量。

## 12. 测试策略

- `engine`：工程模型与命令/事件**确定性**单测 + **离线渲染黄金 WAV 哈希**。
- `synth`：DSP 单测（脉冲响应、稳定性等）。
- `host-vst3`：用免费插件 + VST3 官方 Validator 做集成测试。
- 前端：组件测试；后续用 `tauri-webdriver` 做 E2E。
- 关键不变量：同工程 + 同命令序列 → 逐位一致的渲染结果。

## 13. v1 构建阶段

1. engine 核心：工程模型 + 命令/事件 + 传输 + 离线渲染内置合成器 → WAV。
2. cpal 实时播放同一张图。
3. 插件 ABI + 内置合成器 crate。
4. VST3 宿主 crate + 扫描 + 通用参数。
5. Tauri + Vue 3 外壳：时间线/钢琴卷帘/混音台。
6. 音频片段/采样器 + 录音。
7. 导出（WAV / 分轨）。
8. （后置）MCP/HTTP/WS + skills 包 + AI 可视化 + `analysis` 闭环。

## 14. 决策记录

| 决策 | 结论 | 理由 |
| --- | --- | --- |
| 音频引擎位置 | 独立 crate，v1 同进程 | 兼顾开发速度与"可无头/可拆进程/可被 AI 驱动"的终局 |
| 插件格式 | VST3 优先 | 生态最大；SDK 已 MIT，许可无碍 |
| 内置音色 | 仅一个合成器，实现同一 ABI | 贯彻"所有音色都是插件"，保持精简 |
| 工程格式 | 人类可读 JSON，事件溯源 | AI 与人类共改同一份；可订阅、可撤销 |
| 前端 | Vue 3 + Pinia + Vue Router + Tailwind（Material You） | 实际采用；轻量、生态成熟、便于与 Tauri IPC 对接 |
| 开源许可 | Apache-2.0 | 与 MIT 的 VST3 SDK、clack 兼容 |
