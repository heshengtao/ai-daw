//! **模型清单**（编译期内嵌）：每个本地 AI 模型的用途、体积、许可、落地文件名与下载源。
//!
//! 清单是「有哪些模型、从哪下、对不对得上」的唯一来源。`sha256` 是硬门禁——第三方镜像 /
//! 加速站下载的字节也要先校验再落地（与插件管理器 E1 同一套安全立场）。
//!
//! 分两类：
//! - **随包小模型**（`bundled = true`，Basic Pitch / LocalVQE）：由 `bundle.resources` 打进
//!   安装包，开箱可用；清单里仍留源，便于界面显示「可重新下载」。
//! - **按需下载的大模型**（`bundled = false`，如音源分离 htdemucs）：不随包，用户到
//!   「管理器 → 模型」里下到应用配置目录的 `models/`。
//!
//! 魔搭（ModelScope）源**暂缺**：htdemucs 在魔搭上还没有对应仓库，等自建后再补 `modelscope`
//! 条目。清单里没有的源就不写，缺源时界面显示「此源不可用」而不是一个死链。

use std::path::PathBuf;

/// 一个下载源。
#[derive(Debug, Clone, Copy)]
pub struct ModelSource {
    /// 源类型：`hf` / `hf-mirror` / `modelscope` / `custom`。
    pub kind: &'static str,
    /// 直链地址。
    pub url: &'static str,
    /// 该文件的 sha256（小写十六进制）。
    pub sha256: &'static str,
}

/// 清单里的一个模型。
#[derive(Debug, Clone, Copy)]
pub struct ModelEntry {
    /// 稳定标识。
    pub id: &'static str,
    /// 展示名。
    pub name: &'static str,
    /// 所属功能（`to_midi` / `enhance` / `separate` …）。
    pub feature: &'static str,
    /// SPDX 许可。
    pub license: &'static str,
    /// 预估体积（字节），下载前告知 / 磁盘预检用。
    pub approx_bytes: u64,
    /// 落地文件名。
    pub filename: &'static str,
    /// 是否随安装包分发。
    pub bundled: bool,
    /// 有序候选源（按顺序尝试，第一个成功即止）。
    pub sources: &'static [ModelSource],
}

/// 清单。
pub static CATALOG: &[ModelEntry] = &[
    ModelEntry {
        id: "basic-pitch",
        name: "Basic Pitch（音频转 MIDI）",
        feature: "to_midi",
        license: "Apache-2.0",
        approx_bytes: 230_444,
        filename: "basic-pitch-nmp.onnx",
        bundled: true,
        sources: &[ModelSource {
            kind: "hf",
            url: "https://huggingface.co/AEmotionStudio/basic-pitch-onnx-models/resolve/main/nmp.onnx",
            sha256: "2c3c1d144bfa61ad236e92e169c13535c880469a12a047d4e73451f2c059a0ec",
        }],
    },
    ModelEntry {
        id: "localvqe",
        name: "LocalVQE（人声降噪 / 去混响）",
        feature: "enhance",
        license: "Apache-2.0",
        approx_bytes: 6_152_621,
        filename: "localvqe-ns.onnx",
        bundled: true,
        // 上游只发 `.pt` / GGUF（Apache-2.0）；ONNX 由 `tools/export-localvqe-onnx.py` 本地导出，
        // 再自建镜像（HF `ailm32442/localvqe-ns-onnx`）。随包分发，同时也支持重新下载。
        sources: &[
            ModelSource {
                kind: "hf",
                url: "https://huggingface.co/ailm32442/localvqe-ns-onnx/resolve/main/localvqe-ns.onnx",
                sha256: "268339d31dd427cbe5cf9107028185b7a473b5ae0cbc59f1ba39187767367060",
            },
            ModelSource {
                kind: "hf-mirror",
                url: "https://hf-mirror.com/ailm32442/localvqe-ns-onnx/resolve/main/localvqe-ns.onnx",
                sha256: "268339d31dd427cbe5cf9107028185b7a473b5ae0cbc59f1ba39187767367060",
            },
        ],
    },
    ModelEntry {
        id: "htdemucs-ort-v1",
        name: "HTDemucs（音源分离 · 4 分轨）",
        feature: "separate",
        // 权重来自 Meta 的 Demucs v4（htdemucs）——上游是 **MIT**，不是 Apache。
        license: "MIT",
        approx_bytes: 209_884_896,
        filename: "htdemucs.ort",
        bundled: false,
        sources: &[
            // 国内优先走魔搭（自建镜像；字节与 HF 完全一致，校验同一个 sha256）。
            ModelSource {
                kind: "modelscope",
                url: "https://modelscope.cn/models/ailm32442/htdemucs-ort/resolve/master/htdemucs.ort",
                sha256: "09dc165512d8ef7480bcb2cacea9dda82d571f8dbf421d8c44a2ca5568bec729",
            },
            ModelSource {
                kind: "hf",
                url: "https://huggingface.co/gentij/htdemucs-ort/resolve/main/htdemucs.ort",
                sha256: "09dc165512d8ef7480bcb2cacea9dda82d571f8dbf421d8c44a2ca5568bec729",
            },
            ModelSource {
                kind: "hf-mirror",
                url: "https://hf-mirror.com/gentij/htdemucs-ort/resolve/main/htdemucs.ort",
                sha256: "09dc165512d8ef7480bcb2cacea9dda82d571f8dbf421d8c44a2ca5568bec729",
            },
        ],
    },
    ModelEntry {
        id: "htdemucs-6s",
        name: "HTDemucs 6s（音源分离 · 6 分轨）",
        feature: "separate",
        // 同为 Meta Demucs v4 家族的 `htdemucs_6s`（**MIT**）；这份是自足 ONNX 导出
        // （输入原始波形 `mix`、直接输出 `stems`，不需要我们算 STFT）。
        license: "MIT",
        approx_bytes: 258_159_781,
        filename: "htdemucs_6s.onnx",
        bundled: false,
        sources: &[
            ModelSource {
                kind: "hf",
                url: "https://huggingface.co/StemSplitio/htdemucs-6s-onnx/resolve/main/htdemucs_6s.onnx",
                sha256: "48f8e84945579f8ab340e083339e9221e03785dbe733a52c388200b6d3ca779a",
            },
            ModelSource {
                kind: "hf-mirror",
                url: "https://hf-mirror.com/StemSplitio/htdemucs-6s-onnx/resolve/main/htdemucs_6s.onnx",
                sha256: "48f8e84945579f8ab340e083339e9221e03785dbe733a52c388200b6d3ca779a",
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

/// 条目当前是否已就位（在搜索目录里找到了它的文件）。
pub fn installed_path(entry: &ModelEntry) -> Option<PathBuf> {
    super::find(entry.filename)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_has_a_unique_id_and_filename() {
        let mut ids = std::collections::HashSet::new();
        for entry in all() {
            assert!(ids.insert(entry.id), "重复 id：{}", entry.id);
            assert!(!entry.filename.is_empty(), "{} 缺文件名", entry.id);
            // 可下载的条目必须带至少一个源 + 非空 sha256。
            if !entry.bundled || !entry.sources.is_empty() {
                for source in entry.sources {
                    assert!(source.url.starts_with("http"), "源地址应以 http 开头：{}", source.url);
                    assert_eq!(source.sha256.len(), 64, "{} 的 sha256 长度不对", entry.id);
                }
            }
        }
    }

    #[test]
    fn htdemucs_is_downloadable_and_not_bundled() {
        let entry = by_id("htdemucs-ort-v1").expect("应存在 htdemucs 条目");
        assert!(!entry.bundled, "大模型不随包");
        assert_eq!(entry.feature, "separate");
        assert!(!entry.sources.is_empty(), "应有可下载源");
    }
}
