use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ai_daw_engine::bus::Engine;
use ai_daw_engine::model::TrackId;
use ai_daw_engine::registry::PluginSlot;
use ai_daw_host_vst3::{CatalogPlugin, PluginEditorWindow, SkippedPlugin};

use crate::ai::AiRuntime;
use crate::assets::AssetCache;
use crate::audio::{AudioEngine, EditorSlot};
use crate::presets::PresetStore;
use crate::sound_memories::SoundMemoryStore;

thread_local! {
    /// 已打开的插件原生窗口。
    ///
    /// **只在「开这扇窗的那条线程」上访问**：`PluginWindow` 持有原生句柄，创建 / 销毁 /
    /// 泵事件都必须在同一条线程做（插件把创建实例的那条线程当成消息线程），所以放在
    /// thread-local 而不是共享状态里。
    ///
    /// 于是这张表实际有多份：VST3 的按**模块**各一份（见 `host_vst3::plugin_thread`），
    /// CLAP / AU 的在主线程上。哪一扇归谁，由 [`EditorSlot::module`] 记着。
    static EDITOR_WINDOWS: RefCell<HashMap<(u64, PluginSlot), PluginEditorWindow>> = RefCell::new(HashMap::new());
}

/// 在**当前线程**上操作「(轨道, 槽位) → 插件窗口」表。
///
/// 调用前先确认当前线程就是开这扇窗的那条（VST3 → 插件线程；CLAP / AU → 主线程）。
pub fn with_editor_windows<R>(
    f: impl FnOnce(&mut HashMap<(u64, PluginSlot), PluginEditorWindow>) -> R,
) -> R {
    EDITOR_WINDOWS.with(|windows| f(&mut windows.borrow_mut()))
}

/// 一次录音的目标位置。
#[derive(Debug, Clone, Copy)]
pub struct RecordingTarget {
    /// 录音落在哪条轨道。
    pub track: TrackId,
    /// 录音起点（拍）。
    pub start: f64,
}

/// 应用共享状态：进程内持有唯一的引擎实例与音频引擎句柄。
pub struct AppState {
    /// 领域引擎（工程模型 + 撤销历史）。所有编辑都必须经过它。
    pub engine: Mutex<Engine>,
    /// 实时音频宿主（运行在专用线程上）。
    pub audio: Arc<AudioEngine>,
    /// 已扫描到的插件目录（启动时后台扫描填充；离线渲染也用它）。
    pub catalog: Mutex<Vec<CatalogPlugin>>,
    /// 最近一次扫描里被跳过的插件（崩溃 / 超时 / 探测失败），界面据此提示用户。
    pub scan_skipped: Mutex<Vec<SkippedPlugin>>,
    /// 最近一次扫描**整体**没跑起来的原因（例如扫描探针缺失）；正常时为 `None`。
    pub scan_error: Mutex<Option<String>>,
    /// 「(轨道, 槽位) → 插件编辑器句柄」：插件工厂在实例创建时登记，界面据此打开原生窗口。
    pub editor_handles: Arc<Mutex<HashMap<(u64, PluginSlot), EditorSlot>>>,
    /// 哪些 (轨道, 槽位) 开着插件窗口（主线程那张表的**可跨线程镜像**，供界面查询）。
    pub open_editors: Arc<Mutex<HashSet<(u64, PluginSlot)>>>,
    /// 已开窗口是**为哪个插件**开的（(轨道, 槽位) → plugin id）。
    ///
    /// 重建播放器时插件实例会被「搬」到新播放器复用，所以只要这个槽位的插件没换，窗口就
    /// 依旧有效、不该关；只有插件被换掉（或轨道/槽位消失）时窗口才真的失效。判定靠这份记录。
    pub editor_plugins: Arc<Mutex<HashMap<(u64, PluginSlot), String>>>,
    /// 其中哪些 (轨道, 槽位) 的窗口是**宿主建的**（插件只支持嵌入模式）。用户关掉那扇窗就等于关界面。
    pub host_windows: Arc<Mutex<HashSet<(u64, PluginSlot)>>>,
    /// 插件自己开出来的那扇原生窗口句柄（供「已经开着就把它抬到最前」用）。
    ///
    /// 窗口对象本身是 thread-local（只能在开它的线程上碰），句柄可以跨线程用——
    /// `SetForegroundWindow` 本来就允许别的线程调用。
    pub editor_windows: Arc<Mutex<HashMap<(u64, PluginSlot), ai_daw_host_vst3::NativeWindow>>>,
    /// 用户预设（跨工程；文件位置由 `init_presets` 装上）。
    pub presets: Mutex<PresetStore>,
    /// 音色记忆库（跨工程；文件位置由 `init_sound_memories` 装上）。
    pub sound_memories: Mutex<SoundMemoryStore>,
    /// 音频素材缓存（路径 → 已解码 PCM）。音频线程与离线渲染共用。
    pub assets: Arc<AssetCache>,
    /// 当前工程最后保存/打开的**文件路径**（尚未落盘时为 `None`）。
    ///
    /// 这是界面「保存」要知道的目标；它随会话缓存一起持久化，下次启动能接着用。
    pub session_path: Mutex<Option<String>>,
    /// 当前工程的身份键（未保存工程也稳定）：历史卡片按它归并同一工程的多个版本。
    /// 新建 / 打开工程时换新键；从历史恢复时沿用那份快照的键。
    pub project_key: Mutex<String>,
    /// 正在进行的一次录音的目标：`record_start` 记下，`record_stop` 取走。
    pub recording: Mutex<Option<RecordingTarget>>,
    /// 「工程已经装进音频宿主」的一次性开关：见 [`AppState::start_audio`]。
    audio_started: AtomicBool,
    /// 本地 AI 任务（后台线程 + 进度 + 取消）。
    pub ai: AiRuntime,
    /// 歌声合成任务（独立的进度 / 取消；模型是另一组权重，不该和 AI 面板抢一个槽位）。
    pub singing: crate::singing::SingingRuntime,
}

impl AppState {
    /// 以给定工程名新建共享状态（初始工程带默认分轨模板，且不带历史）。
    ///
    /// 只用于测试：真正的启动路径走 [`AppState::bootstrap`]，它会先读会话缓存，
    /// 免得先装一遍默认模板、再装一遍会话工程。
    #[cfg(test)]
    pub fn new(name: &str) -> Self {
        Self::from_engine(Engine::new_project(name), Vec::new(), true)
    }

    /// 启动时的共享状态：把「最终要用的工程」和「上次扫描的插件目录」**一次**交给音频线程。
    ///
    /// 有会话缓存时直接用还原出来的工程，不再先装载默认分轨模板：模板里的钢琴 / 吉他 /
    /// 贝斯 / 鼓组会各自解码一整套内嵌采样音色（SF3 的 Ogg Vorbis，冷启动要几秒），
    /// 而它们马上就会被会话里的工程替换掉——那几秒是纯浪费，用户看到的就是「启动卡住」。
    ///
    /// 插件目录也一并带上：否则默认工程先建一次宿主、装目录后再建一次，等于白跑一遍。
    ///
    /// 装载本身推迟到 [`AppState::start_audio`]（首屏之后）：外部乐器实例化要占主线程，
    /// 放在建窗口的同一条线程上会把首屏堵住。
    pub fn bootstrap(
        session: Option<crate::session::SessionFile>,
        catalog: Vec<CatalogPlugin>,
    ) -> Self {
        let engine = match &session {
            Some(session) => Engine::restore(session.project.clone(), session.saved.clone()),
            None => Engine::new_project("untitled"),
        };
        let state = Self::from_engine(engine, catalog, false);
        if let Some(session) = session {
            *state.session_path.lock().unwrap() = session.path;
            if !session.project_key.is_empty() {
                *state.project_key.lock().unwrap() = session.project_key;
            }
        }
        state
    }

    /// 把当前工程装进音频宿主——**只在启动时调一次**。
    ///
    /// 重复调用无副作用（第二次直接返回）。首屏画出来之前就装载的话，AU / CLAP 的实例化
    /// 会和「建窗口 + 加载页面」抢同一条主线程，用户看到的是「窗口出来了几秒还是白的」。
    pub fn start_audio(&self) -> bool {
        if self.audio_started.swap(true, Ordering::SeqCst) {
            return false;
        }
        let project = self.engine.lock().unwrap().project().clone();
        self.audio.load(&project);
        true
    }

    /// 用给定工程（与插件目录）装配共享状态；`start_audio` 决定是否立刻装载。
    fn from_engine(engine: Engine, catalog: Vec<CatalogPlugin>, start_audio: bool) -> Self {
        let editor_handles: Arc<Mutex<HashMap<(u64, PluginSlot), EditorSlot>>> =
            Arc::new(Mutex::new(HashMap::new()));
        // 素材缓存要先于音频引擎建好：音频线程重建宿主时要靠它把音频片段接上真实 PCM。
        let assets = Arc::new(AssetCache::new());
        let audio = if start_audio {
            AudioEngine::new(
                engine.project(),
                catalog.clone(),
                Arc::clone(&editor_handles),
                Arc::clone(&assets),
            )
        } else {
            AudioEngine::idle(
                catalog.clone(),
                Arc::clone(&editor_handles),
                Arc::clone(&assets),
            )
        };
        AppState {
            engine: Mutex::new(engine),
            audio: Arc::new(audio),
            catalog: Mutex::new(catalog),
            scan_skipped: Mutex::new(Vec::new()),
            scan_error: Mutex::new(None),
            editor_handles,
            open_editors: Arc::new(Mutex::new(HashSet::new())),
            editor_plugins: Arc::new(Mutex::new(HashMap::new())),
            host_windows: Arc::new(Mutex::new(HashSet::new())),
            editor_windows: Arc::new(Mutex::new(HashMap::new())),
            presets: Mutex::new(PresetStore::load(None)),
            sound_memories: Mutex::new(SoundMemoryStore::load(None)),
            assets,
            session_path: Mutex::new(None),
            project_key: Mutex::new(crate::history::new_key()),
            recording: Mutex::new(None),
            audio_started: AtomicBool::new(start_audio),
            ai: AiRuntime::new(),
            singing: crate::singing::SingingRuntime::new(),
        }
    }

    /// 记下本次录音的目标。
    pub fn set_recording(&self, target: Option<RecordingTarget>) {
        *self.recording.lock().unwrap() = target;
    }

    /// 取走（并清空）本次录音的目标。
    pub fn take_recording(&self) -> Option<RecordingTarget> {
        self.recording.lock().unwrap().take()
    }

    /// 记下当前工程对应的文件路径（保存到新路径 / 打开文件时调用）。
    pub fn set_session_path(&self, path: Option<String>) {
        *self.session_path.lock().unwrap() = path;
    }

    /// 当前工程对应的文件路径。
    pub fn session_path(&self) -> Option<String> {
        self.session_path.lock().unwrap().clone()
    }

    /// 当前工程的身份键（写历史时一起记下，供界面按工程归并版本）。
    pub fn project_key(&self) -> String {
        self.project_key.lock().unwrap().clone()
    }

    /// 换上新工程（新建 / 打开）时重置身份键。
    pub fn renew_project_key(&self) {
        *self.project_key.lock().unwrap() = crate::history::new_key();
    }

    /// 恢复历史 / 会话时沿用原来的身份键。
    pub fn set_project_key(&self, key: String) {
        *self.project_key.lock().unwrap() = key;
    }

    /// 装上预设文件（启动时由 `setup` 调用；拿不到配置目录时只活在内存里）。
    pub fn init_presets(&self, path: Option<PathBuf>) {
        *self.presets.lock().unwrap() = PresetStore::load(path);
    }

    /// 装上音色记忆文件（启动时由 `setup` 调用；拿不到配置目录时只活在内存里）。
    pub fn init_sound_memories(&self, path: Option<PathBuf>) {
        let mut store = SoundMemoryStore::load(path);
        // 老版本可能把整首歌存成了人声参考：启动时就地裁成 10 秒（幂等）。
        let trimmed = store.trim_long_voices(12.0);
        if trimmed > 0 {
            crate::logging::info(format!(
                "[sound-memories] 已把 {trimmed} 条过长的人声参考裁成 10 秒"
            ));
        }
        *self.sound_memories.lock().unwrap() = store;
    }

    /// 安装（或更新）插件目录：写入共享状态，内容真的变了才让音频线程重建注册表。
    ///
    /// 启动时缓存目录与后台扫描的结果通常一模一样，无脑重建会再走一遍「建流 + 激活插件」，
    /// 白花几十到上百毫秒，还会让已打开的插件界面闪一下。
    pub fn install_catalog(&self, catalog: Vec<CatalogPlugin>) {
        let changed = {
            let mut current = self.catalog.lock().unwrap();
            if *current == catalog {
                false
            } else {
                *current = catalog.clone();
                true
            }
        };
        if changed {
            self.audio.set_catalog(catalog);
        }
    }

    /// 当前插件目录的快照。
    pub fn catalog_snapshot(&self) -> Vec<CatalogPlugin> {
        self.catalog.lock().unwrap().clone()
    }

    /// 记下最近一次扫描的结果摘要（被跳过的插件 + 扫描器错误）。
    pub fn set_scan_report(&self, skipped: Vec<SkippedPlugin>, error: Option<String>) {
        *self.scan_skipped.lock().unwrap() = skipped;
        *self.scan_error.lock().unwrap() = error;
    }

    /// 最近一次扫描里被跳过的插件。
    pub fn scan_skipped_snapshot(&self) -> Vec<SkippedPlugin> {
        self.scan_skipped.lock().unwrap().clone()
    }

    /// 最近一次扫描整体失败的原因（正常为 `None`）。
    pub fn scan_error_snapshot(&self) -> Option<String> {
        self.scan_error.lock().unwrap().clone()
    }

    /// 取某条轨道某个槽位插件实例的编辑器句柄（连同它的 plugin id 与所属插件线程）。
    pub fn editor_slot(&self, track: u64, slot: PluginSlot) -> Option<EditorSlot> {
        self.editor_handles
            .lock()
            .unwrap()
            .get(&(track, slot))
            .cloned()
    }
}

/// 引擎事件广播的前端事件名。
pub const EVENT_ENGINE: &str = "engine://event";

/// 传输位置广播的前端事件名。
pub const EVENT_TRANSPORT: &str = "engine://transport";

/// 插件扫描完成的前端事件名。
pub const EVENT_PLUGINS: &str = "engine://plugins";

/// 音色记忆库变更的前端事件名。
pub const EVENT_SOUND_MEMORIES: &str = "engine://sound-memories";

/// 音频导入解码进度的前端事件名。
pub const EVENT_IMPORT: &str = "engine://import";

/// 音频导出（渲染 + 编码）进度的前端事件名。
pub const EVENT_EXPORT: &str = "engine://export";
