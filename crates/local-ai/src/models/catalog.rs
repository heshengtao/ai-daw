//! **模型清单**（编译期内嵌）：每个本地 AI 模型的用途、体积、许可、落地文件与下载源。
//!
//! 清单是「有哪些模型、从哪下、对不对得上」的唯一来源。每个文件的 `sha256` 是硬门禁——
//! 第三方镜像 / 加速站下载的字节也要先校验再落地（与插件管理器 E1 同一套安全立场）。
//!
//! 一个条目是**一组文件**（[`ModelFile`]）而不是单个文件：多数模型只有一个权重，
//! 但「一张图 + 一个 `.onnx.data` 外部权重」这类多组件管线会有好几个文件、缺一不可。
//! 条目用 [`ModelEntry::dir`] 决定落地子目录：
//! - `dir: None` —— 直接落在 `models/` 下（单文件模型，落点与历史版本一致）；
//! - `dir: Some("some-model")` —— 落在 `models/some-model/`，
//!   这样同名文件（以及 `.onnx` 与它的 `.onnx.data`）不会互相踩，`ort` 也才能按相对路径
//!   找到外部权重。
//!
//! 分两类：
//! - **随包小模型**（`bundled = true`，Basic Pitch / LocalVQE）：由 `bundle.resources` 打进
//!   安装包，开箱可用；清单里仍留源，便于界面显示「可重新下载」。
//! - **按需下载的大模型**（`bundled = false`，如音源分离 htdemucs）：
//!   不随包，用户到「管理器 → 模型」里下到应用配置目录的 `models/`。
//!
//! 魔搭（ModelScope）源**暂缺**：htdemucs 在魔搭上还没有对应仓库，等自建后再补 `modelscope`
//! 条目。清单里没有的源就不写，缺源时界面显示「此源不可用」而不是一个死链。

use std::path::PathBuf;

/// 一个下载源。
#[derive(Debug, Clone, Copy)]
pub struct ModelSource {
    /// 源类型：`hf` / `modelscope` / `custom`。清单里**只放官方源**，不写第三方镜像。
    pub kind: &'static str,
    /// 源所在区域：`intl`（国外直连友好，如 HF / GitHub）/ `cn`（国内友好，如魔搭）。
    ///
    /// 下载时按用户网络连通性把「偏好区域」的源排到前面，失败再回退另一区域（见
    /// `src-tauri/src/models.rs`）。
    pub region: &'static str,
    /// 直链地址。
    pub url: &'static str,
}

/// 组里的一个文件。
///
/// `sha256` 挂在文件上（而不是每个源上）——同一个文件的各源**本来就是同一份字节**，
/// 把它放在两处只会给「换源后哈希对不上」留口子。
#[derive(Debug, Clone, Copy)]
pub struct ModelFile {
    /// 落地文件名（含 `.onnx.data` 这类外部权重）。
    pub filename: &'static str,
    /// 预估体积（字节），下载前告知 / 磁盘预检用。
    pub approx_bytes: u64,
    /// 该文件的 sha256（小写十六进制）。
    pub sha256: &'static str,
    /// 有序候选源（按顺序尝试，第一个成功即止）。
    pub sources: &'static [ModelSource],
}

/// 清单里的一个模型（= 一组文件）。
#[derive(Debug, Clone, Copy)]
pub struct ModelEntry {
    /// 稳定标识。
    pub id: &'static str,
    /// 展示名。
    pub name: &'static str,
    /// 所属功能（`to_midi` / `enhance` / `separate` / `detect_tempo` …）。
    pub feature: &'static str,
    /// SPDX 许可。
    pub license: &'static str,
    /// 落地子目录；`None` = 直接落在 `models/` 下。
    pub dir: Option<&'static str>,
    /// 是否随安装包分发。
    pub bundled: bool,
    /// 组内全部文件（**缺一不可**，全部在位才算装好）。
    pub files: &'static [ModelFile],
}

impl ModelEntry {
    /// 整组体积（下载前告知 / 磁盘预检用）。
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.approx_bytes).sum()
    }

    /// 代表文件（界面展示用；组条目取第一个）。
    pub fn primary(&self) -> Option<&'static ModelFile> {
        self.files.first()
    }
}

/// YuE2 权重的一条文件：`path` 是仓库内路径（`sidecars/...` 也要写全），两边仓库同名。
///
/// 官方整理的那一份在 HF / 魔搭都叫 `audio-cpp/Yue2-3B-GGUF`，所以两条源只是域名不同。
macro_rules! yue2_file {
    ($path:literal, $size:expr, $sha:literal) => {
        ModelFile {
            filename: $path,
            approx_bytes: $size,
            sha256: $sha,
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: concat!(
                        "https://huggingface.co/audio-cpp/Yue2-3B-GGUF/resolve/main/",
                        $path
                    ),
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: concat!(
                        "https://modelscope.cn/models/audio-cpp/Yue2-3B-GGUF/resolve/master/",
                        $path
                    ),
                },
            ],
        }
    };
}

/// SheetSage2（听音记谱）的文件：仓库内只有一个 GGUF，HF / 魔搭同名。
macro_rules! sheetsage2_file {
    ($path:literal, $size:expr, $sha:literal) => {
        ModelFile {
            filename: $path,
            approx_bytes: $size,
            sha256: $sha,
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: concat!(
                        "https://huggingface.co/audio-cpp/SheetSage2-GGUF/resolve/main/",
                        $path
                    ),
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: concat!(
                        "https://modelscope.cn/models/audio-cpp/SheetSage2-GGUF/resolve/master/",
                        $path
                    ),
                },
            ],
        }
    };
}

/// `audio-cpp/audio.cpp-gguf` 里的一条 GGUF（HF / 魔搭同路径、字节一致）。
///
/// `$path` 是**仓库内路径**，`$name` 是**落地文件名**——同一个模型族的权重拉平放进
/// `models/<family>/`，换档时不用再建一层目录。
macro_rules! audio_cpp_file {
    ($name:literal, $path:literal, $size:expr, $sha:literal) => {
        ModelFile {
            filename: $name,
            approx_bytes: $size,
            sha256: $sha,
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: concat!(
                        "https://huggingface.co/audio-cpp/audio.cpp-gguf/resolve/main/",
                        $path
                    ),
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: concat!(
                        "https://modelscope.cn/models/audio-cpp/audio.cpp-gguf/resolve/master/",
                        $path
                    ),
                },
            ],
        }
    };
}

/// ACE-Step 权重的一条文件。
///
/// `$path` 是**仓库内路径**（`ACE-Step1.5-GGUF/turbo/...`），`$name` 是**落地文件名**
/// ——两个 GGUF 拉平放进 `models/ace-step/`，换档时不用再建一层目录。
/// 官方整理的那一份在 HF / 魔搭都叫 `audio-cpp/audio.cpp-gguf`，所以两条源只是域名不同。
macro_rules! ace_step_file {
    ($name:literal, $path:literal, $size:expr, $sha:literal) => {
        audio_cpp_file!($name, $path, $size, $sha)
    };
}

/// 清单。
pub static CATALOG: &[ModelEntry] = &[    ModelEntry {
        id: "basic-pitch",
        name: "Basic Pitch（音频转 MIDI）",
        feature: "to_midi",
        license: "Apache-2.0",
        dir: None,
        bundled: true,
        files: &[ModelFile {
            filename: "basic-pitch-nmp.onnx",
            approx_bytes: 230_444,
            sha256: "2c3c1d144bfa61ad236e92e169c13535c880469a12a047d4e73451f2c059a0ec",
            sources: &[ModelSource {
                kind: "hf",
                region: "intl",
                url: "https://huggingface.co/AEmotionStudio/basic-pitch-onnx-models/resolve/main/nmp.onnx",
            }],
        }],
    },
    // MuScriptor（多乐器音频 → MIDI）：audio.cpp 的 GGUF 转换，由 `audiocpp` 引擎以
    // `--task midi --family muscriptor` 调用。权重是 **CC BY-NC 4.0（非商用）**，
    // 不随包、按需下载——与 YuE2 同一套姿态。
    //
    // 仓库里的目录叫 `MuScriptor-Small-GGUF/`，但 **small / medium / large 三档都在里面**
    // （别被目录名骗了，这正是不按目录名遍历、而是逐文件列出的原因）。
    ModelEntry {
        id: "muscriptor-small",
        name: "MuScriptor Small（多乐器音频转 MIDI · 393 MB）",
        feature: "to_midi",
        license: "CC-BY-NC-4.0",
        dir: Some("muscriptor"),
        bundled: false,
        files: &[audio_cpp_file!(
            "muscriptor-small-f32.gguf",
            "MuScriptor-Small-GGUF/muscriptor-small-f32.gguf",
            411_908_672,
            "4bff6f1c45b445e144c176434fbe27614ad6c4686e03f99ee11b17e15163fba2"
        )],
    },
    ModelEntry {
        id: "muscriptor-medium",
        name: "MuScriptor Medium（多乐器音频转 MIDI · 1.1 GB）",
        feature: "to_midi",
        license: "CC-BY-NC-4.0",
        dir: Some("muscriptor"),
        bundled: false,
        files: &[audio_cpp_file!(
            "muscriptor-medium-f32.gguf",
            "MuScriptor-Small-GGUF/muscriptor-medium-f32.gguf",
            1_228_154_944,
            "24302a7d9c6a78c1db2ca0d29c6e48c9fddbf35ff937f56358b0b125d2250ce9"
        )],
    },
    ModelEntry {
        id: "muscriptor-large",
        name: "MuScriptor Large（多乐器音频转 MIDI · 5.1 GB）",
        feature: "to_midi",
        license: "CC-BY-NC-4.0",
        dir: Some("muscriptor"),
        bundled: false,
        files: &[audio_cpp_file!(
            "muscriptor-large-f32.gguf",
            "MuScriptor-Small-GGUF/muscriptor-large-f32.gguf",
            5_465_656_576,
            "570e1048c5d9c01c6a8f698d5c0bfd9d4d5927a20bfbf2bb0c2b99f17bb9c971"
        )],
    },
    ModelEntry {
        id: "localvqe",
        name: "LocalVQE（人声降噪 / 去混响）",
        feature: "enhance",
        license: "Apache-2.0",
        dir: None,
        bundled: true,
        // 上游只发 `.pt` / GGUF（Apache-2.0）；ONNX 由 `tools/export-localvqe-onnx.py` 本地导出，
        // 再自建镜像（HF `ailm32442/localvqe-ns-onnx`）。随包分发，同时也支持重新下载。
        files: &[ModelFile {
            filename: "localvqe-ns.onnx",
            approx_bytes: 6_152_621,
            sha256: "268339d31dd427cbe5cf9107028185b7a473b5ae0cbc59f1ba39187767367060",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/localvqe-ns-onnx/resolve/main/localvqe-ns.onnx",
                },
            ],
        }],
    },
    ModelEntry {
        id: "htdemucs-ort-v1",
        name: "HTDemucs（音源分离 · 4 分轨）",
        feature: "separate",
        // 权重来自 Meta 的 Demucs v4（htdemucs）——上游是 **MIT**，不是 Apache。
        license: "MIT",
        dir: None,
        bundled: false,
        files: &[ModelFile {
            filename: "htdemucs.ort",
            approx_bytes: 209_884_896,
            sha256: "09dc165512d8ef7480bcb2cacea9dda82d571f8dbf421d8c44a2ca5568bec729",
            sources: &[
                // 国内优先走魔搭（自建镜像；字节与 HF 完全一致）。
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/ailm32442/htdemucs-ort/resolve/master/htdemucs.ort",
                },
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/gentij/htdemucs-ort/resolve/main/htdemucs.ort",
                },
            ],
        }],
    },
    ModelEntry {
        id: "htdemucs-6s",
        name: "HTDemucs 6s（音源分离 · 6 分轨）",
        feature: "separate",
        // 同为 Meta Demucs v4 家族的 `htdemucs_6s`（**MIT**）；这份是自足 ONNX 导出
        // （输入原始波形 `mix`、直接输出 `stems`，不需要我们算 STFT）。
        license: "MIT",
        dir: None,
        bundled: false,
        files: &[ModelFile {
            filename: "htdemucs_6s.onnx",
            approx_bytes: 258_159_781,
            sha256: "48f8e84945579f8ab340e083339e9221e03785dbe733a52c388200b6d3ca779a",
            sources: &[
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/ailm32442/htdemucs-ort/resolve/master/htdemucs_6s.onnx",
                },
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/StemSplitio/htdemucs-6s-onnx/resolve/main/htdemucs_6s.onnx",
                },
            ],
        }],
    },
    ModelEntry {
        id: "beat-this",
        name: "Beat This!（节拍 / 速度检测 · final0）",
        feature: "detect_tempo",
        // Beat This! 本体（JKU Linz, ISMIR 2024）是 **MIT**。
        license: "MIT",
        dir: None,
        // 大模型不随包：final0（FP32，~83 MB）由用户在「管理器 → 模型」按需下载。
        // 另一半——log-mel 前端（264 KB）——随安装包分发，因此不在清单里。
        bundled: false,
        files: &[ModelFile {
            filename: "beat_this.onnx",
            approx_bytes: 83_162_650,
            sha256: "5f810debe53459b559127fb55bbad40035bb47cc567b20e501670f968c770f02",
            sources: &[
                // 国内走魔搭自建镜像（与 GitHub Release 字节一致）。
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/ailm32442/beat_this.onnx/resolve/master/beat_this.onnx",
                },
                // 上游把 FP32 权重单独放在 GitHub Releases 的 `model-large` 里（crate 本身不含权重）。
                ModelSource {
                    kind: "custom",
                    region: "intl",
                    url: "https://github.com/danigb/beat-this-rs/releases/download/model-large/beat_this.onnx",
                },
            ],
        }],
    },
    ModelEntry {
        id: "soulx-singer-fp16",
        name: "SoulX-Singer（歌声合成 · fp16）",
        feature: "sing",
        // 上游模型 SoulX-Singer（Soul AI Lab）是 **Apache-2.0**；ONNX 拆分导出由社区完成，
        // 以同一许可再分发。
        license: "Apache-2.0",
        // 9 张图 + 5 个外部权重必须同目录：`ort` 靠 `.onnx` 里记录的**相对路径**找 `.data`。
        dir: Some("soulx-singer-fp16"),
        // 1.4 GB：绝不随包，用户在「管理器 → 模型」按需下载。
        bundled: false,
        // **国内外双源**：国际走 Hugging Face 镜像（`ailm32442/SoulX-Singer-onnx-fp16`，
        // 由 `tools/mirror-soulx-hf.sh` 上传，逐文件 sha256 与魔搭完全一致），国内走魔搭。
        // 下载器按网络位置排序（设置 → 网络 → 模型下载源：自动 / 国外 / 国内），失败再换另一个源。
        // 这里的 sha256 取自魔搭的 LFS 元数据，换源后字节必须一致，否则校验不过。
        files: &[
        ModelFile {
            filename: "note_text_encoder.onnx",
            approx_bytes: 3072460,
            sha256: "331415477a985a8962601a465c616806b5d32a33f5f93230e941a54656d352c6",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp16/resolve/main/note_text_encoder.onnx",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml-fp16/resolve/master/note_text_encoder.onnx",
                },
            ],
        },
        ModelFile {
            filename: "note_text_encoder.onnx.data",
            approx_bytes: 6144000,
            sha256: "998889d7daff9cd50774abf23282d1acad01d2c753db3040b156d1a53b178e79",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp16/resolve/main/note_text_encoder.onnx.data",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml-fp16/resolve/master/note_text_encoder.onnx.data",
                },
            ],
        },
        ModelFile {
            filename: "note_pitch_encoder.onnx",
            approx_bytes: 262601,
            sha256: "741b08a2767e9134cd40d4e5b4e073590c2f74cd6dca0c107c7a7715965a346a",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp16/resolve/main/note_pitch_encoder.onnx",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml-fp16/resolve/master/note_pitch_encoder.onnx",
                },
            ],
        },
        ModelFile {
            filename: "note_pitch_encoder.onnx.data",
            approx_bytes: 524288,
            sha256: "20ddcaaea04315e5a46eb11d8cacb6dd6ecf6a9688ebc3af019ab7b27ee7b9c6",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp16/resolve/main/note_pitch_encoder.onnx.data",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml-fp16/resolve/master/note_pitch_encoder.onnx.data",
                },
            ],
        },
        ModelFile {
            filename: "note_type_encoder.onnx",
            approx_bytes: 262601,
            sha256: "0aba568ee59c350175663bdc04a40be44fe36fc46ceb648f462d65638bbd85b4",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp16/resolve/main/note_type_encoder.onnx",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml-fp16/resolve/master/note_type_encoder.onnx",
                },
            ],
        },
        ModelFile {
            filename: "note_type_encoder.onnx.data",
            approx_bytes: 524288,
            sha256: "becf918cd5e23b25783ae42480c13bfa04bf38b84a4075d7c4273befcb3af4a0",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp16/resolve/main/note_type_encoder.onnx.data",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml-fp16/resolve/master/note_type_encoder.onnx.data",
                },
            ],
        },
        ModelFile {
            filename: "f0_encoder.onnx",
            approx_bytes: 370121,
            sha256: "c7628a2af8b0347a28edbbb2d35bd8a12eb13aa1b138ae4ed267d0e08bb3a046",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp16/resolve/main/f0_encoder.onnx",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml-fp16/resolve/master/f0_encoder.onnx",
                },
            ],
        },
        ModelFile {
            filename: "f0_encoder.onnx.data",
            approx_bytes: 739328,
            sha256: "8c3f3c514b1bf78f723b8fa785ed2f7dd5cb994ca2c1349f22c54ea4909b0f4b",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp16/resolve/main/f0_encoder.onnx.data",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml-fp16/resolve/master/f0_encoder.onnx.data",
                },
            ],
        },
        ModelFile {
            filename: "preflow.onnx",
            approx_bytes: 8601140,
            sha256: "d06b7274491a2346194654e17297e63823a10ccc2e21dd39540f4057ffe62abe",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp16/resolve/main/preflow.onnx",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml-fp16/resolve/master/preflow.onnx",
                },
            ],
        },
        ModelFile {
            filename: "preflow.onnx.data",
            approx_bytes: 16973824,
            sha256: "ce2c424a3a22a22a1aff172767baacc3e6796df0eb83f979596d522d87d2422f",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp16/resolve/main/preflow.onnx.data",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml-fp16/resolve/master/preflow.onnx.data",
                },
            ],
        },
        ModelFile {
            filename: "cond_emb.onnx",
            approx_bytes: 1051289,
            sha256: "3437e166b1dfd5094215565566fd146e71fa29ac2cf50653831a5e6da60deb88",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp16/resolve/main/cond_emb.onnx",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml-fp16/resolve/master/cond_emb.onnx",
                },
            ],
        },
        ModelFile {
            filename: "cond_emb.onnx.data",
            approx_bytes: 2162688,
            sha256: "fbfa95df1617d8dc12c1544c788be65d5bb08b365a68041c69953b02ac066dbe",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp16/resolve/main/cond_emb.onnx.data",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml-fp16/resolve/master/cond_emb.onnx.data",
                },
            ],
        },
        ModelFile {
            filename: "mel_transform.onnx",
            approx_bytes: 261055,
            sha256: "823a41e7b50506c28afc0d954823f27717fe47b50304012eec36daaa7d22685c",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp16/resolve/main/mel_transform.onnx",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml-fp16/resolve/master/mel_transform.onnx",
                },
            ],
        },
        ModelFile {
            filename: "mel_transform.onnx.data",
            approx_bytes: 499712,
            sha256: "2d1e18edf37041270f999b15bbfd08c19cccca723a6111425ae9a8a228c87927",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp16/resolve/main/mel_transform.onnx.data",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml-fp16/resolve/master/mel_transform.onnx.data",
                },
            ],
        },
        ModelFile {
            filename: "diff_step_dml.onnx",
            approx_bytes: 887377148,
            sha256: "1a2857f819562e75b73b291ea86ec3c1157d77f6e51abad1e934d2f318711629",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp16/resolve/main/diff_step_dml.onnx",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml-fp16/resolve/master/diff_step_dml.onnx",
                },
            ],
        },
        ModelFile {
            filename: "vocoder_dml.onnx",
            approx_bytes: 519486458,
            sha256: "185a707f907554196f28848256ecfd1b72031c5335571e2dedcbb0a0de35db95",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp16/resolve/main/vocoder_dml.onnx",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml-fp16/resolve/master/vocoder_dml.onnx",
                },
            ],
        },
        ],
    },
    ModelEntry {
        id: "soulx-singer-fp32",
        name: "SoulX-Singer（歌声合成 · fp32 · 高精度）",
        feature: "sing",
        // 与 fp16 同源同导出：上游 SoulX-Singer（Soul AI Lab）Apache-2.0，ONNX 拆分导出
        // 由社区完成。这一档只把 `diff_step` / `vocoder` 保留 fp32（2.85 GB vs 1.44 GB），
        // 编码器的外部权重两份导出本来字节相同（见 `soulx_singer_precisions_share_encoder_weights`）。
        license: "Apache-2.0",
        dir: Some("soulx-singer-fp32"),
        bundled: false,
        // **国内外双源**：国内走魔搭（`syxppp/SoulX-Singer-onnx-directml`，注意这条没有 `-fp16`
        // 后缀），国际走 HF（`ailm32442/SoulX-Singer-onnx-fp32`，由 `tools/mirror-soulx-hf.sh
        // --fp32` 上传，逐文件 sha256 与魔搭完全一致）。下载器按网络位置排序，失败再换另一个源。
        files: &[
        ModelFile {
            filename: "note_text_encoder.onnx",
            approx_bytes: 433,
            sha256: "087db35205ce93dae9e2c65e0628511266ffbc21beb2c128d80793c150d66714",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp32/resolve/main/note_text_encoder.onnx",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml/resolve/master/note_text_encoder.onnx",
                },
            ],
        },
        ModelFile {
            filename: "note_text_encoder.onnx.data",
            approx_bytes: 6144000,
            sha256: "998889d7daff9cd50774abf23282d1acad01d2c753db3040b156d1a53b178e79",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp32/resolve/main/note_text_encoder.onnx.data",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml/resolve/master/note_text_encoder.onnx.data",
                },
            ],
        },
        ModelFile {
            filename: "note_pitch_encoder.onnx",
            approx_bytes: 433,
            sha256: "90f9ef1cd74e532e1cc89802888bedcf311bd835460b09e27c3eef1be9489bdb",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp32/resolve/main/note_pitch_encoder.onnx",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml/resolve/master/note_pitch_encoder.onnx",
                },
            ],
        },
        ModelFile {
            filename: "note_pitch_encoder.onnx.data",
            approx_bytes: 524288,
            sha256: "20ddcaaea04315e5a46eb11d8cacb6dd6ecf6a9688ebc3af019ab7b27ee7b9c6",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp32/resolve/main/note_pitch_encoder.onnx.data",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml/resolve/master/note_pitch_encoder.onnx.data",
                },
            ],
        },
        ModelFile {
            filename: "note_type_encoder.onnx",
            approx_bytes: 432,
            sha256: "869d4d50a0ff5566346e72cf4fb36c110a52e10ef7292c53aecd008c3f74d594",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp32/resolve/main/note_type_encoder.onnx",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml/resolve/master/note_type_encoder.onnx",
                },
            ],
        },
        ModelFile {
            filename: "note_type_encoder.onnx.data",
            approx_bytes: 524288,
            sha256: "becf918cd5e23b25783ae42480c13bfa04bf38b84a4075d7c4273befcb3af4a0",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp32/resolve/main/note_type_encoder.onnx.data",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml/resolve/master/note_type_encoder.onnx.data",
                },
            ],
        },
        ModelFile {
            filename: "f0_encoder.onnx",
            approx_bytes: 425,
            sha256: "f63a5f8da66af2a50b6c3463f8e8a0b52022f6f14bbb79d7364149da2c576a0f",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp32/resolve/main/f0_encoder.onnx",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml/resolve/master/f0_encoder.onnx",
                },
            ],
        },
        ModelFile {
            filename: "f0_encoder.onnx.data",
            approx_bytes: 739328,
            sha256: "8c3f3c514b1bf78f723b8fa785ed2f7dd5cb994ca2c1349f22c54ea4909b0f4b",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp32/resolve/main/f0_encoder.onnx.data",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml/resolve/master/f0_encoder.onnx.data",
                },
            ],
        },
        ModelFile {
            filename: "preflow.onnx",
            approx_bytes: 145261,
            sha256: "af0680d7356ba1131deb5a1801971934344f7ccd5394c875a612f8a66c8e1a60",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp32/resolve/main/preflow.onnx",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml/resolve/master/preflow.onnx",
                },
            ],
        },
        ModelFile {
            filename: "preflow.onnx.data",
            approx_bytes: 16973824,
            sha256: "ce2c424a3a22a22a1aff172767baacc3e6796df0eb83f979596d522d87d2422f",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp32/resolve/main/preflow.onnx.data",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml/resolve/master/preflow.onnx.data",
                },
            ],
        },
        ModelFile {
            filename: "cond_emb.onnx",
            approx_bytes: 689,
            sha256: "6825d68ea4d42bc83ca66dacf3a6440f5f2593af2a8ca40f956fdcde19559e54",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp32/resolve/main/cond_emb.onnx",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml/resolve/master/cond_emb.onnx",
                },
            ],
        },
        ModelFile {
            filename: "cond_emb.onnx.data",
            approx_bytes: 2162688,
            sha256: "fbfa95df1617d8dc12c1544c788be65d5bb08b365a68041c69953b02ac066dbe",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp32/resolve/main/cond_emb.onnx.data",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml/resolve/master/cond_emb.onnx.data",
                },
            ],
        },
        ModelFile {
            filename: "mel_transform.onnx",
            approx_bytes: 11263,
            sha256: "5652b9b8a46207413db3d2ff65139d78ab502d60fa7eb3a1a660ec8296d1d885",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp32/resolve/main/mel_transform.onnx",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml/resolve/master/mel_transform.onnx",
                },
            ],
        },
        ModelFile {
            filename: "mel_transform.onnx.data",
            approx_bytes: 499712,
            sha256: "2d1e18edf37041270f999b15bbfd08c19cccca723a6111425ae9a8a228c87927",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp32/resolve/main/mel_transform.onnx.data",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml/resolve/master/mel_transform.onnx.data",
                },
            ],
        },
        ModelFile {
            filename: "diff_step_dml.onnx",
            approx_bytes: 1772509288,
            sha256: "bc6e95474793a0eb7dbf8ea4c7cd3820593f6bd5d136f222ea58684b64b07c13",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp32/resolve/main/diff_step_dml.onnx",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml/resolve/master/diff_step_dml.onnx",
                },
            ],
        },
        ModelFile {
            filename: "vocoder_dml.onnx",
            approx_bytes: 1053635812,
            sha256: "8df53cf50ff7f91208fe613f5f777690a2b9ddb9d17fa59bc6efa0fdbf1e8535",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/ailm32442/SoulX-Singer-onnx-fp32/resolve/main/vocoder_dml.onnx",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/syxppp/SoulX-Singer-onnx-directml/resolve/master/vocoder_dml.onnx",
                },
            ],
        },
        ],
    },
    ModelEntry {
        id: "soulx-svc-fp16hq",
        name: "SoulX-Singer-SVC（音色替换 · fp16_hq · 推荐）",
        feature: "voice_swap",
        // 上游 SoulX-Singer-SVC 权重来自 Soul AI Lab 的 SoulX-Singer（Apache-2.0）；
        // ONNX 拆分导出（7 张自足图）由社区完成，以同一许可再分发。
        license: "Apache-2.0",
        // 7 张图必须同目录：其中 whisper / mel / f0 / cond 带外部权重（见下）。
        dir: Some("soulx-svc-fp16hq"),
        // 2.4 GB：不随包，用户在「管理器 → 模型」按需下载。
        bundled: false,
        // **国内外双源**：HF 仓库 `ailm32442/SoulX-Singer-SVC-ONNX` 与魔搭同名仓库，
        // 目录名（onnx_fp16_hq）就是导出的原始布局，下载器按网络位置排序后逐文件校验 sha256。
        files: &[
            ModelFile {
                filename: "whisper.frontend.onnx",
                approx_bytes: 85_087_790,
                sha256: "3573860a14b10a9fbc5960472ade57d66512ba4434da298eb97099a9d500f78d",
                sources: &[
                    ModelSource {
                        kind: "hf",
                        region: "intl",
                        url: "https://huggingface.co/ailm32442/SoulX-Singer-SVC-ONNX/resolve/main/onnx_fp16_hq/whisper.frontend.onnx",
                    },
                    ModelSource {
                        kind: "modelscope",
                        region: "cn",
                        url: "https://modelscope.cn/models/ailm32442/SoulX-Singer-SVC-ONNX/resolve/master/onnx_fp16_hq/whisper.frontend.onnx",
                    },
                ],
            },
            ModelFile {
                filename: "mel.transform.onnx",
                approx_bytes: 15_256_211,
                sha256: "94b336ea1278b3b0bc38f283c1aa3e664e4fdfb1f36b0524420a72c60e81d2c1",
                sources: &[
                    ModelSource {
                        kind: "hf",
                        region: "intl",
                        url: "https://huggingface.co/ailm32442/SoulX-Singer-SVC-ONNX/resolve/main/onnx_fp16_hq/mel.transform.onnx",
                    },
                    ModelSource {
                        kind: "modelscope",
                        region: "cn",
                        url: "https://modelscope.cn/models/ailm32442/SoulX-Singer-SVC-ONNX/resolve/master/onnx_fp16_hq/mel.transform.onnx",
                    },
                ],
            },
            ModelFile {
                filename: "f0.encoder.onnx",
                approx_bytes: 742_014,
                sha256: "b55a1d117d062dc6d31cf67b15627fc12dfdf593363966e68086d5ec1b2c146c",
                sources: &[
                    ModelSource {
                        kind: "hf",
                        region: "intl",
                        url: "https://huggingface.co/ailm32442/SoulX-Singer-SVC-ONNX/resolve/main/onnx_fp16_hq/f0.encoder.onnx",
                    },
                    ModelSource {
                        kind: "modelscope",
                        region: "cn",
                        url: "https://modelscope.cn/models/ailm32442/SoulX-Singer-SVC-ONNX/resolve/master/onnx_fp16_hq/f0.encoder.onnx",
                    },
                ],
            },
            ModelFile {
                filename: "cond.emb.onnx",
                approx_bytes: 1_051_173,
                sha256: "5cdc78d193988c3ad66d6897497e6f30b0b19876f1f44f2a2e64b6039db48302",
                sources: &[
                    ModelSource {
                        kind: "hf",
                        region: "intl",
                        url: "https://huggingface.co/ailm32442/SoulX-Singer-SVC-ONNX/resolve/main/onnx_fp16_hq/cond.emb.onnx",
                    },
                    ModelSource {
                        kind: "modelscope",
                        region: "cn",
                        url: "https://modelscope.cn/models/ailm32442/SoulX-Singer-SVC-ONNX/resolve/master/onnx_fp16_hq/cond.emb.onnx",
                    },
                ],
            },
            ModelFile {
                filename: "dit.step.onnx",
                approx_bytes: 1_770_819_432,
                sha256: "8465a04327719c2965645aa3291a04aebffdf94f16fd61572feb6ebb6dbece2f",
                sources: &[
                    ModelSource {
                        kind: "hf",
                        region: "intl",
                        url: "https://huggingface.co/ailm32442/SoulX-Singer-SVC-ONNX/resolve/main/onnx_fp16_hq/dit.step.onnx",
                    },
                    ModelSource {
                        kind: "modelscope",
                        region: "cn",
                        url: "https://modelscope.cn/models/ailm32442/SoulX-Singer-SVC-ONNX/resolve/master/onnx_fp16_hq/dit.step.onnx",
                    },
                ],
            },
            ModelFile {
                filename: "vocoder.backbone.onnx",
                approx_bytes: 510_185_830,
                sha256: "3496a8100e0726393e5512fb90ec37c9cecaab96052d5047cde0fbf692bd6a37",
                sources: &[
                    ModelSource {
                        kind: "hf",
                        region: "intl",
                        url: "https://huggingface.co/ailm32442/SoulX-Singer-SVC-ONNX/resolve/main/onnx_fp16_hq/vocoder.backbone.onnx",
                    },
                    ModelSource {
                        kind: "modelscope",
                        region: "cn",
                        url: "https://modelscope.cn/models/ailm32442/SoulX-Singer-SVC-ONNX/resolve/master/onnx_fp16_hq/vocoder.backbone.onnx",
                    },
                ],
            },
            ModelFile {
                filename: "vocoder.istft.onnx",
                approx_bytes: 14_772_514,
                sha256: "4647fef6f047d14bc198c5feb3a4a755384d6a5accb07f333add45e87a88a517",
                sources: &[
                    ModelSource {
                        kind: "hf",
                        region: "intl",
                        url: "https://huggingface.co/ailm32442/SoulX-Singer-SVC-ONNX/resolve/main/onnx_fp16_hq/vocoder.istft.onnx",
                    },
                    ModelSource {
                        kind: "modelscope",
                        region: "cn",
                        url: "https://modelscope.cn/models/ailm32442/SoulX-Singer-SVC-ONNX/resolve/master/onnx_fp16_hq/vocoder.istft.onnx",
                    },
                ],
            },
        ],
    },
    ModelEntry {
        id: "soulx-svc-fp32",
        name: "SoulX-Singer-SVC（音色替换 · fp32 · 最高保真）",
        feature: "voice_swap",
        license: "Apache-2.0",
        dir: Some("soulx-svc-fp32"),
        bundled: false,
        // 与 hq 档同源同导出：扩散步（dit.step）字节完全相同，差别只在声码器与条件嵌入。
        // 全 fp32 约 3.0 GB，只给确实需要最高保真的用户。
        files: &[
            ModelFile {
                filename: "whisper.frontend.onnx",
                approx_bytes: 85_087_790,
                sha256: "3573860a14b10a9fbc5960472ade57d66512ba4434da298eb97099a9d500f78d",
                sources: &[
                    ModelSource {
                        kind: "hf",
                        region: "intl",
                        url: "https://huggingface.co/ailm32442/SoulX-Singer-SVC-ONNX/resolve/main/onnx_fp32/whisper.frontend.onnx",
                    },
                    ModelSource {
                        kind: "modelscope",
                        region: "cn",
                        url: "https://modelscope.cn/models/ailm32442/SoulX-Singer-SVC-ONNX/resolve/master/onnx_fp32/whisper.frontend.onnx",
                    },
                ],
            },
            ModelFile {
                filename: "mel.transform.onnx",
                approx_bytes: 15_256_211,
                sha256: "94b336ea1278b3b0bc38f283c1aa3e664e4fdfb1f36b0524420a72c60e81d2c1",
                sources: &[
                    ModelSource {
                        kind: "hf",
                        region: "intl",
                        url: "https://huggingface.co/ailm32442/SoulX-Singer-SVC-ONNX/resolve/main/onnx_fp32/mel.transform.onnx",
                    },
                    ModelSource {
                        kind: "modelscope",
                        region: "cn",
                        url: "https://modelscope.cn/models/ailm32442/SoulX-Singer-SVC-ONNX/resolve/master/onnx_fp32/mel.transform.onnx",
                    },
                ],
            },
            ModelFile {
                filename: "f0.encoder.onnx",
                approx_bytes: 742_014,
                sha256: "b55a1d117d062dc6d31cf67b15627fc12dfdf593363966e68086d5ec1b2c146c",
                sources: &[
                    ModelSource {
                        kind: "hf",
                        region: "intl",
                        url: "https://huggingface.co/ailm32442/SoulX-Singer-SVC-ONNX/resolve/main/onnx_fp32/f0.encoder.onnx",
                    },
                    ModelSource {
                        kind: "modelscope",
                        region: "cn",
                        url: "https://modelscope.cn/models/ailm32442/SoulX-Singer-SVC-ONNX/resolve/master/onnx_fp32/f0.encoder.onnx",
                    },
                ],
            },
            ModelFile {
                filename: "cond.emb.onnx",
                approx_bytes: 2_101_542,
                sha256: "d3e35a2942ef0203c1fa6f3792d142044487be161363a51d867dcab0273af20f",
                sources: &[
                    ModelSource {
                        kind: "hf",
                        region: "intl",
                        url: "https://huggingface.co/ailm32442/SoulX-Singer-SVC-ONNX/resolve/main/onnx_fp32/cond.emb.onnx",
                    },
                    ModelSource {
                        kind: "modelscope",
                        region: "cn",
                        url: "https://modelscope.cn/models/ailm32442/SoulX-Singer-SVC-ONNX/resolve/master/onnx_fp32/cond.emb.onnx",
                    },
                ],
            },
            ModelFile {
                filename: "dit.step.onnx",
                approx_bytes: 1_770_819_432,
                sha256: "8465a04327719c2965645aa3291a04aebffdf94f16fd61572feb6ebb6dbece2f",
                sources: &[
                    ModelSource {
                        kind: "hf",
                        region: "intl",
                        url: "https://huggingface.co/ailm32442/SoulX-Singer-SVC-ONNX/resolve/main/onnx_fp32/dit.step.onnx",
                    },
                    ModelSource {
                        kind: "modelscope",
                        region: "cn",
                        url: "https://modelscope.cn/models/ailm32442/SoulX-Singer-SVC-ONNX/resolve/master/onnx_fp32/dit.step.onnx",
                    },
                ],
            },
            ModelFile {
                filename: "vocoder.backbone.onnx",
                approx_bytes: 1_020_270_742,
                sha256: "b120e64dcefdc1c0e7457a1f1f96860ee39c6d9e8dcc136514f41fe5fb6e479f",
                sources: &[
                    ModelSource {
                        kind: "hf",
                        region: "intl",
                        url: "https://huggingface.co/ailm32442/SoulX-Singer-SVC-ONNX/resolve/main/onnx_fp32/vocoder.backbone.onnx",
                    },
                    ModelSource {
                        kind: "modelscope",
                        region: "cn",
                        url: "https://modelscope.cn/models/ailm32442/SoulX-Singer-SVC-ONNX/resolve/master/onnx_fp32/vocoder.backbone.onnx",
                    },
                ],
            },
            ModelFile {
                filename: "vocoder.istft.onnx",
                approx_bytes: 14_772_514,
                sha256: "4647fef6f047d14bc198c5feb3a4a755384d6a5accb07f333add45e87a88a517",
                sources: &[
                    ModelSource {
                        kind: "hf",
                        region: "intl",
                        url: "https://huggingface.co/ailm32442/SoulX-Singer-SVC-ONNX/resolve/main/onnx_fp32/vocoder.istft.onnx",
                    },
                    ModelSource {
                        kind: "modelscope",
                        region: "cn",
                        url: "https://modelscope.cn/models/ailm32442/SoulX-Singer-SVC-ONNX/resolve/master/onnx_fp32/vocoder.istft.onnx",
                    },
                ],
            },
        ],
    },
    // ---- YuE2（audio.cpp / GGUF）：引擎在「运行时 → 音频引擎」里另下，这里只放权重 ----
    //
    // 权重钉在 `audio-cpp/Yue2-3B-GGUF`（audio.cpp 官方整理的那一份），**国内外双源**：
    // 国际走 HF、国内走魔搭；两边逐文件 sha256 与体积一致（2026-10-31 核对）。
    //
    // 三档量化共用一份 VAE（f16）与同一批 sidecar，放在同一个目录 `yue2/` 里；换档时已经
    // 在位的那几个文件会自动跳过（下载器逐文件比对）。**默认 Q8_0**（audio.cpp 的默认档，
    // 也是文档里验证过的那一档）；Q4_0 省 1.6 GB，BF16 最接近原始权重但 7.3 GB。
    //
    // 注意：这四份 sidecar 必须和 gguf 在**同一个目录**（`yue2/sidecars/...`）——audio.cpp
    // 按相对路径找它们，绝对路径会被拒绝。
    ModelEntry {
        id: "yue2-q8-0",
        name: "YuE2 3B · Q8_0（默认）",
        feature: "compose",
        license: "CC-BY-NC-4.0",
        dir: Some("yue2"),
        bundled: false,
        files: &[
            yue2_file!("yue2-3b-q8_0.gguf", 4_264_195_616, "52be345b5155bf9ef0b2411e7df8d1d2b8e13b6d977a2f70c35612ec69b1f591"),
            yue2_file!("yue2-vae-f16.gguf", 265_218_656, "d4f4a05d8f291ae820cd1e43609da3fa91b56465810091a2b08c3350b751719d"),
            yue2_file!("sidecars/yue2-model-config.json", 959, "ad3477bbef890bf98ae196c1e4b44779494a6231c4ab66f32708eabadf265329"),
            yue2_file!("sidecars/yue2-generation-config.json", 466, "203830cebde6e3644eb291925362990d198e66c3b6006cd11f1aaa21904bcc61"),
            yue2_file!("sidecars/yue2-vae-config.json", 1_378, "f0191bb9694009956de44e0c361a6f1334760be4c8f848e599bde242a54a0970"),
            yue2_file!("sidecars/yue2-qwen.tiktoken", 2_561_218, "b2b1b8dfb5cc5f024bafc373121c6aba3f66f9a5a0269e243470a1de16a33186"),
        ],
    },
    ModelEntry {
        id: "yue2-q4-0",
        name: "YuE2 3B · Q4_0",
        feature: "compose",
        license: "CC-BY-NC-4.0",
        dir: Some("yue2"),
        bundled: false,
        files: &[
            yue2_file!("yue2-3b-q4_0.gguf", 2_665_641_504, "b80215b59a7fe41a4c76b6163c0ae1031672f892b5951d66fefaa2053a815029"),
            yue2_file!("yue2-vae-f16.gguf", 265_218_656, "d4f4a05d8f291ae820cd1e43609da3fa91b56465810091a2b08c3350b751719d"),
            yue2_file!("sidecars/yue2-model-config.json", 959, "ad3477bbef890bf98ae196c1e4b44779494a6231c4ab66f32708eabadf265329"),
            yue2_file!("sidecars/yue2-generation-config.json", 466, "203830cebde6e3644eb291925362990d198e66c3b6006cd11f1aaa21904bcc61"),
            yue2_file!("sidecars/yue2-vae-config.json", 1_378, "f0191bb9694009956de44e0c361a6f1334760be4c8f848e599bde242a54a0970"),
            yue2_file!("sidecars/yue2-qwen.tiktoken", 2_561_218, "b2b1b8dfb5cc5f024bafc373121c6aba3f66f9a5a0269e243470a1de16a33186"),
        ],
    },
    ModelEntry {
        id: "yue2-bf16",
        name: "YuE2 3B · BF16",
        feature: "compose",
        license: "CC-BY-NC-4.0",
        dir: Some("yue2"),
        bundled: false,
        files: &[
            yue2_file!("yue2-3b-bf16.gguf", 7_261_484_576, "4cf3fdffbaaed2d8e30a3122ac7cd27d0c96f8921805f173da9758c685b3c7d6"),
            yue2_file!("yue2-vae-f16.gguf", 265_218_656, "d4f4a05d8f291ae820cd1e43609da3fa91b56465810091a2b08c3350b751719d"),
            yue2_file!("sidecars/yue2-model-config.json", 959, "ad3477bbef890bf98ae196c1e4b44779494a6231c4ab66f32708eabadf265329"),
            yue2_file!("sidecars/yue2-generation-config.json", 466, "203830cebde6e3644eb291925362990d198e66c3b6006cd11f1aaa21904bcc61"),
            yue2_file!("sidecars/yue2-vae-config.json", 1_378, "f0191bb9694009956de44e0c361a6f1334760be4c8f848e599bde242a54a0970"),
            yue2_file!("sidecars/yue2-qwen.tiktoken", 2_561_218, "b2b1b8dfb5cc5f024bafc373121c6aba3f66f9a5a0269e243470a1de16a33186"),
        ],
    },
    // ---- ACE-Step 1.5（audio.cpp `ace_step` 家族）：另一套写歌权重 ----
    //
    // 与 YuE2 并列在「AI 写歌」的模型下拉里，但走路方式不同：YuE2 先写谱再唱（有 ABC 谱、
    // 语义码续写、翻唱），ACE-Step 是「一句描述 + 歌词 → 整首」，不写谱。两家**共用同一个
    // audio.cpp 引擎进程**，差别只在引擎配置里的 `family` 与请求体的形状
    // （见 `src-tauri/src/yue_engine.rs`），所以这里只是多两档权重，不需要第二套引擎。
    //
    // 权重钉在 `audio-cpp/audio.cpp-gguf` 的 `ACE-Step1.5-GGUF/turbo/`：HF 与魔搭
    // **同名同字节**（逐文件 sha256 一致，2026-11 核对），下载器按网络位置排序、失败换源。
    // 只收 Turbo 档：Base 慢一倍、曲风差别不大；Turbo 的扩散步数上限是 8（见文档）。
    ModelEntry {
        id: "ace-step-turbo-q8",
        name: "ACE-Step 1.5 Turbo · Q8_0（默认 · 省空间）",
        feature: "compose",
        // ACE-Step 1.5 上游（ACE-Step/Ace-Step1.5）是 **MIT**——和 YuE2 的 CC-BY-NC 不同，
        // 商业项目可以用。
        license: "MIT",
        dir: Some("ace-step"),
        bundled: false,
        files: &[ace_step_file!(
            "ace-step-1.5-turbo-q8_0.gguf",
            "ACE-Step1.5-GGUF/turbo/ace-step-1.5-turbo-q8_0.gguf",
            6_185_468_096,
            "1f0015f81c10a1182531e6207c7aeaa060b8cad3470a2f88daa3cb74ed636e63"
        )],
    },
    ModelEntry {
        id: "ace-step-turbo-bf16",
        name: "ACE-Step 1.5 Turbo · BF16（最接近原版权重）",
        feature: "compose",
        license: "MIT",
        dir: Some("ace-step"),
        bundled: false,
        // 全 BF16（10 GB）：规划器也是 bf16，固定种子能复现；Q8_0 那份规划器被量化后
        // 同种子会走到另一条采样路径（audio.cpp 的 gguf 文档把 ace_step q8_0 评为「No」）。
        files: &[ace_step_file!(
            "ace-step-1.5-turbo-bf16.gguf",
            "ACE-Step1.5-GGUF/turbo/ace-step-1.5-turbo-bf16.gguf",
            10_090_406_336,
            "7aa8cc911bb7e47647bba619cf25afc22a42e35c568a102cd9a3b28a54b94013"
        )],
    },
    // ---- SheetSage2（听音记谱）：YuE2「翻唱」的前置——录音 → ABC 谱 ----
    //
    // 只有 original-dtype 一档：官方文档明说 **Q8 不支持**。一个 2.5 GB 的 GGUF 自包含
    // （编码器 + 解码头在同一个文件里，不需要像 yu2.cpp 那样另配 MERT 底座）。
    //
    // 授权是 CC-BY-NC-4.0（和 YuE2 权重的「个人创作者可用」不是一回事）：商业分发要另谈。
    ModelEntry {
        id: "sheetsage2-orig",
        name: "SheetSage2（录音转谱）",
        feature: "cover",
        license: "CC-BY-NC-4.0",
        dir: Some("sheetsage2"),
        bundled: false,
        files: &[sheetsage2_file!(
            "sheetsage2-orig.gguf",
            2_708_224_256,
            "d858bfdf5ac7be16261ac6ee38754d6fca502149131b44a5262b4039fb72876e"
        )],
    },
    // ---- 抓歌词用的 ASR（翻唱 → 获取歌词）----
    //
    // 三个档位共用 `feature = "lyrics"`：界面一个下拉选档，缺哪个下哪个。
    // 都是 audio.cpp 的现成 ASR：跑法与本机写歌/转谱同一套（一次性引擎进程）。
    // 只收听得懂「唱」的模型：SenseVoice-Small 是纯语音 ASR，抓歌词会听成念白，已移除。
    ModelEntry {
        id: "qwen3-asr-1.7b",
        name: "Qwen3-ASR 1.7B（抓歌词 · 多语最强）",
        feature: "lyrics",
        license: "Apache-2.0",
        dir: Some("qwen3-asr-1.7b"),
        bundled: false,
        files: &[ModelFile {
            filename: "qwen3-asr-1.7b-q8_0.gguf",
            approx_bytes: 2_473_017_600,
            sha256: "3bc2462eca3a3a8f56abcd860d0943ffdc0546547dde55a9d08589607d6f3936",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/audio-cpp/audio.cpp-gguf/resolve/main/Qwen3-ASR-1.7B-GGUF/qwen3-asr-1.7b-q8_0.gguf",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/audio-cpp/audio.cpp-gguf/resolve/master/Qwen3-ASR-1.7B-GGUF/qwen3-asr-1.7b-q8_0.gguf",
                },
            ],
        }],
    },
    ModelEntry {
        id: "qwen3-asr-0.6b",
        name: "Qwen3-ASR 0.6B（抓歌词 · 轻量）",
        feature: "lyrics",
        license: "Apache-2.0",
        dir: Some("qwen3-asr-0.6b"),
        bundled: false,
        files: &[ModelFile {
            filename: "qwen3-asr-0.6b-q8_0.gguf",
            approx_bytes: 1_151_279_968,
            sha256: "5c53bbb9a6b2ade85c16c290c560658ca18b78a23c323378528f1f945683fcbe",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/audio-cpp/audio.cpp-gguf/resolve/main/Qwen3-ASR-0.6B-GGUF/qwen3-asr-0.6b-q8_0.gguf",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/audio-cpp/audio.cpp-gguf/resolve/master/Qwen3-ASR-0.6B-GGUF/qwen3-asr-0.6b-q8_0.gguf",
                },
            ],
        }],
    },
    ModelEntry {
        id: "crisperwhisper-2-large",
        name: "CrisperWhisper 2.0 Large（抓歌词 · 逐字，英语向）",
        feature: "lyrics",
        license: "MIT",
        dir: Some("crisperwhisper-2-large"),
        bundled: false,
        files: &[ModelFile {
            filename: "crisperwhisper-2-large-q8_0.gguf",
            approx_bytes: 1_716_306_528,
            sha256: "392e636876d6d5ae8716f0f98b78480b61bb8c199d63f20523e95308b3425c2e",
            sources: &[
                ModelSource {
                    kind: "hf",
                    region: "intl",
                    url: "https://huggingface.co/audio-cpp/audio.cpp-gguf/resolve/main/CrisperWhisper2.0-GGUF/crisperwhisper-2-large-q8_0.gguf",
                },
                ModelSource {
                    kind: "modelscope",
                    region: "cn",
                    url: "https://modelscope.cn/models/audio-cpp/audio.cpp-gguf/resolve/master/CrisperWhisper2.0-GGUF/crisperwhisper-2-large-q8_0.gguf",
                },
            ],
        }],
    },
];

/// 全部条目。
pub fn all() -> &'static [ModelEntry] {
    CATALOG
}

/// 按 id 找条目。
pub fn by_id(id: &str) -> Option<&'static ModelEntry> {
    CATALOG.iter().find(|m| m.id == id)
}

/// 写歌权重属于哪个 audio.cpp **家族**（`yue2` / `ace_step`）；不是写歌模型返回 `None`。
///
/// 家族决定三件事：引擎配置里的 `family` 字段、`/v1/tasks/run` 请求体的形状
/// （YuE2 走「谱 → 语义码 → 音频」，ACE-Step 走 `text2music`）、以及界面上能开哪几个
/// 开关（YuE2 有谱面 / 续写 / 翻唱，ACE-Step 只有「描述 + 歌词」）。
///
/// 按 **id 前缀**判定：同一家族再加一档量化不用改这里，加新家族才需要动。
pub fn compose_family(entry: &ModelEntry) -> Option<&'static str> {
    if entry.feature != "compose" {
        return None;
    }
    const FAMILIES: &[(&str, &str)] = &[("yue2-", "yue2"), ("ace-step-", "ace_step")];
    FAMILIES
        .iter()
        .find(|(prefix, _)| entry.id.starts_with(prefix))
        .map(|(_, family)| *family)
}

/// 找到**同时**包含 `files` 全部文件的子目录。
///
/// 多组件模型必须整组待在同一个目录里：`ort` 是靠 `.onnx` 里记录的**相对路径**去找外部
/// 权重的，所以 `.onnx` 与 `.onnx.data`（或分片 `.onnx.data.0/.1/...`）必须相邻。
pub fn group_dir_for(dir: &str, files: &[&str]) -> Option<PathBuf> {
    super::search_dirs()
        .into_iter()
        .map(|d| d.join(dir))
        .find(|d| files.iter().all(|f| d.join(f).is_file()))
}

/// 组条目的落点目录（非组条目返回 `None`）。
pub fn group_dir(entry: &ModelEntry) -> Option<PathBuf> {
    let dir = entry.dir?;
    let names: Vec<&str> = entry.files.iter().map(|f| f.filename).collect();
    group_dir_for(dir, &names)
}

/// 一个文件当前落在哪里：组内文件走组目录，单文件在 `search_dirs` 里逐个找。
pub fn locate(entry: &ModelEntry, file: &ModelFile) -> Option<PathBuf> {
    if entry.dir.is_some() {
        return group_dir(entry).map(|d| d.join(file.filename));
    }
    super::search_dirs()
        .into_iter()
        .map(|d| d.join(file.filename))
        .find(|p| p.is_file())
}

/// 这个文件在磁盘上是否在位（组条目要求整组同目录）。
pub fn file_present(entry: &ModelEntry, file: &ModelFile) -> bool {
    locate(entry, file).is_some()
}

/// 整个条目（单文件或整组）是否就位。
pub fn installed(entry: &ModelEntry) -> bool {
    entry.files.iter().all(|f| file_present(entry, f))
}

/// 已就位时的落点（「在访达中显示」用）；没就位返回 `None`。
pub fn installed_path(entry: &ModelEntry) -> Option<PathBuf> {
    if !installed(entry) {
        return None;
    }
    match entry.dir {
        Some(_) => group_dir(entry),
        None => locate(entry, entry.files.first()?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_has_a_unique_id_and_filename() {
        let mut ids = std::collections::HashSet::new();
        for entry in all() {
            assert!(ids.insert(entry.id), "重复 id：{}", entry.id);
            assert!(!entry.files.is_empty(), "{} 一个文件都没有", entry.id);
            let mut names = std::collections::HashSet::new();
            for file in entry.files {
                assert!(
                    names.insert(file.filename),
                    "{} 内文件重名：{}",
                    entry.id,
                    file.filename
                );
                assert!(
                    !file.sha256.is_empty() || file.sources.is_empty(),
                    "{} / {} 缺 sha256",
                    entry.id,
                    file.filename
                );
                for source in file.sources {
                    assert!(
                        source.url.starts_with("http"),
                        "源地址应以 http 开头：{}",
                        source.url
                    );
                }
            }
        }
    }

    /// SoulX-Singer 的**两档都是国内外双源**：同一份字节，国内走魔搭、国际走 HF，
    /// 少一边就退化成单源（海外直连魔搭会很难受）。
    #[test]
    fn soulx_singer_has_both_cn_and_intl_sources() {
        for (id, host_dir) in [
            ("soulx-singer-fp16", "SoulX-Singer-onnx-fp16"),
            ("soulx-singer-fp32", "SoulX-Singer-onnx-fp32"),
        ] {
            let entry = by_id(id).unwrap_or_else(|| panic!("应存在 {id} 条目"));
            assert_eq!(entry.files.len(), 16, "{id}：9 张图 + 7 个外部权重");
            for file in entry.files {
                assert!(
                    file.sources
                        .iter()
                        .any(|s| s.kind == "modelscope" && s.region == "cn"),
                    "{} / {} 缺国内源",
                    entry.id,
                    file.filename
                );
                assert!(
                    file.sources.iter().any(|s| {
                        s.kind == "hf"
                            && s.region == "intl"
                            && s.url.contains(&format!("huggingface.co/ailm32442/{host_dir}/"))
                            && s.url.ends_with(&format!("/{}", file.filename))
                    }),
                    "{} / {} 缺国际源（或指错了仓库）",
                    entry.id,
                    file.filename
                );
            }
        }
    }

    #[test]
    fn htdemucs_is_downloadable_and_not_bundled() {
        let entry = by_id("htdemucs-ort-v1").expect("应存在 htdemucs 条目");
        assert!(!entry.bundled, "大模型不随包");
        assert_eq!(entry.feature, "separate");
        assert!(!entry.files[0].sources.is_empty(), "应有可下载源");
    }

    /// 非商用（CC BY-NC）权重**绝不能随包**：这是发行策略，不是风格偏好——
    /// 一旦有人把 YuE2 / SheetSage2 加进 `bundle.resources`，安装包就不能再当作
    /// Apache-2.0 发行物分发。它们只能走「用户点了才下载」的路。
    #[test]
    fn noncommercial_weights_are_download_only() {
        let mut seen = 0;
        for entry in all() {
            if !entry.license.starts_with("CC-BY-NC") {
                continue;
            }
            seen += 1;
            assert!(
                !entry.bundled,
                "{} 是非商用权重，不能随安装包分发",
                entry.id
            );
            assert!(
                entry.files.iter().all(|file| !file.sources.is_empty()),
                "{} 必须留着可下载源（用户自选下载）",
                entry.id
            );
        }
        assert!(seen > 0, "清单里应仍有非商用档（YuE2 / SheetSage2）");
    }

    /// 清单里**只允许官方 / 大厂域名**：魔搭 / Hugging Face / GitHub，加上国内几家大厂镜像站。
    ///
    /// 开源项目不替用户选第三方镜像与代理站（个人反代、hf-mirror 这类多一层中间人，
    /// 一旦被接管就是所有人的权重来源被换）；要镜像请用户自己在设置里填。这条是**防回归**用的
    /// ——有人再往清单里塞第三方地址，它会直接失败。
    ///
    /// 白名单与 `src-tauri/src/manager.rs::is_trusted_download_host`（安装插件时的同一套闸）
    /// 必须保持一致：这边是**清单里不许出现**，那边是**运行时必须用户确认**。
    #[test]
    fn every_source_is_an_official_host() {
        const TRUSTED_ROOTS: &[&str] = &[
            // 代码托管 / 发布
            "github.com",
            "githubusercontent.com",
            "gitee.com",
            // 模型仓库
            "modelscope.cn",
            "huggingface.co",
            "hf.co",
            // 大厂镜像站
            "mirrors.tuna.tsinghua.edu.cn",
            "mirrors.aliyun.com",
            "mirrors.huaweicloud.com",
            "mirrors.ustc.edu.cn",
            "mirrors.cloud.tencent.com",
            // 运行时厂商自己
            "api.nuget.org",
            "nvidia.com",
            "nvidia.cn",
            "microsoft.com",
        ];
        let host_of = |url: &str| -> String {
            url.split_once("://")
                .map(|(_, rest)| rest)
                .unwrap_or(url)
                .split(['/', '?', '#'])
                .next()
                .unwrap_or("")
                .rsplit('@')
                .next()
                .unwrap_or("")
                .split(':')
                .next()
                .unwrap_or("")
                .trim_start_matches("www.")
                .trim_end_matches('.')
                .to_ascii_lowercase()
        };
        for entry in all() {
            for file in entry.files {
                for source in file.sources {
                    let host = host_of(source.url);
                    assert!(
                        TRUSTED_ROOTS
                            .iter()
                            .any(|root| host == *root || host.ends_with(&format!(".{root}"))),
                        "{} / {} 用了非官方源：{}（域名 {host}）",
                        entry.id,
                        file.filename,
                        source.url
                    );
                }
            }
        }
    }

    /// fp16 / fp32 是同一份导出的两档：编码器的外部权重（`.onnx.data`）字节完全相同，
    /// 差别只在 `diff_step` 与 `vocoder`。这条防止有人「顺手」改了某一份的权重却只改一边。
    #[test]
    fn soulx_precisions_share_encoder_weights() {
        let fp16 = by_id("soulx-singer-fp16").expect("缺 fp16 条目");
        let fp32 = by_id("soulx-singer-fp32").expect("缺 fp32 条目");
        assert_eq!(fp16.files.len(), fp32.files.len());
        for (a, b) in fp16.files.iter().zip(fp32.files.iter()) {
            assert_eq!(a.filename, b.filename, "两档的文件顺序必须一致");
            if a.filename.ends_with(".onnx.data") {
                assert_eq!(
                    a.sha256, b.sha256,
                    "{} 是编码器权重，两档应字节相同",
                    a.filename
                );
                assert_eq!(a.approx_bytes, b.approx_bytes);
            }
        }
        // 两个大图必须真的不同（否则「fp32 档」就是假的）。
        let diff16 = fp16
            .files
            .iter()
            .find(|f| f.filename == "diff_step_dml.onnx")
            .unwrap();
        let diff32 = fp32
            .files
            .iter()
            .find(|f| f.filename == "diff_step_dml.onnx")
            .unwrap();
        assert_ne!(diff16.sha256, diff32.sha256);
        assert!(diff32.approx_bytes > diff16.approx_bytes * 19 / 10);
    }

    /// 音色替换（SVC）同样是**国内外双源**；两档各 7 张自足图，且 hq / fp32 的差别只该在
    /// 声码器与条件嵌入上（扩散步两档字节相同）。
    #[test]
    fn soulx_svc_tiers_are_dual_sourced_and_share_the_diffusion_step() {
        for id in ["soulx-svc-fp16hq", "soulx-svc-fp32"] {
            let entry = by_id(id).unwrap_or_else(|| panic!("缺 {id} 条目"));
            assert_eq!(entry.feature, "voice_swap");
            assert_eq!(entry.files.len(), 7, "{id} 应是 7 张图");
            for file in entry.files {
                assert!(
                    file.sources
                        .iter()
                        .any(|s| s.kind == "modelscope" && s.region == "cn"),
                    "{} / {} 缺国内源",
                    entry.id,
                    file.filename
                );
                assert!(
                    file.sources
                        .iter()
                        .any(|s| s.kind == "hf" && s.region == "intl"),
                    "{} / {} 缺国际源",
                    entry.id,
                    file.filename
                );
            }
        }
        let hq = by_id("soulx-svc-fp16hq").unwrap();
        let fp32 = by_id("soulx-svc-fp32").unwrap();
        for name in [
            "whisper.frontend.onnx",
            "mel.transform.onnx",
            "f0.encoder.onnx",
            "dit.step.onnx",
            "vocoder.istft.onnx",
        ] {
            let a = hq.files.iter().find(|f| f.filename == name).unwrap();
            let b = fp32.files.iter().find(|f| f.filename == name).unwrap();
            assert_eq!(a.sha256, b.sha256, "{name} 两档应字节相同");
        }
        for name in ["vocoder.backbone.onnx", "cond.emb.onnx"] {
            let a = hq.files.iter().find(|f| f.filename == name).unwrap();
            let b = fp32.files.iter().find(|f| f.filename == name).unwrap();
            assert_ne!(a.sha256, b.sha256, "{name} 是两档的差别所在");
        }
    }

    /// 多文件成组模型的约定：落在自己的子目录、体积按整组算、且 `.onnx` 与它的外部权重
    /// 必须同目录（`ort` 靠图里记录的**相对路径**找权重）。外部权重可能是单文件
    /// `<name>.data`，也可能是**分片** `<name>.data.0/.1/...`。
    ///
    /// 这条约定现在由 SoulX-Singer（歌声合成）实际使用；将来再加多文件模型时照这个形状写即可。
    #[test]
    fn every_group_entry_keeps_its_external_weights_beside_the_graph() {
        for entry in all().iter().filter(|e| e.dir.is_some()) {
            assert!(!entry.bundled, "{} 是 GB 级组模型，不该随包", entry.id);
            for file in entry.files {
                if !file.filename.ends_with(".onnx") {
                    continue;
                }
                // 自足图（权重内嵌在 .onnx 里）没有外部权重，不算缺件。
                const SELF_CONTAINED: &[&str] = &[
                    "diff_step_dml.onnx",
                    "vocoder_dml.onnx",
                    // SoulX-Singer-SVC 的 7 张图都是自足导出。
                    "whisper.frontend.onnx",
                    "mel.transform.onnx",
                    "f0.encoder.onnx",
                    "cond.emb.onnx",
                    "dit.step.onnx",
                    "vocoder.backbone.onnx",
                    "vocoder.istft.onnx",
                ];
                if SELF_CONTAINED.contains(&file.filename) {
                    continue;
                }
                let single = format!("{}.data", file.filename);
                let shard = format!("{}.data.", file.filename);
                let found = entry
                    .files
                    .iter()
                    .any(|f| f.filename == single || f.filename.starts_with(&shard));
                assert!(
                    found,
                    "{} 缺配套的外部权重（{} 或分片）",
                    file.filename, single
                );
            }
        }
    }

    /// 写歌模型**都是国内外双源**：这些权重是几 GB 级的一次性下载，只有单边网络就等于用不了。
    /// 同时钉住两个家族的家族解析：界面与引擎都靠它决定请求体形状。
    #[test]
    fn compose_weights_are_dual_sourced_and_declare_their_family() {
        let mut seen: Vec<(&str, &str)> = Vec::new();
        for entry in all().iter().filter(|e| e.feature == "compose") {
            let family = compose_family(entry)
                .unwrap_or_else(|| panic!("{} 没写明 audio.cpp 家族", entry.id));
            seen.push((entry.id, family));
            for file in entry.files {
                assert!(
                    file.sources
                        .iter()
                        .any(|s| s.kind == "hf" && s.region == "intl"),
                    "{} / {} 缺国际源",
                    entry.id,
                    file.filename
                );
                assert!(
                    file.sources
                        .iter()
                        .any(|s| s.kind == "modelscope" && s.region == "cn"),
                    "{} / {} 缺国内源",
                    entry.id,
                    file.filename
                );
            }
        }
        // YuE2 三档 + ACE-Step 两档：少了哪一档都说明清单被人动过。
        assert!(seen.contains(&("yue2-q8-0", "yue2")));
        assert!(seen.contains(&("ace-step-turbo-q8", "ace_step")));
        assert!(seen.contains(&("ace-step-turbo-bf16", "ace_step")));
        assert_eq!(seen.iter().filter(|(_, f)| *f == "ace_step").count(), 2);
        // 非写歌模型不该有家族（否则引擎会去加载一份不是它管的权重）。
        assert!(compose_family(by_id("htdemucs-ort-v1").unwrap()).is_none());
    }

    /// 抓歌词（`feature = "lyrics"`）只收**听得懂唱**的 ASR：纯语音模型（如 SenseVoice-Small）
    /// 会把歌词听成念白。这条防的是「顺手把一档加回来」——界面下拉与下载管理器都吃这份清单。
    #[test]
    fn lyrics_feature_keeps_only_singing_capable_asr() {
        let ids: Vec<&str> = all()
            .iter()
            .filter(|entry| entry.feature == "lyrics")
            .map(|entry| entry.id)
            .collect();
        assert_eq!(
            ids,
            ["qwen3-asr-1.7b", "qwen3-asr-0.6b", "crisperwhisper-2-large"]
        );
        assert!(
            by_id("sensevoice-small").is_none(),
            "SenseVoice-Small 已下线（纯语音 ASR，抓歌词不成句）"
        );
    }
}
