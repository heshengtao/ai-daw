//! 内置合成器的出厂音色（**前后端共享一份数据**）。
//!
//! 数据源是 `src/data/synth-factory-presets.json`：前端 `data/plugins.ts` 直接 import 它，
//! Rust/MCP 用 `include_str!` 编译进二进制。这样「界面上的 36 条音色」与「MCP 能列出的 36 条」
//! 不可能漂移；其中 5 条在引擎注册表里另有同名预设，`tests` 会把两边的参数逐值对上。
//!
//! JSON 里的 `name` / `note` 是界面用的 i18n key；`label` / `description` 是给模型看的中文快照。

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

/// 一条出厂音色。`params` 是**补丁**（只列偏离默认值的参数），不是完整参数集。
/// JSON 里的 `name` / `note` 是界面用的 i18n key，Rust 侧不需要它们（serde 会忽略）。
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FactoryPreset {
    pub id: String,
    /// 给模型看的中文名。
    pub label: String,
    /// 给模型看的一句话音色描述。
    pub description: String,
    /// `8bit` / `epiano` / `guitar` / `bass` / `drums` / `strings` / `winds` / `pad` / `fx`。
    pub category: String,
    /// 参数补丁（参数 id → 值）。
    pub params: BTreeMap<String, f32>,
}

const RAW: &str = include_str!("../../src/data/synth-factory-presets.json");

/// 全部出厂音色（JSON 顺序 = 界面分类顺序）。
pub(crate) fn all() -> &'static [FactoryPreset] {
    static PRESETS: OnceLock<Vec<FactoryPreset>> = OnceLock::new();
    PRESETS.get_or_init(|| {
        serde_json::from_str(RAW).expect(
            "内置合成器出厂预设 JSON 必须可解析：src/data/synth-factory-presets.json",
        )
    })
}

/// 去重、排序后的类别列表（给 `list_synth_presets` 的目录用）。
pub(crate) fn categories() -> Vec<&'static str> {
    let mut out: Vec<&str> = all().iter().map(|p| p.category.as_str()).collect();
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_daw_engine::registry::BUILTIN_PRESETS;

    #[test]
    fn json_has_36_unique_presets_in_nine_categories() {
        assert_eq!(all().len(), 36, "出厂音色数量");
        let mut ids: Vec<&str> = all().iter().map(|p| p.id.as_str()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "预设 id 不能重复");

        assert_eq!(categories().len(), 9, "类别：{:?}", categories());
        for preset in all() {
            assert!(!preset.label.trim().is_empty(), "{} 缺中文名", preset.id);
            assert!(!preset.description.trim().is_empty(), "{} 缺描述", preset.id);
            assert!(!preset.params.is_empty(), "{} 没有参数", preset.id);
        }
    }

    /// 引擎注册表里的 5 条老预设与共享 JSON 必须逐值一致——这是唯一能挡住两边漂移的地方。
    #[test]
    fn engine_presets_match_the_shared_json() {
        let mut checked = 0;
        for preset in BUILTIN_PRESETS.iter().filter(|p| !p.patch.is_empty()) {
            let entry = all()
                .iter()
                .find(|p| p.id == preset.id)
                .unwrap_or_else(|| panic!("共享 JSON 缺少引擎预设 {}", preset.id));
            assert_eq!(
                entry.params.len(),
                preset.patch.len(),
                "{} 的参数个数与引擎补丁不一致",
                preset.id
            );
            for (id, expected) in preset.patch {
                let got = entry
                    .params
                    .get(*id)
                    .copied()
                    .unwrap_or_else(|| panic!("{} 缺少参数 {id}", preset.id));
                assert!(
                    (got - expected).abs() < 1e-6,
                    "{} 的 {id}：JSON={got}，引擎={expected}",
                    preset.id
                );
            }
            checked += 1;
        }
        assert_eq!(checked, 5, "引擎里应有 5 条带补丁的预设");
    }
}
