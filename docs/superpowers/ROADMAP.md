# daw-party 总体计划（修正版）

- 更新：2026-10-08
- 定位：本文件是**项目进度与后续计划的唯一索引**。设计意图见 `specs/2026-10-07-ai-daw-design.md`；逐步实现步骤见 `plans/` 下的分计划。
- 背景：本文档修正了设计文档与分计划中已经与现实不符的若干陈述（见 §2），并给出剩余工作的统一路线（见 §5）。

## 1. 现状总览

| 编号 | 范围 | 状态 | 证据 / 说明 |
| --- | --- | --- | --- |
| Plan 1 | 无头引擎：工程模型 + 命令/事件 + 离线渲染 + WAV | ✅ 完成 | `crates/{plugin-api,synth,engine}`；`engine/tests/render_to_wav.rs` |
| Plan 2 | cpal 实时播放 + 传输控制 + 离线==实时 | ✅ 完成 | `engine/src/{schedule,player,transport,command_queue,realtime}.rs`；`examples/play_demo.rs` |
| Plan 4a | 最小桌面外壳：窗口 + 工程操作 + 试听 + 导出 | ✅ 完成（栈有变） | `src-tauri/src/commands.rs` 共 7 个命令；前端**已改用 Vue 3**（见 §2） |
| Plan 4b | 把原生实时传输接进 UI | ⬜ 未开始 | 前端仍是「离线渲染 → `<audio>`」（`src/composables/usePlayback.ts`）；后端无 play/pause/seek |
| Plan 3 | VST3 宿主 | ⬜ 未开始 | 无 `crates/host-vst3` |
| Plan 5 | 录音 | ⬜ 未开始 | — |
| Plan 6 | 音频片段 / 采样器 | ⬜ 未开始 | 模型只有 MIDI `Clip`，无 `AudioClip` |
| Plan 7 | AI 接口（MCP + skills + 可视化） | ⬜ 未开始 | 无 MCP；但事件流已带 `actor` 标签（`Engine::execute`） |
| 新增 | `crates/ipc`（Command/Event schema） | ⬜ 未开始 | 设计文档 §5 已列出，实际未建 |
| 新增 | `crates/analysis`（响度/频谱） | ⬜ 未开始 | 设计文档 §5 已列出，实际未建 |
| 新增 | 插件商店 | ⬜ 未开始 | — |

**已具备、可直接复用的能力**：确定性离线渲染（同一工程逐位一致）、cpal 实时宿主（RT-safe，回调内不分配内存）、无锁 SPSC 传输命令队列、事件溯源 + 撤销、统一插件 ABI（`Plugin` trait）、内置合成器、工程 JSON 往返、Tauri 命令层 + 引擎事件广播、Vue 外壳（顶栏 / 传输条 / 轨道列表 / 检视器 / 活动流 / 主题）。

## 2. 修正记录（设计文档 / 分计划 与现实及事实的偏差）

| 项 | 原述 | 实际 / 修正 |
| --- | --- | --- |
| 前端 | React 18 + `@m3e/react` | **Vue 3 + Pinia + Vue Router + Tailwind**（Material You token 适配自 Exameow）；自提交 `94d2797` 起重建 |
| 内置合成器 DSP | `fundsp` | **手写正弦振荡器 + 线性 ADSR**，只依赖 `plugin-api` |
| 开源许可 | 「Apache + GPL 双许可」（外部建议） | **纯 Apache-2.0**（`Cargo.toml` `license = "Apache-2.0"`、`NOTICE`），无 GPL |
| VST3 宿主选型 | 评估 `rack`、`symdaw/audio-plugin-host` | 首选 **`truce-rack`**（MIT OR Apache-2.0、纯 Rust binding，CLAP+VST3 默认、AU/LV2 可选）；`rack` 是其前身（含 C++ glue） |
| 低延迟（Windows） | （外部建议）启用 cpal 的 ASIO 后端 | **不启用 ASIO**：ASIO SDK 为 GPLv3 / 专有，会破坏纯 Apache-2.0 分发。走 WASAPI |
| AI 听感引擎 | （外部建议）Dhwani | **不用**：Dhwani 是 **GPL-3.0-only**，链接即传染。改用 `ebur128` / `rustfft` |
| 插件宿主（外部建议首选） | UAPMD | 仅作 **MCP / 能力参考**：UAPMD 是 **C++/CMake** 工程，接入需 FFI，与引擎 `#![forbid(unsafe_code)]` 相悖 |

## 3. 许可证护栏（强制）

- 本项目许可：**Apache-2.0-only**。
- 允许的依赖许可：MIT / Apache-2.0 / BSD-2-Clause / BSD-3-Clause / ISC / CC0 / Zlib / Unlicense。
- **禁止**：GPL / AGPL / ASIO 专有件 / SSPL；LGPL 需单独评估（动态链接）。
- 落地手段：CI 接入 `cargo-deny`，`[licenses]` 段配置 allow-list，阻断传染性依赖进入。
- `unsafe` 例外：仅 `crates/host-vst3` 允许（Plan 1 约束已声明）；其余 crate 保持 `#![forbid(unsafe_code)]`。

## 4. 技术栈（修正后）

| 层 | 选择 | 状态 |
| --- | --- | --- |
| 应用外壳 | Tauri v2 | ✅ |
| 前端 | Vue 3 + Pinia + Vue Router + Tailwind（Material You） | ✅ |
| 音频 IO | `cpal`（Windows 走 WASAPI，**不用 ASIO**） | ✅ |
| 实时/离线渲染 | 自研 `Player` + `ScheduledEvents`（离线与实时逐位一致） | ✅ |
| 内置合成器 DSP | 手写正弦 + 线性 ADSR | ✅ |
| WAV 读写 | `hound` | ✅ |
| 插件 ABI | 自研 `Plugin` trait（就地平面缓冲） | ✅ |
| VST3 / CLAP 宿主 | **`truce-rack`** | ⬜ 待建 |
| MIDI 输入 | `midir` | ⬜ 待建 |
| 音频解码/重采样 | `symphonia` / `rubato` | ⬜ 待建 |
| 响度/频谱分析 | `ebur128` / `rustfft` | ⬜ 待建 |
| 插件商店 | `@open-audio-stack/core`（前端搜索）+ Rust 侧安装 | ⬜ 待建 |

## 5. 剩余路线

### Stage A — v1 收尾（P0）

- **A1 · `crates/ipc`**：把 `Command` / `Event` / `Project` 的 serde schema 抽成独立 crate，成为「引擎 ↔ Tauri ↔ 未来 MCP」的唯一契约。当前命令面在 `src-tauri/src/commands.rs` 里临时定义，正式化 `ipc` 是解锁后续一切接口的前置。
- **A2 · Plan 4b：原生实时传输接进 UI**
  - 后端：新增 `transport_play` / `transport_pause` / `transport_stop` / `transport_seek` 命令 + `engine://transport` 位置事件（位置从 `AudioHost` 的 `AtomicU64` 读取）。
  - 线程：`cpal::Stream` 在部分平台是 `!Send`，`AudioHost` 需移入专用音频线程，用 `mpsc` 与 Tauri 命令通信。
  - 前端：`usePlayback` 从「离线渲染 + `<audio>`」切到实时命令；离线渲染仅保留给导出与预览。
  - 验收：播放/暂停/停止/定位即时响应，无渲染等待。

### Stage B — 插件托管（P0，核心差异化）

- **B1 · `crates/host-vst3`**：基于 `truce-rack` 实现 扫描 + 加载 + 通用参数 + 状态存取；把 VST3 实例适配成既有 `Plugin` trait（**不替换 ABI**，它是本项目的资产）。
- **B2 · 注册表扩展**：`PluginRegistry` 支持外部插件（扫描结果 → 工厂）；缺失插件给占位与告警。
- **B3 · 前端**：插件浏览器 + 通用参数面板（扩展 `InspectorPanel.vue`）。
- **B4 · 测试**：用免费 VST3 插件 + VST3 官方 Validator 做集成测试。
- 验收：能扫描本机 VST3、挂到轨道、存进工程 JSON、重开还原。

### Stage C — 音频能力（P1）

- **C1 · Plan 6**：`AudioClip` 模型 + 采样器插件 + 波形显示。
- **C2 · Plan 5**：录音（cpal 输入流 → 写入 `AudioClip`）。
- **C3 · 混音完善**：`AutomationLane`、send / bus（设计文档 §6 已列，模型尚未实现）。

### Stage D — AI 层（P1，项目定位所在）

- **D1 · `crates/analysis`**：渲染 → LUFS / 频谱 / 峰值（用 `ebur128` / `rustfft`），构成 AI 的「听感」闭环。
- **D2 · `src-tauri` 内嵌 MCP 服务器**：把 transport / 编曲 / 插件参数 / 渲染导出 / analysis 暴露为工具；AI 与人类共用 `Engine::execute`（`actor = Agent(..)`），天然共享撤销与可视化。
- **D3 · skills 文档包 + 本地 HTTP/WS**（设计文档 §9）。
- **D4 · 前端 AI 可视化**：事件流已带 `actor`，UI 高亮被改动的轨道 / 音符 / 旋钮。

### Stage E — 生态与隔离（P2）

- **E1 · 插件商店**：前端用 `@open-audio-stack/core` 搜索/展示，下载安装放 Rust 侧。
- **E2 · 崩溃隔离**：第三方插件上量后，再把插件宿主拆成 helper 进程（**此时 sidecar 才有意义；音频引擎本身不必拆进程**）。

## 6. 里程碑

M1（Stage A 完成：实时传输可用）→ M2（Stage B：可托管 VST3）→ M3（Stage C：可录音、有音频片段）→ M4（Stage D：可被 AI 驱动）→ M5（Stage E：插件商店 + 崩溃隔离）。

## 7. 文档索引

- 设计意图：`specs/2026-10-07-ai-daw-design.md`
- 分计划：
  - `plans/2026-10-07-engine-core.md`（Plan 1，✅ 完成）
  - `plans/2026-10-07-realtime-playback.md`（Plan 2，✅ 完成）
  - `plans/2026-10-07-minimal-ui-shell.md`（Plan 4a，✅ 完成，实现栈已改为 Vue）
- 进度与后续计划的唯一索引：**本文件**。
