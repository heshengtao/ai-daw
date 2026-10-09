# 音频输出设备适配（多设备 / 热切换 / 错误可见）— 设计文档

- 日期：2026-10-09
- 状态：待评审
- 主题：让实时音频跟随系统默认输出设备变化（例如插上耳机），并在设备不可用时可见、可恢复

## 1. 症状

用户把系统输出从内置扬声器切到耳机后，DAW 就再也没有声音了；重新打开插件界面、单独开内置合成器窗口都无声。用户的判断是「对多声音输出设备没有做好适配」。

## 2. 根因（已从代码确认）

实时宿主只在**收到 `Load` / `SetCatalog` 消息**时才重建，而设备是每次重建时用 `default_output_device()` 现取的；**没有任何代码检测系统默认输出设备的变化**，也没有任何代码在设备出错后恢复。

证据（`file:line`）：

1. **设备只在建流时取一次**：`crates/engine/src/realtime.rs:138-142`
   ```rust
   let host = cpal::default_host();
   let device = host.default_output_device().ok_or(RealtimeError::NoDevice)?;
   let supported = device.default_output_config()...;
   ```
2. **重建只由 Load/SetCatalog 触发**：`src-tauri/src/audio.rs:309-369`（`AudioMessage` 只有 `Load` / `Command` / `SetCatalog`，`audio.rs:38-45`）。切设备不会产生这三条消息中的任何一条。
3. **流的运行期错误被吞掉**：`crates/engine/src/realtime.rs:212-214`
   ```rust
   move |err| { eprintln!("audio stream error: {err}"); }
   ```
   它不写 `available`/`error`，界面因此**一直显示 Realtime**，而实际上已经没有声音。
4. **没有手动恢复入口**：没有 `retry_audio` / `set_output_device` / `list_output_devices` 命令（`src-tauri/src/lib.rs:225-283` 的命令表里没有）。
5. **采样率缓存可能过期**：`audio.rs:376-389` 的 `resolve_sample_rate` 把设备采样率缓存在 `AtomicU32` 里，只在 `start_host` 失败时清零（`audio.rs:286`）；设备换了而缓存还在时，会撞上 `SampleRateMismatch`（`realtime.rs:156-161`）。
6. **配置选择过窄**：只接受 `default_output_config()` 且必须是 F32、并且强制 `BufferSize::Fixed(max_block)`（`realtime.rs:144-166`）。有些设备（蓝牙耳机、聚合设备）默认格式不是 F32、或拒绝固定块大小 → `build_output_stream` 失败 → `available=false` → 无声（`realtime.rs:177-219`）。

**结论**：不是「打开插件界面导致无声」，而是「输出设备一换，流就再也回不来了」，且失败状态对用户不可见。插件窗口只是让用户更早注意到。

## 3. 目标 / 非目标

**目标**

1. 系统默认输出设备变化（含插拔耳机、切换默认设备）后，约 1 秒内自动把音频流迁到新设备，保留播放位置与播放状态。
2. 设备不可用时，界面明确显示「离线」+ 原因，并提供**一键重试**，无需重启应用。
3. 建流时对配置更宽容：优先 F32、按设备实际支持的范围挑采样率、固定块大小失败时回退到默认块大小。
4. 设备恢复（重新插回、切回原设备）后自动恢复，不需要用户操作。

**非目标**

- 不做「手动选择输出设备」的设备下拉框（v1 只跟随系统默认；显式设备选择留作后续）。
- 不改引擎 DSP / 调度表 / 序列化格式。
- 不解决「每次编辑都重建宿主」这个既有开销问题。

## 4. 设计

### 4.1 纯函数：挑格式、判设备变化（可单测）

在 `crates/engine/src/realtime.rs` 里把「挑一段可用配置」和「设备变没变」做成纯函数，cpal 类型只做投影：

```rust
/// 我们关心的输出采样格式（把 cpal 的 `SampleFormat` 缩到支持面）。
pub enum OutputFormat { F32, Other }

/// 设备报告的一段可支持配置（纯数据投影，便于单测）。
pub struct OutputFormatRange { pub format: OutputFormat, pub channels: u16, pub min_rate: u32, pub max_rate: u32 }

/// 从设备报告里挑一段：只接受 F32；优先覆盖 `preferred_rate` 的段，
/// 否则选最接近它的段；同分时优先声道数 ≥ 2。
pub fn choose_output_format(ranges: &[OutputFormatRange], preferred_rate: u32) -> Option<OutputFormatRange>;

/// 默认输出设备的身份：名字 + 默认采样率。
pub struct DeviceSignature { pub name: String, pub sample_rate: u32 }

/// 读取当前默认输出设备的身份（无设备时 `None`）。
pub fn default_output_signature() -> Option<DeviceSignature>;

/// 设备是否发生变化（任一为 `None` 都算变化）。
pub fn device_changed(previous: Option<&DeviceSignature>, current: Option<&DeviceSignature>) -> bool;
```

### 4.2 建流更宽容

`AudioHost::start_with`：

- 优先用 `schedule.sample_rate` 作为偏好采样率，从 `device.supported_output_configs()` 投影出的 `OutputFormatRange` 里 `choose_output_format`；
- 投影为空（设备不报告范围）时退回 `default_output_config()`，保持今天的行为；
- 只有 F32 会被选中（纯函数保证），不再因为「默认格式不是 F32」直接失败；
- **块大小回退**：先用 `BufferSize::Fixed(max_block)` 建流，失败则用 `BufferSize::Default` 再试一次（并注释原因）。

### 4.3 运行期错误上报

新增 `SharedStatus`（引擎 crate 内，`Clone`）：

```rust
/// 音频线程可写、控制侧可读的运行状态：流回调报错时写入。
pub struct SharedStatus { dead: Arc<AtomicBool>, message: Arc<Mutex<Option<String>>> }
```

- `RealtimeOptions` 增加 `status: Arc<SharedStatus>`；错误回调写 `mark_dead(msg)`（不再只 `eprintln!`，同时保留 eprintln 便于命令行调试）。
- `AudioEngine::snapshot()` 合并它：`status.dead()` 时 `available=false`、`error = status.message()`，于是底栏立刻显示离线与原因。

### 4.4 轮询设备 + 自动重建/自动重试

`run_audio_thread` 从「阻塞 `recv()`」改为「`recv_timeout(POLL_MS=1000)`」，超时分支做两件事：

1. 探测 `default_output_signature()`；与「上次建流时的签名」不同 → 重建宿主（先清采样率缓存，再走既有 `start_host` 路径），保留位置与播放状态。
2. 宿主为 `None`（上次建流失败）或 `status.dead()` → 同样重试一次。

这样「切到耳机」≤1s 自动跟随；「设备暂时不可用」也会自动恢复。

### 4.5 手动重试

- `AudioMessage::Retry`：清空采样率缓存与 `SharedStatus`，立刻重建。
- Tauri 命令 `retry_audio` → `AudioEngine::retry_audio()`。
- 前端 `retryAudio()`（tauri + mock）。
- UI：底栏那个 Realtime/Offline 指示在离线时变成可点按钮（`BottomBar.vue`），提示里带 `error` 文案；i18n：`bottom.retryAudio`、`bottom.audioOfflineHint`。

### 4.6 顺带修掉一个死按钮

内置合成器窗口（`src/views/SynthWindowView.vue`）里的「在主窗口选中这条轨道」是**空操作**：它只改本窗口自己的 Pinia `selection`，而合成器窗口是**独立的 Vue 实例**（它自己的 store 没人读，也影响不到主窗口），既不发命令也不发事件。删除按钮及其孤立的 import / store / 函数 / 样式，并修正旁边那句过时注释。

## 5. 验收标准

1. 播放中把系统输出切到耳机：约 1 秒内声音从耳机出来；位置与播放状态不丢。
2. 把设备切到一个不可用的状态（如拔掉 USB 声卡）：底栏显示离线 + 原因，点「重试」可在设备恢复后立刻恢复声。
3. 采样格式不是 F32 的设备（或拒绝固定块大小的设备）也能出声（走 `supported_output_configs` 选择与块大小回退）。
4. 采样率变化后不再报 `SampleRateMismatch`（缓存已按需失效）。
5. 合成器窗口里不再有那个无用按钮，且 `SynthWindowView.vue` 无孤立代码。

## 6. 验证方式

- Rust：纯函数部分走 TDD（`choose_output_format` / `device_changed` 的真实单测）；`cargo test -p daw-party-engine`。
- 前端：`npm run build`（无测试框架）。
- **设备热切换只能人工验证**：本机跑起来，播放中切设备。这一点必须在报告里如实说明，不能声称已验证。

## 7. 风险

- 轮询每秒调用一次 CoreAudio 查询设备，开销可接受；但轮到一次重建会有一声极短的静音（跨设备必然如此）。
- 若某平台插耳机**不改变**默认设备名，签名判断会漏；因此同时依赖 4.3 的流错误上报来触发重试。
- 错误回调里用 `Mutex`：cpal 的错误回调不在实时线程上，且只在出错时写一次，可接受；为降低风险用「原子标志 + 一次性消息」而不是每块加锁。
