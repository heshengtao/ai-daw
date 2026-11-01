//! **audio.cpp 引擎的下载 / 安装 / 删除**——下载管理器「运行时」里的那一组「音频引擎」。
//!
//! 清单在 `ai_daw_local_ai::engine`（编译期内嵌：六份归档的地址 / 体积 / sha256，以及每份
//! 后端目录要落哪些文件），这里只做「落到磁盘」这件事：
//!
//! ```text
//! <应用配置目录>/runtime/audiocpp/
//!   cpu/      audiocpp_server.exe + model_specs/yue2.json + VC 运行库
//!   vulkan/   同上（Vulkan 后端静态编进 exe）
//!   cuda12/   同上 + ggml*.dll + cudart64_12 / cublas* / cufft
//!   cuda13/   同上（CUDA 13 那份）
//!   preferred 用户挑的那一份（不写则按显卡自动挑）
//! ```
//!
//! 每个后端是一份**自包含的便携包**（audio.cpp 官方 release），所以下载就是「整包 → 解出
//! 需要的文件 → 按发布页给的整包 sha256 校验」。不再按 Range 挑文件：CUDA 的运行时归档
//! 里四个 DLL 就是全部内容，整包下反而能校验归档哈希（挑文件只能信服务器）。
//!
//! **不随包**：安装包一个字节都不涨，全部按需下载——音频引擎是可选能力，不是既有功能的前置。

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use tar::Archive as TarArchive;
use tauri::{AppHandle, Emitter, Manager};

use ai_daw_local_ai::engine::{self, EngineArchive, EnginePack, EngineSource};

use crate::manager::{
    download_candidates, download_verified, resolve_proxy, ModelSourcePreference, StoreNetwork,
    StoreRuntime,
};

/// 引擎下载进度广播的前端事件名。
pub const EVENT_YUE: &str = "engine://yue";

/// 进度事件载荷。
#[derive(Debug, Clone, serde::Serialize)]
pub struct YueProgress {
    /// 引擎包 id（`audiocpp-cpu` / `audiocpp-vulkan` / …）。
    pub id: String,
    /// `downloading` / `done` / `error`。
    pub state: String,
    pub received: u64,
    pub total: u64,
    pub message: Option<String>,
}

/// 一个引擎后端的本地状态（界面用）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct YueStatus {
    pub id: String,
    pub label: String,
    pub note: String,
    /// 需要下载的字节（整包）。
    pub bytes: u64,
    /// 装好后的磁盘占用。
    pub disk_bytes: u64,
    /// 组内文件是否齐了。
    pub installed: bool,
    /// 是否是默认那一份（CPU 兜底）。
    pub core: bool,
    /// 引擎是否已经能跑（任意一份后端装好）。
    pub ready: bool,
    /// 用户挑的那一份（即 [`selected`] 的那个 id）。
    pub selected: bool,
    /// 是否处于「自动挑后端」状态（没手动选过；界面据此高亮「自动」）。
    pub automatic: bool,
    /// 本机会自动用哪个后端：`cuda13` / `cuda12` / `vulkan` / `cpu`。
    pub device: String,
    /// 这份后端自己的目录（给「显示位置」用）。
    pub dir: String,
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

/// 是否有 audio.cpp 引擎包正在下载（界面用来纠正卡住的状态）。
pub fn busy() -> bool {
    downloader().running.load(Ordering::SeqCst)
}

/// 引擎根目录（应用配置目录的 `runtime/audiocpp/`）。
pub fn root(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("拿不到应用配置目录：{e}"))?
        .join("runtime")
        .join("audiocpp");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// 启动时调用：把根目录交给清单，清掉没解完的缓存，退役旧版文件，并把上一版
/// `runtime/yue2/` 里已装好的后端搬过来。
pub fn init(app: &AppHandle) {
    if let Ok(dir) = root(app) {
        engine::set_root(dir.clone());
        if let Some(parent) = dir.parent() {
            migrate_old_root(&parent.join("yue2"), &dir);
        }
        let staging = engine::dir().join(".download");
        if staging.is_dir() {
            let _ = fs::remove_dir_all(&staging);
        }
        retire_legacy(&dir);
    }
}

/// 一个目录里有没有服务端（Windows `.exe` / macOS / Linux 无后缀）。
fn has_server(dir: &Path) -> bool {
    dir.join("audiocpp_server.exe").is_file() || dir.join("audiocpp_server").is_file()
}

/// 上一版把引擎根目录叫 `runtime/yue2`（因为当时只有 YuE2 一个模型）。现在引擎是通用的
/// `audio.cpp`，目录改名 `runtime/audiocpp`；这里把旧目录里**已经装好**的后端整体搬过来，
/// 避免用户重新下 1 GB 的 CUDA 包。
fn migrate_old_root(old: &Path, new: &Path) {
    if !old.is_dir() || old == new {
        return;
    }
    // `retire_legacy` 会删掉旧 id 的 `preferred`，所以先读。
    let preferred = fs::read_to_string(old.join("preferred")).ok();
    retire_legacy(old);

    for backend in ["cpu", "vulkan", "cuda12", "cuda13", "metal"] {
        let from = old.join(backend);
        let to = new.join(backend);
        if has_server(&from) && !to.exists() {
            let _ = fs::create_dir_all(new);
            if fs::rename(&from, &to).is_ok() {
                eprintln!("[audiocpp] 已迁移旧引擎目录：{backend}");
            }
        }
    }

    // 旧 id（`yue2-cpu` …）映射成新 id（`audiocpp-cpu` …）。
    if let Some(text) = preferred {
        let id = text.trim();
        if let Some(device) = id.strip_prefix("yue2-") {
            let mapped = format!("audiocpp-{device}");
            if engine::pack(&mapped).is_some() {
                let _ = fs::write(new.join("preferred"), mapped);
            }
        }
    }

    // 日志 / 配置都是运行期重新生成的，别留。
    for name in ["server.log", "server.json", "preferred"] {
        let _ = fs::remove_file(old.join(name));
    }
    let _ = fs::remove_dir(old);
}

/// 清掉 YuE2-Studio 时代的引擎文件（`yue-server.exe` / 根目录的 `ggml*.dll` / 旧
/// `cuda12|13/ggml-cuda.dll`）。
///
/// 新版是「每个后端一个自包含目录」（`cpu/` `vulkan/` `cuda12/` `cuda13/`），旧文件
/// 摊在根目录上，既占几百 MB 又会让用户以为「引擎还在」——实际新清单一个文件都匹配不上。
/// 只删我们自己的文件名，且**不碰**任何 `<backend>/audiocpp_server.exe` 所在的目录。
fn retire_legacy(dir: &Path) {
    let mut removed = 0usize;

    let fixed = [
        "yue-server.exe",
        "runtime.json",
        "cudart64_12.dll",
        "vcomp140.dll",
    ];
    for name in fixed {
        let path = dir.join(name);
        if path.is_file() && fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let legacy = (name.starts_with("ggml") && name.ends_with(".dll"))
                || (name.starts_with("msvcp140") && name.ends_with(".dll"))
                || (name.starts_with("vcruntime140") && name.ends_with(".dll"))
                || (name.starts_with("YuE2-Studio-") && name.ends_with(".zip"));
            if legacy && fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
    }

    // 旧版把 CUDA 后端放在 `cuda12/ggml-cuda.dll` 这种子目录里；新版同一目录会有
    // `audiocpp_server.exe`。没有服务端 = 旧目录，整个删掉。
    for backend in ["cuda12", "cuda13"] {
        let path = dir.join(backend);
        if path.is_dir() && !has_server(&path) {
            if fs::remove_dir_all(&path).is_ok() {
                removed += 1;
            }
        }
    }

    // 旧版 `preferred` 里存的是 `yue2-core` / `yue2-vulkan` 这类旧 id，留着只会让
    // `preferred()` 解析失败（它本来也会忽略），顺手清掉。
    let preferred = dir.join("preferred");
    if let Ok(text) = fs::read_to_string(&preferred) {
        if engine::pack(text.trim()).is_none() {
            let _ = fs::remove_file(&preferred);
        }
    }

    if removed > 0 {
        eprintln!("[yue] 已清理 {removed} 个旧版引擎文件");
    }
}

fn parse_id(id: &str) -> Result<&'static EnginePack, String> {
    engine::pack(id).ok_or_else(|| format!("未知引擎后端：{id}"))
}

/// 用户挑的那一份（没挑过 / 挑了但已删掉就是 `None`）。
fn preferred() -> Option<&'static EnginePack> {
    let text = fs::read_to_string(engine::dir().join("preferred")).ok()?;
    engine::pack(text.trim())
}

/// 本机会用哪个后端。
///
/// - macOS（Apple Silicon）：Metal（唯一的包）；
/// - Windows / Linux：N 卡优先 CUDA 13（驱动 ≥ 580）→ CUDA 12，其次 Vulkan，最后 CPU。
///   「装了但没有对应显卡 / 驱动不够」不算数（AMD 上装了 CUDA 也跑不起来）。
pub fn auto_device() -> &'static str {
    use ai_daw_local_ai::backend::detect;
    let nvidia = detect::detect_cached().gpu == Some(detect::GpuVendor::Nvidia);
    let has = |device: &str| {
        engine::for_device(device)
            .map(engine::installed)
            .unwrap_or(false)
    };
    if std::env::consts::OS == "macos" {
        return if has("metal") { "metal" } else { "cpu" };
    }
    if nvidia {
        if has("cuda13") && cuda13_driver_ok() {
            "cuda13"
        } else if has("cuda12") {
            "cuda12"
        } else if has("vulkan") {
            "vulkan"
        } else {
            "cpu"
        }
    } else if has("vulkan") {
        "vulkan"
    } else {
        "cpu"
    }
}

/// CUDA 13 的驱动门槛：ORT 1.31 / CUDA 13 要 580+；读不到驱动版本时按不满足处理。
fn cuda13_driver_ok() -> bool {
    use ai_daw_local_ai::backend::detect;
    if detect::detect_cached().gpu != Some(detect::GpuVendor::Nvidia) {
        return false;
    }
    detect::detect_cached()
        .driver
        .as_deref()
        .and_then(|text| text.split('.').next())
        .and_then(|major| major.parse::<u32>().ok())
        .map(|major| major >= 580)
        .unwrap_or(false)
}

/// 实际会用哪个：用户挑的（若装了且能用）优先，否则自动。
pub(crate) fn effective_device() -> &'static str {
    match preferred() {
        Some(pack) if engine::installed(pack) && usable(pack.device) => pack.device,
        _ => auto_device(),
    }
}

/// 实际会用的那一份后端（给 `yue_engine` 拿目录用）。
pub(crate) fn effective_pack() -> Option<&'static EnginePack> {
    engine::for_device(effective_device())
}

/// 这份后端在本机能不能真跑（CUDA 需要 N 卡；CUDA 13 还要 580+ 驱动；Metal 要 macOS）。
fn usable(device: &str) -> bool {
    use ai_daw_local_ai::backend::detect;
    match device {
        "metal" => std::env::consts::OS == "macos",
        "cuda13" => cuda13_driver_ok(),
        d if d.starts_with("cuda") => {
            detect::detect_cached().gpu == Some(detect::GpuVendor::Nvidia)
        }
        _ => true,
    }
}

/// 全部引擎后端的状态（下载管理器用）。
#[tauri::command]
pub fn ai_yue_catalog(app: AppHandle) -> Vec<YueStatus> {
    statuses(&app)
}

/// 全部引擎后端的状态（按本机平台过滤）。
pub fn statuses(app: &AppHandle) -> Vec<YueStatus> {
    let _ = root(app);
    let chosen = effective_device();
    let picked = preferred().map(|p| p.id);
    engine::packs(std::env::consts::OS, std::env::consts::ARCH)
        .iter()
        .map(|pack| YueStatus {
            id: pack.id.to_string(),
            label: pack.label.to_string(),
            note: pack.note.to_string(),
            bytes: pack.bytes(),
            disk_bytes: pack.disk_bytes(),
            installed: engine::installed(pack),
            core: pack.core,
            ready: engine::ready(),
            selected: picked == Some(pack.id) || (picked.is_none() && pack.device == chosen),
            automatic: picked.is_none(),
            device: chosen.to_string(),
            dir: engine::dir_of(pack).to_string_lossy().into_owned(),
        })
        .collect()
}

/// 开始下载并安装一个引擎后端（后台线程；进度走 `engine://yue`）。
#[tauri::command]
pub fn ai_yue_install(app: AppHandle, id: String) -> Result<(), String> {
    let pack = parse_id(&id)?;
    if downloader().running.swap(true, Ordering::SeqCst) {
        return Err("已有引擎正在下载".into());
    }
    // 覆盖正在运行的服务端 / DLL 会失败（Windows 共享冲突，Unix 也不安全）。
    crate::yue_engine::stop_for_install(&app);
    downloader().cancel.store(false, Ordering::Relaxed);
    let thread_app = app.clone();
    thread::Builder::new()
        .name("ai-daw-yue".into())
        .spawn(move || run_install(&thread_app, pack))
        .map_err(|e| {
            downloader().running.store(false, Ordering::SeqCst);
            e.to_string()
        })?;
    Ok(())
}

/// 从本地目录安装一个引擎后端（目录里放清单要求的 1–2 个归档）。
///
/// 这是给「GitHub / 魔搭都连不上」或「已经手动下好包」的用户的兜底：不联网，
/// 但仍逐包校验 sha256 / 体积，和下载路径一样严格。
#[tauri::command]
pub fn ai_yue_import(app: AppHandle, id: String, dir: String) -> Result<(), String> {
    let pack = parse_id(&id)?;
    if downloader().running.load(Ordering::SeqCst) {
        return Err("已有引擎正在下载，先等它结束或取消".into());
    }
    crate::yue_engine::stop_for_install(&app);
    let source = PathBuf::from(dir);
    let mut archives = Vec::new();
    for part in pack.parts {
        let meta = engine::archive(part.archive)
            .ok_or_else(|| format!("清单里没有归档 {}", part.archive))?;
        let path = source.join(meta.file);
        if !path.is_file() {
            return Err(format!(
                "{} 里没有 {}（需要清单要求的同名归档）",
                source.display(),
                meta.file
            ));
        }
        verify_archive(&path, meta)?;
        archives.push((path, part));
    }

    let target = engine::dir_of(pack);
    fs::create_dir_all(&target).map_err(|e| format!("建目录失败：{e}"))?;
    for (path, part) in archives {
        extract(&path, part, &target)?;
    }
    emit(&app, pack.id, "done", pack.bytes(), pack.bytes(), None);
    Ok(())
}

fn verify_archive(path: &Path, meta: &ai_daw_local_ai::engine::EngineArchive) -> Result<(), String> {
    let size = fs::metadata(path)
        .map_err(|e| format!("读 {} 失败：{e}", path.display()))?
        .len();
    if size != meta.bytes {
        return Err(format!(
            "{} 体积不对：应为 {} 字节，实际 {size}",
            meta.file, meta.bytes
        ));
    }
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = format!("{:x}", hasher.finalize());
    if !digest.eq_ignore_ascii_case(meta.sha256) {
        return Err(format!(
            "{} sha256 不匹配：清单 {}，实际 {digest}",
            meta.file, meta.sha256
        ));
    }
    Ok(())
}

/// 取消下载（线程在下一个数据块处停下，半截文件不会落地）。
#[tauri::command]
pub fn ai_yue_cancel() {
    downloader().cancel.store(true, Ordering::Relaxed);
}

/// 删除一个已下载的后端（只删清单列出的文件）。
#[tauri::command]
pub fn ai_yue_remove(app: AppHandle, id: String) -> Result<(), String> {
    let pack = parse_id(&id)?;
    crate::yue_engine::stop_for_install(&app);
    engine::remove(pack)?;
    engine::prune_empty();
    // 删掉的正好是用户挑的那一份：把选择清掉，别让它继续指着一个不存在的包。
    if preferred().map(|p| p.id) == Some(pack.id) {
        let _ = fs::remove_file(engine::dir().join("preferred"));
    }
    Ok(())
}

/// 在系统文件管理器里显示引擎目录。
#[tauri::command]
pub fn ai_yue_reveal(id: String) -> Result<(), String> {
    let pack = parse_id(&id)?;
    let dir = engine::dir_of(pack);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    reveal(&dir)
}

/// 记下用户挑的后端（`None` = 恢复「按显卡自动」）。
#[tauri::command]
pub fn ai_yue_select(id: Option<String>) -> Result<(), String> {
    let path = engine::dir().join("preferred");
    match id {
        Some(id) => {
            let pack = parse_id(&id)?;
            if !engine::installed(pack) {
                return Err(format!("{} 还没下载", pack.label));
            }
            if !usable(pack.device) {
                return Err(format!(
                    "{} 在本机不可用（需要对应的显卡 / 驱动）",
                    pack.label
                ));
            }
            fs::write(&path, pack.id).map_err(|e| e.to_string())?;
        }
        None => {
            if path.exists() {
                fs::remove_file(&path).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 下载 + 解包
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
        EVENT_YUE,
        YueProgress {
            id: id.to_string(),
            state: state.to_string(),
            received,
            total,
            message: message.clone(),
        },
    );
    // 界面把引擎后端和 ONNX Runtime 放在同一张列表里，所以同一份进度也发到运行时那条
    // 事件上——下载管理器只订阅 `engine://runtime` 就能同时显示两者。
    let _ = app.emit(
        crate::runtimes::EVENT_RUNTIME,
        crate::runtimes::RuntimeProgress {
            id: id.to_string(),
            state: state.to_string(),
            received,
            total,
            message,
        },
    );
}

fn run_install(app: &AppHandle, pack: &'static EnginePack) {
    let result = crate::job::guard(|| install_now(app, pack).map(|()| String::new()));
    downloader().running.store(false, Ordering::SeqCst);
    match result {
        Ok(_) => emit(app, pack.id, "done", pack.bytes(), pack.bytes(), None),
        Err(message) => emit(app, pack.id, "error", 0, 0, Some(message)),
    }
}

fn install_now(app: &AppHandle, pack: &'static EnginePack) -> Result<(), String> {
    let dir = engine::dir_of(pack);
    fs::create_dir_all(&dir).map_err(|e| format!("建目录失败：{e}"))?;
    let staging = engine::dir().join(".download");
    fs::create_dir_all(&staging).map_err(|e| format!("建缓存目录失败：{e}"))?;

    let network = app.state::<StoreRuntime>().network();
    let proxy = resolve_proxy(&network).proxy;
    // 区域连通性只跟网络有关、跟具体归档无关：一次安装只探一次，几个组件共用同一个结论。
    let region = resolve_region(pack, &network);
    let cancel = &downloader().cancel;
    let total = pack.bytes();
    let mut done = 0u64;

    emit(app, pack.id, "downloading", 0, total, None);

    for part in pack.parts {
        let archive = engine::archive(part.archive)
            .ok_or_else(|| format!("清单里没有归档 {}", part.archive))?;
        if cancel.load(Ordering::Relaxed) {
            return Err("已取消".into());
        }
        let candidates = candidates(archive, &network, region);
        if candidates.is_empty() {
            return Err(format!("{} 没有可用的下载源", archive.file));
        }

        let path = staging.join(archive.file);
        let base = done;
        let mut progress = |received: u64, _total: u64| {
            emit(
                app,
                pack.id,
                "downloading",
                (base + received).min(total),
                total,
                None,
            );
        };
        eprintln!(
            "[yue] 下载 {}（{} MB，首个源 {}）",
            archive.file,
            archive.bytes / (1024 * 1024),
            candidates[0]
        );
        download_verified(
            &candidates,
            &path,
            archive.sha256,
            archive.bytes,
            proxy.as_ref(),
            Some(cancel),
            &mut progress,
        )?;
        if cancel.load(Ordering::Relaxed) {
            let _ = fs::remove_file(&path);
            return Err("已取消".into());
        }

        extract(&path, part, &dir)?;
        let _ = fs::remove_file(&path);
        done += archive.bytes;
        emit(app, pack.id, "downloading", done.min(total), total, None);
    }

    let _ = fs::remove_dir(&staging);
    Ok(())
}

/// 这一次安装优先用哪个区域的源。
///
/// 显式偏好直接用；`auto` 只探一次（几个归档共用同一个网络结论）：国内 / 国际各探一个
/// 候选，谁先通 / 更快就排前面，失败仍回退另一个区域——和模型下载器同一套。
fn resolve_region(pack: &EnginePack, network: &StoreNetwork) -> Option<&'static str> {
    match network.model_source_preference {
        ModelSourcePreference::International => Some("intl"),
        ModelSourcePreference::Domestic => Some("cn"),
        ModelSourcePreference::Auto => {
            let proxy = resolve_proxy(network).proxy;
            pack.parts
                .iter()
                .filter_map(|part| engine::archive(part.archive))
                .find_map(|archive| probe_region(archive, proxy.as_ref()))
        }
    }
}

/// `auto` 模式：分别探一下这份归档的国内外源。只有一边有源、或两边都探不出结果时
/// 返回 `None`（此时保持清单原序，交给下载器逐个回退）。
fn probe_region(archive: &EngineArchive, proxy: Option<&ureq::Proxy>) -> Option<&'static str> {
    let intl = archive.sources.iter().find(|s| s.region == "intl")?;
    let cn = archive.sources.iter().find(|s| s.region == "cn")?;
    let agent = crate::manager::probe_agent(proxy);
    let intl_ms = probe_latency(&agent, intl.url);
    let cn_ms = probe_latency(&agent, cn.url);
    match (intl_ms, cn_ms) {
        (Some(a), Some(b)) => Some(if a <= b { "intl" } else { "cn" }),
        (Some(_), None) => Some("intl"),
        (None, Some(_)) => Some("cn"),
        (None, None) => None,
    }
}

fn probe_latency(agent: &ureq::Agent, url: &str) -> Option<Duration> {
    let started = Instant::now();
    let resp = agent.get(url).header("Range", "bytes=0-0").call().ok()?;
    let status = resp.status().as_u16();
    (status == 200 || status == 206).then(|| started.elapsed())
}

/// 候选地址：区域偏好排序 + 用户镜像 + 代理前缀（与 `runtimes.rs` 同一套）。
fn candidates(archive: &EngineArchive, network: &StoreNetwork, region: Option<&str>) -> Vec<String> {
    let mut ordered: Vec<&EngineSource> = archive.sources.iter().collect();
    if let Some(pref) = region {
        ordered.sort_by_key(|s| if s.region == pref { 0 } else { 1 });
    }
    let mut out: Vec<String> = Vec::new();
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

/// 从整包里解出这一组文件（只解清单里的，别把 `tools/` 全铺开）。
fn extract(
    archive: &Path,
    part: &'static ai_daw_local_ai::engine::EnginePart,
    dir: &Path,
) -> Result<(), String> {
    let meta = engine::archive(part.archive)
        .ok_or_else(|| format!("清单里没有归档 {}", part.archive))?;
    match meta.format {
        engine::EngineFormat::Zip => extract_zip(archive, part, dir),
        engine::EngineFormat::TarGz => extract_tar_gz(archive, part, dir),
    }
}

fn extract_zip(
    archive: &Path,
    part: &'static ai_daw_local_ai::engine::EnginePart,
    dir: &Path,
) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| format!("打开归档失败：{e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("读取 zip 失败：{e}"))?;
    for entry in part.files {
        let mut member = zip
            .by_name(entry.inner)
            .map_err(|_| format!("{} 里没有 {}", part.archive, entry.inner))?;
        let dest = dir.join(entry.dest);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        // 流式解压：`ggml-cuda.dll` 解开有 1.1 GB，整块读进内存太浪费。
        let tmp = dest.with_extension("part");
        let mut out =
            fs::File::create(&tmp).map_err(|e| format!("写入 {} 失败：{e}", dest.display()))?;
        std::io::copy(&mut member, &mut out)
            .map_err(|e| format!("解压 {} 失败：{e}", entry.inner))?;
        out.flush().map_err(|e| e.to_string())?;
        drop(out);
        fs::rename(&tmp, &dest).map_err(|e| format!("落地 {} 失败：{e}", dest.display()))?;
    }
    Ok(())
}

/// `tar.gz`：按条目名挑文件，`entry.unpack()` 会把 Unix 可执行位一起带出来
/// （`audiocpp_server` 必须是 0755）。
fn extract_tar_gz(
    archive: &Path,
    part: &'static ai_daw_local_ai::engine::EnginePart,
    dir: &Path,
) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| format!("打开归档失败：{e}"))?;
    let mut tar = TarArchive::new(GzDecoder::new(file));
    let mut remaining: Vec<&'static ai_daw_local_ai::engine::EngineFile> =
        part.files.iter().collect();

    for entry in tar
        .entries()
        .map_err(|e| format!("读取 tar 失败：{e}"))?
    {
        let mut entry = entry.map_err(|e| format!("读取 tar 条目失败：{e}"))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry
            .path()
            .map_err(|e| format!("tar 条目路径坏掉：{e}"))?;
        let name = path.to_string_lossy().trim_start_matches("./").to_string();
        let Some(pos) = remaining
            .iter()
            .position(|file| file.inner == name.as_str())
        else {
            continue;
        };
        let target = remaining.swap_remove(pos);
        let dest = dir.join(target.dest);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        entry
            .unpack(&dest)
            .map_err(|e| format!("解压 {} 失败：{e}", target.inner))?;
    }

    if let Some(missing) = remaining.first() {
        return Err(format!("{} 里没有 {}", part.archive, missing.inner));
    }
    Ok(())
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

    #[test]
    fn prefix_rewrite_is_shared_with_the_ort_path() {
        assert_eq!(apply_prefix("", "https://a/b"), "https://a/b");
        assert_eq!(
            apply_prefix("https://m/", "https://a/b"),
            "https://m/https://a/b"
        );
    }

    /// 区域偏好要同时改变顺序**并保留另一边**（国内失败还能回退 GitHub，反之亦然）。
    #[test]
    fn candidate_order_follows_the_region() {
        let archive = engine::archive("audio-v0.9.3-bin-windows-x64-cpu-portable.zip").unwrap();
        let network = StoreNetwork::default();
        let intl = candidates(archive, &network, Some("intl"));
        let cn = candidates(archive, &network, Some("cn"));
        assert!(intl[0].starts_with("https://github.com/"), "{intl:?}");
        assert!(cn[0].starts_with("https://modelscope.cn/"), "{cn:?}");
        assert_eq!(intl.len(), cn.len());
        assert!(intl.iter().any(|url| url.contains("modelscope.cn")));
        assert!(cn.iter().any(|url| url.contains("github.com")));
    }

    #[test]
    fn devices_match_pack_ids() {
        for (device, id) in [
            ("cpu", "audiocpp-cpu"),
            ("vulkan", "audiocpp-vulkan"),
            ("cuda12", "audiocpp-cuda12"),
            ("cuda13", "audiocpp-cuda13"),
        ] {
            assert_eq!(
                engine::packs("windows", "x86_64")
                    .iter()
                    .find(|pack| pack.device == device)
                    .map(|pack| pack.id),
                Some(id)
            );
        }
        assert!(engine::packs("windows", "x86_64")
            .iter()
            .all(|pack| pack.device != "metal"));

        assert_eq!(
            engine::packs("macos", "aarch64")
                .iter()
                .find(|pack| pack.device == "metal")
                .map(|pack| pack.id),
            Some("audiocpp-metal")
        );
        for id in ["audiocpp-cpu", "audiocpp-vulkan", "audiocpp-cuda12"] {
            assert!(
                engine::packs("linux", "x86_64")
                    .iter()
                    .any(|pack| pack.id == id),
                "Linux 缺 {id}"
            );
        }
        assert!(engine::packs("linux", "x86_64")
            .iter()
            .all(|pack| pack.device != "cuda13"));
    }

    /// 端到端验证「整包下载 + 只解所需」：真走一遍 Range 断点、sha256、解压。
    ///
    /// 需要网络，默认忽略；`AIDAW_YUE_TEST` 选组（逗号分隔，默认 `audiocpp-cpu`）：
    /// `AIDAW_YUE_TEST=audiocpp-cpu,audiocpp-vulkan cargo test --lib yue -- --ignored --nocapture`
    #[test]
    #[ignore = "needs network"]
    fn installs_engine_packs_from_the_release_archives() {
        let wanted = std::env::var("AIDAW_YUE_TEST").unwrap_or_else(|_| "audiocpp-cpu".into());
        // 这台机器直连 GitHub 不通：手动跑时用 `AIDAW_PROXY=http://127.0.0.1:10809`。
        let proxy = std::env::var("AIDAW_PROXY")
            .ok()
            .and_then(|url| ureq::Proxy::new(&url).ok());
        for id in wanted.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let pack = engine::pack(id).unwrap_or_else(|| panic!("未知引擎包：{id}"));
            let root = std::env::temp_dir().join(format!("ai-daw-yue-install-{id}"));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();

            for part in pack.parts {
                let archive = engine::archive(part.archive).unwrap();
                let path = root.join(archive.file);
                crate::manager::download_verified(
                    &[archive.sources[0].url.to_string()],
                    &path,
                    archive.sha256,
                    archive.bytes,
                    proxy.as_ref(),
                    Some(&AtomicBool::new(false)),
                    &mut |_, _| {},
                )
                .unwrap_or_else(|e| panic!("{id} 下载失败：{e}"));

                let dir = root.join("out");
                fs::create_dir_all(&dir).unwrap();
                extract(&path, part, &dir).unwrap();
                let _ = fs::remove_file(&path);
            }

            let out = root.join("out");
            let missing = pack.missing_in(&out);
            assert!(missing.is_empty(), "{id} 解压后还缺：{missing:?}");
            let on_disk: u64 = pack
                .files()
                .iter()
                .map(|f| fs::metadata(out.join(f.dest)).unwrap().len())
                .sum();
            println!("{id}: 落地 {:.1} MiB", on_disk as f64 / 1048576.0);
            let _ = fs::remove_dir_all(&root);
        }
    }

    /// 真实 tar.gz 的目录结构 / 文件清单核对（本机 `.tmp` 里有官方包时跑）：
    /// `AIDAW_YUE_ARCHIVES=... cargo test --lib yue extracts_macos -- --ignored --nocapture`
    #[test]
    #[ignore = "needs local archives"]
    fn extracts_macos_and_linux_tar_packs() {
        let base = std::env::var("AIDAW_YUE_ARCHIVES")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(r"D:\ai-daw\.tmp\audiocpp-v093"));
        for (os, arch, id) in [
            ("macos", "aarch64", "audiocpp-metal"),
            ("linux", "x86_64", "audiocpp-cpu"),
            ("linux", "x86_64", "audiocpp-vulkan"),
        ] {
            let pack = engine::packs(os, arch)
                .iter()
                .find(|pack| pack.id == id)
                .unwrap_or_else(|| panic!("清单里没有 {id}"));
            let out = std::env::temp_dir().join(format!("ai-daw-yue-tar-{id}"));
            let _ = fs::remove_dir_all(&out);
            fs::create_dir_all(&out).unwrap();
            for part in pack.parts {
                let path = base.join(part.archive);
                assert!(path.is_file(), "缺归档 {}", path.display());
                verify_archive(&path, engine::archive(part.archive).unwrap()).unwrap();
                extract(&path, part, &out).unwrap();
            }
            let missing = pack.missing_in(&out);
            assert!(missing.is_empty(), "{id} 解压后还缺：{missing:?}");
            let _ = fs::remove_dir_all(&out);
        }
    }

    /// 解压要能处理 CUDA 包里 1.1 GB 的 `ggml-cuda.dll`（流式写盘，不整块进内存）。
    #[test]
    fn extract_is_streaming() {
        // 结构检查：`std::io::copy` 写到文件，而不是把整条 entry 读进内存再写。
        let source = include_str!("yue.rs");
        assert!(source.contains("std::io::copy(&mut member, &mut out)"));
    }

    /// 旧版引擎文件要退役，但新版后端目录一个都不能动。
    #[test]
    fn legacy_engine_files_are_retired_without_touching_new_packs() {
        let dir = std::env::temp_dir().join("ai-daw-yue-legacy-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("cuda12")).unwrap();
        fs::create_dir_all(dir.join("cuda13")).unwrap();
        fs::create_dir_all(dir.join("cpu")).unwrap();

        // 旧版摊在根目录上的东西。
        for name in [
            "yue-server.exe",
            "runtime.json",
            "ggml-vulkan.dll",
            "ggml.dll",
            "cudart64_12.dll",
            "vcomp140.dll",
        ] {
            fs::write(dir.join(name), b"old").unwrap();
        }
        // 旧版的后端子目录（只有 dll，没有服务端）。
        fs::write(dir.join("cuda12").join("ggml-cuda.dll"), b"old").unwrap();
        // 新版目录（有服务端）。
        fs::write(dir.join("cuda13").join("audiocpp_server.exe"), b"new").unwrap();
        fs::write(dir.join("cpu").join("audiocpp_server.exe"), b"new").unwrap();
        // 旧 preferred 指向已不存在的 id。
        fs::write(dir.join("preferred"), b"yue2-core").unwrap();

        retire_legacy(&dir);

        for name in [
            "yue-server.exe",
            "runtime.json",
            "ggml-vulkan.dll",
            "ggml.dll",
        ] {
            assert!(!dir.join(name).exists(), "{name} 应该被删掉");
        }
        assert!(!dir.join("cuda12").exists(), "旧 cuda12 子目录应该被删掉");
        assert!(
            dir.join("cuda13").join("audiocpp_server.exe").is_file(),
            "新版 cuda13 不能动"
        );
        assert!(
            dir.join("cpu").join("audiocpp_server.exe").is_file(),
            "新版 cpu 不能动"
        );
        assert!(!dir.join("preferred").exists(), "旧 preferred 应该被清掉");
        let _ = fs::remove_dir_all(&dir);
    }
}
