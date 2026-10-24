//! **执行后端（Execution Provider）的选择**——设置页用。
//!
//! 三个后端（CPU / CoreML / DirectML）**全部随包、零下载**：
//! 预编译构建本就在 `ort` 的 dist 里，加 cargo 特性只解锁 API，不改二进制、不改包体积。
//! 所以这里只有「选哪个」，没有下载 / 删除 / 进度那一套。
//!
//! 选择存在 `store.json` 的 `backend_preference`（见 [`crate::manager::BackendPreference`]），
//! 启动时由 [`init_runtime`] 落到 `ai_daw_local_ai::backend`；改设置后立即生效
//! （建会话是每次现建的，不需要重启）。
//!
//! 平台限制由 `ai_daw_local_ai::backend::catalog` 决定：Intel Mac 没有 CoreML、
//! 非 Windows 没有 DirectML——设置页只会列出本机可用的项。

use tauri::{AppHandle, Manager};

use crate::manager::{BackendPreference, StoreRuntime};
use ai_daw_local_ai::backend::{self as rt, BackendKind};

/// 一个可选后端（给设置页渲染）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct BackendOption {
    /// 稳定标识（`cpu` / `coreml` / `directml`）。
    pub kind: String,
    /// 展示名。
    pub label: String,
    /// 一句话说明。
    pub note: String,
    /// 是否本机平台支持（不可用的会被界面过滤掉）。
    pub available: bool,
}

/// 后端选择 + 本机情况。
#[derive(Debug, Clone, serde::Serialize)]
pub struct BackendInfo {
    /// 已保存的偏好。
    pub preference: String,
    /// 当前生效的后端（偏好在本机不支持时会回落 CPU）。
    pub current: String,
    /// 本机平台描述（展示用，如 `macos / aarch64 · Apple M4`）。
    pub platform: String,
    /// 全部选项（**含**本机不支持的，`available = false`）。
    pub options: Vec<BackendOption>,
}

/// 后端编译产物缓存目录（CoreML 用；跨重启复用）。
///
/// CoreML EP 默认**每次建会话都重编译模型**（实测 LocalVQE 一次 36 s）。给一个持久目录后
/// 第二次起只读缓存。只有选了 CoreML 才会用到。
pub fn coreml_cache_dir(app: &AppHandle) -> Option<std::path::PathBuf> {
    let dir = app
        .path()
        .app_config_dir()
        .ok()?
        .join("backend-cache")
        .join("coreml");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// 把已保存的偏好落到 `local-ai` 并校验 / 打印一行。
pub fn init_runtime(app: &AppHandle) {
    if let Some(dir) = coreml_cache_dir(app) {
        rt::session::set_coreml_cache_dir(dir);
    }
    let preference = app.state::<StoreRuntime>().network().backend_preference;
    rt::set_backend(preference.kind());
    rt::init();
}

/// 当前状态（设置页加载时调用）。
#[tauri::command]
pub fn ai_backend_info(app: AppHandle) -> BackendInfo {
    info_of(&app)
}

/// 保存执行后端偏好并立即应用（返回最新状态）。
#[tauri::command]
pub fn ai_set_backend(
    app: AppHandle,
    preference: BackendPreference,
) -> Result<BackendInfo, String> {
    let store = app.state::<StoreRuntime>();
    let mut network = store.network();
    network.backend_preference = preference;
    store.set(network)?;
    // 立即生效：建会话是每次现建的，下一次 AI 任务就用新后端。
    rt::set_backend(preference.kind());
    rt::init();
    Ok(info_of(&app))
}

fn info_of(app: &AppHandle) -> BackendInfo {
    let profile = rt::detect::detect();
    let preference = app.state::<StoreRuntime>().network().backend_preference;
    // 偏好在本机不支持时（如 Intel Mac 选了 CoreML），生效的是 CPU。
    let current = if rt::catalog::by_kind(preference.kind())
        .map(|e| e.platform_ok(&profile))
        .unwrap_or(false)
    {
        preference.kind()
    } else {
        BackendKind::Cpu
    };

    let options = rt::catalog::all()
        .iter()
        .map(|entry| BackendOption {
            kind: entry.kind.as_str().to_string(),
            label: entry.name.to_string(),
            note: entry.note.to_string(),
            available: entry.platform_ok(&profile),
        })
        .collect();

    BackendInfo {
        preference: preference.kind().as_str().to_string(),
        current: current.as_str().to_string(),
        platform: format!(
            "{} / {}{}",
            profile.os,
            profile.arch,
            profile
                .gpu_name
                .as_deref()
                .map(|g| format!(" · {g}"))
                .unwrap_or_default()
        ),
        options,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 偏好 → 种类的映射与 serde 名要一致。
    #[test]
    fn preference_maps_to_the_same_id_as_serde() {
        for pref in [
            BackendPreference::Cpu,
            BackendPreference::CoreMl,
            BackendPreference::DirectMl,
        ] {
            assert_eq!(pref.kind().as_str(), serde_name(pref));
        }
    }

    /// serde 名要和前端 / store.json 一致（`coreml` 不是一个词）。
    #[test]
    fn preference_serde_names_are_stable() {
        assert_eq!(
            serde_json::to_string(&BackendPreference::Cpu).unwrap(),
            "\"cpu\""
        );
        assert_eq!(
            serde_json::to_string(&BackendPreference::CoreMl).unwrap(),
            "\"coreml\""
        );
        assert_eq!(
            serde_json::from_str::<BackendPreference>("\"directml\"").unwrap(),
            BackendPreference::DirectMl
        );
    }

    /// 默认值必须就是 CPU（用户要求）。
    #[test]
    fn default_preference_is_cpu() {
        assert_eq!(BackendPreference::default(), BackendPreference::Cpu);
    }

    fn serde_name(pref: BackendPreference) -> String {
        serde_json::from_str::<String>(&serde_json::to_string(&pref).unwrap()).unwrap()
    }
}
