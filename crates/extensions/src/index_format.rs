//! 市场索引格式（`format: 1`）。
//!
//! 字段与校验规则照抄 DeepSeek Harness 的 `plugin-market-index`（见
//! `.agents/notes/implemented/feature/2026-10-02-plugin-market-index-format.md`），只把
//! `engines.dsh` 换成 `engines.aidaw`。读取器的纪律也照抄：
//!
//! - **整份拒绝**，不静默丢条目（格式错误是生成器的 bug，丢了就看不见）；
//! - 未知成员忽略（向前兼容）；
//! - 每个错误都带 JSON 路径。

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{ExtError, ExtResult};
use crate::manifest::Localized;

/// 支持的索引格式版本。
pub const FORMAT: u32 = 1;
/// 图标内联上限（解码后的字节数）。
pub const MAX_INLINE_ICON_BYTES: usize = 32 * 1024;
/// 版本收件时间戳上界（Unix 秒）。
pub const MAX_TIMESTAMP: i64 = 100_000_000_000;

/// 根上的一个声明（分类 / 标签）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Declaration {
    /// 标题。
    pub title: Localized,
}

/// 排序键声明。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortKeyDeclaration {
    /// 标题。
    pub title: Localized,
    /// 初始方向（`asc` / `desc`）。
    pub order: String,
}

/// 索引根上的元数据声明。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Metadata {
    /// 分类。
    #[serde(default)]
    pub categories: BTreeMap<String, Declaration>,
    /// 标签。
    #[serde(default)]
    pub labels: BTreeMap<String, Declaration>,
    /// 排序键。
    #[serde(default, rename = "sortKeys")]
    pub sort_keys: BTreeMap<String, SortKeyDeclaration>,
}

/// 插件条目里的市场自定义元数据。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginMetadata {
    /// 分类 id。
    #[serde(default)]
    pub category: Option<String>,
    /// 标签 id。
    #[serde(default)]
    pub labels: Vec<String>,
    /// 关键词。
    #[serde(default)]
    pub keywords: Vec<String>,
    /// 排序键取值。
    #[serde(default, rename = "sortKeys")]
    pub sort_keys: BTreeMap<String, f64>,
}

/// 仓库展示信息（**只用于展示**，永不作为安装源）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    /// 仓库网页地址。
    pub url: String,
    /// 包在仓库里的子目录。
    #[serde(default)]
    pub directory: Option<String>,
}

/// 一个版本的安装源。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Source {
    /// npm registry。
    Npm {
        /// 该版本所在的 registry（缺省 = 用户配置的）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        registry: Option<String>,
    },
    /// Git 仓库（本版本只读不装）。
    Git {
        /// 仓库地址。
        url: String,
        /// 提交 SHA。
        commit: String,
        /// 包所在子目录。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
}

/// 一个版本的兼容性声明。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VersionEngines {
    /// 对 AI DAW 版本的要求。
    pub aidaw: String,
}

/// 索引里的一个版本。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionEntry {
    /// 精确版本。
    pub version: String,
    /// 发布时间（Unix 秒）。
    #[serde(rename = "publishTimestamp")]
    pub publish_timestamp: i64,
    /// 安装源。
    pub source: Source,
    /// 兼容性。
    pub engines: VersionEngines,
}

/// 索引里的一个插件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginEntry {
    /// npm 包名（索引内唯一）。
    pub name: String,
    /// 标题。
    #[serde(default)]
    pub title: Option<Localized>,
    /// 说明。
    #[serde(default)]
    pub description: Option<Localized>,
    /// 图标（data: 或绝对 URL）。
    #[serde(default)]
    pub icon: Option<String>,
    /// 作者。
    #[serde(default)]
    pub author: Option<String>,
    /// 主页。
    #[serde(default)]
    pub homepage: Option<String>,
    /// 仓库。
    #[serde(default)]
    pub repository: Option<Repository>,
    /// 市场元数据。
    #[serde(default)]
    pub metadata: PluginMetadata,
    /// 分发标签（`latest` / `next` …）。
    #[serde(default, rename = "dist-tags")]
    pub dist_tags: BTreeMap<String, String>,
    /// 版本列表（顺序不代表安装版本）。
    pub versions: Vec<VersionEntry>,
}

impl PluginEntry {
    /// 展示标题。
    pub fn display_title(&self, lang: &str) -> String {
        self.title
            .as_ref()
            .map(|title| title.resolve(lang))
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| self.name.clone())
    }

    /// 某条 version 记录。
    pub fn version(&self, version: &str) -> Option<&VersionEntry> {
        self.versions.iter().find(|item| item.version == version)
    }

    /// 最大发布时间（按发布时间排序用）。
    pub fn latest_publish(&self) -> i64 {
        self.versions.iter().map(|item| item.publish_timestamp).max().unwrap_or(0)
    }
}

/// 一份索引文档。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDocument {
    /// 格式版本。
    pub format: u32,
    /// 市场标题。
    pub title: Localized,
    /// 市场说明。
    #[serde(default)]
    pub description: Option<Localized>,
    /// 市场声明。
    #[serde(default)]
    pub metadata: Metadata,
    /// 插件列表（顺序即默认排序）。
    pub plugins: Vec<PluginEntry>,
}

/// 解析并校验一份索引。
///
/// `index_url` 用来解析相对图标地址（读的时候绝对化，客户端只看到绝对 URL）。
pub fn parse_index(bytes: &[u8], index_url: &str) -> ExtResult<IndexDocument> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|err| ExtError::InvalidIndex(format!("不是合法 JSON：{err}")))?;
    let document = validate(&value, index_url)?;
    Ok(document)
}

fn fail(path: &str, message: impl Into<String>) -> ExtError {
    ExtError::InvalidIndex(format!("{path}: {}", message.into()))
}

fn validate(value: &Value, index_url: &str) -> ExtResult<IndexDocument> {
    let root = value
        .as_object()
        .ok_or_else(|| fail("$", "根必须是一个对象"))?;

    let format = root.get("format").and_then(Value::as_u64).unwrap_or(0);
    if format != FORMAT as u64 {
        return Err(fail("$.format", format!("必须是 {FORMAT}，实际是 {format}")));
    }
    let title = localized(root.get("title"), "$.title")?;
    let description = match root.get("description") {
        Some(value) => Some(localized(Some(value), "$.description")?),
        None => None,
    };

    let metadata = validate_metadata(root.get("metadata"))?;

    let plugins_value = root
        .get("plugins")
        .and_then(Value::as_array)
        .ok_or_else(|| fail("$.plugins", "必须是数组（可以为空）"))?;

    let mut seen_names: HashSet<String> = HashSet::new();
    let mut plugins = Vec::with_capacity(plugins_value.len());
    for (index, plugin) in plugins_value.iter().enumerate() {
        let path = format!("$.plugins[{index}]");
        let entry = validate_plugin(plugin, &path, index_url, &metadata)?;
        if !seen_names.insert(entry.name.clone()) {
            return Err(fail(&format!("{path}.name"), format!("重复的包名「{}」", entry.name)));
        }
        plugins.push(entry);
    }

    Ok(IndexDocument {
        format: FORMAT,
        title,
        description,
        metadata,
        plugins,
    })
}

fn validate_metadata(value: Option<&Value>) -> ExtResult<Metadata> {
    let Some(value) = value else {
        return Ok(Metadata::default());
    };
    let object = value.as_object().ok_or_else(|| fail("$.metadata", "必须是对象"))?;
    let mut metadata = Metadata::default();
    for (key, target) in [("categories", 0u8), ("labels", 1u8)] {
        let Some(declarations) = object.get(key) else { continue };
        let declarations = declarations
            .as_object()
            .ok_or_else(|| fail(&format!("$.metadata.{key}"), "必须是对象"))?;
        for (id, declaration) in declarations {
            if id.trim().is_empty() {
                return Err(fail(&format!("$.metadata.{key}"), "id 不能是空串"));
            }
            let declaration = declaration
                .as_object()
                .ok_or_else(|| fail(&format!("$.metadata.{key}.{id}"), "必须是对象"))?;
            let entry = Declaration {
                title: localized(declaration.get("title"), &format!("$.metadata.{key}.{id}.title"))?,
            };
            if target == 0 {
                metadata.categories.insert(id.clone(), entry);
            } else {
                metadata.labels.insert(id.clone(), entry);
            }
        }
    }
    if let Some(sort_keys) = object.get("sortKeys") {
        let sort_keys = sort_keys
            .as_object()
            .ok_or_else(|| fail("$.metadata.sortKeys", "必须是对象"))?;
        for (id, declaration) in sort_keys {
            let declaration = declaration
                .as_object()
                .ok_or_else(|| fail(&format!("$.metadata.sortKeys.{id}"), "必须是对象"))?;
            let order = declaration.get("order").and_then(Value::as_str).unwrap_or("asc");
            if order != "asc" && order != "desc" {
                return Err(fail(&format!("$.metadata.sortKeys.{id}.order"), "必须是 asc 或 desc"));
            }
            metadata.sort_keys.insert(
                id.clone(),
                SortKeyDeclaration {
                    title: localized(declaration.get("title"), &format!("$.metadata.sortKeys.{id}.title"))?,
                    order: order.to_string(),
                },
            );
        }
    }
    Ok(metadata)
}

fn validate_plugin(value: &Value, path: &str, index_url: &str, metadata: &Metadata) -> ExtResult<PluginEntry> {
    let object = value.as_object().ok_or_else(|| fail(path, "必须是对象"))?;

    let name = object
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| valid_package_name(name))
        .ok_or_else(|| fail(&format!("{path}.name"), "不是合法的 npm 包名"))?
        .to_string();

    let title = match object.get("title") {
        Some(value) => Some(localized(Some(value), &format!("{path}.title"))?),
        None => None,
    };
    let description = match object.get("description") {
        Some(value) => Some(localized(Some(value), &format!("{path}.description"))?),
        None => None,
    };
    let icon = match object.get("icon") {
        Some(value) => Some(resolve_icon(value, path, index_url)?),
        None => None,
    };
    let author = object.get("author").and_then(Value::as_str).map(str::to_string);
    let homepage = match object.get("homepage").and_then(Value::as_str) {
        Some(url) if url.starts_with("https://") || url.starts_with("http://") => Some(url.to_string()),
        Some(_) => return Err(fail(&format!("{path}.homepage"), "必须是 http(s) 绝对地址")),
        None => None,
    };
    let repository = match object.get("repository") {
        Some(value) => Some(parse_repository(value, &format!("{path}.repository"))?),
        None => None,
    };

    let mut plugin_metadata = PluginMetadata::default();
    if let Some(value) = object.get("metadata") {
        let meta = value
            .as_object()
            .ok_or_else(|| fail(&format!("{path}.metadata"), "必须是对象"))?;
        if let Some(category) = meta.get("category") {
            let category = category
                .as_str()
                .ok_or_else(|| fail(&format!("{path}.metadata.category"), "必须是字符串"))?;
            if !metadata.categories.contains_key(category) {
                return Err(fail(&format!("{path}.metadata.category"), format!("没有声明分类「{category}」")));
            }
            plugin_metadata.category = Some(category.to_string());
        }
        if let Some(labels) = meta.get("labels") {
            let labels = labels
                .as_array()
                .ok_or_else(|| fail(&format!("{path}.metadata.labels"), "必须是数组"))?;
            for (index, label) in labels.iter().enumerate() {
                let label = label
                    .as_str()
                    .ok_or_else(|| fail(&format!("{path}.metadata.labels[{index}]"), "必须是字符串"))?;
                if !metadata.labels.contains_key(label) {
                    return Err(fail(&format!("{path}.metadata.labels[{index}]"), format!("没有声明标签「{label}」")));
                }
                plugin_metadata.labels.push(label.to_string());
            }
        }
        if let Some(keywords) = meta.get("keywords") {
            let keywords = keywords
                .as_array()
                .ok_or_else(|| fail(&format!("{path}.metadata.keywords"), "必须是数组"))?;
            for (index, keyword) in keywords.iter().enumerate() {
                let keyword = keyword
                    .as_str()
                    .ok_or_else(|| fail(&format!("{path}.metadata.keywords[{index}]"), "必须是字符串"))?;
                plugin_metadata.keywords.push(keyword.to_string());
            }
        }
        if let Some(sort_keys) = meta.get("sortKeys") {
            let sort_keys = sort_keys
                .as_object()
                .ok_or_else(|| fail(&format!("{path}.metadata.sortKeys"), "必须是对象"))?;
            for (key, number) in sort_keys {
                if !metadata.sort_keys.contains_key(key) {
                    return Err(fail(&format!("{path}.metadata.sortKeys.{key}"), format!("没有声明排序键「{key}」")));
                }
                let number = number
                    .as_f64()
                    .ok_or_else(|| fail(&format!("{path}.metadata.sortKeys.{key}"), "必须是数字"))?;
                plugin_metadata.sort_keys.insert(key.clone(), number);
            }
        }
    }

    // 版本
    let versions_value = object
        .get("versions")
        .and_then(Value::as_array)
        .ok_or_else(|| fail(&format!("{path}.versions"), "必须是非空数组"))?;
    if versions_value.is_empty() {
        return Err(fail(&format!("{path}.versions"), "不能是空数组"));
    }
    let mut seen_versions = HashSet::new();
    let mut versions = Vec::with_capacity(versions_value.len());
    for (index, version) in versions_value.iter().enumerate() {
        let version_path = format!("{path}.versions[{index}]");
        let entry = validate_version(version, &version_path)?;
        if !seen_versions.insert(entry.version.clone()) {
            return Err(fail(&format!("{version_path}.version"), format!("重复的版本「{}」", entry.version)));
        }
        versions.push(entry);
    }

    // 分发标签必须指向已列出的精确版本。
    let mut dist_tags = BTreeMap::new();
    if let Some(tags) = object.get("dist-tags") {
        let tags = tags
            .as_object()
            .ok_or_else(|| fail(&format!("{path}.dist-tags"), "必须是对象"))?;
        for (tag, target) in tags {
            let target = target
                .as_str()
                .ok_or_else(|| fail(&format!("{path}.dist-tags.{tag}"), "必须是版本字符串"))?;
            if !seen_versions.contains(target) {
                return Err(fail(
                    &format!("{path}.dist-tags.{tag}"),
                    format!("指向没有列出的版本「{target}」"),
                ));
            }
            dist_tags.insert(tag.clone(), target.to_string());
        }
    }

    Ok(PluginEntry {
        name,
        title,
        description,
        icon,
        author,
        homepage,
        repository,
        metadata: plugin_metadata,
        dist_tags,
        versions,
    })
}

fn validate_version(value: &Value, path: &str) -> ExtResult<VersionEntry> {
    let object = value.as_object().ok_or_else(|| fail(path, "必须是对象"))?;

    let version = object
        .get("version")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|version| semver::Version::parse(version).is_ok())
        .ok_or_else(|| fail(&format!("{path}.version"), "必须是精确的 semver 版本"))?
        .to_string();

    let publish_timestamp = object
        .get("publishTimestamp")
        .and_then(Value::as_i64)
        .filter(|value| (0..MAX_TIMESTAMP).contains(value))
        .ok_or_else(|| fail(&format!("{path}.publishTimestamp"), "必须是 [0, 1e11) 的整数（Unix 秒）"))?;

    let source_value = object
        .get("source")
        .and_then(Value::as_object)
        .ok_or_else(|| fail(&format!("{path}.source"), "必须是对象"))?;
    let source_type = source_value.get("type").and_then(Value::as_str).unwrap_or_default();
    let source = match source_type {
        "npm" => {
            let registry = match source_value.get("registry") {
                Some(value) => {
                    let url = value
                        .as_str()
                        .ok_or_else(|| fail(&format!("{path}.source.registry"), "必须是字符串"))?;
                    if !valid_registry(url) {
                        return Err(fail(&format!("{path}.source.registry"), "必须是 http(s) 地址且不带凭据/查询"));
                    }
                    Some(url.trim_end_matches('/').to_string())
                }
                None => None,
            };
            Source::Npm { registry }
        }
        "git" => {
            return Err(fail(
                &format!("{path}.source.type"),
                "本版本的 AI DAW 只支持 npm 源（索引里出现了 git 源）",
            ));
        }
        other => {
            return Err(fail(&format!("{path}.source.type"), format!("不认识的安装源「{other}」")));
        }
    };

    let engines = object
        .get("engines")
        .and_then(Value::as_object)
        .ok_or_else(|| fail(&format!("{path}.engines"), "必须声明 engines.aidaw"))?;
    let aidaw = engines
        .get("aidaw")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|range| !range.is_empty())
        .ok_or_else(|| fail(&format!("{path}.engines.aidaw"), "必须是非空 semver range"))?;
    if let Err(err) = crate::range::validate(aidaw) {
        return Err(fail(&format!("{path}.engines.aidaw"), format!("「{aidaw}」不可用：{err}")));
    }

    Ok(VersionEntry {
        version,
        publish_timestamp,
        source,
        engines: VersionEngines {
            aidaw: aidaw.to_string(),
        },
    })
}

fn parse_repository(value: &Value, path: &str) -> ExtResult<Repository> {
    let object = value.as_object().ok_or_else(|| fail(path, "必须是对象"))?;
    let url = object
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
        .ok_or_else(|| fail(&format!("{path}.url"), "必须是 http(s) 网页地址"))?;
    let directory = object
        .get("directory")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(Repository {
        url: url.to_string(),
        directory,
    })
}

fn resolve_icon(value: &Value, path: &str, index_url: &str) -> ExtResult<String> {
    let text = value
        .as_str()
        .ok_or_else(|| fail(&format!("{path}.icon"), "必须是字符串"))?
        .trim();

    if let Some(rest) = text.strip_prefix("data:") {
        let (header, payload) = rest
            .split_once(',')
            .ok_or_else(|| fail(&format!("{path}.icon"), "data URL 缺少逗号"))?;
        let media = header.split(';').next().unwrap_or_default();
        if !matches!(media, "image/svg+xml" | "image/png" | "image/jpeg" | "image/webp") {
            return Err(fail(&format!("{path}.icon"), "只支持 SVG / PNG / JPEG / WebP 的 data URL"));
        }
        if !header.split(';').any(|part| part.trim() == "base64") {
            return Err(fail(&format!("{path}.icon"), "data URL 必须是 base64"));
        }
        let payload = payload.trim();
        if payload.is_empty() || !payload.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '+' || ch == '/' || ch == '=') {
            return Err(fail(&format!("{path}.icon"), "base64 负载不合法"));
        }
        let padding = payload.chars().rev().take_while(|ch| *ch == '=').count();
        let decoded = payload.len() / 4 * 3 - padding.min(2);
        if decoded > MAX_INLINE_ICON_BYTES {
            return Err(fail(
                &format!("{path}.icon"),
                format!("内联图标解码后不能超过 {} KiB", MAX_INLINE_ICON_BYTES / 1024),
            ));
        }
        return Ok(text.to_string());
    }

    let base = url::Url::parse(index_url)
        .map_err(|err| fail("$", format!("索引地址不合法：{err}")))?;
    let resolved = base
        .join(text)
        .map_err(|err| fail(&format!("{path}.icon"), format!("无法解析成 URL：{err}")))?;
    let same_origin = resolved.scheme() == base.scheme() && resolved.host_str() == base.host_str() && resolved.port_or_known_default() == base.port_or_known_default();
    if resolved.scheme() != "https" && !same_origin {
        return Err(fail(&format!("{path}.icon"), "必须是 https 或与索引同源"));
    }
    Ok(resolved.to_string())
}

fn localized(value: Option<&Value>, path: &str) -> ExtResult<Localized> {
    let value = value.ok_or_else(|| fail(path, "缺少本地化文本"))?;
    if let Some(text) = value.as_str() {
        if text.trim().is_empty() {
            return Err(fail(path, "文本不能是空白"));
        }
        return Ok(Localized::Plain(text.to_string()));
    }
    let object = value.as_object().ok_or_else(|| fail(path, "必须是字符串或语言映射"))?;
    let mut map = BTreeMap::new();
    for (language, text) in object {
        let text = text
            .as_str()
            .ok_or_else(|| fail(&format!("{path}.{language}"), "必须是字符串"))?;
        if text.trim().is_empty() {
            continue;
        }
        map.insert(language.to_lowercase(), text.to_string());
    }
    if map.is_empty() {
        return Err(fail(path, "没有任何非空文本"));
    }
    Ok(Localized::Map(map))
}

fn valid_package_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 214 {
        return false;
    }
    let rest = match name.strip_prefix('@') {
        Some(rest) => match rest.split_once('/') {
            Some((scope, package)) => {
                if scope.is_empty() {
                    return false;
                }
                package
            }
            None => return false,
        },
        None => name,
    };
    !rest.is_empty()
        && rest.chars().all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_' | '.' | '~')
        })
}

fn valid_registry(url: &str) -> bool {
    let url = url.trim();
    (url.starts_with("https://") || url.starts_with("http://"))
        && !url.contains('@')
        && !url.contains('?')
        && !url.contains('#')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(plugins: &str) -> String {
        format!(
            r#"{{"format":1,"title":{{"en":"Test"}},"plugins":[{plugins}]}}"#
        )
    }

    fn plugin(extra: &str) -> String {
        format!(
            r#"{{"name":"@aidaw/x","versions":[{{"version":"0.1.0","publishTimestamp":1700000000,"source":{{"type":"npm"}},"engines":{{"aidaw":">=1.0.0"}}}}{extra}]}}"#
        )
    }

    #[test]
    fn parses_a_minimal_index() {
        let document = parse_index(base(&plugin("")).as_bytes(), "https://example.com/index.json").unwrap();
        assert_eq!(document.format, 1);
        assert_eq!(document.plugins.len(), 1);
        assert_eq!(document.plugins[0].name, "@aidaw/x");
        assert_eq!(document.plugins[0].versions[0].engines.aidaw, ">=1.0.0");
    }

    #[test]
    fn empty_plugin_list_is_valid() {
        let document = parse_index(r#"{"format":1,"title":"空市场","plugins":[]}"#.as_bytes(), "https://e.com/i.json").unwrap();
        assert!(document.plugins.is_empty());
        assert_eq!(document.title.resolve("zh"), "空市场");
    }

    #[test]
    fn rejects_bad_format_and_missing_title() {
        assert!(parse_index(r#"{"format":2,"title":"x","plugins":[]}"#.as_bytes(), "https://e.com/i.json").is_err());
        assert!(parse_index(r#"{"format":1,"plugins":[]}"#.as_bytes(), "https://e.com/i.json").is_err());
    }

    #[test]
    fn rejects_duplicate_names_and_versions() {
        let two = format!("{},{}", plugin(""), plugin(""));
        assert!(parse_index(base(&two).as_bytes(), "https://e.com/i.json").is_err());

        let duplicate = r#"{"name":"@aidaw/x","versions":[
            {"version":"0.1.0","publishTimestamp":1,"source":{"type":"npm"},"engines":{"aidaw":"*"}},
            {"version":"0.1.0","publishTimestamp":2,"source":{"type":"npm"},"engines":{"aidaw":"*"}}]}"#;
        assert!(parse_index(base(duplicate).as_bytes(), "https://e.com/i.json").is_err());
    }

    #[test]
    fn rejects_missing_or_bad_engine_and_timestamp() {
        let no_engine = r#"{"name":"@aidaw/x","versions":[{"version":"0.1.0","publishTimestamp":1,"source":{"type":"npm"},"engines":{}}]}"#;
        assert!(parse_index(base(no_engine).as_bytes(), "https://e.com/i.json").is_err());

        let bad_range = r#"{"name":"@aidaw/x","versions":[{"version":"0.1.0","publishTimestamp":1,"source":{"type":"npm"},"engines":{"aidaw":"nope"}}]}"#;
        assert!(parse_index(base(bad_range).as_bytes(), "https://e.com/i.json").is_err());

        let bad_time = r#"{"name":"@aidaw/x","versions":[{"version":"0.1.0","publishTimestamp":-1,"source":{"type":"npm"},"engines":{"aidaw":"*"}}]}"#;
        assert!(parse_index(base(bad_time).as_bytes(), "https://e.com/i.json").is_err());
    }

    #[test]
    fn dist_tags_must_point_at_listed_versions() {
        let document = format!(
            r#"{{"name":"@aidaw/x","dist-tags":{{"latest":"0.1.0"}},"versions":[{{"version":"0.1.0","publishTimestamp":1,"source":{{"type":"npm"}},"engines":{{"aidaw":"*"}}}}]}}"#
        );
        assert!(parse_index(base(&document).as_bytes(), "https://e.com/i.json").is_ok());

        let dangling = document.replace("0.1.0\"}", "9.9.9\"}");
        let err = parse_index(base(&dangling).as_bytes(), "https://e.com/i.json").unwrap_err();
        assert!(err.to_string().contains("dist-tags"), "{err}");
    }

    #[test]
    fn git_sources_are_refused_with_a_clear_message() {
        let git = r#"{"name":"@aidaw/x","versions":[{"version":"0.1.0","publishTimestamp":1,"source":{"type":"git","url":"https://github.com/a/b","commit":"0123456789012345678901234567890123456789"},"engines":{"aidaw":"*"}}]}"#;
        let err = parse_index(base(git).as_bytes(), "https://e.com/i.json").unwrap_err();
        assert!(err.to_string().contains("npm"), "{err}");
    }

    #[test]
    fn icons_resolve_relative_to_the_index_and_reject_http_cross_origin() {
        let relative = plugin("").replace("\"name\":\"@aidaw/x\"", "\"name\":\"@aidaw/x\",\"icon\":\"icons/x.svg\"");
        let document = parse_index(base(&relative).as_bytes(), "https://example.com/market/index.json").unwrap();
        assert_eq!(document.plugins[0].icon.as_deref(), Some("https://example.com/market/icons/x.svg"));

        let foreign = plugin("").replace("\"name\":\"@aidaw/x\"", "\"name\":\"@aidaw/x\",\"icon\":\"http://evil.example/x.svg\"");
        assert!(parse_index(base(&foreign).as_bytes(), "https://example.com/index.json").is_err());

        let inline = plugin("").replace(
            "\"name\":\"@aidaw/x\"",
            "\"name\":\"@aidaw/x\",\"icon\":\"data:image/svg+xml;base64,PHN2Zy8+\"",
        );
        assert!(parse_index(base(&inline).as_bytes(), "https://example.com/index.json").is_ok());
    }

    #[test]
    fn undeclared_taxonomy_is_rejected() {
        let bad = plugin("").replace(
            "\"name\":\"@aidaw/x\"",
            "\"name\":\"@aidaw/x\",\"metadata\":{\"category\":\"nope\"}",
        );
        assert!(parse_index(base(&bad).as_bytes(), "https://e.com/i.json").is_err());
    }

    #[test]
    fn unknown_members_are_ignored() {
        let unknown = plugin("").replace(
            "\"name\":\"@aidaw/x\"",
            "\"name\":\"@aidaw/x\",\"future\":42",
        );
        assert!(parse_index(base(&unknown).as_bytes(), "https://e.com/i.json").is_ok());
    }
}
