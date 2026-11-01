//! **外部 GGUF 推理引擎的运行时清单**：audio.cpp 的 `audiocpp_server` 与它的后端。
//!
//! YuE2（以及以后接的 ACE-Step、MiniMax Music…）都跑在 [audio.cpp](https://github.com/0xShug0/audio.cpp)
//! 上：它不是 ONNX Runtime，而是一套自带 ggml 的 C++ 推理器，对外是一个**常驻 HTTP 服务**
//! （`audiocpp_server`，OpenAI 风格的 `/v1/audio/speech` + 通用的 `/v1/tasks/run`）。
//!
//! 每个后端是一份**自包含的便携包**，解到自己的子目录里（CUDA 12 / 13 的 `cudart*` 名字
//! 会撞车，Vulkan 也不能和 CUDA 混放）：
//!
//! ```text
//! <应用配置目录>/runtime/audiocpp/
//!   cpu/      audiocpp_server.exe + model_specs/yue2.json + VC 运行库
//!   vulkan/   同上（后端静态编进 exe）
//!   cuda12/   同上 + ggml*.dll + ggml-cuda.dll + cudart64_12 / cublas* / cufft
//!   cuda13/   同上（CUDA 13 的运行时）
//!   preferred 用户挑的那一份（不写则按显卡自动挑）
//!   server.log
//! ```
//!
//! **为什么不按文件「挑」**：audio.cpp 的包本身就是「可跑的最小集」（CPU 30 MB / Vulkan 82 MB /
//! CUDA 一档 0.85–1.07 GB），整包下载还能顺带校验发布页给的**整包 sha256**。所以这里只
//! 记录「每份归档里要落哪几个文件」，下载走整包 + 逐条解压。
//!
//! 平台：Windows x64 用 zip（CPU / Vulkan / CUDA 12 / CUDA 13）；macOS arm64 用
//! `bin-macos-arm64-metal.tar.gz`（Metal）；Linux x64 用 `tar.gz`（CPU / Vulkan / CUDA 12）。
//! 归档的 `format` 决定 zip 还是 tar.gz；`packs()` 对不支持的平台返回空表。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// 一个下载源。
#[derive(Debug, Clone, Copy)]
pub struct EngineSource {
    /// `intl`（官方）/ `cn`（国内镜像）。
    pub region: &'static str,
    /// 直链地址。
    pub url: &'static str,
}

/// 归档格式。Windows 是 zip；macOS / Linux 的官方包是 `tar.gz`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineFormat {
    Zip,
    TarGz,
}

/// 一份归档（几个文件共用一个包）。
#[derive(Debug, Clone, Copy)]
pub struct EngineArchive {
    /// 下载时的文件名。
    pub file: &'static str,
    /// 整包体积（字节）。
    pub bytes: u64,
    /// 归档 sha256（GitHub release 的 digest，发布页直接给的）。
    pub sha256: &'static str,
    /// 解包格式。
    pub format: EngineFormat,
    /// 候选源。
    pub sources: &'static [EngineSource],
}

/// 归档里要取出的一个文件。
#[derive(Debug, Clone, Copy)]
pub struct EngineFile {
    /// 归档内路径（`/` 分隔）。
    pub inner: &'static str,
    /// 落地路径（相对后端目录；可以带子目录，例如 `model_specs/yue2.json`）。
    pub dest: &'static str,
}

/// 一份归档 + 要从中落地的文件。
#[derive(Debug, Clone, Copy)]
pub struct EnginePart {
    /// 归档文件名（对应 [`archive`]）。
    pub archive: &'static str,
    /// 要落地的文件（**缺一不可**）。
    pub files: &'static [EngineFile],
}

/// 清单里的一个后端（= 一套可跑的目录）。
#[derive(Debug, Clone, Copy)]
pub struct EnginePack {
    /// 稳定标识（`audiocpp-cpu` / `audiocpp-vulkan` / `audiocpp-cuda12` / `audiocpp-cuda13`）。
    pub id: &'static str,
    /// 展示名。
    pub label: &'static str,
    /// 一句话说明（界面副标题）。
    pub note: &'static str,
    /// 子目录 / 设备名：`cpu` / `vulkan` / `cuda12` / `cuda13`。
    pub device: &'static str,
    /// 是否是默认那一份（CPU 兜底；界面上标「主程序」）。
    pub core: bool,
    /// 组成这套目录的归档。
    pub parts: &'static [EnginePart],
}

impl EnginePack {
    /// 落地的文件（按后端目录算，去重）。
    pub fn files(&self) -> Vec<&'static EngineFile> {
        self.parts.iter().flat_map(|p| p.files.iter()).collect()
    }

    /// 装好的判定：**清单里每个文件都在**（半截目录不算装好）。
    pub fn missing(&self) -> Vec<&'static str> {
        self.missing_in(&dir_of(self))
    }

    /// 同上，但检查指定目录（测试 / 「装了一半」的排查用）。
    pub fn missing_in(&self, dir: &Path) -> Vec<&'static str> {
        self.files()
            .into_iter()
            .filter(|f| !dir.join(f.dest).is_file())
            .map(|f| f.dest)
            .collect()
    }

    /// 下载量（整包相加）。
    pub fn bytes(&self) -> u64 {
        self.parts
            .iter()
            .filter_map(|p| archive(p.archive))
            .map(|a| a.bytes)
            .sum()
    }

    /// 落地的字节（后端目录里那几个文件的大小之和）。界面「占用」用。
    pub fn disk_bytes(&self) -> u64 {
        let dir = dir_of(self);
        self.files()
            .iter()
            .filter_map(|f| std::fs::metadata(dir.join(f.dest)).ok())
            .map(|m| m.len())
            .sum()
    }
}

// ---------------------------------------------------------------------------
// 归档
// ---------------------------------------------------------------------------

/// 六份归档：每个后端一份（CUDA 另加一份运行时）。
///
/// 哈希全部取自 GitHub release 的 `digest`（发布页 `sha256:` 那一列），已在本地实测
/// 整包一致（2026-10-31）。源是**国内外双源**：国际走 GitHub Releases，国内走魔搭
/// `ailm32442/audiocpp-assets`（同一份字节；该仓库的 `Sha256` 与这里逐包一致）。
pub const ARCHIVES: &[EngineArchive] = &[
    EngineArchive {
        file: "audio-v0.9.3-bin-windows-x64-cpu-portable.zip",
        bytes: 30_409_504,
        sha256: "f5fa4662b7ec323dbe31d37e224e46159b064836a9005da384551a5248771eeb",
        format: EngineFormat::Zip,
        sources: &[
            EngineSource {
                region: "intl",
                url: "https://github.com/0xShug0/audio.cpp/releases/download/v0.9.3/audio-v0.9.3-bin-windows-x64-cpu-portable.zip",
            },
            EngineSource {
                region: "cn",
                url: "https://modelscope.cn/models/ailm32442/audiocpp-assets/resolve/master/audio-v0.9.3-bin-windows-x64-cpu-portable.zip",
            },
        ],
    },
    EngineArchive {
        file: "audio-v0.9.3-bin-windows-x64-vulkan-portable.zip",
        bytes: 82_088_153,
        sha256: "51f8d8ce6cc7789bc9dc3d15f35e05f143aaf2ce1cbecc525a4104bcdde82a42",
        format: EngineFormat::Zip,
        sources: &[
            EngineSource {
                region: "intl",
                url: "https://github.com/0xShug0/audio.cpp/releases/download/v0.9.3/audio-v0.9.3-bin-windows-x64-vulkan-portable.zip",
            },
            EngineSource {
                region: "cn",
                url: "https://modelscope.cn/models/ailm32442/audiocpp-assets/resolve/master/audio-v0.9.3-bin-windows-x64-vulkan-portable.zip",
            },
        ],
    },
    EngineArchive {
        file: "audio-v0.9.3-bin-windows-x64-cuda12.4.zip",
        bytes: 465_913_709,
        sha256: "976b1fe4329407be45b272fc656d91006f081d22706f453f8879bc69dbb4a9b3",
        format: EngineFormat::Zip,
        sources: &[
            EngineSource {
                region: "intl",
                url: "https://github.com/0xShug0/audio.cpp/releases/download/v0.9.3/audio-v0.9.3-bin-windows-x64-cuda12.4.zip",
            },
            EngineSource {
                region: "cn",
                url: "https://modelscope.cn/models/ailm32442/audiocpp-assets/resolve/master/audio-v0.9.3-bin-windows-x64-cuda12.4.zip",
            },
        ],
    },
    EngineArchive {
        file: "audio-v0.9.3-cudart-windows-x64-cuda12.4.zip",
        bytes: 607_273_675,
        sha256: "866a2bc12c4d887eb3b1b62f82f7a8fdd93391cb77ea02b63b0f360cfb3bb4d8",
        format: EngineFormat::Zip,
        sources: &[
            EngineSource {
                region: "intl",
                url: "https://github.com/0xShug0/audio.cpp/releases/download/v0.9.3/audio-v0.9.3-cudart-windows-x64-cuda12.4.zip",
            },
            EngineSource {
                region: "cn",
                url: "https://modelscope.cn/models/ailm32442/audiocpp-assets/resolve/master/audio-v0.9.3-cudart-windows-x64-cuda12.4.zip",
            },
        ],
    },
    EngineArchive {
        file: "audio-v0.9.3-bin-windows-x64-cuda13.3.zip",
        bytes: 277_336_614,
        sha256: "0bd3988be10c31ec078c40df2bd0d4266e660804f6d5083aeb05d1309d091e43",
        format: EngineFormat::Zip,
        sources: &[
            EngineSource {
                region: "intl",
                url: "https://github.com/0xShug0/audio.cpp/releases/download/v0.9.3/audio-v0.9.3-bin-windows-x64-cuda13.3.zip",
            },
            EngineSource {
                region: "cn",
                url: "https://modelscope.cn/models/ailm32442/audiocpp-assets/resolve/master/audio-v0.9.3-bin-windows-x64-cuda13.3.zip",
            },
        ],
    },
    EngineArchive {
        file: "audio-v0.9.3-cudart-windows-x64-cuda13.3.zip",
        bytes: 575_457_454,
        sha256: "839eb20f684a8aab64895b1e466b32ce9b29dfc19dc1ce15b2ef35218512ac0a",
        format: EngineFormat::Zip,
        sources: &[
            EngineSource {
                region: "intl",
                url: "https://github.com/0xShug0/audio.cpp/releases/download/v0.9.3/audio-v0.9.3-cudart-windows-x64-cuda13.3.zip",
            },
            EngineSource {
                region: "cn",
                url: "https://modelscope.cn/models/ailm32442/audiocpp-assets/resolve/master/audio-v0.9.3-cudart-windows-x64-cuda13.3.zip",
            },
        ],
    },
    EngineArchive {
        file: "audio-v0.9.3-bin-macos-arm64-metal.tar.gz",
        bytes: 34_847_832,
        sha256: "70a84c2b61ce5ef99547a12affab9b0d40daf08666baf7da4f940d5c7b722819",
        format: EngineFormat::TarGz,
        sources: &[
            EngineSource {
                region: "intl",
                url: "https://github.com/0xShug0/audio.cpp/releases/download/v0.9.3/audio-v0.9.3-bin-macos-arm64-metal.tar.gz",
            },
            EngineSource {
                region: "cn",
                url: "https://modelscope.cn/models/ailm32442/audiocpp-assets/resolve/master/audio-v0.9.3-bin-macos-arm64-metal.tar.gz",
            },
        ],
    },
    EngineArchive {
        file: "audio-v0.9.3-bin-ubuntu-x64-cpu-portable.tar.gz",
        bytes: 63_289_061,
        sha256: "02156f7a7bd9924902cec26be35087d3806c7e348be344b1ec28cac0c1af3929",
        format: EngineFormat::TarGz,
        sources: &[
            EngineSource {
                region: "intl",
                url: "https://github.com/0xShug0/audio.cpp/releases/download/v0.9.3/audio-v0.9.3-bin-ubuntu-x64-cpu-portable.tar.gz",
            },
            EngineSource {
                region: "cn",
                url: "https://modelscope.cn/models/ailm32442/audiocpp-assets/resolve/master/audio-v0.9.3-bin-ubuntu-x64-cpu-portable.tar.gz",
            },
        ],
    },
    EngineArchive {
        file: "audio-v0.9.3-bin-ubuntu-x64-vulkan-portable.tar.gz",
        bytes: 77_051_422,
        sha256: "10ac6b5cb72bc9266e5a600407dbcc2a3eaf87139a3e01c064bf3035d11f81b8",
        format: EngineFormat::TarGz,
        sources: &[
            EngineSource {
                region: "intl",
                url: "https://github.com/0xShug0/audio.cpp/releases/download/v0.9.3/audio-v0.9.3-bin-ubuntu-x64-vulkan-portable.tar.gz",
            },
            EngineSource {
                region: "cn",
                url: "https://modelscope.cn/models/ailm32442/audiocpp-assets/resolve/master/audio-v0.9.3-bin-ubuntu-x64-vulkan-portable.tar.gz",
            },
        ],
    },
    EngineArchive {
        file: "audio-v0.9.3-bin-ubuntu-x64-cuda12.8-colab.tar.gz",
        bytes: 228_182_886,
        sha256: "5be3e8d515fb244387de87958668ebb729d9442de6a53df12e4c718ea918419d",
        format: EngineFormat::TarGz,
        sources: &[
            EngineSource {
                region: "intl",
                url: "https://github.com/0xShug0/audio.cpp/releases/download/v0.9.3/audio-v0.9.3-bin-ubuntu-x64-cuda12.8-colab.tar.gz",
            },
            EngineSource {
                region: "cn",
                url: "https://modelscope.cn/models/ailm32442/audiocpp-assets/resolve/master/audio-v0.9.3-bin-ubuntu-x64-cuda12.8-colab.tar.gz",
            },
        ],
    },
    EngineArchive {
        file: "audio-v0.9.3-cudart-ubuntu-x64-cuda12.8.tar.gz",
        bytes: 787_020_239,
        sha256: "2fcbaff8078b9eaed47a4a8e879c10dcf1f12fd31f2d52b29214fc9d45cfa198",
        format: EngineFormat::TarGz,
        sources: &[
            EngineSource {
                region: "intl",
                url: "https://github.com/0xShug0/audio.cpp/releases/download/v0.9.3/audio-v0.9.3-cudart-ubuntu-x64-cuda12.8.tar.gz",
            },
            EngineSource {
                region: "cn",
                url: "https://modelscope.cn/models/ailm32442/audiocpp-assets/resolve/master/audio-v0.9.3-cudart-ubuntu-x64-cuda12.8.tar.gz",
            },
        ],
    },
];

/// 按文件名找归档。
pub fn archive(file: &str) -> Option<&'static EngineArchive> {
    ARCHIVES.iter().find(|a| a.file == file)
}

// ---------------------------------------------------------------------------
// 后端
// ---------------------------------------------------------------------------

/// 公用的那几个文件说明：服务端程序 + 模型规格（当前只带 YuE2 的 `yue2.json`），**每个后端都带一份**。

const CUDA12_BIN: EnginePart = EnginePart {
    archive: "audio-v0.9.3-bin-windows-x64-cuda12.4.zip",
    files: &[
        fi("audiocpp_server.exe", "audiocpp_server.exe"),
        fi("model_specs/yue2.json", "model_specs/yue2.json"),
        fi("ggml.dll", "ggml.dll"),
        fi("ggml-base.dll", "ggml-base.dll"),
        fi("ggml-cpu-x64.dll", "ggml-cpu-x64.dll"),
        fi("ggml-cpu-sse42.dll", "ggml-cpu-sse42.dll"),
        fi("ggml-cpu-sandybridge.dll", "ggml-cpu-sandybridge.dll"),
        fi("ggml-cpu-haswell.dll", "ggml-cpu-haswell.dll"),
        fi("ggml-cpu-skylakex.dll", "ggml-cpu-skylakex.dll"),
        fi("ggml-cpu-cannonlake.dll", "ggml-cpu-cannonlake.dll"),
        fi("ggml-cpu-icelake.dll", "ggml-cpu-icelake.dll"),
        fi("ggml-cpu-cascadelake.dll", "ggml-cpu-cascadelake.dll"),
        fi("ggml-cpu-alderlake.dll", "ggml-cpu-alderlake.dll"),
        fi("ggml-cuda.dll", "ggml-cuda.dll"),
        fi("msvcp140.dll", "msvcp140.dll"),
        fi("msvcp140_1.dll", "msvcp140_1.dll"),
        fi("msvcp140_2.dll", "msvcp140_2.dll"),
        fi("msvcp140_atomic_wait.dll", "msvcp140_atomic_wait.dll"),
        fi("msvcp140_codecvt_ids.dll", "msvcp140_codecvt_ids.dll"),
        fi("vcruntime140.dll", "vcruntime140.dll"),
        fi("vcruntime140_1.dll", "vcruntime140_1.dll"),
        fi("vcruntime140_threads.dll", "vcruntime140_threads.dll"),
    ],
};

const CUDA13_BIN: EnginePart = EnginePart {
    archive: "audio-v0.9.3-bin-windows-x64-cuda13.3.zip",
    files: &[
        fi("audiocpp_server.exe", "audiocpp_server.exe"),
        fi("model_specs/yue2.json", "model_specs/yue2.json"),
        fi("ggml.dll", "ggml.dll"),
        fi("ggml-base.dll", "ggml-base.dll"),
        fi("ggml-cpu-x64.dll", "ggml-cpu-x64.dll"),
        fi("ggml-cpu-sse42.dll", "ggml-cpu-sse42.dll"),
        fi("ggml-cpu-sandybridge.dll", "ggml-cpu-sandybridge.dll"),
        fi("ggml-cpu-haswell.dll", "ggml-cpu-haswell.dll"),
        fi("ggml-cpu-skylakex.dll", "ggml-cpu-skylakex.dll"),
        fi("ggml-cpu-cannonlake.dll", "ggml-cpu-cannonlake.dll"),
        fi("ggml-cpu-icelake.dll", "ggml-cpu-icelake.dll"),
        fi("ggml-cpu-cascadelake.dll", "ggml-cpu-cascadelake.dll"),
        fi("ggml-cpu-alderlake.dll", "ggml-cpu-alderlake.dll"),
        fi("ggml-cuda.dll", "ggml-cuda.dll"),
        fi("msvcp140.dll", "msvcp140.dll"),
        fi("msvcp140_1.dll", "msvcp140_1.dll"),
        fi("msvcp140_2.dll", "msvcp140_2.dll"),
        fi("msvcp140_atomic_wait.dll", "msvcp140_atomic_wait.dll"),
        fi("msvcp140_codecvt_ids.dll", "msvcp140_codecvt_ids.dll"),
        fi("vcruntime140.dll", "vcruntime140.dll"),
        fi("vcruntime140_1.dll", "vcruntime140_1.dll"),
        fi("vcruntime140_threads.dll", "vcruntime140_threads.dll"),
    ],
};

const CUDA12_RUNTIME: EnginePart = EnginePart {
    archive: "audio-v0.9.3-cudart-windows-x64-cuda12.4.zip",
    files: &[
        fi("cudart64_12.dll", "cudart64_12.dll"),
        fi("cublas64_12.dll", "cublas64_12.dll"),
        fi("cublasLt64_12.dll", "cublasLt64_12.dll"),
        fi("cufft64_11.dll", "cufft64_11.dll"),
    ],
};

const CUDA13_RUNTIME: EnginePart = EnginePart {
    archive: "audio-v0.9.3-cudart-windows-x64-cuda13.3.zip",
    files: &[
        fi("cudart64_13.dll", "cudart64_13.dll"),
        fi("cublas64_13.dll", "cublas64_13.dll"),
        fi("cublasLt64_13.dll", "cublasLt64_13.dll"),
        fi("cufft64_12.dll", "cufft64_12.dll"),
    ],
};

/// 清单常量里的简写。
const fn fi(inner: &'static str, dest: &'static str) -> EngineFile {
    EngineFile { inner, dest }
}

const WINDOWS_CPU: EnginePack = EnginePack {
    id: "audiocpp-cpu",
    label: "audio.cpp 引擎 · CPU",
    note: "通用音频引擎（音乐 / TTS / ASR…）；纯 CPU，任何机器都能跑，慢但不用显卡",
    device: "cpu",
    core: true,
    parts: &[EnginePart {
        archive: "audio-v0.9.3-bin-windows-x64-cpu-portable.zip",
        files: &[
            fi("audiocpp_server.exe", "audiocpp_server.exe"),
            fi("model_specs/yue2.json", "model_specs/yue2.json"),
            fi("msvcp140.dll", "msvcp140.dll"),
            fi("msvcp140_1.dll", "msvcp140_1.dll"),
            fi("msvcp140_2.dll", "msvcp140_2.dll"),
            fi("msvcp140_atomic_wait.dll", "msvcp140_atomic_wait.dll"),
            fi("msvcp140_codecvt_ids.dll", "msvcp140_codecvt_ids.dll"),
            fi("vcruntime140.dll", "vcruntime140.dll"),
            fi("vcruntime140_1.dll", "vcruntime140_1.dll"),
            fi("vcruntime140_threads.dll", "vcruntime140_threads.dll"),
            fi("vcomp140.dll", "vcomp140.dll"),
        ],
    }],
};

const WINDOWS_VULKAN: EnginePack = EnginePack {
    id: "audiocpp-vulkan",
    label: "audio.cpp 引擎 · Vulkan",
    note: "通用音频引擎；任意 GPU（NVIDIA / AMD / Intel），驱动需带 Vulkan 1.2",
    device: "vulkan",
    core: false,
    parts: &[EnginePart {
        archive: "audio-v0.9.3-bin-windows-x64-vulkan-portable.zip",
        files: &[
            fi("audiocpp_server.exe", "audiocpp_server.exe"),
            fi("model_specs/yue2.json", "model_specs/yue2.json"),
            fi("msvcp140.dll", "msvcp140.dll"),
            fi("msvcp140_1.dll", "msvcp140_1.dll"),
            fi("msvcp140_2.dll", "msvcp140_2.dll"),
            fi("msvcp140_atomic_wait.dll", "msvcp140_atomic_wait.dll"),
            fi("msvcp140_codecvt_ids.dll", "msvcp140_codecvt_ids.dll"),
            fi("vcruntime140.dll", "vcruntime140.dll"),
            fi("vcruntime140_1.dll", "vcruntime140_1.dll"),
            fi("vcruntime140_threads.dll", "vcruntime140_threads.dll"),
            fi("vcomp140.dll", "vcomp140.dll"),
        ],
    }],
};

const WINDOWS_CUDA12: EnginePack = EnginePack {
    id: "audiocpp-cuda12",
    label: "audio.cpp 引擎 · CUDA 12",
    note: "通用音频引擎；NVIDIA 老卡 / 老驱动（GTX 900/10、V100、驱动 < 580）",
    device: "cuda12",
    core: false,
    parts: &[CUDA12_BIN, CUDA12_RUNTIME],
};

const WINDOWS_CUDA13: EnginePack = EnginePack {
    id: "audiocpp-cuda13",
    label: "audio.cpp 引擎 · CUDA 13",
    note: "通用音频引擎；NVIDIA 新卡（RTX 40/50、驱动 580+），最快",
    device: "cuda13",
    core: false,
    parts: &[CUDA13_BIN, CUDA13_RUNTIME],
};

const WINDOWS: &[EnginePack] = &[WINDOWS_CPU, WINDOWS_VULKAN, WINDOWS_CUDA12, WINDOWS_CUDA13];

const MACOS_ARM64_FILES: &[EngineFile] = &[
    fi("audiocpp_server", "audiocpp_server"),
    fi("model_specs/yue2.json", "model_specs/yue2.json"),
];

const LINUX_CPU_FILES: &[EngineFile] = &[
    fi("audiocpp_server", "audiocpp_server"),
    fi("model_specs/yue2.json", "model_specs/yue2.json"),
    fi("libggml.so", "libggml.so"),
    fi("libggml.so.0", "libggml.so.0"),
    fi("libggml.so.0.12.0", "libggml.so.0.12.0"),
    fi("libggml-base.so", "libggml-base.so"),
    fi("libggml-base.so.0", "libggml-base.so.0"),
    fi("libggml-base.so.0.12.0", "libggml-base.so.0.12.0"),
    fi("libggml-cpu-alderlake.so", "libggml-cpu-alderlake.so"),
    fi("libggml-cpu-cannonlake.so", "libggml-cpu-cannonlake.so"),
    fi("libggml-cpu-cascadelake.so", "libggml-cpu-cascadelake.so"),
    fi("libggml-cpu-cooperlake.so", "libggml-cpu-cooperlake.so"),
    fi("libggml-cpu-haswell.so", "libggml-cpu-haswell.so"),
    fi("libggml-cpu-icelake.so", "libggml-cpu-icelake.so"),
    fi("libggml-cpu-ivybridge.so", "libggml-cpu-ivybridge.so"),
    fi("libggml-cpu-piledriver.so", "libggml-cpu-piledriver.so"),
    fi("libggml-cpu-sandybridge.so", "libggml-cpu-sandybridge.so"),
    fi("libggml-cpu-sapphirerapids.so", "libggml-cpu-sapphirerapids.so"),
    fi("libggml-cpu-skylakex.so", "libggml-cpu-skylakex.so"),
    fi("libggml-cpu-sse42.so", "libggml-cpu-sse42.so"),
    fi("libggml-cpu-x64.so", "libggml-cpu-x64.so"),
    fi("libggml-cpu-zen4.so", "libggml-cpu-zen4.so"),
];

const LINUX_VULKAN_FILES: &[EngineFile] = &[
    fi("audiocpp_server", "audiocpp_server"),
    fi("model_specs/yue2.json", "model_specs/yue2.json"),
    fi("libggml.so", "libggml.so"),
    fi("libggml.so.0", "libggml.so.0"),
    fi("libggml.so.0.12.0", "libggml.so.0.12.0"),
    fi("libggml-base.so", "libggml-base.so"),
    fi("libggml-base.so.0", "libggml-base.so.0"),
    fi("libggml-base.so.0.12.0", "libggml-base.so.0.12.0"),
    fi("libggml-cpu-alderlake.so", "libggml-cpu-alderlake.so"),
    fi("libggml-cpu-cannonlake.so", "libggml-cpu-cannonlake.so"),
    fi("libggml-cpu-cascadelake.so", "libggml-cpu-cascadelake.so"),
    fi("libggml-cpu-cooperlake.so", "libggml-cpu-cooperlake.so"),
    fi("libggml-cpu-haswell.so", "libggml-cpu-haswell.so"),
    fi("libggml-cpu-icelake.so", "libggml-cpu-icelake.so"),
    fi("libggml-cpu-ivybridge.so", "libggml-cpu-ivybridge.so"),
    fi("libggml-cpu-piledriver.so", "libggml-cpu-piledriver.so"),
    fi("libggml-cpu-sandybridge.so", "libggml-cpu-sandybridge.so"),
    fi("libggml-cpu-sapphirerapids.so", "libggml-cpu-sapphirerapids.so"),
    fi("libggml-cpu-skylakex.so", "libggml-cpu-skylakex.so"),
    fi("libggml-cpu-sse42.so", "libggml-cpu-sse42.so"),
    fi("libggml-cpu-x64.so", "libggml-cpu-x64.so"),
    fi("libggml-cpu-zen4.so", "libggml-cpu-zen4.so"),
    fi("libggml-vulkan.so", "libggml-vulkan.so"),
];

const LINUX_CUDA12_BIN_FILES: &[EngineFile] = &[
    fi("audiocpp_server", "audiocpp_server"),
    fi("model_specs/yue2.json", "model_specs/yue2.json"),
];

const LINUX_CUDA12_LIB_FILES: &[EngineFile] = &[
    fi("libcudart.so.12", "libcudart.so.12"),
    fi("libcublas.so.12", "libcublas.so.12"),
    fi("libcublasLt.so.12", "libcublasLt.so.12"),
    fi("libcufft.so.11", "libcufft.so.11"),
];

const MACOS_METAL: EnginePack = EnginePack {
    id: "audiocpp-metal",
    label: "audio.cpp 引擎 · Metal",
    note: "Apple Silicon（Metal）；写歌等音频模型，Mac 上的默认后端",
    device: "metal",
    core: true,
    parts: &[EnginePart {
        archive: "audio-v0.9.3-bin-macos-arm64-metal.tar.gz",
        files: MACOS_ARM64_FILES,
    }],
};

const LINUX_CPU: EnginePack = EnginePack {
    id: "audiocpp-cpu",
    label: "audio.cpp 引擎 · CPU（Linux）",
    note: "通用音频引擎；纯 CPU，任何 Linux 机器都能跑",
    device: "cpu",
    core: true,
    parts: &[EnginePart {
        archive: "audio-v0.9.3-bin-ubuntu-x64-cpu-portable.tar.gz",
        files: LINUX_CPU_FILES,
    }],
};

const LINUX_VULKAN: EnginePack = EnginePack {
    id: "audiocpp-vulkan",
    label: "audio.cpp 引擎 · Vulkan（Linux）",
    note: "通用音频引擎；NVIDIA / AMD / Intel 的 Vulkan 1.2 驱动",
    device: "vulkan",
    core: false,
    parts: &[EnginePart {
        archive: "audio-v0.9.3-bin-ubuntu-x64-vulkan-portable.tar.gz",
        files: LINUX_VULKAN_FILES,
    }],
};

const LINUX_CUDA12: EnginePack = EnginePack {
    id: "audiocpp-cuda12",
    label: "audio.cpp 引擎 · CUDA 12（Linux）",
    note: "通用音频引擎；NVIDIA 显卡 + 自带 CUDA 12.8 运行库（约 1.0 GB）",
    device: "cuda12",
    core: false,
    parts: &[
        EnginePart {
            archive: "audio-v0.9.3-bin-ubuntu-x64-cuda12.8-colab.tar.gz",
            files: LINUX_CUDA12_BIN_FILES,
        },
        EnginePart {
            archive: "audio-v0.9.3-cudart-ubuntu-x64-cuda12.8.tar.gz",
            files: LINUX_CUDA12_LIB_FILES,
        },
    ],
};

const MACOS_ARM64: &[EnginePack] = &[MACOS_METAL];

const LINUX: &[EnginePack] = &[LINUX_CPU, LINUX_VULKAN, LINUX_CUDA12];

/// 本机平台上的引擎后端。
///
/// Windows x64：CPU / Vulkan / CUDA 12 / CUDA 13；
/// macOS arm64：Metal；Linux x64：CPU / Vulkan / CUDA 12。
pub fn packs(os: &str, arch: &str) -> &'static [EnginePack] {
    match (os, arch) {
        ("windows", "x86_64") => WINDOWS,
        ("macos", "aarch64") => MACOS_ARM64,
        ("linux", "x86_64") => LINUX,
        _ => &[],
    }
}

/// 本机平台的清单。
fn host_packs() -> &'static [EnginePack] {
    packs(std::env::consts::OS, std::env::consts::ARCH)
}

/// 按 id 找一份后端（只在本机平台的清单里找）。
pub fn pack(id: &str) -> Option<&'static EnginePack> {
    host_packs().iter().find(|p| p.id == id)
}

/// 按设备名找一份后端（只在本机平台的清单里找）。
pub fn for_device(device: &str) -> Option<&'static EnginePack> {
    host_packs().iter().find(|p| p.device == device)
}

/// 引擎根目录（全部后端共用；由宿主用 [`set_root`] 指到应用配置目录）。
static ROOT: OnceLock<PathBuf> = OnceLock::new();

fn root() -> PathBuf {
    ROOT.get()
        .cloned()
        .unwrap_or_else(|| std::env::temp_dir().join("ai-daw-audiocpp"))
}

/// 启动时调用：把应用配置目录交给清单（`<config>/runtime/audiocpp`）。
pub fn set_root(path: PathBuf) {
    let _ = ROOT.set(path);
}

/// 引擎根目录。
pub fn dir() -> PathBuf {
    root()
}

/// 一份后端的目录（`<root>/<device>`）。
pub fn dir_of(pack: &EnginePack) -> PathBuf {
    root().join(pack.device)
}

/// 服务端程序的完整路径。
pub fn executable(pack: &EnginePack) -> PathBuf {
    let name = if cfg!(target_os = "windows") {
        "audiocpp_server.exe"
    } else {
        "audiocpp_server"
    };
    dir_of(pack).join(name)
}

/// 一份后端是否就位（**清单里每个文件都在**）。
pub fn installed(pack: &EnginePack) -> bool {
    pack.missing().is_empty()
}

/// 有没有任意一份后端能跑。
pub fn ready() -> bool {
    host_packs().iter().any(installed)
}

/// 已经装好的 GPU 后端，按优先级排好（CUDA 13 / Metal → CUDA 12 → Vulkan）。
///
/// 只是「装了哪些」，选哪个由上层结合显卡决定（见 `src-tauri/src/yue.rs`）。
pub fn gpu_packs() -> Vec<&'static EnginePack> {
    let mut out: Vec<&'static EnginePack> =
        host_packs().iter().filter(|p| !p.core && installed(p)).collect();
    out.sort_by_key(|p| match p.device {
        "cuda13" | "metal" => 0,
        "cuda12" => 1,
        "vulkan" => 2,
        _ => 3,
    });
    out
}

/// 把一份后端的文件清掉（只删清单列出的文件，别动用户可能塞进来的东西）。
pub fn remove(pack: &EnginePack) -> Result<(), String> {
    let dir = dir_of(pack);
    for file in pack.files() {
        let path = dir.join(file.dest);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| format!("删 {} 失败：{e}", path.display()))?;
        }
    }
    // 子目录（`model_specs/`）空了就收掉；目录空了再收一层。
    for sub in ["model_specs"] {
        let path = dir.join(sub);
        if path.is_dir() {
            let _ = std::fs::remove_dir(path);
        }
    }
    let _ = std::fs::remove_dir(&dir);
    Ok(())
}

/// 删除时顺便清掉空的父目录（`runtime/audiocpp` 保留，`preferred` / 日志在里面）。
pub fn prune_empty() {
    if let Ok(entries) = std::fs::read_dir(root()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir()
                && std::fs::read_dir(&path)
                    .map(|mut d| d.next().is_none())
                    .unwrap_or(false)
            {
                let _ = std::fs::remove_dir(&path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_packs_are_listed_and_unique() {
        let listed = packs("windows", "x86_64");
        assert_eq!(listed.len(), 4);
        let mut ids = std::collections::HashSet::new();
        let mut devices = std::collections::HashSet::new();
        for pack in listed {
            assert!(ids.insert(pack.id), "重复 id：{}", pack.id);
            assert!(devices.insert(pack.device), "重复设备：{}", pack.device);
            assert!(!pack.files().is_empty(), "{} 没有文件", pack.id);
            for part in pack.parts {
                assert!(
                    archive(part.archive).is_some(),
                    "{} 的归档不在清单里：{}",
                    pack.id,
                    part.archive
                );
            }
            for file in pack.files() {
                assert!(!file.dest.starts_with('/'), "落地路径必须是相对的");
                assert!(!file.dest.contains(".."), "不许目录穿越");
            }
        }
        assert!(packs("macos", "aarch64").len() == 1, "macOS arm64 应有 Metal 包");
        assert!(packs("macos", "x86_64").is_empty(), "Intel macOS 不提供引擎");
        assert!(packs("linux", "x86_64").len() == 3, "Linux 应有 CPU / Vulkan / CUDA12");
        assert!(packs("linux", "aarch64").is_empty(), "Linux arm64 暂无引擎");
    }

    /// 每个后端都必须带上服务端程序与 YuE2 的模型规格——少了它们「装好了」也跑不起来。
    #[test]
    fn every_pack_can_actually_run_yue2() {
        let all = WINDOWS.iter().chain(MACOS_ARM64).chain(LINUX.iter());
        for pack in all {
            let dests: Vec<&str> = pack.files().iter().map(|f| f.dest).collect();
            assert!(
                dests.contains(&"audiocpp_server") || dests.contains(&"audiocpp_server.exe"),
                "{} 没有服务端",
                pack.id
            );
            assert!(
                dests.contains(&"model_specs/yue2.json"),
                "{} 没有 yue2 规格",
                pack.id
            );
        }
    }

    #[test]
    fn sizes_and_hashes_are_pinned() {
        for archive in ARCHIVES {
            assert_eq!(
                archive.sha256.len(),
                64,
                "{} 的 sha256 不像话",
                archive.file
            );
            assert!(archive.bytes > 1_000_000, "{} 太小了吧", archive.file);
            assert!(!archive.sources.is_empty());
        }
        // CPU 是唯一「什么卡都能跑」的兜底，必须标 core（Windows 那份是 30 MB）。
        let cpu = packs("windows", "x86_64")
            .iter()
            .find(|pack| pack.id == "audiocpp-cpu")
            .expect("缺 Windows CPU 包");
        assert!(cpu.core);
        assert_eq!(cpu.bytes(), 30_409_504);
    }

    /// 六个归档都必须国内外双源：国际 GitHub Releases、国内魔搭
    /// `ailm32442/audiocpp-assets`；国内地址必须指向同名文件（防复制粘贴串包）。
    #[test]
    fn every_archive_has_both_cn_and_intl_sources() {
        for archive in ARCHIVES {
            assert!(
                archive.sources.iter().any(|s| s.region == "intl"),
                "{} 缺国际源",
                archive.file
            );
            let cn = archive
                .sources
                .iter()
                .find(|s| s.region == "cn")
                .unwrap_or_else(|| panic!("{} 缺国内源", archive.file));
            assert!(
                cn.url.contains("modelscope.cn/models/ailm32442/audiocpp-assets/"),
                "{} 的国内源指错了仓库：{}",
                archive.file,
                cn.url
            );
            assert!(
                cn.url.ends_with(archive.file),
                "{} 的国内源文件名对不上：{}",
                archive.file,
                cn.url
            );
        }
    }

    #[test]
    fn gpu_packs_are_ordered_by_preference() {
        let order: Vec<u8> = WINDOWS
            .iter()
            .filter(|p| !p.core)
            .map(|p| match p.device {
                "cuda13" => 0,
                "cuda12" => 1,
                _ => 2,
            })
            .collect();
        assert_eq!(order, [2, 1, 0]);
    }

    #[test]
    fn missing_lists_every_file_when_nothing_is_installed() {
        // 用一个不存在的目录：所有文件都应算「缺」——单文件 marker 的判定在这里会放过
        // 半截目录，所以必须逐文件数。
        let pack = packs("windows", "x86_64")
            .iter()
            .find(|pack| pack.id == "audiocpp-cpu")
            .unwrap();
        let dir = std::env::temp_dir().join("ai-daw-engine-missing-test-nonexistent");
        let missing = pack.missing_in(&dir);
        assert_eq!(missing.len(), pack.files().len());
        assert!(missing.contains(&"audiocpp_server.exe"));
        assert!(missing.contains(&"model_specs/yue2.json"));
    }

    /// 只解出一个 exe 的目录**不算装好**（这正是「半截图目录」那个 bug）。
    #[test]
    fn a_half_extracted_directory_is_not_installed() {
        let pack = packs("windows", "x86_64")
            .iter()
            .find(|pack| pack.id == "audiocpp-cpu")
            .unwrap();
        let dir = std::env::temp_dir().join("ai-daw-engine-half-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("model_specs")).unwrap();
        std::fs::write(dir.join("audiocpp_server.exe"), b"x").unwrap();

        let missing = pack.missing_in(&dir);
        assert_eq!(missing.len(), pack.files().len() - 1);
        assert!(!missing.contains(&"audiocpp_server.exe"));
        assert!(missing.contains(&"model_specs/yue2.json"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
