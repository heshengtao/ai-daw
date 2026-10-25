use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ai_daw_engine::bus::Engine;
use ai_daw_engine::model::TrackId;
use ai_daw_engine::registry::PluginSlot;
use ai_daw_host_vst3::{CatalogPlugin, EditorHandle, PluginEditorWindow, SkippedPlugin};

use crate::ai::AiRuntime;
use crate::assets::AssetCache;
use crate::audio::{AudioEngine, EditorSlot};
use crate::presets::PresetStore;
use crate::sound_memories::SoundMemoryStore;

thread_local! {
    /// 已打开的插件原生窗口。
    ///
    /// **只在主线程访问**：`PluginWindow` 持有 `NSWindow`（macOS），本来就是 `!Send`，
    /// 而且创建/销毁/泵事件都必须在主线程做，所以放在主线程局部而不是共享状态里。
    static EDITOR_WINDOWS: RefCell<HashMap<(u64, PluginSlot), PluginEditorWindow>> = RefCell::new(HashMap::new());
}

/// 在主线程里操作「(轨道, 槽位) → 插件窗口」表。
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
    /// 正在进行的一次录音的目标：`record_start` 记下，`record_stop` 取走。
    pub recording: Mutex<Option<RecordingTarget>>,
    /// 本地 AI 任务（后台线程 + 进度 + 取消）。
    pub ai: AiRuntime,
}

impl AppState {
    /// 以给定工程名新建共享状态（初始工程带默认分轨模板，且不带历史）。
    pub fn new(name: &str) -> Self {
        let engine = Engine::new_project(name);
        let editor_handles: Arc<Mutex<HashMap<(u64, PluginSlot), EditorSlot>>> =
            Arc::new(Mutex::new(HashMap::new()));
        // 素材缓存要先于音频引擎建好：音频线程重建宿主时要靠它把音频片段接上真实 PCM。
        let assets = Arc::new(AssetCache::new());
        // 目录稍后由后台扫描补齐；先用空目录启动，避免卡住窗口。
        let audio = Arc::new(AudioEngine::new(
            engine.project(),
            Vec::new(),
            Arc::clone(&editor_handles),
            Arc::clone(&assets),
        ));
        AppState {
            engine: Mutex::new(engine),
            audio,
            catalog: Mutex::new(Vec::new()),
            scan_skipped: Mutex::new(Vec::new()),
            scan_error: Mutex::new(None),
            editor_handles,
            open_editors: Arc::new(Mutex::new(HashSet::new())),
            editor_plugins: Arc::new(Mutex::new(HashMap::new())),
            host_windows: Arc::new(Mutex::new(HashSet::new())),
            presets: Mutex::new(PresetStore::load(None)),
            sound_memories: Mutex::new(SoundMemoryStore::load(None)),
            assets,
            session_path: Mutex::new(None),
            recording: Mutex::new(None),
            ai: AiRuntime::new(),
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

    /// 装上预设文件（启动时由 `setup` 调用；拿不到配置目录时只活在内存里）。
    pub fn init_presets(&self, path: Option<PathBuf>) {
        *self.presets.lock().unwrap() = PresetStore::load(path);
    }

    /// 装上音色记忆文件（启动时由 `setup` 调用；拿不到配置目录时只活在内存里）。
    pub fn init_sound_memories(&self, path: Option<PathBuf>) {
        *self.sound_memories.lock().unwrap() = SoundMemoryStore::load(path);
    }

    /// 安装（或更新）插件目录：写入共享状态并让音频线程重建注册表。
    pub fn install_catalog(&self, catalog: Vec<CatalogPlugin>) {
        *self.catalog.lock().unwrap() = catalog.clone();
        self.audio.set_catalog(catalog);
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

    /// 用会话缓存恢复工作状态（启动时调用）。
    ///
    /// 工作副本与保存基线一起恢复，因此带着未保存改动退出的工程，下次打开依旧显示「未保存」。
    pub fn restore_session(&self, session: crate::session::SessionFile) {
        let project = session.project;
        *self.engine.lock().unwrap() = Engine::restore(project.clone(), session.saved);
        self.audio.load(&project);
        *self.session_path.lock().unwrap() = session.path;
    }

    /// 取某条轨道某个槽位插件实例的编辑器句柄。
    pub fn editor_handle(&self, track: u64, slot: PluginSlot) -> Option<EditorHandle> {
        self.editor_handles
            .lock()
            .unwrap()
            .get(&(track, slot))
            .map(|entry| entry.handle.clone())
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
