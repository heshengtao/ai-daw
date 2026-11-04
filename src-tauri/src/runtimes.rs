//! **ONNX Runtime 运行时的下载 / 安装 / 删除**——设置页「AI → 运行时」用。
//!
//! 清单在 `ai_daw_local_ai::backend::runtime`（编译期内嵌：URL / 体积 / 归档内路径 / 必需文件清单），
//! 这里只做「落到磁盘」这件事：
//!
//! ```text
//! <应用配置目录>/runtime/ort/
//!   cpu/        onnxruntime.dll (+ providers_shared)
//!   cuda/       onnxruntime.dll + onnxruntime_providers_cuda.dll（CUDA 12）
//!   cuda13/     同上（CUDA 13，ORT 1.31 构建）
//!   cudart/ cublas/ cufft/ cudnn/            ← CUDA 12 的组件
//!   cudart13/ cublas13/ cufft13/ cudnn13/    ← CUDA 13 的组件
//! ```
//!
//! 复用插件 / 模型那套下载基建（多源回退、代理、`Range` 断点续传、sha256 硬门禁、
//! 取消），进度走 `engine://runtime`。
//!
//! **装完要重启**：`ort` 的 dylib 在进程内只能加载一次（`OnceLock`），换了运行时也换不了
//! 已经加载的那份。界面据此显示「重启后生效」。

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use ai_daw_local_ai::backend::runtime;
use ai_daw_local_ai::backend::runtime::{
    asset, for_platform, install_dir, RuntimeAsset, RuntimeId, RuntimeSource,
};

use crate::manager::{
    download_candidates, download_verified, probe_agent, resolve_proxy, ModelSourcePreference,
    StoreNetwork, StoreRuntime,
};

/// 运行时下载进度广播的前端事件名。
pub const EVENT_RUNTIME: &str = "engine://runtime";

/// 进度事件载荷。
#[derive(Debug, Clone, serde::Serialize)]
pub struct RuntimeProgress {
    /// 运行时 id（`cpu` / `cuda` / …）。
    pub id: String,
    /// `downloading` / `done` / `error`。
    pub state: String,
    pub received: u64,
    pub total: u64,
    pub message: Option<String>,
}

/// 一个运行时的本地状态（界面用）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct RuntimeStatus {
    pub id: String,
    pub label: String,
    pub note: String,
    /// 需要下载的字节数。
    pub bytes: u64,
    /// 是否已装好。
    pub installed: bool,
    /// 是否是当前设定要用的那一份。
    pub selected: bool,
    /// 是否是**进程内已经加载**的那一份（换它必须重启）。
    pub loaded: bool,
    /// 随安装包分发（CPU / DirectML / CoreML）：不下载、不可删除。
    pub bundled: bool,
    /// 装好的目录（给「显示位置」用）。
    pub dir: String,
    /// 属于哪一套：`ort`（ONNX Runtime）/ `audiocpp`（audio.cpp 引擎）。
    pub group: String,
    /// 是否是引擎的默认那一份（audio.cpp 的 CPU 兜底；ORT 恒为 false）。
    pub core: bool,
    /// 引擎是否处于「自动挑后端」状态（用户没手动选过）；ORT 恒为 false。
    pub automatic: bool,
    /// 引擎「自动模式」会用的设备（`cpu` / `vulkan` / `cuda12` / `cuda13`）；ORT 恒为空。
    pub device: String,
    /// 是不是本机最优的那一份（引擎用；ORT 恒为 false）。与装没装无关。
    pub recommended: bool,
    /// 本机能不能真跑（引擎用；ORT 恒为 true）。AMD 机器上的 CUDA = false。
    pub usable: bool,
}

struct Downloader {
    running: AtomicBool,
    cancel: AtomicBool,
}

fn downloader() -> &'static Downloader {
    static RUNTIME: OnceLock<Downloader> = OnceLock::new();
    RUNTIME.get_or_init(|| Downloader {
        running: AtomicBool::new(false),
        cancel: AtomicBool::new(false),
    })
}

/// 运行时根目录（应用配置目录的 `runtime/ort/`）。
pub fn runtime_root(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = crate::paths::config_dir(app)?.join("runtime").join("ort");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// 启动时调用：把根目录交给 `local-ai`，让它去加载 dylib。
pub fn init(app: &AppHandle) {
    if let Ok(dir) = runtime_root(app) {
        ai_daw_local_ai::backend::runtime::set_root(dir);
    }
    if let Some(dir) = bundled_dir(app) {
        ai_daw_local_ai::backend::runtime::set_bundled_dir(dir);
    }
    // 上一次删除没删掉的目录：**必须在任何 dylib 被加载之前**再来一次，
    // 否则这一进程又会把同样的文件映射住，永远删不掉。
    cleanup_pending(app);
    // 重启之后安装 / 删除就已经生效了，把「待重启」标记清掉。
    clear_restart_needed();
}

/// 随包运行时目录（安装包里的 `ort/`）。
///
/// 打包后是 `<资源目录>/ort`；开发时资源不会自动拷到 target 目录，所以再兜一个
/// `src-tauri/resources/ort`（`npm run fetch:ort` 生成）。
fn bundled_dir(app: &AppHandle) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("AIDAW_ORT_BUNDLED") {
        candidates.push(PathBuf::from(dir));
    }
    if let Ok(dir) = app.path().resource_dir() {
        candidates.push(dir.join("ort"));
        candidates.push(dir.join("resources").join("ort"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("ort"));
        }
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("ort"),
    );
    candidates.into_iter().find(|dir| has_core_library(dir))
}

/// 目录里有没有 ONNX Runtime 核心库。
///
/// Linux 的官方包只放带版本号的 `libonnxruntime.so.1.30.0`（没有 `libonnxruntime.so`
/// 软链），所以不能只按固定文件名判断。
fn has_core_library(dir: &Path) -> bool {
    let name = ai_daw_local_ai::backend::runtime::library_name();
    if dir.join(name).is_file() {
        return true;
    }
    let prefix = if name.ends_with(".dll") {
        "onnxruntime.dll"
    } else {
        "libonnxruntime.so"
    };
    fs::read_dir(dir)
        .map(|entries| {
            entries.flatten().any(|entry| {
                let file = entry.file_name();
                let text = file.to_string_lossy();
                entry.path().is_file() && text.starts_with(prefix) && !text.contains("providers")
            })
        })
        .unwrap_or(false)
}

/// 全部运行时的状态（设置页用）。
#[tauri::command]
pub fn ai_runtime_catalog(app: AppHandle) -> Vec<RuntimeStatus> {
    statuses(&app)
}

// ---------------------------------------------------------------------------
// 「待删除」清单：DLL 被本进程映射时删不掉，留到下次启动再删
// ---------------------------------------------------------------------------

/// 待删除 id 清单（`<配置目录>/runtime/ort/pending-remove.json`）。
fn pending_file(app: &AppHandle) -> Option<PathBuf> {
    crate::paths::config_dir(app)
        .ok()
        .map(|dir| dir.join("runtime").join("ort").join("pending-remove.json"))
}

fn pending_ids(app: &AppHandle) -> Vec<String> {
    pending_file(app)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .unwrap_or_default()
}

fn write_pending(app: &AppHandle, ids: &[String]) -> Result<(), String> {
    let Some(path) = pending_file(app) else {
        return Err("拿不到应用配置目录".into());
    };
    if ids.is_empty() {
        let _ = fs::remove_file(&path);
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(ids).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| e.to_string())
}

/// 记下「这次没删掉、下次启动再删」。界面上从这一刻起就当它不存在了。
fn mark_pending(app: &AppHandle, id: &str) -> Result<(), String> {
    let mut ids = pending_ids(app);
    if !ids.iter().any(|existing| existing == id) {
        ids.push(id.to_string());
    }
    write_pending(app, &ids)
}

/// 重新安装 / 已经删干净：把待删除记录撤掉，免得下次启动把新装的又删了。
fn clear_pending(app: &AppHandle, id: &str) {
    let before = pending_ids(app);
    let mut ids = before.clone();
    ids.retain(|existing| existing != id);
    if ids.len() != before.len() {
        let _ = write_pending(app, &ids);
    }
}

/// 启动时清理待删除目录（此时本进程还没加载任何 dylib，文件不再被占用）。
fn cleanup_pending(app: &AppHandle) {
    let ids = pending_ids(app);
    if ids.is_empty() {
        return;
    }
    let mut left: Vec<String> = Vec::new();
    for id in ids {
        let Some(parsed) = RuntimeId::parse(&id) else {
            continue;
        };
        let dir = install_dir(parsed);
        if !dir.exists() {
            continue;
        }
        match fs::remove_dir_all(&dir) {
            Ok(()) => eprintln!("[runtime] 已清理待删除的目录：{}", dir.display()),
            Err(e) => {
                eprintln!("[runtime] 待删除的目录仍被占用（{}）：{e}", dir.display());
                left.push(id);
            }
        }
    }
    let _ = write_pending(app, &left);
}

/// 运行时在下载管理器里的分组：本体 `ort`，CUDA 组件按大版本分 `cuda12` / `cuda13`。
///
/// **别用 `cuda_flavor()` 直接分组**：`Cuda13` 本体也有 flavor，但它必须留在 `ort`
/// 里（和 Cuda12 本体并列），只有组件才进 `cuda13` 那一组。
fn group_of(id: RuntimeId) -> &'static str {
    if !id.is_component() {
        return "ort";
    }
    if id.cuda_flavor() == Some(13) {
        "cuda13"
    } else {
        "cuda12"
    }
}

/// 全部运行时的状态（按本机平台过滤）——ONNX Runtime 那几份，加上 audio.cpp 的引擎后端。
///
/// 两者共用同一套界面（下载 / 重新安装 / 删除 / 显示位置），靠 `group` 区分；
/// 引擎在 macOS / Linux 上暂时没有包，`yue::statuses` 自己返回空表。
pub fn statuses(app: &AppHandle) -> Vec<RuntimeStatus> {
    let _ = runtime_root(app);
    let selected = ai_daw_local_ai::backend::runtime::chosen();
    // 「待删除」的条目已经不算装着了：文件要等下次启动才清理得掉，界面不该还把它
    // 列在「已安装」里（用户点了删除却原地不动，看起来像删除没生效）。
    let pending = pending_ids(app);
    // 只问「已经加载的那份」：这里每次打开管理器都会被调到，不能顺手把 180 MB 的
    // CUDA provider 拉进进程（那正是「切了 CUDA 之后启动很卡」的一半原因）。
    let loaded = ai_daw_local_ai::backend::runtime::loaded_path()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let mut rows: Vec<RuntimeStatus> = for_platform(std::env::consts::OS, std::env::consts::ARCH)
        .into_iter()
        .map(|a| {
            let dir = install_dir(a.id);
            RuntimeStatus {
                id: a.id.as_str().to_string(),
                label: a.label.to_string(),
                note: a.note.to_string(),
                bytes: a.bytes(),
                installed: !pending.iter().any(|id| id == a.id.as_str())
                    && ai_daw_local_ai::backend::runtime::installed(a.id),
                bundled: a.bundled,
                selected: a.id == selected,
                loaded: loaded.as_deref() == Some(dir.as_path()),
                dir: dir.to_string_lossy().into_owned(),
                group: group_of(a.id).into(),
                core: false,
                automatic: false,
                device: String::new(),
                recommended: false,
                usable: true,
            }
        })
        .collect();
    rows.extend(
        crate::yue::statuses(app)
            .into_iter()
            .map(|s| RuntimeStatus {
                id: s.id,
                label: s.label,
                note: s.note,
                bytes: s.bytes,
                installed: s.installed,
                // 引擎没有「进程内已加载」这回事：每次启动都是一次新进程。
                selected: s.selected,
                loaded: false,
                bundled: false,
                dir: s.dir,
                group: "audiocpp".into(),
                core: s.core,
                automatic: s.automatic,
                device: s.device,
                recommended: s.recommended,
                usable: s.usable,
            }),
    );
    rows
}

/// 「装 / 删过运行时而本进程还没重启」的标记文件。
///
/// 为什么不能只比较「已加载路径 vs 期望路径」：**CUDA 组件**（cudart / cuBLAS /
/// cuFFT / cuDNN）是在 `load()` 那一刻一次性预载进进程的。之后再补装它们，已经注册好
/// 的 provider 用不上——可加载路径根本没变，只看路径会得出「不用重启」的错误结论，
/// 用户装完看到「已安装」，却依旧在跑 CPU。
fn pending_restart_path() -> PathBuf {
    ai_daw_local_ai::backend::runtime::root().join("pending-restart")
}

/// 记下「本进程里的运行时已经过时了」。
fn mark_restart_needed() {
    let path = pending_restart_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&path, b"1");
}

/// 启动时清掉标记：重启之后改动就已经生效了。
fn clear_restart_needed() {
    let _ = fs::remove_file(pending_restart_path());
}

/// 这次装 / 删会不会影响**当前选用**的那份运行时。
///
/// 在 CPU 后端下补下 CUDA 组件不该提示重启——那份运行时根本没加载。
fn affects_chosen(id: RuntimeId) -> bool {
    let chosen = ai_daw_local_ai::backend::runtime::chosen();
    // 组件只影响同大版本的那份 CUDA（12 的组件不会影响 CUDA 13 本体）。
    chosen == id
        || (id.is_component() && chosen.cuda_flavor() == id.cuda_flavor() && chosen.is_cuda())
}

/// 当前选用的那份运行时是否**完整可用**（本体 + 同大版本的全部组件）。
///
/// 只装到一半时不提示重启：重启也仍然缺库，provider 照样注册不上。等最后一个组件
/// 装完，`restart_required` 才会变真——否则用户会在下载队列还没跑完时就被弹窗打断。
fn chosen_runtime_ready() -> bool {
    use ai_daw_local_ai::backend::runtime;
    let chosen = runtime::chosen();
    runtime::installed(chosen)
        && runtime::components_of(chosen)
            .into_iter()
            .all(runtime::installed)
}

/// 当前设定要用的运行时是否**已经加载**；不是就意味着「重启后生效」。
///
/// **不触发加载**：这个查询在启动路径与后端信息里都会被调到，CUDA 那份加载要几秒。
/// 进程里还没加载过任何一份时返回 false（下次建会话才会按当前选择加载，谈不上重启）。
pub fn restart_required() -> bool {
    // macOS 静态链接：没有可换的运行时。
    if ai_daw_local_ai::backend::runtime::statically_linked() {
        return false;
    }
    // 正在下载 / 解压：此时文件可能还是一半，等这一轮装完再判断。
    if downloader().running.load(Ordering::SeqCst) {
        return false;
    }
    // 本体或组件还没装齐：现在重启没有意义，等下载完再说。
    if !chosen_runtime_ready() {
        return false;
    }
    let Some(loaded) = ai_daw_local_ai::backend::runtime::loaded_path() else {
        return false;
    };
    if loaded
        != ai_daw_local_ai::backend::runtime::library_path(
            ai_daw_local_ai::backend::runtime::chosen(),
        )
    {
        return true;
    }
    // 路径没变，但本进程加载之后又装 / 删过东西（典型：补装 cuDNN 或 cuFFT）。
    pending_restart_path().is_file()
}

fn parse_id(id: &str) -> Result<RuntimeId, String> {
    RuntimeId::parse(id).ok_or_else(|| format!("未知运行时：{id}"))
}

fn asset_of(id: &str) -> Result<&'static RuntimeAsset, String> {
    let id = parse_id(id)?;
    asset(id).ok_or_else(|| format!("本机没有 {} 这份运行时", id.as_str()))
}

/// 开始下载并安装一个运行时（后台线程；进度走 `engine://runtime`）。
#[tauri::command]
pub fn ai_runtime_install(app: AppHandle, id: String) -> Result<(), String> {
    // 引擎后端（`audiocpp-*`）与 ONNX Runtime 是两条安装路径，界面只认一个 id。
    if id.starts_with("audiocpp-") {
        return crate::yue::ai_yue_install(app, id);
    }
    let asset = asset_of(&id)?;
    if asset.bundled {
        return Err(format!("{} 随安装包分发，不需要下载", asset.label));
    }
    if downloader().running.swap(true, Ordering::SeqCst) {
        return Err("已有运行时正在下载".into());
    }
    downloader().cancel.store(false, Ordering::Relaxed);
    let thread_app = app.clone();
    thread::Builder::new()
        .name("ai-daw-runtime".into())
        .spawn(move || run_install(&thread_app, asset))
        .map_err(|e| {
            downloader().running.store(false, Ordering::SeqCst);
            e.to_string()
        })?;
    Ok(())
}

/// 取消下载（线程在下一个数据块处停下，并删掉半成品）。
#[tauri::command]
pub fn ai_runtime_cancel() {
    crate::yue::ai_yue_cancel();
    downloader().cancel.store(true, Ordering::Relaxed);
}

/// 当前是否有运行时正在下载（界面挂载时用来纠正「卡在下载中」的状态）。
#[tauri::command]
pub fn ai_runtime_busy() -> bool {
    downloader().running.load(Ordering::SeqCst) || crate::yue::busy()
}

/// 删除一个已下载的运行时（只删下载目录）。
///
/// 文件被本进程映射时（Windows 删不掉已加载的 dll）不报错：记进「待删除」，
/// 下次启动在加载任何 dylib 之前清掉。
#[tauri::command]
pub fn ai_runtime_remove(app: AppHandle, id: String) -> Result<(), String> {
    if id.starts_with("audiocpp-") {
        return crate::yue::ai_yue_remove(app, id);
    }
    let asset = asset_of(&id)?;
    if asset.bundled {
        return Err(format!("{} 随安装包分发，不能删除", asset.label));
    }
    let dir = install_dir(asset.id);
    if !dir.exists() {
        clear_pending(&app, asset.id.as_str());
        return Ok(());
    }
    // 删掉的是当前在用的那份（或它的组件）：本进程里它还映射着，重启后才真正释放。
    if affects_chosen(asset.id) {
        mark_restart_needed();
    }
    match fs::remove_dir_all(&dir) {
        Ok(()) => {
            clear_pending(&app, asset.id.as_str());
            Ok(())
        }
        Err(e) => {
            // Windows 上 dll 一旦被本进程映射（CUDA 后端跑过一次就会这样），目录就删不掉。
            // 不把它当成失败：记进「待删除」，界面立刻按已删除显示，下次启动在加载任何
            // dylib 之前把目录删干净。
            mark_pending(&app, asset.id.as_str()).map_err(|mark| {
                format!("删除 {} 失败：{e}（记录待清理也失败：{mark}）", asset.label)
            })?;
            eprintln!(
                "[runtime] {} 的文件正在使用，已安排下次启动清理：{}",
                asset.label,
                dir.display()
            );
            Ok(())
        }
    }
}

/// 在系统文件管理器里显示运行时目录。
#[tauri::command]
pub fn ai_runtime_reveal(id: String) -> Result<(), String> {
    if id.starts_with("audiocpp-") {
        return crate::yue::ai_yue_reveal(id);
    }
    let asset = asset_of(&id)?;
    let dir = install_dir(asset.id);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    reveal(&dir)
}

/// 本机发现的现成 CUDA 版 ONNX Runtime（pip / conda / 官方解压目录）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredRuntime {
    /// 目录（core 与 provider 同在一个目录里）。
    pub dir: String,
    /// 版本；读不到时为空。
    pub version: String,
    /// `python` / `conda` / `file`。
    pub source: String,
    /// CUDA 大版本（`12` / `13`）：借来的那份要放进同版本的组件目录。
    pub flavor: String,
}

/// 扫描本机已有的 CUDA 版 ONNX Runtime（只做有限深度，设置页打开时调用）。
#[tauri::command]
pub fn ai_runtime_discover() -> Vec<DiscoveredRuntime> {
    // macOS 静态链接、也没有 CUDA 后端：没有要借的运行时。
    if ai_daw_local_ai::backend::runtime::statically_linked() {
        return Vec::new();
    }
    ai_daw_local_ai::backend::runtime::discover_cuda()
        .into_iter()
        .map(|d| DiscoveredRuntime {
            dir: d.dir.to_string_lossy().into_owned(),
            version: d.version.unwrap_or_default(),
            source: d.source,
            flavor: d.flavor.to_string(),
        })
        .collect()
}

/// 把发现的那一份**复制**进应用自己的运行时目录（对方升级 / 卸载都不再影响我们）。
#[tauri::command]
pub fn ai_runtime_import(dir: String) -> Result<(), String> {
    let installed = ai_daw_local_ai::backend::runtime::import_cuda(std::path::Path::new(&dir))?;
    // 借来的这份同样是「装好了但本进程没加载」：CUDA 正在用时得重启才生效。
    // 目录名就是运行时 id（`cuda` / `cuda13`），按它判断影响的是哪一份。
    let id = installed
        .file_name()
        .and_then(|name| RuntimeId::parse(&name.to_string_lossy()))
        .unwrap_or(RuntimeId::Cuda);
    if affects_chosen(id) {
        mark_restart_needed();
    }
    Ok(())
}

/// 本机推荐的后端：NVIDIA + 新卡用 CUDA 13，否则 CUDA 12；都不是就 CPU。
///
/// 「推荐」只看本机是否跑得动，不看装没装——设置页会把「需要下载」单独标出来。
pub fn recommended_backend() -> ai_daw_local_ai::backend::BackendKind {
    use ai_daw_local_ai::backend::{detect, runtime, BackendKind};
    let profile = detect::detect_cached();
    let nvidia = profile.gpu == Some(detect::GpuVendor::Nvidia);
    if nvidia {
        if runtime::cuda13_supported(profile) && runtime::asset(RuntimeId::Cuda13).is_some() {
            return BackendKind::Cuda13;
        }
        if runtime::asset(RuntimeId::Cuda).is_some() {
            return BackendKind::Cuda12;
        }
    }
    BackendKind::Cpu
}

// ---------------------------------------------------------------------------
// 下载 + 解压
// ---------------------------------------------------------------------------

fn emit(
    app: &AppHandle,
    id: &str,
    state: &str,
    received: u64,
    total: u64,
    message: Option<String>,
) {
    let _ = app.emit(
        EVENT_RUNTIME,
        RuntimeProgress {
            id: id.to_string(),
            state: state.to_string(),
            received,
            total,
            message,
        },
    );
}

fn run_install(app: &AppHandle, asset: &'static RuntimeAsset) {
    // CUDA 本体：顺手把**同大版本**的缺失组件一起装上——用户点一次「下载 CUDA」就够，
    // 不用再逐个点 cudart / cuBLAS / cuFFT / cuDNN；CUDA 12 与 13 两套不会混。
    let components: Vec<&'static RuntimeAsset> = if asset.id.is_cuda() {
        runtime::components_of(asset.id)
            .into_iter()
            .filter(|id| !runtime::installed(*id))
            .filter_map(runtime::asset)
            .collect()
    } else {
        Vec::new()
    };
    let total = asset.bytes() + components.iter().map(|a| a.bytes()).sum::<u64>();

    let result = install_bundle(app, asset, &components, total);
    downloader().running.store(false, Ordering::SeqCst);
    match result {
        Ok(()) => {
            // 装好了就把「待删除」撤掉：否则下次启动会把刚装上的这份当残留删掉。
            for done in std::iter::once(asset).chain(components.iter().copied()) {
                clear_pending(app, done.id.as_str());
                if affects_chosen(done.id) {
                    mark_restart_needed();
                }
            }
            emit(app, asset.id.as_str(), "done", total, total, None)
        }
        Err(message) => emit(app, asset.id.as_str(), "error", 0, 0, Some(message)),
    }
}

/// 装一整包：本体 + 它的组件（组件进度折算进同一条进度条）。
fn install_bundle(
    app: &AppHandle,
    asset: &'static RuntimeAsset,
    components: &[&'static RuntimeAsset],
    total: u64,
) -> Result<(), String> {
    emit(app, asset.id.as_str(), "downloading", 0, total, None);
    let mut base = 0u64;
    install_parts(app, asset, asset.id.as_str(), base, total)?;
    base += asset.bytes();
    for component in components {
        install_parts(app, component, asset.id.as_str(), base, total)?;
        base += component.bytes();
    }
    Ok(())
}

/// 装一份归档：`report_id` 是进度事件上挂的 id（组件也挂在 CUDA 本体下）。
fn install_parts(
    app: &AppHandle,
    asset: &'static RuntimeAsset,
    report_id: &str,
    base: u64,
    total: u64,
) -> Result<(), String> {
    let dir = install_dir(asset.id);
    fs::create_dir_all(&dir).map_err(|e| format!("建目录失败：{e}"))?;
    let network = app.state::<StoreRuntime>().network();
    let proxy = resolve_proxy(&network).proxy;
    // 一次安装只探一次区域：五个组件共用同一个网络结论。`auto`（默认）下这一步决定
    // 先走境外源还是国内镜像——不探就会在连不上的境外源上按 30 秒超时逐个卡过去。
    let region = asset
        .parts
        .first()
        .and_then(|part| preferred_region(part.sources, &network, proxy.as_ref()));
    if let Some(region) = region {
        eprintln!(
            "[runtime] 下载源按{}优先",
            if region == "cn" {
                "国内镜像"
            } else {
                "境外官方源"
            }
        );
    }
    let cancel = &downloader().cancel;
    let mut done = 0u64;

    for part in asset.parts {
        let archive = dir.join(part.archive);
        let candidates = candidates_for(part.sources, &network, region);
        if candidates.is_empty() {
            return Err(format!("{} 没有可用的下载源", part.archive));
        }
        eprintln!(
            "[runtime] 下载 {}（{} MB，首个源 {}）",
            part.archive,
            part.bytes / (1024 * 1024),
            candidates[0]
        );

        let plan = part.pick_bytes.unwrap_or(part.bytes);
        let archive_bytes = part.bytes.max(1);
        let part_base = done;
        let mut progress = |received: u64, _total: u64| {
            // 回退整包下载时把「归档字节」折算回计划字节，长条不会一下冲满。
            let advanced = if plan == part.bytes {
                received
            } else {
                received.saturating_mul(plan) / archive_bytes
            };
            emit(
                app,
                report_id,
                "downloading",
                (base + part_base + advanced.min(plan)).min(total),
                total,
                None,
            );
        };

        // 优先「只取清单里那几个文件」：官方 release 包是整套 SDK（解压 417 MB），
        // DirectML 的 nupkg 里还塞着各架构的 DLL——整包下 79 MB / 205 MB，只取只要 6 MB / 15 MB。
        let picked = if part.pick_bytes.is_some() {
            pick_install(
                &candidates,
                part,
                &dir,
                proxy.as_ref(),
                cancel,
                &mut progress,
            )
        } else {
            Err("清单没给 pick 尺寸".into())
        };
        if picked.is_ok() {
            done += plan;
            emit(
                app,
                report_id,
                "downloading",
                (base + done).min(total),
                total,
                None,
            );
            continue;
        }
        eprintln!("[runtime] 回退整包下载：{}", picked.unwrap_err());

        download_verified(
            &candidates,
            &archive,
            part.sha256.unwrap_or(""),
            part.bytes,
            proxy.as_ref(),
            Some(cancel),
            &mut progress,
        )?;
        if cancel.load(Ordering::Relaxed) {
            let _ = fs::remove_file(&archive);
            return Err("已取消".into());
        }

        extract(&archive, part.files, &dir)?;
        let _ = fs::remove_file(&archive);
        done += plan;
        emit(
            app,
            report_id,
            "downloading",
            (base + done).min(total),
            total,
            None,
        );
    }

    Ok(())
}

/// 探测一个地址的「首字节延迟」；连不上 / 超时 / 非 2xx 返回 `None`。
///
/// 与模型下载器同一套做法：`Range: bytes=0-0` 只取一个字节，既确认可达，又不真的下载。
fn probe_latency(agent: &ureq::Agent, url: &str) -> Option<Duration> {
    let started = Instant::now();
    let resp = agent.get(url).header("Range", "bytes=0-0").call().ok()?;
    let status = resp.status().as_u16();
    if status == 200 || status == 206 {
        Some(started.elapsed())
    } else {
        None
    }
}

/// `auto` 偏好：国内外各探一个源，谁通 / 谁快就先用谁。
///
/// 这是运行时下载「不挂代理就卡在境外源上很久」的根：CUDA 的清单把
/// `nuget.org` / `developer.download.nvidia.com` 排在前面，而这两家在境内多半连不上，
/// 下载器的连接超时是 30 秒、还要重试——五个组件各卡一轮就是好几分钟。
/// 探测用 [`probe_agent`]（3 秒连接 / 4 秒首响应），最坏几秒就能给出答案。
fn probe_region(sources: &[RuntimeSource], proxy: Option<&ureq::Proxy>) -> Option<&'static str> {
    let intl = sources.iter().find(|s| s.region == "intl")?;
    let cn = sources.iter().find(|s| s.region == "cn")?;
    let agent = probe_agent(proxy);
    let intl_ms = probe_latency(&agent, intl.url);
    let cn_ms = probe_latency(&agent, cn.url);
    match (intl_ms, cn_ms) {
        (Some(a), Some(b)) => Some(if a <= b { "intl" } else { "cn" }),
        (Some(_), None) => Some("intl"),
        (None, Some(_)) => Some("cn"),
        (None, None) => None,
    }
}

/// 这一次安装该优先哪个区域的源：`auto` 才联网探测，显式偏好直接用。
///
/// 一次安装只调用一次（几个组件共用同一个结论）：区域连通性只跟网络有关。
fn preferred_region(
    sources: &[RuntimeSource],
    network: &StoreNetwork,
    proxy: Option<&ureq::Proxy>,
) -> Option<&'static str> {
    match network.model_source_preference {
        ModelSourcePreference::International => Some("intl"),
        ModelSourcePreference::Domestic => Some("cn"),
        ModelSourcePreference::Auto => probe_region(sources, proxy),
    }
}

/// 候选地址：区域偏好排序 + 用户镜像 + 代理前缀。
fn candidates_for(
    sources: &'static [RuntimeSource],
    network: &StoreNetwork,
    region: Option<&str>,
) -> Vec<String> {
    // 偏好区域的源排前面，其余保持清单原序（`sort_by_key` 是稳定排序）。
    let mut ordered: Vec<_> = sources.iter().collect();
    if let Some(region) = region {
        ordered.sort_by_key(|s| if s.region == region { 0 } else { 1 });
    }
    let mut out = Vec::new();
    let mut push = |url: String| {
        if !url.is_empty() && !out.iter().any(|existing| existing == &url) {
            out.push(url);
        }
    };
    for source in ordered {
        for mirror in &network.model_mirrors {
            push(apply_prefix(mirror, source.url));
        }
        for candidate in download_candidates(source.url, &network.download_prefix) {
            push(candidate);
        }
    }
    out
}

fn apply_prefix(prefix: &str, url: &str) -> String {
    let prefix = prefix.trim();
    if prefix.is_empty() {
        return url.to_string();
    }
    if prefix.contains("{url}") {
        prefix.replace("{url}", url)
    } else {
        format!("{prefix}{url}")
    }
}

// ---------------------------------------------------------------------------
// 只取所需文件：HTTP Range + zip 中央目录（实现在 `crate::pick`）
// ---------------------------------------------------------------------------

/// 依次试候选源，用 Range 只抓清单里那几个文件。
fn pick_install(
    candidates: &[String],
    part: &'static ai_daw_local_ai::backend::runtime::RuntimePart,
    dir: &Path,
    proxy: Option<&ureq::Proxy>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), String> {
    // 下载用的 agent：探测那套 3–6 秒超时会把 5 MB 的 Range 读超时掉。
    let agent = crate::pick::agent(proxy);
    let targets: Vec<crate::pick::PickTarget> = part
        .files
        .iter()
        .map(|f| crate::pick::PickTarget {
            inner: f.inner,
            dest: f.dest,
            sha256: None,
        })
        .collect();
    let plan = part.pick_bytes.unwrap_or(part.bytes);
    let mut last = String::new();
    for url in candidates {
        if cancel.load(Ordering::Relaxed) {
            return Err("已取消".into());
        }
        match crate::pick::pick(&agent, url, &targets, plan, dir, cancel, progress) {
            Ok(outcome) => {
                eprintln!(
                    "[runtime] 只取所需文件：传输 {} KB",
                    outcome.transferred / 1024
                );
                return Ok(());
            }
            Err(e) => {
                last = format!("{url} → {e}");
                eprintln!("[runtime] 只取所需文件失败：{last}");
            }
        }
    }
    Err(last)
}

/// 从归档里只取出清单列出的文件（`dest` 是相对运行时目录的固定名字，不会目录穿越）。
fn extract(
    archive: &Path,
    files: &'static [ai_daw_local_ai::backend::runtime::RuntimeFile],
    dir: &Path,
) -> Result<(), String> {
    for file in files {
        let bytes = read_entry(archive, file.inner)?;
        let dest = dir.join(file.dest);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let tmp = dest.with_extension("part");
        fs::write(&tmp, &bytes).map_err(|e| format!("写入 {} 失败：{e}", dest.display()))?;
        fs::rename(&tmp, &dest).map_err(|e| format!("落地 {} 失败：{e}", dest.display()))?;
    }
    Ok(())
}

fn read_entry(archive: &Path, inner: &str) -> Result<Vec<u8>, String> {
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.ends_with(".zip") || name.ends_with(".nupkg") {
        read_zip(archive, inner)
    } else if name.ends_with(".tgz") || name.ends_with(".tar.gz") {
        read_targz(archive, inner)
    } else if name.ends_with(".tar.xz") || name.ends_with(".txz") {
        read_tarxz(archive, inner)
    } else {
        Err(format!("不认识的归档格式：{name}"))
    }
}

fn read_zip(archive: &Path, inner: &str) -> Result<Vec<u8>, String> {
    let file = fs::File::open(archive).map_err(|e| format!("打开归档失败：{e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("读取 zip 失败：{e}"))?;
    let mut entry = zip
        .by_name(inner)
        .map_err(|_| format!("归档里没有 {inner}"))?;
    let mut out = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut out)
        .map_err(|e| format!("解压 {inner} 失败：{e}"))?;
    Ok(out)
}

fn read_targz(archive: &Path, inner: &str) -> Result<Vec<u8>, String> {
    let file = fs::File::open(archive).map_err(|e| format!("打开归档失败：{e}"))?;
    read_tar(flate2::read::GzDecoder::new(file), inner, archive)
}

/// `.tar.xz`：NVIDIA 的 Linux CUDA 组件只发这种格式。
fn read_tarxz(archive: &Path, inner: &str) -> Result<Vec<u8>, String> {
    let file = fs::File::open(archive).map_err(|e| format!("打开归档失败：{e}"))?;
    read_tar(xz2::read::XzDecoder::new(file), inner, archive)
}

/// 在 tar（任意解压器）里找一个条目并读出内容。
fn read_tar<R: Read>(reader: R, inner: &str, archive: &Path) -> Result<Vec<u8>, String> {
    let mut tar = tar::Archive::new(reader);
    let entries = tar.entries().map_err(|e| format!("读取 tar 失败：{e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("读取 tar 失败：{e}"))?;
        let raw = entry
            .path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        // 有的打包工具会写 `./` 前缀；清单里不写。
        let path = raw.trim_start_matches("./");
        if path == inner {
            let mut out = Vec::new();
            entry
                .read_to_end(&mut out)
                .map_err(|e| format!("解压 {} 失败：{e}", archive.display()))?;
            return Ok(out);
        }
    }
    Err(format!("{} 里没有 {inner}", archive.display()))
}

#[cfg(target_os = "windows")]
fn reveal(path: &Path) -> Result<(), String> {
    std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn reveal(path: &Path) -> Result<(), String> {
    std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn reveal(path: &Path) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CUDA 13 **本体**必须留在 `ort` 组；只有组件才分到 `cuda12` / `cuda13`。
    #[test]
    fn cuda_cores_stay_in_the_ort_group_while_components_split_by_flavor() {
        assert_eq!(group_of(RuntimeId::Cpu), "ort");
        assert_eq!(group_of(RuntimeId::Cuda), "ort");
        assert_eq!(group_of(RuntimeId::Cuda13), "ort");
        assert_eq!(group_of(RuntimeId::Cudart), "cuda12");
        assert_eq!(group_of(RuntimeId::Cudnn), "cuda12");
        assert_eq!(group_of(RuntimeId::Cudart13), "cuda13");
        assert_eq!(group_of(RuntimeId::Cudnn13), "cuda13");
    }

    /// 端到端验证 `.tar.xz` 解包（Linux 的 CUDA 组件只有这种格式）。
    ///
    /// 需要本机有那份归档，默认忽略：
    /// `AIDAW_TARXZ_TEST=D:\tmp\cudart.tar.xz cargo test --lib reads_linux_cuda_archive -- --ignored --nocapture`
    #[test]
    #[ignore = "needs a local NVIDIA .tar.xz"]
    fn reads_linux_cuda_archive() {
        let path = std::env::var("AIDAW_TARXZ_TEST").expect("设 AIDAW_TARXZ_TEST 指向 .tar.xz");
        let entry = std::env::var("AIDAW_TARXZ_ENTRY").unwrap_or_else(|_| {
            "cuda_cudart-linux-x86_64-12.8.90-archive/lib/libcudart.so.12.8.90".to_string()
        });
        let bytes = read_tarxz(Path::new(&path), &entry)
            .unwrap_or_else(|e| panic!("{path} / {entry}: {e}"));
        assert!(bytes.len() > 100_000, "解出来只有 {} 字节", bytes.len());
        println!("{entry}: {} 字节", bytes.len());
    }

    #[test]
    fn prefix_rewrite_supports_placeholder() {
        assert_eq!(apply_prefix("", "https://a/b"), "https://a/b");
        assert_eq!(
            apply_prefix("https://m/", "https://a/b"),
            "https://m/https://a/b"
        );
        assert_eq!(
            apply_prefix("https://m/?u={url}", "https://a/b"),
            "https://m/?u=https://a/b"
        );
    }

    #[test]
    fn candidates_put_the_preferred_region_first_and_keep_the_rest() {
        static SOURCES: &[RuntimeSource] = &[
            RuntimeSource {
                region: "intl",
                url: "https://intl.example/x.nupkg",
            },
            RuntimeSource {
                region: "cn",
                url: "https://cn.example/x.nupkg",
            },
        ];
        let list = candidates_for(SOURCES, &StoreNetwork::default(), Some("cn"));
        assert_eq!(list[0], "https://cn.example/x.nupkg");
        assert!(list.iter().any(|u| u == "https://intl.example/x.nupkg"));

        let list = candidates_for(SOURCES, &StoreNetwork::default(), Some("intl"));
        assert_eq!(list[0], "https://intl.example/x.nupkg");

        // 探不出区域（两边都不通 / 一边没有源）：保持清单原序，交给下载器逐个回退。
        let list = candidates_for(SOURCES, &StoreNetwork::default(), None);
        assert_eq!(list[0], "https://intl.example/x.nupkg");
    }

    /// 显式偏好不该联网：`auto` 之外的分支只看配置。
    #[test]
    fn explicit_preference_skips_probing() {
        static SOURCES: &[RuntimeSource] = &[
            RuntimeSource {
                region: "intl",
                url: "https://intl.invalid/x",
            },
            RuntimeSource {
                region: "cn",
                url: "https://cn.invalid/x",
            },
        ];
        let domestic = StoreNetwork {
            model_source_preference: ModelSourcePreference::Domestic,
            ..Default::default()
        };
        assert_eq!(preferred_region(SOURCES, &domestic, None), Some("cn"));

        let international = StoreNetwork {
            model_source_preference: ModelSourcePreference::International,
            ..Default::default()
        };
        assert_eq!(
            preferred_region(SOURCES, &international, None),
            Some("intl")
        );
    }

    /// 端到端验证「只取所需文件」：真的走一遍 HTTP Range + zip 目录 + 解压。
    ///
    /// 需要网络，默认忽略；手动跑：
    /// `cargo test -p ai-daw --lib pick -- --ignored --nocapture`
    #[test]
    #[ignore = "needs network"]
    fn pick_downloads_only_the_listed_files() {
        let asset = ai_daw_local_ai::backend::runtime::asset(RuntimeId::Cpu).expect("cpu asset");
        let part = &asset.parts[0];
        let dir = std::env::temp_dir().join("ai-daw-pick-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let candidates = vec![part.sources[0].url.to_string()];
        let cancel = AtomicBool::new(false);
        let mut received = 0u64;
        let mut progress = |r: u64, _t: u64| received = r;
        if let Err(e) = pick_install(&candidates, part, &dir, None, &cancel, &mut progress) {
            panic!("pick 失败：{e}");
        }

        let mut on_disk = 0u64;
        for file in part.files {
            let meta = fs::metadata(dir.join(file.dest))
                .unwrap_or_else(|_| panic!("{} 没落地", file.dest));
            on_disk += meta.len();
        }
        println!(
            "传输 {:.1} MiB / 落地 {:.1} MiB（整包 {:.1} MiB）",
            received as f64 / 1048576.0,
            on_disk as f64 / 1048576.0,
            part.bytes as f64 / 1048576.0
        );
        assert_eq!(on_disk, part.disk_bytes, "落地字节应与清单一致");
        // 只取两个 dll 的传输量应该是整包的零头。
        assert!(received < part.bytes / 5, "传输 {received} 太大了");
        let _ = fs::remove_dir_all(&dir);
    }
}
