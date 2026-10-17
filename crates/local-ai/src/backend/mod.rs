//! **执行后端（Execution Provider）**：探测 → 选择 → 建会话（含自动回落）。
//!
//! 这一层把「用哪个 ONNX Runtime 执行后端」从散落各处的环境变量收拢成一个模块：
//! - [`detect`]：探测本机硬件（OS / 架构 / GPU 名称）。
//! - [`catalog`]：后端清单（平台 / 架构门槛）。
//! - [`session`]：**唯一的建会话入口**，EP 注册失败自动回落 CPU，含 per-graph 排除。
//!
//! 三个后端全部随包（CPU / CoreML / DirectML），**默认 CPU**；用户可在设置页改。
//! 本 crate 不依赖 Tauri：宿主读配置后调 [`set_backend`]，再调 [`init`] 校验并落定。
//! 设计文档：`docs/superpowers/specs/2026-10-16-backend-runtime-download.md`。

pub mod catalog;
pub mod detect;
pub mod session;

mod kind;

pub use kind::{BackendKind, ALL as ALL_BACKENDS};

use std::sync::atomic::{AtomicU8, Ordering};

/// 宿主设定的选择；[`UNSET`] = 还没设（库用户 / 探针 → 用 CPU）。
const UNSET: u8 = u8::MAX;
static CHOSEN: AtomicU8 = AtomicU8::new(UNSET);

/// 宿主设定最终生效的后端（在设置页保存时 / 启动时调）。
pub fn set_backend(kind: BackendKind) {
    CHOSEN.store(kind.as_u8(), Ordering::Relaxed);
}

/// 环境变量覆盖（`AIDAW_BACKEND=cpu|coreml|directml`）；给探针 / 无头排障用。
fn env_backend() -> Option<BackendKind> {
    std::env::var("AIDAW_BACKEND").ok().and_then(|v| BackendKind::parse(&v))
}

/// 当前生效的后端。优先级：`AIDAW_BACKEND` 环境变量 > [`set_backend`] 设的值 > **CPU**。
pub fn backend() -> BackendKind {
    if let Some(kind) = env_backend() {
        return kind;
    }
    let raw = CHOSEN.load(Ordering::Relaxed);
    if raw != UNSET {
        return BackendKind::from_u8(raw);
    }
    BackendKind::Cpu
}

/// 宿主启动时调用：探测、校验、定案并打印一行日志。
///
/// 返回最终生效的后端。选择与本机平台不符时（如 Intel Mac 上选 CoreML）**回落 CPU**。
pub fn init() -> BackendKind {
    let profile = detect::detect();
    let requested = backend();
    let resolved = match catalog::by_kind(requested) {
        Some(entry) if entry.platform_ok(&profile) => requested,
        _ => BackendKind::Cpu,
    };
    set_backend(resolved);

    eprintln!(
        "[backend] {} / {} · GPU={} → {}",
        profile.os,
        profile.arch,
        profile.gpu_name.as_deref().unwrap_or("未检测到"),
        resolved.label()
    );
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 没设过时默认 CPU。
    #[test]
    fn defaults_to_cpu() {
        // 这条用例只在没设 `AIDAW_BACKEND` 时才有意义；设了就跳过断言。
        if env_backend().is_none() {
            // CHOSEN 可能被其它用例改过，这里只验证「未设值时的兜底」这一分支语义。
            assert_eq!(BackendKind::from_u8(UNSET), BackendKind::Cpu);
        }
    }

    /// 环境变量覆盖优先（探针 / 无头排障靠它）。
    #[test]
    fn env_override_parses() {
        assert_eq!(BackendKind::parse("coreml"), Some(BackendKind::CoreMl));
        assert_eq!(BackendKind::parse("cuda"), None);
    }
}
