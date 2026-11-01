//! **ONNX Runtime 运行时（dylib）**：清单、安装状态、加载。
//!
//! 为什么需要这一层：`ort` 编译时只有一个选择——要么**静态链接**一份 ONNX Runtime
//! （`download-binaries`，预编译包缺一个特性就构建失败），要么**运行期加载**
//! （`load-dynamic`：`ort-sys` 的 build 脚本直接 return，不下载、不链接、不解析 dist）。
//! 后者才允许「安装包里只有 CPU 运行时，CUDA 按需下载」。
//!
//! 一旦切到 `load-dynamic`，进程里**只能有一个** `onnxruntime` dylib（`ort` 用
//! `OnceLock` 缓存），所以：
//! - 加载在**第一次建会话前**完成，之后不可更换 → 切后端要重启（界面必须说清楚）；
//! - 加载失败时**绝不能**让 `ort` 走它的默认路径（那会 `expect` 直接 panic）——
//!   本模块先 `ort::init_from()` 拿到 `Result`，失败就把错误留给
//!   [`super::session::build`] 变成一句可读的话。
//!
//! 运行时按 **平台 + 架构 + 构建种类** 分成几个条目（[`for_platform`]）：
//! - `cpu`：纯 CPU 构建（全平台）；
//! - `cuda`：CUDA **12** EP 构建（Windows / Linux x86_64，数百 MB，按需下载）；
//! - `cuda13`：CUDA **13** EP 构建（Windows / Linux x86_64，ORT 1.31+，按需下载）；
//! - `directml`：DirectML EP 构建（Windows；ORT 只为它发布到 1.24）；
//! - `coreml`：macOS arm64 构建（本就含 CoreML EP，`cpu` 与 `coreml` 共用这一份）。
//!
//! CUDA 12 / 13 的组件（cudart / cuBLAS / cuFFT / cuDNN）也各有一套，目录名带 `13`
//! 后缀；预加载按 dylib 所在目录名选对应的库名，避免把两套 CUDA 混进一个进程。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

/// 清单锁定的 ONNX Runtime 版本（CPU / CUDA 12 那两份）。
///
/// 本 crate 按 `api-24` 编译：1.24 起的 dylib 都能加载（含 1.30 / 1.31）。CUDA 13 那份
/// 用 ORT 1.31.1（1.31 起官方 GPU 包才提供 CUDA 13 构建），同样落在 api-24 的 ABI 上。
/// 升级 ORT 时整表一起换，并把 sha256 重新钉死。
pub const ORT_VERSION: &str = "1.30.0";

/// 运行时种类（= 一份可加载的 dylib）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeId {
    /// 纯 CPU 构建。
    Cpu,
    /// DirectML 构建（Windows）。
    DirectMl,
    /// CUDA **12** 构建（Windows / Linux）。
    Cuda,
    /// CUDA **13** 构建（ORT 1.30+；新卡 / 新驱动）。
    Cuda13,
    /// macOS arm64 构建（含 CoreML）。
    CoreMl,
    /// CUDA 12 运行时（`cudart64_12.dll`）。
    Cudart,
    /// CUDA 12 的 cuBLAS。
    Cublas,
    /// cuDNN 9（CUDA 12 版）。
    Cudnn,
    /// cuFFT（CUDA 12 的 `cufft64_11.dll`）：ORT 1.26 的 CUDA provider 硬依赖它。
    Cufft,
    /// CUDA 13 运行时（`cudart64_13.dll`）。
    Cudart13,
    /// CUDA 13 的 cuBLAS。
    Cublas13,
    /// cuDNN 9（CUDA 13 版）。
    Cudnn13,
    /// CUDA 13 的 cuFFT（`cufft64_12.dll`）。
    Cufft13,
}

impl RuntimeId {
    pub fn as_str(self) -> &'static str {
        match self {
            RuntimeId::Cpu => "cpu",
            RuntimeId::DirectMl => "directml",
            RuntimeId::Cuda => "cuda",
            RuntimeId::Cuda13 => "cuda13",
            RuntimeId::CoreMl => "coreml",
            RuntimeId::Cudart => "cudart",
            RuntimeId::Cublas => "cublas",
            RuntimeId::Cudnn => "cudnn",
            RuntimeId::Cufft => "cufft",
            RuntimeId::Cudart13 => "cudart13",
            RuntimeId::Cublas13 => "cublas13",
            RuntimeId::Cudnn13 => "cudnn13",
            RuntimeId::Cufft13 => "cufft13",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "cpu" => Some(RuntimeId::Cpu),
            "directml" | "dml" => Some(RuntimeId::DirectMl),
            "cuda" | "gpu" => Some(RuntimeId::Cuda),
            "cuda13" | "cuda-13" | "cuda_13" => Some(RuntimeId::Cuda13),
            "coreml" => Some(RuntimeId::CoreMl),
            "cudart" => Some(RuntimeId::Cudart),
            "cublas" => Some(RuntimeId::Cublas),
            "cudnn" => Some(RuntimeId::Cudnn),
            "cufft" => Some(RuntimeId::Cufft),
            "cudart13" | "cudart-13" => Some(RuntimeId::Cudart13),
            "cublas13" | "cublas-13" => Some(RuntimeId::Cublas13),
            "cudnn13" | "cudnn-13" => Some(RuntimeId::Cudnn13),
            "cufft13" | "cufft-13" => Some(RuntimeId::Cufft13),
            _ => None,
        }
    }

    /// 是不是 CUDA 运行时的组件（挂在 ORT CUDA 下面）。
    pub const fn is_component(self) -> bool {
        matches!(
            self,
            RuntimeId::Cudart
                | RuntimeId::Cublas
                | RuntimeId::Cudnn
                | RuntimeId::Cufft
                | RuntimeId::Cudart13
                | RuntimeId::Cublas13
                | RuntimeId::Cudnn13
                | RuntimeId::Cufft13
        )
    }

    /// 是不是 CUDA 本体（12 / 13 两份构建）。
    pub const fn is_cuda(self) -> bool {
        matches!(self, RuntimeId::Cuda | RuntimeId::Cuda13)
    }

    /// CUDA 大版本（12 / 13）；非 CUDA 相关返回 `None`。
    pub const fn cuda_flavor(self) -> Option<u8> {
        match self {
            RuntimeId::Cuda
            | RuntimeId::Cudart
            | RuntimeId::Cublas
            | RuntimeId::Cudnn
            | RuntimeId::Cufft => Some(12),
            RuntimeId::Cuda13
            | RuntimeId::Cudart13
            | RuntimeId::Cublas13
            | RuntimeId::Cudnn13
            | RuntimeId::Cufft13 => Some(13),
            _ => None,
        }
    }

    pub const fn as_u8(self) -> u8 {
        match self {
            RuntimeId::Cpu => 0,
            RuntimeId::DirectMl => 1,
            RuntimeId::Cuda => 2,
            RuntimeId::CoreMl => 3,
            RuntimeId::Cudart => 4,
            RuntimeId::Cublas => 5,
            RuntimeId::Cudnn => 6,
            RuntimeId::Cufft => 7,
            RuntimeId::Cuda13 => 8,
            RuntimeId::Cudart13 => 9,
            RuntimeId::Cublas13 => 10,
            RuntimeId::Cudnn13 => 11,
            RuntimeId::Cufft13 => 12,
        }
    }

    pub const fn from_u8(value: u8) -> Self {
        match value {
            1 => RuntimeId::DirectMl,
            2 => RuntimeId::Cuda,
            3 => RuntimeId::CoreMl,
            4 => RuntimeId::Cudart,
            5 => RuntimeId::Cublas,
            6 => RuntimeId::Cudnn,
            7 => RuntimeId::Cufft,
            8 => RuntimeId::Cuda13,
            9 => RuntimeId::Cudart13,
            10 => RuntimeId::Cublas13,
            11 => RuntimeId::Cudnn13,
            12 => RuntimeId::Cufft13,
            _ => RuntimeId::Cpu,
        }
    }
}

/// dylib 的平台文件名。
pub const fn library_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "onnxruntime.dll"
    }
    #[cfg(target_os = "macos")]
    {
        "libonnxruntime.dylib"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "libonnxruntime.so"
    }
}

/// 一个下载源。
#[derive(Debug, Clone, Copy)]
pub struct RuntimeSource {
    /// `intl`（官方）/ `cn`（国内镜像）。
    pub region: &'static str,
    pub url: &'static str,
}

/// 归档里要取出的一个文件。
#[derive(Debug, Clone, Copy)]
pub struct RuntimeFile {
    /// 归档内路径。
    pub inner: &'static str,
    /// 落地文件名（相对运行时目录；可带子目录）。
    pub dest: &'static str,
}

/// 一份归档（一个运行时可能需要两份：例如 DirectML 构建 + `DirectML.dll` 本体）。
#[derive(Debug, Clone, Copy)]
pub struct RuntimePart {
    /// 下载时的文件名。
    pub archive: &'static str,
    /// 整包体积（字节；断点续传 / 回退整包下载时用）。
    pub bytes: u64,
    /// **只取清单里那几个文件**时的传输量（HTTP Range）。
    ///
    /// 官方 release 包是个 SDK（head/import lib 都有，解压 417 MB），而我们要的只有两个 dll；
    /// 实测 CPU 从 79 MB 降到 5.7 MB、DirectML 从 205 MB 降到 14.7 MB。`None` = 未测量，
    /// 按整包算。
    pub pick_bytes: Option<u64>,
    /// 解压后落到磁盘的字节数（界面显示「占用」用）。
    pub disk_bytes: u64,
    /// 归档 sha256；`None` = 暂未钉死（只校验大小）。
    pub sha256: Option<&'static str>,
    pub sources: &'static [RuntimeSource],
    pub files: &'static [RuntimeFile],
}

/// 清单里的一个运行时。
#[derive(Debug, Clone, Copy)]
pub struct RuntimeAsset {
    pub id: RuntimeId,
    /// `std::env::consts::OS`。
    pub os: &'static str,
    /// `std::env::consts::ARCH`。
    pub arch: &'static str,
    /// 展示名。
    pub label: &'static str,
    /// 一句话说明（界面副标题）。
    pub note: &'static str,
    /// 装好的判定文件（相对运行时目录）——**空数组表示「用解压清单里的全部文件」**。
    ///
    /// 以前这里是单个 `marker`：只要 `onnxruntime.dll` 在就算装好，于是「解压到一半」
    /// （缺 `providers_cuda.dll` / `cudnn_*` / `cufft64_11.dll`）的目录也会显示成已安装，
    /// 直到建会话才炸、而且静默回落 CPU。现在逐文件判定：缺一个就当没装（与 ai-cut 同策略）。
    pub required: &'static [&'static str],
    /// 随安装包分发（CPU / DirectML / CoreML）：不下载、不可删除。
    pub bundled: bool,
    pub parts: &'static [RuntimePart],
}

impl RuntimeAsset {
    /// **下载量**：能只取所需文件时按那个算，否则按整包。
    pub fn bytes(&self) -> u64 {
        self.parts
            .iter()
            .map(|p| p.pick_bytes.unwrap_or(p.bytes))
            .sum()
    }

    /// 装完占用（字节）。
    pub fn disk_bytes(&self) -> u64 {
        self.parts.iter().map(|p| p.disk_bytes).sum()
    }

    /// 「装好」必须存在的文件（逐文件判定）。
    ///
    /// 清单给了 `required` 就用它（随包的那几份只可能有一两个文件）；否则用每份归档
    /// 解压出来的**全部** `dest`——半截目录只要缺一个就不算装好。
    pub fn required(&self) -> Vec<&'static str> {
        if !self.required.is_empty() {
            return self.required.to_vec();
        }
        self.parts
            .iter()
            .flat_map(|p| p.files.iter().map(|f| f.dest))
            .collect()
    }
}

static WINDOWS: &[RuntimeAsset] = &[
    RuntimeAsset {
        id: RuntimeId::Cpu,
        os: "windows",
        arch: "x86_64",
        label: "ONNX Runtime（CPU）",
        note: "纯 CPU，全平台可用，默认",
        required: &["onnxruntime.dll"],
        bundled: true,
        parts: &[RuntimePart {
            archive: "onnxruntime-win-x64-1.30.0.zip",
            bytes: 82_645_522,
            pick_bytes: Some(5_936_510),
            disk_bytes: 16_484_464,
            sha256: None,
            sources: &[RuntimeSource {
                region: "intl",
                url: "https://github.com/microsoft/onnxruntime/releases/download/v1.30.0/onnxruntime-win-x64-1.30.0.zip",
            }],
            files: &[
                RuntimeFile {
                    inner: "onnxruntime-win-x64-1.30.0/lib/onnxruntime.dll",
                    dest: "onnxruntime.dll",
                },
                RuntimeFile {
                    inner: "onnxruntime-win-x64-1.30.0/lib/onnxruntime_providers_shared.dll",
                    dest: "onnxruntime_providers_shared.dll",
                },
            ],
        }],
    },
    RuntimeAsset {
        id: RuntimeId::Cuda,
        os: "windows",
        arch: "x86_64",
        label: "ONNX Runtime（CUDA 12）",
        // 为什么钉 1.26 而不是最新的 1.30：**版本必须和组件对齐**。
        // 1.30 的 `onnxruntime_providers_cuda.dll` 是 CUDA **13** 构建（硬依赖 `cublas64_13`），
        // 而我们给用户下载的是 CUDA 12 的 cudart / cuBLAS / cuDNN；对不上就 `LoadLibrary`
        // 失败、ORT 静默回落 CPU。1.26 是 CUDA **12** 构建（依赖 `cublas64_12` / `cudart64_12`
        // / `cufft64_11` / `cudnn64_9`），与组件完全匹配，且 CUDA 12.8 覆盖 Turing…Blackwell。
        note: "NVIDIA 显卡加速（CUDA 12，匹配随包组件）；缺的 cudart / cuBLAS / cuFFT / cuDNN 9 会自动补齐。与 audio.cpp 引擎的 CUDA 13 互不影响（不同进程、各自一套 DLL）",
        // 逐文件判定：三个 dll 缺一个都不算装好（半截的 CUDA 目录会静默回落 CPU）。
        required: &[],
        bundled: false,
        parts: &[RuntimePart {
            archive: "microsoft.ml.onnxruntime.gpu.windows.1.26.0.nupkg",
            bytes: 187_934_177,
            pick_bytes: None,
            disk_bytes: 315_000_000,
            sha256: Some("2820b8cca24b7c2817f28aa6ea8774a5c519295caa7447b86d190c7079aabf0f"),
            sources: &[
                RuntimeSource {
                    region: "intl",
                    url: "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime.gpu.windows/1.26.0/microsoft.ml.onnxruntime.gpu.windows.1.26.0.nupkg",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://mirrors.huaweicloud.com/artifactory/api/nuget/v3/nuget-remote/microsoft.ml.onnxruntime.gpu.windows/1.26.0/microsoft.ml.onnxruntime.gpu.windows.1.26.0.nupkg",
                },
            ],
            files: &[
                RuntimeFile {
                    inner: "runtimes/win-x64/native/onnxruntime.dll",
                    dest: "onnxruntime.dll",
                },
                RuntimeFile {
                    inner: "runtimes/win-x64/native/onnxruntime_providers_shared.dll",
                    dest: "onnxruntime_providers_shared.dll",
                },
                RuntimeFile {
                    inner: "runtimes/win-x64/native/onnxruntime_providers_cuda.dll",
                    dest: "onnxruntime_providers_cuda.dll",
                },
            ],
        }],
    },
    RuntimeAsset {
        id: RuntimeId::Cuda13,
        os: "windows",
        arch: "x86_64",
        label: "ONNX Runtime（CUDA 13）",
        // 1.30 起 ORT 的 Windows GPU 包换成 CUDA 13 构建（provider 硬依赖 `cublas64_13` /
        // `cudart64_13` / `cufft64_12` / `cudnn64_9`）。CUDA 13 不再支持 Maxwell / Pascal /
        // Volta，且驱动要 580+——所以只在检测到新卡时推荐它（`cuda13_preferred`）。
        note: "NVIDIA 新卡加速（CUDA 13，ORT 1.31 构建）；配套 cudart 13 / cuBLAS 13 / cuFFT 12 / cuDNN 9 会一起装",
        required: &[],
        bundled: false,
        parts: &[RuntimePart {
            archive: "microsoft.ml.onnxruntime.gpu.windows.1.31.1.nupkg",
            bytes: 160_446_793,
            pick_bytes: None,
            disk_bytes: 215_000_000,
            sha256: Some("9c9c7c951b9e76036b5e90d2a519359969d717d1c16dfc718c808afb42d778b1"),
            sources: &[
                RuntimeSource {
                    region: "intl",
                    url: "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime.gpu.windows/1.31.1/microsoft.ml.onnxruntime.gpu.windows.1.31.1.nupkg",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://mirrors.huaweicloud.com/artifactory/api/nuget/v3/nuget-remote/microsoft.ml.onnxruntime.gpu.windows/1.31.1/microsoft.ml.onnxruntime.gpu.windows.1.31.1.nupkg",
                },
            ],
            files: &[
                RuntimeFile {
                    inner: "runtimes/win-x64/native/onnxruntime.dll",
                    dest: "onnxruntime.dll",
                },
                RuntimeFile {
                    inner: "runtimes/win-x64/native/onnxruntime_providers_shared.dll",
                    dest: "onnxruntime_providers_shared.dll",
                },
                RuntimeFile {
                    inner: "runtimes/win-x64/native/onnxruntime_providers_cuda.dll",
                    dest: "onnxruntime_providers_cuda.dll",
                },
            ],
        }],
    },
    RuntimeAsset {
        id: RuntimeId::Cudart,
        os: "windows",
        arch: "x86_64",
        label: "CUDA Runtime 12 (cudart)",
        note: "CUDA 12 运行时；ORT 的 CUDA 版需要它，切换 CUDA 时自动补齐",
        required: &[],
        bundled: false,
        parts: &[RuntimePart {
            archive: "cuda_cudart-windows-x86_64-12.8.90-archive.zip",
            bytes: 3_037_735,
            pick_bytes: Some(3_037_735),
            disk_bytes: 600_000,
            sha256: Some("4a39058fd8519444a81cfc7ae055d136f48d1a31ffa41ae255b35b2edd61e13b"),
            sources: &[
                RuntimeSource {
                    region: "intl",
                    url: "https://developer.download.nvidia.com/compute/cuda/redist/cuda_cudart/windows-x86_64/cuda_cudart-windows-x86_64-12.8.90-archive.zip",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://developer.download.nvidia.cn/compute/cuda/redist/cuda_cudart/windows-x86_64/cuda_cudart-windows-x86_64-12.8.90-archive.zip",
                },
            ],
            files: &[RuntimeFile {
                inner: "cuda_cudart-windows-x86_64-12.8.90-archive/bin/cudart64_12.dll",
                dest: "cudart64_12.dll",
            }],
        }],
    },
    RuntimeAsset {
        id: RuntimeId::Cublas,
        os: "windows",
        arch: "x86_64",
        label: "cuBLAS 12",
        note: "CUDA 12 的矩阵运算库；ORT 的 CUDA 版需要它，切换 CUDA 时自动补齐",
        required: &[],
        bundled: false,
        parts: &[RuntimePart {
            archive: "libcublas-windows-x86_64-12.8.4.1-archive.zip",
            bytes: 563_660_944,
            pick_bytes: None,
            disk_bytes: 620_000_000,
            sha256: Some("57a470112cec7e112c95253dde8b3c7184d795dbd92b0bde77a4cb7f8c94c8aa"),
            sources: &[
                RuntimeSource {
                    region: "intl",
                    url: "https://developer.download.nvidia.com/compute/cuda/redist/libcublas/windows-x86_64/libcublas-windows-x86_64-12.8.4.1-archive.zip",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://developer.download.nvidia.cn/compute/cuda/redist/libcublas/windows-x86_64/libcublas-windows-x86_64-12.8.4.1-archive.zip",
                },
            ],
            files: &[
                RuntimeFile {
                    inner: "libcublas-windows-x86_64-12.8.4.1-archive/bin/cublas64_12.dll",
                    dest: "cublas64_12.dll",
                },
                RuntimeFile {
                    inner: "libcublas-windows-x86_64-12.8.4.1-archive/bin/cublasLt64_12.dll",
                    dest: "cublasLt64_12.dll",
                },
                RuntimeFile {
                    inner: "libcublas-windows-x86_64-12.8.4.1-archive/bin/nvblas64_12.dll",
                    dest: "nvblas64_12.dll",
                },
            ],
        }],
    },
    RuntimeAsset {
        id: RuntimeId::Cudnn,
        os: "windows",
        arch: "x86_64",
        label: "cuDNN 9（CUDA 12）",
        note: "NVIDIA 深度神经网络库；ORT 1.30 的 CUDA 版需要 cuDNN 9，切换 CUDA 时自动补齐",
        required: &[],
        bundled: false,
        parts: &[RuntimePart {
            archive: "cudnn-windows-x86_64-9.27.0.42_cuda12-archive.zip",
            bytes: 1_924_314_965,
            pick_bytes: None,
            disk_bytes: 1_500_000_000,
            sha256: Some("93949680d6499ad0f17fc1abb131825c9dd41b460844513f608e840e0c8ce72b"),
            sources: &[
                RuntimeSource {
                    region: "intl",
                    url: "https://developer.download.nvidia.com/compute/cudnn/redist/cudnn/windows-x86_64/cudnn-windows-x86_64-9.27.0.42_cuda12-archive.zip",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://developer.download.nvidia.cn/compute/cudnn/redist/cudnn/windows-x86_64/cudnn-windows-x86_64-9.27.0.42_cuda12-archive.zip",
                },
            ],
            files: &[
                RuntimeFile {
                    inner: "cudnn-windows-x86_64-9.27.0.42_cuda12-archive/bin/x64/cudnn64_9.dll",
                    dest: "cudnn64_9.dll",
                },
                RuntimeFile {
                    inner: "cudnn-windows-x86_64-9.27.0.42_cuda12-archive/bin/x64/cudnn_adv64_9.dll",
                    dest: "cudnn_adv64_9.dll",
                },
                RuntimeFile {
                    inner: "cudnn-windows-x86_64-9.27.0.42_cuda12-archive/bin/x64/cudnn_cnn64_9.dll",
                    dest: "cudnn_cnn64_9.dll",
                },
                RuntimeFile {
                    inner: "cudnn-windows-x86_64-9.27.0.42_cuda12-archive/bin/x64/cudnn_engines_precompiled64_9.dll",
                    dest: "cudnn_engines_precompiled64_9.dll",
                },
                RuntimeFile {
                    inner: "cudnn-windows-x86_64-9.27.0.42_cuda12-archive/bin/x64/cudnn_engines_runtime_compiled64_9.dll",
                    dest: "cudnn_engines_runtime_compiled64_9.dll",
                },
                RuntimeFile {
                    inner: "cudnn-windows-x86_64-9.27.0.42_cuda12-archive/bin/x64/cudnn_engines_tensor_ir64_9.dll",
                    dest: "cudnn_engines_tensor_ir64_9.dll",
                },
                RuntimeFile {
                    inner: "cudnn-windows-x86_64-9.27.0.42_cuda12-archive/bin/x64/cudnn_ext64_9.dll",
                    dest: "cudnn_ext64_9.dll",
                },
                RuntimeFile {
                    inner: "cudnn-windows-x86_64-9.27.0.42_cuda12-archive/bin/x64/cudnn_graph64_9.dll",
                    dest: "cudnn_graph64_9.dll",
                },
                RuntimeFile {
                    inner: "cudnn-windows-x86_64-9.27.0.42_cuda12-archive/bin/x64/cudnn_heuristic64_9.dll",
                    dest: "cudnn_heuristic64_9.dll",
                },
                RuntimeFile {
                    inner: "cudnn-windows-x86_64-9.27.0.42_cuda12-archive/bin/x64/cudnn_ops64_9.dll",
                    dest: "cudnn_ops64_9.dll",
                },
            ],
        }],
    },
    RuntimeAsset {
        id: RuntimeId::Cufft,
        os: "windows",
        arch: "x86_64",
        label: "cuFFT 11（CUDA 12）",
        note: "CUDA 12 的 FFT 库；ORT 的 CUDA provider 用它做频谱类算子，切换 CUDA 时自动补齐",
        required: &[],
        bundled: false,
        parts: &[RuntimePart {
            archive: "libcufft-windows-x86_64-11.3.3.83-archive.zip",
            bytes: 190_568_498,
            pick_bytes: Some(276_121_600),
            disk_bytes: 276_200_000,
            sha256: None,
            sources: &[
                RuntimeSource {
                    region: "intl",
                    url: "https://developer.download.nvidia.com/compute/cuda/redist/libcufft/windows-x86_64/libcufft-windows-x86_64-11.3.3.83-archive.zip",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://developer.download.nvidia.cn/compute/cuda/redist/libcufft/windows-x86_64/libcufft-windows-x86_64-11.3.3.83-archive.zip",
                },
            ],
            files: &[RuntimeFile {
                inner: "libcufft-windows-x86_64-11.3.3.83-archive/bin/cufft64_11.dll",
                dest: "cufft64_11.dll",
            }],
        }],
    },
    // ---- CUDA 13 组件（配 ORT 1.31 的 CUDA 13 构建；与 CUDA 12 那套并存，互不干扰） ----
    RuntimeAsset {
        id: RuntimeId::Cudart13,
        os: "windows",
        arch: "x86_64",
        label: "CUDA Runtime 13 (cudart)",
        note: "CUDA 13 运行时；选 CUDA 13 后端时自动安装",
        required: &[],
        bundled: false,
        parts: &[RuntimePart {
            archive: "cuda_cudart-windows-x86_64-13.0.96-archive.zip",
            bytes: 2_873_447,
            pick_bytes: None,
            disk_bytes: 600_000,
            sha256: Some("a2ed875f9997aa24904fb70cc9db3acd9308433cde99bc8e63ec1271c9da31b4"),
            sources: &[
                RuntimeSource {
                    region: "intl",
                    url: "https://developer.download.nvidia.com/compute/cuda/redist/cuda_cudart/windows-x86_64/cuda_cudart-windows-x86_64-13.0.96-archive.zip",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://developer.download.nvidia.cn/compute/cuda/redist/cuda_cudart/windows-x86_64/cuda_cudart-windows-x86_64-13.0.96-archive.zip",
                },
            ],
            files: &[RuntimeFile {
                inner: "cuda_cudart-windows-x86_64-13.0.96-archive/bin/x64/cudart64_13.dll",
                dest: "cudart64_13.dll",
            }],
        }],
    },
    RuntimeAsset {
        id: RuntimeId::Cublas13,
        os: "windows",
        arch: "x86_64",
        label: "cuBLAS 13",
        note: "CUDA 13 的矩阵运算库；选 CUDA 13 后端时自动安装",
        required: &[],
        bundled: false,
        parts: &[RuntimePart {
            archive: "libcublas-windows-x86_64-13.0.2.14-archive.zip",
            bytes: 401_229_263,
            pick_bytes: None,
            disk_bytes: 460_000_000,
            sha256: Some("b03fd06fb14fa33e27fc441433fe74ccee9738b62c62a08d39960cb0ce9f1d14"),
            sources: &[
                RuntimeSource {
                    region: "intl",
                    url: "https://developer.download.nvidia.com/compute/cuda/redist/libcublas/windows-x86_64/libcublas-windows-x86_64-13.0.2.14-archive.zip",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://developer.download.nvidia.cn/compute/cuda/redist/libcublas/windows-x86_64/libcublas-windows-x86_64-13.0.2.14-archive.zip",
                },
            ],
            files: &[
                RuntimeFile {
                    inner: "libcublas-windows-x86_64-13.0.2.14-archive/bin/x64/cublas64_13.dll",
                    dest: "cublas64_13.dll",
                },
                RuntimeFile {
                    inner: "libcublas-windows-x86_64-13.0.2.14-archive/bin/x64/cublasLt64_13.dll",
                    dest: "cublasLt64_13.dll",
                },
            ],
        }],
    },
    RuntimeAsset {
        id: RuntimeId::Cufft13,
        os: "windows",
        arch: "x86_64",
        label: "cuFFT 12（CUDA 13）",
        note: "CUDA 13 的 FFT 库（`cufft64_12.dll`）；选 CUDA 13 后端时自动安装",
        required: &[],
        bundled: false,
        parts: &[RuntimePart {
            archive: "libcufft-windows-x86_64-12.0.0.61-archive.zip",
            bytes: 212_026_245,
            pick_bytes: None,
            disk_bytes: 300_000_000,
            sha256: Some("4f2695ea50f895618316ee32636f5d0e5e0bc330e74b3e0876bff002af9e1eb3"),
            sources: &[
                RuntimeSource {
                    region: "intl",
                    url: "https://developer.download.nvidia.com/compute/cuda/redist/libcufft/windows-x86_64/libcufft-windows-x86_64-12.0.0.61-archive.zip",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://developer.download.nvidia.cn/compute/cuda/redist/libcufft/windows-x86_64/libcufft-windows-x86_64-12.0.0.61-archive.zip",
                },
            ],
            files: &[RuntimeFile {
                inner: "libcufft-windows-x86_64-12.0.0.61-archive/bin/x64/cufft64_12.dll",
                dest: "cufft64_12.dll",
            }],
        }],
    },
    RuntimeAsset {
        id: RuntimeId::Cudnn13,
        os: "windows",
        arch: "x86_64",
        label: "cuDNN 9（CUDA 13）",
        note: "CUDA 13 的深度神经网络库；选 CUDA 13 后端时自动安装",
        required: &[],
        bundled: false,
        parts: &[RuntimePart {
            archive: "cudnn-windows-x86_64-9.27.0.42_cuda13-archive.zip",
            bytes: 1_330_803_635,
            pick_bytes: None,
            disk_bytes: 1_600_000_000,
            sha256: Some("15595c7b52896262a0e590f02b3b3851c6a2c9046a72047526deaf59a9cbf7d5"),
            sources: &[
                RuntimeSource {
                    region: "intl",
                    url: "https://developer.download.nvidia.com/compute/cudnn/redist/cudnn/windows-x86_64/cudnn-windows-x86_64-9.27.0.42_cuda13-archive.zip",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://developer.download.nvidia.cn/compute/cudnn/redist/cudnn/windows-x86_64/cudnn-windows-x86_64-9.27.0.42_cuda13-archive.zip",
                },
            ],
            files: &[
                RuntimeFile {
                    inner: "cudnn-windows-x86_64-9.27.0.42_cuda13-archive/bin/x64/cudnn64_9.dll",
                    dest: "cudnn64_9.dll",
                },
                RuntimeFile {
                    inner: "cudnn-windows-x86_64-9.27.0.42_cuda13-archive/bin/x64/cudnn_adv64_9.dll",
                    dest: "cudnn_adv64_9.dll",
                },
                RuntimeFile {
                    inner: "cudnn-windows-x86_64-9.27.0.42_cuda13-archive/bin/x64/cudnn_cnn64_9.dll",
                    dest: "cudnn_cnn64_9.dll",
                },
                RuntimeFile {
                    inner: "cudnn-windows-x86_64-9.27.0.42_cuda13-archive/bin/x64/cudnn_engines_precompiled64_9.dll",
                    dest: "cudnn_engines_precompiled64_9.dll",
                },
                RuntimeFile {
                    inner: "cudnn-windows-x86_64-9.27.0.42_cuda13-archive/bin/x64/cudnn_engines_runtime_compiled64_9.dll",
                    dest: "cudnn_engines_runtime_compiled64_9.dll",
                },
                RuntimeFile {
                    inner: "cudnn-windows-x86_64-9.27.0.42_cuda13-archive/bin/x64/cudnn_engines_tensor_ir64_9.dll",
                    dest: "cudnn_engines_tensor_ir64_9.dll",
                },
                RuntimeFile {
                    inner: "cudnn-windows-x86_64-9.27.0.42_cuda13-archive/bin/x64/cudnn_ext64_9.dll",
                    dest: "cudnn_ext64_9.dll",
                },
                RuntimeFile {
                    inner: "cudnn-windows-x86_64-9.27.0.42_cuda13-archive/bin/x64/cudnn_graph64_9.dll",
                    dest: "cudnn_graph64_9.dll",
                },
                RuntimeFile {
                    inner: "cudnn-windows-x86_64-9.27.0.42_cuda13-archive/bin/x64/cudnn_heuristic64_9.dll",
                    dest: "cudnn_heuristic64_9.dll",
                },
                RuntimeFile {
                    inner: "cudnn-windows-x86_64-9.27.0.42_cuda13-archive/bin/x64/cudnn_ops64_9.dll",
                    dest: "cudnn_ops64_9.dll",
                },
            ],
        }],
    },
    RuntimeAsset {
        id: RuntimeId::DirectMl,
        os: "windows",
        arch: "x86_64",
        label: "ONNX Runtime（DirectML）",
        note: "任意 DX12 显卡；ORT 只为它发布到 1.24，功能受限",
        // 随包：dylib + DirectML.dll 都在才算数。
        required: &["onnxruntime.dll", "DirectML.dll"],
        bundled: true,
        parts: &[
            RuntimePart {
                archive: "microsoft.ml.onnxruntime.directml.1.24.4.nupkg",
                bytes: 12_458_649,
                pick_bytes: Some(6_051_509),
                disk_bytes: 17_350_192,
                sha256: None,
                sources: &[RuntimeSource {
                    region: "intl",
                    url: "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime.directml/1.24.4/microsoft.ml.onnxruntime.directml.1.24.4.nupkg",
                }],
                files: &[
                    RuntimeFile {
                        inner: "runtimes/win-x64/native/onnxruntime.dll",
                        dest: "onnxruntime.dll",
                    },
                    RuntimeFile {
                        inner: "runtimes/win-x64/native/onnxruntime_providers_shared.dll",
                        dest: "onnxruntime_providers_shared.dll",
                    },
                ],
            },
            RuntimePart {
                archive: "microsoft.ai.directml.1.15.4.nupkg",
                bytes: 202_292_617,
                pick_bytes: Some(9_332_741),
                disk_bytes: 18_527_776,
                sha256: None,
                sources: &[RuntimeSource {
                    region: "intl",
                    url: "https://api.nuget.org/v3-flatcontainer/microsoft.ai.directml/1.15.4/microsoft.ai.directml.1.15.4.nupkg",
                }],
                files: &[RuntimeFile {
                    inner: "bin/x64-win/DirectML.dll",
                    dest: "DirectML.dll",
                }],
            },
        ],
    },
];

static LINUX: &[RuntimeAsset] = &[
    RuntimeAsset {
        id: RuntimeId::Cpu,
        os: "linux",
        arch: "x86_64",
        label: "ONNX Runtime（CPU）",
        note: "纯 CPU，默认",
        required: &["libonnxruntime.so.1.30.0"],
        bundled: true,
        parts: &[RuntimePart {
            archive: "onnxruntime-linux-x64-1.30.0.tgz",
            bytes: 11_306_877,
            pick_bytes: None,
            disk_bytes: 11_306_877,
            sha256: None,
            sources: &[RuntimeSource {
                region: "intl",
                url: "https://github.com/microsoft/onnxruntime/releases/download/v1.30.0/onnxruntime-linux-x64-1.30.0.tgz",
            }],
            files: &[RuntimeFile {
                inner: "onnxruntime-linux-x64-1.30.0/lib/libonnxruntime.so.1.30.0",
                dest: "libonnxruntime.so.1.30.0",
            }],
        }],
    },
    RuntimeAsset {
        id: RuntimeId::Cuda,
        os: "linux",
        arch: "x86_64",
        label: "ONNX Runtime（CUDA 12）",
        // 和 Windows 一样钉 1.26：1.30+ 的 Linux GPU 包已经是 CUDA 13 构建
        // （provider 的 DT_NEEDED 是 libcublas.so.13 / libcudart.so.13），
        // 只有 1.26 才是 .so.12，能和下面那套 CUDA 12 组件对上。版本是「CUDA 大版本」
        // 决定的，不是「别的系统的源」。
        note: "NVIDIA 显卡加速（Linux，CUDA 12）；cudart / cuBLAS / cuFFT / cuDNN 9 可在这里一起下载",
        required: &[],
        bundled: false,
        parts: &[RuntimePart {
            archive: "microsoft.ml.onnxruntime.gpu.linux.1.26.0.nupkg",
            bytes: 225_183_750,
            pick_bytes: None,
            disk_bytes: 370_319_192,
            sha256: Some("cf7943f58b9360fe1448c4baf40ac12ce4175978ecd3fe6f21a4c00976f63a59"),
            sources: &[
                RuntimeSource {
                    region: "intl",
                    url: "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime.gpu.linux/1.26.0/microsoft.ml.onnxruntime.gpu.linux.1.26.0.nupkg",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://mirrors.huaweicloud.com/artifactory/api/nuget/v3/nuget-remote/microsoft.ml.onnxruntime.gpu.linux/1.26.0/microsoft.ml.onnxruntime.gpu.linux.1.26.0.nupkg",
                },
            ],
            files: &[
                RuntimeFile {
                    inner: "runtimes/linux-x64/native/libonnxruntime.so",
                    dest: "libonnxruntime.so",
                },
                RuntimeFile {
                    inner: "runtimes/linux-x64/native/libonnxruntime_providers_cuda.so",
                    dest: "libonnxruntime_providers_cuda.so",
                },
                RuntimeFile {
                    inner: "runtimes/linux-x64/native/libonnxruntime_providers_shared.so",
                    dest: "libonnxruntime_providers_shared.so",
                },
            ],
        }],
    },
    RuntimeAsset {
        id: RuntimeId::Cuda13,
        os: "linux",
        arch: "x86_64",
        label: "ONNX Runtime（CUDA 13）",
        note: "NVIDIA 新卡加速（Linux，CUDA 13 / ORT 1.31）；配套 cudart 13 / cuBLAS 13 / cuFFT 12 / cuDNN 9 会一起装",
        required: &[],
        bundled: false,
        parts: &[RuntimePart {
            archive: "microsoft.ml.onnxruntime.gpu.linux.1.31.1.nupkg",
            bytes: 244_926_281,
            pick_bytes: None,
            disk_bytes: 317_102_464,
            sha256: Some("cfb5315b87f812d8d91194075eb4aab3cbfd97a932f2b56c9bfa41e36087c4a2"),
            sources: &[
                RuntimeSource {
                    region: "intl",
                    url: "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime.gpu.linux/1.31.1/microsoft.ml.onnxruntime.gpu.linux.1.31.1.nupkg",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://mirrors.huaweicloud.com/artifactory/api/nuget/v3/nuget-remote/microsoft.ml.onnxruntime.gpu.linux/1.31.1/microsoft.ml.onnxruntime.gpu.linux.1.31.1.nupkg",
                },
            ],
            files: &[
                RuntimeFile {
                    inner: "runtimes/linux-x64/native/libonnxruntime.so",
                    dest: "libonnxruntime.so",
                },
                RuntimeFile {
                    inner: "runtimes/linux-x64/native/libonnxruntime_providers_cuda.so",
                    dest: "libonnxruntime_providers_cuda.so",
                },
                RuntimeFile {
                    inner: "runtimes/linux-x64/native/libonnxruntime_providers_shared.so",
                    dest: "libonnxruntime_providers_shared.so",
                },
            ],
        }],
    },
    // Linux 的 CUDA 组件：以前只靠 `CUDA_PATH` / pip 里的系统库，现在和 Windows 一样
    // 可以整套下载（NVIDIA redist 只发 `.tar.xz`，解压走 `xz2`）。装到 `runtime/ort/<id>/`，
    // `preload_cuda` 会把这几个目录里的 `.so` 按 SONAME 预加载进进程。
    RuntimeAsset {
        id: RuntimeId::Cudart,
        os: "linux",
        arch: "x86_64",
        label: "CUDA Runtime 12 (cudart)",
        note: "CUDA 12 运行时（Linux）；切换 CUDA 时自动补齐",
        required: &[],
        bundled: false,
        parts: &[RuntimePart {
            archive: "cuda_cudart-linux-x86_64-12.8.90-archive.tar.xz",
            bytes: 1_354_240,
            pick_bytes: None,
            disk_bytes: 700_000,
            sha256: Some("8d566b5fe745c46842dc16945cf36686227536decd2302c372be86da37faca68"),
            sources: &[
                RuntimeSource {
                    region: "intl",
                    url: "https://developer.download.nvidia.com/compute/cuda/redist/cuda_cudart/linux-x86_64/cuda_cudart-linux-x86_64-12.8.90-archive.tar.xz",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://developer.download.nvidia.cn/compute/cuda/redist/cuda_cudart/linux-x86_64/cuda_cudart-linux-x86_64-12.8.90-archive.tar.xz",
                },
            ],
            // 落到 SONAME：ORT 的 `CUDA_DYLIBS` 按 `libcudart.so.12` 这个名字找，
            // 归档里那个 `libcudart.so.12` 是符号链接（没有内容），要取实体再改名。
            files: &[RuntimeFile {
                inner: "cuda_cudart-linux-x86_64-12.8.90-archive/lib/libcudart.so.12.8.90",
                dest: "libcudart.so.12",
            }],
        }],
    },
    RuntimeAsset {
        id: RuntimeId::Cublas,
        os: "linux",
        arch: "x86_64",
        label: "cuBLAS 12",
        note: "CUDA 12 的矩阵运算库（Linux）；切换 CUDA 时自动补齐",
        required: &[],
        bundled: false,
        parts: &[RuntimePart {
            archive: "libcublas-linux-x86_64-12.8.4.1-archive.tar.xz",
            bytes: 938_651_324,
            pick_bytes: None,
            disk_bytes: 800_000_000,
            sha256: Some("21718957c2cf000bacd69d36c95708a2319199e39e056f8b4f0f68e3b9f323bb"),
            sources: &[
                RuntimeSource {
                    region: "intl",
                    url: "https://developer.download.nvidia.com/compute/cuda/redist/libcublas/linux-x86_64/libcublas-linux-x86_64-12.8.4.1-archive.tar.xz",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://developer.download.nvidia.cn/compute/cuda/redist/libcublas/linux-x86_64/libcublas-linux-x86_64-12.8.4.1-archive.tar.xz",
                },
            ],
            files: &[
                RuntimeFile {
                    inner: "libcublas-linux-x86_64-12.8.4.1-archive/lib/libcublas.so.12.8.4.1",
                    dest: "libcublas.so.12",
                },
                RuntimeFile {
                    inner: "libcublas-linux-x86_64-12.8.4.1-archive/lib/libcublasLt.so.12.8.4.1",
                    dest: "libcublasLt.so.12",
                },
            ],
        }],
    },
    RuntimeAsset {
        id: RuntimeId::Cufft,
        os: "linux",
        arch: "x86_64",
        label: "cuFFT 11（CUDA 12）",
        note: "CUDA 12 的 FFT 库（Linux）；ORT 的 CUDA provider 用它做频谱类算子",
        required: &[],
        bundled: false,
        parts: &[RuntimePart {
            archive: "libcufft-linux-x86_64-11.3.3.83-archive.tar.xz",
            bytes: 448_695_100,
            pick_bytes: None,
            disk_bytes: 400_000_000,
            sha256: Some("126fe08fc0df07a7420c2da05a2ae72ea19660fff79bf06de16520d1f915cb83"),
            sources: &[
                RuntimeSource {
                    region: "intl",
                    url: "https://developer.download.nvidia.com/compute/cuda/redist/libcufft/linux-x86_64/libcufft-linux-x86_64-11.3.3.83-archive.tar.xz",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://developer.download.nvidia.cn/compute/cuda/redist/libcufft/linux-x86_64/libcufft-linux-x86_64-11.3.3.83-archive.tar.xz",
                },
            ],
            files: &[RuntimeFile {
                inner: "libcufft-linux-x86_64-11.3.3.83-archive/lib/libcufft.so.11.3.3.83",
                dest: "libcufft.so.11",
            }],
        }],
    },
    RuntimeAsset {
        id: RuntimeId::Cudnn,
        os: "linux",
        arch: "x86_64",
        label: "cuDNN 9（CUDA 12）",
        note: "NVIDIA 深度神经网络库（Linux）；cudnn 9 把实现拆成十个 .so，全都要",
        required: &[],
        bundled: false,
        parts: &[RuntimePart {
            archive: "cudnn-linux-x86_64-9.27.0.42_cuda12-archive.tar.xz",
            bytes: 1_012_666_264,
            pick_bytes: None,
            disk_bytes: 1_500_000_000,
            sha256: Some("dc45f8c5fd44955c6e950c690ea92c28ea848ffb1fefbaabf3a9d175d961263a"),
            sources: &[
                RuntimeSource {
                    region: "intl",
                    url: "https://developer.download.nvidia.com/compute/cudnn/redist/cudnn/linux-x86_64/cudnn-linux-x86_64-9.27.0.42_cuda12-archive.tar.xz",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://developer.download.nvidia.cn/compute/cudnn/redist/cudnn/linux-x86_64/cudnn-linux-x86_64-9.27.0.42_cuda12-archive.tar.xz",
                },
            ],
            files: &[
                RuntimeFile {
                    inner: "cudnn-linux-x86_64-9.27.0.42_cuda12-archive/lib/libcudnn.so.9.27.0",
                    dest: "libcudnn.so.9",
                },
                RuntimeFile {
                    inner: "cudnn-linux-x86_64-9.27.0.42_cuda12-archive/lib/libcudnn_adv.so.9.27.0",
                    dest: "libcudnn_adv.so.9",
                },
                RuntimeFile {
                    inner: "cudnn-linux-x86_64-9.27.0.42_cuda12-archive/lib/libcudnn_cnn.so.9.27.0",
                    dest: "libcudnn_cnn.so.9",
                },
                RuntimeFile {
                    inner: "cudnn-linux-x86_64-9.27.0.42_cuda12-archive/lib/libcudnn_engines_precompiled.so.9.27.0",
                    dest: "libcudnn_engines_precompiled.so.9",
                },
                RuntimeFile {
                    inner: "cudnn-linux-x86_64-9.27.0.42_cuda12-archive/lib/libcudnn_engines_runtime_compiled.so.9.27.0",
                    dest: "libcudnn_engines_runtime_compiled.so.9",
                },
                RuntimeFile {
                    inner: "cudnn-linux-x86_64-9.27.0.42_cuda12-archive/lib/libcudnn_engines_tensor_ir.so.9.27.0",
                    dest: "libcudnn_engines_tensor_ir.so.9",
                },
                RuntimeFile {
                    inner: "cudnn-linux-x86_64-9.27.0.42_cuda12-archive/lib/libcudnn_ext.so.9.27.0",
                    dest: "libcudnn_ext.so.9",
                },
                RuntimeFile {
                    inner: "cudnn-linux-x86_64-9.27.0.42_cuda12-archive/lib/libcudnn_graph.so.9.27.0",
                    dest: "libcudnn_graph.so.9",
                },
                RuntimeFile {
                    inner: "cudnn-linux-x86_64-9.27.0.42_cuda12-archive/lib/libcudnn_heuristic.so.9.27.0",
                    dest: "libcudnn_heuristic.so.9",
                },
                RuntimeFile {
                    inner: "cudnn-linux-x86_64-9.27.0.42_cuda12-archive/lib/libcudnn_ops.so.9.27.0",
                    dest: "libcudnn_ops.so.9",
                },
            ],
        }],
    },
    // ---- CUDA 13 组件（Linux）：与 CUDA 12 那套并存，目录名带 13 后缀 ----
    //
    // 版本与 Windows 那份保持同一套（cudart 13.0.96 / cuBLAS 13.0.2.14 / cuFFT 12.0.0.61 /
    // cuDNN 9.27.0.42_cuda13），哈希取自 NVIDIA redist 的 `redistrib_13.0.1.json` 与
    // `redistrib_9.27.0.json`。落到 SONAME（`libcublas.so.13`）而不是带小版本的文件名：
    // ORT 的 provider 是**按 SONAME dlopen** 的，符号链接在 tar.xz 里没有内容。
    RuntimeAsset {
        id: RuntimeId::Cudart13,
        os: "linux",
        arch: "x86_64",
        label: "CUDA Runtime 13 (cudart)",
        note: "CUDA 13 运行时（Linux）；选 CUDA 13 后端时自动安装",
        required: &[],
        bundled: false,
        parts: &[RuntimePart {
            archive: "cuda_cudart-linux-x86_64-13.0.96-archive.tar.xz",
            bytes: 1_487_960,
            pick_bytes: None,
            disk_bytes: 700_000,
            sha256: Some("25b8071951baba827be1580b841d363464f6ee6c39f48d33a81646f90cc95ed1"),
            sources: &[
                RuntimeSource {
                    region: "intl",
                    url: "https://developer.download.nvidia.com/compute/cuda/redist/cuda_cudart/linux-x86_64/cuda_cudart-linux-x86_64-13.0.96-archive.tar.xz",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://developer.download.nvidia.cn/compute/cuda/redist/cuda_cudart/linux-x86_64/cuda_cudart-linux-x86_64-13.0.96-archive.tar.xz",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://developer.download.nvidia.cn/compute/cuda/redist/cuda_cudart/linux-x86_64/cuda_cudart-linux-x86_64-13.0.96-archive.tar.xz",
                },
            ],
            files: &[RuntimeFile {
                inner: "cuda_cudart-linux-x86_64-13.0.96-archive/lib/libcudart.so.13.0.96",
                dest: "libcudart.so.13",
            }],
        }],
    },
    RuntimeAsset {
        id: RuntimeId::Cublas13,
        os: "linux",
        arch: "x86_64",
        label: "cuBLAS 13",
        note: "CUDA 13 的矩阵运算库（Linux）；选 CUDA 13 后端时自动安装",
        required: &[],
        bundled: false,
        parts: &[RuntimePart {
            archive: "libcublas-linux-x86_64-13.0.2.14-archive.tar.xz",
            bytes: 800_056_524,
            pick_bytes: None,
            disk_bytes: 900_000_000,
            sha256: Some("158c1d7e862282d5218ed82f173ba14679c0f8afa1fe8cb4a3a34a4c0a9a5ff9"),
            sources: &[
                RuntimeSource {
                    region: "intl",
                    url: "https://developer.download.nvidia.com/compute/cuda/redist/libcublas/linux-x86_64/libcublas-linux-x86_64-13.0.2.14-archive.tar.xz",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://developer.download.nvidia.cn/compute/cuda/redist/libcublas/linux-x86_64/libcublas-linux-x86_64-13.0.2.14-archive.tar.xz",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://developer.download.nvidia.cn/compute/cuda/redist/libcublas/linux-x86_64/libcublas-linux-x86_64-13.0.2.14-archive.tar.xz",
                },
            ],
            files: &[
                RuntimeFile {
                    inner: "libcublas-linux-x86_64-13.0.2.14-archive/lib/libcublas.so.13.0.2.14",
                    dest: "libcublas.so.13",
                },
                RuntimeFile {
                    inner: "libcublas-linux-x86_64-13.0.2.14-archive/lib/libcublasLt.so.13.0.2.14",
                    dest: "libcublasLt.so.13",
                },
            ],
        }],
    },
    RuntimeAsset {
        id: RuntimeId::Cufft13,
        os: "linux",
        arch: "x86_64",
        label: "cuFFT 12（CUDA 13）",
        note: "CUDA 13 的 FFT 库（Linux，`libcufft.so.12`）；选 CUDA 13 后端时自动安装",
        required: &[],
        bundled: false,
        parts: &[RuntimePart {
            archive: "libcufft-linux-x86_64-12.0.0.61-archive.tar.xz",
            bytes: 354_634_628,
            pick_bytes: None,
            disk_bytes: 400_000_000,
            sha256: Some("5cdef1238c270f8f148da18587fdba8b9478c596e40f5490bba2b15c94915dd9"),
            sources: &[
                RuntimeSource {
                    region: "intl",
                    url: "https://developer.download.nvidia.com/compute/cuda/redist/libcufft/linux-x86_64/libcufft-linux-x86_64-12.0.0.61-archive.tar.xz",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://developer.download.nvidia.cn/compute/cuda/redist/libcufft/linux-x86_64/libcufft-linux-x86_64-12.0.0.61-archive.tar.xz",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://developer.download.nvidia.cn/compute/cuda/redist/libcufft/linux-x86_64/libcufft-linux-x86_64-12.0.0.61-archive.tar.xz",
                },
            ],
            files: &[RuntimeFile {
                inner: "libcufft-linux-x86_64-12.0.0.61-archive/lib/libcufft.so.12.0.0.61",
                dest: "libcufft.so.12",
            }],
        }],
    },
    RuntimeAsset {
        id: RuntimeId::Cudnn13,
        os: "linux",
        arch: "x86_64",
        label: "cuDNN 9（CUDA 13）",
        note: "CUDA 13 的深度神经网络库（Linux）；选 CUDA 13 后端时自动安装",
        required: &[],
        bundled: false,
        parts: &[RuntimePart {
            archive: "cudnn-linux-x86_64-9.27.0.42_cuda13-archive.tar.xz",
            bytes: 937_996_620,
            pick_bytes: None,
            disk_bytes: 1_500_000_000,
            sha256: Some("dd3245ba3aca8593934fc946d08649ad3a5ee8500e8da3870bc23bf7ddccd7f9"),
            sources: &[
                RuntimeSource {
                    region: "intl",
                    url: "https://developer.download.nvidia.com/compute/cudnn/redist/cudnn/linux-x86_64/cudnn-linux-x86_64-9.27.0.42_cuda13-archive.tar.xz",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://developer.download.nvidia.cn/compute/cudnn/redist/cudnn/linux-x86_64/cudnn-linux-x86_64-9.27.0.42_cuda13-archive.tar.xz",
                },
                RuntimeSource {
                    region: "cn",
                    url: "https://developer.download.nvidia.cn/compute/cudnn/redist/cudnn/linux-x86_64/cudnn-linux-x86_64-9.27.0.42_cuda13-archive.tar.xz",
                },
            ],
            files: &[
                RuntimeFile {
                    inner: "cudnn-linux-x86_64-9.27.0.42_cuda13-archive/lib/libcudnn.so.9.27.0",
                    dest: "libcudnn.so.9",
                },
                RuntimeFile {
                    inner: "cudnn-linux-x86_64-9.27.0.42_cuda13-archive/lib/libcudnn_adv.so.9.27.0",
                    dest: "libcudnn_adv.so.9",
                },
                RuntimeFile {
                    inner: "cudnn-linux-x86_64-9.27.0.42_cuda13-archive/lib/libcudnn_cnn.so.9.27.0",
                    dest: "libcudnn_cnn.so.9",
                },
                RuntimeFile {
                    inner: "cudnn-linux-x86_64-9.27.0.42_cuda13-archive/lib/libcudnn_engines_precompiled.so.9.27.0",
                    dest: "libcudnn_engines_precompiled.so.9",
                },
                RuntimeFile {
                    inner: "cudnn-linux-x86_64-9.27.0.42_cuda13-archive/lib/libcudnn_engines_runtime_compiled.so.9.27.0",
                    dest: "libcudnn_engines_runtime_compiled.so.9",
                },
                RuntimeFile {
                    inner: "cudnn-linux-x86_64-9.27.0.42_cuda13-archive/lib/libcudnn_engines_tensor_ir.so.9.27.0",
                    dest: "libcudnn_engines_tensor_ir.so.9",
                },
                RuntimeFile {
                    inner: "cudnn-linux-x86_64-9.27.0.42_cuda13-archive/lib/libcudnn_ext.so.9.27.0",
                    dest: "libcudnn_ext.so.9",
                },
                RuntimeFile {
                    inner: "cudnn-linux-x86_64-9.27.0.42_cuda13-archive/lib/libcudnn_graph.so.9.27.0",
                    dest: "libcudnn_graph.so.9",
                },
                RuntimeFile {
                    inner: "cudnn-linux-x86_64-9.27.0.42_cuda13-archive/lib/libcudnn_heuristic.so.9.27.0",
                    dest: "libcudnn_heuristic.so.9",
                },
                RuntimeFile {
                    inner: "cudnn-linux-x86_64-9.27.0.42_cuda13-archive/lib/libcudnn_ops.so.9.27.0",
                    dest: "libcudnn_ops.so.9",
                },
            ],
        }],
    },
];

/// 本机的全部运行时条目。
///
/// 两张表**始终参与编译**（只是数据），这样 Linux 那份清单在 Windows 上也能被类型检查和
/// 测试覆盖——按 `cfg` 把整张表编掉的话，写错一个文件名要到 Linux 上才发现。
pub fn for_platform(os: &str, arch: &str) -> Vec<&'static RuntimeAsset> {
    match (os, arch) {
        ("windows", "x86_64") => WINDOWS.iter().collect(),
        ("linux", "x86_64") => LINUX.iter().collect(),
        // macOS：静态链接，没有运行时可管理。
        _ => Vec::new(),
    }
}

/// 本机的某个运行时条目。
pub fn asset(id: RuntimeId) -> Option<&'static RuntimeAsset> {
    for_platform(std::env::consts::OS, std::env::consts::ARCH)
        .into_iter()
        .find(|a| a.id == id)
}

/// 后端种类 → 需要的运行时。macOS 上 CPU 与 CoreML 共用同一份 dylib。
///
/// CUDA 12 / 13 现在是**两个显式后端**，用户选哪个就是哪个，不再按显卡猜：
/// 猜错时 ORT 只会静默回落 CPU，而用户明明在设置页选了 CUDA 13。
pub fn for_backend(kind: super::BackendKind) -> RuntimeId {
    use super::BackendKind;
    match kind {
        BackendKind::Cuda12 => RuntimeId::Cuda,
        BackendKind::Cuda13 => RuntimeId::Cuda13,
        BackendKind::DirectMl => RuntimeId::DirectMl,
        BackendKind::CoreMl => RuntimeId::CoreMl,
        BackendKind::Cpu => {
            #[cfg(target_os = "macos")]
            {
                RuntimeId::CoreMl
            }
            #[cfg(not(target_os = "macos"))]
            {
                RuntimeId::Cpu
            }
        }
    }
}

/// 本机是否适合 CUDA 13：Oracle 是「驱动 ≥ 580 且计算能力 ≥ 7.5」。
///
/// CUDA 13 不再支持 Maxwell（5.x）/ Pascal（6.x）/ Volta（7.0）；Turing（7.5）起才行。
/// 驱动版本读不到就算「不支持」（保守留在 CUDA 12，那套在 580+ 驱动上照样能跑）。
pub fn cuda13_preferred() -> bool {
    cuda13_supported(&super::detect::detect_cached())
}

/// 纯函数版本（单测用）。
pub fn cuda13_supported(profile: &super::detect::HardwareProfile) -> bool {
    if profile.gpu != Some(super::detect::GpuVendor::Nvidia) {
        return false;
    }
    let driver_ok = profile
        .driver
        .as_deref()
        .and_then(|d| d.split('.').next())
        .and_then(|major| major.parse::<u32>().ok())
        .map(|major| major >= 580)
        .unwrap_or(false);
    if !driver_ok {
        return false;
    }
    // 读不到计算能力时按「支持」处理：N 卡 + 新驱动已经是很强的信号，
    // 真跑不起来也会在 ORT 里报错（而不是悄悄用错内核）。
    profile
        .compute_cap
        .as_deref()
        .and_then(|cap| cap.trim().parse::<f32>().ok())
        .map(|cap| cap >= 7.5)
        .unwrap_or(true)
}

/// 某个核心运行时在本机平台上的**组件**（CUDA 的 cudart / cuBLAS / cuFFT / cuDNN）。
///
/// 组件从属于 core：CUDA 缺任何一个，`onnxruntime_providers_cuda.dll` 都载不进来，
/// ORT 随后**静默**把会话建在 CPU 上。所以「能不能用 CUDA」必须把 core 与组件一起看，
/// 状态页也据此告诉用户还缺哪几项。
pub fn components_of(core: RuntimeId) -> Vec<RuntimeId> {
    // 目前只有 CUDA 有组件（DirectML / CoreML / CPU 都是单份 dylib）。
    // CUDA 12 与 13 各有一套，按 core 的大版本过滤，别把两套混着装。
    if !core.is_cuda() {
        return Vec::new();
    }
    let flavor = core.cuda_flavor();
    for_platform(std::env::consts::OS, std::env::consts::ARCH)
        .into_iter()
        .filter(|asset| asset.id.is_component() && asset.id.cuda_flavor() == flavor)
        .map(|asset| asset.id)
        .collect()
}

// ---------------------------------------------------------------------------
// 安装目录与状态
// ---------------------------------------------------------------------------

static ROOT: OnceLock<PathBuf> = OnceLock::new();
static BUNDLED: OnceLock<PathBuf> = OnceLock::new();
static CHOSEN: AtomicU8 = AtomicU8::new(u8::MAX);

/// 宿主启动时设定运行时根目录（通常是应用配置目录下的 `runtime/ort/`）。
pub fn set_root(dir: PathBuf) {
    let _ = ROOT.set(dir);
}

/// 运行时根目录（宿主没设过就是 `<临时目录>/ai-daw-runtime/ort`）。
pub fn root() -> PathBuf {
    ROOT.get()
        .cloned()
        .unwrap_or_else(|| std::env::temp_dir().join("ai-daw-runtime").join("ort"))
}

/// 某项运行时的安装目录。
pub fn install_dir(id: RuntimeId) -> PathBuf {
    root().join(id.as_str())
}

/// dylib 的完整路径：随包的在资源目录，下载的在应用运行时目录。
///
/// 文件名取清单里第一个 `required`：Linux 的 NuGet 包是 `libonnxruntime.so`，
/// 旧的 GitHub tar.gz 是带版本号的 `libonnxruntime.so.1.30.0`；统一从清单取，别硬拼。
pub fn library_path(id: RuntimeId) -> PathBuf {
    let name = asset(id)
        .and_then(|a| a.required().into_iter().next())
        .unwrap_or_else(library_name);
    if is_bundled(id) {
        if let Some(base) = bundled_root() {
            return base.join(name);
        }
    }
    install_dir(id).join(name)
}

/// 这一项是否随安装包分发。
pub fn is_bundled(id: RuntimeId) -> bool {
    asset(id).map(|a| a.bundled).unwrap_or(false)
}

/// 这一项是否可用：随包的看资源目录，下载的看安装目录。
///
/// **逐文件判定**：清单里要求的每个文件都在才算装好。只认一个 marker 会让「解压到一半」
/// （缺 `providers_cuda.dll` / `cudnn_*` / `cufft64_11.dll`）的目录显示成已安装，直到建
/// 会话才炸、而且静默回落 CPU。
pub fn installed(id: RuntimeId) -> bool {
    let Some(asset) = asset(id) else {
        return false;
    };
    let dir = if asset.bundled {
        match bundled_root() {
            Some(base) => base.to_path_buf(),
            None => return false,
        }
    } else {
        install_dir(id)
    };
    asset.required().iter().all(|name| dir.join(name).is_file())
}

/// 宿主启动时设定随包运行时目录（安装包里的 `ort/`）。
pub fn set_bundled_dir(dir: PathBuf) {
    let _ = BUNDLED.set(dir);
}

/// 随包目录：宿主设的 > `AIDAW_ORT_BUNDLED`（探针 / 开发用）。
fn bundled_root() -> Option<&'static Path> {
    if let Some(dir) = BUNDLED.get() {
        return Some(dir.as_path());
    }
    let env = std::env::var_os("AIDAW_ORT_BUNDLED")?;
    Some(Box::leak(Box::new(PathBuf::from(env))).as_path())
}

/// 宿主设定要用的运行时（保存的偏好）。
pub fn set_runtime(id: RuntimeId) {
    CHOSEN.store(id.as_u8(), Ordering::Relaxed);
}

/// 当前想用的运行时：`AIDAW_ORT_RUNTIME_KIND` 环境变量 > 宿主设定 > CPU。
pub fn chosen() -> RuntimeId {
    if let Some(id) = std::env::var("AIDAW_ORT_RUNTIME_KIND")
        .ok()
        .and_then(|text| RuntimeId::parse(&text))
    {
        return id;
    }
    let raw = CHOSEN.load(Ordering::Relaxed);
    if raw == u8::MAX {
        // 宿主没设过（探针 / example / 库用户）：跟着后端选择走，`AIDAW_BACKEND` 才算数。
        return for_backend(super::backend());
    }
    RuntimeId::from_u8(raw)
}

// ---------------------------------------------------------------------------
// 加载
// ---------------------------------------------------------------------------

/// 加载结果缓存：**只缓存成功**。
///
/// 失败必须可重试——用户去设置页下完运行时之后，同一进程里应该能直接用上
/// （`ort` 的 dylib 本身仍然是进程内一次，装新的一份才需要重启）。
#[cfg(not(target_os = "macos"))]
static LOADED: std::sync::Mutex<Option<Result<PathBuf, String>>> = std::sync::Mutex::new(None);

/// 加载 ONNX Runtime（**进程内一次成功**），返回实际加载的 dylib 路径。
///
/// 搜索顺序：
/// 1. `AIDAW_ORT_RUNTIME`（文件或目录；探针 / 开发用）；
/// 2. 宿主设定的运行时目录里 [`chosen`] 那一项；
/// 3. 本机其它已装好的运行时（CPU 兜底）。
///
/// 失败**不会 panic**：错误被记下来，[`super::session::build`] 会把它变成一句可读的话；
/// 下次调用会重试（用户可能刚下载完）。
/// 已加载的 dylib 目录里的 CUDA provider 路径（`init_from` 之后要注册给 ORT）。
static CUDA_PROVIDER_PATH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// CUDA EP 库是否**真的注册成功**。
///
/// 只有它为 `true` 时建出来的会话才可能在显卡上；否则即便 `commit` 成功也是 CPU。
/// 以前不区分这两件事，日志就会写「→ 加速器」而实际跑在 CPU（假绿灯）。

/// CUDA 执行器库是否已经注册成功（`false` = 后面建的会话只能是 CPU）。
pub fn cuda_ep_ready() -> bool {
    CUDA_EP_FLAVOR.load(std::sync::atomic::Ordering::Relaxed) != 0
}

/// 注册成功的 CUDA provider 是不是指定大版本（12 / 13）。
///
/// 进程里只加载一份 ONNX Runtime：用户从 CUDA 12 切到 13、但还没重启时，
/// provider 仍是 12 的。界面必须据此提示「重启后生效」，而不是给一个假的绿灯。
pub fn cuda_ep_ready_for(flavor: u8) -> bool {
    CUDA_EP_FLAVOR.load(std::sync::atomic::Ordering::Relaxed) == flavor
}

/// 已注册的 CUDA provider 大版本（0 = 没注册）。
static CUDA_EP_FLAVOR: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub fn ensure_loaded() -> Result<PathBuf, String> {
    // macOS 是**静态链接**（没有 CUDA，不需要运行期换 dylib）：运行时代码就在二进制里。
    #[cfg(target_os = "macos")]
    {
        return Ok(PathBuf::new());
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut slot = LOADED.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(Ok(path)) = slot.as_ref() {
            return Ok(path.clone());
        }
        let result = load();
        if result.is_ok() {
            *slot = Some(result.clone());
        }
        result
    }
}

/// 运行时是否已成功加载（建会话前的门禁）。
pub fn loaded() -> bool {
    loaded_path().is_some()
}

/// **已经加载**的那一份 dylib 路径；没加载就 `None`——**不会触发加载**。
///
/// 启动路径上的状态查询（后端信息 / 运行时列表 / 「重启后生效」）只该看这个：
/// CUDA 那份有 180 MB provider，加载 + 初始化显卡要好几秒，不能因为界面要显示一行
/// 状态就把整份运行时拉进进程。
pub fn loaded_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        return None;
    }
    #[cfg(not(target_os = "macos"))]
    {
        LOADED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|r| r.as_ref().ok().cloned())
    }
}

/// 已加载的那份 dylib 是不是 `id` 对应的文件（`None` = 还没加载）。
///
/// 用来发现「运行时在 [`super::init`] 之前就被别的路径加载了」这种顺序问题：
/// 那样偏好是 CUDA 也换不回来（一个进程只能有一份 dylib），只能重启。
pub fn loaded_is(id: RuntimeId) -> Option<bool> {
    loaded_path().map(|loaded| loaded == library_path(id))
}

/// 运行时是否**静态链接**在二进制里（macOS）：没有可下载、可删除的运行时。
pub const fn statically_linked() -> bool {
    cfg!(target_os = "macos")
}
#[cfg(not(target_os = "macos"))]

fn load() -> Result<PathBuf, String> {
    let (id, path) = locate()?;
    // **第一件事**就把变量钉到即将加载的这一份。
    //
    // 原因见下面 `preload_cuda` 的注释：`ort` 有些辅助函数在**构造错误对象**时就会去拿 API，
    // 那一刻 dylib 还没初始化，它只会退回裸文件名 `onnxruntime.dll`——Windows 的系统目录里
    // 自带一份 1.17.1，版本太旧，`expect` 在 `extern` 边界上 panic，整条后台任务线程直接死。
    std::env::set_var("ORT_DYLIB_PATH", &path);
    // CUDA 构建要先把它依赖的 CUDA / cuDNN 库按顺序 dlopen 进来：
    // ORT 的 CUDA EP 在 session 创建时才去解析这些符号，路径不在 PATH 里就会失败。
    //
    // 判断依据是**同一目录里有没有 CUDA provider**，而不是 `id`：探针可能用
    // `AIDAW_ORT_RUNTIME` 直接指到一份 CUDA 构建上。
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    // 运行时目录也进 PATH：ORT 注册 CUDA EP 时按**裸文件名**去找
    // `onnxruntime_providers_cuda.dll`，而 Windows 的搜索顺序里没有「dll 自己所在目录」，
    // 应用又是从 `target\debug` 启动的——只有 PATH 里加了才找得到。
    #[cfg(not(target_os = "macos"))]
    {
        use std::path::PathBuf;
        let mut dirs = vec![dir.clone()];
        // 我们按组件装的目录（`cudart` / `cublas` / `cufft` / `cudnn`）也要进 PATH。
        //
        // 这是 cuDNN 9.x 那个坑的正解：它的实现被拆成十几个 `cudnn_*.dll` 子库，
        // **载哪几个、什么时候载由 cuDNN 前端在运行期决定**（9.27 就会额外要一个
        // `cudnn_engines_tensor_ir64_9.dll`，`ort` 的名字清单里根本没有），而且都是按
        // **裸文件名** `LoadLibrary`。Windows 的搜索顺序里没有「dll 自己所在目录」，
        // 目录又没进 PATH，于是：会话照建、日志写「→ 加速器」，跑第一个 Conv 才炸
        // 「Could not locate cudnn_engines_tensor_ir64_9.dll」。
        dirs.extend(component_dirs(&dir));
        // 新版 CUDA 工具包把 x64 的 dll 放在 `bin\x64`，而 PATH 里通常只有 `bin`。
        // CUDA 11 的 cuDNN 子库同样按裸文件名找；Linux 上 ORT 的 provider 按 SONAME
        // `dlopen`，所以两个变量都要补，不能只改 PATH。
        let flavor = cuda_flavor_of_dir(&dir);
        dirs.extend(toolkit_bins(flavor));
        if let Some(old) = std::env::var_os("PATH") {
            dirs.extend(std::env::split_paths(&old));
        }
        if let Ok(joined) = std::env::join_paths(dirs.iter().map(PathBuf::as_path)) {
            std::env::set_var("PATH", joined);
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let mut lib_dirs = dirs.clone();
            if let Some(old) = std::env::var_os("LD_LIBRARY_PATH") {
                lib_dirs.extend(std::env::split_paths(&old));
            }
            if let Ok(joined) = std::env::join_paths(lib_dirs.iter().map(PathBuf::as_path)) {
                std::env::set_var("LD_LIBRARY_PATH", joined);
            }
        }
    }
    let cuda_provider = if cfg!(target_os = "windows") {
        dir.join("onnxruntime_providers_cuda.dll").is_file()
    } else {
        dir.join("libonnxruntime_providers_cuda.so").is_file()
    };
    // DirectML 与 CUDA 不一样：它是 ORT 的**运行期依赖**，ORT 用裸文件名 `LoadLibrary`
    // 找它，Windows 的搜索顺序里没有「dll 自己所在目录」。所以先按全路径把它加载进进程，
    // 之后 ORT 再按名字找时就命中已加载的模块。
    #[cfg(target_os = "windows")]
    {
        let directml = dir.join("DirectML.dll");
        if directml.is_file() {
            let _ = ort::util::preload_dylib(&directml);
        }
    }
    if id.is_cuda() || cuda_provider {
        // 顺序很重要：先把 cudart / cublas / cudnn 载进来，再载 CUDA provider。
        // provider 依赖这些符号，先载它必失败（实测 `LoadLibraryExW failed`）。
        preload_cuda(&dir);
        // 为什么要自己载 provider：ORT 注册 CUDA EP 时按**裸文件名**去 LoadLibrary，
        // 而 Windows 的搜索顺序里没有「onnxruntime.dll 所在目录」——应用是从
        // `target\debug` 启动的，那里也没有这个 dll。于是注册静默失败、会话落回 CPU，
        // 界面只看到「CUDA 不可用」甚至什么都看不到。按全路径载入后，ORT 再按名字找
        // 就命中已加载的模块。
        let provider = if cfg!(target_os = "windows") {
            dir.join("onnxruntime_providers_cuda.dll")
        } else {
            dir.join("libonnxruntime_providers_cuda.so")
        };
        // 记下来：`init_from` 之后要把它**注册**给 ORT（见下）。
        CUDA_PROVIDER_PATH.set(provider.clone()).ok();
        if !provider.is_file() {
            crate::backend::logfile::note(&format!(
                "[runtime] 目录里没有 CUDA provider dll：{}",
                dir.display()
            ));
        }
    }
    // 变量已经在函数开头钉好了（见上），这里只管真的把 dylib 初始化。
    match ort::init_from(&path) {
        Ok(builder) => {
            let _committed = builder.with_name("ai-daw").commit();
            // **必须显式注册 provider 库**：CUDA 的 EP 实现早就不在核心 dll 里了。
            // 只调 `with_execution_providers([CUDA])` 而没注册库时，ORT 会**静默忽略**它
            // ——会话照常 commit 到 CPU，一句错误都没有，表现就是「能跑但 GPU 利用率 0」。
            if let Some(provider) = CUDA_PROVIDER_PATH.get().cloned() {
                match ort::environment::Environment::current() {
                    Ok(env) => match env.register_ep_library("CUDA", &provider) {
                        Ok(library) => {
                            // 句柄一 drop 就会注销，这里故意不 drop。
                            std::mem::forget(library);
                            CUDA_EP_FLAVOR.store(
                                id.cuda_flavor().unwrap_or(12),
                                std::sync::atomic::Ordering::Relaxed,
                            );
                            crate::backend::logfile::note(&format!(
                                "[runtime] 已注册 CUDA EP 库（{}）",
                                provider.display()
                            ));
                        }
                        Err(e) => crate::backend::logfile::note(&format!(
                            "[runtime] 注册 CUDA EP 库失败（{}）：{e}——CUDA 会回落 CPU",
                            provider.display()
                        )),
                    },
                    Err(e) => crate::backend::logfile::note(&format!(
                        "[runtime] 取 ONNX Runtime 环境失败：{e}"
                    )),
                }
            }
            crate::backend::logfile::note(&format!(
                "[runtime] 已加载 {}（{}）",
                path.display(),
                id.as_str()
            ));
            // 自检：这份 dylib 里到底有没有 CUDA EP。注册失败会回落 CPU，
            // 但那是「每张图试一次」，先把答案打出来省得排查。
            //
            // **只在 AIDAW_ORT_DEBUG 下跑**：`is_available()` 会真的初始化一次 CUDA
            // （枚举设备 / 加载 cuBLAS，实测 1–3 秒），启动路径上不该付这个钱。
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            if std::env::var_os("AIDAW_ORT_DEBUG").is_some() {
                use ort::ep::ExecutionProvider as _;
                let cuda = ort::ep::CUDA::default().is_available().unwrap_or(false);
                crate::backend::logfile::note(&format!(
                    "[runtime] CUDA EP {}",
                    if cuda {
                        "可用"
                    } else {
                        "不可用（会回落 CPU）"
                    }
                ));
            }
            Ok(path)
        }
        Err(e) => {
            crate::backend::logfile::note(&format!("[runtime] 加载 {} 失败：{e}", path.display()));
            Err(format!("加载 {} 失败：{e}", path.display()))
        }
    }
}
#[cfg(not(target_os = "macos"))]

/// 一份 dylib 属于哪种构建：看**同目录**里有没有对应 EP 的 provider（探针 / 开发用）。
///
/// 只影响日志里那个「（cuda）」标签，不影响加载逻辑——加载本来就按目录内容判断。
fn kind_of(path: &Path) -> RuntimeId {
    let dir = path.parent().unwrap_or_else(|| Path::new(""));
    let cuda = if cfg!(target_os = "windows") {
        dir.join("onnxruntime_providers_cuda.dll")
    } else {
        dir.join("libonnxruntime_providers_cuda.so")
    };
    if cuda.is_file() {
        // 目录名能区分 12 / 13（`.../cuda` 与 `.../cuda13`）；
        // 探针直接指到一份解压目录时按里面的 `cudart64_13` / `libcudart.so.13` 判断。
        if cuda_flavor_of_dir(dir) == 13
            || dir.join("cudart64_13.dll").is_file()
            || dir.join("libcudart.so.13").is_file()
        {
            RuntimeId::Cuda13
        } else {
            RuntimeId::Cuda
        }
    } else if dir.join("DirectML.dll").is_file() {
        RuntimeId::DirectMl
    } else {
        RuntimeId::Cpu
    }
}
#[cfg(not(target_os = "macos"))]

/// 找到要加载的那份 dylib。
fn locate() -> Result<(RuntimeId, PathBuf), String> {
    if let Ok(raw) = std::env::var("AIDAW_ORT_RUNTIME") {
        let path = PathBuf::from(raw.trim());
        let path = if path.is_dir() {
            path.join(library_name())
        } else {
            path
        };
        if path.is_file() {
            let id = std::env::var("AIDAW_ORT_RUNTIME_KIND")
                .ok()
                .and_then(|text| RuntimeId::parse(&text))
                .unwrap_or_else(|| kind_of(&path));
            return Ok((id, path));
        }
        return Err(format!(
            "AIDAW_ORT_RUNTIME 指向的文件不存在：{}",
            path.display()
        ));
    }
    let wanted = chosen();
    if installed(wanted) {
        return Ok((wanted, library_path(wanted)));
    }
    // 兜底：本机装了哪个就用哪个（CPU 优先），别因为偏好选了个没装的就把所有模型打死。
    for id in [
        RuntimeId::Cpu,
        RuntimeId::CoreMl,
        RuntimeId::DirectMl,
        RuntimeId::Cuda,
        RuntimeId::Cuda13,
    ] {
        if installed(id) {
            return Ok((id, library_path(id)));
        }
    }
    Err("还没有安装 ONNX Runtime 运行时（设置 → AI → 运行时，先下载 CPU 或 CUDA 版）".into())
}
#[cfg(not(target_os = "macos"))]

/// 把 CUDA / cuDNN 的库预加载进来（CUDA 12 与 13 各用自己那套文件名）。
///
/// 目录来源：应用自己的组件目录（`runtime/ort/cudart[13]` 等）、
/// `AIDAW_CUDA_DIR` / `AIDAW_CUDA13_DIR`、`CUDA_PATH`，以及系统里对应的 CUDA 工具包。
/// 缺失不算错误——用户机器上可能已经装好 CUDA，靠系统搜索路径也能找到。
fn preload_cuda(dir: &Path) {
    // **按文件找目录**，而不是「先挑一个目录、再从里面拿所有文件」：
    // 我们的运行时是按组件分开装的（`runtime/ort/cudart`、`../cublas`、`../cudnn`），
    // 一个目录里凑不齐全部依赖；老写法只挑第一个「含 cublas64_12 的目录」，
    // 结果 `cudart64_12.dll` 根本没被预加载，ORT 去 PATH 里找又只有新版 CUDA 13 的
    // `cudart64_13`，CUDA EP 建不起来 → **静默回落 CPU**（表现就是 GPU 利用率不涨）。
    let flavor = cuda_flavor_of_dir(dir);
    let mut cuda_dirs: Vec<PathBuf> = cuda_candidates(dir)
        .into_iter()
        .filter(|p| p.is_dir())
        .collect();
    // 系统工具包（选 12 就只加 v12，选 13 就只加 v13），避免把两套 DLL 混进一个进程。
    cuda_dirs.extend(toolkit_bins(flavor));
    let cudnn_dirs: Vec<PathBuf> = cudnn_candidates(dir)
        .into_iter()
        .filter(|p| has_cudnn(p))
        .collect();

    if cuda_dirs.is_empty() && cudnn_dirs.is_empty() {
        return;
    }
    // **不用** `ort::ep::cuda::preload_dylibs`：那个 helper 用 `?` 提前返回，失败时还要
    // 构造 `ort::Error`——而 `ort::Error::new_internal` 会**顺手去拿 `ort` 的 API**
    // （见 ort 源码 `error.rs` 里的 `api()` 调用）。此刻 dylib 还没加载，于是它退回裸文件名
    // `onnxruntime.dll`，在系统目录里抓到一份 1.17.1，版本不达标就在 `expect` 上 panic。
    // 实测（crates/local-ai/tests/runtime_probe.rs）本机 `cufft64_11.dll` 不在我们下载的
    // 组件里，于是 `preload_dylibs` 必然失败 → 必然 panic → 音色替换线程当场死掉、
    // 界面上的进度条永远停在 0。这里改成逐个 load，缺哪个就跳过哪个；真正缺的库 ORT
    // 在用的时候会自己报错，而且那时运行时已经加载好了。
    let mut missing = Vec::new();
    let mut loaded = 0usize;
    for name in cuda_dylibs(flavor) {
        // 同名文件可能在多个目录里（组件目录 / pip wheel / torch/lib），取第一个存在的。
        let found = cuda_dirs
            .iter()
            .map(|root| root.join(name))
            .find(|file| file.is_file());
        match found {
            Some(file) => {
                if ort::util::preload_dylib(&file).is_ok() {
                    loaded += 1;
                }
            }
            None => missing.push(*name),
        }
    }
    // cuDNN **不点名**，整目录全载——原因见 [`preload_cudnn`]。
    match cudnn_dirs.first() {
        Some(dir) => loaded += preload_cudnn(dir),
        None => missing.extend(ort::ep::cuda::CUDNN_DYLIBS),
    }
    if std::env::var_os("AIDAW_ORT_DEBUG").is_some() {
        crate::backend::logfile::note(&format!(
            "[runtime] CUDA {flavor} 预加载 {loaded} 个{}",
            if missing.is_empty() {
                String::new()
            } else {
                format!("，缺：{}（交给 ORT 自己找）", missing.join("、"))
            }
        ));
    }
}

/// CUDA 12 / 13 各自的核心库（ORT 的 provider 硬依赖这几个）。
///
/// `ort` crate 里的 `CUDA_DYLIBS` 常量只列了 CUDA 12 的名字；CUDA 13 的 provider 要的是
/// `cudart64_13` / `cublas64_13` / `cufft64_12`（Linux 是 `lib*.so.13` / `.so.12`），
/// 不自己按大版本列出来，13 一个都预加载不到，最后又是一次静默回落 CPU。
#[cfg(target_os = "windows")]
fn cuda_dylibs(flavor: u8) -> &'static [&'static str] {
    if flavor == 13 {
        &[
            "cudart64_13.dll",
            "cublasLt64_13.dll",
            "cublas64_13.dll",
            "cufft64_12.dll",
        ]
    } else {
        &[
            "cudart64_12.dll",
            "cublasLt64_12.dll",
            "cublas64_12.dll",
            "cufft64_11.dll",
        ]
    }
}

#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
fn cuda_dylibs(flavor: u8) -> &'static [&'static str] {
    if flavor == 13 {
        &[
            "libcudart.so.13",
            "libcublasLt.so.13",
            "libcublas.so.13",
            "libcufft.so.12",
            "libnvrtc.so.13",
            "libcurand.so.10",
        ]
    } else {
        &[
            "libcudart.so.12",
            "libcublasLt.so.12",
            "libcublas.so.12",
            "libcufft.so.11",
            "libnvrtc.so.12",
            "libcurand.so.10",
        ]
    }
}
#[cfg(not(target_os = "macos"))]

/// 把**一个**目录里的 cuDNN 子库全载进来，返回载入个数。
///
/// cuDNN 9 把实现拆成了十几个 `cudnn_*.dll`，而且每个 9.x 小版本都在加新的：
/// `ort` 的 `CUDNN_DYLIBS` 清单停在 9.1 时代的 8 个，9.27 新加的
/// `cudnn_engines_tensor_ir64_9.dll`、`cudnn_ext64_9.dll` 都不在里面。
///
/// 漏一个的后果很阴——**建会话完全正常**（日志照样写「→ 加速器」），一直到跑第一个
/// Conv 才炸：
///
/// ```text
/// Could not locate cudnn_engines_tensor_ir64_9.dll. Please make sure it is in your library path!
/// CUDNN_BACKEND_TENSOR_DESCRIPTOR cudnnFinalize failed ... CUDNN_STATUS_SUBLIBRARY_LOADING_FAILED
/// ```
///
/// 因为 cuDNN 前端是**按裸文件名**去 `LoadLibrary` 这些子库的，而 Windows 的搜索顺序里
/// 没有「dll 自己所在目录」；它们又都跟我们下载的 `cudnn64_9.dll` 躺在
/// `runtime/ort/cudnn/`，没进 PATH，于是查无此库。
///
/// 所以这里不再点名，直接把目录里的 `cudnn*` 全载进来；**只取一个目录**——不同目录里
/// 可能混着不同小版本的子库，混用比缺库更难查。
#[cfg(not(target_os = "macos"))]
fn preload_cudnn(dir: &Path) -> usize {
    let prefix = if cfg!(target_os = "windows") {
        "cudnn"
    } else {
        "libcudnn"
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_file())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| {
                            name.starts_with(prefix)
                                && (name.ends_with(".dll") || name.contains(".so"))
                        })
                })
                .collect()
        })
        .unwrap_or_default();
    // 排序保证 `cudnn64_9.dll` 这类基础库先于依赖它的子库进进程（按文件名天然满足）。
    files.sort();
    files
        .iter()
        .filter(|file| ort::util::preload_dylib(*file).is_ok())
        .count()
}
#[cfg(not(target_os = "macos"))]

/// 系统里对应大版本 CUDA 工具包的库目录（按优先级）。
///
/// Windows：`C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.x`，13 起 x64 的 dll
/// 在 `bin\x64`（`bin` 下是 arm64 等其它架构），两个都试。
/// Linux：`/usr/local/cuda-13.x` 与 `/usr/local/cuda`，库在 `lib64` / `lib`。
///
/// **只收对应大版本**：同一进程里混进 CUDA 12 与 13 的 cudart / cuBLAS 是「能载入、
/// 但跑起来算错」的最短路径；选 12 就只找 v12，选 13 就只找 v13。
#[cfg(not(target_os = "macos"))]
fn toolkit_bins(flavor: u8) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for key in [
        "AIDAW_CUDA_DIR",
        "AIDAW_CUDA13_DIR",
        "CUDA_PATH",
        "CUDA_HOME",
    ] {
        if let Some(value) = std::env::var_os(key) {
            roots.push(PathBuf::from(value));
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(entries) =
            std::fs::read_dir(r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA")
        {
            let prefix = format!("v{flavor}");
            let mut versions: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
                .map(|e| e.path())
                .collect();
            versions.sort();
            roots.extend(versions.into_iter().rev());
        }
    }
    #[cfg(all(unix, not(target_os = "windows")))]
    {
        if let Ok(entries) = std::fs::read_dir("/usr/local") {
            let prefix = format!("cuda-{flavor}");
            let mut versions: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
                .map(|e| e.path())
                .collect();
            versions.sort();
            roots.extend(versions.into_iter().rev());
            // `/usr/local/cuda` 是「当前默认工具包」，只有版本对得上才用。
            let current = PathBuf::from("/usr/local/cuda");
            if current.is_dir() {
                roots.push(current);
            }
        }
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for root in roots {
        // `CUDA_PATH` 可能指的是另一个大版本：只有真能找到对应 `cudart` 的根才收，
        // 否则会把 CUDA 12 的 cuBLAS 塞给 CUDA 13 的 provider。
        if !root_has_cuda(&root, flavor) {
            continue;
        }
        for sub in [
            "bin\\x64",
            "bin",
            "lib64",
            "lib",
            "lib\\x64",
            "lib\\x64\\x64",
        ] {
            let dir = root.join(sub);
            if dir.is_dir() && !out.contains(&dir) {
                out.push(dir);
            }
        }
        // 有些解压目录把库直接摊在根上。
        if !out.contains(&root) {
            out.push(root);
        }
    }
    out
}

/// 这个目录（或其 `bin` / `lib` 子目录）里有没有对应大版本的 cudart。
#[cfg(not(target_os = "macos"))]
fn root_has_cuda(root: &Path, flavor: u8) -> bool {
    #[cfg(target_os = "windows")]
    let name = format!("cudart64_{flavor}.dll");
    #[cfg(all(unix, not(target_os = "windows")))]
    let name = format!("libcudart.so.{flavor}");
    [
        root.to_path_buf(),
        root.join("bin"),
        root.join("bin").join("x64"),
        root.join("lib"),
        root.join("lib64"),
    ]
    .iter()
    .any(|dir| dir.join(&name).is_file())
}

/// **我们自己下载**的那几个组件目录（`runtime/ort/cudart|cublas|cufft|cudnn`）。
///
/// 只收「应用自己管理」的目录，不碰 `torch/lib` / site-packages：那些目录里有几百个
/// 同名第三方 dll（`msvcp140` / `zlib` / `libiomp5md` …），塞进 PATH 会把别的库带偏。
/// 这个列表的用途见 [`load`]：既要给 `LoadLibrary` 的**裸文件名**查找用（cuDNN 9 的子库、
/// ORT 的 provider），也要给 `preload_dylib` 按全路径载入的 **dll 依赖**用——libloading
/// 走的是 `LoadLibraryExW(path, 0, 0)`，flags 为 0 意味着**不会**去 dll 自己所在的目录
/// 找依赖，所以「目录进 PATH」是唯一能让子库互相找到的办法。
#[cfg(not(target_os = "macos"))]
fn component_dirs(dir: &Path) -> Vec<PathBuf> {
    let names = component_dir_names(dir);
    let mut out = Vec::new();
    if let Some(parent) = dir.parent() {
        for name in names {
            let candidate = parent.join(name);
            if candidate.is_dir() {
                out.push(candidate);
            }
        }
    }
    let beside = dir.join(names[3]);
    if beside.is_dir() {
        out.push(beside);
    }
    out
}

/// ORT 的 CUDA 大版本（由 dylib 所在目录名 / 同目录里的 cudart 决定）。
fn cuda_flavor_of_dir(dir: &Path) -> u8 {
    if dir.file_name().and_then(|name| name.to_str()) == Some("cuda13") {
        return 13;
    }
    if dir.join("cudart64_13.dll").is_file() || dir.join("libcudart.so.13").is_file() {
        13
    } else {
        12
    }
}

/// 对应大版本的组件目录名：`[cudart, cublas, cufft, cudnn]`（均为 `runtime/ort/` 下的一级目录）。
fn component_dir_names(dir: &Path) -> [&'static str; 4] {
    if cuda_flavor_of_dir(dir) == 13 {
        ["cudart13", "cublas13", "cufft13", "cudnn13"]
    } else {
        ["cudart", "cublas", "cufft", "cudnn"]
    }
}
#[cfg(not(target_os = "macos"))]

/// CUDA 运行库的候选目录（按优先级）。
///
/// 除了应用自己的目录，还认 **pip 的 `nvidia-*` wheel** 与 **PyTorch 的 `torch/lib`**：
/// 这两处是 ML 机器上最常见、且自带 `cublas64_12` / `cudnn64_9` 的地方。
/// （ORT 自己的 Python 版 `preload_dlls()` 用的也是这个顺序。）
fn cuda_candidates(dir: &Path) -> Vec<PathBuf> {
    // **顺序很重要：先我们自己下载的组件，再考虑系统里的。**
    // 以前把 `CUDA_PATH`（用户机器上的工具包，可能是 13.x 或别的 12.x 小版本）排在
    // 组件目录之前，于是进程里会混进系统那份 cudart/cublas —— 同一台机器上「我们装的
    // 是一套、实际用的是另一套」，排查时非常容易被误导。现在系统只作兜底。
    let names = component_dir_names(dir);
    let flavor = cuda_flavor_of_dir(dir);
    let mut out = Vec::new();
    // 组件各自装在自己的目录里：runtime/ort/cudart(13)、runtime/ort/cublas(13) …
    if let Some(parent) = dir.parent() {
        for name in [names[0], names[1], names[2]] {
            out.push(parent.join(name));
        }
    }
    if flavor == 12 {
        out.push(dir.join("cuda"));
    }
    // 系统工具包 / AIDAW_CUDA*_DIR 由 [`toolkit_bins`] 按大版本补，避免 12 / 13 混用。
    for site in all_site_packages() {
        for sub in ["bin", "lib"] {
            out.push(site.join("nvidia").join("cublas").join(sub));
            out.push(site.join("nvidia").join("cuda_runtime").join(sub));
            out.push(site.join("nvidia").join("cufft").join(sub));
        }
        out.push(site.join("torch").join("lib"));
    }
    out
}
#[cfg(not(target_os = "macos"))]

/// cuDNN 9 的候选目录（按优先级）。
fn cudnn_candidates(dir: &Path) -> Vec<PathBuf> {
    let names = component_dir_names(dir);
    // 同 `cuda_candidates`：先我们自己下载的 cuDNN，再系统安装的那份。
    let mut out = Vec::new();
    if let Some(parent) = dir.parent() {
        out.push(parent.join(names[3]));
    }
    out.push(dir.join(names[3]));
    // 也认「cuDNN 和 onnxruntime.dll 摆在同一个目录」的布局（有些第三方整合包就是这样）。
    out.push(dir.to_path_buf());
    out.push(
        std::env::var_os("AIDAW_CUDNN_DIR")
            .map(PathBuf::from)
            .unwrap_or_default(),
    );
    out.push(
        std::env::var_os("CUDNN_PATH")
            .map(PathBuf::from)
            .unwrap_or_default(),
    );
    out.push(
        std::env::var_os("CUDNN_HOME")
            .map(PathBuf::from)
            .unwrap_or_default(),
    );
    #[cfg(target_os = "windows")]
    {
        // 官方 cuDNN 安装器：C:\Program Files\NVIDIA\CUDNN\v9.x\bin
        if let Ok(entries) = std::fs::read_dir(r"C:\Program Files\NVIDIA\CUDNN") {
            let mut versions: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with('v'))
                .map(|e| e.path().join("bin"))
                .collect();
            versions.sort();
            out.extend(versions.into_iter().rev());
        }
    }
    for site in all_site_packages() {
        for sub in ["bin", "lib"] {
            out.push(site.join("nvidia").join("cudnn").join(sub));
        }
        out.push(site.join("torch").join("lib"));
    }
    #[cfg(all(unix, not(target_os = "windows")))]
    {
        // 发行版 / 官方 .deb 的默认安装位置。
        out.push(PathBuf::from("/usr/lib/x86_64-linux-gnu"));
        out.push(PathBuf::from("/usr/local/cuda/lib64"));
    }
    out
}

/// 这台机器上所有能找到的 site-packages（有限深度，启动路径上不能慢）。
fn all_site_packages() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in python_roots() {
        out.extend(python_envs(&root));
    }
    out
}
#[cfg(not(target_os = "macos"))]
/// 目录里有没有 cuDNN 9。
fn has_cudnn(dir: &Path) -> bool {
    dir.is_dir() && (dir.join("cudnn64_9.dll").is_file() || dir.join("libcudnn.so.9").is_file())
}

// ---------------------------------------------------------------------------
// 借用本机已有的那份 CUDA 版运行时
// ---------------------------------------------------------------------------

/// 本机发现的一份现成 CUDA 版 ONNX Runtime。
#[derive(Debug, Clone)]
pub struct Discovered {
    /// **同时含 core 与 CUDA provider** 的目录（借就必须整对借）。
    pub dir: PathBuf,
    /// 版本（能从 dist-info / VERSION_NUMBER / 目录名读到时）。
    pub version: Option<String>,
    /// 来源说明，界面上直接显示。
    pub source: String,
    /// CUDA 大版本（12 / 13）。借来的那份要落到同版本的组件目录里。
    pub flavor: u8,
}

/// provider 的文件名。
fn provider_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "onnxruntime_providers_cuda.dll"
    }
    #[cfg(not(target_os = "windows"))]
    {
        "libonnxruntime_providers_cuda.so"
    }
}

/// 在常见位置找「core + CUDA provider 同目录」的现成运行时。
///
/// 这三类来源都是**自带完整一对**的：
/// - pip / conda 的 `onnxruntime-gpu`：`.../site-packages/onnxruntime/capi/`（实测 1.31.1 的
///   wheel 里同时有 `onnxruntime.dll` 与 `onnxruntime_providers_cuda.dll`）；
/// - 官方 release 解压目录：`onnxruntime-win-x64-gpu-*/lib/`；
/// - 用户自己指定的目录（`AIDAW_ORT_SEARCH`，`;` 分隔）。
///
/// 只做**有限深度**的扫描，不遍历全盘：启动路径上不能卡住。
pub fn discover_cuda() -> Vec<Discovered> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    // 1) 显式指定（探针 / 高阶用户）。
    if let Some(list) = std::env::var_os("AIDAW_ORT_SEARCH") {
        for part in list.to_string_lossy().split(';') {
            let part = part.trim();
            if !part.is_empty() {
                dirs.push(PathBuf::from(part));
            }
        }
    }

    // 2) Python / conda 的 site-packages。
    for root in python_roots() {
        for env in python_envs(&root) {
            let capi = env.join("onnxruntime").join("capi");
            if capi.is_dir() {
                dirs.push(capi);
            }
        }
    }

    // 3) 官方 release 解压出来的目录（放在 ~/Downloads、~ 下之类）。
    for base in user_search_bases() {
        if let Ok(entries) = std::fs::read_dir(&base) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                if name.starts_with("onnxruntime") && name.contains("gpu") {
                    let lib = entry.path().join("lib");
                    if lib.is_dir() {
                        dirs.push(lib);
                    }
                }
            }
        }
    }

    let mut found: Vec<Discovered> = Vec::new();
    for dir in dirs {
        let dir = match dir.canonicalize() {
            Ok(d) => d,
            Err(_) => continue,
        };
        if found.iter().any(|d| d.dir == dir) {
            continue;
        }
        if !dir.join(library_name()).is_file() || !dir.join(provider_name()).is_file() {
            continue;
        }
        let source = if dir.to_string_lossy().contains("site-packages") {
            if dir.to_string_lossy().contains("conda") {
                "conda"
            } else {
                "python"
            }
        } else {
            "file"
        };
        found.push(Discovered {
            version: read_version(&dir),
            flavor: detect_cuda_flavor(&dir),
            dir,
            source: source.to_string(),
        });
    }
    found
}

/// 把发现的那一份**复制**进本应用的运行时目录。
///
/// 为什么复制而不是引用：对方 `pip uninstall onnxruntime-gpu` 或升个版本，我们下次启动就会
/// 加载失败；复制一次（~195 MB，本地盘内）换「永不受影响」。
pub fn import_cuda(from: &Path) -> Result<PathBuf, String> {
    let core = find_core_library(from);
    let provider = from.join(provider_name());
    if core.is_none() || !provider.is_file() {
        return Err(format!(
            "{} 里没有成对的 {} 与 {}",
            from.display(),
            library_name(),
            provider_name()
        ));
    }
    let flavor = detect_cuda_flavor(from);
    let id = if flavor == 13 {
        RuntimeId::Cuda13
    } else {
        RuntimeId::Cuda
    };
    let dir = install_dir(id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("建目录失败：{e}"))?;
    let dest_core = library_path(id)
        .file_name()
        .map(|name| name.to_owned())
        .unwrap_or_else(|| std::ffi::OsString::from(library_name()));
    let shared = if cfg!(target_os = "windows") {
        "onnxruntime_providers_shared.dll"
    } else {
        "libonnxruntime_providers_shared.so"
    };
    let names: Vec<(std::ffi::OsString, std::ffi::OsString)> = vec![
        (core.unwrap(), dest_core),
        (
            std::ffi::OsString::from(provider_name()),
            std::ffi::OsString::from(provider_name()),
        ),
        (
            std::ffi::OsString::from(shared),
            std::ffi::OsString::from(shared),
        ),
    ];
    for (src_name, dest_name) in names {
        let src = from.join(&src_name);
        if src.is_file() {
            std::fs::copy(&src, dir.join(&dest_name))
                .map_err(|e| format!("复制 {} 失败：{e}", src_name.to_string_lossy()))?;
        }
    }
    Ok(dir)
}

/// 找到目录里的 ONNX Runtime 核心库（Linux 的官方包 / pip wheel 名字带版本号）。
fn find_core_library(dir: &Path) -> Option<std::ffi::OsString> {
    let direct = dir.join(library_name());
    if direct.is_file() {
        return Some(std::ffi::OsString::from(library_name()));
    }
    let prefix = if cfg!(target_os = "windows") {
        "onnxruntime.dll"
    } else {
        "libonnxruntime.so"
    };
    let mut found: Vec<std::ffi::OsString> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| {
            let name = entry.file_name();
            let text = name.to_string_lossy();
            // 只收核心库；`libonnxruntime_providers_*.so` 以外的东西不要。
            (text.starts_with(prefix) && !text.contains("providers")).then_some(name)
        })
        .collect();
    found.sort();
    // 带版本号的那个优先（`libonnxruntime.so.1.31.1` 比残留的软链可靠）。
    found.pop()
}

/// 借来的那份是 CUDA 12 还是 13：先看同目录里的 cudart，再看 provider 二进制里的依赖名。
///
/// pip / conda 的 `onnxruntime-gpu` 把 nvidia 库放在 `site-packages/nvidia/` 下，不在
/// provider 同目录；这时只能从 provider 的导入表 / 字符串里认出 `cublas64_13`。
/// 扫 300 MB 的 provider 用流式分块，不整块读进内存。
fn detect_cuda_flavor(dir: &Path) -> u8 {
    for (name, flavor) in [
        ("cudart64_13.dll", 13u8),
        ("libcudart.so.13", 13),
        ("cudart64_12.dll", 12),
        ("libcudart.so.12", 12),
    ] {
        if dir.join(name).is_file() {
            return flavor;
        }
    }
    let provider = dir.join(provider_name());
    if file_contains(&provider, b"cublas64_13")
        || file_contains(&provider, b"libcublas.so.13")
        || file_contains(&provider, b"cufft64_12")
        || file_contains(&provider, b"libcufft.so.12")
    {
        return 13;
    }
    12
}

/// 文件里有没有这段字节（1 MiB 分块 + needle 长度的重叠，避免跨块漏掉）。
fn file_contains(path: &Path, needle: &[u8]) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    if needle.is_empty() {
        return true;
    }
    let mut carry: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let Ok(read) = file.read(&mut buf) else {
            return false;
        };
        if read == 0 {
            return false;
        }
        let mut chunk = std::mem::take(&mut carry);
        chunk.extend_from_slice(&buf[..read]);
        if chunk.windows(needle.len()).any(|window| window == needle) {
            return true;
        }
        let keep = needle.len().saturating_sub(1).min(chunk.len());
        carry = chunk[chunk.len() - keep..].to_vec();
    }
}

/// 读一份运行时的版本：`onnxruntime-*.dist-info` > `VERSION_NUMBER` > 目录名里的 `x.y.z`。
fn read_version(dir: &Path) -> Option<String> {
    // <site-packages>/onnxruntime/capi → <site-packages>
    let site = dir.parent()?.parent()?;
    if let Ok(entries) = std::fs::read_dir(site) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("onnxruntime") && name.ends_with(".dist-info") {
                let version = name
                    .trim_end_matches(".dist-info")
                    .rsplit('-')
                    .next()
                    .unwrap_or_default()
                    .to_string();
                if !version.is_empty() {
                    return Some(version);
                }
            }
        }
    }
    for candidate in [
        dir.join("VERSION_NUMBER"),
        dir.parent()?.join("VERSION_NUMBER"),
    ] {
        if let Ok(text) = std::fs::read_to_string(candidate) {
            let text = text.trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    let name = dir.file_name()?.to_string_lossy().into_owned();
    name.rsplit('-')
        .next()
        .filter(|v| v.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(str::to_string)
}

/// 可能装着 Python 的根目录。
fn python_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(target_os = "windows")]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            roots.push(PathBuf::from(&local).join("Programs").join("Python"));
            // Microsoft Store 版 Python。
            roots.push(PathBuf::from(&local).join("Packages"));
        }
        if let Some(roaming) = std::env::var_os("APPDATA") {
            roots.push(PathBuf::from(&roaming).join("Python"));
        }
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let home = PathBuf::from(home);
        for name in ["anaconda3", "miniconda3", "miniforge3", "mambaforge"] {
            roots.push(home.join(name));
        }
    }
    roots
}

/// 某个根目录下的 site-packages（含 venv / conda env，深度有限）。
fn python_envs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push = |dir: PathBuf| {
        if dir.is_dir() {
            out.push(dir);
        }
    };
    // 根目录自己就是一个环境。
    push(root.join("Lib").join("site-packages"));
    push(root.join("Lib").join("site-packages"));
    // <root>/<env>/Lib/site-packages（Windows 的 Python 版本目录 / conda env）。
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten().take(40) {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            push(path.join("Lib").join("site-packages"));
            // Microsoft Store：Packages/<pkg>/LocalCache/local-packages/Python*/site-packages
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with("PythonSoftwareFoundation")
            {
                let cache = path.join("LocalCache").join("local-packages");
                if let Ok(pys) = std::fs::read_dir(&cache) {
                    for py in pys.flatten().take(10) {
                        push(py.path().join("site-packages"));
                    }
                }
            }
        }
    }
    // conda：<root>/envs/<env>/Lib/site-packages
    let envs = root.join("envs");
    if let Ok(entries) = std::fs::read_dir(&envs) {
        for entry in entries.flatten().take(20) {
            push(entry.path().join("Lib").join("site-packages"));
        }
    }
    out
}

/// 用户可能解压官方 release 的地方（只扫一层）。
fn user_search_bases() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let home = PathBuf::from(home);
        out.push(home.join("Downloads"));
        out.push(home.clone());
    }
    out.into_iter().filter(|p| p.is_dir()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for id in [
            RuntimeId::Cpu,
            RuntimeId::Cuda,
            RuntimeId::Cuda13,
            RuntimeId::DirectMl,
            RuntimeId::CoreMl,
            RuntimeId::Cudart,
            RuntimeId::Cublas,
            RuntimeId::Cudnn,
            RuntimeId::Cufft,
            RuntimeId::Cudart13,
            RuntimeId::Cublas13,
            RuntimeId::Cudnn13,
            RuntimeId::Cufft13,
        ] {
            assert_eq!(RuntimeId::parse(id.as_str()), Some(id));
            assert_eq!(RuntimeId::from_u8(id.as_u8()), id);
        }
        assert_eq!(RuntimeId::parse("GPU"), Some(RuntimeId::Cuda));
    }

    #[test]
    fn every_runtime_is_well_formed() {
        // 两张表都查（Windows / Linux 是同一份结构，Linux 那份在 Windows 上也能校验）。
        for asset in WINDOWS.iter().chain(LINUX.iter()) {
            assert!(!asset.label.is_empty());
            assert!(
                !asset.parts.is_empty(),
                "{} 没有下载内容",
                asset.id.as_str()
            );
            for part in asset.parts {
                assert!(part.bytes > 0);
                assert!(!part.sources.is_empty(), "{} 没有源", part.archive);
                assert!(!part.files.is_empty());
                for source in part.sources {
                    assert!(
                        source.url.starts_with("https://"),
                        "{} 不是 https",
                        source.url
                    );
                    // 运行时只从厂商自己（GitHub / NVIDIA / NuGet）或大厂镜像站下，
                    // 与清单侧 `every_source_is_an_official_host` 同一套白名单。
                    let host = source
                        .url
                        .split_once("://")
                        .map(|(_, rest)| rest)
                        .unwrap_or(source.url)
                        .split(['/', '?', '#'])
                        .next()
                        .unwrap_or("")
                        .split(':')
                        .next()
                        .unwrap_or("")
                        .trim_start_matches("www.")
                        .to_ascii_lowercase();
                    const TRUSTED_ROOTS: &[&str] = &[
                        "github.com",
                        "githubusercontent.com",
                        "api.nuget.org",
                        "nvidia.com",
                        "nvidia.cn",
                        "microsoft.com",
                        "mirrors.tuna.tsinghua.edu.cn",
                        "mirrors.aliyun.com",
                        "mirrors.huaweicloud.com",
                        "mirrors.ustc.edu.cn",
                        "mirrors.cloud.tencent.com",
                    ];
                    assert!(
                        TRUSTED_ROOTS
                            .iter()
                            .any(|root| host == *root || host.ends_with(&format!(".{root}"))),
                        "{} 用了非官方源：{}（域名 {host}）",
                        part.archive,
                        source.url
                    );
                }
            }
            // 装好的判定：清单里每个文件都必须在解压清单里，缺一个就等于没装。
            let required = asset.required();
            assert!(
                !required.is_empty(),
                "{} 没有可判定的文件清单",
                asset.id.as_str()
            );
            for name in required {
                assert!(
                    asset
                        .parts
                        .iter()
                        .flat_map(|p| p.files.iter())
                        .any(|f| f.dest == name),
                    "{} 要求的 {name} 不在解压清单里",
                    asset.id.as_str()
                );
            }
        }
    }

    /// CUDA 13 只给新卡：驱动 ≥ 580 且计算能力 ≥ 7.5；其余留在 CUDA 12。
    #[test]
    fn cuda13_is_only_recommended_for_new_cards() {
        use crate::backend::detect::{GpuVendor, HardwareProfile};
        let profile = |vendor, driver: &str, cap: &str| HardwareProfile {
            os: "windows",
            arch: "x86_64",
            gpu: Some(vendor),
            gpu_name: None,
            driver: Some(driver.to_string()),
            compute_cap: Some(cap.to_string()),
        };
        assert!(cuda13_supported(&profile(
            GpuVendor::Nvidia,
            "581.15",
            "8.9"
        )));
        assert!(cuda13_supported(&profile(
            GpuVendor::Nvidia,
            "580.00",
            "7.5"
        )));
        // 老驱动 / 老架构（Pascal 6.1、Volta 7.0）都不行。
        assert!(!cuda13_supported(&profile(
            GpuVendor::Nvidia,
            "560.94",
            "8.9"
        )));
        assert!(!cuda13_supported(&profile(
            GpuVendor::Nvidia,
            "581.15",
            "6.1"
        )));
        assert!(!cuda13_supported(&profile(
            GpuVendor::Nvidia,
            "581.15",
            "7.0"
        )));
        // 非 N 卡永远不走 CUDA 13。
        assert!(!cuda13_supported(&profile(GpuVendor::Amd, "581.15", "8.9")));
    }

    /// CUDA 12 / 13 各有一套组件，`components_of` 不能把两套混在一起。
    #[test]
    fn each_cuda_flavor_has_its_own_components() {
        let twelve = components_of(RuntimeId::Cuda);
        let thirteen = components_of(RuntimeId::Cuda13);
        assert!(twelve.iter().all(|id| id.cuda_flavor() == Some(12)));
        assert!(thirteen.iter().all(|id| id.cuda_flavor() == Some(13)));
        assert!(twelve.contains(&RuntimeId::Cudart));
        assert!(thirteen.contains(&RuntimeId::Cudart13));
        assert!(!twelve.iter().any(|id| thirteen.contains(id)));
    }

    #[test]
    fn component_folders_follow_the_cuda_flavor() {
        let twelve = component_dir_names(Path::new("C:/rt/ort/cuda"));
        let thirteen = component_dir_names(Path::new("C:/rt/ort/cuda13"));
        assert_eq!(twelve, ["cudart", "cublas", "cufft", "cudnn"]);
        assert_eq!(thirteen, ["cudart13", "cublas13", "cufft13", "cudnn13"]);
    }

    #[test]
    fn backend_maps_to_a_runtime() {
        use crate::backend::BackendKind;
        assert_eq!(for_backend(BackendKind::Cuda12), RuntimeId::Cuda);
        assert_eq!(for_backend(BackendKind::Cuda13), RuntimeId::Cuda13);
        assert_eq!(for_backend(BackendKind::DirectMl), RuntimeId::DirectMl);
    }

    /// Windows / Linux 都必须有 CUDA 13 本体 + 13 那一套组件，否则设置页只会有 12。
    #[test]
    fn both_platforms_offer_cuda_13_and_its_components() {
        for os in ["windows", "linux"] {
            let ids: Vec<RuntimeId> = for_platform(os, "x86_64")
                .into_iter()
                .map(|asset| asset.id)
                .collect();
            for required in [
                RuntimeId::Cuda,
                RuntimeId::Cuda13,
                RuntimeId::Cudart13,
                RuntimeId::Cublas13,
                RuntimeId::Cufft13,
                RuntimeId::Cudnn13,
            ] {
                assert!(
                    ids.contains(&required),
                    "{os} 清单里没有 {}",
                    required.as_str()
                );
            }
            // 同一平台不能把 12 / 13 的组件混进同一个条目。
            for id in [
                RuntimeId::Cudart13,
                RuntimeId::Cublas13,
                RuntimeId::Cufft13,
                RuntimeId::Cudnn13,
            ] {
                assert_eq!(id.cuda_flavor(), Some(13));
            }
        }
    }

    /// Linux 的 CUDA 12 / 13 本体走 NuGet（`Microsoft.ML.OnnxRuntime.GPU.Linux`），
    /// 和 Windows 一样有 nuget.org + 华为镜像两边；包内核心库是不带版本后缀的
    /// `libonnxruntime.so`，`library_path` 必须能直接对上。
    #[test]
    fn linux_cuda_bodies_are_dual_source_nuget() {
        let linux = for_platform("linux", "x86_64");
        for id in [RuntimeId::Cuda, RuntimeId::Cuda13] {
            let asset = linux
                .iter()
                .find(|asset| asset.id == id)
                .unwrap_or_else(|| panic!("Linux 清单里没有 {}", id.as_str()));
            let part = &asset.parts[0];
            let expected = if id == RuntimeId::Cuda {
                "1.26.0"
            } else {
                "1.31.1"
            };
            assert!(
                part.archive.contains(expected),
                "{} 应该钉 ORT {}，实际是 {}",
                id.as_str(),
                expected,
                part.archive
            );
            assert!(
                part.archive.ends_with(".nupkg"),
                "{} 应该走 NuGet 包，实际是 {}",
                id.as_str(),
                part.archive
            );
            assert!(
                part.sources.iter().any(|source| source.region == "intl"),
                "{} 缺国际源",
                id.as_str()
            );
            assert!(
                part.sources.iter().any(|source| source.region == "cn"),
                "{} 缺国内源",
                id.as_str()
            );
            assert_eq!(part.files[0].dest, "libonnxruntime.so");
            // `library_path()` 取的是**当前平台**的清单；这里是 Linux 条目，直接看它的
            // `required()` 首项即可。
            assert_eq!(asset.required().first().copied(), Some("libonnxruntime.so"));
        }
    }

    /// 预加载清单必须按大版本给出对应的库名；`ort` 自带的常量只有 CUDA 12。
    #[test]
    #[cfg(not(target_os = "macos"))]
    fn preload_names_follow_the_cuda_flavor() {
        assert!(cuda_dylibs(13).iter().any(|name| name.contains("13")));
        assert!(cuda_dylibs(12)
            .iter()
            .any(|name| name.contains("12") || name.contains("11")));
        // cudart / cublas 必须跟着大版本走；cuFFT 在 CUDA 13 里仍是 .12 / 64_12。
        for name in cuda_dylibs(13) {
            if name.contains("cudart") || name.contains("cublas") {
                assert!(name.contains("13"), "CUDA 13 不该预加载 {name}");
            }
        }
        for name in cuda_dylibs(12) {
            if name.contains("cudart") || name.contains("cublas") {
                assert!(!name.contains("13"), "CUDA 12 不该预加载 {name}");
            }
        }
    }

    /// CUDA 那几份组件**每个文件都要**：只装一个 `onnxruntime_providers_cuda.dll`
    /// 就会在跑第一个卷积时炸，或者干脆静默回落 CPU。
    #[test]
    fn components_require_every_extracted_file() {
        for id in [
            RuntimeId::Cuda,
            RuntimeId::Cuda13,
            RuntimeId::Cudart,
            RuntimeId::Cublas,
            RuntimeId::Cudnn,
            RuntimeId::Cufft,
            RuntimeId::Cudart13,
            RuntimeId::Cublas13,
            RuntimeId::Cudnn13,
            RuntimeId::Cufft13,
        ] {
            let asset = asset(id).unwrap_or_else(|| panic!("{} 不在本机清单里", id.as_str()));
            let required = asset.required();
            assert!(!required.is_empty(), "{} 没有判定清单", id.as_str());
            for name in &required {
                assert!(
                    asset
                        .parts
                        .iter()
                        .flat_map(|p| p.files.iter())
                        .any(|f| f.dest == *name),
                    "{} 要求的 {name} 没在解压清单里",
                    id.as_str()
                );
            }
        }
        // cuDNN 9 的实现是十个 .so/.dll，缺哪个都可能在推理时炸。
        for id in [RuntimeId::Cudnn, RuntimeId::Cudnn13] {
            let cudnn = asset(id).unwrap();
            assert!(
                cudnn.required().len() >= 10,
                "{} 只要求了 {} 个文件",
                id.as_str(),
                cudnn.required().len()
            );
        }
    }
}
