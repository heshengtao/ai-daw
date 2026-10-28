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

/// 清单。
pub static CATALOG: &[ModelEntry] = &[
    ModelEntry {
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
];

/// 全部条目。
pub fn all() -> &'static [ModelEntry] {
    CATALOG
}

/// 按 id 找条目。
pub fn by_id(id: &str) -> Option<&'static ModelEntry> {
    CATALOG.iter().find(|m| m.id == id)
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
                assert!(names.insert(file.filename), "{} 内文件重名：{}", entry.id, file.filename);
                assert!(!file.sha256.is_empty() || file.sources.is_empty(), "{} / {} 缺 sha256", entry.id, file.filename);
                for source in file.sources {
                    assert!(source.url.starts_with("http"), "源地址应以 http 开头：{}", source.url);
                }
            }
        }
    }

    /// SoulX-Singer 是**国内外双源**：同一份字节，国内走魔搭、国际走 HF，
    /// 少一边就退化成单源（海外直连魔搭会很难受）。
    #[test]
    fn soulx_singer_has_both_cn_and_intl_sources() {
        let entry = by_id("soulx-singer-fp16").expect("应存在 SoulX-Singer 条目");
        assert_eq!(entry.files.len(), 16, "9 张图 + 7 个外部权重");
        for file in entry.files {
            assert!(
                file.sources.iter().any(|s| s.kind == "modelscope" && s.region == "cn"),
                "{} / {} 缺国内源",
                entry.id,
                file.filename
            );
            assert!(
                file.sources.iter().any(|s| s.kind == "hf" && s.region == "intl"),
                "{} / {} 缺国际源",
                entry.id,
                file.filename
            );
        }
    }

    #[test]
    fn htdemucs_is_downloadable_and_not_bundled() {
        let entry = by_id("htdemucs-ort-v1").expect("应存在 htdemucs 条目");
        assert!(!entry.bundled, "大模型不随包");
        assert_eq!(entry.feature, "separate");
        assert!(!entry.files[0].sources.is_empty(), "应有可下载源");
    }

    /// 清单里**只允许官方源**：HF / GitHub / 魔搭。
    ///
    /// 开源项目不替用户选第三方镜像与代理站；要镜像请用户自己在设置里填。这条是**防回归**用的
    /// ——有人再往清单里塞第三方地址，它会直接失败。
    #[test]
    fn every_source_is_an_official_host() {
        const OFFICIAL: &[&str] = &[
            "https://huggingface.co/",
            "https://github.com/",
            "https://modelscope.cn/",
            "https://www.modelscope.cn/",
        ];
        for entry in all() {
            for file in entry.files {
                for source in file.sources {
                    assert!(
                        OFFICIAL.iter().any(|host| source.url.starts_with(host)),
                        "{} / {} 用了非官方源：{}",
                        entry.id,
                        file.filename,
                        source.url
                    );
                }
            }
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
                const SELF_CONTAINED: &[&str] = &["diff_step_dml.onnx", "vocoder_dml.onnx"];
                if SELF_CONTAINED.contains(&file.filename) {
                    continue;
                }
                let single = format!("{}.data", file.filename);
                let shard = format!("{}.data.", file.filename);
                let found = entry
                    .files
                    .iter()
                    .any(|f| f.filename == single || f.filename.starts_with(&shard));
                assert!(found, "{} 缺配套的外部权重（{} 或分片）", file.filename, single);
            }
        }
    }
}
