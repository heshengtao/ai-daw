# 音频输出设备适配 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 系统默认输出设备变化（插耳机等）后自动把音频流迁到新设备；设备不可用时界面可见并可一键重试；建流对设备配置更宽容。

**Architecture:** 把「挑格式」「判设备变化」做成引擎 crate 里的**纯函数**（TDD）；`AudioHost::start_with` 用纯函数的结果建流，块大小失败可回退；新增 `SharedStatus` 让流运行期错误能到达界面；音频线程从阻塞 `recv()` 改为 1s `recv_timeout` 轮询，设备变了/流死了就重建；再加一条 `retry_audio` 命令与底栏重试按钮。

**Tech Stack:** Rust（cpal、thiserror）、Tauri 2、Vue 3 `<script setup>` + Pinia + TypeScript。

## Global Constraints

- 引擎 crate = `daw-party-engine`（`crates/engine`）；`src-tauri` 包 = `daw-party-app`，**自成一个 workspace**（`--manifest-path src-tauri/Cargo.toml`）。
- **验证引擎必须带 feature**：`realtime` 模块被 `#[cfg(feature = "realtime")]` 门控（`crates/engine/src/lib.rs:13`），而 `default = []`。所以命令一律是
  `cargo test -p daw-party-engine --features realtime`——**不带 `--features realtime` 时 `realtime.rs` 根本不会被编译，测试会「全绿」但什么也没验证**（这是本计划第一轮踩过的坑）。
- 前端仍**没有测试框架**：验证 = `npm run build`。
- 实时回调里**不得加锁、不得分配**（既有约定）。`SharedStatus` 的 `Mutex` 只允许在 cpal 的**错误回调**（非实时线程）里用。
- 注释与文案用中文；界面文案必须 i18n（zh + en 各一份）。
- 不改调度表结构、序列化格式、DSP。
- 提交信息格式：`fix(audio): …` / `feat(audio): …`。

---

## File Structure

- `crates/engine/src/realtime.rs` — 纯函数（`OutputFormat` / `OutputFormatRange` / `choose_output_format` / `DeviceSignature` / `default_output_signature` / `device_changed`）、`SharedStatus`、建流回退、错误回调。
- `src-tauri/src/audio.rs` — `SharedStatus` 接入、`AudioMessage::Retry`、`run_audio_thread` 轮询、`resolve_sample_rate` 失效策略、`AudioEngine::retry_audio`。
- `src-tauri/src/commands.rs`、`src-tauri/src/lib.rs` — `retry_audio` 命令注册。
- `src/api/tauri.ts`、`src/api/mock.ts` — `retryAudio()`。
- `src/components/layout/BottomBar.vue` — 离线时把指示变成重试按钮 + 原因提示。
- `src/i18n/zh.ts`、`src/i18n/en.ts` — 新文案。
- `src/views/SynthWindowView.vue` — 删除无用按钮（见计划 E 的任务，不在此计划内）。

---

### Task 1: 纯函数：挑输出格式 + 判设备变化（TDD）

**Files:**
- Modify: `crates/engine/src/realtime.rs`

**Interfaces:**
- Produces: `OutputFormat`、`OutputFormatRange`、`choose_output_format(&[OutputFormatRange], u32) -> Option<OutputFormatRange>`、`DeviceSignature`、`default_output_signature() -> Option<DeviceSignature>`、`device_changed(Option<&DeviceSignature>, Option<&DeviceSignature>) -> bool`。

- [ ] **Step 1: 先写失败的测试**

在 `crates/engine/src/realtime.rs` 的 `mod tests` 里加（此刻还没实现，必须编译失败）：

```rust
    #[test]
    fn choose_format_rejects_non_f32() {
        let ranges = [range(OutputFormat::Other, 2, 44_100, 44_100)];
        assert!(choose_output_format(&ranges, 48_000).is_none());
    }

    #[test]
    fn choose_format_prefers_a_range_covering_the_preferred_rate() {
        let ranges = [
            range(OutputFormat::F32, 2, 44_100, 44_100),
            range(OutputFormat::F32, 2, 48_000, 96_000),
        ];
        let picked = choose_output_format(&ranges, 48_000).unwrap();
        assert_eq!((picked.min_rate, picked.max_rate), (48_000, 96_000));
    }

    #[test]
    fn choose_format_falls_back_to_the_nearest_range() {
        let ranges = [
            range(OutputFormat::F32, 2, 44_100, 44_100),
            range(OutputFormat::F32, 2, 96_000, 96_000),
        ];
        // 偏好 48k：44.1k 比 96k 近。
        let picked = choose_output_format(&ranges, 48_000).unwrap();
        assert_eq!(picked.min_rate, 44_100);
    }

    #[test]
    fn choose_format_prefers_stereo_when_tied() {
        let ranges = [
            range(OutputFormat::F32, 1, 48_000, 48_000),
            range(OutputFormat::F32, 2, 48_000, 48_000),
        ];
        assert_eq!(choose_output_format(&ranges, 48_000).unwrap().channels, 2);
    }

    #[test]
    fn range_clamps_the_rate_into_its_span() {
        assert_eq!(range(OutputFormat::F32, 2, 44_100, 48_000).nearest_rate(96_000), 48_000);
        assert_eq!(range(OutputFormat::F32, 2, 44_100, 48_000).nearest_rate(8_000), 44_100);
    }

    #[test]
    fn device_changed_flags_any_presence_or_identity_change() {
        let a = DeviceSignature { name: "A".into(), sample_rate: 48_000 };
        let a2 = DeviceSignature { name: "A".into(), sample_rate: 48_000 };
        let b = DeviceSignature { name: "B".into(), sample_rate: 48_000 };
        let a44 = DeviceSignature { name: "A".into(), sample_rate: 44_100 };
        assert!(device_changed(None, Some(&a)));
        assert!(device_changed(Some(&a), None));
        assert!(!device_changed(Some(&a), Some(&a2)));
        assert!(device_changed(Some(&a), Some(&b)));
        assert!(device_changed(Some(&a), Some(&a44)));
    }
```

并在 `mod tests` 顶部加一个构造函数（放在 `fn schedule()` 附近）：

```rust
    fn range(format: OutputFormat, channels: u16, min_rate: u32, max_rate: u32) -> OutputFormatRange {
        OutputFormatRange { format, channels, min_rate, max_rate }
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p daw-party-engine --features realtime realtime`
Expected: 编译失败（`OutputFormat` / `choose_output_format` / `device_changed` 未定义）

- [ ] **Step 3: 实现**

在 `crates/engine/src/realtime.rs` 的 `RealtimeError` 之后、`write_interleaved` 之前加：

```rust
/// 我们关心的输出采样格式（把 cpal 的 `SampleFormat` 缩到支持面）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// 32 位浮点：唯一被支持的格式。
    F32,
    /// 其它格式（i16/u16/…）：不支持。
    Other,
}

/// 设备报告的一段可支持配置（纯数据投影，便于单测）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputFormatRange {
    /// 采样格式。
    pub format: OutputFormat,
    /// 声道数。
    pub channels: u16,
    /// 该段覆盖的最低采样率（Hz）。
    pub min_rate: u32,
    /// 该段覆盖的最高采样率（Hz）。
    pub max_rate: u32,
}

impl OutputFormatRange {
    /// 该段内最接近 `rate` 的采样率（夹在 `[min_rate, max_rate]` 内）。
    pub fn nearest_rate(&self, rate: u32) -> u32 {
        rate.clamp(self.min_rate, self.max_rate)
    }

    /// 该段是否覆盖给定采样率。
    pub fn covers(&self, rate: u32) -> bool {
        self.min_rate <= rate && rate <= self.max_rate
    }
}

/// 从设备报告的配置里挑一段可用配置。
///
/// 只接受 F32；优先覆盖 `preferred_rate` 的段，否则选 `nearest_rate` 离它最近的段；
/// 仍并列时优先声道数 ≥ 2。全都不支持时返回 `None`。
pub fn choose_output_format(ranges: &[OutputFormatRange], preferred_rate: u32) -> Option<OutputFormatRange> {
    let f32_only = || ranges.iter().copied().filter(|r| r.format == OutputFormat::F32);
    f32_only()
        .filter(|r| r.covers(preferred_rate))
        .max_by_key(|r| (r.channels >= 2, r.channels))
        .or_else(|| {
            f32_only().min_by_key(|r| {
                let delta = r.nearest_rate(preferred_rate).abs_diff(preferred_rate);
                (delta, !(r.channels >= 2), u16::MAX - r.channels)
            })
        })
}

/// 默认输出设备的身份：名字 + 默认采样率。用于判断「设备换了没有」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSignature {
    /// 设备名（系统显示名）。
    pub name: String,
    /// 设备默认输出采样率（Hz）。
    pub sample_rate: u32,
}

/// 读取当前默认输出设备的身份；无设备（或查询失败）时 `None`。
pub fn default_output_signature() -> Option<DeviceSignature> {
    let host = cpal::default_host();
    let device = host.default_output_device()?;
    let name = device.name().ok()?;
    let sample_rate = device.default_output_config().ok()?.sample_rate();
    Some(DeviceSignature { name, sample_rate })
}

/// 设备是否发生变化：任一为 `None` 都算变化（出错后重试也因此会触发）。
pub fn device_changed(previous: Option<&DeviceSignature>, current: Option<&DeviceSignature>) -> bool {
    previous != current
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p daw-party-engine --features realtime`
Expected: 全部通过（含既有 13 个）

- [ ] **Step 5: 提交**

```bash
git add crates/engine/src/realtime.rs
git commit -m "feat(audio): pure helpers to choose an output format and detect device changes"
```

---

### Task 2: 建流更宽容（格式选择 + 块大小回退）

**Files:**
- Modify: `crates/engine/src/realtime.rs`

**Interfaces:**
- Consumes: Task 1 的 `OutputFormat` / `OutputFormatRange` / `choose_output_format`。
- Produces: `AudioHost::start_with` 不再因为「默认格式不是 F32」或「设备拒绝固定块大小」而直接失败。

- [ ] **Step 1: 写解析辅助 + 改 start_with**

在 `crates/engine/src/realtime.rs` 里（`default_output_sample_rate` 之后）加一个把 cpal 类型投影成纯数据的辅助：

```rust
/// 把设备报告的可支持配置投影成纯数据；设备不报告范围时返回空表。
fn supported_formats(device: &cpal::Device) -> Vec<OutputFormatRange> {
    let Ok(ranges) = device.supported_output_configs() else { return Vec::new() };
    ranges
        .map(|r| OutputFormatRange {
            format: if r.sample_format() == cpal::SampleFormat::F32 {
                OutputFormat::F32
            } else {
                OutputFormat::Other
            },
            channels: r.channels(),
            min_rate: r.min_sample_rate().0,
            max_rate: r.max_sample_rate().0,
        })
        .collect()
}
```

把 `start_with` 里从 `let supported = device.default_output_config()...` 到 `config.buffer_size = ...` 的整段（`realtime.rs:140-166`）替换为：

```rust
        let preferred_rate = schedule.sample_rate.round().max(1.0) as u32;
        let chosen = choose_output_format(&supported_formats(&device), preferred_rate);

        // 优先用设备报告的范围挑：只要有一段 F32 就能出声（默认格式不是 F32 的设备也能救回来）。
        // 设备不报告范围时退回默认配置，保持旧行为。
        let mut config = match chosen {
            Some(range) => cpal::StreamConfig {
                channels: range.channels,
                sample_rate: cpal::SampleRate(range.nearest_rate(preferred_rate)),
                buffer_size: cpal::BufferSize::Default,
            },
            None => {
                let supported = device
                    .default_output_config()
                    .map_err(|e| RealtimeError::UnsupportedConfig(e.to_string()))?;
                if supported.sample_format() != cpal::SampleFormat::F32 {
                    return Err(RealtimeError::UnsupportedConfig(format!(
                        "device default output format is {:?}, and no f32 range is offered",
                        supported.sample_format()
                    )));
                }
                supported.config()
            }
        };

        // 事件表以固定采样率定点；与设备采样率不一致会静默错位，必须拒绝。
        if schedule.sample_rate != config.sample_rate.0 as f64 {
            return Err(RealtimeError::SampleRateMismatch {
                schedule: schedule.sample_rate,
                device: config.sample_rate.0 as f64,
            });
        }

        let max_block = options.max_block.max(1);
```

注意：`channels` 变量与后面的 `write_interleaved` 用到的声道数必须来自最终的 `config.channels`——把回调里原来的 `let channels = ...;` 改成在闭包前 `let channels = config.channels as usize;`。

- [ ] **Step 2: 块大小回退**

在建流处（`device.build_output_stream(...)`）加注释并抽出一次尝试，让固定块大小失败时回退到默认块大小：

```rust
        // 先请求固定块大小（回调内 scratch 预分配即上界）；有些设备（蓝牙/聚合设备）拒绝
        // 固定块大小，这时回退到设备默认块大小，宁可让回调内的分块循环兜底。
        let build = |cfg: cpal::StreamConfig| {
            device
                .build_output_stream(cfg, on_data, on_error, None)
                .map_err(|e| RealtimeError::Cpal(e.to_string()))
        };
        config.buffer_size = cpal::BufferSize::Fixed(max_block as u32);
        let stream = match build(config.clone()) {
            Ok(s) => s,
            Err(first) => {
                config.buffer_size = cpal::BufferSize::Default;
                build(config).map_err(|_| first)?
            }
        };
```

把原来的 `move |data, _| {...}` 与 `move |err| {...}` 两个闭包先绑定为 `on_data` / `on_error`，再传给 `build`。`on_error` 在 Task 3 里会改成写 `SharedStatus`；本任务先保持 `eprintln!`。

- [ ] **Step 3: 验证**

Run: `cargo test -p daw-party-engine --features realtime`（应通过，含依赖真实设备的冒烟测试 `start_returns_result_without_panicking`）
Run: `cargo build --manifest-path src-tauri/Cargo.toml`（应通过）

- [ ] **Step 4: 提交**

```bash
git add crates/engine/src/realtime.rs
git commit -m "fix(audio): pick any supported f32 output format and fall back on buffer size"
```

---

### Task 3: 流运行期错误进入共享状态

**Files:**
- Modify: `crates/engine/src/realtime.rs`、`src-tauri/src/audio.rs`

**Interfaces:**
- Produces: `SharedStatus`（`Clone`）、`SharedStatus::dead()`、`SharedStatus::message()`、`SharedStatus::mark_dead(&str)`、`SharedStatus::clear()`；`RealtimeOptions.status: Arc<SharedStatus>`；`AudioEngine.snapshot()` 在流死亡时 `available=false` 且带上原因。

- [ ] **Step 1: 先写失败的测试**

在 `crates/engine/src/realtime.rs` 的 `mod tests` 里加：

```rust
    #[test]
    fn shared_status_starts_healthy_and_records_the_first_error() {
        let status = SharedStatus::new();
        assert!(!status.dead());
        assert_eq!(status.message(), None);
        status.mark_dead("device gone");
        assert!(status.dead());
        assert_eq!(status.message().as_deref(), Some("device gone"));
        // 首次错误保留，不被后续覆盖。
        status.mark_dead("later");
        assert_eq!(status.message().as_deref(), Some("device gone"));
        status.clear();
        assert!(!status.dead());
        assert_eq!(status.message(), None);
    }
```

Run: `cargo test -p daw-party-engine --features realtime shared_status` → 编译失败。

- [ ] **Step 2: 实现 SharedStatus**

在 `SharedTransport` 之后加：

```rust
/// 音频流的运行期状态：流回调出错时写入，控制侧读取。
///
/// 只在 cpal 的**错误回调**里写（那是 cpal 自己的线程，不是实时回调），因此这里的
/// `Mutex` 不会出现在音频处理路径上。
#[derive(Clone, Default)]
pub struct SharedStatus {
    dead: Arc<AtomicBool>,
    message: Arc<Mutex<Option<String>>>,
}

impl SharedStatus {
    /// 新建一份健康状态。
    pub fn new() -> Self {
        Self::default()
    }

    /// 流是否已失效（设备被拔掉/切换后 cpal 报告的错误）。
    pub fn dead(&self) -> bool {
        self.dead.load(Ordering::Relaxed)
    }

    /// 失效原因（若有）。
    pub fn message(&self) -> Option<String> {
        self.message.lock().unwrap().clone()
    }

    /// 标记失效；保留**第一条**原因（后续连环错误通常只是前一因的余波）。
    pub fn mark_dead(&self, reason: &str) {
        self.dead.store(true, Ordering::Relaxed);
        let mut slot = self.message.lock().unwrap();
        if slot.is_none() {
            *slot = Some(reason.to_string());
        }
    }

    /// 清空状态（重建宿主前调用）。
    pub fn clear(&self) {
        self.dead.store(false, Ordering::Relaxed);
        *self.message.lock().unwrap() = None;
    }
}
```

需要在文件头补 `use std::sync::Mutex;`。

- [ ] **Step 3: 接进 RealtimeOptions 与错误回调**

- `RealtimeOptions` 加字段：
```rust
    /// 流状态：错误回调写入，控制侧读取。
    pub status: Arc<SharedStatus>,
```
- `start_with` 里在绑定 `on_error` 处改为：
```rust
        let status = Arc::clone(&options.status);
        let on_error = move |err: cpal::StreamError| {
            eprintln!("audio stream error: {err}");
            status.mark_dead(&err.to_string());
        };
```

- [ ] **Step 4: audio.rs 接入**

- `AudioEngine` 加字段 `status: Arc<SharedStatus>`（`AudioEngine::new` 里 `Arc::new(SharedStatus::new())`，并 clone 进线程）。
- `start_host` 的 `options` 加 `status: Arc::clone(status)`（把 `status` 作为新参数传下去）。
- `snapshot()` 里合并：
```rust
        let dead = self.status.dead();
        let available = self.available.load(Ordering::Relaxed) && !dead;
        TransportSnapshot {
            available,
            playing: available && self.shared.is_playing(),
            position_frames: if available { self.shared.position_frames() } else { 0 },
            sample_rate: self.sample_rate.load(Ordering::Relaxed),
            error: if dead { self.status.message() } else { self.error.lock().unwrap().clone() },
        }
```

- [ ] **Step 5: 验证**

Run: `cargo test -p daw-party-engine --features realtime`（含新测试）
Run: `cargo build --manifest-path src-tauri/Cargo.toml`

- [ ] **Step 6: 提交**

```bash
git add crates/engine/src/realtime.rs src-tauri/src/audio.rs
git commit -m "feat(audio): surface runtime stream errors to the UI snapshot"
```

---

### Task 4: 轮询设备变化 + 自动重建/自动重试

**Files:**
- Modify: `src-tauri/src/audio.rs`

**Interfaces:**
- Consumes: `device_changed` / `default_output_signature`（引擎 crate 已导出）。
- Produces: `AudioMessage::Retry`；音频线程每秒检查一次设备，变了或流死了就重建。

- [ ] **Step 1: 加 Retry 消息与重试入口**

```rust
enum AudioMessage {
    Load(Box<Project>),
    Command(TransportCommand),
    SetCatalog(Vec<CatalogPlugin>),
    /// 立刻重新探测设备并重建宿主（界面上的「重试」，以及设备变化时的自动重建）。
    Retry,
}
```

`AudioEngine` 加：
```rust
    /// 重新探测输出设备并重建宿主（界面「重试」按钮）。
    pub fn retry_audio(&self) {
        let _ = self.tx.send(AudioMessage::Retry);
    }
```

- [ ] **Step 2: 把重建抽成一个闭包函数**

为避免 `Load` / `SetCatalog` / `Retry` / 轮询四处重复，抽一个局部辅助（在 `run_audio_thread` 内，借用 `current` / `catalog` / 各原子量）：

```rust
    /// 重建宿主：保留位置与播放状态。`reset_rate` 为真时先清采样率缓存（设备可能换了）。
    fn rebuild(
        current: &Option<Project>,
        catalog: &[CatalogPlugin],
        editor_handles: &EditorHandles,
        shared: &SharedTransport,
        status: &Arc<SharedStatus>,
        sample_rate: &AtomicU32,
        available: &AtomicBool,
        error: &Mutex<Option<String>>,
        scope: &Arc<ScopeBuffer>,
        reset_rate: bool,
    ) -> Option<AudioHost> {
        let Some(project) = current.clone() else { return None };
        if reset_rate {
            sample_rate.store(0, Ordering::Relaxed);
        }
        status.clear();
        start_host(&project, catalog, editor_handles, shared, status, sample_rate, available, error, scope)
    }
```

（把它写成 `run_audio_thread` 外的普通函数，参数就是上面这些；`start_host` 增加 `status` 参数。）

- [ ] **Step 3: 轮询循环**

把 `while let Ok(message) = rx.recv()` 改为：

```rust
    /// 设备轮询间隔：够快能跟上插拔耳机，又不至于每秒多次查询 CoreAudio 太过分。
    const POLL: std::time::Duration = std::time::Duration::from_millis(1000);

    let mut signature = default_output_signature();
    let mut host: Option<AudioHost> = None;
    let mut current: Option<Project> = None;

    loop {
        match rx.recv_timeout(POLL) {
            Ok(AudioMessage::Command(command)) => {
                if let Some(h) = host.as_mut() {
                    h.send(command);
                }
            }
            Ok(AudioMessage::SetCatalog(new_catalog)) => {
                catalog = new_catalog;
                let was_playing = shared.is_playing();
                let frame = shared.position_frames();
                drop(host.take());
                host = rebuild(/* …, */ false);
                if let Some(h) = host.as_mut() {
                    h.seek(frame);
                    if was_playing {
                        h.play();
                    }
                }
                signature = default_output_signature();
            }
            Ok(AudioMessage::Load(project)) => {
                current = Some((*project).clone());
                let was_playing = shared.is_playing();
                let frame = shared.position_frames();
                drop(host.take());
                host = rebuild(/* …, */ false);
                if let Some(h) = host.as_mut() {
                    h.seek(frame);
                    if was_playing {
                        h.play();
                    }
                }
            }
            Ok(AudioMessage::Retry) => {
                let was_playing = shared.is_playing();
                let frame = shared.position_frames();
                drop(host.take());
                host = rebuild(/* …, */ true);
                if let Some(h) = host.as_mut() {
                    h.seek(frame);
                    if was_playing {
                        h.play();
                    }
                }
                signature = default_output_signature();
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // 设备换了，或上次建流失败/流已死：重建一次（保留位置与播放状态）。
                let now = default_output_signature();
                let needs = host.is_none() || status.dead() || device_changed(signature.as_ref(), now.as_ref());
                if !needs {
                    continue;
                }
                let was_playing = shared.is_playing();
                let frame = shared.position_frames();
                drop(host.take());
                host = rebuild(/* …, */ true);
                if let Some(h) = host.as_mut() {
                    h.seek(frame);
                    if was_playing {
                        h.play();
                    }
                }
                signature = now;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
```

要点：`host.is_none()` 这一条让**建流失败后每秒自动重试**（设备恢复即自动恢复）；`status.dead()` 让「设备名没变但流死了」也能救。

- [ ] **Step 4: 验证**

Run: `cargo test -p daw-party-engine --features realtime`
Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Run: `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`（若项目原本 clippy 干净；不干净则跳过并说明）

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/audio.rs
git commit -m "fix(audio): follow the system output device and auto-retry after stream failure"
```

---

### Task 5: `retry_audio` 命令 + 底栏重试按钮（i18n）

**Files:**
- Modify: `src-tauri/src/commands.rs`、`src-tauri/src/lib.rs`、`src/api/tauri.ts`、`src/api/mock.ts`、`src/components/layout/BottomBar.vue`、`src/i18n/zh.ts`、`src/i18n/en.ts`

**Interfaces:**
- Consumes: `AudioEngine::retry_audio()`（Task 4）。
- Produces: 前端 `retryAudio(): Promise<void>`；底栏离线时显示原因与重试按钮。

- [ ] **Step 1: 命令**

`src-tauri/src/commands.rs`：
```rust
/// 重新探测输出设备并重建实时宿主（设备恢复后无需重启应用）。
#[tauri::command]
pub fn retry_audio(state: State<'_, AppState>) -> Result<(), String> {
    state.audio.retry_audio();
    Ok(())
}
```
并在 `src-tauri/src/lib.rs` 的 `invoke_handler` 列表里注册（加在 `transport_status` 附近）。

- [ ] **Step 2: 前端 API**

`src/api/tauri.ts`：
```ts
/** 重新探测输出设备并重建实时宿主（设备变化后无需重启）。 */
export function retryAudio(): Promise<void> {
  if (mockEnabled()) return mockApi.retryAudio();
  return invoke<void>("retry_audio");
}
```
`src/api/mock.ts`：加 `retryAudio: () => Promise.resolve()`（与相邻 mock 方法一致的写法）。

- [ ] **Step 3: 底栏**

`src/components/layout/BottomBar.vue`：把 Realtime/Offline 那个指示改为——可用时不变；不可用时渲染成 `<button class="audio-retry" @click="onRetryAudio">`，文案用 `i18n.t("bottom.offline")`，`title`/`aria-label` 用 `i18n.t("bottom.audioOfflineHint", { error: transport.error ?? i18n.t("bottom.audioUnavailable") })`，并加一个重试图标。`onRetryAudio` 直接 `void retryAudio()`。

- [ ] **Step 4: i18n**

`src/i18n/zh.ts` / `en.ts` 各加：`bottom.retryAudio`（`重试音频` / `Retry audio`）、`bottom.audioOfflineHint`（`音频输出不可用：{error}。点击重试。` / `Audio output unavailable: {error}. Click to retry.`）、`bottom.audioUnavailable`（`未知原因` / `unknown reason`）。

- [ ] **Step 5: 验证**

Run: `npm run build`
Run: `cargo build --manifest-path src-tauri/Cargo.toml`
手动：播放中把系统输出切到耳机 → ≤1s 声音跟随；把输出切到不存在的设备（拔掉 USB 声卡）→ 底栏离线并显示原因 → 点重试（设备插回后）立刻恢复。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs src/api/tauri.ts src/api/mock.ts src/components/layout/BottomBar.vue src/i18n/zh.ts src/i18n/en.ts
git commit -m "feat(audio): manual retry for the output device"
```

---

## Self-Review

**Spec coverage（对照 `2026-10-09-audio-device-resilience-design.md`）**
- 4.1 纯函数 → Task 1。
- 4.2 建流更宽容（格式选择 + 块大小回退）→ Task 2。
- 4.3 运行期错误上报 → Task 3。
- 4.4 轮询 + 自动重建/自动重试 → Task 4。
- 4.5 手动重试 → Task 5。
- 4.6 死按钮 → **计划 E 内的一节**（不在此计划）。
- §6「设备热切换只能人工验证」→ Task 4 Step 4 / Task 5 Step 5 里明确写出「必须人工验证，不得声称已验证」。

**Placeholder 扫描**：无 TBD；`/* …, */` 出现在 Task 4 的代码片段里，是「沿用上面已列出的参数表」的缩写，实现时必须展开为完整参数（Task 4 Step 2 已列出完整签名）。

**类型一致性**：`OutputFormat` / `OutputFormatRange` / `choose_output_format` / `DeviceSignature` / `default_output_signature` / `device_changed` / `SharedStatus` 都在 Task 1/3 定义，Task 2/4 使用；`RealtimeOptions.status` 在 Task 3 定义并被 Task 3/4 使用；`AudioMessage::Retry` 与 `AudioEngine::retry_audio` 在 Task 4 定义，Task 5 使用；`retryAudio()` 在 Task 5 定义并使用。

**风险提示**：Task 4 的重构（把 `run_audio_thread` 的循环改造成 `recv_timeout` + `rebuild` 辅助）是本次改动里最容易出错的点——行为必须保持：`Load`/`SetCatalog` 之后位置与播放状态不丢；并且只能在**真的需要时**才重建（`needs` 判断），否则每秒重建会一直卡顿。

---

## 执行记录（随进度更新）

### 已完成

- **Task 1 — 完成**（commit `9f38b4f`）。纯函数 `OutputFormat` / `OutputFormatRange`（`covers` / `nearest_rate`）/ `choose_output_format` / `DeviceSignature` / `default_output_signature` / `device_changed` + 6 个单测。
  - 实现时对计划的一处必要修正：**cpal 0.18 已移除 `DeviceTrait::name()`**，改为 `device.description().ok()?.name().to_owned()`（计划里写的 `device.name()` 编译不过）。
  - 复审结论：Approved（无 Critical/Important）。

### 必须知道的坑（第一轮踩过）

- **`realtime` 模块被 feature 门控**：`crates/engine/src/lib.rs:13` 是 `#[cfg(feature = "realtime")] pub mod realtime;`，而 `default = []`。因此
  **`cargo test -p daw-party-engine` 不会编译 `realtime.rs`**——它会报「96 passed」但实际上一个 realtime 测试都没跑。
  正确命令：`cargo test -p daw-party-engine --features realtime`（当前 107 个 lib 测试，含 11 个 realtime 测试）。
  这条已同步进上面的 Global Constraints 与各任务的验证步骤。

### 遗留（不阻塞，但别忘）

- Task 1（Minor）：
  - `choose_output_format` 的并列取舍依赖输入顺序（覆盖分支取「最后一个最大值」，最近分支取「第一个最小值」）；影响可忽略（并列者 `nearest_rate` 相同），但没写进文档。fallback 分支的「优先立体声/更多声道」没有测试覆盖。
  - `nearest_rate` 在 `min_rate > max_rate` 时会 panic（`Ord::clamp`）；cpal 保证不会出现，**Task 2 做投影时应顺手把范围归一化**（`min ≤ max`）。
  - 设备描述名为空时两个不同设备会被判为同一个（未加保护）。
- **Task 4/5 的真实设备热切换只能人工验证**：本机跑起来、播放中切输出设备。任何报告都不得声称已验证。

### 待办

- Task 2（建流更宽容）、Task 3（`SharedStatus` + 错误上报）、Task 4（1s 轮询 + 自动重建/重试）、Task 5（`retry_audio` + 底栏重试按钮）。
