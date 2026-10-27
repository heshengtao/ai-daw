use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use thiserror::Error;

use crate::audio::DecodedAudio;
use crate::command_queue::{command_queue, CommandSender};
use crate::player::{Player, ReusableInstances, YieldSink};
use crate::recorder::Recorder;
use crate::registry::PluginRegistry;
use crate::schedule::ScheduledEvents;
use crate::scope::ScopeBuffer;
use crate::transport::TransportCommand;

/// 实时宿主选项。
///
/// 采样率不在选项中：它按 [`ScheduledEvents::sample_rate`] 从设备报告的支持范围里**挑**
/// （`nearest_rate` 夹到最近的一档），设备不报范围时才退回设备默认输出采样率；最终仍会校验
/// 与 [`ScheduledEvents::sample_rate`] 是否一致，不一致时报错。所以设备默认采样率不是唯一
/// 权威来源——它只是「设备不报范围」时的兜底。
#[derive(Debug, Clone)]
pub struct RealtimeOptions {
    /// 音频回调请求的最大帧数：用于预分配 scratch 缓冲，并作为固定块大小请求设备
    /// （设备拒绝固定块大小时回退到它的默认块大小）。
    pub max_block: usize,
    /// 控制侧 → 音频侧命令队列容量。
    pub command_capacity: usize,
    /// 界面画波形用的缓冲；音频回调每块往里推一份。
    pub scope: Arc<ScopeBuffer>,
    /// 流状态：错误回调写入，控制侧读取。
    pub status: Arc<SharedStatus>,
}

/// 实时宿主错误。
#[derive(Debug, Error)]
pub enum RealtimeError {
    /// 系统没有可用的默认输出设备。
    #[error("no default output device available")]
    NoDevice,
    /// 指定的输出设备不存在（被拔出/改名）。
    #[error("output device \"{0}\" is unavailable")]
    NoSuchDevice(String),
    /// 系统没有可用的输入设备（或指定名字的设备不存在）。
    #[error("no input device available")]
    NoInputDevice,
    /// 设备不支持所需的 f32 输出配置（既不报 f32 范围，默认格式也不是 f32）。
    #[error("device does not support an f32 output configuration: {0}")]
    UnsupportedConfig(String),
    /// 调度表构建时使用的采样率与设备采样率不一致（会导致事件静默错位）。
    #[error("schedule sample rate {schedule} Hz does not match device rate {device} Hz")]
    SampleRateMismatch {
        /// 构建调度表时使用的采样率（Hz）。
        schedule: f64,
        /// 最终选定配置的采样率（Hz）：来自所选设备范围的 `nearest_rate`，设备不报范围时
        /// 才是设备默认输出采样率。不一定是设备默认值。
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
    ///
    /// 字段是 `pub`，可能来自畸形外部输入：这里用 `min`/`max` 先把区间归一化，而不是直接
    /// `Ord::clamp`——`clamp` 在 `min > max` 时会 panic，反向区间绝不能让它崩。反向时按
    /// 交换后的合法区间处理（与 [`OutputFormatRange::covers`] 保持一致）。
    pub fn nearest_rate(&self, rate: u32) -> u32 {
        let (min, max) = (
            self.min_rate.min(self.max_rate),
            self.min_rate.max(self.max_rate),
        );
        rate.clamp(min, max)
    }

    /// 该段是否覆盖给定采样率。
    ///
    /// 与 [`OutputFormatRange::nearest_rate`] 用同一套归一化：反向区间按交换后的区间判定，
    /// 二者对畸形输入的解释因此始终一致。
    pub fn covers(&self, rate: u32) -> bool {
        let (min, max) = (
            self.min_rate.min(self.max_rate),
            self.min_rate.max(self.max_rate),
        );
        min <= rate && rate <= max
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

/// 一个输出设备的概况（供界面列选）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OutputDeviceInfo {
    /// 设备显示名。
    pub name: String,
    /// 默认输出采样率（Hz）。
    pub sample_rate: u32,
    /// 默认输出声道数。
    pub channels: u16,
    /// 是否系统默认输出设备。
    pub is_default: bool,
}

/// 设备显示名。cpal 0.18 起 `DeviceTrait::name()` 被 `description()` 取代（返回结构化描述）。
fn output_device_name(device: &cpal::Device) -> Option<String> {
    device.description().ok().map(|d| d.name().to_owned())
}

/// 按名字取输出设备；`None` = 系统默认设备。名字对不上（设备已拔出/改名）时返回 `None`。
pub fn find_output_device(name: Option<&str>) -> Option<cpal::Device> {
    let host = cpal::default_host();
    match name {
        None => host.default_output_device(),
        Some(name) => host
            .output_devices()
            .ok()?
            .find(|device| output_device_name(device).as_deref() == Some(name)),
    }
}

/// 指定输出设备的身份（`None` = 系统默认）；设备不存在（或查询失败）时 `None`。
pub fn output_signature(name: Option<&str>) -> Option<DeviceSignature> {
    let device = find_output_device(name)?;
    let sample_rate = device.default_output_config().ok()?.sample_rate();
    Some(DeviceSignature {
        name: output_device_name(&device)?,
        sample_rate,
    })
}

/// 读取当前默认输出设备的身份；无设备（或查询失败）时 `None`。
pub fn default_output_signature() -> Option<DeviceSignature> {
    output_signature(None)
}

/// 本机所有输出设备（查询失败的设备被跳过）。
pub fn list_output_devices() -> Vec<OutputDeviceInfo> {
    let host = cpal::default_host();
    let default_name = host
        .default_output_device()
        .and_then(|device| output_device_name(&device));
    let Ok(devices) = host.output_devices() else {
        return Vec::new();
    };
    devices
        .filter_map(|device| {
            let name = output_device_name(&device)?;
            let supported = device.default_output_config().ok()?;
            Some(OutputDeviceInfo {
                is_default: default_name.as_deref() == Some(name.as_str()),
                name,
                sample_rate: supported.sample_rate(),
                channels: supported.channels(),
            })
        })
        .collect()
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

/// 到达播放边界时的动作。
///
/// - 没开循环 → 暂停（现状）。
/// - 开了循环且有播放范围 → 无缝回到**范围起点**继续。
/// - 开了循环但没范围 → 无缝回到工程起点继续（整首循环）。
pub(crate) fn end_of_project_action(
    looping: bool,
    loop_range: Option<(u64, u64)>,
) -> TransportCommand {
    if !looping {
        return TransportCommand::Pause;
    }
    match loop_range {
        Some((start, _)) => TransportCommand::Seek { frame: start },
        None => TransportCommand::Seek { frame: 0 },
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
    looping: Arc<AtomicBool>,
    metronome: Arc<AtomicBool>,
    /// 播放范围起点（帧）；`loop_end > loop_start` 才算有效范围。
    loop_start: Arc<AtomicU64>,
    /// 播放范围终点（帧）。
    loop_end: Arc<AtomicU64>,
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

    /// 音频线程最近一次报告的循环开关。
    pub fn is_looping(&self) -> bool {
        self.looping.load(Ordering::Relaxed)
    }

    /// 音频线程最近一次报告的节拍器开关。
    pub fn is_metronome(&self) -> bool {
        self.metronome.load(Ordering::Relaxed)
    }

    /// 音频线程最近一次报告的播放范围起点（帧）。
    pub fn loop_start_frames(&self) -> u64 {
        self.loop_start.load(Ordering::Relaxed)
    }

    /// 音频线程最近一次报告的播放范围终点（帧）。
    pub fn loop_end_frames(&self) -> u64 {
        self.loop_end.load(Ordering::Relaxed)
    }

    /// 当前生效的播放范围（帧）。`None` = 没设范围。
    pub fn loop_range(&self) -> Option<(u64, u64)> {
        let start = self.loop_start_frames();
        let end = self.loop_end_frames();
        if end > start {
            Some((start, end))
        } else {
            None
        }
    }
}

/// 音频流的运行期状态：流回调出错时写入，控制侧读取。
///
/// 运行期唯一写入点是 cpal 的**错误回调**（那是 cpal 自己的线程，不是实时回调），
/// 因此这里的 `Mutex` 不会出现在音频处理路径上；`clear()` 由控制侧在重建前重置。
///
/// 除「已死标志 + 首条原因」外还带一个**代数（epoch）**，用来作废上一代流的错误回调：
/// 所有流代（generation）共享同一个 `SharedStatus`（`options.status` 在同一进程里始终是
/// 同一份实例）。重建时先 `clear()` 再建新流，但刚被丢弃的旧流的错误回调仍持有同一个
/// `Arc`，而 cpal 的 CoreAudio 后端可能在 `drop` 之后才把在途的 `DeviceNotAvailable`
/// 投递出来。若没有代数守卫，这条「迟到的」错误会把健康的新流重新标记为死；轮询看到
/// `dead` 又重建——于是退化成约每秒一次的重建/爆音循环（正是设备抖动场景会踩到的）。
/// 代数让旧代回调的写入变成空操作，只有当前代才能改状态。
#[derive(Debug, Clone)]
pub struct SharedStatus {
    dead: Arc<AtomicBool>,
    message: Arc<Mutex<Option<String>>>,
    /// 单调递增的代数，初值 1；`clear()` 递增它（即 retire 旧 generation）。
    epoch: Arc<AtomicU64>,
    /// 回调心跳：数据回调每处理一块就 +1（只有一次原子写，实时安全）。
    ///
    /// 「流还活着、但已经不再出声」是错误回调**抓不到**的一种失效：cpal 那边没报错，
    /// `host` 也还在，可 CoreAudio 就是不叫我们的 IOProc 了（设备刚被别的客户端重新
    /// 配置、旧 AudioUnit 还没释放完就建了新流……）。这种情况下所有传输/试听命令都
    /// 排进队列没人消费，界面看着「在播」，实际一点声音都没有，**只有重启应用才能恢复**。
    /// 控制侧靠这份心跳判断停摆并重建（见 `stream_stalled`）。
    heartbeat: Arc<AtomicU64>,
}

impl Default for SharedStatus {
    fn default() -> Self {
        Self {
            dead: Arc::new(AtomicBool::new(false)),
            message: Arc::new(Mutex::new(None)),
            epoch: Arc::new(AtomicU64::new(1)),
            heartbeat: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl SharedStatus {
    /// 新建一份健康状态（代数从 1 开始）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前代数（epoch）。调用方在开始建流前捕获它，错误回调据此判断自己是否已过期。
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Relaxed)
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

    /// 仅当调用方捕获的 `epoch` 仍是**当前代**时才标记失效，否则空操作。
    ///
    /// 这是防止「被 drop 的旧流的迟到错误」污染新流的关键：旧回调的代数在 `clear()`
    /// 之后已过期，因此它的写入被丢弃。同一代内仍是「第一条原因胜出」。
    pub fn mark_dead_if_current(&self, epoch: u64, reason: &str) {
        if self.epoch() != epoch {
            return;
        }
        self.mark_dead(reason);
    }

    /// 清空状态（重建宿主前调用）。
    ///
    /// 先**递增代数**以 retire 旧 generation（让在途的旧错误回调失效），再重置标志与原因。
    pub fn clear(&self) {
        self.epoch.fetch_add(1, Ordering::Relaxed);
        self.dead.store(false, Ordering::Relaxed);
        *self.message.lock().unwrap() = None;
    }

    /// 记一次回调心跳（**音频回调内调用**：单次原子加，不加锁、不分配）。
    pub fn beat(&self) {
        self.heartbeat.fetch_add(1, Ordering::Relaxed);
    }

    /// 累计心跳次数。控制侧隔一段时间读一次，前后两次相同 = 回调没再跑过。
    pub fn heartbeats(&self) -> u64 {
        self.heartbeat.load(Ordering::Relaxed)
    }
}

/// 判定「流还活着，但回调已经停摆」。
///
/// `alive` = 宿主存在且流没报错（报错的情况由 `dead` 那条路径处理，不归这里管）。
/// 停摆时心跳计数在两轮轮询之间一动不动——设备拔了/换了也一样会走到这里，但那条路
/// 通常先被错误回调标记为 `dead`，两者互不冲突。
pub fn stream_stalled(alive: bool, beats_before: u64, beats_now: u64) -> bool {
    alive && beats_before == beats_now
}

/// 停摆判定的基础阈值：连续这么多个轮询周期（控制侧每周期约 1 秒）没有心跳才算停摆。
///
/// 取 2 而不是 1，是给「刚建好流、还没跑第一块」留一格余量：建流与第一次回调之间
/// 隔着 CoreAudio 的启动，一次轮询太容易正好落在缝里。
pub const STALL_POLLS: u32 = 2;

/// 第 `backoff` 次「重建完又立刻停摆」时，判定阈值该是多少个轮询周期。
///
/// 阈值按 2 → 4 → 8 → 16 → 32 翻倍（32 个周期约半分钟），之后不再增长。
///
/// **为什么必须退避**：如果一条流建起来就再也不回调（设备被别的进程独占、驱动的
/// IOProc 干脆不跑），固定阈值会让看门狗每 2 秒把流推倒重建一次，永远循环——每次
/// 重建都要丢掉旧 `AudioUnit` 再建新的，正是最容易被卡成「有流、没回调」的时机。
/// 退避之后它仍然一直尝试（真恢复了就立刻出声），但不会把机器拖进抖动。
pub fn stall_polls_required(backoff: u32) -> u32 {
    STALL_POLLS.saturating_mul(1u32 << backoff.min(4))
}

/// 指定输出设备的采样率（Hz）；`None` = 系统默认设备，无法查询时返回 `None`
/// （例如无音频设备的 CI 环境）。
///
/// 调度表以固定采样率定点，必须在构建之前知道设备采样率，因此需要一个不打开流的
/// 探测入口。
pub fn output_sample_rate(name: Option<&str>) -> Option<u32> {
    let device = find_output_device(name)?;
    let supported = device.default_output_config().ok()?;
    Some(supported.sample_rate())
}

/// 默认输出设备的采样率（Hz）；无法查询时返回 `None`。
pub fn default_output_sample_rate() -> Option<u32> {
    output_sample_rate(None)
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
/// 空表会让 [`choose_output_format`] 返回 `None`，`AudioHost::start_on` 据此退回设备
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
    /// 重建前用来向音频回调**索取插件实例**的回收槽（见 [`AudioHost::take_instances`]）。
    yield_sink: YieldSink,
    /// 已打开（armed）的输入设备采集器；`None` = 没有输入流。
    recorder: Option<Recorder>,
    /// 当前输入流对应的设备名（`None` = 默认设备）。用于避免重复开关同一条流。
    armed_device: Option<String>,
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
        Self::start_with(
            schedule,
            registry,
            options,
            SharedTransport::new(),
            ReusableInstances::default(),
        )
    }

    /// 与 [`AudioHost::start`] 相同，但复用调用方提供的共享状态与插件实例。
    ///
    /// 音频线程重建宿主（例如工程被编辑）时传入同一个 `SharedTransport`，界面侧因此
    /// 不必重新订阅就能继续读到位置与播放状态；`carried` 是上一个宿主交出来的插件实例，
    /// 新播放器会复用它们——插件状态与已打开的界面窗口因此得以跨重建保留。
    pub fn start_with(
        schedule: ScheduledEvents,
        registry: &PluginRegistry,
        options: RealtimeOptions,
        shared: SharedTransport,
        carried: ReusableInstances,
    ) -> Result<Self, RealtimeError> {
        Self::start_on(None, schedule, registry, options, shared, carried)
    }

    /// 与 [`AudioHost::start_with`] 相同，但可指定输出设备名（`None` = 系统默认设备）。
    ///
    /// 显式指定的设备不存在（被拔出/改名）时返回 [`RealtimeError::NoSuchDevice`]，
    /// **不会**悄悄改用默认设备——否则界面选的是耳机、声音却从扬声器出来。
    pub fn start_on(
        device_name: Option<&str>,
        schedule: ScheduledEvents,
        registry: &PluginRegistry,
        options: RealtimeOptions,
        shared: SharedTransport,
        carried: ReusableInstances,
    ) -> Result<Self, RealtimeError> {
        // 启动诊断：`AIDAW_STARTUP_TRACE=1` 时把建流各阶段打到 stderr。默认不输出，
        // 避免每次编辑重建播放器都刷一行。
        let trace = std::env::var_os("AIDAW_STARTUP_TRACE").is_some();
        let boot = std::time::Instant::now();
        let device = find_output_device(device_name).ok_or_else(|| match device_name {
            Some(name) => RealtimeError::NoSuchDevice(name.to_string()),
            None => RealtimeError::NoDevice,
        })?;
        let device_ms = boot.elapsed().as_millis();

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

        // 探测一次是为了避免「失败后重建」把插件实例化两遍：先拿一份 no-op 回调的**临时探针流**
        // 去试固定块大小，探针里没有 `Player`、不加载任何插件，所以无论成败都不会有副作用。
        // 有些设备（蓝牙/聚合设备）拒绝固定块大小，这时改用设备默认块大小，宁可让回调内的
        // 分块循环兜底。
        //
        // 探针流从不 `play()`（不会出声），且必须在建真正的流之前**丢弃**：它的 `Drop` 会释放
        // 自己占用的音频单元，否则同一设备上会同时挂着两条流。
        let probe_config = cpal::StreamConfig {
            buffer_size: cpal::BufferSize::Fixed(max_block as u32),
            ..config
        };
        let probe_at = std::time::Instant::now();
        let buffer_size = match device.build_output_stream(
            probe_config,
            |_data: &mut [f32], _info: &cpal::OutputCallbackInfo| {},
            |_err: cpal::Error| {},
            None,
        ) {
            Ok(probe) => {
                // 立刻丢弃探针：真正的流要独占设备。
                drop(probe);
                cpal::BufferSize::Fixed(max_block as u32)
            }
            Err(_) => cpal::BufferSize::Default,
        };
        let probe_ms = probe_at.elapsed().as_millis();
        config.buffer_size = buffer_size;

        // 声道数与采样率取自最终的 `config`：回调里的交错写回必须与设备实际声道数一致
        // （块大小选择只改 `buffer_size`，不动这两项）。
        let channels = config.channels as usize;
        let device_rate = config.sample_rate as f64;

        // 回调只构造一次：固定块大小是否可用已经由上面的探针判定过，这里不再有「失败后重建」，
        // 因此 `Player::new`（以及它加载的每个插件）在整条路径上只跑一次。
        let position_cb = Arc::clone(&shared.position);
        let playing_cb = Arc::clone(&shared.playing);
        let looping_cb = Arc::clone(&shared.looping);
        let metronome_cb = Arc::clone(&shared.metronome);
        let loop_start_cb = Arc::clone(&shared.loop_start);
        let loop_end_cb = Arc::clone(&shared.loop_end);
        // 波形缓冲：回调侧只写，界面侧只读（都用原子，不加锁）。
        let scope = Arc::clone(&options.scope);

        let (tx, mut rx) = command_queue(options.command_capacity);
        // 回收槽：重建前宿主在这里放一个发送端，回调收到 `YieldInstances` 时把实例交出来。
        let yield_sink: YieldSink = Arc::new(Mutex::new(None));
        // `Player` 的 scratch 全部按 `max_block` 预分配：回调内不分配、不加锁。
        // 调度表按值移交：回调只建一份，不再需要为「可能的第二次尝试」克隆一份。
        let player_at = std::time::Instant::now();
        let mut player = Player::with_carried(
            schedule,
            registry,
            device_rate,
            max_block,
            carried,
            Arc::clone(&yield_sink),
        );
        let player_ms = player_at.elapsed().as_millis();
        // 重建宿主时延续循环开关：界面不必重新订阅，用户看到的状态也不会因编辑而复位。
        player.apply_command(TransportCommand::SetLoop(shared.is_looping()));
        // 节拍器同理：它是运行时开关，重建（如编辑后）不该把它悄悄关掉。
        player.apply_command(TransportCommand::SetMetronome(shared.is_metronome()));
        // 播放范围也同理：用户划好的循环段不该因为一次编辑（重建播放器）就丢掉。
        player.apply_command(TransportCommand::SetLoopRange {
            start: shared.loop_start_frames(),
            end: shared.loop_end_frames(),
        });
        let mut staging: Vec<f32> = Vec::with_capacity(max_block * 2);
        // 心跳：回调每跑一块就记一次，供控制侧判断「流还在、但已经不出声」。
        let status_cb = Arc::clone(&options.status);

        let on_data = move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            status_cb.beat();
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

            // 宿主策略：播到播放边界。开循环 → 无缝回（范围起点 / 工程起点）继续；否则暂停。
            let boundary = player.playback_end_frame();
            if boundary > 0 && player.position_frames() >= boundary {
                player.apply_command(end_of_project_action(
                    player.is_looping(),
                    player.loop_range(),
                ));
            }
            position_cb.store(player.position_frames(), Ordering::Relaxed);
            playing_cb.store(player.is_playing(), Ordering::Relaxed);
            looping_cb.store(player.is_looping(), Ordering::Relaxed);
            metronome_cb.store(player.is_metronome(), Ordering::Relaxed);
            if let Some((start, end)) = player.loop_range() {
                loop_start_cb.store(start, Ordering::Relaxed);
                loop_end_cb.store(end, Ordering::Relaxed);
            } else {
                loop_start_cb.store(0, Ordering::Relaxed);
                loop_end_cb.store(0, Ordering::Relaxed);
            }
        };
        // 状态只在 cpal 的**错误回调**里写：实时数据回调（`on_data`）依旧不加锁、不分配。
        // cpal 在独立线程上调用错误回调，因此这里的 `Mutex` 不在音频处理路径上。
        // 注：cpal 0.18 里该回调的参数是 `cpal::Error`（旧版叫 `StreamError`）。
        let status = Arc::clone(&options.status);
        // 在建流**之前**捕获当前代数：错误回调只在代数仍是最新时才写状态。
        // 否则被 drop 的旧流可能在之后投递迟到错误，把健康的新流误标为死（见 `SharedStatus` 注释）。
        let epoch = options.status.epoch();
        let on_error = move |err: cpal::Error| {
            eprintln!("audio stream error: {err}");
            status.mark_dead_if_current(epoch, &err.to_string());
        };

        // 真正的流只建一次，用上面选定的块大小；失败原样向上抛 `RealtimeError::Cpal`。
        let stream_at = std::time::Instant::now();
        let stream = device
            .build_output_stream(config, on_data, on_error, None)
            .map_err(|e| RealtimeError::Cpal(e.to_string()))?;

        stream
            .play()
            .map_err(|e| RealtimeError::Cpal(e.to_string()))?;
        let stream_ms = stream_at.elapsed().as_millis();

        if trace {
            eprintln!(
                "[startup] 引擎建流：找设备 {device_ms} ms / 探测流 {probe_ms} ms / Player+插件 {player_ms} ms / 正式流 {stream_ms} ms（共 {} ms）",
                boot.elapsed().as_millis()
            );
        }

        Ok(AudioHost {
            stream,
            commands: tx,
            shared,
            yield_sink,
            recorder: None,
            armed_device: None,
        })
    }

    /// 打开（或替换）输入设备：供电平表使用，尚未录制。
    ///
    /// **同一设备已在用时直接复用**（不重开流，避免一次多余的开关抖动）；换设备才重建。
    /// 传 `None` 表示用**默认输入设备**；要「关闭输入流」请调 [`AudioHost::disarm_input`]。
    pub fn arm_input(
        &mut self,
        device: Option<String>,
        scope: Arc<ScopeBuffer>,
    ) -> Result<(), RealtimeError> {
        if self.recorder.is_some() && self.armed_device == device {
            return Ok(());
        }
        // 先丢弃旧流（若在录制中，这也顺带结束本次采集）。
        self.recorder = None;
        self.armed_device = None;
        let recorder = Recorder::open(device.clone(), scope)?;
        self.recorder = Some(recorder);
        self.armed_device = device;
        Ok(())
    }

    /// 关闭输入流（会一并丢弃正在采集的内容）。
    pub fn disarm_input(&mut self) {
        self.recorder = None;
        self.armed_device = None;
    }

    /// 开始录制；尚未 arm 输入设备时报 [`RealtimeError::NoInputDevice`]。
    pub fn start_recording(&mut self) -> Result<(), RealtimeError> {
        let recorder = self.recorder.as_mut().ok_or(RealtimeError::NoInputDevice)?;
        recorder.begin();
        Ok(())
    }

    /// 结束录制并取走采集内容（未录制 / 空则 `None`）。输入流保持打开。
    pub fn stop_recording(&mut self) -> Option<DecodedAudio> {
        self.recorder
            .as_mut()
            .and_then(|recorder| recorder.finish())
    }

    /// 让音频回调把当前**活着的**插件实例交出来（重建宿主前调用）。
    ///
    /// 回调可能已经不在跑（流已死 / 被插件卡住），所以这里**有界等待**：超时就返回空表，
    /// 重建照常进行（代价只是插件状态与界面窗口保不住，退回旧行为）。
    pub fn take_instances(&mut self) -> ReusableInstances {
        let (tx, rx) = std::sync::mpsc::channel();
        *self.yield_sink.lock().unwrap() = Some(tx);
        if !self.send(TransportCommand::YieldInstances) {
            // 队列满：把刚装上的发送端收回，免得回调稍后把实例丢进一个没人等的通道。
            self.yield_sink.lock().unwrap().take();
            return ReusableInstances::default();
        }
        match rx.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok(carried) => carried,
            Err(_) => ReusableInstances::default(),
        }
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
            meta: None,
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
            sends: vec![],
            automation: vec![],
        };
        track.clips.push(Clip {
            id: ClipId(1),
            start: 0.0,
            length: 1.0,
            notes: vec![Note {
                id: 0,
                channel: None,
                pressure: None,
                pitch: 69,
                start: 0.0,
                duration: 0.5,
                velocity: 100,
                volume: 127,
                offset: 0.0,
                legato: false,
            }],
            audio: None,
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

    #[test]
    fn stale_generation_error_is_ignored_and_current_one_sticks() {
        // 代数守卫：`clear()` 作废旧一代流的错误回调，迟到的旧错误不能把健康的新流标记为死。
        let status = SharedStatus::new();
        let e1 = status.epoch();
        status.clear();
        let e2 = status.epoch();
        assert_ne!(e1, e2, "clear() 必须递增代数以 retire 旧 generation");

        // 旧代（e1）的迟到错误：必须被忽略。
        status.mark_dead_if_current(e1, "stale");
        assert!(!status.dead());
        assert_eq!(status.message(), None);

        // 当前代（e2）的错误正常生效，且保留第一条。
        status.mark_dead_if_current(e2, "fresh");
        assert!(status.dead());
        assert_eq!(status.message().as_deref(), Some("fresh"));
        status.mark_dead_if_current(e2, "later");
        assert_eq!(status.message().as_deref(), Some("fresh"));
    }

    #[test]
    fn shared_transport_starts_idle_and_survives_cloning() {
        let shared = SharedTransport::new();
        let observer = shared.clone();
        assert_eq!(observer.position_frames(), 0);
        assert!(!observer.is_playing());
    }

    /// 到达边界的动作：开循环回到起点，关循环暂停。
    #[test]
    fn end_of_project_action_wraps_or_pauses() {
        assert_eq!(
            end_of_project_action(true, None),
            TransportCommand::Seek { frame: 0 }
        );
        assert_eq!(end_of_project_action(false, None), TransportCommand::Pause);
    }

    /// 设了播放范围时：开循环回到**范围起点**；关循环仍然只是暂停（范围不生效）。
    #[test]
    fn end_of_project_action_honors_loop_range() {
        assert_eq!(
            end_of_project_action(true, Some((4800, 9600))),
            TransportCommand::Seek { frame: 4800 }
        );
        assert_eq!(
            end_of_project_action(false, Some((4800, 9600))),
            TransportCommand::Pause
        );
    }

    /// 共享状态的循环开关初始为关，且克隆共享同一份。
    #[test]
    fn shared_transport_loop_flag() {
        let shared = SharedTransport::new();
        assert!(!shared.is_looping());
        shared.looping.store(true, Ordering::Relaxed);
        assert!(shared.clone().is_looping());
    }

    // 依赖真实音频设备的冒烟测试：在无设备环境（CI）应返回错误而非 panic。
    #[test]
    fn start_returns_result_without_panicking() {
        let options = RealtimeOptions {
            max_block: 2048,
            command_capacity: 64,
            scope: Arc::new(ScopeBuffer::new(crate::SCOPE_CAPACITY)),
            status: Arc::new(SharedStatus::new()),
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

    /// 心跳必须由**真实的回调**推进——这是看门狗的立身之本。
    ///
    /// 两种错法都只有真机才暴露：`beat()` 没被回调调用 → 看门狗误判「永远停摆」、把流
    /// 无限重建；`beat()` 在别处被调用 → 停摆永远抓不到。所以这里真的开一条流、等它跑
    /// 若干块，再断言计数确实涨了。
    #[test]
    fn a_live_stream_beats_its_heartbeat() {
        let status = Arc::new(SharedStatus::new());
        let options = RealtimeOptions {
            max_block: 2048,
            command_capacity: 64,
            scope: Arc::new(ScopeBuffer::new(crate::SCOPE_CAPACITY)),
            status: Arc::clone(&status),
        };
        let Ok(host) = AudioHost::start(schedule(), &PluginRegistry::with_builtins(), options)
        else {
            // 无可用设备（无头 CI）：这条测试无从验证，跳过而不是失败。
            return;
        };
        let before = status.heartbeats();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let after = status.heartbeats();
        drop(host);
        assert!(
            after > before,
            "真实回调必须推进心跳（before={before} after={after}）"
        );
        assert!(!stream_stalled(true, before, after), "有心跳就不是停摆");
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
    fn reversed_range_does_not_panic_and_matches_covers() {
        // 反向区间直接喂给 `pub` 的 `nearest_rate`（不经投影归一化）：必须不 panic，
        // 且按交换后的区间处理，与 `covers` 保持一致。
        let r = range(OutputFormat::F32, 2, 48_000, 44_100);
        assert_eq!(r.nearest_rate(96_000), 48_000);
        assert_eq!(r.nearest_rate(8_000), 44_100);
        assert!(r.covers(45_000));
        assert!(!r.covers(44_000));
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

    /// 心跳计数只增不减，控制侧靠「前后两次相同」判断停摆。
    #[test]
    fn heartbeat_only_moves_when_the_callback_runs() {
        let status = SharedStatus::new();
        let before = status.heartbeats();
        // 还没跑过任何一块：与上一轮相同 → 停摆（刚建好的流由调用方重置基线兜住）。
        assert!(stream_stalled(true, before, status.heartbeats()));
        status.beat();
        status.beat();
        assert_eq!(status.heartbeats(), before + 2);
        assert!(
            !stream_stalled(true, before, status.heartbeats()),
            "回调跑过就不是停摆"
        );
        // 两轮轮询之间一动不动 → 停摆。
        let idle = status.heartbeats();
        assert!(stream_stalled(true, idle, idle));
    }

    /// 没有宿主 / 流已报错时不走「停摆」这条判定：那两种情况另有专门的处理路径。
    #[test]
    fn stall_check_ignores_dead_or_missing_hosts() {
        assert!(!stream_stalled(false, 7, 7), "宿主不存在时不判停摆");
    }

    /// `clear()`（重建前调用）不重置心跳：计数必须单调，否则「前后相同」会被误判。
    #[test]
    fn clear_keeps_the_heartbeat_monotonic() {
        let status = SharedStatus::new();
        status.beat();
        let beats = status.heartbeats();
        status.clear();
        assert_eq!(status.heartbeats(), beats);
    }

    /// 退避阈值：翻倍增长、封顶，且第一次永远是最灵敏的那一档。
    #[test]
    fn stall_threshold_doubles_then_caps() {
        assert_eq!(stall_polls_required(0), STALL_POLLS, "第一次必须最灵敏");
        assert_eq!(stall_polls_required(1), STALL_POLLS * 2);
        assert_eq!(stall_polls_required(2), STALL_POLLS * 4);
        // 封顶：再多次连续停摆也不会让判定间隔无限变长（否则真恢复也要等太久）。
        let cap = stall_polls_required(4);
        assert_eq!(stall_polls_required(5), cap);
        assert_eq!(stall_polls_required(1_000), cap);
        // 不溢出：极端入参也得给出一个有限的正数。
        assert!(stall_polls_required(u32::MAX) > 0);
    }
}
