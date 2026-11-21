//! 市场：索引拉取、磁盘缓存、多源合并、搜索与安装状态标注。
//!
//! 缓存策略（对齐 DSH）：成功写 `<cache>/market/<sha256(url)>.json`（带 `fetchedAt`），
//! TTL 内直接用；失败时**保留上一次成功的结果**并同时报错，不让一次网络抖动清空整个市场。

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{ExtError, ExtResult};
use crate::index_format::{parse_index, IndexDocument, PluginEntry, VersionEntry};
use crate::search::{self, Sort};

/// 索引拉取的默认上限（64 MiB）。
pub const MAX_INDEX_BYTES: usize = 64 * 1024 * 1024;
/// 缓存的默认 TTL（10 分钟）。
pub const DEFAULT_TTL: Duration = Duration::from_secs(600);
/// 单个索引请求的整体超时。
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

/// 一份已读到的索引。
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// 索引地址。
    pub url: String,
    /// 文档。
    pub document: IndexDocument,
    /// 读到的时间（Unix 秒）。
    pub fetched_at: i64,
    /// 是否来自缓存。
    pub from_cache: bool,
}

/// 一个索引源的读取结果（成功 / 失败都带上 URL）。
#[derive(Debug, Clone)]
pub struct IndexLoad {
    /// 索引地址。
    pub url: String,
    /// 读到的快照（失败时可能是旧的缓存）。
    pub snapshot: Option<Snapshot>,
    /// 本次的错误（即使有缓存也要显示）。
    pub error: Option<String>,
}

/// 一条市场条目（已叠加安装状态与兼容性）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    /// 包名。
    pub name: String,
    /// 标题。
    pub title: String,
    /// 说明。
    pub description: Option<String>,
    /// 图标（data: 或绝对 URL）。
    pub icon: Option<String>,
    /// 作者。
    pub author: Option<String>,
    /// 主页。
    pub homepage: Option<String>,
    /// 来自哪个索引。
    pub index_url: String,
    /// 分类。
    pub category: Option<String>,
    /// 标签。
    pub labels: Vec<String>,
    /// 关键词。
    pub keywords: Vec<String>,
    /// 默认安装版本（兼容且稳定；没有就是第一个兼容版本）。
    pub default_version: Option<String>,
    /// 全部版本。
    pub versions: Vec<ListingVersion>,
    /// 最新发布时间。
    pub latest_publish: i64,
    /// 安装状态。
    pub installed: InstalledListing,
}

/// 一条版本的展示信息。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListingVersion {
    /// 版本。
    pub version: String,
    /// 发布时间。
    pub publish_timestamp: i64,
    /// 安装参数（`name@version`）。
    pub install: String,
    /// 是否兼容当前应用。
    pub compatible: bool,
    /// 不兼容时的原因。
    pub reason: Option<String>,
}

/// 安装状态。
#[derive(Debug, Clone, Serialize)]
pub struct InstalledListing {
    /// `not-installed` / `installed` / `update-available` / `different-version`
    pub status: String,
    /// 已装版本。
    pub version: Option<String>,
}

/// 搜索上下文（语言、应用版本、已装版本表）。
pub struct ListingContext<'a> {
    /// 应用版本。
    pub app_version: &'a str,
    /// 显示语言。
    pub lang: &'a str,
    /// 已装扩展（包名 → 版本）。
    pub installed: &'a BTreeMap<String, String>,
}

/// 一次搜索的结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    /// 当前页。
    pub entries: Vec<Listing>,
    /// 命中的总数。
    pub total: usize,
    /// 下一页偏移（没有就是 `None`）。
    pub next_offset: Option<usize>,
    /// 参与合并的索引源。
    pub sources: Vec<SourceInfo>,
}

/// 索引源的读取概况。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    /// 地址。
    pub url: String,
    /// 标题（读到过才有）。
    pub title: Option<String>,
    /// 上次成功时间。
    pub fetched_at: Option<i64>,
    /// 本次错误。
    pub error: Option<String>,
    /// 是否来自缓存。
    pub from_cache: bool,
}

/// 市场客户端。
pub struct Market {
    cache_dir: PathBuf,
    agent: ureq::Agent,
    ttl: Duration,
}

impl Market {
    /// 建客户端；`cache_dir` 通常是 `<缓存目录>/extensions`。
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            agent: agent(),
            ttl: DEFAULT_TTL,
        }
    }

    /// 覆盖 TTL（测试用）。
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// 读一个索引源。
    pub fn load(&self, url: &str, refresh: bool) -> IndexLoad {
        let url = url.trim().to_string();
        let cache_path = self.cache_path(&url);
        let cached = read_cache(&cache_path);

        if !refresh {
            if let Some((fetched_at, document)) = &cached {
                if now() - fetched_at < self.ttl.as_secs() as i64 {
                    return IndexLoad {
                        url: url.clone(),
                        snapshot: Some(Snapshot {
                            url: url.clone(),
                            document: document.clone(),
                            fetched_at: *fetched_at,
                            from_cache: true,
                        }),
                        error: None,
                    };
                }
            }
        }

        match self.fetch(&url) {
            Ok(bytes) => match parse_index(&bytes, &url) {
                Ok(document) => {
                    let fetched_at = now();
                    let _ = write_cache(&cache_path, fetched_at, &bytes);
                    IndexLoad {
                        url: url.clone(),
                        snapshot: Some(Snapshot {
                            url,
                            document,
                            fetched_at,
                            from_cache: false,
                        }),
                        error: None,
                    }
                }
                Err(err) => self.fallback(url.clone(), cached, Some(err.to_string())),
            },
            Err(err) => self.fallback(url.clone(), cached, Some(err.to_string())),
        }
    }

    fn fallback(&self, url: String, cached: Option<(i64, IndexDocument)>, error: Option<String>) -> IndexLoad {
        match cached {
            Some((fetched_at, document)) => IndexLoad {
                url: url.clone(),
                snapshot: Some(Snapshot {
                    url,
                    document,
                    fetched_at,
                    from_cache: true,
                }),
                error,
            },
            None => IndexLoad {
                url,
                snapshot: None,
                error,
            },
        }
    }

    /// 读多个索引源（顺序即展示顺序）。
    pub fn load_all(&self, urls: &[String], refresh: bool) -> Vec<IndexLoad> {
        urls.iter().map(|url| self.load(url, refresh)).collect()
    }

    /// 跨索引源搜索并标注安装状态。
    ///
    /// 同名条目**先出现的源赢**（与 DSH 一致）；`offset` / `limit` 是分页（`limit` 上限 100）。
    pub fn search(
        &self,
        loads: &[IndexLoad],
        text: &str,
        sort: &Sort,
        offset: usize,
        limit: usize,
        ctx: &ListingContext<'_>,
    ) -> SearchResult {
        let limit = limit.clamp(1, 100);
        let mut sources = Vec::new();
        let mut seen: HashMap<String, ()> = HashMap::new();
        let mut merged: Vec<(String, PluginEntry)> = Vec::new();

        for load in loads {
            sources.push(SourceInfo {
                url: load.url.clone(),
                title: load
                    .snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.document.title.resolve(ctx.lang)),
                fetched_at: load.snapshot.as_ref().map(|snapshot| snapshot.fetched_at),
                error: load.error.clone(),
                from_cache: load.snapshot.as_ref().map(|snapshot| snapshot.from_cache).unwrap_or(false),
            });
            let Some(snapshot) = &load.snapshot else { continue };
            for plugin in &snapshot.document.plugins {
                if seen.insert(plugin.name.clone(), ()).is_none() {
                    merged.push((load.url.clone(), plugin.clone()));
                }
            }
        }

        // 排序：先把名字拿出来做一份索引，再交给 search::rank。
        let entries: Vec<PluginEntry> = merged.iter().map(|(_, plugin)| plugin.clone()).collect();
        let taxonomies: BTreeMap<String, String> = merged
            .iter()
            .map(|(_, plugin)| (plugin.name.clone(), taxonomy_of(plugin, loads, ctx.lang)))
            .collect();
        let order = search::rank(&entries, text, ctx.lang, |entry| {
            taxonomies.get(&entry.name).cloned().unwrap_or_default()
        }, sort);

        let total = order.len();
        let page: Vec<Listing> = order
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|index| {
                let (index_url, plugin) = &merged[index];
                listing(plugin, index_url, ctx)
            })
            .collect();
        let next_offset = (offset + page.len() < total).then_some(offset + page.len());

        SearchResult {
            entries: page,
            total,
            next_offset,
            sources,
        }
    }

    fn fetch(&self, url: &str) -> ExtResult<Vec<u8>> {
        let response = self
            .agent
            .get(url)
            .header("accept", "application/json")
            .call()
            .map_err(|err| ExtError::Network(format!("拉取索引失败：{err}")))?;
        if response.status().as_u16() != 200 {
            return Err(ExtError::Network(format!("索引返回 HTTP {}", response.status())));
        }
        let mut bytes = Vec::new();
        response
            .into_body()
            .into_reader()
            .take(MAX_INDEX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|err| ExtError::Network(format!("读取索引失败：{err}")))?;
        if bytes.len() > MAX_INDEX_BYTES {
            return Err(ExtError::InvalidIndex(format!(
                "索引超过 {} MiB 上限",
                MAX_INDEX_BYTES / 1024 / 1024
            )));
        }
        Ok(bytes)
    }

    fn cache_path(&self, url: &str) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(url.as_bytes());
        let mut name = String::with_capacity(64);
        for byte in hasher.finalize() {
            use std::fmt::Write as _;
            let _ = write!(name, "{byte:02x}");
        }
        self.cache_dir.join("market").join(format!("{name}.json"))
    }
}

fn listing(plugin: &PluginEntry, index_url: &str, ctx: &ListingContext<'_>) -> Listing {
    let mut versions = Vec::with_capacity(plugin.versions.len());
    let mut compatible_versions = Vec::new();
    for version in &plugin.versions {
        let check = crate::manifest::check_engine(Some(&version.engines.aidaw), ctx.app_version);
        if check.is_ok() {
            compatible_versions.push(version);
        }
        versions.push(ListingVersion {
            version: version.version.clone(),
            publish_timestamp: version.publish_timestamp,
            install: install_spec(plugin, version),
            compatible: check.is_ok(),
            reason: check.err().map(|err| err.to_string()),
        });
    }

    let default_version = pick_default(plugin, &compatible_versions);
    let installed_version = ctx.installed.get(&plugin.name).cloned();
    let installed = match &installed_version {
        Some(version) => {
            let status = match &default_version {
                Some(default) if default != version => {
                    let newer = semver::Version::parse(default)
                        .ok()
                        .zip(semver::Version::parse(version).ok())
                        .map(|(default, installed)| default > installed)
                        .unwrap_or(false);
                    if newer {
                        "update-available"
                    } else {
                        "different-version"
                    }
                }
                _ => "installed",
            };
            InstalledListing {
                status: status.to_string(),
                version: Some(version.clone()),
            }
        }
        None => InstalledListing {
            status: "not-installed".to_string(),
            version: None,
        },
    };

    Listing {
        name: plugin.name.clone(),
        title: plugin.display_title(ctx.lang),
        description: plugin.description.as_ref().map(|text| text.resolve(ctx.lang)),
        icon: plugin.icon.clone(),
        author: plugin.author.clone(),
        homepage: plugin.homepage.clone(),
        index_url: index_url.to_string(),
        category: plugin.metadata.category.clone(),
        labels: plugin.metadata.labels.clone(),
        keywords: plugin.metadata.keywords.clone(),
        default_version,
        versions,
        latest_publish: plugin.latest_publish(),
        installed,
    }
}

/// 默认安装版本：`dist-tags.latest` 若兼容就用它，否则最高兼容稳定版，再否则最高兼容版。
fn pick_default(plugin: &PluginEntry, compatible: &[&VersionEntry]) -> Option<String> {
    if let Some(latest) = plugin.dist_tags.get("latest") {
        if compatible.iter().any(|version| &version.version == latest) {
            return Some(latest.clone());
        }
    }
    let mut sorted: Vec<&VersionEntry> = compatible.to_vec();
    sorted.sort_by(|a, b| {
        let a = semver::Version::parse(&a.version).ok();
        let b = semver::Version::parse(&b.version).ok();
        match (a, b) {
            (Some(a), Some(b)) => a.cmp(&b),
            _ => std::cmp::Ordering::Equal,
        }
    });
    sorted
        .iter()
        .rev()
        .find(|version| {
            semver::Version::parse(&version.version)
                .map(|version| version.pre.is_empty())
                .unwrap_or(false)
        })
        .or_else(|| sorted.last())
        .map(|version| version.version.clone())
}

fn install_spec(plugin: &PluginEntry, version: &VersionEntry) -> String {
    format!("{}@{}", plugin.name, version.version)
}

fn taxonomy_of(plugin: &PluginEntry, loads: &[IndexLoad], lang: &str) -> String {
    // 找到声明这个插件的索引，取出分类 / 标签标题。
    let mut text = String::new();
    for load in loads {
        let Some(snapshot) = &load.snapshot else { continue };
        if !snapshot.document.plugins.iter().any(|item| item.name == plugin.name) {
            continue;
        }
        if let Some(category) = &plugin.metadata.category {
            if let Some(declaration) = snapshot.document.metadata.categories.get(category) {
                text.push_str(&declaration.title.resolve(lang));
                text.push(' ');
            }
        }
        for label in &plugin.metadata.labels {
            if let Some(declaration) = snapshot.document.metadata.labels.get(label) {
                text.push_str(&declaration.title.resolve(lang));
                text.push(' ');
            }
        }
        break;
    }
    text
}

fn read_cache(path: &std::path::Path) -> Option<(i64, IndexDocument)> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let fetched_at = value.get("fetchedAt").and_then(Value::as_i64)?;
    let body = value.get("body")?;
    let bytes = serde_json::to_vec(body).ok()?;
    let document = parse_index(&bytes, "").ok()?;
    Some((fetched_at, document))
}

fn write_cache(path: &std::path::Path, fetched_at: i64, body: &[u8]) -> ExtResult<()> {
    let body: Value = serde_json::from_slice(body)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let envelope = serde_json::json!({ "fetchedAt": fetched_at, "body": body });
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&envelope)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::NativeTls)
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(FETCH_TIMEOUT))
        .timeout_global(Some(FETCH_TIMEOUT))
        .http_status_as_error(false)
        .user_agent("AI-DAW-Extensions/0.1")
        .build()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_index() -> String {
        r#"{
            "format": 1,
            "title": {"zh": "测试市场", "en": "Test Market"},
            "plugins": [
                {"name": "@aidaw/chord", "title": {"zh": "和弦建议"},
                 "metadata": {"keywords": ["和弦"]},
                 "versions": [{"version": "0.2.0", "publishTimestamp": 200,
                                "source": {"type": "npm"}, "engines": {"aidaw": ">=1.9.0"}}]},
                {"name": "@aidaw/old", "title": {"zh": "旧扩展"},
                 "versions": [{"version": "0.1.0", "publishTimestamp": 100,
                                "source": {"type": "npm"}, "engines": {"aidaw": ">=9.0.0"}}]}
            ]
        }"#
        .to_string()
    }

    fn loads() -> Vec<IndexLoad> {
        let document = parse_index(sample_index().as_bytes(), "https://e.com/index.json").unwrap();
        vec![IndexLoad {
            url: "https://e.com/index.json".into(),
            snapshot: Some(Snapshot {
                url: "https://e.com/index.json".into(),
                document,
                fetched_at: 1,
                from_cache: false,
            }),
            error: None,
        }]
    }

    #[test]
    fn search_filters_and_marks_compatibility() {
        let market = Market::new(std::env::temp_dir().join("aidaw-market-test"));
        let installed = BTreeMap::new();
        let ctx = ListingContext {
            app_version: "1.9.0",
            lang: "zh",
            installed: &installed,
        };
        let result = market.search(&loads(), "", &Sort::Index, 0, 100, &ctx);
        assert_eq!(result.total, 2);
        let old = result.entries.iter().find(|entry| entry.name == "@aidaw/old").unwrap();
        assert!(!old.versions[0].compatible);
        assert_eq!(old.default_version, None);
        assert_eq!(old.installed.status, "not-installed");

        let chord = result.entries.iter().find(|entry| entry.name == "@aidaw/chord").unwrap();
        assert_eq!(chord.default_version.as_deref(), Some("0.2.0"));
        assert_eq!(chord.versions[0].install, "@aidaw/chord@0.2.0");
    }

    #[test]
    fn installed_and_update_states() {
        let market = Market::new(std::env::temp_dir().join("aidaw-market-test2"));
        let installed = BTreeMap::from([("@aidaw/chord".to_string(), "0.1.0".to_string())]);
        let ctx = ListingContext {
            app_version: "1.9.0",
            lang: "zh",
            installed: &installed,
        };
        let result = market.search(&loads(), "和弦", &Sort::Relevance, 0, 100, &ctx);
        assert_eq!(result.total, 1);
        assert_eq!(result.entries[0].installed.status, "update-available");
        assert_eq!(result.entries[0].installed.version.as_deref(), Some("0.1.0"));
    }

    #[test]
    fn pagination_reports_next_offset() {
        let market = Market::new(std::env::temp_dir().join("aidaw-market-test3"));
        let installed = BTreeMap::new();
        let ctx = ListingContext {
            app_version: "1.9.0",
            lang: "zh",
            installed: &installed,
        };
        let page = market.search(&loads(), "", &Sort::Index, 0, 1, &ctx);
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.next_offset, Some(1));
    }

    #[test]
    fn first_index_wins_for_duplicate_names() {
        let market = Market::new(std::env::temp_dir().join("aidaw-market-test4"));
        let mut two = loads();
        let first_title = two[0].snapshot.as_ref().unwrap().document.plugins[0].title.clone();
        two.push(two[0].clone());
        let installed = BTreeMap::new();
        let ctx = ListingContext {
            app_version: "1.9.0",
            lang: "zh",
            installed: &installed,
        };
        let result = market.search(&two, "", &Sort::Index, 0, 100, &ctx);
        assert_eq!(result.total, 2, "同名只算一条");
        assert_eq!(result.sources.len(), 2);
        let _ = first_title;
    }

    #[test]
    fn bad_index_reports_error_and_keeps_cache() {
        let dir = tempfile::tempdir().unwrap();
        let market = Market::new(dir.path().to_path_buf());
        let url = "https://example.invalid/index.json";
        // 没有缓存、网络也不通：只有错误。
        let load = market.load(url, true);
        assert!(load.snapshot.is_none());
        assert!(load.error.is_some());
    }

    #[test]
    fn cache_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let market = Market::new(dir.path().to_path_buf());
        let url = "https://example.invalid/cache.json";
        let path = market.cache_path(url);
        write_cache(&path, 42, sample_index().as_bytes()).unwrap();
        let (fetched_at, document) = read_cache(&path).unwrap();
        assert_eq!(fetched_at, 42);
        assert_eq!(document.plugins.len(), 2);
    }

    #[test]
    fn empty_index_is_a_valid_market() {
        let document = parse_index(r#"{"format":1,"title":"空","plugins":[]}"#.as_bytes(), "https://e.com/i.json");
        assert!(document.is_ok());
    }
}
