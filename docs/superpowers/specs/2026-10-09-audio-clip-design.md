# 音频片段（C1）— 设计

> **状态（2026-10-09）**：后端部分（解码 + 模型 + 排程/播放）已实现；宿主接线与前端见计划。

## 1 背景

设计文档 §6 把 `Clip` 定义为 `MidiClip`（含 `Note`）**或** `AudioClip`（含音频引用/波形/crop），
但实现里 `Clip` 只有 `notes`。Stage C 的 C1 要求：`AudioClip` 模型 + 采样器 + 波形显示。

## 2 目标与非目标

**目标**

- 音频片段能进工程模型、进时间轴、能出声、能被撤销；波形可显示。

**非目标（v1 明确不做）**

- **保音高变速**：见下面的语义决策。
- 素材库 / 相对路径（v1 存绝对路径）。
- 片段内裁剪 UI（模型已留 `offset_frames`，但界面暂不暴露）。
- 循环播放素材、淡入淡出包络、多声道 > 2 的真下混。

## 3 关键决策：时间语义 = 铺满片段跨度（会变调）

片段 `length`（拍）是时间轴上唯一真相；播放时把素材**铺满**整段跨度：

```
ratio = source_window_frames / span_frames      （span_frames 由 TempoMap 把 beats 换算成帧）
cursor(源帧) 每过一个输出帧 += ratio
```

后果（有意接受）：

- 拖宽窄片段 → 按比例重采样 → **音高改变**（等价于简单采样器的时间拉伸）。
- 改工程速度 → 跨度秒数变化 → 同样变调。

好处是模型与 MIDI 片段完全一致（都是「拍」），时间轴宽度、吸附、`Project::length_beats()` 全部复用，
实现也最简（每条片段一个固定比值、逐样本线性插值）。保音高变速留待后续（`rubato`）。

源窗口 = 素材从 `offset_frames` 到结尾的全部帧；片段跨度比它长时末尾静音，比它短时被截断。

## 4 模型与数据

- `AudioSource { path: String, offset_frames: u64 }`（`#[serde(default)]` on `offset_frames`）。
- `Clip.audio: Option<AudioSource>`，`#[serde(default, skip_serializing_if = "Option::is_none")]`
  → **MIDI 片段的 JSON 输出形状与从前逐字一致**（`src/api/types.ts` 与 wire-shape 测试都依赖）。
- `Clip::is_audio()`；`Clip::normalize`：音频片段的 `notes` 必须为空。
- 命令：`AddAudioClip { track, start, length, source }`、`SetClipAudio { track, clip, audio: Option<_> }`
  （`None` = 变回 MIDI 片段）。搬运/删除/缩放复用既有命令。
- 事件：`AudioClipAdded { …, source }`、`ClipAudioChanged { …, audio }`；两者都是**结构性**变更
  （非 `is_mix_only`），宿主据此重建。
- `DecodedAudio { samples: Arc<[f32]>, channels, sample_rate }` 放在 `crates/engine/src/audio.rs`；
  `SampleProvider` trait 让调度「按路径取素材」，引擎因此不依赖文件系统。

## 5 引擎改动

- `ScheduledEvents::from_project_with_assets(project, rate, assets)`；`from_project` 保留并委托
  （空素材表 → 音频片段静音），测试与调用点不必全改。
- `ScheduledAudioClip { start_frame, end_frame, offset_frames, source_window_frames, audio }`，
  绝对帧用与 MIDI **同一个** `to_frame` 闭包，保证严格对齐；素材缺失/窗口为空的片段被跳过（静音）。
- `Player`：
  - `TrackVoice.plugin` 变为 `Option<Box<dyn Plugin>>`——**没挂乐器的轨道仍然存在**，因为它可能有音频片段。
  - 音频片段在**乐器之后、效果链之前**相加 → 与 MIDI 一样经过轨道效果、轨道增益/声像与 master。
  - 逐样本线性插值（`lerp_frame`），游标用 `f64` 累加避免漂移；`process_block` 内**零分配**
    （素材与游标都在 `Player::new` 准备好，由 `tests/realtime_safety.rs` 强制）。
  - `seek` 按新位置重算每个片段的游标；**暂停时不播放音频片段**（它是时间轴内容，
    与「插件继续跑让界面键盘能发声」不同）。
- **暂停 = 静音（本轮修正）**：`TransportCommand::Pause` / `Stop` 会重置乐器**与效果链**
  （等价 all-notes-off）并丢掉待送试听音。原实现只把 `playing` 置 false，于是长音的 NoteOff
  永远送不到，插件会**一直挂着那个音**——听上去就是「暂停了还在响」。
  插件实例不销毁，所以暂停时界面键盘（audition）与插件自带界面的预览仍然出声；它们只在暂停
  那一刻被收掉，随后照常工作。
- **声像律改为线性 balance（本轮修正）**：原先轨道与 master 各套一次等功率声像
  （居中各 ×√0.5），整链凭空 −6 dB——导入满幅素材会明显变轻。现在居中 = 单位增益，
  向一侧偏则衰减另一侧。
- `render_with_assets(...)`；`render(...)` 委托（空素材）。

## 6 解码（新 crate `daw-party-asset`）

- `symphonia`（wav / flac / mp3 / ogg-vorbis / aac / m4a），**MPL-2.0**，仅作未修改依赖（见 NOTICE）。
- `decode(path) -> DecodedAudio`：声道收敛到 1/2（>2 取前两声道）；**保留素材自身采样率**；
  坏帧跳过、致命错误才失败。
- `peaks(audio, buckets) -> Vec<f32>`：每桶 min/max，直接给波形绘制。
- 已知粗糙：忽略 `Packet::trim_start/trim_end`（mp3 编码器延迟未裁剪），v1 接受。

## 7 验收标准

1. 模型：旧工程（无 `audio`）可读；MIDI 片段序列化**不含** `audio` 键；音频片段的 normalize 清空音符。
2. 命令：`AddAudioClip` 归一化（起点夹到 ≥0、长度夹到合法区间）且可撤销/重做；
   `SetClipAudio` 能在音频/MIDI 之间切换并清空音符。
3. 播放（量化断言，见 `crates/engine/tests/audio_clip.rs`）：
   - 满幅 1 kHz 素材放在第 0 拍 → 频谱峰 1 kHz、峰值与 RMS 符合链路增益；
   - 放在第 4 拍 → 起点前**严格为 0**；
   - 跨度减半 → 频谱峰移到 2 kHz（证明「铺满 → 变调」）；
   - `seek` 到片段中段 → 第一帧取自素材对应位置（用阶梯素材精确断言）；
   - 未 `Play` 时静音；素材缺失时静音且**不改变时间轴长度**；
   - 同轨「乐器 + 音频片段」是**相加**（RMS 明显升高）。
4. 解码：单/立体声 WAV 的帧数、采样率、峰值正确；`peaks` 每桶包住信号；缺失/非音频文件报错。
5. 实时安全：`realtime_safety.rs` 的零分配门禁**继续通过**。

## 8 验证方式

```
cargo test --workspace
cargo test -p daw-party-engine --features realtime
cargo test -p daw-party-engine --features realtime --test realtime_safety
cargo check --manifest-path src-tauri/Cargo.toml
```

## 9 风险

| 风险 | 缓解 |
| --- | --- |
| 音频进实时回调破坏「零分配」 | 素材/游标全部在构造期准备；零分配测试是硬门禁 |
| 无乐器轨道原本没有 TrackVoice，音频片段会静默丢失 | `TrackVoice.plugin` 改 `Option`，并专门写测试覆盖 |
| 「铺满」变调可能不符合预期 | 已作为决策记录在 §3 与 ROADMAP §2；保音高变速留后续 |
| ~~既有增益结构整链 ×0.5（−6 dB）~~ | **已修（2026-10-09）**：声像律由等功率改为线性 balance，居中 = 单位增益。见 §5 与 ROADMAP §2 |
| 暂停后长音挂住（听上去「暂停了还在响」） | **已修（2026-10-09）**：`Pause`/`Stop` 重置乐器与效果链。见 §5 |
| 工程播完（非循环）时同样会 all-notes-off，插件内部的混响尾音会被切掉 | 接受：与「停止即静音」一致；**离线导出**不受影响（`render` 按 `tail_seconds` 继续渲染尾音） |
