//! **后端清单**（编译期内嵌）：每个执行后端支持的平台 / 架构。
//!
//! 三个条目全部**随包、零下载**——预编译构建本就在 `ort` 的 dist 里（macOS 的 aarch64 构建含
//! CoreML、Windows 构建含 DirectML），加对应 cargo 特性只解锁 `ort::ep::*` 的 API，
//! **不改下载的二进制、不改包体积**。

use super::detect::HardwareProfile;
use super::kind::BackendKind;

/// 清单里的一个后端。
#[derive(Debug, Clone, Copy)]
pub struct BackendEntry {
    /// 稳定标识。
    pub id: &'static str,
    /// 展示名。
    pub name: &'static str,
    /// 后端种类。
    pub kind: BackendKind,
    /// 支持的操作系统（`std::env::consts::OS` 的取值）。
    pub platforms: &'static [&'static str],
    /// 支持的架构（`std::env::consts::ARCH` 的取值）；空 = 不限。
    pub arches: &'static [&'static str],
    /// 一句话说明（界面上作为选项的副标题）。
    pub note: &'static str,
}

impl BackendEntry {
    /// 这个后端在本机画像下是否**平台可用**（OS + 架构都对得上）。
    pub fn platform_ok(&self, profile: &HardwareProfile) -> bool {
        self.platforms.contains(&profile.os)
            && (self.arches.is_empty() || self.arches.contains(&profile.arch))
    }
}

/// 清单。
pub static BACKEND_CATALOG: &[BackendEntry] = &[
    BackendEntry {
        id: "cpu",
        name: "CPU",
        kind: BackendKind::Cpu,
        platforms: &["macos", "windows", "linux"],
        arches: &[],
        note: "纯 CPU（默认，全平台可用）",
    },
    BackendEntry {
        id: "coreml",
        name: "Apple CoreML",
        kind: BackendKind::CoreMl,
        // **只 macOS + aarch64**：dist.tsv 里 `aarch64-apple-darwin` 只有 coreml / coreml+webgpu
        // 两个构建，`x86_64-apple-darwin` **一条都没有**（Intel Mac 只能 CPU）。
        platforms: &["macos"],
        arches: &["aarch64"],
        note: "Apple 神经引擎 / GPU（实测在本机的这些小模型上比 CPU 慢）",
    },
    BackendEntry {
        id: "directml",
        name: "DirectML",
        kind: BackendKind::DirectMl,
        // Windows 的默认预编译构建就含 DirectML。
        platforms: &["windows"],
        arches: &["aarch64", "x86_64"],
        note: "DirectX 12 硬件加速（Windows）",
    },
];

/// 全部条目。
pub fn all() -> &'static [BackendEntry] {
    BACKEND_CATALOG
}

/// 按 id 找条目。
pub fn by_id(id: &str) -> Option<&'static BackendEntry> {
    BACKEND_CATALOG.iter().find(|e| e.id == id)
}

/// 按种类找条目。
pub fn by_kind(kind: BackendKind) -> Option<&'static BackendEntry> {
    BACKEND_CATALOG.iter().find(|e| e.kind == kind)
}

/// 本机**平台可用**的后端（顺序同清单）。
pub fn supported(profile: &HardwareProfile) -> Vec<BackendKind> {
    BACKEND_CATALOG
        .iter()
        .filter(|e| e.platform_ok(profile))
        .map(|e| e.kind)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::detect::GpuVendor;

    fn profile(os: &'static str, arch: &'static str) -> HardwareProfile {
        HardwareProfile {
            os,
            arch,
            gpu: Some(GpuVendor::Apple),
            gpu_name: None,
            driver: None,
        }
    }

    #[test]
    fn ids_are_unique_and_every_kind_is_covered() {
        let mut ids = std::collections::HashSet::new();
        for entry in all() {
            assert!(ids.insert(entry.id), "重复 id：{}", entry.id);
            assert!(!entry.note.is_empty(), "{} 缺说明", entry.id);
        }
        for kind in crate::backend::ALL_BACKENDS {
            assert!(by_kind(kind).is_some(), "{kind} 在清单里没有条目");
        }
    }

    #[test]
    fn coreml_is_apple_silicon_only() {
        let coreml = by_id("coreml").unwrap();
        assert!(coreml.platform_ok(&profile("macos", "aarch64")));
        // Intel Mac 没有对应预编译构建。
        assert!(!coreml.platform_ok(&profile("macos", "x86_64")));
        assert!(!coreml.platform_ok(&profile("windows", "x86_64")));
    }

    #[test]
    fn supported_set_matches_the_platform() {
        assert_eq!(
            supported(&profile("macos", "aarch64")),
            vec![BackendKind::Cpu, BackendKind::CoreMl]
        );
        assert_eq!(
            supported(&profile("macos", "x86_64")),
            vec![BackendKind::Cpu]
        );
        assert_eq!(
            supported(&profile("windows", "x86_64")),
            vec![BackendKind::Cpu, BackendKind::DirectMl]
        );
        assert_eq!(
            supported(&profile("linux", "x86_64")),
            vec![BackendKind::Cpu]
        );
    }
}
