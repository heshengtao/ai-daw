use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use thiserror::Error;

use crate::command_queue::{command_queue, CommandSender};
use crate::player::Player;
use crate::registry::PluginRegistry;
use crate::schedule::ScheduledEvents;
use crate::scope::ScopeBuffer;
use crate::transport::TransportCommand;

/// 实时宿主选项。
///
/// 采样率不在选项中：设备默认输出采样率才是权威来源，[`AudioHost::start`] 会用它，
/// 并在与 [`ScheduledEvents::sample_rate`] 不一致时报错。
#[derive(Debug, Clone)]
pub struct RealtimeOptions {
    /// 音频回调请求的最大帧数：用于预分配 scratch 缓冲，并作为固定块大小请求设备
    /// （设备拒绝固定块大小时回退到它的默认块大小）。
    pub max_block: usize,
    /// 控制侧 → 音频侧命令队列容量。
    pub command_capacity: usize,
    /// 界面画波形用的缓冲；音频回调每块往里推一份。
    pub scope: Arc<ScopeBuffer>,
}

/// 实时宿主错误。
#[derive(Debug, Error)]
pub enum RealtimeError {
    /// 系统没有可用的默认输出设备。
    #[error("no default output device available")]
    NoDevice,
    /// 设备不支持所需的 f32 输出配置（既不报 f32 范围，默认格式也不是 f32）。
    #[error("device does not support an f32 output configuration: {0}")]
    UnsupportedConfig(String),
    /// 调度表构建时使用的采样率与设备采样率不一致（会导致事件静默错位）。
    #[error("schedule sample rate {schedule} Hz does not match device rate {device} Hz")]
    SampleRateMismatch {
        /// 构建调度表时使用的采样率（Hz）。
        schedule: f64,
        /// 设备默认输出采样率（Hz）。
        device: f64,
    },
    /// 底层 cpal 流错误。
    #[error("cpal error: {0}")]
    Cpal(String),
}

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
pub fn choose_output_format(
    ranges: &[OutputFormatRange],
    preferred_rate: u32,
) -> Option<OutputFormatRange> {
    let f32_only = || {
        ranges
            .iter()
            .copied()
            .filter(|r| r.format == OutputFormat::F32)
    };
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
    // cpal 0.18 起 `DeviceTrait::name()` 被 `description()` 取代（返回结构化描述）。
    let name = device.description().ok()?.name().to_owned();
    let sample_rate = device.default_output_config().ok()?.sample_rate();
    Some(DeviceSignature { name, sample_rate })
}

/// 设备是否发生变化：身份（名字或采样率）不同、或一侧有一侧无设备都算变化；
/// 两边都没有设备时不算（真要有问题，调用方还会靠「宿主为空」触发重试）。
pub fn device_changed(
    previous: Option<&DeviceSignature>,
    current: Option<&DeviceSignature>,
) -> bool {
    previous != current
}

/// 把交错立体声 `staging`（`frames*2`）映射到设备的 `channels` 路交错 `data`（`frames*channels`）。
///
/// 单声道设备取左右声道均值；其它设备把 L/R 写入前两路，其余通道清零。纯函数，无分配。
pub fn write_interleaved(staging: &[f32], data: &mut [f32], frames: usize, channels: usize) {
    for frame in 0..frames {
        let l = staging[frame * 2];
        let r = staging[frame * 2 + 1];
        let base = frame * channels;
        if channels == 1 {
            data[base] = 0.5 * (l + r);
        } else {
            data[base] = l;
            data[base + 1] = r;
            for c in 2..channels {
                data[base + c] = 0.0;
            }
        }
    }
}

/// 控制侧可观测的传输状态：音频线程写入，任意线程读取。
///
/// 单独抽出来是因为 `cpal::Stream` 在多数平台上是 `!Send`，宿主必须被创建并持有在
/// 专用音频线程里；而界面需要随时读取播放位置与播放状态。`SharedTransport` 就是这条
/// 跨线程通道：音频线程重建宿主时复用同一个实例，因此对外可观测状态不会随重建丢失。
#[derive(Clone, Default)]
pub struct SharedTransport {
    position: Arc<AtomicU64>,
    playing: Arc<AtomicBool>,
}

impl SharedTransport {
    /// 新建一份共享状态（初始位置 0、未播放）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 音频线程最近一次报告的播放位置（帧）。
    pub fn position_frames(&self) -> u64 {
        self.position.load(Ordering::Relaxed)
    }

    /// 音频线程最近一次报告的播放状态。
    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }
}

/// 默认输出设备的采样率（Hz）；无法查询时返回 `None`（例如无音频设备的 CI 环境）。
///
/// 调度表以固定采样率定点，必须在构建之前知道设备采样率，因此需要一个不打开流的
/// 探测入口。
pub fn default_output_sample_rate() -> Option<u32> {
    let host = cpal::default_host();
    let device = host.default_output_device()?;
    let supported = device.default_output_config().ok()?;
    Some(supported.sample_rate())
}

/// 归一化设备报出的一段采样率区间，保证 `min_rate <= max_rate`。
///
/// [`OutputFormatRange::nearest_rate`] 内部用 `Ord::clamp`，`min > max` 会 panic。
/// cpal 的 `min/max_sample_rate` 本应总有 `min <= max`，但个别后端（或未来的 cpal）
/// 可能报出反向的畸形区间；这里投影时兜底交换成合法区间，最多牺牲「选到理想采样率」，
/// 绝不会崩。（Task 1 review 的遗留项。）
fn normalise_rate_span(min_rate: u32, max_rate: u32) -> (u32, u32) {
    (min_rate.min(max_rate), min_rate.max(max_rate))
}

/// 把设备报告的可支持配置投影成纯数据；设备不报告范围（或查询失败）时返回空表。
///
/// 空表会让 [`choose_output_format`] 返回 `None`，`AudioHost::start_with` 据此退回设备
/// 默认配置——即「设备不报范围」时保持旧行为。
fn supported_formats(device: &cpal::Device) -> Vec<OutputFormatRange> {
    let Ok(ranges) = device.supported_output_configs() else {
        return Vec::new();
    };
    ranges
        .map(|r| {
            // cpal 0.18 里 `SampleRate` 就是 `u32` 别名，`min/max_sample_rate()` 直接给数值。
            let (min_rate, max_rate) =
                normalise_rate_span(r.min_sample_rate(), r.max_sample_rate());
            OutputFormatRange {
                format: if r.sample_format() == cpal::SampleFormat::F32 {
                    OutputFormat::F32
                } else {
                    OutputFormat::Other
                },
                channels: r.channels(),
                min_rate,
                max_rate,
            }
        })
        .collect()
}

/// cpal 实时播放宿主：拥有输出流与控制侧命令发送端。
pub struct AudioHost {
    stream: cpal::Stream,
    commands: CommandSender,
    shared: SharedTransport,
}

impl AudioHost {
    /// 打开默认输出设备并开始播放工程（初始为暂停态，需调用 `play`）。
    ///
    /// 只支持 f32 设备输出：优先按调度表采样率从设备报告的范围里挑一段 f32 配置（挑不到
    /// 就退回设备默认配置）；调度表的采样率必须与最终配置的采样率一致。块大小先请求固定值，
    /// 设备拒绝时回退到其默认块大小。
    pub fn start(
        schedule: ScheduledEvents,
        registry: &PluginRegistry,
        options: RealtimeOptions,
    ) -> Result<Self, RealtimeError> {
        Self::start_with(schedule, registry, options, SharedTransport::new())
    }

    /// 与 [`AudioHost::start`] 相同，但复用调用方提供的共享状态。
    ///
    /// 音频线程重建宿主（例如工程被编辑）时传入同一个 `SharedTransport`，界面侧因此
    /// 不必重新订阅就能继续读到位置与播放状态。
    pub fn start_with(
        schedule: ScheduledEvents,
        registry: &PluginRegistry,
        options: RealtimeOptions,
        shared: SharedTransport,
    ) -> Result<Self, RealtimeError> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or(RealtimeError::NoDevice)?;

        let preferred_rate = schedule.sample_rate.round().max(1.0) as u32;
        let chosen = choose_output_format(&supported_formats(&device), preferred_rate);

        // 优先用设备报告的范围挑：只要有一段 F32 就能出声（默认格式不是 F32 的设备也能救回来）。
        // 设备不报告范围时退回默认配置，保持旧行为。
        let mut config = match chosen {
            Some(range) => cpal::StreamConfig {
                channels: range.channels,
                sample_rate: range.nearest_rate(preferred_rate),
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
        // 注意：挑出来的可能是「设备支持但不是首选」的档位（`nearest_rate` 夹到的值），
        // 这时同样拒绝；上层轮询会按新设备重新解析采样率后重试。
        if schedule.sample_rate != config.sample_rate as f64 {
            return Err(RealtimeError::SampleRateMismatch {
                schedule: schedule.sample_rate,
                device: config.sample_rate as f64,
            });
        }

        // 请求固定块大小，使回调内 scratch 的预分配即为上界。`.max(1)` 避免块大小为 0
        // 时回调内的分块循环无法推进（防御性；正常调用方不会传 0）。
        let max_block = options.max_block.max(1);
        // 声道数与采样率取自最终的 `config`：回调里的交错写回必须与设备实际声道数一致
        // （块大小回退只改 `buffer_size`，不动这两项）。
        let channels = config.channels as usize;
        let device_rate = config.sample_rate as f64;

        // 回调工厂：`build_output_stream` 按值拿走回调，构建失败时回调（连同其中的 `Player`
        // 状态）随之释放、无法取回，所以「块大小回退」这一分支需要一份全新的回调。
        // 正常路径只建一份；只有设备拒绝固定块大小时才会付第二次 `Player::new` 的代价。
        let position_cb = Arc::clone(&shared.position);
        let playing_cb = Arc::clone(&shared.playing);
        // 波形缓冲：回调侧只写，界面侧只读（都用原子，不加锁）。
        let scope = Arc::clone(&options.scope);

        let make_callbacks = || {
            let (tx, mut rx) = command_queue(options.command_capacity);
            let position_cb = Arc::clone(&position_cb);
            let playing_cb = Arc::clone(&playing_cb);
            let scope = Arc::clone(&scope);

            // `Player` 的 scratch 全部按 `max_block` 预分配：回调内不分配、不加锁。
            let mut player = Player::new(schedule.clone(), registry, device_rate, max_block);
            let mut staging: Vec<f32> = Vec::with_capacity(max_block * 2);

            let on_data = move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                player.drain_commands(&mut rx);

                let frames = data.len() / channels.max(1);
                let need = frames * 2;
                // 一次性增长到设备块大小属可接受的预热；此后不再分配。
                if staging.len() < need {
                    staging.resize(need, 0.0);
                }

                // 即便后端忽略固定块大小，也按 max_block 分块，确保 `Player` 及其
                // scratch 不会在回调内 resize（分配）。
                let mut offset = 0usize;
                while offset < frames {
                    let n = (frames - offset).min(max_block);
                    player.process_block(&mut staging[offset * 2..(offset + n) * 2], n);
                    offset += n;
                }

                write_interleaved(&staging[..need], data, frames, channels);

                // 给界面画波形：推入刚渲染好的这块（下混成单声道）。
                scope.push_interleaved(&staging[..need], frames);

                // 宿主策略：播放到工程结尾即暂停（而非回卷）。这是 `Player` 不再
                // 自行自动停止后，「播到结尾即停」行为的归属地。
                if player.end_frame() > 0 && player.position_frames() >= player.end_frame() {
                    player.apply_command(TransportCommand::Pause);
                }
                position_cb.store(player.position_frames(), Ordering::Relaxed);
                playing_cb.store(player.is_playing(), Ordering::Relaxed);
            };
            let on_error = move |err| {
                eprintln!("audio stream error: {err}");
            };

            (tx, on_data, on_error)
        };

        // 先请求固定块大小（回调内 scratch 预分配即上界）；有些设备（蓝牙/聚合设备）拒绝
        // 固定块大小，这时回退到设备默认块大小，宁可让回调内的分块循环兜底。
        // `StreamConfig` 是 `Copy`，两次尝试各用自己的那份。
        let build = |cfg: cpal::StreamConfig| {
            let (tx, on_data, on_error) = make_callbacks();
            device
                .build_output_stream(cfg, on_data, on_error, None)
                .map(|stream| (tx, stream))
                .map_err(|e| RealtimeError::Cpal(e.to_string()))
        };
        config.buffer_size = cpal::BufferSize::Fixed(max_block as u32);
        let (tx, stream) = match build(config) {
            Ok(built) => built,
            Err(first) => {
                // 回退时保留第一次的错误（第二次的错误信息更没用：设备连默认块大小都拒绝了）。
                config.buffer_size = cpal::BufferSize::Default;
                build(config).map_err(|_| first)?
            }
        };

        stream
            .play()
            .map_err(|e| RealtimeError::Cpal(e.to_string()))?;

        Ok(AudioHost {
            stream,
            commands: tx,
            shared,
        })
    }

    /// 发送任意传输命令，返回是否成功入队（队列满时不阻塞地返回 `false`）。
    pub fn send(&mut self, command: TransportCommand) -> bool {
        self.commands.send(command)
    }

    /// 发送 `Play` 命令，返回是否成功入队（队列满时不阻塞地返回 `false`）。
    pub fn play(&mut self) -> bool {
        self.send(TransportCommand::Play)
    }

    /// 发送 `Pause` 命令，返回是否成功入队。
    pub fn pause(&mut self) -> bool {
        self.send(TransportCommand::Pause)
    }

    /// 发送 `Stop` 命令（停止并回到起点），返回是否成功入队。
    pub fn stop(&mut self) -> bool {
        self.send(TransportCommand::Stop)
    }

    /// 发送 `Seek` 命令定位到指定帧，返回是否成功入队。
    pub fn seek(&mut self, frame: u64) -> bool {
        self.send(TransportCommand::Seek { frame })
    }

    /// 本宿主对外暴露的共享传输状态。
    pub fn shared(&self) -> SharedTransport {
        self.shared.clone()
    }

    /// 音频线程最近一次报告的播放位置（帧）。
    pub fn position_frames(&self) -> u64 {
        self.shared.position_frames()
    }

    /// 音频线程最近一次报告的播放状态。
    pub fn is_playing(&self) -> bool {
        self.shared.is_playing()
    }
}

impl Drop for AudioHost {
    /// 宿主析构时显式暂停底层输出流，确保设备在控制侧退出后立即静音。
    fn drop(&mut self) {
        let _ = self.stream.pause();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipId, Note, PluginInstance, Project, Track, TrackId};

    fn range(
        format: OutputFormat,
        channels: u16,
        min_rate: u32,
        max_rate: u32,
    ) -> OutputFormatRange {
        OutputFormatRange {
            format,
            channels,
            min_rate,
            max_rate,
        }
    }

    fn schedule() -> ScheduledEvents {
        let mut p = Project::new("rt");
        let mut track = Track {
            id: TrackId(1),
            name: "Lead".into(),
            instrument: Some(PluginInstance {
                plugin_id: "builtin.synth".into(),
                params: Default::default(),
                state: vec![],
            }),
            effects: vec![],
            clips: vec![],
            mixer: Default::default(),
        };
        track.clips.push(Clip {
            id: ClipId(1),
            start: 0.0,
            length: 1.0,
            notes: vec![Note {
                pitch: 69,
                start: 0.0,
                duration: 0.5,
                velocity: 100,
                volume: 127,
            }],
        });
        p.tracks.push(track);
        ScheduledEvents::from_project(&p, 48_000.0)
    }

    #[test]
    fn write_interleaved_downmixes_to_mono() {
        // 两帧、双声道输入：L/R 分别为 (1.0, 0.0) 与 (0.0, 1.0)，单声道取均值 0.5。
        let staging = [1.0f32, 0.0, 0.0, 1.0];
        let mut data = [0.0f32; 2];
        write_interleaved(&staging, &mut data, 2, 1);
        assert_eq!(data, [0.5, 0.5]);
    }

    #[test]
    fn write_interleaved_separates_stereo_channels() {
        let staging = [0.25f32, -0.5, 0.75, 1.0];
        let mut data = [0.0f32; 4];
        write_interleaved(&staging, &mut data, 2, 2);
        assert_eq!(data, [0.25, -0.5, 0.75, 1.0]);
    }

    #[test]
    fn write_interleaved_zeroes_extra_channels() {
        // 6 声道设备：ch0=L、ch1=R，其余清零。
        let staging = [0.25f32, -0.5, 0.75, 1.0];
        let mut data = [1.0f32; 12];
        write_interleaved(&staging, &mut data, 2, 6);
        assert_eq!(
            data,
            [0.25, -0.5, 0.0, 0.0, 0.0, 0.0, 0.75, 1.0, 0.0, 0.0, 0.0, 0.0]
        );
    }

    #[test]
    fn shared_transport_starts_idle_and_survives_cloning() {
        let shared = SharedTransport::new();
        let observer = shared.clone();
        assert_eq!(observer.position_frames(), 0);
        assert!(!observer.is_playing());
    }

    // 依赖真实音频设备的冒烟测试：在无设备环境（CI）应返回错误而非 panic。
    #[test]
    fn start_returns_result_without_panicking() {
        let options = RealtimeOptions {
            max_block: 2048,
            command_capacity: 64,
            scope: Arc::new(ScopeBuffer::new(crate::SCOPE_CAPACITY)),
        };
        match AudioHost::start(schedule(), &PluginRegistry::with_builtins(), options) {
            Ok(mut host) => {
                assert!(host.play());
                assert_eq!(host.position_frames(), 0);
                // 宿主暴露的共享状态必须与自身读数一致。
                assert_eq!(host.shared().position_frames(), host.position_frames());
                assert!(!host.shared().is_playing());
            }
            Err(e) => {
                assert!(
                    matches!(
                        e,
                        RealtimeError::NoDevice
                            | RealtimeError::UnsupportedConfig(_)
                            | RealtimeError::SampleRateMismatch { .. }
                            | RealtimeError::Cpal(_)
                    ),
                    "unexpected error variant: {e:?}"
                );
            }
        }
    }

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
    fn inverted_device_range_is_normalised_before_projection() {
        // 畸形区间（min > max）：投影时交换成合法区间，`nearest_rate` 的 clamp 不会 panic。
        let (min, max) = normalise_rate_span(48_000, 44_100);
        assert_eq!((min, max), (44_100, 48_000));
        assert_eq!(
            range(OutputFormat::F32, 2, min, max).nearest_rate(96_000),
            48_000
        );
    }

    #[test]
    fn no_reported_ranges_means_no_choice() {
        // 设备不报范围 → `choose_output_format` 返回 `None` → 退回默认配置（旧行为）。
        assert!(choose_output_format(&[], 48_000).is_none());
    }

    #[test]
    fn range_clamps_the_rate_into_its_span() {
        assert_eq!(
            range(OutputFormat::F32, 2, 44_100, 48_000).nearest_rate(96_000),
            48_000
        );
        assert_eq!(
            range(OutputFormat::F32, 2, 44_100, 48_000).nearest_rate(8_000),
            44_100
        );
    }

    #[test]
    fn device_changed_flags_any_presence_or_identity_change() {
        let a = DeviceSignature {
            name: "A".into(),
            sample_rate: 48_000,
        };
        let a2 = DeviceSignature {
            name: "A".into(),
            sample_rate: 48_000,
        };
        let b = DeviceSignature {
            name: "B".into(),
            sample_rate: 48_000,
        };
        let a44 = DeviceSignature {
            name: "A".into(),
            sample_rate: 44_100,
        };
        assert!(device_changed(None, Some(&a)));
        assert!(device_changed(Some(&a), None));
        assert!(!device_changed(Some(&a), Some(&a2)));
        assert!(device_changed(Some(&a), Some(&b)));
        assert!(device_changed(Some(&a), Some(&a44)));
    }
}
