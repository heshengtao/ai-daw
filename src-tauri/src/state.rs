use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use daw_party_engine::bus::Engine;
use daw_party_host_vst3::{CatalogPlugin, EditorHandle, PluginEditorWindow};

use crate::audio::AudioEngine;
use crate::presets::PresetStore;

thread_local! {
    /// 已打开的插件原生窗口。
    ///
    /// **只在主线程访问**：`PluginWindow` 持有 `NSWindow`（macOS），本来就是 `!Send`，
    /// 而且创建/销毁/泵事件都必须在主线程做，所以放在主线程局部而不是共享状态里。
    static EDITOR_WINDOWS: RefCell<HashMap<u64, PluginEditorWindow>> = RefCell::new(HashMap::new());
}

/// 在主线程里操作「轨道 → 插件窗口」表。
pub fn with_editor_windows<R>(f: impl FnOnce(&mut HashMap<u64, PluginEditorWindow>) -> R) -> R {
    EDITOR_WINDOWS.with(|windows| f(&mut windows.borrow_mut()))
}

/// 应用共享状态：进程内持有唯一的引擎实例与音频引擎句柄。
pub struct AppState {
    /// 领域引擎（工程模型 + 撤销历史）。所有编辑都必须经过它。
    pub engine: Mutex<Engine>,
    /// 实时音频宿主（运行在专用线程上）。
    pub audio: Arc<AudioEngine>,
    /// 已扫描到的插件目录（启动时后台扫描填充；离线渲染也用它）。
    pub catalog: Mutex<Vec<CatalogPlugin>>,
    /// 「轨道 → 插件编辑器句柄」：插件工厂在实例创建时登记，界面据此打开原生窗口。
    pub editor_handles: Arc<Mutex<HashMap<u64, EditorHandle>>>,
    /// 哪几条轨道开着插件窗口（主线程那张表的**可跨线程镜像**，供界面查询）。
    pub open_editors: Arc<Mutex<HashSet<u64>>>,
    /// 其中哪些轨道的窗口是**宿主建的**（插件只支持嵌入模式）。用户关掉那扇窗就等于关界面。
    pub host_windows: Arc<Mutex<HashSet<u64>>>,
    /// 用户预设（跨工程；文件位置由 `init_presets` 装上）。
    pub presets: Mutex<PresetStore>,
}

impl AppState {
    /// 以给定工程名新建共享状态（初始工程带默认分轨模板，且不带历史）。
    pub fn new(name: &str) -> Self {
        let engine = Engine::new_project(name);
        let editor_handles: Arc<Mutex<HashMap<u64, EditorHandle>>> =
            Arc::new(Mutex::new(HashMap::new()));
        // 目录稍后由后台扫描补齐；先用空目录启动，避免卡住窗口。
        let audio = Arc::new(AudioEngine::new(
            engine.project(),
            Vec::new(),
            Arc::clone(&editor_handles),
        ));
        AppState {
            engine: Mutex::new(engine),
            audio,
            catalog: Mutex::new(Vec::new()),
            editor_handles,
            open_editors: Arc::new(Mutex::new(HashSet::new())),
            host_windows: Arc::new(Mutex::new(HashSet::new())),
            presets: Mutex::new(PresetStore::load(None)),
        }
    }

    /// 装上预设文件（启动时由 `setup` 调用；拿不到配置目录时只活在内存里）。
    pub fn init_presets(&self, path: Option<PathBuf>) {
        *self.presets.lock().unwrap() = PresetStore::load(path);
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

    /// 取某条轨道当前插件实例的编辑器句柄。
    pub fn editor_handle(&self, track: u64) -> Option<EditorHandle> {
        self.editor_handles.lock().unwrap().get(&track).cloned()
    }
}

/// 引擎事件广播的前端事件名。
pub const EVENT_ENGINE: &str = "engine://event";

/// 传输位置广播的前端事件名。
pub const EVENT_TRANSPORT: &str = "engine://transport";

/// 插件扫描完成的前端事件名。
pub const EVENT_PLUGINS: &str = "engine://plugins";
