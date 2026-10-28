//! **应用内模型下载器**：把 Stage G 的（大）模型权重下到应用配置目录的 `models/`。
//!
//! 复用插件管理器（[`crate::manager`]）的下载基建：多源回退、代理 / 镜像、`Range` 断点续传、
//! **sha256 硬门禁**。区别只在落点与事件：
//! - 权重落到应用配置目录 `models/`（宿主启动时把它登记进模型搜索路径）；
//! - 进度走 [`EVENT_MODEL`] = `engine://model`，载荷 [`ModelProgress`]。
//!
//! **大模型不随包**：Basic Pitch / LocalVQE 随安装包分发（`bundle.resources`），只有像
//! 音源分离（209 MB）这类才按需下载。下载完把 `.part`
//! 原子 rename 成正式文件——中途的半成品不会被 `models::find` 当成「已安装」。
//!
//! **一个条目是一组文件**：多数模型只有一个权重，但多组件管线是
//! 「一张 `.onnx` 图 + 一个 `.onnx.data` 外部权重」成对出现的，十来个文件缺一不可。
//! 这类条目带自己的落点子目录（[`ModelEntry::dir`]），下载器**串行**逐个下、逐个校验、
//! 逐个原子落地，进度按跨文件累计字节上报。整组齐备才算装好（`ort` 要靠同目录的相对路径
//! 找外部权重，图和权重分处两处就会加载失败）。
//!
//! 网络配置（代理 / 镜像 / 地址重写）**与插件共用同一份**（设置页「网络」一节，`store.json`）：
//! 下载失败或代理有问题时，界面统一提示去「网络」页配置。

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use ai_daw_local_ai::models::catalog::{self, ModelEntry, ModelFile, ModelSource};

use crate::manager::{
    download_candidates, download_verified, probe_agent, resolve_proxy, ModelSourcePreference,
    StoreNetwork, StoreRuntime,
};

/// 模型下载进度广播的前端事件名。
pub const EVENT_MODEL: &str = "engine://model";

/// 进度事件载荷。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelProgress {
    /// 模型 id。
    pub id: String,
    /// `downloading` / `done` / `error`。
    ///
    /// 校验是**流式**进行的（边下边算 sha256，见 [`crate::manager::download_verified`]），
    /// 不存在独立的「校验中」阶段；组条目也只报一根跨文件的累计进度条。
    pub state: String,
    /// 已接收字节（组条目为**跨文件累计值**）。
    pub received: u64,
    /// 总字节（未知为 0）。
    pub total: u64,
    /// 出错 / 取消时的一句话。
    pub message: Option<String>,
}

/// 下载运行时（一次只下一个）。
struct ModelRuntime {
    running: AtomicBool,
    cancel: AtomicBool,
}

fn runtime() -> &'static ModelRuntime {
    static RUNTIME: OnceLock<ModelRuntime> = OnceLock::new();
    RUNTIME.get_or_init(|| ModelRuntime {
        running: AtomicBool::new(false),
        cancel: AtomicBool::new(false),
    })
}

/// 权重落地目录（应用配置目录的 `models/`）。
pub fn models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("拿不到应用配置目录：{e}"))?
        .join("models");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// 某个条目的下载落点目录（**不创建子目录**；查询状态时用）。
fn entry_dir_path(app: &AppHandle, entry: &ModelEntry) -> Result<PathBuf, String> {
    let base = models_dir(app)?;
    Ok(match entry.dir {
        Some(sub) => base.join(sub),
        None => base,
    })
}

/// 下载落点目录（并确保存在；下载 / 落盘时用）。
fn entry_dir(app: &AppHandle, entry: &ModelEntry) -> Result<PathBuf, String> {
    let dir = entry_dir_path(app, entry)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// 组内单个文件的状态（界面用：一个 3 GB 的组光有整组百分比不够，
/// 还需要「下到第几个文件了」）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelFileStatus {
    /// 文件名。
    pub filename: String,
    /// 预估体积（字节）。
    pub approx_bytes: u64,
    /// 是否已在位。
    pub installed: bool,
    /// 已在位时的实际大小。
    pub bytes: Option<u64>,
}

/// 清单条目 + 本地状态（给界面显示）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelStatus {
    /// 稳定标识。
    pub id: String,
    /// 展示名。
    pub name: String,
    /// 所属功能。
    pub feature: String,
    /// SPDX 许可。
    pub license: String,
    /// 整组预估体积（字节）。
    pub approx_bytes: u64,
    /// 代表文件名（界面展示用；组条目取第一个）。
    pub filename: String,
    /// 是否随安装包分发。
    pub bundled: bool,
    /// 是否已就位（组条目要求**组内全部**文件在位）。
    pub installed: bool,
    /// 已就位时代表文件的大小。
    pub bytes: Option<u64>,
    /// 已就位时代表文件的路径。
    pub path: Option<String>,
    /// 是否有可下载的源。
    pub downloadable: bool,
    /// 组内文件总数。
    pub total_files: u32,
    /// 组内已就位的文件数。
    pub installed_files: u32,
    /// 组内每个文件的状态。
    pub files: Vec<ModelFileStatus>,
}

pub(crate) fn statuses(app: &AppHandle) -> Vec<ModelStatus> {
    catalog::all()
        .iter()
        .map(|entry| {
            // 落点目录里的实际字节数（`locate` 会因为「组不完整」而给不出路径，
            // 所以进度类的信息直接读落点目录）。
            let dir = entry_dir_path(app, entry).ok();
            let files: Vec<ModelFileStatus> = entry
                .files
                .iter()
                .map(|f| {
                    let bytes = dir
                        .as_ref()
                        .and_then(|d| fs::metadata(d.join(f.filename)).ok())
                        .map(|m| m.len());
                    ModelFileStatus {
                        filename: f.filename.to_string(),
                        approx_bytes: f.approx_bytes,
                        installed: catalog::file_present(entry, f),
                        bytes,
                    }
                })
                .collect();
            let path = catalog::installed_path(entry);
            ModelStatus {
                id: entry.id.to_string(),
                name: entry.name.to_string(),
                feature: entry.feature.to_string(),
                license: entry.license.to_string(),
                approx_bytes: entry.total_bytes(),
                filename: entry
                    .primary()
                    .map(|f| f.filename.to_string())
                    .unwrap_or_default(),
                bundled: entry.bundled,
                installed: catalog::installed(entry),
                bytes: path
                    .as_ref()
                    .and_then(|p| fs::metadata(p).ok())
                    .map(|m| m.len()),
                path: path.map(|p| p.to_string_lossy().into_owned()),
                downloadable: entry.files.iter().any(|f| !f.sources.is_empty()),
                total_files: entry.files.len() as u32,
                installed_files: files.iter().filter(|f| f.installed).count() as u32,
                files,
            }
        })
        .collect()
}

/// 列出全部模型与本地状态。
#[tauri::command]
pub fn model_catalog(app: AppHandle) -> Vec<ModelStatus> {
    statuses(&app)
}

/// 开始下载某个模型（后台线程；进度走 `engine://model`）。
#[tauri::command]
pub fn model_download(app: AppHandle, id: String) -> Result<(), String> {
    let entry = catalog::by_id(&id).ok_or_else(|| format!("未知模型：{id}"))?;
    if !entry.files.iter().any(|f| !f.sources.is_empty()) {
        return Err("这个模型没有可下载的源".into());
    }
    if runtime().running.swap(true, Ordering::SeqCst) {
        return Err("已有模型正在下载".into());
    }
    runtime().cancel.store(false, Ordering::Relaxed);

    let thread_app = app.clone();
    thread::Builder::new()
        .name("ai-daw-model".into())
        .spawn(move || run_download(&thread_app, entry))
        .map_err(|e| {
            runtime().running.store(false, Ordering::SeqCst);
            e.to_string()
        })?;
    Ok(())
}

/// 取消正在进行的下载（下载线程在下一个数据块处停下，并删掉半成品）。
#[tauri::command]
pub fn model_cancel() {
    runtime().cancel.store(true, Ordering::Relaxed);
}

/// 删除**已下载**的模型文件（只删下载目录里的，不动随包的资源文件）。
///
/// 组条目会把整组文件都删掉（含子目录），这样「重下」才有干净的状态。
#[tauri::command]
pub fn model_remove(app: AppHandle, id: String) -> Result<(), String> {
    let entry = catalog::by_id(&id).ok_or_else(|| format!("未知模型：{id}"))?;
    let dir = entry_dir_path(&app, entry)?;
    for file in entry.files {
        for name in [file.filename.to_string(), format!("{}.part", file.filename)] {
            let path = dir.join(name);
            if path.exists() {
                fs::remove_file(&path).map_err(|e| e.to_string())?;
            }
        }
    }
    // 组条目清空后顺手把空目录收掉，别在 `models/` 里留一个空壳。
    if entry.dir.is_some() {
        let _ = fs::remove_dir(&dir);
    }
    Ok(())
}

/// 在系统文件管理器里显示模型文件（找不到就直接打开下载目录）。
#[tauri::command]
pub fn model_reveal(app: AppHandle, id: String) -> Result<(), String> {
    let entry = catalog::by_id(&id).ok_or_else(|| format!("未知模型：{id}"))?;
    // 权重不一定躺在下载目录里：随包的在资源目录、开发时可能在仓库 `models/`、
    // 也可能被 `AIDAW_MODELS` 指到别处。`installed_path` 会在全部搜索路径里找，
    // 所以**已就位时一律以它为准**——按下载落点算会出现「路径不存在 → open -R 静默失败」。
    let target = match catalog::installed_path(entry) {
        Some(path) => path,
        // 没就位：退回落点目录（缺失就建出来，至少能看见「该往哪儿放」）。
        None => {
            let dir = entry_dir_path(&app, entry)?;
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            dir
        }
    };
    reveal_in_file_manager(&target)
}

/// 按当前网络配置解析出该模型**实际会先请求**的下载地址（「复制下载链接」用）。
///
/// 组条目逐个文件都有各自的地址，这里给的是**代表文件**（第一个）的地址。
/// `auto` 偏好会联网探测一次连通性，故放到 `spawn_blocking`，不阻塞界面。
#[tauri::command]
pub async fn model_resolve_url(app: AppHandle, id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let entry = catalog::by_id(&id).ok_or_else(|| format!("未知模型：{id}"))?;
        let file = entry
            .primary()
            .ok_or_else(|| "这个模型没有文件".to_string())?;
        let network = app.state::<StoreRuntime>().network();
        let region = resolve_region(entry, &network);
        candidates_for(entry, file, &network, region)
            .into_iter()
            .next()
            .ok_or_else(|| "这个模型没有可用的下载源".to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// 内部实现
// ---------------------------------------------------------------------------

/// 单个源地址套上镜像前缀（含 `{url}` 占位符就替换，否则前缀拼接）。
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

/// 探测一个地址的「首字节延迟」；连不上 / 超时 / 非 2xx 返回 `None`。
///
/// 用 `Range: bytes=0-0` 只取一个字节——既确认可达，又不真的下载权重。
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

/// `auto` 模式：分别探测清单里国内外各一个源，谁先通 / 更快就优先谁。
///
/// 只有一边有源、或两边都探不出结果时返回 `None`（此时按清单原序，交给下载器逐个回退）。
fn probe_region(file: &ModelFile, proxy: Option<&ureq::Proxy>) -> Option<&'static str> {
    let intl = file.sources.iter().find(|s| s.region == "intl");
    let cn = file.sources.iter().find(|s| s.region == "cn");
    let (Some(intl), Some(cn)) = (intl, cn) else {
        return None;
    };
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

/// 这个模型该优先用哪个区域的源：显式偏好直接给；`auto` 才联网探测连通性。
///
/// 区域连通性只跟网络有关、跟具体文件无关，所以用**代表文件**探一次即可
/// （组条目不必逐个文件探测）。
fn resolve_region(entry: &ModelEntry, network: &StoreNetwork) -> Option<&'static str> {
    // 环境变量整体覆盖候选源（测试 / 无头场景）：不参与区域排序。
    if std::env::var("AIDAW_MODEL_SOURCES").is_ok() {
        return None;
    }
    match network.model_source_preference {
        ModelSourcePreference::International => Some("intl"),
        ModelSourcePreference::Domestic => Some("cn"),
        ModelSourcePreference::Auto => {
            probe_region(entry.primary()?, resolve_proxy(network).proxy.as_ref())
        }
    }
}

/// 按区域偏好给某个文件的候选源排序：偏好区域在前，其余保持清单原序
/// （`sort_by_key` 是稳定排序）。
fn ordered_sources<'a>(file: &'a ModelFile, region: Option<&str>) -> Vec<&'a ModelSource> {
    let mut sources: Vec<&ModelSource> = file.sources.iter().collect();
    if let Some(pref) = region {
        sources.sort_by_key(|s| if s.region == pref { 0 } else { 1 });
    }
    sources
}

/// 某个文件的候选下载地址：偏好区域的源优先 → 用户镜像 → 插件代理前缀重写 → 官方源，逐个去重。
///
/// `AIDAW_MODEL_SOURCES`（逗号分隔）整体覆盖候选列表，供测试 / 无头场景使用。
/// 它是一份**与文件无关**的地址表，所以只对**单文件**条目生效——多文件组仍走清单里的源
/// （否则整组会拿同一份地址去下，必然错）。
fn candidates_for(
    entry: &ModelEntry,
    file: &ModelFile,
    network: &StoreNetwork,
    region: Option<&str>,
) -> Vec<String> {
    if entry.files.len() == 1 {
        if let Ok(list) = std::env::var("AIDAW_MODEL_SOURCES") {
            let urls: Vec<String> = list
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !urls.is_empty() {
                return urls;
            }
        }
    }
    let mut out: Vec<String> = Vec::new();
    let mut push = |url: String| {
        if !url.is_empty() && !out.iter().any(|existing| existing == &url) {
            out.push(url);
        }
    };
    for source in ordered_sources(file, region) {
        for mirror in &network.model_mirrors {
            push(apply_prefix(mirror, source.url));
        }
        for candidate in download_candidates(source.url, &network.download_prefix) {
            push(candidate);
        }
    }
    out
}

fn emit_model(
    app: &AppHandle,
    id: &str,
    state: &str,
    received: u64,
    total: u64,
    message: Option<String>,
) {
    let _ = app.emit(
        EVENT_MODEL,
        ModelProgress {
            id: id.to_string(),
            state: state.to_string(),
            received,
            total,
            message,
        },
    );
}

/// 下载线程主体：逐个文件下到 `*.part` → 校验 sha256（在 `download_verified` 内流式进行）
/// → 原子 `rename` 到正式文件。
///
/// 组条目**串行**下（同时拉几个 GB 只会互相抢带宽），并且跳过落点目录里已经有的文件——
/// 中途取消后重来不必从第一个文件重新下起。
fn run_download(app: &AppHandle, entry: &ModelEntry) {
    let dir = match entry_dir(app, entry) {
        Ok(dir) => dir,
        Err(e) => return finish(app, entry, Err(e)),
    };
    let network = app.state::<StoreRuntime>().network();
    let resolved = resolve_proxy(&network);
    let region = resolve_region(entry, &network);
    let group_total = entry.total_bytes();
    let cancel = &runtime().cancel;

    emit_model(app, entry.id, "downloading", 0, group_total, None);

    // 已完成文件的累计字节：界面只画一根条，进度是**跨文件的累计值**。
    let mut done_bytes: u64 = 0;
    for file in entry.files {
        let final_path = dir.join(file.filename);
        // 组内的文件名**可以带子目录**（例如 LM 规划器的分词器是 `tokenizer/tokenizer.json`，
        // 与上游仓库的目录结构一致）。`dir` 本身在上面建过了，但子目录得在这里补建，
        // 否则 `*.part` 根本落不下去。
        if let Some(parent) = final_path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                return finish(app, entry, Err(format!("建目录失败：{e}")));
            }
        }
        if final_path.is_file() {
            done_bytes += file.approx_bytes;
            emit_model(
                app,
                entry.id,
                "downloading",
                done_bytes.min(group_total),
                group_total,
                None,
            );
            continue;
        }
        if file.sources.is_empty() {
            return finish(app, entry, Err(format!("{} 没有可下载的源", file.filename)));
        }
        let candidates = candidates_for(entry, file, &network, region);
        if candidates.is_empty() {
            return finish(
                app,
                entry,
                Err(format!("{} 没有可用的下载源", file.filename)),
            );
        }
        eprintln!(
            "[model] 下载 {} / {}（源区域偏好 {:?}，首个源 {}）",
            entry.name, file.filename, region, candidates[0]
        );

        let base = done_bytes;
        let mut progress = |received: u64, _file_total: u64| {
            emit_model(
                app,
                entry.id,
                "downloading",
                (base + received).min(group_total),
                group_total,
                None,
            );
        };

        let part_path = dir.join(format!("{}.part", file.filename));
        let result = download_verified(
            &candidates,
            &part_path,
            file.sha256,
            file.approx_bytes,
            resolved.proxy.as_ref(),
            Some(cancel),
            &mut progress,
        );

        match result {
            Ok(()) => {
                if runtime().cancel.load(Ordering::Relaxed) {
                    let _ = fs::remove_file(&part_path);
                    return finish(app, entry, Err("已取消".into()));
                }
                match fs::rename(&part_path, &final_path) {
                    Ok(()) => {
                        done_bytes += file.approx_bytes;
                        emit_model(
                            app,
                            entry.id,
                            "downloading",
                            done_bytes.min(group_total),
                            group_total,
                            None,
                        );
                    }
                    Err(e) => {
                        let _ = fs::remove_file(&part_path);
                        return finish(app, entry, Err(format!("落地失败：{e}")));
                    }
                }
            }
            Err(e) => {
                // 半成品必须删掉，否则下一轮会被当成「断点」继续拼一个可能已经错的文件。
                let _ = fs::remove_file(&part_path);
                return finish(app, entry, Err(e));
            }
        }
    }

    finish(app, entry, Ok(()))
}

fn finish(app: &AppHandle, entry: &ModelEntry, result: Result<(), String>) {
    let total = entry.total_bytes();
    runtime().running.store(false, Ordering::SeqCst);
    match result {
        Ok(()) => emit_model(app, entry.id, "done", total, total, None),
        Err(e) => emit_model(app, entry.id, "error", 0, 0, Some(e)),
    }
}

/// 在系统文件管理器里定位一个文件（沿用它所在平台的原生命令）。
///
/// 命令是**等它跑完并检查退出码**的：`open` / `explorer` 找不到路径时只会在 stderr 里
/// 抱怨、退出码非 0，以前 `spawn()` 完就不管，界面就会「点了没反应」。
fn reveal_in_file_manager(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = std::process::Command::new("open");
        // `-R` 在访达里选中它（对目录 = 在其父目录里选中该目录）。
        c.arg("-R").arg(path);
        c
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = std::process::Command::new("explorer");
        c.arg(format!("/select,{}", path.display()));
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        // Linux：没有统一「选中文件」接口，直接打开所在目录。
        let dir = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
        let mut c = std::process::Command::new("xdg-open");
        c.arg(dir);
        c
    };
    let output = command
        .output()
        .map_err(|e| format!("启动文件管理器失败（{}）：{e}", path.display()))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if stderr.is_empty() {
        format!("文件管理器打不开：{}", path.display())
    } else {
        format!("文件管理器打不开：{stderr}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network_with(mirrors: &[&str], prefix: &str) -> crate::manager::StoreNetwork {
        crate::manager::StoreNetwork {
            model_mirrors: mirrors.iter().map(|s| s.to_string()).collect(),
            download_prefix: prefix.to_string(),
            ..Default::default()
        }
    }

    /// 取某条目代表文件（这些测试都是单文件模型）。
    fn file_of(entry: &'static ModelEntry) -> &'static ModelFile {
        entry.primary().expect("条目应有文件")
    }

    #[test]
    fn candidates_prefer_user_mirrors_then_official_sources() {
        let entry = catalog::by_id("htdemucs-ort-v1").unwrap();
        let network = network_with(&["https://mirror.example/"], "");
        let candidates = candidates_for(entry, file_of(entry), &network, None);
        // 每个源都会先给一个套了镜像前缀的候选（第一条即第一个源）。
        assert!(candidates[0].starts_with("https://mirror.example/http"));
        // 官方源仍然在候选里兜底（魔搭 + HF 都在清单里）。
        assert!(candidates.iter().any(|u| u
            == "https://modelscope.cn/models/ailm32442/htdemucs-ort/resolve/master/htdemucs.ort"));
        assert!(candidates
            .iter()
            .any(|u| u == "https://huggingface.co/gentij/htdemucs-ort/resolve/main/htdemucs.ort"));
    }

    #[test]
    fn candidate_prefix_supports_url_placeholder() {
        let entry = catalog::by_id("htdemucs-ort-v1").unwrap();
        let network = network_with(&["https://mirror.example/?u={url}"], "");
        let candidates = candidates_for(entry, file_of(entry), &network, None);
        assert!(candidates[0].starts_with("https://mirror.example/?u=http"));
    }

    #[test]
    fn candidate_list_is_deduplicated() {
        let entry = catalog::by_id("htdemucs-ort-v1").unwrap();
        let network = network_with(&["https://mirror.example/"], "");
        let candidates = candidates_for(entry, file_of(entry), &network, None);
        let mut seen = std::collections::HashSet::new();
        for url in &candidates {
            assert!(seen.insert(url.clone()), "候选地址重复：{url}");
        }
    }

    /// 区域偏好要把该区域的源排到最前，另一个区域仍留在候选里兜底。
    #[test]
    fn region_preference_orders_sources_and_keeps_the_fallback() {
        let entry = catalog::by_id("htdemucs-6s").unwrap();
        let file = file_of(entry);
        let network = network_with(&[], "");

        let cn_first = candidates_for(entry, file, &network, Some("cn"));
        assert!(
            cn_first[0].contains("modelscope.cn"),
            "国内偏好应优先魔搭：{}",
            cn_first[0]
        );

        let intl_first = candidates_for(entry, file, &network, Some("intl"));
        assert!(
            intl_first[0].contains("huggingface.co"),
            "国外偏好应优先 HF：{}",
            intl_first[0]
        );
        // 换源兜底：两个方向都必须同时含国内外源，否则单边网络就下不动。
        for list in [cn_first, intl_first] {
            assert!(list.iter().any(|u| u.contains("modelscope.cn")));
            assert!(list.iter().any(|u| u.contains("huggingface.co")));
        }
    }

    /// `auto` 靠探测排序；探测不出来时（这里两个 `None`）保持清单原序，不 panic。
    #[test]
    fn probe_failure_keeps_catalog_order() {
        let entry = catalog::by_id("htdemucs-6s").unwrap();
        let file = file_of(entry);
        let ordered = ordered_sources(file, None);
        assert_eq!(ordered[0].kind, file.sources[0].kind);
    }
}
