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
| Plan 3 | VST3 / CLAP 宿主 | ✅ 完成（栈有变） | `crates/host-vst3`（VST3 用 `vst3-host`、CLAP 用 `clack-host`，**未用 truce-rack**）；`src-tauri/src/plugins.rs` 装配注册表 + 原生窗口 |
| Plan 5 | 录音 | ⬜ 未开始 | — |
| Plan 6 | 音频片段 / 采样器 | 🟡 引擎侧完成 | `crates/asset` + `Clip.audio` + 播放路径 + 7 条量化测试；宿主接线与前端未做 |
| Plan 7 | AI 接口（MCP + skills + 可视化） | 🟡 进行中 | MCP ✅（`src-tauri/src/mcp.rs`，27 工具）+ 前端 AI 可视化 ✅（D4）；skills 文档包（D3）未做 |
| 新增 | `crates/ipc`（Command/Event schema） | ✅ 完成 | `crates/ipc`（`daw-party-ipc`）：工程/命令/事件 schema 的**唯一来源**；engine 用 `pub use` 薄壳保持既有路径不变（零行为变化） |
| 新增 | `crates/analysis`（响度/频谱） | ✅ 完成 | `crates/analysis`（ebur128 + rustfft）：LUFS/LRA/真峰值 + RMS + 平均幅度谱；`analyze_project` 命令 |
| 新增 | 插件商店 | ⬜ 未开始 | — |
| 品牌 | 应用图标 / favicon | ✅ 完成 | `src-tauri/app-icon.svg`（薰衣草紫，与 `common/Logo.vue` 同图形）→ `tauri icon` 生成 `src-tauri/icons/*`；`public/logo.svg` 作网页 favicon |

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
| VST3 宿主实装 | （承上）预期用 truce-rack | **实际改用 `vst3-host` + `clack-host`**（见 `crates/host-vst3/Cargo.toml` 注释）：truce-rack 的 VST3 后端托管不了「无音频输入总线的音源」，其 CLAP 后端加载 KR-106 会段错误。两者同为宽松许可，不破坏 Apache-2.0 分发。 |
| 低延迟（Windows） | （外部建议）启用 cpal 的 ASIO 后端 | **不启用 ASIO**：ASIO SDK 为 GPLv3 / 专有，会破坏纯 Apache-2.0 分发。走 WASAPI |
| AI 听感引擎 | （外部建议）Dhwani | **不用**：Dhwani 是 **GPL-3.0-only**，链接即传染。改用 `ebur128` / `rustfft` |
| 音频解码 | `symphonia` / `rubato`（拟定） | **symphonia 已采用**（wav/flac/mp3/ogg/aac/m4a）。它是 **MPL-2.0**，原先不在允许清单里 → 2026-10-09 把 MPL-2.0 加入 §3（**仅限未修改依赖**，见 `NOTICE`/`deny.toml`）。`rubato`（保音高变速）暂未引入。 |
| 音频片段时间语义 | （未指定） | **片段拍长是唯一真相，素材铺满跨度**：缩放/改速度按比例重采样 → **会变调**（v1 不做保音高变速）。理由：模型与 MIDI 片段一致、时间轴/吸附/长度全部复用、实现最简。 |
| 插件宿主（外部建议首选） | UAPMD | 仅作 **MCP / 能力参考**：UAPMD 是 **C++/CMake** 工程，接入需 FFI，与引擎 `#![forbid(unsafe_code)]` 相悖 |

## 3. 许可证护栏（强制）

- 本项目许可：**Apache-2.0-only**。
- 允许的依赖许可：MIT / Apache-2.0 / BSD-2-Clause / BSD-3-Clause / ISC / CC0 / Zlib / Unlicense
  / **MPL-2.0（仅限「未修改的依赖」，2026-10-09 加入）**。
- **禁止**：GPL / AGPL / ASIO 专有件 / SSPL；LGPL 需单独评估（动态链接）。
- MPL-2.0 是**文件级**弱著佐权：把**未修改**的 MPL 库当依赖链接进 Apache-2.0 程序不会传染本仓库
  代码，义务只有「保留其许可声明」+「若修改其源码则须公开该文件的修改」。首个（也是当前唯一）
  用例是 `symphonia`（音频解码，见 `crates/asset` 与 `NOTICE`）。
- 落地手段：仓库根已落 `deny.toml`（`cargo deny check licenses`，但本机/CI 尚未执行）；CI 接入待办。
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
| VST3 / CLAP 宿主 | **`vst3-host`** + **`clack-host`**（见 §2） | ✅ |
| MIDI 输入 | `midir` | ⬜ 待建 |
| 音频解码/重采样 | `symphonia` / `rubato` | ⬜ 待建 |
| 响度/频谱分析 | `ebur128` / `rustfft` | ✅ |
| 插件商店 | `@open-audio-stack/core`（前端搜索）+ Rust 侧安装 | ⬜ 待建 |

## 5. 剩余路线

### Stage A — v1 收尾（P0）

- **A1 · `crates/ipc`**：✅ 完成（2026-10-09）。`crates/ipc`（`daw-party-ipc`）只依赖 `serde`，
  持有 `model/*` / `command.rs` / `event.rs`（含 `Actor`、`Event::actor()`、`Event::is_mix_only()` 与全部
  `normalize_*`）；`daw-party-engine` 用 `pub use` 薄壳再导出，**既有路径与行为零变化**
  （测试通过数守恒：engine 106→90、新增 ipc 17、`--features realtime` 123→107）。
  设计/计划：`specs/2026-10-09-ipc-contract-design.md`、`plans/2026-10-09-ipc-contract.md`。
  顺带落 `deny.toml` 并把 MPL-2.0（仅限未修改依赖）加入 §3 允许清单。
- **A2 · Plan 4b：原生实时传输接进 UI** ✅ 完成（2026-10-08）
  - 后端：`transport_play` / `transport_pause` / `transport_stop` / `transport_seek` / `transport_status` + `engine://transport` 位置事件（位置从音频线程的 `SharedTransport` 读取）。
  - 线程：`AudioHost` 移入专用音频线程（`src-tauri/src/audio.rs`），用 `mpsc` 与命令层通信。
  - 前端：`stores/transport.ts` 走实时命令；离线渲染仅保留给导出与「设备不可用时的回退试听」。
  - 验收：播放/暂停/停止/定位即时响应，无渲染等待 ✅（浏览器预览为 mock 模拟，真实设备行为需在桌面端验证）。

### Stage B — 插件托管（P0，核心差异化）— ✅ 基本完成（2026-10-09 核对）

- **B1 · `crates/host-vst3`**：✅ 已实现。VST3 后端用 `vst3-host`（能托管无输入总线的音源）、CLAP 后端用 `clack-host`（truce-rack 的 VST3/CLAP 均不可用，见 §2 修正记录）；两者都适配到既有 `Plugin` trait（未替换 ABI）。含扫描（磁盘缓存）、加载、参数、状态、原生窗口（`editor.rs`）、主线程派发（`dispatch.rs`）。
- **B2 · 注册表扩展**：✅ 已实现（`src-tauri/src/plugins.rs`）。`build_registry` 装配外部插件工厂；缺失插件给 `missing_plugin` 静音占位；工程引用但本机没有的插件也会 `register_missing`。
- **B3 · 前端**：✅ 已实现。插件清单 / 通用参数面板 / 效果链 / 原生界面窗口（`InspectorPanel`、`PluginsPane`、`data/plugins.ts`、`usePluginInterface`）。
- **B4 · 测试**：🟡 部分。`crates/host-vst3/tests/kr106.rs` + `src-tauri/src/plugins.rs::renders_a_track_hosting_the_clap_kr106`（真 KR-106，未安装则跳过）。**尚未接入 VST3 官方 Validator**，也未覆盖更多免费插件。
- 验收：能扫描本机 VST3/CLAP、挂到轨道、存进工程 JSON、重开还原 ✅。

### Stage C — 音频能力（P1）

- **C1 · Plan 6**：`AudioClip` 模型 + 采样器 + 波形显示。🟡 **引擎侧已完成（2026-10-09）**：
  `crates/asset`（symphonia 解码 + `peaks`）、`Clip.audio`/`AudioSource`、`AddAudioClip`/`SetClipAudio` 与对应事件、
  `SampleProvider` + `ScheduledAudioClip` + `Player` 混音路径（含 `seek`/暂停语义）、`crates/engine/tests/audio_clip.rs`
  的 7 条**量化**断言。**待办**：宿主 `AssetCache`、`import_audio`/`audio_waveform` 命令 + MCP 工具、前端波形与导入入口。
  设计与计划：`specs/2026-10-09-audio-clip-design.md`、`plans/2026-10-09-audio-clip.md`。
  语义决策：**片段拍长是唯一真相，素材铺满跨度 → 缩放/改速度会变调**（v1 不做保音高变速）。
- **C2 · Plan 5**：录音（cpal 输入流 → 写入 `AudioClip`）。
- **C3 · 混音完善**：`AutomationLane`、send / bus（设计文档 §6 已列，模型尚未实现）。

### Stage D — AI 层（P1，项目定位所在）

- **D1 · `crates/analysis`**：✅ 完成。渲染 → 响度（EBU R128：LUFS / LRA / 采样峰值 / 真峰值）+ RMS + 平均幅度谱（Hann 窗 + 50% 交叠 FFT）。对上层是 `analyze_project` IPC 命令；这是 AI 的「听感」闭环。
- **D2 · `src-tauri` 内嵌 MCP 服务器**：✅ 完成。`src-tauri/src/mcp.rs`（`rmcp` 3，Streamable HTTP，绑 `127.0.0.1` 临时端口，启动时把 `http://127.0.0.1:<port>/mcp` 打到 stderr）。27 个工具：`get_project` / `get_transport` / `list_plugins` / `transport_*` / `set_tempo` / `set_time_signature` / `add_track` / `remove_track` / `rename_track` / `set_track_instrument` / `set_param` / `add_clip` / `remove_clip` / `add_note` / `set_gain` / `set_pan` / `set_mute` / `set_solo` / `set_master_gain` / `undo` / `redo` / `analyze_project` / `export_wav`。写操作全部走 `commands::run_agent`（`actor = Agent("mcp")`），与界面**同一条**管线：撤销历史、`engine://event` 事件流、音频同步、结构变更关插件窗口。
  - **配置**：设置页「AI / MCP」一节可开关、改端口（0 = 临时端口），即时启停；配置落在应用配置目录的 `mcp.json`。
  - 安全：只绑回环；rmcp 默认只放行回环 `Host`（防 DNS rebinding）。**当前不鉴权**——同机任意进程都能连，后续可加 token。
- **D3 · skills 文档包 + 本地 HTTP/WS**（设计文档 §9）：⬜ 未开始。
- **D4 · 前端 AI 可视化**：✅ 完成。`stores/engineEvents.ts` 解析事件里的 `Actor`，把**代理**发起的变更单独记账（代理名、事件类型、涉及轨道）；底栏胶囊只在代理动手时点亮并显示代理名（tooltip 给出最近改动处数与轨道），音轨栏对被代理改动的行短暂高亮（900ms 胶囊窗 / 1600ms 轨道窗）。人类自己的编辑不再误点亮 AI 指示。浏览器预览的 mock 会轮播代理事件，便于无后端演示。

### Stage E — 生态与隔离（P2）

- **E1 · 插件商店**：前端用 `@open-audio-stack/core` 搜索/展示，下载安装放 Rust 侧。
- **E2 · 崩溃隔离**：第三方插件上量后，再把插件宿主拆成 helper 进程（**此时 sidecar 才有意义；音频引擎本身不必拆进程**）。

## 6. 里程碑

**M1（Stage A 完成：契约单一化 + 实时传输可用）**：✅ 达成。A2 已完成；A1 于 2026-10-09 完成 ——
`Command` / `Event` / `Project` 的 schema 现由 `crates/ipc` 单点持有，引擎 ↔ Tauri ↔ MCP 共用同一份契约。

**M2（Stage B：可托管 VST3/CLAP）**：✅ 基本达成（B1–B3 完成，B4 部分；见 Stage B）。

M3（Stage C：可录音、有音频片段）→ M4（Stage D：可被 AI 驱动）→ M5（Stage E：插件商店 + 崩溃隔离）。

## 7. 文档索引

- 设计意图：`specs/2026-10-07-ai-daw-design.md`
- 分计划：
  - `plans/2026-10-07-engine-core.md`（Plan 1，✅ 完成）
  - `plans/2026-10-07-realtime-playback.md`（Plan 2，✅ 完成）
  - `plans/2026-10-07-minimal-ui-shell.md`（Plan 4a，✅ 完成，实现栈已改为 Vue）
- 进度与后续计划的唯一索引：**本文件**。
