//! **音素表与中文 G2P**（歌声合成用）。
//!
//! 两件事，都是纯查表、无模型：
//! 1. [`phone_id`]：`"zh_xiao3"` / `"<SP>"` 这类 token → 模型词表里的 id。
//!    词表是 SoulX-Singer 的 3033 个 token（`assets/phone_set.txt`，上游 Apache-2.0；
//!    经 SXSEditor 的 `phone_set.json` 转录，逐行一个、行号即 id）。
//! 2. [`char_phone`]：汉字 → 带声调的拼音音节 → 音素 id。
//!    音节表 `assets/zh_pinyin_map.tsv` 由 `pinyin-pro`（MIT）逐字转换生成，
//!    与 SXSEditor 的取音规则一致（多音字取**单字默认读音**，与参考实现相同）。
//!
//! **为什么要自带音节表**：模型词表里只有 1201 个 `zh_*` 音节，`pinyin-pro` 这类库
//! 会把 `女` 转成 `nü3`（参考实现再换成 `nv3`），而词表里**根本没有 `v` 韵母**。
//! 所以查表要带回落链：精确 → `v`/`u` 互换 → 同音节其他声调 → `<UNK>`。
//! 这条链让「词表没覆盖到的字」仍然能唱出来（声调可能不准），而不是整句变成 UNK。

use std::collections::HashMap;
use std::sync::OnceLock;

/// 音素词表（行号 = id）。
const PHONE_SET: &str = include_str!("../../assets/phone_set.txt");
/// 汉字 → 拼音音节（`字\txiao3` 一行一个）。
const PINYIN_MAP: &str = include_str!("../../assets/zh_pinyin_map.tsv");

/// `<UNK>` 的 id（词表缺失时的兜底，正常不会是 0）。
pub fn unk() -> u16 {
    phone_id("<UNK>")
}

fn table() -> &'static (Vec<&'static str>, HashMap<&'static str, u16>) {
    static TABLE: OnceLock<(Vec<&'static str>, HashMap<&'static str, u16>)> = OnceLock::new();
    TABLE.get_or_init(|| {
        let names: Vec<&'static str> = PHONE_SET.lines().collect();
        let mut index = HashMap::with_capacity(names.len());
        for (id, name) in names.iter().enumerate() {
            index.insert(*name, id as u16);
        }
        (names, index)
    })
}

/// token → id；表里没有返回 `<UNK>`。
pub fn phone_id(name: &str) -> u16 {
    let (_, index) = table();
    index
        .get(name)
        .copied()
        .unwrap_or_else(|| *index.get("<UNK>").unwrap_or(&0))
}

/// id → token（测试与日志用）。
#[allow(dead_code)]
pub fn phone_name(id: u16) -> &'static str {
    let (names, _) = table();
    names.get(id as usize).copied().unwrap_or("<UNK>")
}

/// 词表大小。
#[allow(dead_code)]
pub fn vocab_len() -> usize {
    table().0.len()
}

fn pinyin() -> &'static HashMap<char, &'static str> {
    static MAP: OnceLock<HashMap<char, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut map = HashMap::with_capacity(21_000);
        for line in PINYIN_MAP.lines() {
            if let Some((ch, syl)) = line.split_once('\t') {
                if let (Some(c), false) = (ch.chars().next(), syl.is_empty()) {
                    map.insert(c, syl);
                }
            }
        }
        map
    })
}

/// 汉字的默认拼音音节（不带 `zh_` 前缀，带声调数字）。非汉字 / 表外字返回 `None`。
pub fn syllable_of(c: char) -> Option<&'static str> {
    pinyin().get(&c).copied()
}

/// 汉字 → 音素 id。
///
/// `tone` 非空时强制用这个声调（歌词里写 `小3` 这种），否则用字表里的默认声调。
pub fn char_phone(c: char, tone: Option<char>) -> u16 {
    let Some(syl) = syllable_of(c) else {
        return unk();
    };
    let base = syl.trim_end_matches(|ch: char| ch.is_ascii_digit());
    let tone = tone
        .filter(|t| ('1'..='5').contains(t))
        .or_else(|| syl.chars().last().filter(|t| t.is_ascii_digit()))
        .unwrap_or('5');

    // 精确 → ü 写成 u（词表没有 v 韵母）→ 同音节其他声调 → UNK。
    let mut candidates = Vec::with_capacity(7);
    candidates.push(format!("zh_{base}{tone}"));
    if base.contains('v') {
        candidates.push(format!("zh_{}{tone}", base.replace('v', "u")));
    }
    for t in ['4', '1', '2', '3', '5'] {
        if t == tone {
            continue;
        }
        candidates.push(format!("zh_{base}{t}"));
        if base.contains('v') {
            candidates.push(format!("zh_{}{t}", base.replace('v', "u")));
        }
    }
    for cand in &candidates {
        let id = phone_id(cand);
        if phone_name(id) == cand.as_str() {
            return id;
        }
    }
    unk()
}

/// 「没有歌词时唱什么」：`la`（词表里一定有 `zh_la1`）。
pub fn la_phone() -> u16 {
    let id = phone_id("zh_la1");
    if phone_name(id) == "zh_la1" {
        id
    } else {
        unk()
    }
}

/// 休止 / 换气 token 的 id。
pub fn silence_phone() -> u16 {
    phone_id("<SP>")
}

/// 吸气 token 的 id。
#[allow(dead_code)]
pub fn breath_phone() -> u16 {
    phone_id("<AP>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocab_has_the_expected_specials() {
        assert_eq!(phone_name(phone_id("<PAD>")), "<PAD>");
        assert_eq!(phone_name(phone_id("<SP>")), "<SP>");
        assert_eq!(phone_name(phone_id("<BOW>")), "<BOW>");
        assert_eq!(phone_name(phone_id("<EOW>")), "<EOW>");
        assert!(vocab_len() > 3000, "词表应该有一千多个中文音节 + 英文音素");
    }

    #[test]
    fn chinese_chars_map_to_toned_syllables() {
        assert_eq!(phone_name(char_phone('小', None)), "zh_xiao3");
        assert_eq!(phone_name(char_phone('河', None)), "zh_he2");
        // 显式声调覆盖字表默认值。
        assert_eq!(phone_name(char_phone('小', Some('4'))), "zh_xiao4");
    }

    #[test]
    fn missing_syllables_fall_back_instead_of_unk() {
        // `女` 是 nü3：词表没有 v 韵母，回落链应该给到 u 韵母的某个声调。
        let id = char_phone('女', None);
        assert_ne!(id, unk(), "ü 韵母应该走 u 的回落，而不是 UNK");
        assert!(phone_name(id).starts_with("zh_nu"));
        // 完全表外的字符仍然是 UNK。
        assert_eq!(char_phone('§', None), unk());
    }

    #[test]
    fn every_pinyin_map_entry_is_a_syllable() {
        // 抽查：音节必须以 zh_ 开头、以 1..5 结尾。
        for c in ['春', '江', '花', '月', '夜'] {
            let name = phone_name(char_phone(c, None));
            assert!(name.starts_with("zh_"), "{c} → {name}");
            assert!(name.ends_with(|ch: char| ('1'..='5').contains(&ch)));
        }
    }
}
