# 音频片段（C1）Implementation Plan

> **状态（2026-10-09）**：🟡 进行中 —— Task 1–4（引擎侧：解码、模型、命令/事件、排程与播放）✅ 完成并量化验证；
> Task 5–7（宿主接线、Tauri/MCP、前端）⬜ 未开始。
>
> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans。

**Goal：** 让音频片段进模型、进时间轴、能出声、能被撤销，并能显示波形。

**Architecture：** 解码隔离在 `daw-party-asset`（symphonia）；引擎新增「素材铺满片段跨度」的播放路径
（`SampleProvider` → `ScheduledAudioClip` → `TrackVoice.audio`）；宿主用 `AssetCache` 把路径变成
`Arc<DecodedAudio>`；契约改动全部落在 `crates/ipc`。

**Tech Stack：** Rust edition 2021、serde、symphonia（MPL-2.0）、`daw-party-analysis`（仅测试，做量化断言）。

## Global Constraints

- 许可 Apache-2.0-only；允许清单见 ROADMAP §3（含 MPL-2.0，仅限未修改依赖）。
- `#![forbid(unsafe_code)]`（`crates/asset` 同样）。
- **实时回调零分配**：`crates/engine/tests/realtime_safety.rs` 是硬门禁。
- 新增模型字段一律 `#[serde(default)]`，且**旧字段输出形状不变**。
- 每个 Task 末尾：`cargo test --workspace` 全绿；涉及 src-tauri 另跑 `cargo check --manifest-path src-tauri/Cargo.toml`；
  涉及前端跑 `npm run build` 并保证中英键数一致。
- 提交结尾 `Co-authored-by: CommandCodeBot <noreply@commandcode.ai>`。

## 文件结构

```
crates/asset/                    新增：symphonia 解码 + peaks
crates/ipc/src/model/clip.rs     AudioSource + Clip.audio + is_audio/normalize
crates/ipc/src/{command,event}.rs AddAudioClip / SetClipAudio / AudioClipAdded / ClipAudioChanged
crates/engine/src/audio.rs       DecodedAudio + SampleProvider
crates/engine/src/schedule.rs    ScheduledAudioClip + from_project_with_assets
crates/engine/src/player.rs      AudioPlayback + 混音 + seek + Option<plugin>
crates/engine/src/render.rs      render_with_assets
crates/engine/tests/audio_clip.rs 新增：7 条量化断言
src-tauri/src/assets.rs          新增：AssetCache（任务 5）
src-tauri/src/{commands,state,audio,mcp}.rs  接线（任务 5–6）
src/api/*、src/views/*、src/i18n/*            前端（任务 7）
```

## Task 1: `crates/asset`（解码 + 包络）

**Files:** Create `crates/asset/{Cargo.toml,src/lib.rs}`；Modify 根 `Cargo.toml`。

- [x] **Step 1**：`symphonia = { version = "0.6", features = ["all"] }`、`thiserror`；dev-dep `hound`/`tempfile`。
- [x] **Step 2**：`decode()` —— probe → `make_audio_decoder` → 逐包解码（`copy_to_vec_interleaved` 是**覆盖**语义，
      必须每包拷进临时缓冲再追加）→ 声道收敛 1/2。
- [x] **Step 3**：`peaks(audio, buckets)` 每桶 min/max；空输入返回全 0。
- [x] **Step 4**：测试：单/立体声 WAV 的帧数/采样率/峰值；`peaks` 每桶包住 ±峰值；缺失与非法文件报错。

## Task 2: 模型（`crates/ipc`）

- [x] **Step 1**：`AudioSource { path, offset_frames }`；`Clip.audio: Option<AudioSource>`（`skip_serializing_if`）。
- [x] **Step 2**：`Clip::is_audio()` / `Clip::normalize`（音频片段清空音符）。
- [x] **Step 3**：测试：MIDI 片段不含 `audio` 键；旧 JSON 可读；音频片段往返 + normalize；`offset_frames` 缺省为 0。
- [x] **Step 4**：修全部 `Clip { … }` 构造点（编译器穷举）。

## Task 3: 命令 / 事件 / apply

- [x] **Step 1**：`Command::{AddAudioClip, SetClipAudio}`；`Event::{AudioClipAdded, ClipAudioChanged}` + `actor()` 分支。
- [x] **Step 2**：`bus.rs::translate`（归一化 + `alloc_clip_id`）；`apply.rs` 两条分支。
- [x] **Step 3**：测试：`AddAudioClip` 归一化 + 可撤销/重做；`SetClipAudio` 音频↔MIDI 切换且清空音符。

## Task 4: 排程与播放（核心）

- [x] **Step 1**：`DecodedAudio` + `SampleProvider`（`no_assets()`）。
- [x] **Step 2**：`ScheduledAudioClip` + `from_project_with_assets`；`from_project` 委托。
- [x] **Step 3**：`TrackVoice.plugin: Option<_>`（无乐器轨道仍要出声）+ `audio: Vec<AudioPlayback>`。
- [x] **Step 4**：`process_block` 在「乐器之后、效果链之前」相加；`playing` 时才混；线性插值 + `f64` 游标。
- [x] **Step 5**：`seek` 重算游标；`render_with_assets`。
- [x] **Step 6**：`crates/engine/tests/audio_clip.rs` 7 条量化断言全部通过；零分配门禁仍通过。

## Task 5: 宿主素材缓存（⬜ 未开始）

**Files:** Create `src-tauri/src/assets.rs`；Modify `src-tauri/src/{lib,state,audio,commands}.rs`、`src-tauri/Cargo.toml`。

- [ ] **Step 1**：`AssetCache { map: Mutex<HashMap<String, Arc<DecodedAudio>>> }`，`load()` 带缓存（失败也缓存，避免反复重试）、`peaks()`。
- [ ] **Step 2**：`AppState.assets`；`start_host` / `render_offline` 改用 `from_project_with_assets`/`render_with_assets`
      （锁内 clone 出素材表快照，**不带锁过 await**）。
- [ ] **Step 3**：缺失素材 → 静音，并把「缺失」告知界面（沿用既有 `error` 通道的同款做法）。

## Task 6: Tauri 命令 + MCP（⬜ 未开始）

- [ ] **Step 1**：`import_audio(path, track, start)`（解码 → 按当前速度把自然时长换算成拍 → `AddAudioClip`）、
      `audio_waveform(path, buckets)`；两者解码都在 `spawn_blocking` 里。
- [ ] **Step 2**：注册进 `generate_handler!`；MCP 增 `import_audio`/`add_audio_clip`/`audio_waveform` 并更新工具表测试。

## Task 7: 前端（⬜ 未开始）

- [ ] **Step 1**：`types.ts` 的 `Clip.audio?`；`importAudio`/`audioWaveform` 两条命令 + mock 假数据。
- [ ] **Step 2**：`ClipWaveform.vue`（canvas，照抄 `Oscilloscope.vue` 的 dpr/取色/ResizeObserver 骨架）。
- [ ] **Step 3**：`ArrangeView` 用波形取代 MIDI 迷你卷帘；`TopBar` 加「导入音频…」；i18n 中英。

## 自检 / Self-Review

- **Spec coverage**：对照 `2026-10-09-audio-clip-design.md` §7 的 5 条 —— 1/2/3/4 已由测试覆盖并通过；
  5（零分配）已通过。§7 是后端验收，前端视觉部分在 Task 7 后另行人工确认。
- **Placeholder 扫描**：无 TODO / 未实现分支；临时缓冲复用而非每次分配。
- **类型一致性**：`AudioSource`（ipc）与前端 `Clip.audio` 字段同名同形；`DecodedAudio` 只定义在 engine，
  `asset` 负责产出。
- **偏差**：`crates/asset` 依赖 `daw-party-engine`（只为复用 `DecodedAudio` 类型）。计划原意是「类型放 engine、
  asset 只产出」，方向一致；代价是 asset 会把 engine 拉进依赖图，可接受（两者本来同在依赖链上）。

## 执行记录（随进度更新）

### Task 1–4 完成（引擎侧，2026-10-09）

| 任务 | 内容 |
| --- | --- |
| 1 | `crates/asset`：symphonia 解码（wav/flac/mp3/ogg/aac/m4a）+ `peaks` |
| 2 | `AudioSource` / `Clip.audio` / `is_audio` / `normalize`；全部构造点补齐 |
| 3 | `AddAudioClip` / `SetClipAudio` / `AudioClipAdded` / `ClipAudioChanged` + translate + apply |
| 4 | `DecodedAudio` / `SampleProvider` / `ScheduledAudioClip` / `Option<plugin>` / 混音 / seek / `render_with_assets` |

**验证**：
- `cargo test --workspace` → **154 通过**（改动前 146）。
- `cargo test -p daw-party-engine --features realtime` → 全绿；`--test realtime_safety` 的
  **零分配门禁仍然通过**（这是音频路径进实时回调的硬条件）。
- `cargo check --manifest-path src-tauri/Cargo.toml` → 通过。
- `crates/engine/tests/audio_clip.rs` 7 条量化断言全通过：1 kHz 素材位置/音准、起点前严格静音、
  跨度减半→2 kHz（铺满语义）、`seek` 用阶梯素材精确对齐、暂停静音、素材缺失静音且不改长度、
  与乐器相加。

**必须知道的坑**：
- `symphonia` 0.6 的 `GenericAudioBufferRef::copy_to_vec_interleaved` 是**覆盖**语义（会把目标 resize），
  逐包解码必须自己追加，否则只能拿到最后一个包。
- 0.6 的 `Packet.track_id` 是**字段**不是方法；`FormatReader::next_packet` 返回 `Result<Option<Packet>>`。

**已知遗留（不阻塞，但需拍板）**：
- **整链 ×0.5（−6 dB）**：居中的轨道与居中的 master **各**套一次等功率声像。这是**既有**行为
  （MIDI 同样如此），不是音频片段引入的；测试用显式常量 `CENTER_GAIN` 写实。是否要让 master 不再套
  声像律（或对立体声源改用 balance 语义）需要单独决策——它会改变所有既有输出，**故不在本轮改**。
- 解码忽略 `Packet::trim_start/trim_end`（mp3 编码器延迟未裁剪）。
- `crates/asset` 的依赖图里带上了 `daw-party-engine`（只为一个类型）。

### Task 5–7 待办

宿主素材缓存、Tauri 命令 + MCP 工具、前端波形与导入入口。
