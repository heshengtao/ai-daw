//! **执行后端（Execution Provider）的种类**。
//!
//! - `Cpu`：ONNX Runtime 自带的 CPU EP，全平台可用，也是**默认**；
//! - `CoreMl`：macOS Apple Silicon（运行时随 `coreml` 条目下载，`cpu` 共用同一份）；
//! - `DirectMl`：Windows（需要 DirectML 版运行时；ORT 只为它发布到 1.24）；
//! - `Cuda12`：NVIDIA CUDA 12（Windows / Linux x86_64；老卡 / 老驱动）；
//! - `Cuda13`：NVIDIA CUDA 13（Windows / Linux x86_64；Turing 起 + 驱动 580+）。
//!
//! CUDA 的两份构建是**两个独立后端**，和 audio.cpp 引擎一样让用户自己挑：
//! 一份运行时只对一个大版本有效（CUDA 13 不再支持 Maxwell / Pascal / Volta），
//! 自动猜错时用户只会看到「选了 CUDA 却回落 CPU」，不如把选择权交出来。
//!
//! 静态链接时代（`download-binaries`）后端是「编译期三选一」；现在是 `load-dynamic`：
//! 运行时是下载来的 dylib，**一个进程只能加载一份**，所以切后端要重启（见
//! [`super::runtime`]）。
//!
//! 这个类型**不引 serde**（本 crate 无 serde 依赖）：宿主用 [`BackendKind::as_str`] 自己序列化。

/// 执行后端种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendKind {
    /// 纯 CPU（ONNX Runtime 内建，恒可用，默认）。
    Cpu,
    /// Apple CoreML（macOS，Apple Silicon）。
    CoreMl,
    /// Windows DirectML（DirectX 12 硬件）。
    DirectMl,
    /// NVIDIA CUDA **12**（Windows / Linux x86_64）。**需要单独下载**一份带 CUDA 12 EP 的
    /// ONNX Runtime，另需 CUDA 12 运行库与 cuDNN 9。
    Cuda12,
    /// NVIDIA CUDA **13**（Windows / Linux x86_64；ORT 1.31+ 构建）。需要 CUDA 13 运行库。
    Cuda13,
}

/// 清单里出现的全部后端（顺序即界面展示顺序）。
pub const ALL: [BackendKind; 5] = [
    BackendKind::Cpu,
    BackendKind::CoreMl,
    BackendKind::DirectMl,
    BackendKind::Cuda12,
    BackendKind::Cuda13,
];

impl BackendKind {
    /// 稳定标识（序列化 / store.json 用它）。
    pub fn as_str(self) -> &'static str {
        match self {
            BackendKind::Cpu => "cpu",
            BackendKind::CoreMl => "coreml",
            BackendKind::DirectMl => "directml",
            BackendKind::Cuda12 => "cuda12",
            BackendKind::Cuda13 => "cuda13",
        }
    }

    /// 从标识解析（大小写不敏感）；不认识返回 `None`。
    ///
    /// `cuda` / `gpu` 是旧配置里的写法，统一落到 CUDA 12：老配置是在只有 CUDA 12
    /// 的年代存下来的，按 12 解释不会把老卡带到跑不起来的 13 上。
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "cpu" => Some(BackendKind::Cpu),
            "coreml" | "core_ml" => Some(BackendKind::CoreMl),
            "directml" | "dml" => Some(BackendKind::DirectMl),
            "cuda12" | "cuda-12" | "cuda_12" | "cuda" | "gpu" => Some(BackendKind::Cuda12),
            "cuda13" | "cuda-13" | "cuda_13" => Some(BackendKind::Cuda13),
            _ => None,
        }
    }

    /// 展示名（给界面用）。
    pub fn label(self) -> &'static str {
        match self {
            BackendKind::Cpu => "CPU",
            BackendKind::CoreMl => "CoreML",
            BackendKind::DirectMl => "DirectML",
            BackendKind::Cuda12 => "CUDA 12",
            BackendKind::Cuda13 => "CUDA 13",
        }
    }

    /// 是不是 CUDA 家族（12 / 13）。
    pub const fn is_cuda(self) -> bool {
        matches!(self, BackendKind::Cuda12 | BackendKind::Cuda13)
    }

    /// CUDA 大版本（12 / 13）；非 CUDA 返回 `None`。
    pub const fn cuda_flavor(self) -> Option<u8> {
        match self {
            BackendKind::Cuda12 => Some(12),
            BackendKind::Cuda13 => Some(13),
            _ => None,
        }
    }

    /// 存进原子变量的字节表示。
    pub(crate) const fn as_u8(self) -> u8 {
        match self {
            BackendKind::Cpu => 0,
            BackendKind::CoreMl => 1,
            BackendKind::DirectMl => 2,
            BackendKind::Cuda12 => 3,
            BackendKind::Cuda13 => 4,
        }
    }

    pub(crate) const fn from_u8(value: u8) -> Self {
        match value {
            1 => BackendKind::CoreMl,
            2 => BackendKind::DirectMl,
            3 => BackendKind::Cuda12,
            4 => BackendKind::Cuda13,
            _ => BackendKind::Cpu,
        }
    }
}

impl std::fmt::Display for BackendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_the_stable_id() {
        for kind in ALL {
            assert_eq!(BackendKind::parse(kind.as_str()), Some(kind));
            assert_eq!(BackendKind::from_u8(kind.as_u8()), kind);
        }
        // 大小写不敏感 + 常见别名。
        assert_eq!(BackendKind::parse(" CoreML "), Some(BackendKind::CoreMl));
        assert_eq!(BackendKind::parse("DML"), Some(BackendKind::DirectMl));
        assert_eq!(BackendKind::parse("CUDA"), Some(BackendKind::Cuda12));
        assert_eq!(BackendKind::parse("cuda13"), Some(BackendKind::Cuda13));
        assert_eq!(BackendKind::parse("vulkan"), None);
    }

    #[test]
    fn cuda_ids_are_distinct_and_ordered() {
        assert_eq!(BackendKind::Cuda12.as_str(), "cuda12");
        assert_eq!(BackendKind::Cuda13.as_str(), "cuda13");
        assert_eq!(BackendKind::Cuda12.cuda_flavor(), Some(12));
        assert_eq!(BackendKind::Cuda13.cuda_flavor(), Some(13));
        assert_ne!(BackendKind::Cuda12.as_u8(), BackendKind::Cuda13.as_u8());
    }
}
