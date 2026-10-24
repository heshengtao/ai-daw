//! ArtiCluster Universal Definition（YAML）导入。
//!
//! 数据来自 <https://github.com/r-koubou/ArticulationMappingFiles>（MIT），
//! 随包快照放在 `resources/articulation-maps/`。这里只做只读解析与索引；
//! 真正写入工程仍走 `Command::SetArticulations`，因此导入是一个撤销步。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ai_daw_engine::model::{ArticulationEvent, ArticulationSource, ArticulationSwitch, TrackMeta};
use ai_daw_plugin_api::MidiEvent;
use serde::{Deserialize, Serialize};

/// 单份映射最多接受多少条奏法（防止脏数据把工程撑爆）。
pub const MAX_ARTICULATIONS: usize = 256;
/// 单条奏法最多接受多少条 MIDI 消息。
pub const MAX_MESSAGES: usize = ArticulationSwitch::MAX_EVENTS;

/// 映射库条目：界面搜索 / 选择用。
#[derive(Debug, Clone, Serialize)]
pub struct ArticulationMapSummary {
    /// 相对文件路径（不含扩展名），作为导入时的稳定 id。
    pub id: String,
    pub manufacturer: String,
    pub product: String,
    pub patch: String,
    pub description: String,
    /// 实际可发送的奏法数量（不含空消息的 Idle）。
    pub articulation_count: usize,
}

/// 一份解析后的映射。
#[derive(Debug, Clone)]
pub struct ParsedArticulationMap {
    pub summary: ArticulationMapSummary,
    pub source: ArticulationSource,
    pub articulations: BTreeMap<String, ArticulationSwitch>,
    /// 数据里显式声明的默认奏法（目前 Universal Definition 没有标准字段，通常为空）。
    pub default: Option<String>,
    /// 无法转换或已跳过的条目（界面可以提示，但不阻塞导入）。
    pub warnings: Vec<String>,
}

/// 导入计划：把解析结果与轨道现有映射合并后的最终状态。
#[derive(Debug, Clone)]
pub struct ImportPlan {
    pub articulations: BTreeMap<String, ArticulationSwitch>,
    pub default: Option<String>,
    pub source: ArticulationSource,
    pub patch: String,
    /// 这份映射本身包含多少条奏法（不是合并后的总数）。
    pub imported_count: usize,
}

/// 把一份映射应用到轨道现有映射上。
///
/// `merge = false` 整体替换；`merge = true` 保留现有并让导入项覆盖同名项。
/// 默认奏法的优先级：显式请求 > 现有默认（仍存在）> 映射自带的默认。
pub fn plan_import(
    existing: &TrackMeta,
    parsed: ParsedArticulationMap,
    merge: bool,
    requested_default: Option<String>,
) -> ImportPlan {
    let ParsedArticulationMap {
        summary,
        source,
        articulations,
        default: parsed_default,
        ..
    } = parsed;
    let imported_count = articulations.len();
    let mut merged = if merge {
        existing.articulations.clone()
    } else {
        Default::default()
    };
    merged.extend(articulations);

    let default = requested_default
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .filter(|name| merged.contains_key(name))
        .or_else(|| {
            existing
                .default_articulation
                .clone()
                .filter(|name| merged.contains_key(name))
        })
        .or_else(|| parsed_default.filter(|name| merged.contains_key(name)));

    ImportPlan {
        articulations: merged,
        default,
        source,
        patch: summary.patch,
        imported_count,
    }
}

/// 递归扫描的映射库。
#[derive(Debug, Clone)]
pub struct ArticulationMapCatalog {
    root: PathBuf,
}

impl ArticulationMapCatalog {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// 列出所有映射；`query` 对厂商 / 产品 / patch / 路径做大小写不敏感匹配。
    pub fn list(&self, query: Option<&str>, limit: usize) -> Vec<ArticulationMapSummary> {
        let needle = query
            .map(|query| query.trim().to_lowercase())
            .filter(|query| !query.is_empty());
        let mut out = Vec::new();
        self.walk(&self.root, &mut out);
        if let Some(needle) = needle {
            out.retain(|item| {
                let haystack = format!(
                    "{} {} {} {}",
                    item.manufacturer, item.product, item.patch, item.id
                )
                .to_lowercase();
                haystack.contains(&needle)
            });
        }
        out.sort_by(|a, b| {
            (
                a.manufacturer.to_lowercase(),
                a.product.to_lowercase(),
                a.patch.to_lowercase(),
            )
                .cmp(&(
                    b.manufacturer.to_lowercase(),
                    b.product.to_lowercase(),
                    b.patch.to_lowercase(),
                ))
        });
        out.truncate(limit.clamp(1, 2000));
        out
    }

    /// 按 id 读取并解析一份映射。
    pub fn load(&self, id: &str) -> Result<ParsedArticulationMap, String> {
        let path = self.resolve(id)?;
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("读取映射失败（{}）：{error}", path.display()))?;
        parse_definition(&text, id)
    }

    /// 找到 id 对应的文件，并确保它仍在映射库目录内。
    fn resolve(&self, id: &str) -> Result<PathBuf, String> {
        let trimmed = id.trim();
        if trimmed.is_empty()
            || trimmed.contains("..")
            || trimmed.starts_with('/')
            || trimmed.starts_with('\\')
            || trimmed.contains('\\')
        {
            return Err(format!("非法的映射 id：{id}"));
        }
        let candidate = self.root.join(trimmed);
        let path = if matches!(
            candidate.extension().and_then(|ext| ext.to_str()),
            Some("yaml" | "yml")
        ) {
            candidate
        } else {
            let yaml = candidate.with_extension("yaml");
            if yaml.is_file() {
                yaml
            } else {
                candidate.with_extension("yml")
            }
        };
        if !path.is_file() {
            return Err(format!("找不到映射：{id}"));
        }
        let root =
            std::fs::canonicalize(&self.root).map_err(|error| format!("映射库不可用：{error}"))?;
        let resolved =
            std::fs::canonicalize(&path).map_err(|error| format!("映射不可用：{error}"))?;
        if !resolved.starts_with(&root) {
            return Err(format!("映射路径越界：{id}"));
        }
        Ok(resolved)
    }

    fn walk(&self, dir: &Path, out: &mut Vec<ArticulationMapSummary>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                self.walk(&path, out);
                continue;
            }
            let is_yaml = matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("yaml" | "yml")
            );
            if !is_yaml {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let id = id_from_path(&self.root, &path);
            if let Ok(parsed) = parse_definition(&text, &id) {
                out.push(parsed.summary);
            }
        }
    }
}

fn id_from_path(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
        .trim_end_matches(".yaml")
        .trim_end_matches(".yml")
        .to_string()
}

#[derive(Debug, Deserialize)]
struct RawDefinition {
    #[serde(rename = "FormatVersion", default)]
    _format_version: u32,
    #[serde(rename = "Id", default)]
    id: String,
    #[serde(rename = "ManufacturerName", default)]
    manufacturer: String,
    #[serde(rename = "ProductName", default)]
    product: String,
    #[serde(rename = "PatchName", default)]
    patch: String,
    #[serde(rename = "Description", default)]
    description: String,
    #[serde(rename = "ArticulationGroups", default)]
    groups: Vec<RawGroup>,
}

#[derive(Debug, Deserialize)]
struct RawGroup {
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "Articulations", default)]
    articulations: Vec<RawArticulation>,
}

#[derive(Debug, Deserialize)]
struct RawArticulation {
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "MidiMessages", default)]
    messages: Vec<RawMessage>,
    #[serde(rename = "Extra", default)]
    extra: Option<serde_yaml::Value>,
}

#[derive(Debug, Deserialize)]
struct RawMessage {
    #[serde(rename = "Status")]
    status: u8,
    #[serde(rename = "Channel", default)]
    channel: Option<i16>,
    #[serde(rename = "Data1", default)]
    data1: Option<u8>,
    #[serde(rename = "Data2", default)]
    data2: Option<u8>,
}

/// 解析一份 Universal Definition YAML。`fallback_id` 是资源里的相对路径。
pub fn parse_definition(text: &str, fallback_id: &str) -> Result<ParsedArticulationMap, String> {
    let raw: RawDefinition =
        serde_yaml::from_str(text).map_err(|error| format!("YAML 解析失败：{error}"))?;

    let fallback_patch = fallback_id
        .rsplit('/')
        .next()
        .filter(|part| !part.is_empty());
    let manufacturer = raw.manufacturer.trim().to_string();
    let product = raw.product.trim().to_string();
    let patch = if raw.patch.trim().is_empty() {
        fallback_patch.unwrap_or(fallback_id).to_string()
    } else {
        raw.patch.trim().to_string()
    };
    let summary = ArticulationMapSummary {
        id: fallback_id.to_string(),
        manufacturer,
        product,
        patch,
        description: raw.description.trim().to_string(),
        articulation_count: 0,
    };
    let source = ArticulationSource {
        manufacturer: Some(summary.manufacturer.clone()),
        product: Some(summary.product.clone()),
        patch: Some(summary.patch.clone()),
        map_id: Some(if raw.id.trim().is_empty() {
            fallback_id.to_string()
        } else {
            raw.id.trim().to_string()
        }),
    }
    .normalized()
    .unwrap_or_default();
    let mut parsed = ParsedArticulationMap {
        summary,
        source,
        articulations: BTreeMap::new(),
        default: None,
        warnings: Vec::new(),
    };

    let prefix_groups = raw.groups.len() > 1;
    'groups: for group in &raw.groups {
        let group_name = group.name.trim();
        for articulation in &group.articulations {
            let name = articulation.name.trim();
            if name.is_empty() {
                continue;
            }
            let display_name = if prefix_groups && !group_name.is_empty() {
                format!("{group_name} / {name}")
            } else {
                name.to_string()
            };
            if parsed.articulations.len() >= MAX_ARTICULATIONS {
                parsed.warnings.push(format!(
                    "奏法数量超过上限 {MAX_ARTICULATIONS}，其余条目已忽略"
                ));
                break 'groups;
            }
            if parsed.articulations.contains_key(&display_name) {
                parsed
                    .warnings
                    .push(format!("奏法重名，已跳过：{display_name}"));
                continue;
            }

            let mut events = Vec::new();
            let mut skipped = 0usize;
            for message in articulation.messages.iter().take(MAX_MESSAGES) {
                match message_to_event(message) {
                    Ok(event) => events.push(event),
                    Err(reason) => {
                        skipped += 1;
                        if parsed.warnings.len() < 32 {
                            parsed.warnings.push(format!("{display_name}：{reason}"));
                        }
                    }
                }
            }
            if articulation.messages.len() > MAX_MESSAGES {
                parsed.warnings.push(format!(
                    "{display_name}：消息超过上限 {MAX_MESSAGES}，已截断"
                ));
            }
            if events.is_empty() {
                // `Idle` 这类空奏法：插件本来就不需要切换，跳过即可。
                if skipped > 0 {
                    parsed.warnings.push(format!(
                        "{display_name}：{skipped} 条不支持的消息，已跳过该奏法"
                    ));
                }
                continue;
            }

            let lead_beats = extra_f32(articulation.extra.as_ref(), &["lead_beats", "LeadBeats"]);
            let momentary = extra_bool(articulation.extra.as_ref(), &["momentary", "Momentary"]);
            parsed.articulations.insert(
                display_name,
                ArticulationSwitch {
                    keyswitch: None,
                    cc: None,
                    program: None,
                    events,
                    lead_beats,
                    momentary,
                }
                .normalized(),
            );
        }
    }

    parsed.summary.articulation_count = parsed.articulations.len();
    Ok(parsed)
}

fn message_to_event(message: &RawMessage) -> Result<ArticulationEvent, String> {
    let data1 = message.data1.unwrap_or(0).min(127);
    let data2 = message.data2.unwrap_or(0).min(127);
    let event = match message.status & 0xF0 {
        0x80 => MidiEvent::NoteOff { note: data1 },
        0x90 => MidiEvent::NoteOn {
            note: data1,
            velocity: data2,
        },
        0xA0 => MidiEvent::PolyPressure {
            note: data1,
            value: data2,
        },
        0xB0 => MidiEvent::ControlChange {
            controller: data1,
            value: data2,
        },
        0xC0 => MidiEvent::ProgramChange { program: data1 },
        0xD0 => MidiEvent::ChannelPressure { value: data1 },
        _ => {
            return Err(format!(
                "不支持的 MIDI 状态字节 {}（只支持 Note/CC/Program/Pressure）",
                message.status
            ))
        }
    };
    let channel = message
        .channel
        .filter(|channel| (0..=15).contains(channel))
        .map(|channel| channel as u8);
    Ok(ArticulationEvent {
        offset_beats: 0.0,
        channel,
        event,
    })
}

fn extra_lookup<'a>(
    extra: Option<&'a serde_yaml::Value>,
    keys: &[&str],
) -> Option<&'a serde_yaml::Value> {
    let mapping = extra?.as_mapping()?;
    for (key, value) in mapping {
        let Some(key) = key.as_str() else {
            continue;
        };
        if keys
            .iter()
            .any(|candidate| key.eq_ignore_ascii_case(candidate))
        {
            return Some(value);
        }
    }
    None
}

fn extra_f32(extra: Option<&serde_yaml::Value>, keys: &[&str]) -> Option<f32> {
    extra_lookup(extra, keys)?
        .as_f64()
        .map(|value| value as f32)
}

fn extra_bool(extra: Option<&serde_yaml::Value>, keys: &[&str]) -> Option<bool> {
    extra_lookup(extra, keys)?.as_bool()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_daw_plugin_api::MidiEvent;

    const SAMPLE: &str = r#"
FormatVersion: 0
Id: test-map
ManufacturerName: Acme
ProductName: Strings
PatchName: Violins
Description: Test map
ArticulationGroups:
  - Name: Main
    Articulations:
      - Name: Idle
        MidiMessages: []
      - Name: Long
        MidiMessages:
          - Status: 144
            Data1: 24
            Data2: 100
      - Name: Mute
        MidiMessages:
          - Status: 176
            Channel: 2
            Data1: 1
            Data2: 0
"#;

    #[test]
    fn parses_notes_and_cc() {
        let parsed = parse_definition(SAMPLE, "Acme/Strings/Violins").unwrap();
        assert_eq!(parsed.summary.manufacturer, "Acme");
        assert_eq!(parsed.summary.product, "Strings");
        assert_eq!(parsed.summary.patch, "Violins");
        assert_eq!(parsed.summary.articulation_count, 2);
        assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);

        let long = parsed.articulations.get("Long").unwrap();
        assert!(matches!(
            long.events[0].event,
            MidiEvent::NoteOn {
                note: 24,
                velocity: 100
            }
        ));
        let mpe = parsed.articulations.get("Mute").unwrap();
        assert_eq!(mpe.events[0].channel, Some(2));
        assert!(matches!(
            mpe.events[0].event,
            MidiEvent::ControlChange {
                controller: 1,
                value: 0
            }
        ));
    }

    #[test]
    fn prefixes_group_names_only_when_needed() {
        let one = parse_definition(SAMPLE, "x").unwrap();
        assert!(one.articulations.contains_key("Long"));

        let multi = r#"
ManufacturerName: Acme
ProductName: Guitar
PatchName: Electric
ArticulationGroups:
  - Name: Main
    Articulations:
      - Name: Long
        MidiMessages: [{Status: 144, Data1: 24}]
  - Name: FX
    Articulations:
      - Name: Long
        MidiMessages: [{Status: 144, Data1: 25}]
"#;
        let parsed = parse_definition(multi, "x").unwrap();
        assert!(parsed.articulations.contains_key("Main / Long"));
        assert!(parsed.articulations.contains_key("FX / Long"));
    }

    #[test]
    fn unsupported_status_is_reported_not_fatal() {
        let yaml = r#"
ManufacturerName: Acme
ProductName: Synth
PatchName: Lead
ArticulationGroups:
  - Name: Main
    Articulations:
      - Name: Weird
        MidiMessages: [{Status: 240, Data1: 1}]
      - Name: Long
        MidiMessages: [{Status: 144, Data1: 60}]
"#;
        let parsed = parse_definition(yaml, "x").unwrap();
        assert_eq!(parsed.summary.articulation_count, 1);
        assert!(parsed.articulations.contains_key("Long"));
        assert!(parsed.warnings.iter().any(|w| w.contains("Weird")));
    }

    fn parsed_with(names: &[&str]) -> ParsedArticulationMap {
        let mut articulations = BTreeMap::new();
        for (index, name) in names.iter().enumerate() {
            articulations.insert(
                (*name).to_string(),
                ArticulationSwitch {
                    events: vec![ArticulationEvent {
                        offset_beats: 0.0,
                        channel: None,
                        event: MidiEvent::NoteOn {
                            note: 24 + index as u8,
                            velocity: 1,
                        },
                    }],
                    ..Default::default()
                },
            );
        }
        ParsedArticulationMap {
            summary: ArticulationMapSummary {
                id: "test".into(),
                manufacturer: "Acme".into(),
                product: "Strings".into(),
                patch: "Violins".into(),
                description: String::new(),
                articulation_count: articulations.len(),
            },
            source: ArticulationSource {
                patch: Some("Violins".into()),
                ..Default::default()
            },
            articulations,
            default: None,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn import_plan_replaces_or_merges_and_keeps_a_valid_default() {
        let mut existing = TrackMeta {
            default_articulation: Some("Long".into()),
            ..Default::default()
        };
        existing.articulations.insert(
            "Long".into(),
            ArticulationSwitch {
                keyswitch: Some(1),
                ..Default::default()
            },
        );
        existing.articulations.insert(
            "Old".into(),
            ArticulationSwitch {
                keyswitch: Some(2),
                ..Default::default()
            },
        );

        let replaced = plan_import(&existing, parsed_with(&["Long", "Short"]), false, None);
        assert_eq!(replaced.imported_count, 2);
        assert_eq!(replaced.articulations.len(), 2);
        assert!(!replaced.articulations.contains_key("Old"));
        assert_eq!(replaced.default.as_deref(), Some("Long"));

        let merged = plan_import(&existing, parsed_with(&["Long", "Short"]), true, None);
        assert_eq!(merged.articulations.len(), 3);
        assert!(merged.articulations.contains_key("Old"));

        let requested = plan_import(
            &existing,
            parsed_with(&["Short"]),
            false,
            Some("Missing".into()),
        );
        assert_eq!(requested.default, None, "不存在的默认奏法必须被丢弃");

        // 清除映射后重新导入：不依赖任何旧状态。
        let fresh = plan_import(&TrackMeta::default(), parsed_with(&["Long"]), false, None);
        assert_eq!(fresh.imported_count, 1);
        assert_eq!(fresh.articulations.len(), 1);
        assert!(fresh.articulations.contains_key("Long"));
        assert_eq!(fresh.default, None);
    }

    #[test]
    fn catalog_lists_and_loads_snapshot() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/articulation-maps");
        let catalog = ArticulationMapCatalog::new(root);
        let everything = catalog.list(None, 2000);
        assert!(
            everything.len() >= 300,
            "随包快照应当解析出 300+ 份映射，实际 {}",
            everything.len()
        );
        let all = catalog.list(Some("BBC Symphony"), 20);
        assert!(!all.is_empty(), "随包快照里应当有 BBC Symphony 映射");
        let first = all.first().unwrap();
        let parsed = catalog.load(&first.id).unwrap();
        assert!(!parsed.articulations.is_empty());
        assert_eq!(parsed.summary.manufacturer, "Spitfire");
    }
}
