//! 专用音频线程与传输控制。
//!
//! `cpal::Stream` 在多数平台上是 `!Send`，因此实时宿主必须被创建并持有在一个专用
//! 线程里；界面与命令层只通过消息通道与共享原子量与它通信。
//!
//! 三条通道各司其职：
//! - **控制 → 音频**：`mpsc::Sender<AudioMessage>`（装载工程、传输命令、退出）。
//! - **音频 → 控制**：`SharedTransport`（播放位置与播放状态，原子量，无锁）。
//! - **错误/可用性**：`available` + `error`，供界面决定「实时播放」还是「离线回退」。

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use ai_daw_engine::audio::DecodedAudio;
use ai_daw_engine::event::Event;
use ai_daw_engine::model::{Project, TrackId};
use ai_daw_engine::player::ReusableInstances;
use ai_daw_engine::realtime::{
    default_output_sample_rate, default_output_signature, device_changed, AudioHost,
    RealtimeOptions, SharedStatus, SharedTransport,
};
use ai_daw_engine::schedule::ScheduledEvents;
use ai_daw_engine::scope::ScopeBuffer;
use ai_daw_engine::transport::TransportCommand;
use ai_daw_engine::SCOPE_CAPACITY;
use ai_daw_host_vst3::{CatalogPlugin, EditorHandle};

use crate::assets::AssetCache;
use crate::plugins::build_registry;

/// 「轨道 → 插件编辑器句柄」表（插件工厂在创建实例时登记）。
pub type EditorHandles = std::sync::Arc<std::sync::Mutex<std::collections::HashMap<u64, EditorHandle>>>;

/// 音频回调请求的最大帧数：同时也是请求设备的固定块大小。
const MAX_BLOCK: usize = 2048;
/// 控制侧 → 音频侧命令队列容量（传输 + 混音命令共用）。
const COMMAND_CAPACITY: usize = 256;

/// 控制侧 → 音频线程 的消息。
enum AudioMessage {
    /// 装载（或重新装载）工程：重建播放器，并尽量保留播放位置与播放状态。
    Load(Box<Project>),
    /// 一条传输/混音命令。
    Command(TransportCommand),
    /// 插件目录有更新（启动时后台扫描完成）：重建注册表并重新装载当前工程。
    SetCatalog(Vec<CatalogPlugin>),
    /// 立刻重新探测设备并重建宿主（界面上的「重试」，以及设备变化时的自动重建）。
    Retry,
    /// 打开（或替换）输入设备；`None` = 用默认输入设备。
    /// 回复是否成功（`None` = 调用方不等待，见 `arm_input` 的 `wait` 参数）。
    ArmInput(Option<String>, Option<Sender<Result<(), String>>>),
    /// 关闭输入流。
    DisarmInput,
    /// 开始录制。回复是否成功。
    StartRecording(Sender<Result<(), String>>),
    /// 结束录制并把采集内容交回（`None` = 没录到东西）。
    StopRecording(Sender<Option<DecodedAudio>>),
}

/// 传输状态快照：界面轮询与事件推送共用。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TransportSnapshot {
    /// 实时播放是否可用（音频设备打开成功）。
    pub available: bool,
    /// 是否正在播放。
    pub playing: bool,
    /// 当前播放位置（帧）。
    pub position_frames: u64,
    /// 设备采样率（Hz）；不可用时为 0。
    pub sample_rate: u32,
    /// 是否开启循环播放。
    #[serde(rename = "loop")]
    pub loop_: bool,
    /// 不可用时的原因。
    pub error: Option<String>,
}

/// 音频引擎的控制侧句柄。
pub struct AudioEngine {
    tx: Sender<AudioMessage>,
    shared: SharedTransport,
    sample_rate: Arc<AtomicU32>,
    available: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    /// 流的运行期状态：错误回调写为「已死」（设备被拔/切换）；控制侧在每次重建前
    /// `clear()`，`snapshot()` 据此把实时播放判为不可用。
    status: Arc<SharedStatus>,
    /// 实时波形缓冲（音频线程写、界面读）。播放器重建时沿用同一个，界面不必重新拿。
    scope: Arc<ScopeBuffer>,
    /// 输入电平表缓冲（音频线程写、界面读），与输出 `scope` 分开。
    input_scope: Arc<ScopeBuffer>,
}

impl AudioEngine {
    /// 启动音频线程并装载初始工程（带插件目录与音频素材缓存）。
    pub fn new(
        project: &Project,
        catalog: Vec<CatalogPlugin>,
        editor_handles: EditorHandles,
        assets: Arc<AssetCache>,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let shared = SharedTransport::new();
        let sample_rate = Arc::new(AtomicU32::new(0));
        let available = Arc::new(AtomicBool::new(false));
        let error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let status = Arc::new(SharedStatus::new());
        let scope = Arc::new(ScopeBuffer::new(SCOPE_CAPACITY));
        let input_scope = Arc::new(ScopeBuffer::new(SCOPE_CAPACITY));

        let engine = AudioEngine {
            tx,
            shared: shared.clone(),
            sample_rate: Arc::clone(&sample_rate),
            available: Arc::clone(&available),
            error: Arc::clone(&error),
            status: Arc::clone(&status),
            scope: Arc::clone(&scope),
            input_scope: Arc::clone(&input_scope),
        };
        thread::Builder::new()
            .name("ai-daw-audio".into())
            .spawn(move || {
                run_audio_thread(
                    rx,
                    shared,
                    sample_rate,
                    available,
                    error,
                    status,
                    catalog,
                    editor_handles,
                    scope,
                    input_scope,
                    assets,
                )
            })
            .expect("failed to spawn audio thread");

        // 首次装载：打开设备并让界面尽快知道实时播放是否可用。
        engine.load(project);
        engine
    }

    /// 安装（或更新）插件目录；音频线程会重建注册表并重新装载当前工程。
    pub fn set_catalog(&self, catalog: Vec<CatalogPlugin>) {
        let _ = self.tx.send(AudioMessage::SetCatalog(catalog));
    }

    /// 装载工程：重建播放器，保留当前位置与播放状态。
    pub fn load(&self, project: &Project) {
        let _ = self.tx.send(AudioMessage::Load(Box::new(project.clone())));
    }

    /// 重新探测输出设备并重建宿主（界面上的「重试」按钮）。
    ///
    /// 由 `retry_audio` Tauri 命令调用：设备被拔掉后再插回，用户无需重启应用。
    pub fn retry_audio(&self) {
        let _ = self.tx.send(AudioMessage::Retry);
    }

    /// 打开（或替换）输入设备；`None` = 用默认输入设备。选定后输入电平表即可工作。
    ///
    /// `wait = false` 时**只排队不等待**：界面启动时自动选默认设备就走这条——那时音频线程
    /// 正在建宿主 / 装载工程（工程里有音频片段时还要解码），等它只会白等出一句
    /// 「等待音频线程超时」；请求排队后照样会被执行。录音前的挂载用 `wait = true`，
    /// 这样设备打不开能立刻报给用户。
    pub fn arm_input(&self, device: Option<String>, wait: bool) -> Result<(), String> {
        if self.unavailable() {
            return Err(self.unavailable_reason());
        }
        if !wait {
            return self
                .tx
                .send(AudioMessage::ArmInput(device, None))
                .map_err(|_| "音频线程已退出".to_string());
        }
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(AudioMessage::ArmInput(device, Some(tx)))
            .map_err(|_| "音频线程已退出".to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(3))
            .map_err(|_| "等待音频线程超时".to_string())?
    }

    /// 关闭输入流。
    pub fn disarm_input(&self) {
        let _ = self.tx.send(AudioMessage::DisarmInput);
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

    /// 结束录制并取回采集内容（`None` = 没录到东西）。
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

    /// 实时播放当前是否不可用：设备未打开，**或**运行期的流已死（设备被拔/切换）。
    ///
    /// 流死亡后不再是「可控」状态：此时入队的传输命令没有流来消费，只会堆积，
    /// 因此与「打开失败」一视同仁，交由轮询路径自动重建。
    fn unavailable(&self) -> bool {
        !self.available.load(Ordering::Relaxed) || self.status.dead()
    }

    /// 发送一条传输/混音命令；实时播放不可用时返回原因（界面据此回退到离线试听）。
    pub fn send(&self, command: TransportCommand) -> Result<(), String> {
        if self.unavailable() {
            return Err(self.unavailable_reason());
        }
        self.tx
            .send(AudioMessage::Command(command))
            .map_err(|_| "音频线程已退出".to_string())
    }

    /// 只影响实时监听的混音预览：不写入工程、不进入撤销历史。
    ///
    /// 拖动推子时以每秒数十次的频率调用；松手时界面才发出一条真正的 `SetGain` 命令。
    pub fn monitor_track_mix(&self, track: TrackId, gain_db: f32, pan: f32, mute: bool, solo: bool) {
        if self.unavailable() {
            return;
        }
        let _ = self.tx.send(AudioMessage::Command(TransportCommand::SetTrackMix {
            track,
            gain_db,
            pan,
            mute,
            solo,
        }));
    }

    /// 取最近 `points` 个采样，供界面画实时波形。
    pub fn scope_snapshot(&self, points: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; points.clamp(16, SCOPE_CAPACITY)];
        self.scope.read_last(&mut out);
        out
    }

    /// 试听一个音（合成器界面里的键盘）：不写工程、不进撤销历史、不等传输。
    pub fn audition_note(&self, track: TrackId, note: u8, velocity: u8, on: bool) {
        if self.unavailable() {
            return;
        }
        let _ = self.tx.send(AudioMessage::Command(TransportCommand::Audition {
            track,
            note,
            velocity,
            on,
        }));
    }

    /// 当前传输状态快照。
    pub fn snapshot(&self) -> TransportSnapshot {
        // 除「打开失败」外，运行期流错误（设备被拔/切换）也要算不可用：否则界面会一直显示
        // 「Realtime」而实际已经没声音。流死亡时 `available=false` 且带上死亡原因。
        let dead = self.status.dead();
        let available = self.available.load(Ordering::Relaxed) && !dead;
        TransportSnapshot {
            available,
            playing: available && self.shared.is_playing(),
            position_frames: if available { self.shared.position_frames() } else { 0 },
            sample_rate: self.sample_rate.load(Ordering::Relaxed),
            loop_: available && self.shared.is_looping(),
            error: if dead { self.status.message() } else { self.error.lock().unwrap().clone() },
        }
    }

    /// 按事件把工程变更同步到音频线程。
    ///
    /// 混音类事件（增益/声像/静音/独奏/总输出）只推参数，不重建播放器——这是拖动推子
    /// 不会打断音频的原因；其余事件会改变定帧事件表，必须重建。
    pub fn sync_events(&self, project: &Project, events: &[Event]) {
        if events.is_empty() {
            return;
        }
        let mut reload = false;
        for event in events {
            match event {
                Event::GainChanged { track, .. }
                | Event::PanChanged { track, .. }
                | Event::MuteChanged { track, .. }
                | Event::SoloChanged { track, .. } => {
                    if let Some(t) = project.track(*track) {
                        self.queue_if_available(TransportCommand::SetTrackMix {
                            track: *track,
                            gain_db: t.mixer.gain_db,
                            pan: t.mixer.pan,
                            mute: t.mixer.mute,
                            solo: t.mixer.solo,
                        });
                    }
                }
                Event::MasterGainChanged { .. } => {
                    self.queue_if_available(TransportCommand::SetMasterGain {
                        gain_db: project.master.gain_db,
                    });
                }
                // 参数变化走实时通道：插件界面上的旋钮会高频产生它们，重建播放器会不断
                // 重置插件状态并造成音频中断。
                Event::ParamChanged { track, param, value, .. } => {
                    self.queue_if_available(TransportCommand::SetParam {
                        track: *track,
                        effect: None,
                        param: param.clone(),
                        value: *value,
                    });
                }
                Event::EffectParamChanged { track, index, param, value, .. } => {
                    self.queue_if_available(TransportCommand::SetParam {
                        track: *track,
                        effect: Some(*index),
                        param: param.clone(),
                        value: *value,
                    });
                }
                // 自动化是「仅混音」变更：把新曲线直接推给音频线程，逐块求值——不必重建。
                Event::AutomationChanged { track, param, points, .. } => {
                    self.queue_if_available(TransportCommand::SetAutomation {
                        track: *track,
                        param: param.clone(),
                        points: points.clone(),
                    });
                }
                other => {
                    // 兜底：任何新增事件默认都走重建，宁可多重建也不要漏掉结构变更。
                    debug_assert!(!other.is_mix_only(), "mix-only event must be handled above");
                    reload = true;
                }
            }
        }
        if reload {
            // 结构性变更**不**受可用性门控：`load` 只是让音频线程记住最新工程（`current`），
            // 建流失败也无妨；设备恢复后重建时才会用到它。若在这里跳过，恢复后会播到旧内容。
            self.load(project);
        }
    }

    /// 与 `send`/`monitor_track_mix`/`audition_note` 一致地把命令入队，但**离线时直接丢弃**。
    ///
    /// `sync_events` 的混音/参数推送此前绕过可用性门控、直接写 `self.tx`；离线时没有流来
    /// 消费这些命令，只会占满队列。这里统一走 `unavailable()` 判定，与其余入口保持对称。
    fn queue_if_available(&self, command: TransportCommand) {
        if self.unavailable() {
            return;
        }
        let _ = self.tx.send(AudioMessage::Command(command));
    }

    fn unavailable_reason(&self) -> String {
        // 流死亡时原因在 `status` 里（与 `snapshot()` 一致），否则用打开失败的原因。
        if self.status.dead() {
            if let Some(reason) = self.status.message() {
                return reason;
            }
        }
        self.error
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| "音频输出不可用".to_string())
    }
}

/// 让当前宿主把活着的插件实例交出来（宿主为空时返回空表）。
///
/// 必须在**丢弃旧宿主之前**调用：实例（连同插件内部状态与已打开的界面窗口）只存在于
/// 播放器里，重建后靠它们复用。
fn take_instances(host: &mut Option<AudioHost>, available: &AtomicBool) -> ReusableInstances {
    // 离线（没打开流 / 流已死）时回调根本不在跑，别白等：直接返回空表，重建走旧路径。
    if !available.load(Ordering::Relaxed) {
        return ReusableInstances::default();
    }
    match host.as_mut() {
        Some(host) => host.take_instances(),
        None => ReusableInstances::default(),
    }
}

/// 用当前插件目录构建并启动一次宿主。
///
/// `carried` 是上一个宿主交出来的插件实例：新播放器会复用 plugin id 仍匹配的那些，
/// 插件状态与已打开的界面窗口因此跨重建保留。失败时记录原因并置 `available = false`，
/// 返回 `None`。
#[allow(clippy::too_many_arguments)]
fn start_host(
    project: &Project,
    catalog: &[CatalogPlugin],
    editor_handles: &EditorHandles,
    assets: &Arc<AssetCache>,
    shared: &SharedTransport,
    sample_rate: &AtomicU32,
    available: &AtomicBool,
    error: &Mutex<Option<String>>,
    status: &Arc<SharedStatus>,
    scope: &Arc<ScopeBuffer>,
    carried: ReusableInstances,
) -> Option<AudioHost> {
    let rate = match resolve_sample_rate(sample_rate) {
        Ok(rate) => rate,
        Err(reason) => {
            available.store(false, Ordering::Relaxed);
            *error.lock().unwrap() = Some(reason);
            return None;
        }
    };

    let registry = build_registry(catalog, project, Arc::clone(editor_handles));
    // 素材必须在这里（**音频回调之外**）就加载好：回调里绝不解码、不分配。
    // 缺失的素材会被跳过 → 该片段静音（与「缺失插件用静音占位」同一策略）。
    let samples = assets.snapshot_for(&project.audio_paths());
    let schedule = ScheduledEvents::from_project_with_assets(project, rate as f64, &samples);
    let options = RealtimeOptions {
        max_block: MAX_BLOCK,
        command_capacity: COMMAND_CAPACITY,
        scope: Arc::clone(scope),
        status: Arc::clone(status),
    };
    match AudioHost::start_with(schedule, &registry, options, shared.clone(), carried) {
        Ok(host) => {
            available.store(true, Ordering::Relaxed);
            *error.lock().unwrap() = None;
            Some(host)
        }
        Err(e) => {
            // 清空缓存的采样率：设备可能已切换，下次装载重新探测。
            sample_rate.store(0, Ordering::Relaxed);
            available.store(false, Ordering::Relaxed);
            *error.lock().unwrap() = Some(e.to_string());
            None
        }
    }
}

/// 用当前工程与插件目录重建宿主；`reset_rate` 为真时先清采样率缓存（设备可能换了）。
///
/// **重建前必须 `status.clear()`**：否则上一根流留下的「已死」标记会一直粘滞，
/// 连健康的新流也会被 `snapshot()` 判为离线（Task 3 review 的强制协调项）。
/// `current` 为空（尚未装载工程）时不建流，直接返回 `None`。
#[allow(clippy::too_many_arguments)]
fn rebuild(
    current: &Option<Project>,
    catalog: &[CatalogPlugin],
    editor_handles: &EditorHandles,
    assets: &Arc<AssetCache>,
    shared: &SharedTransport,
    status: &Arc<SharedStatus>,
    sample_rate: &AtomicU32,
    available: &AtomicBool,
    error: &Mutex<Option<String>>,
    scope: &Arc<ScopeBuffer>,
    reset_rate: bool,
    carried: ReusableInstances,
) -> Option<AudioHost> {
    let project = current.clone()?;
    if reset_rate {
        sample_rate.store(0, Ordering::Relaxed);
    }
    // 关键：清掉上一根流的失效标记，让重建后的健康流重新可被判定为可用。
    status.clear();
    start_host(
        &project,
        catalog,
        editor_handles,
        assets,
        shared,
        sample_rate,
        available,
        error,
        status,
        scope,
        carried,
    )
}

/// 重建后恢复位置与播放状态。
///
/// 这两条命令会在第一个回调块开始前入队并被应用，因此界面不会观察到位置瞬间归零——
/// 但这**依赖两个前提**：其一，`Player` 的数据回调在每个块的最开头先 `drain_commands`
/// 收取命令（见 `crates/engine/src/realtime.rs` 的 `on_data`）；其二，重建宿主时
/// `shared.position` 从不被重置，`snapshot()` 读到的仍是旧流最后写入的帧。若将来回调改成
/// 「先出音频再收命令」，或重建时清零共享位置，这里的位置连续性就不再成立（界面会看到位置
/// 短暂归零）。
fn restore_transport(host: &mut Option<AudioHost>, frame: u64, was_playing: bool) {
    if let Some(h) = host.as_mut() {
        h.seek(frame);
        if was_playing {
            h.play();
        }
    }
}

/// 音频线程主体：独占宿主，按消息重建播放器或转发传输命令，并每秒轮询一次设备。
fn run_audio_thread(
    rx: Receiver<AudioMessage>,
    shared: SharedTransport,
    sample_rate: Arc<AtomicU32>,
    available: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    status: Arc<SharedStatus>,
    mut catalog: Vec<CatalogPlugin>,
    editor_handles: EditorHandles,
    scope: Arc<ScopeBuffer>,
    input_scope: Arc<ScopeBuffer>,
    assets: Arc<AssetCache>,
) {
    /// 设备轮询间隔：够快能跟上插拔耳机，又不至于每秒多次查询 CoreAudio 太过分。
    const POLL: std::time::Duration = std::time::Duration::from_millis(1000);

    let mut host: Option<AudioHost> = None;
    let mut current: Option<Project> = None;
    // 已知的默认输出设备身份：用来判断设备是否换了。每次重建后同步刷新。
    let mut signature = default_output_signature();

    // 所有发送端都析构后 `recv_timeout` 返回 `Disconnected`，线程随之退出（并释放宿主）。
    loop {
        match rx.recv_timeout(POLL) {
            Ok(AudioMessage::Command(command)) => {
                if let Some(h) = host.as_mut() {
                    h.send(command);
                }
            }
            // 目录更新（启动时扫描完成）：重建注册表，重新装载当前工程。
            Ok(AudioMessage::SetCatalog(new_catalog)) => {
                catalog = new_catalog;
                // 重建前后尽量保持听感连续：记住位置与播放状态。
                let was_playing = shared.is_playing();
                let frame = shared.position_frames();
                // 先把活着的插件实例取出来（重建后复用——插件状态与已打开的界面窗口因此
                // 跨重建保留），再释放旧宿主。
                let carried = take_instances(&mut host, &available);
                drop(host.take());
                host = rebuild(
                    &current,
                    &catalog,
                    &editor_handles,
                    &assets,
                    &shared,
                    &status,
                    &sample_rate,
                    &available,
                    &error,
                    &scope,
                    false,
                    carried,
                );
                restore_transport(&mut host, frame, was_playing);
                signature = default_output_signature();
            }
            Ok(AudioMessage::Load(project)) => {
                current = Some((*project).clone());
                let was_playing = shared.is_playing();
                let frame = shared.position_frames();
                // 先把活着的插件实例取出来，让重建后的播放器复用——插件状态与已打开的
                // 界面窗口因此跨重建保留（改效果链 / 移动片段不再重置乐器、不再关窗）。
                let carried = take_instances(&mut host, &available);
                drop(host.take());
                host = rebuild(
                    &current,
                    &catalog,
                    &editor_handles,
                    &assets,
                    &shared,
                    &status,
                    &sample_rate,
                    &available,
                    &error,
                    &scope,
                    // 工程变化不改采样率（采样率取决于设备），沿用缓存即可。
                    false,
                    carried,
                );
                restore_transport(&mut host, frame, was_playing);
                // 重建后记录当前设备身份，避免下一次轮询把同一设备误判为「变化」。
                signature = default_output_signature();
            }
            Ok(AudioMessage::Retry) => {
                let was_playing = shared.is_playing();
                let frame = shared.position_frames();
                // 先把活着的插件实例取出来，让重建后的播放器复用。
                let carried = take_instances(&mut host, &available);
                drop(host.take());
                // 设备可能换了：清采样率缓存后重新探测。
                host = rebuild(
                    &current,
                    &catalog,
                    &editor_handles,
                    &assets,
                    &shared,
                    &status,
                    &sample_rate,
                    &available,
                    &error,
                    &scope,
                    true,
                    carried,
                );
                restore_transport(&mut host, frame, was_playing);
                signature = default_output_signature();
            }
            Ok(AudioMessage::ArmInput(device, reply)) => {
                let result = match host.as_mut() {
                    Some(h) => h
                        .arm_input(device.clone(), Arc::clone(&input_scope))
                        .map_err(|e| e.to_string()),
                    None => Err("音频输出不可用，无法打开输入设备".to_string()),
                };
                match reply {
                    Some(reply) => {
                        let _ = reply.send(result);
                    }
                    // 没人等在对面（界面启动时的自动选设备）：失败了只记一条日志。
                    None => {
                        if let Err(e) = result {
                            eprintln!("[audio] 打开输入设备失败：{e}");
                        }
                    }
                }
            }
            Ok(AudioMessage::DisarmInput) => {
                if let Some(h) = host.as_mut() {
                    h.disarm_input();
                }
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
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // 设备换了，或上次建流失败/流已死：重建一次（保留位置与播放状态）。
                // `host.is_none()` 让建流失败后每秒自动重试；`status.dead()` 让「设备名
                // 没变但流死了」也能救。
                let now = default_output_signature();
                let needs = host.is_none()
                    || status.dead()
                    || device_changed(signature.as_ref(), now.as_ref());
                if !needs {
                    continue;
                }
                let was_playing = shared.is_playing();
                let frame = shared.position_frames();
                // 先把活着的插件实例取出来，让重建后的播放器复用——插件状态与已打开的
                // 界面窗口因此跨重建保留（改效果链 / 移动片段不再重置乐器、不再关窗）。
                let carried = take_instances(&mut host, &available);
                drop(host.take());
                host = rebuild(
                    &current,
                    &catalog,
                    &editor_handles,
                    &assets,
                    &shared,
                    &status,
                    &sample_rate,
                    &available,
                    &error,
                    &scope,
                    true,
                    carried,
                );
                restore_transport(&mut host, frame, was_playing);
                // 记录本次探测到（并已据此重建）的设备身份，稳住下一轮判定。
                signature = now;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    // 退出前显式释放宿主，让设备立刻静音。
    drop(host);
}

/// 解析（并缓存）设备采样率；未知时探测一次。
fn resolve_sample_rate(cache: &AtomicU32) -> Result<u32, String> {
    let cached = cache.load(Ordering::Relaxed);
    if cached != 0 {
        return Ok(cached);
    }
    match default_output_sample_rate() {
        Some(rate) => {
            cache.store(rate, Ordering::Relaxed);
            Ok(rate)
        }
        None => Err("未找到可用的音频输出设备".to_string()),
    }
}
