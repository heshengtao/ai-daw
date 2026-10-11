//! **应用内模型下载器**：把 Stage G 的（大）模型权重下到应用配置目录的 `models/`。
//!
//! 复用插件管理器（[`crate::manager`]）的下载基建：多源回退、代理 / 镜像、`Range` 断点续传、
//! **sha256 硬门禁**。区别只在落点与事件：
//! - 权重落到应用配置目录 `models/`（宿主启动时把它登记进模型搜索路径）；
//! - 进度走 [`EVENT_MODEL`] = `engine://model`，载荷 [`ModelProgress`]。
//!
//! **大模型不随包**：Basic Pitch / LocalVQE 随安装包分发（`bundle.resources`），只有像
//! 音源分离（209 MB）这种才按需下载。下载完把 `.part` 原子 rename 成正式文件——中途的半成品
//! 不会被 `models::find` 当成「已安装」。
//!
//! 网络配置（代理 / 镜像 / 地址重写）**与插件共用同一份**（设置页「网络」一节，`store.json`）：
//! 下载失败或代理有问题时，界面统一提示去「网络」页配置。

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::thread;

use tauri::{AppHandle, Emitter, Manager};

use ai_daw_local_ai::models::catalog::{self, ModelEntry};

use crate::manager::{download_candidates, download_verified, resolve_proxy, StoreRuntime};

/// 模型下载进度广播的前端事件名。
pub const EVENT_MODEL: &str = "engine://model";

/// 进度事件载荷。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelProgress {
    /// 模型 id。
    pub id: String,
    /// `downloading` / `verifying` / `done` / `error`。
    pub state: String,
    /// 已接收字节。
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
    /// 预估体积（字节）。
    pub approx_bytes: u64,
    /// 落地文件名。
    pub filename: String,
    /// 是否随安装包分发。
    pub bundled: bool,
    /// 是否已就位。
    pub installed: bool,
    /// 已就位时文件大小。
    pub bytes: Option<u64>,
    /// 已就位时文件路径。
    pub path: Option<String>,
    /// 是否有可下载的源。
    pub downloadable: bool,
}

fn statuses() -> Vec<ModelStatus> {
    catalog::all()
        .iter()
        .map(|entry| {
            let path = catalog::installed_path(entry);
            let bytes = path
                .as_ref()
                .and_then(|p| fs::metadata(p).ok())
                .map(|m| m.len());
            ModelStatus {
                id: entry.id.to_string(),
                name: entry.name.to_string(),
                feature: entry.feature.to_string(),
                license: entry.license.to_string(),
                approx_bytes: entry.approx_bytes,
                filename: entry.filename.to_string(),
                bundled: entry.bundled,
                installed: path.is_some(),
                bytes,
                path: path.map(|p| p.to_string_lossy().into_owned()),
                downloadable: !entry.sources.is_empty(),
            }
        })
        .collect()
}

/// 列出全部模型与本地状态。
#[tauri::command]
pub fn model_catalog() -> Vec<ModelStatus> {
    statuses()
}

/// 开始下载某个模型（后台线程；进度走 `engine://model`）。
#[tauri::command]
pub fn model_download(app: AppHandle, id: String) -> Result<(), String> {
    let entry = catalog::by_id(&id).ok_or_else(|| format!("未知模型：{id}"))?;
    if entry.sources.is_empty() {
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
#[tauri::command]
pub fn model_remove(app: AppHandle, id: String) -> Result<(), String> {
    let entry = catalog::by_id(&id).ok_or_else(|| format!("未知模型：{id}"))?;
    let dir = models_dir(&app)?;
    for name in [entry.filename.to_string(), format!("{}.part", entry.filename)] {
        let path = dir.join(name);
        if path.exists() {
            fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// 在系统文件管理器里显示模型文件（找不到就直接打开下载目录）。
#[tauri::command]
pub fn model_reveal(app: AppHandle, id: String) -> Result<(), String> {
    let entry = catalog::by_id(&id).ok_or_else(|| format!("未知模型：{id}"))?;
    let target = catalog::installed_path(entry).unwrap_or_else(|| models_dir(&app).unwrap_or_default());
    reveal_in_file_manager(&target)
}

/// 按当前网络配置解析出该模型**实际会先请求**的下载地址（「复制下载链接」用）。
#[tauri::command]
pub fn model_resolve_url(app: AppHandle, id: String) -> Result<String, String> {
    let entry = catalog::by_id(&id).ok_or_else(|| format!("未知模型：{id}"))?;
    let network = app.state::<StoreRuntime>().network();
    candidates_for(entry, &network)
        .into_iter()
        .next()
        .ok_or_else(|| "这个模型没有可用的下载源".to_string())
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

/// 候选下载地址：用户镜像 → 插件代理前缀重写 → 官方源，逐个去重。
///
/// `AIDAW_MODEL_SOURCES`（逗号分隔）整体覆盖，供测试 / 无头场景使用。
fn candidates_for(entry: &ModelEntry, network: &crate::manager::StoreNetwork) -> Vec<String> {
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
    let mut out: Vec<String> = Vec::new();
    let mut push = |url: String| {
        if !url.is_empty() && !out.iter().any(|existing| existing == &url) {
            out.push(url);
        }
    };
    for source in entry.sources {
        for mirror in &network.model_mirrors {
            push(apply_prefix(mirror, source.url));
        }
        for candidate in download_candidates(source.url, &network.download_prefix) {
            push(candidate);
        }
    }
    out
}

/// 该条目的 sha256（清单里每个源的哈希一致，取第一个）。
fn sha256_of(entry: &ModelEntry) -> String {
    entry
        .sources
        .first()
        .map(|s| s.sha256.to_string())
        .unwrap_or_default()
}

fn emit_model(app: &AppHandle, id: &str, state: &str, received: u64, total: u64, message: Option<String>) {
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

/// 下载线程主体：下到 `*.part` → 校验 sha256（在 `download_verified` 内）→ 原子 rename。
fn run_download(app: &AppHandle, entry: &ModelEntry) {
    let dir = match models_dir(app) {
        Ok(dir) => dir,
        Err(e) => return finish(app, entry, Err(e)),
    };
    let final_path = dir.join(entry.filename);
    let part_path = dir.join(format!("{}.part", entry.filename));

    let network = app.state::<StoreRuntime>().network();
    let resolved = resolve_proxy(&network);
    let candidates = candidates_for(entry, &network);
    if candidates.is_empty() {
        return finish(app, entry, Err("这个模型没有可用的下载源".into()));
    }
    eprintln!("[model] 下载 {}：{}", entry.name, resolved.source);

    emit_model(app, entry.id, "downloading", 0, entry.approx_bytes, None);
    let cancel = &runtime().cancel;
    let id = entry.id;
    let approx = entry.approx_bytes;
    let mut progress = |received: u64, total: u64| {
        emit_model(app, id, "downloading", received, if total > 0 { total } else { approx }, None);
    };
    let result = download_verified(
        &candidates,
        &part_path,
        &sha256_of(entry),
        entry.approx_bytes,
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
            emit_model(app, entry.id, "verifying", 0, 0, None);
            match fs::rename(&part_path, &final_path) {
                Ok(()) => finish(app, entry, Ok(())),
                Err(e) => {
                    let _ = fs::remove_file(&part_path);
                    finish(app, entry, Err(format!("落地失败：{e}")))
                }
            }
        }
        Err(e) => {
            // 半成品必须删掉，否则下一轮会被当成「断点」继续拼一个可能已经错的文件。
            let _ = fs::remove_file(&part_path);
            finish(app, entry, Err(e))
        }
    }
}

fn finish(app: &AppHandle, entry: &ModelEntry, result: Result<(), String>) {
    runtime().running.store(false, Ordering::SeqCst);
    match result {
        Ok(()) => emit_model(app, entry.id, "done", entry.approx_bytes, entry.approx_bytes, None),
        Err(e) => emit_model(app, entry.id, "error", 0, 0, Some(e)),
    }
}

/// 在系统文件管理器里定位一个文件（沿用它所在平台的原生命令）。
fn reveal_in_file_manager(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = std::process::Command::new("open");
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
        let dir = path.parent().unwrap_or(path);
        let mut c = std::process::Command::new("xdg-open");
        c.arg(dir);
        c
    };
    command.spawn().map_err(|e| e.to_string())?;
    Ok(())
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

    #[test]
    fn candidates_prefer_user_mirrors_then_official_sources() {
        let entry = catalog::by_id("htdemucs-ort-v1").unwrap();
        let network = network_with(&["https://mirror.example/"], "");
        let candidates = candidates_for(entry, &network);
        // 每个源都会先给一个套了镜像前缀的候选（第一条即第一个源）。
        assert!(candidates[0].starts_with("https://mirror.example/http"));
        // 官方源仍然在候选里兜底（魔搭 + HF 都在清单里）。
        assert!(candidates
            .iter()
            .any(|u| u == "https://modelscope.cn/models/ailm32442/htdemucs-ort/resolve/master/htdemucs.ort"));
        assert!(candidates
            .iter()
            .any(|u| u == "https://huggingface.co/gentij/htdemucs-ort/resolve/main/htdemucs.ort"));
    }

    #[test]
    fn candidate_prefix_supports_url_placeholder() {
        let entry = catalog::by_id("htdemucs-ort-v1").unwrap();
        let network = network_with(&["https://mirror.example/?u={url}"], "");
        let candidates = candidates_for(entry, &network);
        assert!(candidates[0].starts_with("https://mirror.example/?u=http"));
    }

    #[test]
    fn candidate_list_is_deduplicated() {
        let entry = catalog::by_id("htdemucs-ort-v1").unwrap();
        let network = network_with(&["https://mirror.example/"], "");
        let candidates = candidates_for(entry, &network);
        let mut seen = std::collections::HashSet::new();
        for url in &candidates {
            assert!(seen.insert(url.clone()), "候选地址重复：{url}");
        }
    }
}
