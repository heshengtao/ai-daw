# 录音（C2）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal：** 从音频输入设备录制，落到当前选中轨道的播放头处，成为可播放、可撤销的音频片段。

**Architecture：** 采集走**引擎**：`crates/engine/src/recorder.rs` 用 cpal 输入流 + 无锁 `rtrb` 环 + 消费线程，
组装成 `DecodedAudio`；`AudioHost` 增加输入流生命周期。宿主（src-tauri）把结果写 WAV、塞进 `AssetCache`、
发 `AddAudioClip`。**零 IPC 契约变更**（复用 C1 的 `AudioSource` / `AddAudioClip`）。

**Tech Stack：** Rust 2021、cpal（输入流）、rtrb（SPSC 环）、hound（写 WAV）、Vue 3 + Pinia。

设计见 `../specs/2026-10-10-recording-design.md`。

## Global Constraints

- 许可 Apache-2.0-only；允许清单见 ROADMAP §3（本项目**不新增依赖**：`cpal` / `rtrb` / `hound` 都已在用）。
- `crates/engine` 保持 `#![forbid(unsafe_code)]`（`Recorder` 同样不得引入 `unsafe`）。
- **实时回调零分配**：输入回调（`capture_block`）必须零分配，由新的集成测试硬门禁。
- 新增模型字段一律 `#[serde(default)]`，且**旧字段输出形状不变**（本计划**不改模型**）。
- 每个 Task 末尾：`cargo test --workspace` 全绿；涉及 src-tauri 另跑
  `cargo check --manifest-path src-tauri/Cargo.toml`；涉及前端跑 `npm run build` 并保证 i18n 中英键数一致。
- 提交结尾加 `Co-authored-by: CommandCodeBot <noreply@commandcode.ai>`。

## 文件结构

```
crates/engine/src/recorder.rs             新增：设备枚举 + 采集缓冲 + capture_block + Recorder
crates/engine/src/lib.rs                  Modify：pub mod recorder（realtime 下）+ pub use rtrb
crates/engine/src/realtime.rs             Modify：AudioHost 输入流生命周期 + RealtimeError::NoInputDevice
crates/engine/tests/input_capture_safety.rs  新增：输入回调零分配门禁（单独二进制，见文件内注释）
src-tauri/src/assets.rs                   Modify：AssetCache::insert
src-tauri/src/audio.rs                    Modify：输入相关 AudioMessage + AudioEngine 方法 + input_scope
src-tauri/src/state.rs                    Modify：录音目标（track + start）暂存
src-tauri/src/commands.rs                 Modify：list_input_devices / set_input_device / input_level / record_start / record_stop
src-tauri/src/lib.rs                      Modify：generate_handler! 注册 5 条命令
src/api/types.ts                          Modify：InputDeviceInfo
src/api/tauri.ts                          Modify：5 条命令封装
src/api/mock.ts                           Modify：mock 实现
src/stores/project.ts                     Modify：applyRecorded（套用录音返回的工程）
src/stores/transport.ts                   Modify：录音状态 + 设备/电平 + start/stopRecording
src/components/layout/BottomBar.vue       Modify：录音键 + 输入设备选择 + 电平表
src/i18n/zh.ts / src/i18n/en.ts           Modify：录音相关键（中英一致）
docs/superpowers/ROADMAP.md               Modify：C2 状态翻正（末任务）
```

---

## Task 1: 引擎采集基元（纯逻辑，可测）

**Files:**
- Create: `crates/engine/src/recorder.rs`
- Modify: `crates/engine/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub struct InputDeviceInfo { pub name: String, pub sample_rate: u32, pub channels: u16 }`（`Serialize`）
  - `pub fn list_input_devices() -> Vec<InputDeviceInfo>`
  - `pub fn default_input_device() -> Option<InputDeviceInfo>`
  - `pub struct CaptureBuffer { /* … */ }` with `new(channels, sample_rate)`, `append(f32)`, `is_empty() -> bool`, `frames() -> usize`, `into_decoded() -> Option<DecodedAudio>`
  - `pub fn drain_into(consumer: &mut rtrb::Consumer<f32>, buffer: &mut CaptureBuffer) -> usize`
  - `pub fn capture_block<T: Copy>(data: &[T], channels_in: usize, out_channels: usize, to_f32: impl Fn(T) -> f32, producer: &mut rtrb::Producer<f32>, dropped: &AtomicUsize, scope: &ScopeBuffer, active: &AtomicBool)`

- [ ] **Step 1: 写失败测试（放 `recorder.rs` 底部的 `#[cfg(test)] mod tests`）**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_buffer_assembles_and_converts_to_decoded() {
        let mut buf = CaptureBuffer::new(2, 48_000.0);
        for v in [0.1f32, -0.1, 0.2, -0.2] {
            buf.append(v);
        }
        assert_eq!(buf.frames(), 2, "4 个采样 / 2 声道 = 2 帧");
        assert!(!buf.is_empty());
        let audio = buf.into_decoded().expect("非空必须产出素材");
        assert_eq!(audio.channels, 2);
        assert_eq!(audio.frames(), 2);
        assert_eq!(audio.sample_rate, 48_000.0);
    }

    #[test]
    fn empty_capture_buffer_yields_nothing() {
        let buf = CaptureBuffer::new(1, 44_100.0);
        assert!(buf.is_empty());
        assert!(buf.into_decoded().is_none());
    }

    #[test]
    fn capture_block_downmixes_stereo_to_mono_and_meters() {
        let (mut producer, mut consumer) = rtrb::RingBuffer::<f32>::new(64);
        let dropped = AtomicUsize::new(0);
        let scope = ScopeBuffer::new(64);
        let active = AtomicBool::new(true);
        // 两帧立体声：L/R = (1.0, 0.0) 与 (0.0, 1.0)。
        let block = [1.0f32, 0.0, 0.0, 1.0];
        capture_block(&block, 2, 1, |x| x, &mut producer, &dropped, &scope, &active);
        // 下混成单声道：两帧各 0.5。
        assert_eq!(consumer.pop().unwrap(), 0.5);
        assert_eq!(consumer.pop().unwrap(), 0.5);
        assert!(consumer.pop().is_err(), "只应产出 2 个采样");
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
        let mut meter = [0.0f32; 2];
        scope.read_last(&mut meter);
        assert!((meter[1] - 0.5).abs() < 1e-6, "电平表应看到输入：{meter:?}");
    }

    #[test]
    fn capture_block_skips_pushing_when_inactive_but_still_meters() {
        let (mut producer, mut consumer) = rtrb::RingBuffer::<f32>::new(64);
        let dropped = AtomicUsize::new(0);
        let scope = ScopeBuffer::new(64);
        let active = AtomicBool::new(false);
        capture_block(&[0.5f32, 0.5], 1, 1, |x| x, &mut producer, &dropped, &scope, &active);
        assert!(consumer.pop().is_err(), "未开始录制时不得写环");
        let mut meter = [0.0f32; 1];
        scope.read_last(&mut meter);
        assert!((meter[0] - 0.5).abs() < 1e-6, "未录制时电平表仍应工作");
    }

    #[test]
    fn capture_block_counts_dropped_samples_when_ring_is_full() {
        // 环只有 2 格：推 3 个采样 → 1 个被丢。
        let (mut producer, _consumer) = rtrb::RingBuffer::<f32>::new(2);
        let dropped = AtomicUsize::new(0);
        let scope = ScopeBuffer::new(8);
        let active = AtomicBool::new(true);
        let block = [0.1f32, 0.2, 0.3, 0.4, 0.5, 0.6]; // 单声道 6 帧
        capture_block(&block, 1, 1, |x| x, &mut producer, &dropped, &scope, &active);
        assert_eq!(dropped.load(Ordering::Relaxed), 4, "6 帧 - 2 容量 = 丢 4");
    }

    #[test]
    fn drain_into_moves_everything_available() {
        let (mut producer, mut consumer) = rtrb::RingBuffer::<f32>::new(16);
        let mut buf = CaptureBuffer::new(1, 48_000.0);
        for v in [0.1f32, 0.2, 0.3] {
            producer.push(v).unwrap();
        }
        assert_eq!(drain_into(&mut consumer, &mut buf), 3);
        assert_eq!(buf.frames(), 3);
        // 再抽一次：没东西可抽。
        assert_eq!(drain_into(&mut consumer, &mut buf), 0);
    }

    #[test]
    fn i16_and_u16_convert_to_unit_range() {
        assert!((sample_i16(0) - 0.0).abs() < 1e-6);
        assert!((sample_i16(i16::MAX) - 1.0).abs() < 0.001);
        assert!((sample_u16(u16::MAX) - 1.0).abs() < 0.001);
        assert!((sample_u16(0) + 1.0).abs() < 0.001);
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p ai-daw-engine --features realtime recorder 2>&1 | tail -20`
Expected: 编译失败（`recorder` 模块 / 这些项还不存在）。

- [ ] **Step 3: 实现 `recorder.rs`（只写本任务用到的部分）**

```rust
//! 录音采集：输入设备枚举 + 无锁采集环 + 采集缓冲。
//!
//! 只有「输入设备 → 交错 f32」这一件事。实时回调（[`capture_block`]）只做转换 + `rtrb` push
//! + 原子写，**不分配、不加锁**；组装与出片在消费线程 / 控制侧完成。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use cpal::traits::{DeviceTrait, HostTrait};

use crate::audio::DecodedAudio;
use crate::scope::ScopeBuffer;

/// 一个输入设备的概况（供界面列选）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InputDeviceInfo {
    pub name: String,
    pub sample_rate: u32,
    pub channels: u16,
}

/// 读一个设备的概况；查询失败（无权限 / 无默认配置）返回 `None`。
pub(crate) fn device_info(device: &cpal::Device) -> Option<InputDeviceInfo> {
    let name = device.description().ok()?.name().to_owned();
    let supported = device.default_input_config().ok()?;
    Some(InputDeviceInfo {
        name,
        sample_rate: supported.sample_rate(),
        channels: supported.channels(),
    })
}

/// 本机所有输入设备（查询失败的设备被跳过）。
pub fn list_input_devices() -> Vec<InputDeviceInfo> {
    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };
    devices.filter_map(|d| device_info(&d)).collect()
}

/// 默认输入设备的概况；没有输入设备时 `None`。
pub fn default_input_device() -> Option<InputDeviceInfo> {
    let host = cpal::default_host();
    let device = host.default_input_device()?;
    device_info(&device)
}

/// 采集缓冲：消费线程追加交错 f32，控制侧在结束时取走。
///
/// `channels` 是**输出声道数**（1 或 2，回调已下混），因此 `samples.len()` 恒为 `channels` 的整数倍。
pub struct CaptureBuffer {
    samples: Vec<f32>,
    channels: usize,
    sample_rate: f64,
}

impl CaptureBuffer {
    /// 新建空缓冲。
    pub fn new(channels: usize, sample_rate: f64) -> Self {
        Self { samples: Vec::new(), channels: channels.max(1), sample_rate }
    }

    /// 追加一个采样（消费线程调用；允许分配）。
    pub fn append(&mut self, sample: f32) {
        self.samples.push(sample);
    }

    /// 是否一个采样都没有。
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// 已采集的帧数。
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels
    }

    /// 取走内容做成素材；空缓冲返回 `None`。
    pub fn into_decoded(self) -> Option<DecodedAudio> {
        if self.samples.is_empty() {
            return None;
        }
        Some(DecodedAudio {
            samples: self.samples.into(),
            channels: self.channels,
            sample_rate: self.sample_rate,
        })
    }
}

/// 把环里当前可取的采样全部搬进缓冲；返回搬运的采样数（非阻塞）。
pub fn drain_into(consumer: &mut rtrb::Consumer<f32>, buffer: &mut CaptureBuffer) -> usize {
    let mut moved = 0;
    while let Ok(sample) = consumer.pop() {
        buffer.append(sample);
        moved += 1;
    }
    moved
}

/// i16 → f32（−1.0..=1.0）。
fn sample_i16(value: i16) -> f32 {
    value as f32 / 32_768.0
}

/// u16 → f32（−1.0..=1.0）。
fn sample_u16(value: u16) -> f32 {
    (value as f32 - 32_768.0) / 32_768.0
}

/// 把一块设备采样（交错）下混成 `out_channels` 路、转 f32；录制中则推进环。
///
/// **实时回调路径：零分配、无锁。** 电平表（`scope`）**始终**更新（未录制时也能看电平）；
/// 只有 `active` 为真时才把采样推进环。环满 → 丢帧并在 `dropped` 计数（绝不阻塞）。
pub fn capture_block<T: Copy>(
    data: &[T],
    channels_in: usize,
    out_channels: usize,
    to_f32: impl Fn(T) -> f32,
    producer: &mut rtrb::Producer<f32>,
    dropped: &AtomicUsize,
    scope: &ScopeBuffer,
    active: &AtomicBool,
) {
    if channels_in == 0 || out_channels == 0 {
        return;
    }
    let recording = active.load(Ordering::Relaxed);
    for frame in data.chunks_exact(channels_in) {
        let l = to_f32(frame[0]);
        let r = if channels_in >= 2 { to_f32(frame[1]) } else { l };
        let (o0, o1) = if out_channels == 1 {
            let mono = 0.5 * (l + r);
            (mono, mono)
        } else {
            (l, r)
        };
        // 电平表：始终更新（下混交给 ScopeBuffer）。
        if out_channels >= 2 {
            scope.push_interleaved(&[o0, o1], 1);
        } else {
            scope.push_interleaved(&[o0], 1);
        }
        if !recording {
            continue;
        }
        if producer.push(o0).is_err() {
            dropped.fetch_add(1, Ordering::Relaxed);
        }
        if out_channels >= 2 && producer.push(o1).is_err() {
            dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

// 注意：`sample_i16` / `sample_u16` 在 Task 2 的流构造里使用；`device_info` 被
// `list_input_devices` / `default_input_device` 使用；本任务若出现 `dead_code` 警告，
// 在实现处加 `#[allow(dead_code)]` 并在 Task 2 移除——不要删掉它们。
```

- [ ] **Step 4: `lib.rs` 挂模块 + 再导出 `rtrb`**

在 `crates/engine/src/lib.rs` 的模块声明与 `pub use` 附近加：

```rust
#[cfg(feature = "realtime")]
pub mod recorder;
/// 采集环的 SPSC 类型：`capture_block` / `drain_into` 的公开签名用到它，集成测试也要用。
pub use rtrb;
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p ai-daw-engine --features realtime recorder 2>&1 | tail -30`
Expected: 6 个测试通过。

- [ ] **Step 6: 提交**

```bash
git add crates/engine/src/recorder.rs crates/engine/src/lib.rs
git commit -F - <<'EOF'
feat(engine): recording capture primitives (device list, ring, buffer)

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

## Task 2: `Recorder`（cpal 输入流 + 消费线程）与 `AudioHost` 接线

**Files:**
- Modify: `crates/engine/src/recorder.rs`
- Modify: `crates/engine/src/realtime.rs`
- Create: `crates/engine/tests/input_capture_safety.rs`

**Interfaces:**
- Consumes: Task 1 的 `CaptureBuffer` / `drain_into` / `capture_block` / `device_info` / `sample_i16` / `sample_u16`。
- Produces:
  - `pub struct Recorder` with `open(device: Option<String>, scope: Arc<ScopeBuffer>) -> Result<Recorder, RealtimeError>`, `begin(&mut self)`, `finish(&mut self) -> Option<DecodedAudio>`
  - `AudioHost::arm_input(&mut self, device: Option<String>, scope: Arc<ScopeBuffer>) -> Result<(), RealtimeError>`
  - `AudioHost::disarm_input(&mut self)`
  - `AudioHost::start_recording(&mut self) -> Result<(), RealtimeError>`
  - `AudioHost::stop_recording(&mut self) -> Option<DecodedAudio>`
  - `RealtimeError::NoInputDevice`

- [ ] **Step 1: 写失败测试（追加到 `recorder.rs` 的 `mod tests`）**

```rust
    #[test]
    fn recorder_open_reports_result_without_panicking() {
        // 无设备（CI）→ Err；有设备 → Ok。两种都必须不 panic。
        let scope = std::sync::Arc::new(ScopeBuffer::new(64));
        let opened = Recorder::open(None, scope);
        assert!(opened.is_ok() || opened.is_err());
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p ai-daw-engine --features realtime recorder::tests::recorder_open 2>&1 | tail -20`
Expected: 编译失败（`Recorder` 未定义）。

- [ ] **Step 3: 在 `recorder.rs` 增加 `Recorder`**

在 `capture_block` 之后追加：

```rust
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::thread;

use crate::realtime::RealtimeError;

/// 采集环容量（采样数）：262144 ≈ 2.7 s 立体声 @48 kHz，给消费线程足够余量。
const RING_SAMPLES: usize = 1 << 18;
/// 消费线程轮询间隔。
const DRAIN_INTERVAL: Duration = Duration::from_millis(2);

/// 输入设备采集器：持有 cpal 输入流 + 消费线程。
///
/// - `begin()` 开始把采样累积进缓冲；`finish()` 取走已采集内容（不关闭流，电平表继续工作）。
/// - 流在 `Drop` 时停止并 join 消费线程。
///
/// **`!Send`**（持有 `cpal::Stream`）：只能活在拥有它的音频线程上。
pub struct Recorder {
    stream: cpal::Stream,
    active: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    dropped: Arc<AtomicUsize>,
    buffer: Arc<Mutex<CaptureBuffer>>,
    join: Option<thread::JoinHandle<()>>,
    channels: usize,
    sample_rate: f64,
}

impl Recorder {
    /// 打开（默认或按名字匹配的）输入设备，开始给电平表供数（尚未录制）。
    pub fn open(device: Option<String>, scope: Arc<ScopeBuffer>) -> Result<Recorder, RealtimeError> {
        let host = cpal::default_host();
        let device = match device.as_deref() {
            Some(name) => host
                .input_devices()
                .ok()
                .and_then(|mut ds| ds.find(|d| d.description().map(|x| x.name() == name).unwrap_or(false)))
                .ok_or(RealtimeError::NoInputDevice)?,
            None => host.default_input_device().ok_or(RealtimeError::NoInputDevice)?,
        };
        let supported = device
            .default_input_config()
            .map_err(|e| RealtimeError::UnsupportedConfig(e.to_string()))?;
        let sample_format = supported.sample_format();
        let config = supported.config();
        let channels_in = config.channels as usize;
        let out_channels = channels_in.clamp(1, 2);
        let sample_rate = config.sample_rate as f64;

        let active = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));
        let dropped = Arc::new(AtomicUsize::new(0));
        let buffer = Arc::new(Mutex::new(CaptureBuffer::new(out_channels, sample_rate)));

        let (producer, consumer) = rtrb::RingBuffer::<f32>::new(RING_SAMPLES);

        // 三种采样格式各建一条流：回调只做转换 + push + 电平表（零分配）。
        let build = |producer: rtrb::Producer<f32>| -> Result<cpal::Stream, RealtimeError> {
            let active_cb = Arc::clone(&active);
            let dropped_cb = Arc::clone(&dropped);
            let scope_cb = Arc::clone(&scope);
            let err_scope = Arc::clone(&scope);
            let on_err = move |e: cpal::StreamError| eprintln!("input stream error: {e}");
            let _ = &err_scope;
            macro_rules! stream {
                ($t:ty, $conv:expr) => {{
                    let mut producer = producer;
                    device
                        .build_input_stream(
                            &config,
                            move |data: &[$t], _: &cpal::InputCallbackInfo| {
                                capture_block(
                                    data, channels_in, out_channels, $conv, &mut producer,
                                    &dropped_cb, &scope_cb, &active_cb,
                                )
                            },
                            on_err,
                            None,
                        )
                        .map_err(|e| RealtimeError::Cpal(e.to_string()))
                }};
            }
            match sample_format {
                cpal::SampleFormat::F32 => stream!(f32, |x: f32| x),
                cpal::SampleFormat::I16 => stream!(i16, sample_i16),
                cpal::SampleFormat::U16 => stream!(u16, sample_u16),
                other => Err(RealtimeError::UnsupportedConfig(format!(
                    "输入设备采样格式 {other:?} 不受支持（仅 f32 / i16 / u16）"
                ))),
            }
        };

        let stream = build(producer)?;

        // 消费线程：把环里的采样搬进缓冲（控制侧分配，允许）。
        let buf_cb = Arc::clone(&buffer);
        let run_cb = Arc::clone(&running);
        let mut consumer = consumer;
        let join = thread::Builder::new()
            .name("ai-daw-capture".into())
            .spawn(move || {
                let mut consumer = &mut consumer;
                while run_cb.load(Ordering::Relaxed) {
                    if let Ok(mut buffer) = buf_cb.lock() {
                        drain_into(consumer, &mut buffer);
                    }
                    thread::sleep(DRAIN_INTERVAL);
                }
                // 收尾：把最后一点也搬进去。
                if let Ok(mut buffer) = buf_cb.lock() {
                    drain_into(consumer, &mut buffer);
                }
            })
            .map_err(|e| RealtimeError::Cpal(e.to_string()))?;

        stream.play().map_err(|e| RealtimeError::Cpal(e.to_string()))?;

        Ok(Recorder {
            stream,
            active,
            running,
            dropped,
            buffer,
            join: Some(join),
            channels: out_channels,
            sample_rate,
        })
    }

    /// 开始录制：清空缓冲并让回调把采样推进环。
    pub fn begin(&mut self) {
        if let Ok(mut buffer) = self.buffer.lock() {
            *buffer = CaptureBuffer::new(self.channels, self.sample_rate);
        }
        self.dropped.store(0, Ordering::Relaxed);
        self.active.store(true, Ordering::Relaxed);
    }

    /// 结束录制：停止累积并取走内容（空则 `None`）。输入流保持打开（电平表继续）。
    pub fn finish(&mut self) -> Option<DecodedAudio> {
        self.active.store(false, Ordering::Relaxed);
        let taken = self
            .buffer
            .lock()
            .ok()
            .map(|mut buffer| std::mem::replace(&mut *buffer, CaptureBuffer::new(self.channels, self.sample_rate)));
        taken.and_then(|buffer| buffer.into_decoded())
    }

    /// 本次录制被丢弃的采样数（诊断用）。
    pub fn dropped_samples(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // 先停回调（暂停流），再让消费线程退出，最后 join。
        self.running.store(false, Ordering::Relaxed);
        let _ = self.stream.pause();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p ai-daw-engine --features realtime recorder 2>&1 | tail -30`
Expected: 全部通过（有设备则 `Ok`，无设备则 `Err`）。

- [ ] **Step 5: `AudioHost` 接线（`realtime.rs`）**

在 `RealtimeError` 增加一个变体（放在 `NoDevice` 之后）：

```rust
    /// 系统没有可用的输入设备（或指定名字的设备不存在）。
    #[error("no input device available")]
    NoInputDevice,
```

给 `AudioHost` 结构体加字段（并 import）：

```rust
use crate::audio::DecodedAudio;
use crate::recorder::Recorder;
```

```rust
pub struct AudioHost {
    stream: cpal::Stream,
    commands: CommandSender,
    shared: SharedTransport,
    yield_sink: YieldSink,
    /// 已打开（armed）的输入设备采集器；`None` = 没有输入流。
    recorder: Option<Recorder>,
}
```

在 `start_with` 的 `Ok(AudioHost { … })` 里补 `recorder: None,`。

新增方法（放在 `take_instances` 附近）：

```rust
    /// 打开（或替换）输入设备：供电平表使用，尚未录制。
    ///
    /// 传 `None` 等价于 `disarm_input`（关闭输入流）。
    pub fn arm_input(
        &mut self,
        device: Option<String>,
        scope: Arc<ScopeBuffer>,
    ) -> Result<(), RealtimeError> {
        self.recorder = None;
        match device {
            Some(name) => self.recorder = Some(Recorder::open(Some(name), scope)?),
            None => self.recorder = Some(Recorder::open(None, scope)?),
        }
        Ok(())
    }

    /// 关闭输入流。
    pub fn disarm_input(&mut self) {
        self.recorder = None;
    }

    /// 开始录制（尚未 arm 则报 `NoInputDevice`）。
    pub fn start_recording(&mut self) -> Result<(), RealtimeError> {
        let recorder = self.recorder.as_mut().ok_or(RealtimeError::NoInputDevice)?;
        recorder.begin();
        Ok(())
    }

    /// 结束录制并取走内容（未录制 / 空则 `None`）。输入流保持打开。
    pub fn stop_recording(&mut self) -> Option<DecodedAudio> {
        self.recorder.as_mut().and_then(|recorder| recorder.finish())
    }
```

> 注意 `arm_input` 里 `Some(name)` / `None` 都调用 `Recorder::open`，因为 `open` 自己会
> 「按名字找、找不到退回默认」。若希望 `None` 表示「关闭」，由**调用方**在 `set_input_device(None)`
> 时改调 `disarm_input`（见 Task 3）。这里保留两分支是为了让 `arm_input(Some(..))` 语义明确。

- [ ] **Step 6: 新增输入回调零分配门禁**

Create `crates/engine/tests/input_capture_safety.rs`：

```rust
//! 输入采集回调（`capture_block`）的零分配门禁。
//!
//! 与 `realtime_safety.rs` 一样：`#[global_allocator]` 作用于整个测试二进制，故本文件
//! **只放这一个测试**，避免并行测试污染计数。

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ai_daw_engine::recorder::{capture_block, CaptureBuffer, drain_into};
use ai_daw_engine::scope::ScopeBuffer;

struct CountingAllocator;
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}
#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

#[test]
fn input_capture_path_does_not_allocate() {
    const FRAMES: usize = 512;
    let (mut producer, mut consumer) = rtrb::RingBuffer::<f32>::new(1 << 16);
    let dropped = AtomicUsize::new(0);
    let scope = ScopeBuffer::new(8192);
    let active = AtomicBool::new(true);
    let frames: Vec<f32> = (0..FRAMES * 2).map(|i| (i as f32 * 0.01).sin()).collect();
    let mut buffer = CaptureBuffer::new(2, 48_000.0);

    // 预热：让 ScopeBuffer / 环的第一次写入路径跑通；并预分配 CaptureBuffer。
    capture_block(&frames, 2, 2, |x| x, &mut producer, &dropped, &scope, &active);
    drain_into(&mut consumer, &mut buffer);
    buffer.append(0.0);
    buffer.into_decoded();

    let mut buffer = CaptureBuffer::new(2, 48_000.0);
    ALLOCATIONS.store(0, Ordering::SeqCst);
    for _ in 0..10_000 {
        capture_block(&frames, 2, 2, |x| x, &mut producer, &dropped, &scope, &active);
        drain_into(&mut consumer, &mut buffer);
    }
    let allocations = ALLOCATIONS.load(Ordering::SeqCst);
    assert_eq!(allocations, 0, "输入回调路径不得分配（saw {allocations}）");
}
```

> `rtrb` 通过 `ai_daw_engine::rtrb` 可用（Task 1 已再导出）。若集成测试直接写 `rtrb::`
> 不解析，改成 `use ai_daw_engine::rtrb::RingBuffer;` 或加 `rtrb` 到 `[dev-dependencies]`
> （两者皆可，优先用再导出）。

- [ ] **Step 7: 跑门禁与全量引擎测试**

Run: `cargo test -p ai-daw-engine --features realtime 2>&1 | grep -E "test result|FAILED"`
Expected: 全绿，含 `input_capture_path_does_not_allocate`。

- [ ] **Step 8: 提交**

```bash
git add crates/engine/src/recorder.rs crates/engine/src/realtime.rs crates/engine/tests/input_capture_safety.rs
git commit -F - <<'EOF'
feat(engine): audio input Recorder and AudioHost input lifecycle

Add a cpal input stream + lock-free capture ring + drain thread, and wire
arm/disarm/start/stop recording into AudioHost. Add a zero-allocation gate
for the input callback.

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

## Task 3: 宿主接线（AudioEngine 消息 + 命令 + AssetCache）

**Files:**
- Modify: `src-tauri/src/assets.rs`
- Modify: `src-tauri/src/audio.rs`
- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: Task 2 的 `AudioHost::{arm_input, disarm_input, start_recording, stop_recording}`、`ai_daw_engine::recorder::{list_input_devices, InputDeviceInfo}`。
- Produces（Tauri 命令，前端 Task 4/5 依赖）：
  - `list_input_devices() -> Vec<InputDeviceInfo>`
  - `set_input_device(device: Option<String>) -> Result<(), String>`
  - `input_level(points: usize) -> Vec<f32>`
  - `record_start(track: u64, start: f64, device: Option<String>) -> Result<(), String>`
  - `record_stop() -> Result<Option<Project>, String>`

- [ ] **Step 1: `AssetCache::insert`**

在 `src-tauri/src/assets.rs` 的 `impl AssetCache` 内加：

```rust
    /// 插入一份已解码素材（录音刚落盘时用：免去立刻重解码）。
    pub fn insert(&self, path: &str, audio: Arc<DecodedAudio>) {
        self.map.lock().unwrap().insert(path.to_string(), Some(audio));
    }
```

- [ ] **Step 2: 跑一个失败测试（放在 `assets.rs` 底部新增 `#[cfg(test)] mod tests`）**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_serves_a_predecoded_asset_without_touching_disk() {
        let cache = AssetCache::new();
        let audio = Arc::new(DecodedAudio {
            samples: vec![0.25f32, -0.25].into(),
            channels: 2,
            sample_rate: 48_000.0,
        });
        cache.insert("/nonexistent/rec.wav", Arc::clone(&audio));
        let got = cache.load("/nonexistent/rec.wav").expect("插入过的路径必须直接命中");
        assert_eq!(got.samples.len(), 2);
        assert_eq!(got.sample_rate, 48_000.0);
    }
}
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib assets 2>&1 | tail -20`
Expected: 先失败（`insert` 不存在），加实现后通过。

- [ ] **Step 3: `state.rs` 暂存录音目标**

在 `AppState` 加字段（并 import `TrackId`）：

```rust
    /// 正在进行的一次录音的目标：轨道 + 起始拍（`record_start` 记下，`record_stop` 取走）。
    pub recording: Mutex<Option<RecordingTarget>>,
```

```rust
/// 一次录音的目标位置。
#[derive(Debug, Clone, Copy)]
pub struct RecordingTarget {
    pub track: TrackId,
    pub start: f64,
}
```

在 `AppState::new`（或等价构造处）初始化 `recording: Mutex::new(None),`。加两个方法：

```rust
impl AppState {
    /// 记下本次录音的目标。
    pub fn set_recording(&self, target: Option<RecordingTarget>) {
        *self.recording.lock().unwrap() = target;
    }

    /// 取走（并清空）本次录音的目标。
    pub fn take_recording(&self) -> Option<RecordingTarget> {
        self.recording.lock().unwrap().take()
    }
}
```

- [ ] **Step 4: `audio.rs` 增加输入消息与 AudioEngine 方法**

`AudioMessage` 增加：

```rust
    /// 打开（或替换）输入设备（`None` = 关闭输入流）。回复是否成功。
    ArmInput(Option<String>, Sender<Result<(), String>>),
    /// 开始录制。回复是否成功。
    StartRecording(Sender<Result<(), String>>),
    /// 结束录制并把采集内容交回。回复 `None` = 没录到东西。
    StopRecording(Sender<Option<DecodedAudio>>),
```

`AudioEngine` 增加 `input_scope` 字段，并在 `new` 里建一份（与输出 `scope` 同法）：

```rust
    /// 输入电平表缓冲（音频线程写、界面读），与输出 `scope` 分开。
    input_scope: Arc<ScopeBuffer>,
```

`new` 里：

```rust
        let input_scope = Arc::new(ScopeBuffer::new(SCOPE_CAPACITY));
```
（并 `input_scope: Arc::clone(&input_scope)` 存入 `engine`，把 `input_scope` 传进 `run_audio_thread`。）

`AudioEngine` 方法：

```rust
    /// 打开（或替换）输入设备；`None` = 关闭输入流。
    pub fn arm_input(&self, device: Option<String>) -> Result<(), String> {
        if self.unavailable() {
            return Err(self.unavailable_reason());
        }
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(AudioMessage::ArmInput(device, tx))
            .map_err(|_| "音频线程已退出".to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(3))
            .map_err(|_| "等待音频线程超时".to_string())?
    }

    /// 开始录制。
    pub fn start_recording(&self) -> Result<(), String> {
        if self.unavailable() {
            return Err(self.unavailable_reason());
        }
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(AudioMessage::StartRecording(tx))
            .map_err(|_| "音频线程已退出".to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(3))
            .map_err(|_| "等待音频线程超时".to_string())?
    }

    /// 结束录制并取回采集内容。
    pub fn stop_recording(&self) -> Result<Option<DecodedAudio>, String> {
        let (tx, rx) = mpsc::channel();
        if self.tx.send(AudioMessage::StopRecording(tx)).is_err() {
            return Ok(None);
        }
        match rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(captured) => Ok(captured),
            Err(_) => Ok(None),
        }
    }

    /// 输入电平表：最近 `points` 个采样（供界面画条）。
    pub fn input_scope_snapshot(&self, points: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; points.clamp(16, SCOPE_CAPACITY)];
        self.input_scope.read_last(&mut out);
        out
    }
```

`run_audio_thread` 增加参数 `input_scope: Arc<ScopeBuffer>`，并在消息分支里处理：

```rust
            Ok(AudioMessage::ArmInput(device, reply)) => {
                let result = match host.as_mut() {
                    Some(h) => match &device {
                        Some(_) => h.arm_input(device.clone(), Arc::clone(&input_scope)),
                        None => {
                            h.disarm_input();
                            Ok(())
                        }
                    }
                    .map_err(|e| e.to_string()),
                    None => Err("音频输出不可用，无法打开输入设备".to_string()),
                };
                let _ = reply.send(result);
            }
            Ok(AudioMessage::StartRecording(reply)) => {
                let result = match host.as_mut() {
                    Some(h) => h.start_recording().map_err(|e| e.to_string()),
                    None => Err("音频输出不可用，无法录音".to_string()),
                };
                let _ = reply.send(result);
            }
            Ok(AudioMessage::StopRecording(reply)) => {
                let captured = host.as_mut().and_then(|h| h.stop_recording());
                let _ = reply.send(captured);
            }
```

> `Recorder` 持有 `cpal::Stream`，与 `AudioHost` 一样**只在音频线程**创建与丢弃——
> 上面三条都在 `run_audio_thread` 里执行，满足该约束。

- [ ] **Step 5: `commands.rs` 增加 5 条命令**

在「音频素材（导入 / 波形）」一节附近追加：

```rust
// ---------------------------------------------------------------------------
// 录音（C2）
// ---------------------------------------------------------------------------

/// 本机可用的音频输入设备。
#[tauri::command]
pub fn list_input_devices() -> Vec<ai_daw_engine::recorder::InputDeviceInfo> {
    ai_daw_engine::recorder::list_input_devices()
}

/// 选择输入设备（`None` = 关闭输入流）。选定后输入电平表即可工作。
#[tauri::command]
pub fn set_input_device(state: State<'_, AppState>, device: Option<String>) -> Result<(), String> {
    state.audio.arm_input(device)
}

/// 输入电平表：最近 `points` 个采样。
#[tauri::command]
pub fn input_level(state: State<'_, AppState>, points: usize) -> Vec<f32> {
    state.audio.input_scope_snapshot(points)
}

/// 开始录音：打开输入流（若设备有变）、开始采集，并从播放头走带。
#[tauri::command]
pub fn record_start(
    state: State<'_, AppState>,
    track: u64,
    start: f64,
    device: Option<String>,
) -> Result<(), String> {
    state.audio.arm_input(device)?;
    state.audio.start_recording()?;
    state.set_recording(Some(crate::state::RecordingTarget { track: TrackId(track), start }));
    let _ = state.audio.send(TransportCommand::Play);
    Ok(())
}

/// 结束录音：停带、写 WAV、挂到轨道上（一个撤销步骤）。没录到内容返回 `None`。
#[tauri::command]
pub async fn record_stop(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<Project>, String> {
    let _ = state.audio.send(TransportCommand::Pause);
    let target = state.take_recording();
    let captured = state.audio.stop_recording()?;
    let (Some(target), Some(audio)) = (target, captured) else {
        return Ok(None);
    };
    if audio.frames() == 0 {
        return Ok(None);
    }

    // 落盘：应用数据目录 /recordings/rec-<epochms>.wav
    let path = recording_path(&app)?;
    let audio = Arc::new(audio);
    let for_write = Arc::clone(&audio);
    let path_for_write = path.clone();
    let rate = audio.sample_rate.round().max(1.0) as u32;
    tauri::async_runtime::spawn_blocking(move || {
        let mut buffer = ai_daw_engine::audio::AudioBuffer::silence(for_write.channels, for_write.frames());
        buffer.data_mut().copy_from_slice(&for_write.samples);
        ai_daw_engine::wav::write_wav(&path_for_write, &buffer, rate).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;

    // 免去立刻重解码，并追加一条音频片段（可撤销）。
    state.assets.insert(&path, Arc::clone(&audio));
    let start = ai_daw_engine::model::Clip::normalize_start(target.start);
    let length = {
        let engine = state.engine.lock().unwrap();
        natural_length_beats(engine.project(), start, audio.frames() as f64 / audio.sample_rate)
    };
    Ok(Some(run(
        &app,
        &state,
        Command::AddAudioClip {
            track: target.track,
            start,
            length,
            source: AudioSource { path, offset_frames: 0, frames: Some(audio.frames() as u64) },
        },
    )))
}

/// 录音文件路径：应用数据目录下的 `recordings/rec-<epochms>.wav`。
fn recording_path(app: &tauri::AppHandle) -> Result<String, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("拿不到应用数据目录：{e}"))?
        .join("recordings");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    Ok(dir.join(format!("rec-{stamp}.wav")).to_string_lossy().into_owned())
}
```

> `run` 是既有的「署名 Human、返回工程快照」的入口；`natural_length_beats` / `AudioSource` /
> `TrackId` / `Command` 已在本文件使用。确认 `use` 里有 `Arc`、`Command`、`AudioSource`、
> `Project`、`TrackId`、`TransportCommand`（缺哪个补哪个）。

- [ ] **Step 6: `lib.rs` 注册命令**

在 `generate_handler![…]` 里加：

```rust
            commands::list_input_devices,
            commands::set_input_device,
            commands::input_level,
            commands::record_start,
            commands::record_stop,
```

- [ ] **Step 7: 编译与宿主单测**

Run: `cargo check --manifest-path src-tauri/Cargo.toml && cargo test --manifest-path src-tauri/Cargo.toml --lib 2>&1 | tail -20`
Expected: 编译通过；`assets::insert` 单测通过。

- [ ] **Step 8: 提交**

```bash
git add src-tauri/src/assets.rs src-tauri/src/audio.rs src-tauri/src/state.rs src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -F - <<'EOF'
feat(host): recording commands (input device, level, start/stop)

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

## Task 4: 前端接线（类型 / API / mock / store / i18n）

**Files:**
- Modify: `src/api/types.ts`
- Modify: `src/api/tauri.ts`
- Modify: `src/api/mock.ts`
- Modify: `src/stores/project.ts`
- Modify: `src/stores/transport.ts`
- Modify: `src/i18n/zh.ts`, `src/i18n/en.ts`

**Interfaces:**
- Consumes: Task 3 的 5 条命令。
- Produces（Task 5 依赖）：
  - `project.applyRecorded(next: Project): Promise<void>`
  - transport store：`recording`、`inputDevices`、`inputDevice`、`inputLevelPeak`、`refreshInputDevices()`、`selectInput(device: string | null)`、`startRecording(track: number, startBeat: number)`、`stopRecording()`

- [ ] **Step 1: `types.ts` 加类型**

```ts
/** 一个音频输入设备（与后端 `recorder::InputDeviceInfo` 一一对应）。 */
export interface InputDeviceInfo {
  name: string;
  sample_rate: number;
  channels: number;
}
```

- [ ] **Step 2: `tauri.ts` 加命令封装**

```ts
/** 本机可用的音频输入设备。 */
export function listInputDevices(): Promise<InputDeviceInfo[]> {
  if (mockEnabled()) return Promise.resolve(mockApi.listInputDevices());
  return invoke<InputDeviceInfo[]>("list_input_devices");
}

/** 选择输入设备（null = 关闭输入流）。 */
export function setInputDevice(device: string | null): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.setInputDevice());
  return invoke<void>("set_input_device", { device });
}

/** 输入电平表：最近 `points` 个采样。 */
export function inputLevel(points: number): Promise<number[]> {
  if (mockEnabled()) return Promise.resolve(mockApi.inputLevel(points));
  return invoke<number[]>("input_level", { points });
}

/** 开始录音（从 `start` 拍起，走带播放）。 */
export function recordStart(track: number, start: number, device: string | null): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.recordStart());
  return invoke<void>("record_start", { track, start, device });
}

/** 结束录音；返回带新片段的工程，没录到内容则为 null。 */
export function recordStop(): Promise<Project | null> {
  if (mockEnabled()) return Promise.resolve(mockApi.recordStop());
  return invoke<Project | null>("record_stop");
}
```

（`InputDeviceInfo` 要加进 `types.ts` 的 import 列表。）

- [ ] **Step 3: `mock.ts` 加 mock**

在 `mockApi` 对象里加：

```ts
  listInputDevices: (): InputDeviceInfo[] => [
    { name: "内置麦克风", sample_rate: 48000, channels: 1 },
  ],
  setInputDevice: (): void => undefined,
  inputLevel: (points: number): number[] =>
    Array.from({ length: Math.max(1, points) }, (_, i) => Math.sin(i / 6) * 0.3),
  recordStart: (): void => undefined,
  // 浏览器预览没有真实采集：如实返回 null（界面提示「没有录到内容」）。
  recordStop: (): Project | null => null,
```

- [ ] **Step 4: `project.ts` 加 `applyRecorded`**

```ts
  /** 套用一次录音返回的工程快照（录音本身已是后端的一次可撤销编辑）。 */
  async function applyRecorded(next: Project): Promise<void> {
    project.value = next;
    await refreshDepths();
  }
```

并导出：在 `return { … }` 里加 `applyRecorded,`。

- [ ] **Step 5: `transport.ts` 加录音状态与动作**

在 store 顶部 import：`import type { InputDeviceInfo } from "@/api/types";`

```ts
  // ---- 录音（C2） ----

  /** 是否正在录音。 */
  const recording = ref(false);
  /** 可选的输入设备列表。 */
  const inputDevices = ref<InputDeviceInfo[]>([]);
  /** 当前选中的输入设备名（null = 未选）。 */
  const inputDevice = ref<string | null>(null);
  /** 输入电平表最近一帧的峰值（0..1）。 */
  const inputLevelPeak = ref(0);

  /** 拉取输入设备列表。 */
  async function refreshInputDevices(): Promise<void> {
    try {
      inputDevices.value = await api.listInputDevices();
    } catch {
      inputDevices.value = [];
    }
  }

  /** 选择输入设备（null = 关闭输入流）。 */
  async function selectInput(device: string | null): Promise<void> {
    inputDevice.value = device;
    try {
      await api.setInputDevice(device);
    } catch (err) {
      layout.notify(String(err));
    }
  }

  /** 读一次输入电平（供界面画条）。 */
  async function pollInputLevel(points = 64): Promise<void> {
    try {
      const samples = await api.inputLevel(points);
      inputLevelPeak.value = samples.reduce((m, v) => Math.max(m, Math.abs(v)), 0);
    } catch {
      inputLevelPeak.value = 0;
    }
  }

  /** 开始录音：记录起点并走带。 */
  async function startRecording(track: number, startBeat: number): Promise<void> {
    try {
      await api.recordStart(track, startBeat, inputDevice.value);
      recording.value = true;
      playing.value = true;
    } catch (err) {
      layout.notify(String(err));
    }
  }

  /** 结束录音：停带并套用返回的工程。 */
  async function stopRecording(): Promise<void> {
    if (!recording.value) return;
    recording.value = false;
    try {
      const next = await api.recordStop();
      playing.value = false;
      if (next) await project.applyRecorded(next);
      else layout.notify(i18n.t("bottom.recordEmpty"));
    } catch (err) {
      layout.notify(i18n.t("error.record", { error: String(err) }));
    }
  }
```

并导出这些（放进 `return { … }`）。

- [ ] **Step 6: i18n 键（中英各加，键集必须一致）**

`zh.ts`：

```ts
  "bottom.record": "录音",
  "bottom.recordLabel": "开始录音",
  "bottom.recordStop": "停止录音",
  "bottom.recordStopLabel": "停止录音",
  "bottom.recordInput": "输入设备",
  "bottom.recordInputNone": "未选择输入",
  "bottom.recordInputLabel": "音频输入设备",
  "bottom.recordLevel": "输入电平",
  "bottom.recordEmpty": "没有录到内容",
  "error.record": "录音失败：{error}",
```

`en.ts`：

```ts
  "bottom.record": "Record",
  "bottom.recordLabel": "Start recording",
  "bottom.recordStop": "Stop recording",
  "bottom.recordStopLabel": "Stop recording",
  "bottom.recordInput": "Input device",
  "bottom.recordInputNone": "No input selected",
  "bottom.recordInputLabel": "Audio input device",
  "bottom.recordLevel": "Input level",
  "bottom.recordEmpty": "Nothing was recorded",
  "error.record": "Recording failed: {error}",
```

- [ ] **Step 7: 构建并核对 i18n 键数**

Run: `npm run build`
Expected: 构建通过。再核对中英键数一致（用现有习惯做法）：

Run: `node -e "const z=require('fs').readFileSync('src/i18n/zh.ts','utf8'),e=require('fs').readFileSync('src/i18n/en.ts','utf8');const c=s=>(s.match(/^\s*\"[^\"]+\":/gm)||[]).length;console.log('zh',c(z),'en',c(e))"`
Expected: 两数相等。

- [ ] **Step 8: 提交**

```bash
git add src/api/types.ts src/api/tauri.ts src/api/mock.ts src/stores/project.ts src/stores/transport.ts src/i18n/zh.ts src/i18n/en.ts
git commit -F - <<'EOF'
feat(ui): recording API, store state and i18n

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

## Task 5: 底部条录音 UI + 端到端验证 + 文档翻正

**Files:**
- Modify: `src/components/layout/BottomBar.vue`
- Modify: `docs/superpowers/ROADMAP.md`
- Modify: `docs/superpowers/specs/2026-10-10-recording-design.md`（状态行）

**Interfaces:**
- Consumes: Task 4 的 transport store 录音 API。

- [ ] **Step 1: `BottomBar.vue` 加录音键（中间组，停止键右侧）**

script 里加：

```ts
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import Md3Select from "@/components/common/Md3Select.vue";
import { useSelectionStore } from "@/stores/selection";
import { useTransportStore } from "@/stores/transport";

const selection = useSelectionStore();

/** 录音目标轨道：选中轨道，否则首轨。 */
const recordTrack = computed(() => selection.trackId ?? store.project?.tracks[0]?.id ?? null);

/** 输入设备下拉选项。 */
const inputOptions = computed(() =>
  transport.inputDevices.map((d) => ({ id: d.name, label: d.name, hint: `${d.sample_rate} Hz · ${d.channels}ch` })),
);

/** 电平表宽度（0..100%）。 */
const levelPct = computed(() => `${Math.round(Math.min(1, transport.inputLevelPeak) * 100)}%`);

let levelTimer: number | null = null;

/** 切设备时重建输入流。 */
async function onSelectInput(name: string): Promise<void> {
  await transport.selectInput(name);
}

/** 录音开关：开始前把起点吸附到网格。 */
async function onToggleRecord(): Promise<void> {
  if (transport.recording) {
    await transport.stopRecording();
    return;
  }
  if (recordTrack.value === null) return;
  await transport.startRecording(recordTrack.value, timeline.snapBeats(transport.positionBeats));
}

onMounted(async () => {
  await transport.refreshInputDevices();
  if (transport.inputDevice === null && transport.inputDevices.length > 0) {
    await transport.selectInput(transport.inputDevices[0]!.name);
  }
  levelTimer = window.setInterval(() => void transport.pollInputLevel(), 120);
});
onUnmounted(() => {
  if (levelTimer !== null) window.clearInterval(levelTimer);
});
```

> `onMounted` / `onUnmounted` 若与现有 import 冲突，按现有写法合并（本组件已 import `computed, ref, watch`）。

- [ ] **Step 2: 模板加控件**

在中间组 `.core` 的「停止」按钮之后加录音键：

```html
      <Tooltip
        :text="transport.recording ? i18n.t('bottom.recordStop') : i18n.t('bottom.record')"
        place="above"
        align="center"
      >
        <button
          class="rec-btn"
          :class="{ on: transport.recording }"
          :disabled="recordTrack === null"
          :aria-pressed="transport.recording"
          :aria-label="transport.recording ? i18n.t('bottom.recordStopLabel') : i18n.t('bottom.recordLabel')"
          @click="onToggleRecord"
        >
          <span class="rec-dot" />
        </button>
      </Tooltip>
```

在左栏 `.zone.left` 的 `TimeSignatureField` 之前加输入设备 + 电平表：

```html
      <Md3Select
        :model-value="transport.inputDevice"
        :options="inputOptions"
        :label="i18n.t('bottom.recordInputLabel')"
        :placeholder="i18n.t('bottom.recordInputNone')"
        @update:model-value="onSelectInput"
      />
      <span class="level-meter" :title="i18n.t('bottom.recordLevel')">
        <span class="level-fill" :style="{ width: levelPct }" />
      </span>
```

样式（`<style scoped>` 末尾）：

```css
.rec-btn {
  width: 32px;
  height: 32px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  cursor: pointer;
}
.rec-btn:disabled {
  opacity: 0.35;
  cursor: default;
}
.rec-dot {
  width: 14px;
  height: 14px;
  border-radius: 999px;
  background-color: rgb(var(--md-error));
  transition: box-shadow var(--md-ease-emphasized), transform var(--md-ease-emphasized);
}
.rec-btn.on .rec-dot {
  box-shadow: 0 0 0 4px rgb(var(--md-error) / 0.25);
  transform: scale(1.1);
}
.level-meter {
  display: inline-block;
  width: 48px;
  height: 8px;
  flex: 0 0 auto;
  border-radius: 999px;
  overflow: hidden;
  background-color: rgb(var(--md-surface-container-high));
}
.level-fill {
  display: block;
  height: 100%;
  width: 0;
  background-color: rgb(var(--md-primary));
  transition: width 120ms linear;
}
@media (max-width: 1080px) {
  .level-meter { display: none; }
}
```

- [ ] **Step 3: 构建**

Run: `npm run build`
Expected: 通过。

- [ ] **Step 4: 全量验证**

Run:
```bash
cargo test --workspace
cargo test -p ai-daw-engine --features realtime
cargo test -p ai-daw-engine --features realtime --test input_capture_safety
cargo check --manifest-path src-tauri/Cargo.toml
npm run build
```
Expected: 全绿。

- [ ] **Step 5: 真机端到端（必须真机，不得声称已验证）**

1. 选输入设备 → 对麦克风说话，底部电平条随声音动。
2. 选中一条轨道 → 按录音键 → 走带开始、按键高亮。
3. 出声 → 按停止 → 轨道上出现音频片段（波形可见），播放能听到刚录的内容。
4. `Cmd/Ctrl+Z` 一次即移除该片段。
5. 确认既有能力不回归：导入音频、播放/暂停/定位、导出 WAV。

- [ ] **Step 6: 文档翻正**

- `specs/2026-10-10-recording-design.md` 顶部状态行改为「**已完成**」并简列交付。
- `ROADMAP.md`：§1 总览表把 **Plan 5 / C2 录音** 标为 ✅；Stage C 的 C2 条目改写为完成说明（附 spec/plan 路径）；
  §7 文档索引补 `specs/2026-10-10-recording-design.md` 与 `plans/2026-10-10-recording.md`。

- [ ] **Step 7: 提交**

```bash
git add src/components/layout/BottomBar.vue docs/superpowers
git commit -F - <<'EOF'
feat(ui): record button, input device picker and level meter

Wire recording into the bottom bar and flip the C2 docs to done.

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

## 自检 / Self-Review

- **Spec coverage**（对照 `2026-10-10-recording-design.md`）：
  - §2 目标（录到选中轨道、可选设备、录前电平、可撤销）→ Task 1–5 全覆盖。
  - §3 决策（引擎归属 / 零契约 / 原生采样率 / app_data 落盘 / 不监听 / 不补偿 / 选定即开流）→ Task 1–3 实现。
  - §4 数据流（回调→环→消费线程→DecodedAudio→WAV+AssetCache+AddAudioClip）→ Task 1–3。
  - §5 命令（5 条）→ Task 3。
  - §6 前端（录音键 / 设备下拉 / 电平表 / store / i18n）→ Task 4–5。
  - §7 错误处理（无设备 / 拔出 / 环满 / 空录制 / 写盘失败）→ Task 3 命令返回 + Task 4 store 提示 + Task 5 UI。
  - §8 验收（引擎单测 / 零分配 / 宿主 / 前端 / 真机）→ 对应各 Task 的测试与验证步骤。
  - §10 风险（零分配 / 时钟漂移 / 非 f32 格式 / 延迟 / 孤儿文件 / 重建隔离）→ 门禁、格式分支、决策记录覆盖。
- **Placeholder 扫描**：无 TODO / 「稍后实现」；每个代码步骤都给了可编译内容。
- **类型一致性**：`InputDeviceInfo`（Rust ↔ TS）字段同名；`record_start/record_stop` 参数与 Task 4 封装一致；
  `RecordingTarget { track: TrackId, start: f64 }` 与 `record_stop` 用法一致；`recording`/`applyRecorded` 命名前后一致。
- **已知取舍（不阻塞）**：录音需输出宿主可用（音频线程存在）；`arm_input` 的 `Some(None)` 分支语义见 Task 2 注记；
  录音不删文件（撤销成孤儿素材）与导入同一策略。
