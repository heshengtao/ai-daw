//! **执行后端（Execution Provider）的选择 + 运行时状态**——设置页用。
//!
//! 两件事分开：
//! - **选哪个后端**（CPU / CoreML / DirectML / CUDA 12 / CUDA 13）：存在 `store.json` 的
//!   `backend_preference`，启动时由 [`init_runtime`] 落到 `local-ai`；
//! - **运行时装没装**：由 [`crate::runtimes`] 下载管理。没装的那一项会被标出来
//!   （`runtime_installed = false`），界面给「下载（体积）」按钮。
//!
//! **换运行时必须重启**：`ort` 的 dylib 在进程内只能加载一份（`OnceLock`），
//! 所以选了新后端、装了新运行时之后，本次进程仍跑旧的——[`BackendInfo::restart_required`]
//! 就是给界面的「重启后生效」提示。
//!
//! 光看「装没装」还不够，所以这里多下发两个诚实信号：
//! - [`BackendOption::missing`]：CUDA 的 cudart / cuBLAS / cuFFT / cuDNN 还缺哪几个。
//!   provider 缺一个依赖就载不进来，缺了却不说，用户只会看到「已安装 + GPU 利用率 0」。
//! - [`BackendInfo::ep_ready`]：provider 库是否**真的注册进了 ONNX Runtime**。
//!   没注册成功时 ORT 会静默忽略 CUDA，会话照建在 CPU 上。
//!
//! 平台限制由 `ai_daw_local_ai::backend::catalog` 决定：Intel Mac 没有 CoreML、
//! 非 Windows 没有 DirectML、macOS 没有 CUDA——设置页只会列出本机可用的项。
//! CUDA 另加一条：明确认出本机是 AMD / Intel / Apple 显卡时同样不给选（省掉 2.9 GB 白下）。

use tauri::{AppHandle, Manager};

use crate::manager::{BackendPreference, StoreRuntime};
use ai_daw_local_ai::backend::{self as rt, BackendKind, RuntimeId};

/// 一个可选后端（给设置页渲染）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct BackendOption {
    /// 稳定标识（`cpu` / `coreml` / `directml` / `cuda12` / `cuda13`）。
    pub kind: String,
    /// 展示名。
    pub label: String,
    /// 一句话说明。
    pub note: String,
    /// 是否本机可用（平台支持，且 CUDA 还要求本机不是别家显卡）。
    pub available: bool,
    /// 它需要哪一份运行时（`cpu` / `cuda` / …）。
    pub runtime: String,
    /// 那份运行时装好了没有。
    pub runtime_installed: bool,
    /// 需要下载多少字节（0 = 已装好）。
    pub download_bytes: u64,
    /// 还缺哪些**组件**（运行时 id，例如 `cufft`）。空 = 齐了 / 与它无关。
    ///
    /// CUDA 的 provider 硬依赖这几个，缺一个就载不进来并**静默回落 CPU**；
    /// 界面要能提前说出来，而不是等用户发现「GPU 利用率 0」。
    pub missing: Vec<String>,
}

/// 后端选择 + 本机情况。
#[derive(Debug, Clone, serde::Serialize)]
pub struct BackendInfo {
    /// 已保存的偏好。
    pub preference: String,
    /// 当前生效的后端（偏好在本机不支持时会回落 CPU）。
    pub current: String,
    /// 本机推荐的后端（NVIDIA 新卡 + 驱动 580+ 时是 `cuda13`，老卡 `cuda12`，否则 `cpu`）。
    pub recommended: String,
    /// 本机平台描述（展示用，如 `windows / x86_64 · NVIDIA GeForce RTX 4080`）。
    pub platform: String,
    /// 进程里**已经加载**的那份运行时；未加载为 `null`。
    pub loaded_runtime: Option<String>,
    /// 选了新后端 / 新运行时，但还没重启。
    pub restart_required: bool,
    /// CUDA provider 是否**真的注册进了 ONNX Runtime**。
    ///
    /// 只注册了核心 dylib 而 provider 库没注册成功时，ORT 会把
    /// `with_execution_providers([CUDA])` **静默忽略**：会话照建、跑在 CPU 上，
    /// 一句错误都没有。界面据此提示「本次会话会回落 CPU」。
    pub ep_ready: bool,
    /// 全部选项（**含**本机不支持的，`available = false`）。
    pub options: Vec<BackendOption>,
}

/// 后端编译产物缓存目录（CoreML 用；跨重启复用）。
pub fn coreml_cache_dir(app: &AppHandle) -> Option<std::path::PathBuf> {
    let dir = crate::paths::config_dir(app)
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
    // 运行时根目录要在任何加载之前交给 local-ai。
    crate::runtimes::init(app);
    // audio.cpp 引擎和 ONNX Runtime 各管各的目录（CUDA 版本也可以不同：引擎是独立进程）。
    crate::yue::init(app);
    let preference = app.state::<StoreRuntime>().network().backend_preference;
    rt::set_backend(preference.kind());
    rt::runtime::set_runtime(rt::runtime::for_backend(preference.kind()));
    rt::init();
}

/// 当前状态（设置页加载时调用）。
#[tauri::command]
pub fn ai_backend_info(app: AppHandle) -> BackendInfo {
    info_of(&app)
}

/// 保存执行后端偏好（返回最新状态；运行时没装的话界面会去下载）。
#[tauri::command]
pub fn ai_set_backend(
    app: AppHandle,
    preference: BackendPreference,
) -> Result<BackendInfo, String> {
    let store = app.state::<StoreRuntime>();
    let mut network = store.network();
    network.backend_preference = preference;
    store.set(network)?;
    // 立即生效的部分：下一次建会话用的 EP。运行时若还没加载，会在那时按新选择加载；
    // 已经加载过就改不了——界面据此提示重启。
    rt::set_backend(preference.kind());
    rt::runtime::set_runtime(rt::runtime::for_backend(preference.kind()));
    Ok(info_of(&app))
}

fn info_of(app: &AppHandle) -> BackendInfo {
    // 命中启动时的后台预热缓存；只有当设置页抢在预热完成前打开时，才会等这一次探测。
    let profile = rt::hardware();
    let preference = app.state::<StoreRuntime>().network().backend_preference;
    // CUDA 只对 N 卡有意义：明确认出是别家显卡时就不给选了（能省掉一次 2.9 GB 的白下载）。
    // 探测不出来（`None`）时不拦——宁可让用户自己决定，也不要把能用的人挡在外面。
    let cuda_ok = !matches!(
        profile.gpu,
        Some(
            rt::detect::GpuVendor::Apple
                | rt::detect::GpuVendor::Amd
                | rt::detect::GpuVendor::Intel
        )
    );
    // 偏好在本机不支持时（如 Intel Mac 选了 CoreML、或在 AMD 机器上留着旧配置的 CUDA），
    // 生效的是 CPU —— 这里必须和 `available` 用同一套判断，否则界面会出现「没有选中项」。
    let current = rt::catalog::by_kind(preference.kind())
        .filter(|entry| entry.platform_ok(profile))
        .filter(|entry| !entry.kind.is_cuda() || cuda_ok)
        .map(|entry| entry.kind)
        .unwrap_or(BackendKind::Cpu);

    let options = rt::catalog::all()
        .iter()
        .map(|entry| {
            let runtime = rt::runtime::for_backend(entry.kind);
            // 静态链接的平台（macOS）运行时就编在二进制里，永远算「已装好」。
            let installed = rt::runtime::statically_linked() || rt::runtime::installed(runtime);
            let is_cuda = entry.kind.is_cuda();
            let missing: Vec<String> = if is_cuda {
                rt::runtime::components_of(runtime)
                    .into_iter()
                    .filter(|id| !(rt::runtime::statically_linked() || rt::runtime::installed(*id)))
                    .map(|id| id.as_str().to_string())
                    .collect()
            } else {
                Vec::new()
            };
            BackendOption {
                kind: entry.kind.as_str().to_string(),
                label: entry.name.to_string(),
                note: entry.note.to_string(),
                available: entry.platform_ok(profile) && (!is_cuda || cuda_ok),
                runtime: runtime.as_str().to_string(),
                runtime_installed: installed,
                download_bytes: if installed {
                    0
                } else {
                    rt::runtime::asset(runtime).map(|a| a.bytes()).unwrap_or(0)
                },
                missing,
            }
        })
        .collect();

    BackendInfo {
        preference: preference.kind().as_str().to_string(),
        current: current.as_str().to_string(),
        recommended: crate::runtimes::recommended_backend().as_str().to_string(),
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
        loaded_runtime: rt::runtime::loaded_path().and_then(|p| {
            p.parent()
                .and_then(|d| d.file_name())
                .map(|n| n.to_string_lossy().into_owned())
        }),
        restart_required: crate::runtimes::restart_required(),
        // 只看**当前选中的那个大版本**：从 12 切到 13 还没重启时，provider 仍是 12，
        // 对 13 来说就是没启用（界面会提示重启，而不是给一个假的绿灯）。
        ep_ready: preference
            .kind()
            .cuda_flavor()
            .map(rt::runtime::cuda_ep_ready_for)
            .unwrap_or(false),
        options,
    }
}

/// 当前运行时的 id（探针 / MCP 用）。
pub fn loaded_runtime() -> Option<RuntimeId> {
    ai_daw_local_ai::backend::runtime::loaded_path()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .and_then(|d| {
            d.file_name()
                .and_then(|n| RuntimeId::parse(&n.to_string_lossy()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_maps_to_the_same_id_as_serde() {
        for pref in [
            BackendPreference::Cpu,
            BackendPreference::CoreMl,
            BackendPreference::DirectMl,
            BackendPreference::Cuda12,
            BackendPreference::Cuda13,
        ] {
            assert_eq!(pref.kind().as_str(), serde_name(pref));
        }
    }

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
        assert_eq!(
            serde_json::to_string(&BackendPreference::Cuda12).unwrap(),
            "\"cuda12\""
        );
        assert_eq!(
            serde_json::from_str::<BackendPreference>("\"cuda13\"").unwrap(),
            BackendPreference::Cuda13
        );
        // 旧版只存 `"cuda"`：继续按 CUDA 12 解释，别把老卡带到 13。
        assert_eq!(
            serde_json::from_str::<BackendPreference>("\"cuda\"").unwrap(),
            BackendPreference::Cuda12
        );
    }

    #[test]
    fn default_preference_is_cpu() {
        assert_eq!(BackendPreference::default(), BackendPreference::Cpu);
    }

    fn serde_name(pref: BackendPreference) -> String {
        serde_json::from_str::<String>(&serde_json::to_string(&pref).unwrap()).unwrap()
    }
}
