//! 市场搜索：规范化 + 中日韩双字切分 + 前缀/容错匹配 + 字段权重。
//!
//! 算法照抄 DeepSeek Harness 的 `plugin-market/search.ts`（MiniSearch 配置）：
//!
//! - 权重：包名 / 标题 ×3，关键词 ×2，其余（说明 / 作者 / 分类标签）×1；
//! - 所有查询词必须命中（AND）；
//! - 词长 ≥ 4 才允许容错，编辑距离上限 `round(len/5)` 且不超过 2；
//! - 中日韩文本按「重叠双字 + 末字」切分，不引分词库。
//!
//! 差异：不做完整的 Unicode NFKC（只做全角→半角与大小写折叠），少一个依赖。

use crate::index_format::PluginEntry;

/// 排序方式。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Sort {
    /// 索引顺序（默认）。
    #[default]
    Index,
    /// 相关度。
    Relevance,
    /// 最新发布。
    Published,
    /// 市场声明的排序键。
    Key(String),
}

impl Sort {
    /// 从查询串解析（`relevance` / `published` / `key:<id>`）。
    pub fn parse(text: &str) -> Self {
        match text.trim() {
            "" | "index" => Self::Index,
            "relevance" => Self::Relevance,
            "published" => Self::Published,
            other => match other.strip_prefix("key:") {
                Some(key) => Self::Key(key.to_string()),
                None => Self::Index,
            },
        }
    }
}

/// 是否中日韩字符（含扩展 A / 兼容 / 假名 / 谚文）。
pub fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x3040..=0x30FF      // 平假名 / 片假名
        | 0x31F0..=0x31FF
        | 0x3400..=0x4DBF    // 汉字扩展 A
        | 0x4E00..=0x9FFF    // 基本汉字
        | 0xF900..=0xFAFF    // 兼容汉字
        | 0xAC00..=0xD7A3    // 谚文音节
        | 0x20000..=0x2FA1F  // 汉字扩展 B–F
    )
}

/// 全角 → 半角、大小写折叠（NFKC 的实用子集）。
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        let mapped = match ch as u32 {
            0xFF01..=0xFF5E => char::from_u32(ch as u32 - 0xFEE0).unwrap_or(ch),
            0x3000 => ' ',
            _ => ch,
        };
        for lower in mapped.to_lowercase() {
            out.push(lower);
        }
    }
    out
}

/// 切词：连续中日韩串 → 重叠双字 + 末字；其余按字母/数字串。
pub fn tokenize(text: &str) -> Vec<String> {
    let normalized = normalize(text);
    let mut terms = Vec::new();
    let mut word = String::new();
    let mut cjk: Vec<char> = Vec::new();

    let flush_word = |word: &mut String, terms: &mut Vec<String>| {
        if !word.is_empty() {
            terms.push(std::mem::take(word));
        }
    };
    let flush_cjk = |cjk: &mut Vec<char>, terms: &mut Vec<String>| {
        if cjk.is_empty() {
            return;
        }
        if cjk.len() == 1 {
            terms.push(cjk[0].to_string());
        } else {
            for pair in cjk.windows(2) {
                terms.push(pair.iter().collect());
            }
            terms.push(cjk[cjk.len() - 1].to_string());
        }
        cjk.clear();
    };

    for ch in normalized.chars() {
        if is_cjk(ch) {
            flush_word(&mut word, &mut terms);
            cjk.push(ch);
        } else if ch.is_alphanumeric() {
            flush_cjk(&mut cjk, &mut terms);
            word.push(ch);
        } else {
            flush_word(&mut word, &mut terms);
            flush_cjk(&mut cjk, &mut terms);
        }
    }
    flush_word(&mut word, &mut terms);
    flush_cjk(&mut cjk, &mut terms);
    terms
}

/// 带权重的字段。
struct Field {
    weight: f64,
    terms: Vec<String>,
}

fn fields(entry: &PluginEntry, lang: &str) -> Vec<Field> {
    let mut fields = Vec::new();
    let mut push = |weight: f64, text: &str| {
        let terms = tokenize(text);
        if !terms.is_empty() {
            fields.push(Field { weight, terms });
        }
    };
    push(3.0, &entry.name);
    if let Some(title) = &entry.title {
        push(3.0, &title.resolve(lang));
    }
    if !entry.metadata.keywords.is_empty() {
        push(2.0, &entry.metadata.keywords.join(" "));
    }
    if let Some(description) = &entry.description {
        push(1.0, &description.resolve(lang));
    }
    if let Some(author) = &entry.author {
        push(1.0, author);
    }
    // 分类与标签的标题也算文本（调用方把市场声明解析成文本传进来）。
    fields
}

fn edit_distance_within(a: &str, b: &str, limit: usize) -> Option<usize> {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > limit {
        return None;
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        current[0] = i + 1;
        let mut row_min = current[0];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            current[j + 1] = (previous[j] + cost)
                .min(previous[j + 1] + 1)
                .min(current[j] + 1);
            row_min = row_min.min(current[j + 1]);
        }
        if row_min > limit {
            return None;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let distance = previous[b.len()];
    (distance <= limit).then_some(distance)
}

/// 一个查询词对某个索引词的质量（0 分 = 不命中）。
fn match_quality(term: &str, indexed: &str) -> f64 {
    if indexed == term {
        return 1.0;
    }
    if indexed.starts_with(term) {
        return 0.85;
    }
    let length = term.chars().count();
    if length >= 4 {
        let limit = ((length as f64) / 5.0).round() as usize;
        let limit = limit.clamp(1, 2);
        if let Some(distance) = edit_distance_within(term, indexed, limit) {
            return 1.0 - 0.15 * distance as f64;
        }
    }
    0.0
}

/// 给一个插件打分：`None` = 不命中（有查询词没落在任何字段）。
///
/// `taxonomy` 是市场声明的分类/标签标题拼成的文本（调用方提供，通常按当前语言）。
pub fn score(entry: &PluginEntry, query_terms: &[String], lang: &str, taxonomy: &str) -> Option<f64> {
    if query_terms.is_empty() {
        return Some(0.0);
    }
    let mut fields = fields(entry, lang);
    if !taxonomy.trim().is_empty() {
        let terms = tokenize(taxonomy);
        if !terms.is_empty() {
            fields.push(Field { weight: 1.0, terms });
        }
    }

    let mut total = 0.0;
    for term in query_terms {
        let mut best = 0.0f64;
        for field in &fields {
            for indexed in &field.terms {
                let quality = match_quality(term, indexed);
                if quality > 0.0 {
                    best = best.max(field.weight * quality);
                }
            }
        }
        if best == 0.0 {
            return None;
        }
        total += best;
    }
    Some(total)
}

/// 排序结果（返回**原索引下标**，调用方按需分页）。
pub fn rank(
    entries: &[PluginEntry],
    text: &str,
    lang: &str,
    taxonomy_of: impl Fn(&PluginEntry) -> String,
    sort: &Sort,
) -> Vec<usize> {
    let terms = tokenize(text);
    let mut scored: Vec<(usize, f64)> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            score(entry, &terms, lang, &taxonomy_of(entry)).map(|score| (index, score))
        })
        .collect();

    match sort {
        Sort::Index => {}
        Sort::Relevance => scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0))),
        Sort::Published => scored.sort_by(|a, b| {
            let left = entries[a.0].latest_publish();
            let right = entries[b.0].latest_publish();
            right.cmp(&left).then_with(|| a.0.cmp(&b.0))
        }),
        Sort::Key(key) => scored.sort_by(|a, b| {
            let left = entries[a.0].metadata.sort_keys.get(key).copied();
            let right = entries[b.0].metadata.sort_keys.get(key).copied();
            match (left, right) {
                // 没有值的排最后（两种方向都一样），等值保持索引顺序。
                (Some(left), Some(right)) => right.total_cmp(&left).then_with(|| a.0.cmp(&b.0)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.0.cmp(&b.0),
            }
        }),
    }

    scored.into_iter().map(|(index, _)| index).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index_format::{PluginEntry, VersionEntry, VersionEngines};
    use serde_json::json;

    fn entry(name: &str, title: &str, keywords: &[&str]) -> PluginEntry {
        serde_json::from_value(json!({
            "name": name,
            "title": { "zh": title, "en": title },
            "metadata": { "keywords": keywords },
            "versions": [{
                "version": "0.1.0",
                "publishTimestamp": 1700000000,
                "source": { "type": "npm" },
                "engines": { "aidaw": ">=1.0.0" }
            }]
        }))
        .unwrap()
    }

    #[test]
    fn cjk_runs_become_bigrams_and_last_char() {
        assert_eq!(tokenize("和弦建议"), vec!["和弦", "弦建", "建议", "议"]);
        assert_eq!(tokenize("drum machine"), vec!["drum", "machine"]);
    }

    #[test]
    fn fullwidth_and_case_fold() {
        assert_eq!(normalize("ＤＲＵＭ"), "drum");
        assert_eq!(normalize("ＡＢＣ鼓"), "abc鼓");
    }

    #[test]
    fn matches_by_prefix_and_keyword_weight() {
        let entries = vec![
            entry("@aidaw/chord-suggest", "和弦建议", &["和弦", "配器"]),
            entry("@aidaw/drummer", "鼓机", &["drums"]),
        ];
        let taxonomy = |_: &PluginEntry| String::new();
        let hits = rank(&entries, "和弦", "zh", taxonomy, &Sort::Relevance);
        assert_eq!(hits, vec![0]);

        let hits = rank(&entries, "drum", "zh", taxonomy, &Sort::Relevance);
        assert_eq!(hits, vec![1]);
    }

    #[test]
    fn all_terms_must_match() {
        let entries = vec![entry("@aidaw/chord-suggest", "和弦建议", &["配器"])];
        let taxonomy = |_: &PluginEntry| String::new();
        assert!(rank(&entries, "和弦 不存在的词", "zh", taxonomy, &Sort::Relevance).is_empty());
    }

    #[test]
    fn fuzzy_only_for_long_terms() {
        let entries = vec![entry("@aidaw/drummer", "鼓机", &[])];
        let taxonomy = |_: &PluginEntry| String::new();
        // 6 个字符 → 容错 1：drummr（少一个 e）命中 drummer，且不是前缀命中。
        assert!(!rank(&entries, "drummr", "zh", taxonomy, &Sort::Relevance).is_empty());
        // 4 个字符 → 容错 1，drux 与 drummer 差 3，不命中。
        assert!(rank(&entries, "drux", "zh", taxonomy, &Sort::Relevance).is_empty());
    }

    #[test]
    fn empty_query_returns_everything_in_index_order() {
        let entries = vec![entry("@aidaw/a", "A", &[]), entry("@aidaw/b", "B", &[])];
        let taxonomy = |_: &PluginEntry| String::new();
        assert_eq!(rank(&entries, "", "zh", taxonomy, &Sort::Index), vec![0, 1]);
    }

    #[test]
    fn published_sort_prefers_newer() {
        let mut old = entry("@aidaw/old", "旧", &[]);
        let mut new = entry("@aidaw/new", "新", &[]);
        old.versions[0].publish_timestamp = 100;
        new.versions[0].publish_timestamp = 200;
        let entries = vec![old, new];
        let taxonomy = |_: &PluginEntry| String::new();
        assert_eq!(rank(&entries, "", "zh", taxonomy, &Sort::Published), vec![1, 0]);
    }

    #[test]
    fn sort_key_missing_values_go_last() {
        let mut a = entry("@aidaw/a", "A", &[]);
        let b = entry("@aidaw/b", "B", &[]);
        a.metadata.sort_keys.insert("downloads".into(), 10.0);
        let entries = vec![b, a];
        let taxonomy = |_: &PluginEntry| String::new();
        assert_eq!(
            rank(&entries, "", "zh", taxonomy, &Sort::Key("downloads".into())),
            vec![1, 0]
        );
    }

    #[test]
    fn type_is_parsed() {
        assert_eq!(Sort::parse("relevance"), Sort::Relevance);
        assert_eq!(Sort::parse("key:downloads"), Sort::Key("downloads".into()));
        assert_eq!(Sort::parse("nonsense"), Sort::Index);
    }

    #[test]
    fn unused_import_guard() {
        // 保持类型可用（避免测试里 unused import 警告）。
        let _ = VersionEntry {
            version: "0.1.0".into(),
            publish_timestamp: 0,
            source: crate::index_format::Source::Npm { registry: None },
            engines: VersionEngines { aidaw: "*".into() },
        };
    }
}
