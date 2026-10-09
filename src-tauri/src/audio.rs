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

use daw_party_engine::event::Event;
use daw_party_engine::model::{Project, TrackId};
use daw_party_engine::realtime::{
    default_output_sample_rate, AudioHost, RealtimeOptions, SharedStatus, SharedTransport,
};
use daw_party_engine::schedule::ScheduledEvents;
use daw_party_engine::scope::ScopeBuffer;
use daw_party_engine::transport::TransportCommand;
use daw_party_engine::SCOPE_CAPACITY;
use daw_party_host_vst3::{CatalogPlugin, EditorHandle};

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
    /// 流的运行期状态：错误回调写入（设备被拔/切换），`snapshot()` 据此把实时播放判为不可用。
    status: Arc<SharedStatus>,
    /// 实时波形缓冲（音频线程写、界面读）。播放器重建时沿用同一个，界面不必重新拿。
    scope: Arc<ScopeBuffer>,
}

impl AudioEngine {
    /// 启动音频线程并装载初始工程（带插件目录）。
    pub fn new(project: &Project, catalog: Vec<CatalogPlugin>, editor_handles: EditorHandles) -> Self {
        let (tx, rx) = mpsc::channel();
        let shared = SharedTransport::new();
        let sample_rate = Arc::new(AtomicU32::new(0));
        let available = Arc::new(AtomicBool::new(false));
        let error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let status = Arc::new(SharedStatus::new());
        let scope = Arc::new(ScopeBuffer::new(SCOPE_CAPACITY));

        let engine = AudioEngine {
            tx,
            shared: shared.clone(),
            sample_rate: Arc::clone(&sample_rate),
            available: Arc::clone(&available),
            error: Arc::clone(&error),
            status: Arc::clone(&status),
            scope: Arc::clone(&scope),
        };
        thread::Builder::new()
            .name("daw-party-audio".into())
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

    /// 发送一条传输/混音命令；实时播放不可用时返回原因（界面据此回退到离线试听）。
    pub fn send(&self, command: TransportCommand) -> Result<(), String> {
        if !self.available.load(Ordering::Relaxed) {
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
        if !self.available.load(Ordering::Relaxed) {
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
        if !self.available.load(Ordering::Relaxed) {
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
                        let _ = self.tx.send(AudioMessage::Command(TransportCommand::SetTrackMix {
                            track: *track,
                            gain_db: t.mixer.gain_db,
                            pan: t.mixer.pan,
                            mute: t.mixer.mute,
                            solo: t.mixer.solo,
                        }));
                    }
                }
                Event::MasterGainChanged { .. } => {
                    let _ = self.tx.send(AudioMessage::Command(TransportCommand::SetMasterGain {
                        gain_db: project.master.gain_db,
                    }));
                }
                // 参数变化走实时通道：插件界面上的旋钮会高频产生它们，重建播放器会不断
                // 重置插件状态并造成音频中断。
                Event::ParamChanged { track, param, value, .. } => {
                    let _ = self.tx.send(AudioMessage::Command(TransportCommand::SetParam {
                        track: *track,
                        effect: None,
                        param: param.clone(),
                        value: *value,
                    }));
                }
                Event::EffectParamChanged { track, index, param, value, .. } => {
                    let _ = self.tx.send(AudioMessage::Command(TransportCommand::SetParam {
                        track: *track,
                        effect: Some(*index),
                        param: param.clone(),
                        value: *value,
                    }));
                }
                other => {
                    // 兜底：任何新增事件默认都走重建，宁可多重建也不要漏掉结构变更。
                    debug_assert!(!other.is_mix_only(), "mix-only event must be handled above");
                    reload = true;
                }
            }
        }
        if reload {
            self.load(project);
        }
    }

    fn unavailable_reason(&self) -> String {
        self.error
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| "音频输出不可用".to_string())
    }
}

/// 用当前插件目录构建并启动一次宿主。
///
/// 失败时记录原因并置 `available = false`，返回 `None`。
fn start_host(
    project: &Project,
    catalog: &[CatalogPlugin],
    editor_handles: &EditorHandles,
    shared: &SharedTransport,
    sample_rate: &AtomicU32,
    available: &AtomicBool,
    error: &Mutex<Option<String>>,
    status: &Arc<SharedStatus>,
    scope: &Arc<ScopeBuffer>,
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
    let schedule = ScheduledEvents::from_project(project, rate as f64);
    let options = RealtimeOptions {
        max_block: MAX_BLOCK,
        command_capacity: COMMAND_CAPACITY,
        scope: Arc::clone(scope),
        status: Arc::clone(status),
    };
    match AudioHost::start_with(schedule, &registry, options, shared.clone()) {
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

/// 音频线程主体：独占宿主，按消息重建播放器或转发传输命令。
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
) {
    let mut host: Option<AudioHost> = None;
    let mut current: Option<Project> = None;

    // 所有发送端都析构后 `recv` 返回错误，线程随之退出（并释放宿主）。
    while let Ok(message) = rx.recv() {
        match message {
            AudioMessage::Command(command) => {
                if let Some(h) = host.as_mut() {
                    h.send(command);
                }
            }
            // 目录更新（启动时扫描完成）：重建注册表，重新装载当前工程。
            AudioMessage::SetCatalog(new_catalog) => {
                catalog = new_catalog;
                let Some(project) = current.clone() else { continue };
                let was_playing = shared.is_playing();
                let frame = shared.position_frames();
                // 先释放旧宿主（`Drop` 会暂停底层输出流），避免同时占用设备。
                drop(host.take());
                host = start_host(
                    &project,
                    &catalog,
                    &editor_handles,
                    &shared,
                    &sample_rate,
                    &available,
                    &error,
                    &status,
                    &scope,
                );
                if let Some(h) = host.as_mut() {
                    h.seek(frame);
                    if was_playing {
                        h.play();
                    }
                }
            }
            AudioMessage::Load(project) => {
                current = Some((*project).clone());
                // 编辑前后尽量保持听感连续：记住位置与播放状态。
                let was_playing = shared.is_playing();
                let frame = shared.position_frames();

                // 先释放旧宿主（`Drop` 会暂停底层输出流），避免同时占用设备。
                drop(host.take());

                host = start_host(
                    &project,
                    &catalog,
                    &editor_handles,
                    &shared,
                    &sample_rate,
                    &available,
                    &error,
                    &status,
                    &scope,
                );
                if let Some(h) = host.as_mut() {
                    // 这两条命令会在第一个回调块开始前入队并被应用，因此界面不会
                    // 观察到位置瞬间归零。
                    h.seek(frame);
                    if was_playing {
                        h.play();
                    }
                }
            }
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
