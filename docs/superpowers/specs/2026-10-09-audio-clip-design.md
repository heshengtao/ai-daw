# 音频片段（C1）— 设计

> **状态（2026-10-10 校准）**：**已完成** —— 后端（解码 + 模型 + 排程/播放）、宿主接线、前端波形与「导入音频…」入口、真机端到端均已落地。保音高变速等仍按 §2 非目标不做。

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

## 3 关键决策：时间语义 = 素材原速播放（与工程 BPM 解耦）

片段 `length`（拍）只决定时间轴上的**方框**；素材按 `rate`（播放速度）**原速播放**，与工程速度无关：

```
cursor(源帧) 每过一个输出帧 += rate × r    （rate = AudioSource.rate，默认 1.0；
                                            r  = 素材采样率 / 工程采样率 = source_ratio）
end_frame 仍由 length 决定（可听上界）；素材放完或方框到头即停
```

- **采样率换算（`r`）**：播放器按**输出帧**推进，而素材存的是**它自己采样率下**的帧。`start_frame` /
  `end_frame` 是输出帧，`offset_frames` / `source_window_frames` 是源帧——所以读指针每个输出帧必须
  前进 `r` 个源帧（`lerp_frame` 插值，即采样率换算：**时长与音高都不变**）。
  > 2026-10-15 修：早先这一步漏了 `r`（直接 `+= rate`），于是 44.1 kHz 素材在 48 kHz 工程里会快
  > 48000/44100 ≈ 8.8% 并升调，尾端还会被方框截掉——听感正是「最后一小节没声音」。
  > `crates/engine/tests/audio_clip.rs::material_at_another_sample_rate_keeps_its_duration_and_pitch` 守它。
- **改工程速度不改音频**（不再重采样）。改速度时宿主把每个音频片段的 `length` 重算成它的**原真实
  秒数**在新速度下的拍数（`commands::resize_audio_clips_for_transport`，与速度命令同一批 → 一条撤销），
  于是方框始终覆盖素材、音频原速、网格随速度走。
- **变速 / 升降调是片段自己的参数**（`AudioSource.rate` / `pitch_semitones`），在检查器里改；
  引擎用 **WSOLA 时值伸缩**（保音高）+ 重采样实现：`pitch = 2^(半音/12)`、`stretch = pitch / rate`
  （`crates/engine/src/stretch.rs`）：
  - `pitch = 0` → 只伸缩 → **变速不变调**；
  - `rate = 1` → 只重采样 → **变调不变速**。
  - 采样率换算 `r` **并进读步进**（`pitch · r`），**不进伸缩**：WSOLA 保的是**帧周期**，把 `r`
    折进伸缩会把 44.1k 的帧周期原样留在 48k 输出里 → 照样升调。
  > 2026-10-15 修（**只在这一条路径上**才走 WSOLA：`rate ≠ 1` 或 `pitch ≠ 0`，其余走原速直读）：
  > ① 对齐搜索原来对 `±SEARCH` 里每个候选都重算 512 抽头互相关 = **105 万次乘加每块**，而一块只出
  > `HOP` 个输出帧 ≈ 每输出帧 2000 次乘加。Debug 构建实测 **42–170 µs/输出帧**，实时预算只有
  > 20.8 µs（200%–800%）→ 音频回调必然爆音、卡死，听感正是「一设速度/升降调就是噪音 + 卡住」。
  > 改成**先粗后精**（`COARSE=4` 步长 + `DECIM=4` 抽点粗搜 → 最优点 ±4 内全抽点精搜），快 ~15×，
  > Debug 下也稳过实时。粗搜步长必须远小于相关峰宽度——用 16 时鼓点会被对到相邻的另一个瞬态上。
  > ② `seek` 之后第一块的 `prev` 是清零的，但 `block > 0` 仍让 `has_template` 为真：零模板的能量为 0，
  > 归一化互相关的分母只剩 1e-9，所有候选得分退化成 0 →「第一个候选胜出」把取段硬拽到
  > `a − SEARCH`（≈21 ms 错位）。现在模板能量为 0 就直接跳过搜索（取名义位置）。
  > 定位、循环回卷、每次结构性重建（重建后会 seek 到当前位置）都会踩这一脚。
  > 回归测试：`crates/engine/src/stretch.rs::seek_lands_on_the_nominal_position_not_a_search_radius_offset`
  > （粗/精两路的选段质量由既有的 `low_frequency_pitch_shift_is_correct` 等频响断言守住）。
- **`rate` 必须参与一切「拍 / 时间轴秒 ↔ 素材帧」的换算**：时间轴 1 秒 = 素材 **`rate`** 秒。
  宿主侧统一走 `commands::{span_to_source_frames, source_frames_to_span}`（被 `trimmed_window`、
  `clip_bounds_for`、`set_clip_rate`、`split_clip` 使用）；前端波形把 `rate` 传给 `ClipWaveform`
  参与「时间轴秒 → 素材秒」的映射。
  > 2026-10-15 修：`trim_clip` / `clip_bounds` / `split_clip` / `ClipWaveform` 四处都漏了 `rate`。
  > 后果是「设过播放速度之后再拖动片段」会把素材窗口设成 `1/rate`——`rate = 2` 时只播前半段、
  > 后半段静音，右边缘还能拖到素材早已放完之后；波形也会与听到的对不上。
  > 回归测试：`src-tauri/src/commands.rs` 的 `trimming_consumes_material_in_proportion_to_playback_rate`
  > / `moving_a_clip_shifts_the_window_by_playback_rate` / `clip_bounds_account_for_playback_rate`。
- **拖左右边缘 = 裁剪**（改读写窗口 `offset_frames` / `frames`），**不改速度、不变调**。

好处：BPM 只管网格 + MIDI 时序；音频原速、可独立变速变调——与主流 DAW 的默认行为一致。

> 历史：早期版本是「铺满片段跨度」（`ratio = source_window_frames / span_frames`，改速度/拖宽会
> 变调）。2026-10-13 改成上面的解耦模型；`rate`/`pitch` 缺省的旧工程形状不变。

## 4 模型与数据

- `AudioSource { path, offset_frames, frames: Option<u64>, rate, pitch_semitones }`
  （`offset_frames`/`frames`/`rate`/`pitch_semitones` 全带 `#[serde(default)]`；`rate`/`pitch` 等于
  缺省值时 `skip_serializing_if` 不写出 → 旧工程形状不变）。
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

## 6 解码（新 crate `ai-daw-asset`）

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
    - 跨度减半 → 只**截断**，频谱峰仍是 1 kHz（方框不改音高）；
    - 工程速度翻倍 → 音频仍是 1 kHz（**与 BPM 解耦**）；
    - `rate = 2` → 时长减半、仍 1 kHz（变速不变调）；`pitch = +12` → 仍是原长、2 kHz（变调不变速）；
   - `seek` 到片段中段 → 第一帧取自素材对应位置（用阶梯素材精确断言）；
   - 未 `Play` 时静音；素材缺失时静音且**不改变时间轴长度**；
   - 同轨「乐器 + 音频片段」是**相加**（RMS 明显升高）。
4. 解码：单/立体声 WAV 的帧数、采样率、峰值正确；`peaks` 每桶包住信号；缺失/非音频文件报错。
5. 实时安全：`realtime_safety.rs` 的零分配门禁**继续通过**。

## 8 验证方式

```
cargo test --workspace
cargo test -p ai-daw-engine --features realtime
cargo test -p ai-daw-engine --features realtime --test realtime_safety
cargo check --manifest-path src-tauri/Cargo.toml
```

## 9 风险

| 风险 | 缓解 |
| --- | --- |
| 音频进实时回调破坏「零分配」 | 素材/游标全部在构造期准备；零分配测试是硬门禁 |
| 无乐器轨道原本没有 TrackVoice，音频片段会静默丢失 | `TrackVoice.plugin` 改 `Option`，并专门写测试覆盖 |
| 「铺满」变调不符合预期 | 2026-10-13 已改为**解耦模型**（音频原速、`rate`/`pitch` 独立），见 §3 与 ROADMAP §2 |
| ~~既有增益结构整链 ×0.5（−6 dB）~~ | **已修（2026-10-09）**：声像律由等功率改为线性 balance，居中 = 单位增益。见 §5 与 ROADMAP §2 |
| 暂停后长音挂住（听上去「暂停了还在响」） | **已修（2026-10-09）**：`Pause`/`Stop` 重置乐器与效果链。见 §5 |
| 工程播完（非循环）时同样会 all-notes-off，插件内部的混响尾音会被切掉 | 接受：与「停止即静音」一致；**离线导出**不受影响（`render` 按 `tail_seconds` 继续渲染尾音） |
