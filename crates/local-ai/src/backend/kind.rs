//! **执行后端（Execution Provider）的种类**。
//!
//! 只有三个，且**全部是随包 / 零下载**的：
//! - `Cpu`：ONNX Runtime 自带的 CPU EP，全平台可用，也是**默认**；
//! - `CoreMl`：macOS Apple Silicon。`aarch64-apple-darwin` 的预编译构建本就含 CoreML，
//!   加特性只解锁 `ort::ep::CoreML` 的 API，**不改下载的二进制、不改包体积**；
//! - `DirectMl`：Windows。`x86_64-pc-windows-msvc` 的预编译构建本就含 DirectML，同理零下载。
//!
//! 没有 CUDA：它需要 `load-dynamic` + 运行时下载数百 MB 的 dylib，且本机无法验证；
//! 对一个「小模型」为主的桌面应用来说收益不符成本（CoreML 实测还更慢，见 spec §10.3）。
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
}

/// 清单里出现的全部后端（顺序即界面展示顺序）。
pub const ALL: [BackendKind; 3] = [BackendKind::Cpu, BackendKind::CoreMl, BackendKind::DirectMl];

impl BackendKind {
    /// 稳定标识（序列化 / store.json 用它）。
    pub fn as_str(self) -> &'static str {
        match self {
            BackendKind::Cpu => "cpu",
            BackendKind::CoreMl => "coreml",
            BackendKind::DirectMl => "directml",
        }
    }

    /// 从标识解析（大小写不敏感）；不认识返回 `None`。
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "cpu" => Some(BackendKind::Cpu),
            "coreml" | "core_ml" => Some(BackendKind::CoreMl),
            "directml" | "dml" => Some(BackendKind::DirectMl),
            _ => None,
        }
    }

    /// 展示名（给界面用）。
    pub fn label(self) -> &'static str {
        match self {
            BackendKind::Cpu => "CPU",
            BackendKind::CoreMl => "CoreML",
            BackendKind::DirectMl => "DirectML",
        }
    }

    /// 存进原子变量的字节表示。
    pub(crate) const fn as_u8(self) -> u8 {
        match self {
            BackendKind::Cpu => 0,
            BackendKind::CoreMl => 1,
            BackendKind::DirectMl => 2,
        }
    }

    pub(crate) const fn from_u8(value: u8) -> Self {
        match value {
            1 => BackendKind::CoreMl,
            2 => BackendKind::DirectMl,
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
        assert_eq!(BackendKind::parse("cuda"), None);
    }
}
