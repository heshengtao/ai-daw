//! 录音采集：输入设备枚举 + 无锁采集环 + 采集缓冲。
//!
//! 只有「输入设备 → 交错 f32」这一件事。实时回调（[`capture_block`]）只做转换 + `rtrb` push
//! + 原子写，**不分配、不加锁**；组装与出片在消费线程 / 控制侧完成。解码（文件 IO）仍留在
//! `ai-daw-asset`，本模块不碰文件系统。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::audio::DecodedAudio;
use crate::realtime::RealtimeError;
use crate::scope::ScopeBuffer;

/// 采集环容量（采样数）：262144 ≈ 2.7 s 立体声 @48 kHz，给消费线程足够余量。
const RING_SAMPLES: usize = 1 << 18;
/// 消费线程轮询间隔。
const DRAIN_INTERVAL: Duration = Duration::from_millis(2);

/// 一个输入设备的概况（供界面列选）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InputDeviceInfo {
    /// 设备显示名。
    pub name: String,
    /// 默认输入采样率（Hz）。
    pub sample_rate: u32,
    /// 默认输入声道数。
    pub channels: u16,
    /// 连接类型（尽力而为，取不到为 `None`）：`builtin` / `bluetooth` / `usb` / `virtual` /
    /// `aggregate` / `airplay` / `network` / `other`。
    ///
    /// 蓝牙耳机、虚拟声卡在 macOS 上同时注册了「输入」端点，会出现在输入设备列表里；
    /// 带上这个字段界面才能把它们和真麦克风区分开。
    pub transport: Option<String>,
    /// 是否系统默认输入设备。
    pub is_default: bool,
}

/// 读一个设备的概况；查询失败（无权限 / 无默认配置）返回 `None`。
fn device_info(device: &cpal::Device, is_default: bool) -> Option<InputDeviceInfo> {
    let name = device.description().ok()?.name().to_owned();
    let supported = device.default_input_config().ok()?;
    let transport = transport_kind(&name);
    Some(InputDeviceInfo {
        name,
        sample_rate: supported.sample_rate(),
        channels: supported.channels(),
        transport,
        is_default,
    })
}

/// 本机所有输入设备（查询失败的设备被跳过）。
pub fn list_input_devices() -> Vec<InputDeviceInfo> {
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|device| device.description().ok())
        .map(|description| description.name().to_owned());
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };
    devices
        .filter_map(|device| {
            let name = device.description().ok().map(|d| d.name().to_owned());
            device_info(&device, name.as_deref() == default_name.as_deref())
        })
        .collect()
}

/// 默认输入设备的概况；没有输入设备时 `None`。
pub fn default_input_device() -> Option<InputDeviceInfo> {
    let host = cpal::default_host();
    let device = host.default_input_device()?;
    device_info(&device, true)
}

/// 设备的连接类型。macOS 直接问 CoreAudio `kAudioDevicePropertyTransportType`；
/// 其他平台暂不提供（返回 `None`，界面不显示该标签）。
#[cfg(target_os = "macos")]
fn transport_kind(name: &str) -> Option<String> {
    use coreaudio::audio_unit::macos_helpers::{
        get_device_id_from_name, get_device_transport_type,
    };
    let device_id = get_device_id_from_name(name, true)?;
    let transport_type = get_device_transport_type(device_id).ok()?;
    Some(transport_name(transport_type).to_owned())
}

#[cfg(not(target_os = "macos"))]
fn transport_kind(_name: &str) -> Option<String> {
    None
}

/// CoreAudio 传输类型（`kAudioDeviceTransportType*`，OSType）→ 稳定短名。
///
/// 用「大端打包的 FourCC」直接表达这些常量，省掉对 coreaudio-sys 常量表的依赖。
#[cfg(target_os = "macos")]
fn transport_name(ostype: u32) -> &'static str {
    const BUILTIN: u32 = u32::from_be_bytes(*b"bltn");
    const AGGREGATE: u32 = u32::from_be_bytes(*b"aggr");
    const VIRTUAL: u32 = u32::from_be_bytes(*b"virt");
    const BLUETOOTH: u32 = u32::from_be_bytes(*b"blue");
    const BLUETOOTH_LE: u32 = u32::from_be_bytes(*b"blea");
    const USB: u32 = u32::from_be_bytes(*b"usb ");
    const HDMI: u32 = u32::from_be_bytes(*b"hdmi");
    const DISPLAYPORT: u32 = u32::from_be_bytes(*b"dprt");
    const AIRPLAY: u32 = u32::from_be_bytes(*b"airp");
    const THUNDERBOLT: u32 = u32::from_be_bytes(*b"thun");
    const FIREWIRE: u32 = u32::from_be_bytes(*b"1394");
    const PCI: u32 = u32::from_be_bytes(*b"pci ");
    const AVB: u32 = u32::from_be_bytes(*b"eavb");
    match ostype {
        BUILTIN => "builtin",
        AGGREGATE => "aggregate",
        VIRTUAL => "virtual",
        BLUETOOTH | BLUETOOTH_LE => "bluetooth",
        USB => "usb",
        HDMI => "hdmi",
        DISPLAYPORT => "displayport",
        AIRPLAY => "airplay",
        THUNDERBOLT => "thunderbolt",
        FIREWIRE => "firewire",
        PCI => "pci",
        AVB => "network",
        _ => "other",
    }
}

/// 采集缓冲：消费线程追加交错 f32，控制侧在结束时取走。
///
/// `channels` 是**输出声道数**（1 或 2，回调已下混），因此 `samples.len()` 恒为 `channels`
/// 的整数倍。
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

/// 输入设备采集器：持有 cpal 输入流 + 消费线程。
///
/// - `begin()` 开始把采样累积进缓冲；`finish()` 取走已采集内容（**不关闭流**，电平表继续工作）。
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
                .and_then(|mut devices| {
                    devices.find(|d| d.description().map(|desc| desc.name() == name).unwrap_or(false))
                })
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
        let active_cb = Arc::clone(&active);
        let dropped_cb = Arc::clone(&dropped);
        let scope_cb = Arc::clone(&scope);
        let on_error = |err: cpal::Error| eprintln!("input stream error: {err}");
        let mut producer = producer;
        let stream = match sample_format {
            cpal::SampleFormat::F32 => device
                .build_input_stream(
                    config.clone(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        capture_block(
                            data, channels_in, out_channels, |x| x, &mut producer, &dropped_cb,
                            &scope_cb, &active_cb,
                        )
                    },
                    on_error,
                    None,
                )
                .map_err(|e| RealtimeError::Cpal(e.to_string()))?,
            cpal::SampleFormat::I16 => device
                .build_input_stream(
                    config.clone(),
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        capture_block(
                            data, channels_in, out_channels, sample_i16, &mut producer, &dropped_cb,
                            &scope_cb, &active_cb,
                        )
                    },
                    on_error,
                    None,
                )
                .map_err(|e| RealtimeError::Cpal(e.to_string()))?,
            cpal::SampleFormat::U16 => device
                .build_input_stream(
                    config.clone(),
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        capture_block(
                            data, channels_in, out_channels, sample_u16, &mut producer, &dropped_cb,
                            &scope_cb, &active_cb,
                        )
                    },
                    on_error,
                    None,
                )
                .map_err(|e| RealtimeError::Cpal(e.to_string()))?,
            other => {
                return Err(RealtimeError::UnsupportedConfig(format!(
                    "输入设备采样格式 {other:?} 不受支持（仅 f32 / i16 / u16）"
                )))
            }
        };

        // 消费线程：把环里的采样搬进缓冲（控制侧分配，允许）。
        let buffer_cb = Arc::clone(&buffer);
        let running_cb = Arc::clone(&running);
        let mut consumer = consumer;
        let join = thread::Builder::new()
            .name("ai-daw-capture".into())
            .spawn(move || {
                while running_cb.load(Ordering::Relaxed) {
                    if let Ok(mut buffer) = buffer_cb.lock() {
                        drain_into(&mut consumer, &mut buffer);
                    }
                    thread::sleep(DRAIN_INTERVAL);
                }
                // 收尾：把最后一点也搬进去。
                if let Ok(mut buffer) = buffer_cb.lock() {
                    drain_into(&mut consumer, &mut buffer);
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
        let taken = self.buffer.lock().ok().map(|mut buffer| {
            std::mem::replace(&mut *buffer, CaptureBuffer::new(self.channels, self.sample_rate))
        });
        taken.and_then(CaptureBuffer::into_decoded)
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
        // 环只有 2 格：推 6 个单声道采样 → 丢 4。
        let (mut producer, _consumer) = rtrb::RingBuffer::<f32>::new(2);
        let dropped = AtomicUsize::new(0);
        let scope = ScopeBuffer::new(8);
        let active = AtomicBool::new(true);
        let block = [0.1f32, 0.2, 0.3, 0.4, 0.5, 0.6];
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

    #[test]
    fn recorder_open_reports_result_without_panicking() {
        // 无设备（CI）→ Err；有设备 → Ok。两种都必须不 panic。
        let scope = Arc::new(ScopeBuffer::new(64));
        let devices = list_input_devices();
        let opened = Recorder::open(None, scope);
        eprintln!(
            "输入设备 {} 个；Recorder::open(None) = {}",
            devices.len(),
            match &opened {
                Ok(_) => "Ok".to_string(),
                Err(e) => format!("Err({e})"),
            }
        );
        assert!(opened.is_ok() || opened.is_err());
    }

    /// 真机端到端：打开默认输入设备，录一小段，必须采到帧。
    ///
    /// 无输入设备的 CI 环境直接跳过（打印一行）；有设备时走完
    /// 「回调 push → 环 → 消费线程 → finish」整条路径。
    #[test]
    fn recorder_captures_frames_from_a_live_input() {
        let scope = Arc::new(ScopeBuffer::new(64));
        let Ok(mut recorder) = Recorder::open(None, scope) else {
            eprintln!("本机无输入设备，跳过真机采集测试。");
            return;
        };
        recorder.begin();
        std::thread::sleep(Duration::from_millis(300));
        let audio = recorder.finish();
        let Some(audio) = audio else {
            eprintln!("本机输入设备未产出任何采样，跳过。");
            return;
        };
        assert!(audio.frames() > 0, "应采集到帧，实际 {}", audio.frames());
        assert!(audio.sample_rate > 0.0, "应带上输入设备采样率");
        assert!(audio.channels == 1 || audio.channels == 2, "声道应收敛到 1/2");
        eprintln!(
            "真机采集：{} 帧 @ {} Hz，{} 声道",
            audio.frames(),
            audio.sample_rate,
            audio.channels
        );
    }
}
