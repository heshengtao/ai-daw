//! 影片清单（film.json）→ 编曲骨架。
//!
//! 这一层只做三件事：
//! 1. 把清单里的**秒**换算成拍（`beat = (t - offset) * bpm / 60`）；
//! 2. 把 `instrument` 语义名解析成内置乐器 + 参数补丁（不猜外部插件）；
//! 3. 产出 [`CueSheetPlan`]——引擎一次事务建好轨道 / 乐器 / 片段 / 段落 / 和弦。
//!
//! 不写工程、不碰音频；纯函数，方便单测。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use serde::Serialize;
use serde_json::{json, Value};

use ai_daw_engine::model::{
    CueClip, CueSheetPlan, CueTrack, HarmonyBar, Note, PitchExpression, PitchRange, Section,
};
use ai_daw_host_vst3::CatalogPlugin;
use ai_daw_plugin_api::PluginKind;

use crate::{factory_presets, plugin_info};

// ---------------------------------------------------------------------------
// 音色映射
// ---------------------------------------------------------------------------

/// 一条语义音色规则。`needles` 是折叠后（小写、去标点）的片段。
struct Rule {
    needles: &'static [&'static str],
    plugin_id: &'static str,
    /// 内置合成器的出厂音色 id（参数补丁）。
    preset_id: Option<&'static str>,
    /// 在预设之上再覆盖的参数。
    params: &'static [(&'static str, f32)],
    effects: &'static [&'static str],
    why: &'static str,
}

/// 内置乐器里能近似哪些民族/世界乐器。
///
/// 原则：**只用真实存在的内置音色**，并在 `why` 里说清「这是近似」——
/// 宁可告诉模型「没有古琴，用尼龙吉他 + 混响」，也不编一个不存在的插件。
const RULES: &[Rule] = &[
    Rule {
        needles: &["guqin", "guqinlike", "古琴", "qin"],
        plugin_id: "builtin.nylon_guitar",
        preset_id: None,
        params: &[],
        effects: &["builtin.reverb"],
        why: "没有古琴音源；尼龙弦木吉他的拨弦 + 混响是最接近的内置近似（音域偏低、衰减自然）",
    },
    Rule {
        needles: &[
            "guzheng", "zheng", "古筝", "koto", "筝", "pipa", "琵琶", "pluck", "拨弦", "拨奏",
        ],
        plugin_id: "builtin.pizzicato",
        preset_id: None,
        params: &[],
        effects: &["builtin.reverb"],
        why: "弦乐拨奏（pizzicato）提供短促的拨弦；古筝/琵琶/筝的刮奏用快速五声音阶跑动模拟",
    },
    Rule {
        needles: &[
            "gamelan",
            "甘美兰",
            "印尼",
            "metallophone",
            "bell",
            "钟",
            "glock",
        ],
        plugin_id: "builtin.synth",
        preset_id: Some("builtin.synth.ep.tine"),
        params: &[],
        effects: &["builtin.reverb"],
        why: "甘美兰/金属打击乐用 FM 电钢的 tine 音色近似：金属泛音 + 快衰减，加混响拉空间",
    },
    Rule {
        needles: &["shamisen", "三味线", "sitar", "西塔琴", "banjo", "班卓"],
        plugin_id: "builtin.nylon_guitar",
        preset_id: None,
        params: &[],
        effects: &["builtin.reverb"],
        why: "拨弦类没有专用音源；尼龙弦吉他最接近，短音符 + 快速交替拨弦可模拟三味线",
    },
    Rule {
        needles: &["erhu", "二胡", "huqin", "胡琴"],
        plugin_id: "builtin.violin",
        preset_id: None,
        params: &[],
        effects: &["builtin.reverb"],
        why: "二胡用独奏小提琴近似（单音、连奏）；滑音用音符 expr 的 bend_to + vibrato 写",
    },
    Rule {
        needles: &[
            "dizi", "笛子", "flute", "箫", "xiao", "ocarina", "陶笛", "whistle",
        ],
        plugin_id: "builtin.synth",
        preset_id: Some("builtin.synth.winds.flute"),
        params: &[],
        effects: &["builtin.reverb"],
        why: "内置没有采样笛子；合成器的 flute 预设 + 混响可以做出气息感",
    },
    Rule {
        needles: &["piano", "钢琴", "keys"],
        plugin_id: "builtin.piano",
        preset_id: None,
        params: &[],
        effects: &[],
        why: "内置采样原声钢琴，直接可用",
    },
    Rule {
        needles: &["strings", "弦乐", "violin", "cello", "ensemble"],
        plugin_id: "builtin.strings",
        preset_id: None,
        params: &[],
        effects: &[],
        why: "内置采样弦乐群奏；长音、铺和弦都合适",
    },
    Rule {
        needles: &["bass", "贝斯", "低音", "sub"],
        plugin_id: "builtin.bass",
        preset_id: None,
        params: &[],
        effects: &[],
        why: "内置采样指弹贝斯，写根音与八分音符 bass line",
    },
    Rule {
        needles: &["drums", "鼓", "percussion", "打击", "kick", "snare"],
        plugin_id: "builtin.drums",
        preset_id: None,
        params: &[],
        effects: &[],
        why: "内置采样原声鼓组（GM 35–59）",
    },
    Rule {
        needles: &["pad", "铺底", "ambient", "drone", "atmos"],
        plugin_id: "builtin.synth",
        preset_id: Some("builtin.synth.pad"),
        params: &[],
        effects: &["builtin.reverb"],
        why: "内置合成器的 pad 预设：慢起慢落，铺和弦",
    },
    Rule {
        needles: &["brass", "铜管", "trumpet", "horn"],
        plugin_id: "builtin.synth_brass_2",
        preset_id: None,
        params: &[],
        effects: &[],
        why: "内置采样 DX7 风格合成铜管，适合副歌和弦与号角式主音",
    },
    Rule {
        needles: &["guitar", "吉他", "nylon", "acoustic"],
        plugin_id: "builtin.nylon_guitar",
        preset_id: None,
        params: &[],
        effects: &[],
        why: "内置采样尼龙弦木吉他",
    },
    Rule {
        needles: &["vaporwave", "蒸汽波", "tape", "复古电子", "citypop"],
        plugin_id: "builtin.synth",
        preset_id: Some("builtin.synth.ep.chorus"),
        params: &[],
        effects: &["builtin.chorus", "builtin.reverb"],
        why: "蒸汽波的磁带感用合唱电钢 + 混响近似；真正的 varispeed/tape 由外部渲染或 Web Audio 做",
    },
    Rule {
        needles: &["lead", "主音", "melody"],
        plugin_id: "builtin.synth",
        preset_id: Some("builtin.synth.lead"),
        params: &[],
        effects: &[],
        why: "内置合成器的 8-bit 主音预设",
    },
];

/// 折叠：小写、只留 Unicode 字母数字（保留中文），`JC-303` / `jc 303` 视为同一个。
fn fold(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct InstrumentSuggestion {
    pub plugin_id: String,
    pub name: String,
    pub kind: String,
    pub category: String,
    /// `high` / `medium` / `low` / `approximation`。
    pub confidence: String,
    /// `installed` = 本机扫描到的 VST3/CLAP/AU；`builtin` = 内置；`approximation` = 语义近似。
    pub source: String,
    /// 排序分（越大越靠前）；安装插件的名字命中永远高于内置近似。
    pub score: u32,
    pub why: String,
    /// 插件自己的描述（VST3 moduleinfo / CLAP descriptor）；内置/近似为 `None`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset_id: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<String>,
    pub roles: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pitch_range: Option<[u8; 2]>,
    pub mono: bool,
}

/// 查询里的噪声词：不该拿去匹配插件名。
const STOPWORDS: &[&str] = &[
    "like",
    "sound",
    "style",
    "instrument",
    "voice",
    "音色",
    "乐器",
    "类似",
];

fn preset_params(id: &str) -> Option<BTreeMap<String, f32>> {
    factory_presets::all()
        .iter()
        .find(|preset| preset.id == id)
        .map(|preset| preset.params.clone())
}

fn confidence_for(score: u32) -> &'static str {
    if score >= 85 {
        "high"
    } else if score >= 55 {
        "medium"
    } else {
        "low"
    }
}

fn rule_suggestion(rule: &Rule) -> Option<InstrumentSuggestion> {
    let entries = plugin_info::entries(&[]);
    let entry = entries.iter().find(|entry| entry.id == rule.plugin_id)?;
    let card = plugin_info::brief(entry);
    let mut params = rule.preset_id.and_then(preset_params).unwrap_or_default();
    for (key, value) in rule.params {
        params.insert((*key).to_string(), *value);
    }
    Some(InstrumentSuggestion {
        plugin_id: card.id.clone(),
        name: card.name.clone(),
        kind: plugin_kind_label(card.kind).to_string(),
        category: card.category.to_string(),
        confidence: "approximation".into(),
        source: "approximation".into(),
        score: 20,
        why: rule.why.to_string(),
        native_description: None,
        preset_id: rule.preset_id.map(str::to_string),
        params,
        effects: rule.effects.iter().map(|id| (*id).to_string()).collect(),
        roles: card.roles.iter().map(|role| (*role).to_string()).collect(),
        pitch_range: card.pitch_range,
        mono: card.mono,
    })
}

fn plugin_kind_label(kind: PluginKind) -> &'static str {
    match kind {
        PluginKind::Instrument => "instrument",
        PluginKind::Effect => "effect",
    }
}

fn catalog_suggestion(entry: &plugin_info::Entry, score: u32, why: String) -> InstrumentSuggestion {
    let card = plugin_info::brief(entry);
    // 安装插件在同等匹配下优先于内置：用户特意装的音源通常比近似更合适。
    let score = if entry.builtin {
        score
    } else {
        (score + 15).min(100)
    };
    InstrumentSuggestion {
        plugin_id: card.id.clone(),
        name: card.name.clone(),
        kind: plugin_kind_label(card.kind).to_string(),
        category: card.category.to_string(),
        confidence: confidence_for(score).into(),
        source: if entry.builtin {
            "builtin"
        } else {
            "installed"
        }
        .into(),
        score,
        why,
        native_description: card.native_description.clone(),
        preset_id: None,
        params: BTreeMap::new(),
        effects: Vec::new(),
        roles: card.roles.iter().map(|role| (*role).to_string()).collect(),
        pitch_range: card.pitch_range,
        mono: card.mono,
    }
}

/// 查询命中的语义规则（用于补充跨语言别名：`guqin-like` 也会搜「古琴」）。
fn matched_rules(query: &str) -> Vec<&'static Rule> {
    let needle = fold(query);
    if needle.is_empty() {
        return Vec::new();
    }
    RULES
        .iter()
        .filter(|rule| {
            rule.needles.iter().any(|item| {
                let folded = fold(item);
                !folded.is_empty() && needle.contains(&folded)
            })
        })
        .collect()
}

fn push_term(terms: &mut Vec<String>, value: &str) {
    let folded = fold(value);
    if folded.chars().count() < 2 || STOPWORDS.contains(&folded.as_str()) {
        return;
    }
    if !terms.contains(&folded) {
        terms.push(folded);
    }
}

/// 查询词 + 规则别名（中英双语），长的优先。
fn terms_for(query: &str, rules: &[&'static Rule]) -> Vec<String> {
    let mut terms = Vec::new();
    push_term(&mut terms, query);
    for part in query.split(|value: char| !value.is_alphanumeric()) {
        push_term(&mut terms, part);
    }
    for rule in rules {
        for needle in rule.needles {
            push_term(&mut terms, needle);
        }
    }
    terms.sort_by_key(|term| std::cmp::Reverse(term.chars().count()));
    terms
}

fn bump(best: &mut Option<(u32, String)>, score: u32, why: String) {
    if best.as_ref().map_or(true, |(current, _)| score > *current) {
        *best = Some((score, why));
    }
}

/// 给一条插件卡片打分：安装插件的名字命中 > 标签 > 安装包文件名 > 厂商/id > 描述。
fn score_card(
    card: &plugin_info::Brief,
    terms: &[String],
    path_hint: &str,
) -> Option<(u32, String)> {
    let name = fold(&card.name);
    let tags = fold(&card.tags.join(" "));
    let native = fold(card.native_description.as_deref().unwrap_or(""));
    let path = fold(path_hint);
    let vendor = fold(&card.vendor);
    let id = fold(&card.id);
    let description = fold(&card.description);
    let category = fold(card.category);
    let mut best: Option<(u32, String)> = None;
    for term in terms {
        if name == *term {
            return Some((100, format!("插件名精确命中「{term}」")));
        }
        if name.contains(term) {
            bump(&mut best, 85, format!("插件名包含「{term}」"));
        }
        if tags.contains(term) {
            bump(&mut best, 70, format!("标签命中「{term}」"));
        }
        if native.contains(term) {
            bump(&mut best, 65, format!("插件自述命中「{term}」"));
        }
        if path.contains(term) {
            bump(&mut best, 60, format!("安装包文件名命中「{term}」"));
        }
        if vendor.contains(term) || id.contains(term) {
            bump(&mut best, 55, format!("厂商/id 命中「{term}」"));
        }
        if description.contains(term) || category.contains(term) {
            bump(&mut best, 40, format!("描述/分类命中「{term}」"));
        }
    }
    best
}

/// 把近似方案里的音色提示（preset / params / effects）并进已安装或内置的候选。
/// 例：`pad` 命中内置 `builtin.synth` 的标签，但 `builtin.synth.pad` 的预设只在规则里，
/// 不合并就会丢掉 preset。
fn merge_hints(target: &mut InstrumentSuggestion, hint: &InstrumentSuggestion) {
    if target.preset_id.is_none() {
        target.preset_id = hint.preset_id.clone();
    }
    for (key, value) in &hint.params {
        target.params.entry(key.clone()).or_insert(*value);
    }
    for effect in &hint.effects {
        if !target.effects.contains(effect) {
            target.effects.push(effect.clone());
        }
    }
}

fn insert_best(out: &mut Vec<InstrumentSuggestion>, candidate: InstrumentSuggestion) {
    if let Some(existing) = out
        .iter_mut()
        .find(|item| item.plugin_id == candidate.plugin_id)
    {
        if candidate.score > existing.score {
            let mut next = candidate;
            if existing.source == "approximation" {
                merge_hints(&mut next, existing);
            }
            *existing = next;
        } else if candidate.source == "approximation" {
            merge_hints(existing, &candidate);
        }
    } else {
        out.push(candidate);
    }
}

/// 按语义名/插件名找候选音色。
///
/// 排序规则：**已安装插件**的名字/标签命中永远排在语义近似之前。查询 `guqin-like` 时，
/// 本机装的 `Guqin` 或 `古琴` 都会命中（规则别名提供中英双语），没有装才退回内置近似。
pub(crate) fn suggestions(
    query: &str,
    kind: Option<PluginKind>,
    limit: usize,
    catalog: &[CatalogPlugin],
) -> Vec<InstrumentSuggestion> {
    let rules = matched_rules(query);
    let terms = terms_for(query, &rules);
    let entries = plugin_info::entries(catalog);
    let mut out: Vec<InstrumentSuggestion> = Vec::new();

    if terms.is_empty() {
        for entry in entries
            .iter()
            .filter(|entry| kind.map_or(true, |k| entry.kind == k))
        {
            out.push(catalog_suggestion(entry, 0, "按当前目录列出的候选".into()));
            if out.len() >= limit.clamp(1, 50) {
                break;
            }
        }
        return out;
    }

    for entry in &entries {
        if let Some(kind) = kind {
            if entry.kind != kind {
                continue;
            }
        }
        let card = plugin_info::brief(entry);
        let path_hint = entry
            .path
            .as_ref()
            .and_then(|path| path.file_stem())
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if let Some((score, why)) = score_card(&card, &terms, path_hint) {
            insert_best(&mut out, catalog_suggestion(entry, score, why));
        }
    }

    // 没有安装插件命中名字/标签时，让语义规则的目标内置乐器在同类里优先
    // （`strings` 应该先给 builtin.strings，而不是名字里同样含「弦乐」的合成弦乐）。
    let installed_match = out
        .iter()
        .any(|item| item.source == "installed" && item.score >= 60);
    if !installed_match {
        let targets: HashSet<&str> = rules.iter().map(|rule| rule.plugin_id).collect();
        for item in &mut out {
            if item.source == "builtin" && targets.contains(item.plugin_id.as_str()) {
                item.score = item.score.max(88);
                if !item.why.is_empty() {
                    item.why.push_str("（语义规则目标）");
                }
            }
        }
    }

    if kind != Some(PluginKind::Effect) {
        for rule in rules {
            if let Some(suggestion) = rule_suggestion(rule) {
                insert_best(&mut out, suggestion);
            }
        }
    }

    out.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.name.cmp(&b.name)));
    out.truncate(limit.clamp(1, 50));
    out
}

/// 导入时用的单条解析：先按上面的排名取第一；没有命中再退回目录解析。
pub(crate) fn resolve_instrument(
    query: &str,
    catalog: &[CatalogPlugin],
) -> Option<InstrumentSuggestion> {
    if let Some(best) = suggestions(query, Some(PluginKind::Instrument), 8, catalog)
        .into_iter()
        .next()
    {
        return Some(best);
    }
    let entries = plugin_info::entries(catalog);
    let entry = plugin_info::resolve(&entries, query).ok()?;
    Some(catalog_suggestion(
        &entry,
        50,
        format!("按插件名/id 解析「{query}」"),
    ))
}

// ---------------------------------------------------------------------------
// film.json → 计划
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub(crate) struct ImportOptions {
    /// 先清空工程（含默认分轨模板）再导入。
    pub clear: bool,
    /// 显式工程名；`None` = 不改名。
    pub name: Option<String>,
    /// 覆盖 manifest 的 BPM。
    pub bpm: Option<f64>,
    /// 覆盖 manifest 的 offset（秒）。
    pub offset: Option<f64>,
    /// 是否新建轨道（false = 只导入速度/段落/和弦）。
    pub create_tracks: bool,
    /// 是否解析 `instrument` 并挂乐器。
    pub apply_instruments: bool,
}

pub(crate) struct CueSheetImport {
    pub plan: CueSheetPlan,
    pub summary: Value,
    pub warnings: Vec<String>,
}

fn number(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
}

fn text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn parse_time_sig(value: Option<&Value>) -> Option<(u8, u8)> {
    let value = value?;
    if let Some(text) = value.as_str() {
        let (num, den) = text.split_once('/')?;
        return Some((num.trim().parse().ok()?, den.trim().parse().ok()?));
    }
    let object = value.as_object()?;
    let num = object.get("num").and_then(Value::as_u64)? as u8;
    let den = object.get("den").and_then(Value::as_u64)? as u8;
    Some((num, den))
}

/// 支持 `{min,max}` 与 `[min,max]` 两种写法。
fn parse_pitch_range(value: Option<&Value>) -> Option<PitchRange> {
    if let Some(array) = value?.as_array() {
        let min = array.first()?.as_u64()? as u8;
        let max = array.get(1)?.as_u64()? as u8;
        return (max >= min).then_some(PitchRange { min, max });
    }
    let object = value?.as_object()?;
    let min = object.get("min").and_then(Value::as_u64)? as u8;
    let max = object.get("max").and_then(Value::as_u64)? as u8;
    (max >= min).then_some(PitchRange { min, max })
}

/// 把清单解析成引擎计划。`raw` 是已解析的 film.json。
pub(crate) fn build_plan(
    raw: &Value,
    options: &ImportOptions,
    catalog: &[CatalogPlugin],
) -> Result<CueSheetImport, String> {
    let mut warnings = Vec::new();
    let music = raw.get("music").and_then(Value::as_object);
    let manifest_bpm = music.and_then(|m| number(m.get("bpm")));
    let bpm = options
        .bpm
        .or(manifest_bpm)
        .unwrap_or(120.0)
        .clamp(1.0, 1000.0);
    if options.bpm.is_none() && manifest_bpm.is_none() {
        warnings.push("film.json 没有 music.bpm，按 120 BPM 换算".into());
    }
    let offset = options
        .offset
        .or_else(|| music.and_then(|m| number(m.get("offset"))))
        .unwrap_or(0.0);
    let time_sig = music
        .and_then(|m| {
            parse_time_sig(m.get("time_signature"))
                .or_else(|| parse_time_sig(m.get("timeSignature")))
        })
        .unwrap_or((4, 4));

    let to_beat = |seconds: f64| -> f64 {
        let beat = (seconds - offset) * bpm / 60.0;
        if beat.is_finite() {
            beat.max(0.0)
        } else {
            0.0
        }
    };

    // ---- 段落：来自 scenes ----
    let mut sections: Vec<Section> = Vec::new();
    if let Some(scenes) = raw.get("scenes").and_then(Value::as_array) {
        for (index, scene) in scenes.iter().enumerate() {
            let id = text(scene.get("id")).unwrap_or_else(|| format!("s{:02}", index + 1));
            let t0 = number(scene.get("t0"));
            let t1 = number(scene.get("t1"));
            let (Some(t0), Some(t1)) = (t0, t1) else {
                warnings.push(format!("场景 {id} 缺少 t0/t1，已跳过段落"));
                continue;
            };
            let start = to_beat(t0);
            let end = to_beat(t1);
            if !(end > start) {
                warnings.push(format!("场景 {id} 的拍长非正，已跳过段落"));
                continue;
            }
            sections.push(Section {
                name: text(scene.get("title")).unwrap_or(id),
                start,
                end,
                role: text(scene.get("role")).or_else(|| Some("scene".into())),
            });
        }
    }
    sections.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // ---- 和弦：music.harmony / music.chords / cues[].chord ----
    let mut harmony: Vec<HarmonyBar> = Vec::new();
    let harmony_source = music
        .and_then(|m| m.get("harmony"))
        .and_then(Value::as_array)
        .or_else(|| {
            music
                .and_then(|m| m.get("chords"))
                .and_then(Value::as_array)
        });
    if let Some(bars) = harmony_source {
        for (index, bar) in bars.iter().enumerate() {
            let symbol = text(bar.get("symbol")).or_else(|| text(bar.get("chord")));
            let Some(symbol) = symbol else {
                warnings.push(format!("和弦表第 {} 条缺少 symbol，已跳过", index + 1));
                continue;
            };
            if let Err(error) = ai_daw_engine::harmony::parse_chord(&symbol) {
                warnings.push(format!("和弦「{symbol}」无法识别（{error}），已跳过"));
                continue;
            }
            let t0 = number(bar.get("t0")).or_else(|| number(bar.get("start")));
            let t1 = number(bar.get("t1"))
                .or_else(|| number(bar.get("length")).map(|length| t0.unwrap_or(0.0) + length));
            let (Some(t0), Some(t1)) = (t0, t1) else {
                warnings.push(format!("和弦「{symbol}」缺少时间范围，已跳过"));
                continue;
            };
            let start = to_beat(t0);
            let end = to_beat(t1);
            if !(end > start) {
                warnings.push(format!("和弦「{symbol}」拍长非正，已跳过"));
                continue;
            }
            harmony.push(HarmonyBar {
                start,
                length: end - start,
                symbol,
                source: Some("film.json".into()),
            });
        }
        harmony.sort_by(|a, b| {
            a.start
                .partial_cmp(&b.start)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    // ---- 轨道：按 instrument / track 分组 ----
    struct Group {
        name: String,
        instrument: Option<String>,
        plugin_id: Option<String>,
        preset_id: Option<String>,
        params: BTreeMap<String, f32>,
        effects: Vec<String>,
        gain_db: Option<f32>,
        pan: Option<f32>,
        pitch_range: Option<PitchRange>,
        clips: Vec<CueClip>,
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut group_index: HashMap<String, usize> = HashMap::new();

    if options.create_tracks {
        let catalog_entries = plugin_info::entries(catalog);
        if let Some(cues) = music.and_then(|m| m.get("cues")).and_then(Value::as_array) {
            for (index, cue) in cues.iter().enumerate() {
                let t0 = number(cue.get("t0"));
                let t1 = number(cue.get("t1"));
                let (Some(t0), Some(t1)) = (t0, t1) else {
                    warnings.push(format!("cue {} 缺少 t0/t1，已跳过", index + 1));
                    continue;
                };
                let start = to_beat(t0);
                let end = to_beat(t1);
                if !(end > start) {
                    warnings.push(format!("cue {} 拍长非正，已跳过", index + 1));
                    continue;
                }
                let instrument = text(cue.get("instrument"));
                let explicit_track = text(cue.get("track"));
                let name = explicit_track
                    .clone()
                    .or_else(|| instrument.clone())
                    .unwrap_or_else(|| "Cue".into());
                let key = fold(&name);
                let group = if let Some(index) = group_index.get(&key) {
                    &mut groups[*index]
                } else {
                    groups.push(Group {
                        name: name.clone(),
                        instrument: instrument.clone(),
                        plugin_id: None,
                        preset_id: None,
                        params: BTreeMap::new(),
                        effects: Vec::new(),
                        gain_db: None,
                        pan: None,
                        pitch_range: None,
                        clips: Vec::new(),
                    });
                    group_index.insert(key, groups.len() - 1);
                    groups.last_mut().expect("刚插入的组一定存在")
                };

                if group.gain_db.is_none() {
                    group.gain_db = number(cue.get("gainDb")).map(|value| value as f32);
                }
                if group.pan.is_none() {
                    group.pan = number(cue.get("pan")).map(|value| value as f32);
                }
                if group.pitch_range.is_none() {
                    group.pitch_range = parse_pitch_range(cue.get("pitchRange"))
                        .or_else(|| parse_pitch_range(cue.get("pitch_range")));
                }
                if let Some(effects) = cue.get("effects").and_then(Value::as_array) {
                    for effect in effects {
                        let Some(effect) = text(Some(effect)) else {
                            continue;
                        };
                        match plugin_info::resolve(&catalog_entries, &effect) {
                            Ok(entry) => {
                                if !group.effects.contains(&entry.id) {
                                    group.effects.push(entry.id);
                                }
                            }
                            Err(_) => warnings.push(format!(
                                "cue {} 的效果器「{effect}」未找到，已跳过",
                                index + 1
                            )),
                        }
                    }
                }
                if let Some(params) = cue.get("params").and_then(Value::as_object) {
                    for (key, value) in params {
                        if let Some(value) = value.as_f64() {
                            group.params.insert(key.clone(), value as f32);
                        }
                    }
                }
                if group.plugin_id.is_none() {
                    group.plugin_id = text(cue.get("pluginId"));
                }
                if group.preset_id.is_none() {
                    group.preset_id = text(cue.get("presetId"));
                }

                let mut notes = Vec::new();
                if let Some(list) = cue.get("notes").and_then(Value::as_array) {
                    for (note_index, note) in list.iter().enumerate() {
                        let Some(pitch) = note.get("pitch").and_then(Value::as_u64) else {
                            warnings.push(format!(
                                "cue {} 的第 {} 个音符缺少 pitch，已跳过",
                                index + 1,
                                note_index + 1
                            ));
                            continue;
                        };
                        let start = number(note.get("start")).unwrap_or(0.0);
                        let duration = number(note.get("duration")).unwrap_or(1.0);
                        let velocity =
                            note.get("velocity").and_then(Value::as_u64).unwrap_or(100) as u8;
                        notes.push(Note {
                            id: 0,
                            channel: None,
                            pressure: None,
                            pitch: pitch as u8,
                            start,
                            duration,
                            velocity,
                            volume: 127,
                            offset: 0.0,
                            legato: false,
                            expr: PitchExpression::NONE,
                        });
                    }
                }
                group.clips.push(CueClip {
                    start,
                    length: end - start,
                    notes,
                });
            }
        }
    }

    // ---- 音色解析 + 组装轨道 ----
    let mut tracks: Vec<CueTrack> = Vec::new();
    let mut track_summaries: Vec<Value> = Vec::new();
    for mut group in groups {
        group.clips.sort_by(|a, b| {
            a.start
                .partial_cmp(&b.start)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut plugin_id = group.plugin_id.clone();
        let mut params = group.params.clone();
        let mut effects = group.effects.clone();
        let mut role = None;
        let mut pitch_range = group.pitch_range;
        let mut mono = false;
        let mut resolved_label = None;

        if options.apply_instruments && plugin_id.is_none() {
            let query = group
                .instrument
                .clone()
                .unwrap_or_else(|| group.name.clone());
            if let Some(suggestion) = resolve_instrument(&query, catalog) {
                plugin_id = Some(suggestion.plugin_id.clone());
                resolved_label = Some(suggestion.name.clone());
                for (key, value) in &suggestion.params {
                    params.entry(key.clone()).or_insert(*value);
                }
                for effect in &suggestion.effects {
                    if !effects.contains(effect) {
                        effects.push(effect.clone());
                    }
                }
                if pitch_range.is_none() {
                    if let Some(range) = suggestion.pitch_range {
                        pitch_range = Some(PitchRange {
                            min: range[0],
                            max: range[1],
                        });
                    }
                }
                mono = suggestion.mono;
                role = suggestion.roles.first().map(|value| (*value).to_string());
            } else if !query.trim().is_empty() && query != "Cue" {
                warnings.push(format!(
                    "音色「{query}」没有匹配到内置/已安装乐器；轨道已建但未挂乐器"
                ));
            }
        }

        // cue 里显式给了 presetId：把出厂音色参数补进来（显式 params 优先）。
        if let Some(preset_id) = &group.preset_id {
            if let Some(patch) = preset_params(preset_id) {
                for (key, value) in patch {
                    params.entry(key).or_insert(value);
                }
            } else {
                warnings.push(format!("未知出厂音色「{preset_id}」，已忽略"));
            }
        }

        if plugin_id.is_none() {
            // 没有音色也保留轨道，agent 可以后补；但不写空 params。
            params.clear();
            effects.clear();
            pitch_range = None;
        }

        let clip_count = group.clips.len();
        let note_count: usize = group.clips.iter().map(|clip| clip.notes.len()).sum();
        tracks.push(CueTrack {
            name: group.name.clone(),
            plugin_id: plugin_id.clone(),
            params,
            effects: effects.clone(),
            role: role.clone(),
            pitch_range,
            gain_db: group.gain_db,
            pan: group.pan,
            clips: group.clips,
        });
        track_summaries.push(json!({
            "name": group.name,
            "instrument": group.instrument,
            "pluginId": plugin_id,
            "resolvedName": resolved_label,
            "role": role,
            "pitchRange": pitch_range.map(|r| [r.min, r.max]),
            "mono": mono,
            "effects": effects,
            "clips": clip_count,
            "notes": note_count,
        }));
    }

    let summary = json!({
        "bpm": bpm,
        "timeSignature": [time_sig.0, time_sig.1],
        "offset": offset,
        "key": music.and_then(|m| text(m.get("key"))),
        "bars": music.and_then(|m| number(m.get("bars"))),
        "sections": sections.len(),
        "harmony": harmony.len(),
        "tracks": track_summaries,
        "warnings": warnings,
    });

    Ok(CueSheetImport {
        plan: CueSheetPlan {
            clear: options.clear,
            name: options.name.clone(),
            tempo: Some(bpm),
            time_sig: Some(time_sig),
            sections,
            harmony,
            tracks,
        },
        summary,
        warnings,
    })
}

/// 从清单里读出工程名（给「没显式给 name 时用 manifest.name」的场景留个口子）。
pub(crate) fn manifest_name(raw: &Value) -> Option<String> {
    text(raw.get("name"))
}

// ---------------------------------------------------------------------------
// 文件名
// ---------------------------------------------------------------------------

/// 轨道名 → 安全文件名。保留中文，替换路径非法字符；空名回退。
pub(crate) fn safe_file_name(name: &str, fallback: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|value| {
            if value.is_control()
                || matches!(value, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            {
                '_'
            } else {
                value
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').to_string();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.chars().take(80).collect()
    }
}

/// 唯一化：`a.wav`、`a-2.wav`……
pub(crate) fn unique_stem_path(
    dir: &Path,
    stem: &str,
    used: &mut HashSet<String>,
) -> std::path::PathBuf {
    let mut candidate = format!("{stem}.wav");
    let mut index = 2;
    while used.contains(&candidate.to_lowercase()) {
        candidate = format!("{stem}-{index}.wav");
        index += 1;
    }
    used.insert(candidate.to_lowercase());
    dir.join(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Value {
        json!({
            "version": 1,
            "name": "137s 草稿",
            "fps": 60,
            "scenes": [
                {"id": "s01", "t0": 0.0, "t1": 4.0, "title": "开场"},
                {"id": "s02", "t0": 4.0, "t1": 8.0, "title": "发展"}
            ],
            "music": {
                "bpm": 120,
                "offset": 0,
                "timeSignature": "4/4",
                "cues": [
                    {"scene": "s01", "t0": 0.0, "t1": 4.0, "instrument": "guqin-like", "gainDb": -3},
                    {"scene": "s02", "t0": 4.0, "t1": 8.0, "instrument": "guqin-like"},
                    {"scene": "s02", "t0": 4.0, "t1": 6.0, "instrument": "gamelan-like"}
                ],
                "harmony": [
                    {"t0": 0.0, "t1": 2.0, "symbol": "Am"},
                    {"t0": 2.0, "t1": 4.0, "symbol": "F"}
                ]
            }
        })
    }

    #[test]
    fn seconds_become_beats_and_sections_follow_scenes() {
        let import = build_plan(
            &manifest(),
            &ImportOptions {
                create_tracks: true,
                apply_instruments: true,
                ..Default::default()
            },
            &[],
        )
        .unwrap();
        assert_eq!(import.plan.tempo, Some(120.0));
        // 120 BPM 4/4：1 秒 = 2 拍。
        assert_eq!(import.plan.sections.len(), 2);
        assert_eq!(import.plan.sections[0].start, 0.0);
        assert_eq!(import.plan.sections[0].end, 8.0);
        assert_eq!(import.plan.sections[1].start, 8.0);
        assert_eq!(import.plan.harmony.len(), 2);
        assert_eq!(import.plan.harmony[0].length, 4.0);
    }

    #[test]
    fn cues_group_by_instrument_and_get_a_builtin_instrument() {
        let import = build_plan(
            &manifest(),
            &ImportOptions {
                create_tracks: true,
                apply_instruments: true,
                ..Default::default()
            },
            &[],
        )
        .unwrap();
        assert_eq!(
            import.plan.tracks.len(),
            2,
            "guqin-like 两条 cue 合成一条轨，gamelan 单独一条"
        );
        let guqin = import
            .plan
            .tracks
            .iter()
            .find(|track| track.name == "guqin-like")
            .unwrap();
        assert_eq!(guqin.plugin_id.as_deref(), Some("builtin.nylon_guitar"));
        assert_eq!(guqin.clips.len(), 2);
        assert_eq!(guqin.clips[0].length, 8.0);
        assert_eq!(guqin.gain_db, Some(-3.0));
        assert!(guqin.effects.contains(&"builtin.reverb".to_string()));

        let gamelan = import
            .plan
            .tracks
            .iter()
            .find(|track| track.name == "gamelan-like")
            .unwrap();
        assert_eq!(gamelan.plugin_id.as_deref(), Some("builtin.synth"));
        assert_eq!(gamelan.clips[0].length, 4.0);
    }

    #[test]
    fn offset_moves_the_beat_origin() {
        let mut raw = manifest();
        raw["music"]["offset"] = json!(2.0);
        let import = build_plan(
            &raw,
            &ImportOptions {
                create_tracks: false,
                apply_instruments: false,
                ..Default::default()
            },
            &[],
        )
        .unwrap();
        // 场景 0..4s、offset=2s → 0..4 拍（而不是 0..8）。
        assert_eq!(import.plan.sections[0].start, 0.0);
        assert_eq!(import.plan.sections[0].end, 4.0);
    }

    #[test]
    fn invalid_chords_are_skipped_with_a_warning() {
        let mut raw = manifest();
        raw["music"]["harmony"] = json!([
            {"t0": 0.0, "t1": 2.0, "symbol": "not-a-chord"},
            {"t0": 2.0, "t1": 4.0, "symbol": "C"}
        ]);
        let import = build_plan(
            &raw,
            &ImportOptions {
                create_tracks: false,
                apply_instruments: false,
                ..Default::default()
            },
            &[],
        )
        .unwrap();
        assert_eq!(import.plan.harmony.len(), 1);
        assert!(import.warnings.join("").contains("无法识别"));
    }

    #[test]
    fn unknown_instrument_keeps_the_track_and_warns() {
        let raw = json!({
            "version": 1,
            "music": {
                "bpm": 120,
                "cues": [{"t0": 0.0, "t1": 2.0, "instrument": "theremin-like"}]
            }
        });
        let import = build_plan(
            &raw,
            &ImportOptions {
                create_tracks: true,
                apply_instruments: true,
                ..Default::default()
            },
            &[],
        )
        .unwrap();
        assert_eq!(import.plan.tracks.len(), 1);
        assert!(import.plan.tracks[0].plugin_id.is_none());
        assert!(import.warnings.join("").contains("theremin-like"));
    }

    #[test]
    fn cue_notes_are_parsed_relative_to_the_clip() {
        let raw = json!({
            "version": 1,
            "music": {
                "bpm": 120,
                "cues": [{
                    "t0": 0.0, "t1": 2.0, "instrument": "piano",
                    "notes": [
                        {"pitch": 60, "start": 0.0, "duration": 1.0, "velocity": 90},
                        {"pitch": 64, "start": 1.0, "duration": 1.0}
                    ]
                }]
            }
        });
        let import = build_plan(
            &raw,
            &ImportOptions {
                create_tracks: true,
                apply_instruments: true,
                ..Default::default()
            },
            &[],
        )
        .unwrap();
        let notes = &import.plan.tracks[0].clips[0].notes;
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].pitch, 60);
        assert_eq!(notes[1].velocity, 100, "缺省力度 100");
    }

    #[test]
    fn semantic_resolution_covers_the_projects_world_instruments() {
        for (query, expected) in [
            ("guqin-like", "builtin.nylon_guitar"),
            ("古琴", "builtin.nylon_guitar"),
            ("guzheng", "builtin.pizzicato"),
            ("gamelan-like", "builtin.synth"),
            ("shamisen-like", "builtin.nylon_guitar"),
            ("erhu-like", "builtin.violin"),
            ("dizi-like", "builtin.synth"),
            ("piano", "builtin.piano"),
            ("strings", "builtin.strings"),
            ("bass", "builtin.bass"),
            ("drums", "builtin.drums"),
        ] {
            let resolved =
                resolve_instrument(query, &[]).unwrap_or_else(|| panic!("{query} 应能解析"));
            assert_eq!(resolved.plugin_id, expected, "{query}");
        }
    }

    #[test]
    fn find_returns_approximation_when_nothing_is_installed() {
        let list = suggestions("guqin-like", None, 10, &[]);
        assert!(!list.is_empty());
        assert_eq!(list[0].plugin_id, "builtin.nylon_guitar");
        assert_eq!(list[0].source, "approximation");
        assert_eq!(list[0].confidence, "approximation");
        assert!(list.iter().all(|item| item.kind == "instrument"));
    }

    fn external(id: &str, name: &str, vendor: &str) -> CatalogPlugin {
        CatalogPlugin {
            id: id.into(),
            name: name.into(),
            vendor: vendor.into(),
            kind: PluginKind::Instrument,
            format: "vst3".into(),
            accepts_midi: true,
            has_editor: false,
            description: None,
            path: std::path::PathBuf::from("/nonexistent"),
        }
    }

    fn external_described(id: &str, name: &str, vendor: &str, description: &str) -> CatalogPlugin {
        let mut entry = external(id, name, vendor);
        entry.description = Some(description.into());
        entry
    }

    /// 装了真插件就优先用它：`guqin-like` 不能永远先返回内置近似。
    #[test]
    fn installed_plugin_outranks_the_approximation() {
        for name in ["Guqin", "古琴"] {
            let catalog = vec![external("vst3:guqin", name, "Ethno Labs")];
            let list = suggestions("guqin-like", None, 10, &catalog);
            assert_eq!(list[0].plugin_id, "vst3:guqin", "{name}");
            assert_eq!(list[0].source, "installed");
            assert_eq!(list[0].confidence, "high");
            assert!(
                list.iter()
                    .any(|item| item.plugin_id == "builtin.nylon_guitar"
                        && item.source == "approximation"),
                "近似方案仍应作为备选返回"
            );
            assert_eq!(
                resolve_instrument("guqin-like", &catalog)
                    .expect("应能解析")
                    .plugin_id,
                "vst3:guqin",
                "导入时也必须优先用已安装插件"
            );
        }
    }

    /// 导入 cue 时，装了真乐器就挂真乐器，而不是内置近似。
    #[test]
    fn cue_import_prefers_an_installed_instrument() {
        let catalog = vec![external("vst3:guqin", "Guqin", "Ethno Labs")];
        let import = build_plan(
            &manifest(),
            &ImportOptions {
                create_tracks: true,
                apply_instruments: true,
                ..Default::default()
            },
            &catalog,
        )
        .unwrap();
        let guqin = import
            .plan
            .tracks
            .iter()
            .find(|track| track.name == "guqin-like")
            .expect("应有 guqin-like 轨");
        assert_eq!(guqin.plugin_id.as_deref(), Some("vst3:guqin"));
    }

    /// 查询语言不同也能命中：英文 `guqin` 能搜到中文名「古琴」，反之亦然。
    #[test]
    fn cross_language_aliases_match_installed_plugins() {
        let chinese = vec![external("vst3:qin", "古琴", "Ethno Labs")];
        assert_eq!(
            suggestions("guqin", None, 5, &chinese)[0].plugin_id,
            "vst3:qin"
        );

        let english = vec![external("vst3:qin", "Guqin", "Ethno Labs")];
        assert_eq!(
            suggestions("古琴", None, 5, &english)[0].plugin_id,
            "vst3:qin"
        );
    }

    /// 插件名是代号、但自述里写了 `guqin`：必须能搜到，且优先于内置近似。
    #[test]
    fn native_description_is_searchable() {
        let catalog = vec![external_described(
            "vst3:z",
            "Zebra9000",
            "Acme",
            "A guqin-like plucked zither with a long decay",
        )];
        let list = suggestions("guqin-like", None, 5, &catalog);
        assert_eq!(list[0].plugin_id, "vst3:z");
        assert_eq!(list[0].source, "installed");
        assert!(list[0].why.contains("自述"), "{}", list[0].why);

        // 中文查询同样能命中（规则别名把「古琴」也加进查询词）。
        assert_eq!(
            resolve_instrument("古琴", &catalog).unwrap().plugin_id,
            "vst3:z"
        );
    }

    #[test]
    fn installed_pipa_beats_the_plucked_approximation() {
        let catalog = vec![external("vst3:pipa", "Pipa", "Ethno Labs")];
        let list = suggestions("pipa-like", None, 5, &catalog);
        assert_eq!(list[0].plugin_id, "vst3:pipa");
        assert_eq!(list[0].source, "installed");
    }

    #[test]
    fn empty_query_lists_the_catalog() {
        let catalog = vec![external("vst3:a", "Alpha", "Acme")];
        let list = suggestions("", Some(PluginKind::Instrument), 50, &catalog);
        assert!(list.len() > 5, "空查询按目录顺序给前一页");
        assert!(list.iter().any(|item| item.plugin_id == "vst3:a"));
        assert!(list.iter().all(|item| item.confidence == "low"));
    }

    #[test]
    fn stem_names_are_safe_and_unique() {
        assert_eq!(safe_file_name("古琴 / Lead", "stem"), "古琴 _ Lead");
        assert_eq!(safe_file_name("...", "stem"), "stem");
        let mut used = HashSet::new();
        let first = unique_stem_path(Path::new("/tmp"), "古琴", &mut used);
        let second = unique_stem_path(Path::new("/tmp"), "古琴", &mut used);
        assert_eq!(first.file_name().unwrap(), "古琴.wav");
        assert_eq!(second.file_name().unwrap(), "古琴-2.wav");
    }
}
