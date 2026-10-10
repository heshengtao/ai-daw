# 开源零件调研：录音 / 混音（自动化·总线·发送）/ MIDI 输入 / 插件商店

- 日期：2026-10-10
- 用途：为 Roadmap 的 **C2 录音**、**C3 混音完善**、**MIDI 输入**、**E1 插件商店**提前锁定「可直接用的库」与「只能参考的项目」，开工时不必再查。
- 重要前提：本仓库是**纯 Apache-2.0 分发**（`deny.toml` 护栏，见 ROADMAP §3）。因此：
  - **可直接依赖**：MIT / Apache-2.0 / BSD / ISC / CC0 等宽松许可。
  - **只能读思路、禁止抄代码**：GPL / AGPL 项目（参考实现）。
  - 下列许可为 2026-10 核对，**引入前用 `cargo deny check licenses` 再核一遍**，并同步 `NOTICE`。

---

## 1. MIDI 输入（Roadmap：`midir`，⬜ 未开始）

**结论：有现成 crate，可直接用。**

| 库 | 许可 | 用途 |
| --- | --- | --- |
| `midir` | MIT | 跨平台实时 MIDI 输入/输出（CoreMIDI / WinMM / ALSA），多端口枚举 + 输入连接 + 回调。设计文档 §3 点名的就是它。 |
| `wmidi` | MIT | MIDI 消息解析（可选；也可直接手解析 `midir` 回调里的原始字节）。 |

**接入方式（与现有架构对齐）**
- 起一条 MIDI 线程 `midir::MidiInput::connect`，回调里把消息转成引擎自己的 `MidiEvent`。
- 走**已有的无锁命令队列**（`crates/engine/src/command_queue.rs`，底层 `rtrb`）投递给实时线程——与传输命令同一条通道。
- 录音臂（arm）、输入延迟补偿属产品逻辑，自己写；**不需要**「DAW 级 MIDI 引擎」crate。

---

## 2. 录音（Roadmap：C2 / Plan 5，⬜ 未开始）

**结论：没有 drop-in 的「录音器」crate，但底层零件齐全，自己写（几百行量级）。**

| 库 | 许可 | 用途 |
| --- | --- | --- |
| `cpal` | MIT / Apache-2.0 | `build_input_stream` 采集输入；**已是本项目依赖**（`realtime` feature）。官方 `examples/record_wav.rs` 可照抄结构。 |
| `rtrb` | MIT / Apache-2.0 | 无锁 SPSC ring buffer：录音回调推帧、消费线程取帧。**已在用**（命令队列）。 |
| `rubato` | MIT | 设备采样率 ≠ 工程采样率时的重采样；也正是「保音高变速」未来要用的同一个库。 |
| `hound` | ISC | 需要落盘时写 WAV（或复用 `crates/asset`）。 |

**落地路径**
- 仿 `crates/engine/src/realtime.rs` 的**输出**流写法，加一条**输入**流；回调内零分配（沿用 `tests/realtime_safety.rs` 的门禁思路）。
- 停止录音 → 把缓冲封成 `AudioClip`（模型已在 C1 落地：`Clip.audio`）挂到轨道上。
- 录音臂 / 输入设备选择 / 监听 / 延迟补偿：产品逻辑，无库。

---

## 3. 混音完善（Roadmap：C3，⬜ 模型未实现）

**结论：没有任何 crate 提供 AutomationLane / Bus / send —— 这是 DAW 领域模型，自己设计。**

**落点（复用现有引擎结构）**
- 直接做进 `ScheduledEvents` + `Player::refresh_mix`（**每块求值**）：
  - **send** = 轨道信号按增益抽一份进目标总线（aux 发送）。
  - **bus** = 接收其他轨道的轨道，最终汇总到 master（支持层级）。
  - **automation** = 关键点曲线（沿用 `TempoMap` 的分段思路，加线性/指数段），逐块插值写进目标参数。
- 现有 `refresh_mix` 已做实时混音（推子/声像/静音/独奏），自动化天然挂在这一层。

**可直接用的宽松许可零件**

| 库 | 许可 | 用途 |
| --- | --- | --- |
| `nih-plug` | ISC | 参数平滑 / ramp（`SmoothedParam`），自动化读数的成熟做法，可借鉴/复用。 |
| `fundsp` | MIT | 把总线/发送做成 DSP 图时用于求和与组合。 |
| `rtrb` | MIT / Apache-2.0 | 无锁传递（已在用）。 |
| `basedrop` | MIT（引入前复核） | 实时线程的安全 GC（可选）。 |

**只能参考思路的开源 DAW（copyleft，勿抄码）**

| 项目 | 许可 | 参考点 |
| --- | --- | --- |
| Ardour | GPL-2.0+ | automation lane 模型：lane 依附轨道、一条 lane 控一个参数、curve 为关键点插值。 |
| Zrythm | AGPL-3.0 | bus / send 的图结构。 |
| LMMS / Qtractor | GPL | 轨道/总线组织的通用做法。 |

---

## 4. 插件商店（Roadmap：E1，⬜ 未开始）

**结论：`@open-audio-stack/core` 可用，且最契合；另可叠加 PluginDB 做发现。**

| 项目 | 许可 | 能力 | 备注 |
| --- | --- | --- | --- |
| **Open Audio Stack**（`open-audio-stack-registry`） | 数据 CC0-1.0 | 插件/预设/工程开放注册表；**静态 JSON API**，每个文件带 **url + sha256 + size** | 只收**免费开源、跨平台**包（一旦被下架/移除由维护者决定）；前端 `@open-audio-stack/core` 即其客户端/类型包。**只提供注册表与规范**，下载/校验/解压/安装仍需自己在 Rust 侧写（正是 E1「前端搜索 + Rust 侧安装」）。早期项目（~67 star）但数据开放、格式稳定。 |
| **PluginDB**（`blakebratcher/plugindb`） | 代码 MIT，数据 CC0 | FastAPI + SQLite 的 REST API，「插件的 MusicBrainz」，覆盖 VST/AU/CLAP，**含商业插件** | 适合发现/元数据/相似推荐，**不提供可下载二进制**，不能当安装源；非常新（~1 star）。 |

**建议**：安装型商店用 **Open Audio Stack**；将来想做「认识全网插件（含商业）」的搜索/推荐，再叠加 **PluginDB** 补充，两者不冲突。

---

## 5. 一句话速查

| 需求 | 直接用 | 自己写 | 只参考 |
| --- | --- | --- | --- |
| MIDI 输入 | `midir`(MIT) + `wmidi`(MIT) | 录音臂/延迟补偿 | — |
| 录音 | `cpal`(MIT/Apache) + `rtrb` + `rubato`(MIT) + `hound`(ISC) | 采集→`AudioClip` 流程 | — |
| 混音/自动化/总线/发送 | `nih-plug`(ISC) + `fundsp`(MIT) + `rtrb` | 领域模型与逐块求值 | Ardour(GPL)、Zrythm(AGPL)、LMMS(GPL) |
| 插件商店 | `@open-audio-stack/core` + Open Audio Stack 注册表(CC0) | Rust 侧下载/校验/安装 | PluginDB 作发现层补充 |

---

## 6. 来源

- [midir — Lib.rs](https://lib.rs/crates/midir)
- [midir (Boddlnagg/midir)](https://github.com/Boddlnagg/midir)
- [cpal — RustAudio/cpal](https://github.com/RustAudio/cpal)
- [cpal: Recording Audio (DeepWiki)](https://deepwiki.com/RustAudio/cpal/5.2-recording-audio)
- [Ardour — Automation Lanes](https://brucekomike.github.io/ardour-manual-sphinx-preview/automation-lanes.html)
- [Zrythm](https://github.com/zrythm/zrythm)
- [nice-plug (nih-plug fork)](https://codeberg.org/RustAudio/nice-plug)
- [fundsp (SamiPerttu/fundsp)](https://github.com/SamiPerttu/fundsp)
- [Open Audio Stack — Registry](https://github.com/open-audio-stack/open-audio-stack-registry)
- [PluginDB](https://github.com/blakebratcher/plugindb)
