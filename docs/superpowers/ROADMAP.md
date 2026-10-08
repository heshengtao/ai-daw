# daw-party 总体计划（修正版）

- 更新：2026-10-08
- 定位：本文件是**项目进度与后续计划的唯一索引**。设计意图见 `specs/2026-10-07-ai-daw-design.md`；逐步实现步骤见 `plans/` 下的分计划。
- 背景：本文档修正了设计文档与分计划中已经与现实不符的若干陈述（见 §2），并给出剩余工作的统一路线（见 §5）。

## 1. 现状总览

| 编号 | 范围 | 状态 | 证据 / 说明 |
| --- | --- | --- | --- |
| Plan 1 | 无头引擎：工程模型 + 命令/事件 + 离线渲染 + WAV | ✅ 完成 | `crates/{plugin-api,synth,engine}`；`engine/tests/render_to_wav.rs` |
| Plan 2 | cpal 实时播放 + 传输控制 + 离线==实时 | ✅ 完成 | `engine/src/{schedule,player,transport,command_queue,realtime}.rs`；`examples/play_demo.rs` |
| Plan 4a | 最小桌面外壳：窗口 + 工程操作 + 试听 + 导出 | ✅ 完成（栈有变） | `src-tauri/src/commands.rs`；前端**已改用 Vue 3**（见 §2） |
| Plan 4b | 把原生实时传输接进 UI | ✅ 完成 | `src-tauri/src/audio.rs`（专用音频线程）+ `engine://transport` 事件 + `stores/transport.ts`；界面全部改为真实操作（见下） |
| Plan 3 | VST3 宿主 | ⬜ 未开始 | 无 `crates/host-vst3` |
| Plan 5 | 录音 | ⬜ 未开始 | — |
| Plan 6 | 音频片段 / 采样器 | ⬜ 未开始 | 模型只有 MIDI `Clip`，无 `AudioClip` |
| Plan 7 | AI 接口（MCP + skills + 可视化） | ⬜ 未开始 | 无 MCP；但事件流已带 `actor` 标签（`Engine::execute`） |
| 新增 | `crates/ipc`（Command/Event schema） | ⬜ 未开始 | 设计文档 §5 已列出，实际未建 |
| 新增 | `crates/analysis`（响度/频谱） | ⬜ 未开始 | 设计文档 §5 已列出，实际未建 |
| 新增 | 插件商店 | ⬜ 未开始 | — |

### Plan 4b 的实际交付（2026-10-08）

**实时传输链路**（`cpal::Stream` 是 `!Send`，因此宿主必须独占一条线程）：

- `crates/engine/src/realtime.rs`：新增 `SharedTransport`（跨线程共享位置/播放态）、
  `AudioHost::start_with`（复用共享状态重建宿主）、`AudioHost::send`、
  `default_output_sample_rate()`（不打开流即可探测设备采样率）。
- `src-tauri/src/audio.rs`：**专用音频线程**持有 `AudioHost`，经 `mpsc` 接收
  `Load`（装载/重载工程）与 `Command`（传输与混音）；控制侧通过原子量读取状态。
- 传输命令：`transport_play` / `transport_pause` / `transport_stop` / `transport_seek` /
  `transport_status`；位置经 `engine://transport` 以约 30 Hz 推送（**仅状态变化时**推送，
  暂停后完全静默）。
- 编辑 → 声音的两条路径（由事件类型决定，见 `Event::is_mix_only`）：
  - **混音类事件**（增益/声像/静音/独奏/总输出）→ 只推参数给音频线程，**不重建播放器**，
    因此拖动推子不会造成音频中断；
  - **其余事件** → 重建定帧事件表，并保留播放位置与播放状态。
- 前端 `stores/transport.ts`：统一 `realtime` / `preview` 两种模式；设备不可用时自动降级为
  「离线渲染 → `<audio>`」，位置由音频元素推进，界面无需分支。

**各视图的真实操作**（引擎命令面同步扩展，全部可撤销、可被 AI 复用）：

| 视图 | 可执行的操作 | 对应命令 |
| --- | --- | --- |
| 底部传输条 | 播放/暂停、停止、回到起点、位置读数、点击标尺定位；**播到结尾后再按播放会从头重播** | `transport_*`（重播策略在 `Player::apply_command`） |
| **共享时间轴** | 编排与编辑**同一条时间轴**：同一缩放、同一横向滚动位置、同一套由**拍号**推导的网格；标尺是同一个组件（`TimelineRuler.vue`）。点击/拖动标尺定位播放头（正在播放时从新位置继续）。`Ctrl/Cmd + 滚轮`缩放（锚点在指针下，两视图同步） | — |
| 编排 | **在空白处拖动画出等长片段**（框选模式下拖动 = 框选）、拖动移动（多选整组移动、**单个片段可跨轨道拖动**）、拖右边缘改长度、左键点选 / Ctrl 加选、**单击片段顺手打开右侧检查器**（方便立刻换音色）、**右键点击删除**、**右键拖动框选批量删除**、**双击进入钢琴卷帘** | `add_clip` / `move_clip` / `move_clips`（带 `to_track`）/ `resize_clip` / `remove_clip` / `remove_clips` |
| 编辑（钢琴卷帘） | **顶部有与编排同一条的标尺**；**在空白处拖动画出等长音符**、拖动改音高与起点（多选整组）、拖右边缘改时长、左键点选 / Ctrl 加选、右键点击删除、右键拖动框选批量删除；**左侧琴键列画成真正的键盘（白键 / 黑键 + C 音名）**，与可放音符的网格明显区分；进入片段时纵向自动对准音符；轨道没有片段时给出「新建 1 小节片段」入口 | `add_note` / `set_note` / `set_notes` / `remove_note` / `remove_notes` / `add_clip` |
| **控制器泳道**（卷帘下方） | 每个音符一根直角条：**横位置与宽度与上方音符一一对应**（绝对时间轴，片段有偏移也对齐），高度表示所选属性；属性是**音量 / 力度**（所有音源通用；不同插件将来会带来各自属性），上下拖动改值（多选整组、松手提交一次）、框选、右键删除；与卷帘**横向滚动同步**，且与卷帘之间的**分隔线可拖拽**（高度持久化） | `set_notes` / `remove_notes` |
| 混音台 | 推子改增益（相对拖动）、声像、静音、独奏、主输出增益 | `set_gain` / `set_pan` / `set_mute` / `set_solo` / `set_master_gain` |
| 检查器 | 轨道重命名、**音色用分类下拉菜单切换**（附「浏览全部音色…」模态框：搜索 / 分类筛选 / 排序）、乐器参数、混音参数 | `rename_track` / `set_track_instrument` / `set_param` |
| 音轨栏 | 选中、静音、独奏、删除；**在钢琴卷帘界面点它会跳到该轨道「同一时刻」的片段，没有片段就回编排** | 同上 |
| 插件面板 | 按分类列出插件，把插件加载到当前选中轨道 | `set_track_instrument` |

**鼠标模式（顶栏切换 / 快捷键 `1` 拖拽、`2` 框选）**：解决「左键拖动空白 = 创建」与
「左键拖动 = 框选」的手势冲突。模式决定左键的默认含义，`Shift` / `Alt` 随时可临时切到
框选；右键拖动始终是框选删除。

**网格由拍号推导**（`stores/timeline.ts`）：`meterBeat = 4 / 分母`（拍号里那一「拍」有多长），
`barBeats = 分子 × meterBeat`（一小节多长：4/4 → 4、3/4 → 3、6/8 → 3、2/2 → 4），
`snap = meterBeat / 4`（吸附网格：4/4 → 十六分、6/8 → 三十二分）。**切换拍号会同时改变
编排与编辑两个视图的网格格子与吸附精度**。细分网格密于 10px 时自动不画，避免糊成一片密纹。

**新工程的默认分轨**（`STARTER_TRACKS`，引擎常量，数组顺序即轨道顺序）：

| # | 轨道 | 角色 | 默认音色（内置合成器预设） |
| --- | --- | --- | --- |
| 1 | 鼓组 | 节奏骨架 | 拨弦 Pluck（最短促的包络） |
| 2 | 低音 | 根音与低频 | 低音 Bass |
| 3 | 和弦 | 和声铺底 | 铺底 Pad |
| 4 | 主旋律 | 主线条 | 主音 Lead |
| 5 | 铺底 | 织体 / 氛围 | 铺底 Pad |

选的是流行与电子编曲里最通用的五个**功能位**（节奏 → 低频 → 和声 → 主旋律 → 色彩），
避免「节奏」与「鼓组」这类职责重叠的命名。鼓组专用采样器属 Stage C，接入后只需改 `plugin_id`。
`Engine::new_project`（应用启动时的初始状态）不写历史，用户主动点「新建工程」走命令、可撤销。

**内置音色**：`builtin.synth` 是合成器本体，另有 4 个**预设音色**（`builtin.synth.lead` /
`.pluck` / `.bass` / `.pad`）——同一套 DSP 配不同默认包络，各自是独立插件 id
（`crates/engine/src/registry.rs` 的 `BUILTIN_PRESETS`），所以音色选择器里就是一条条真音色。

**音符的两个「响度类」属性**：`velocity` **力度**（演奏强度，插件各自解释；内置合成器折算成振幅）
与 `volume` **音量**（单音电平微调，不改音色，默认 127 = 不衰减）。插件 ABI 的 `NoteOn` 只有
一个力度字段，所以两者在**排程时**合成（`Note::midi_velocity`）；旧工程缺 `volume` 字段时按
127 读入，听感完全不变。

**批量操作与撤销**：框选删除、整组拖动、跨轨道拖动、泳道批量改值都各自只产生**一条**
撤销记录（引擎侧 `RemoveNotes` / `SetNotes` / `RemoveClips` / `MoveClips` 四条批量命令；
`RemoveNotes` 内部倒序删除并去重、越界保护；`ClipsMoved` 先校验目标轨道存在，非法目标整条跳过）。

**已修复的隐患**：`Md3Slider` 的 `commit` 事件此前报的是 `props.modelValue`——在父组件
用「草稿 + 只监听 commit」的模式下那是**尚未更新的旧值**，会让拖完松手等于没改。现在
`commit` 报的是本次交互实际选中的值（混音台声像、检查器增益/声像、音符力度都受益）。

**主题在挂载前应用**（`main.ts` 调一次 `useTheme()`）：此前主题只在设置页挂载时才写入
`<html>`，选了深色后重新打开应用会退回浅色。

**实时安全**：`Player` 新增实时混音（`TransportCommand::SetTrackMix` / `SetMasterGain`），
`refresh_mix` 只做算术与字段写入；`tests/realtime_safety.rs` 已把混音命令纳入零分配断言。
总输出增益现在真正生效（`ScheduledEvents` 携带 `master`，离线渲染与实时播放一致）。

**已知取舍**：结构性编辑会重建播放器（重新打开设备流），播放中做此类编辑会有极短的
音频间断；混音参数不受影响。参数改动同样走重建，因此拖动参数滑块只在松手时生效一次。


**已具备、可直接复用的能力**：确定性离线渲染（同一工程逐位一致）、cpal 实时宿主（RT-safe，回调内不分配内存）、**专用音频线程 + 共享传输状态**、**实时混音（推子/声像/静音/独奏/总输出不重建事件表）**、无锁 SPSC 传输命令队列、事件溯源 + 撤销、统一插件 ABI（`Plugin` trait）、内置合成器、工程 JSON 往返、Tauri 命令层 + 引擎事件广播、Vue 外壳（顶栏 / 传输条 / 轨道列表 / 标签页面板 / 主题 / 选中态）。

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
- **A2 · Plan 4b：原生实时传输接进 UI** ✅ 完成（2026-10-08）
  - 后端：`transport_play` / `transport_pause` / `transport_stop` / `transport_seek` / `transport_status` + `engine://transport` 位置事件（位置从音频线程的 `SharedTransport` 读取）。
  - 线程：`AudioHost` 移入专用音频线程（`src-tauri/src/audio.rs`），用 `mpsc` 与命令层通信。
  - 前端：`stores/transport.ts` 走实时命令；离线渲染仅保留给导出与「设备不可用时的回退试听」。
  - 验收：播放/暂停/停止/定位即时响应，无渲染等待 ✅（浏览器预览为 mock 模拟，真实设备行为需在桌面端验证）。

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

**M1（Stage A 完成：实时传输可用）**：A2 已完成 ✅；A1（`crates/ipc` 契约抽离）仍未开始——
在 A1 完成前，命令面仍定义在 `src-tauri/src/commands.rs`，尚未成为「引擎 ↔ Tauri ↔ 未来 MCP」
的唯一契约。

M2（Stage B：可托管 VST3）→ M3（Stage C：可录音、有音频片段）→ M4（Stage D：可被 AI 驱动）→ M5（Stage E：插件商店 + 崩溃隔离）。

## 7. 文档索引

- 设计意图：`specs/2026-10-07-ai-daw-design.md`
- 分计划：
  - `plans/2026-10-07-engine-core.md`（Plan 1，✅ 完成）
  - `plans/2026-10-07-realtime-playback.md`（Plan 2，✅ 完成）
  - `plans/2026-10-07-minimal-ui-shell.md`（Plan 4a，✅ 完成，实现栈已改为 Vue）
- 进度与后续计划的唯一索引：**本文件**。
