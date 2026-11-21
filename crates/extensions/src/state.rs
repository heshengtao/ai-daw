//! 扩展状态：`<配置目录>/extensions.json`。
//!
//! 只存**用户的选择与来源**，不存注册信息（那是运行时派生的）。字段带 `version`，格式变了
//! 走迁移，不猜。

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{ExtError, ExtResult};

/// 状态文件当前版本。
pub const STATE_VERSION: u32 = 1;

/// 默认市场索引地址：官方扩展商店仓库的 `index.json`（GitHub Pages 静态托管）。
///
/// 仓库：<https://github.com/heshengtao/ai-daw-plugins>。
///
/// **为什么用 Pages 而不是 raw.githubusercontent.com**：实测国内网络下，raw 的 TLS 连接
/// 会被 reset（curl 能过、Rust 的 native-tls 被掐），而 `*.github.io` 走 Fastly，能稳定取到。
/// raw 地址仍可用，用户也可以在「扩展 → 设置」里换成任意 https 索引源。
pub const DEFAULT_INDEX_URL: &str =
    "https://heshengtao.github.io/ai-daw-plugins/index.json";

/// 默认索引源列表。
///
/// 地址里若还带着 `<...>` 占位（例如别人 fork 后没改），视为**未配置**：给一个打不开的
/// URL 只会让「发现」页每次刷新都报一条「invalid uri」的错，不如老实显示「还没有配置索引源」。
pub fn default_indexes() -> Vec<String> {
    if DEFAULT_INDEX_URL.contains('<') || DEFAULT_INDEX_URL.contains('>') {
        Vec::new()
    } else {
        vec![DEFAULT_INDEX_URL.to_string()]
    }
}

/// 默认 npm registry。
pub const DEFAULT_REGISTRY: &str = "https://registry.npmjs.org/";

/// 国内可直连的 npm 镜像（顺序即回退顺序）。
pub const DEFAULT_MIRRORS: &[&str] = &["https://registry.npmmirror.com/"];

/// 已安装扩展的来源。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Source {
    /// `npm` / `local` / `tarball`。
    #[serde(rename = "type")]
    pub kind: String,
    /// 装它时用的 registry（`npm`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
    /// 从哪个市场索引装的（`npm`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<String>,
    /// 本地导入时的原始路径（`local` / `tarball`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl Source {
    /// 来自 npm registry。
    pub fn npm(registry: impl Into<String>, index: Option<String>) -> Self {
        Self {
            kind: "npm".into(),
            registry: Some(registry.into()),
            index,
            path: None,
        }
    }

    /// 本地导入。
    pub fn local(path: impl Into<String>) -> Self {
        Self {
            kind: "local".into(),
            path: Some(path.into()),
            ..Default::default()
        }
    }

    /// 给人看的一句话。
    pub fn label(&self) -> String {
        match self.kind.as_str() {
            "npm" => format!("npm · {}", self.registry.clone().unwrap_or_default()),
            "local" => format!("本地 · {}", self.path.clone().unwrap_or_default()),
            other => other.to_string(),
        }
    }
}

/// 一条已安装记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Installed {
    /// 已装版本。
    pub version: String,
    /// 是否启用（`false` 时完全不装载）。
    pub enabled: bool,
    /// 来源。
    #[serde(default)]
    pub source: Source,
    /// 安装时间（Unix 秒）。
    #[serde(default)]
    pub installed_at: i64,
    /// 扩展自己的配置（`activate(aidaw, config)` 的第二个参数）。
    #[serde(default)]
    pub config: Value,
    /// 清单声明的能力快照（展示用）。
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// 清单声明的权限快照（展示与确认用）。
    #[serde(default)]
    pub permissions: Vec<String>,
}

impl Installed {
    /// 是否声明了某个能力。
    pub fn has(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|item| item == capability)
    }
}

/// `extensions.json` 的内容。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    /// 状态格式版本。
    #[serde(default = "default_version")]
    pub version: u32,
    /// 市场索引地址（数组顺序 = 展示顺序，同名条目先出现者赢）。
    #[serde(default)]
    pub indexes: Vec<String>,
    /// npm registry。
    #[serde(default = "default_registry")]
    pub registry: String,
    /// npm 镜像（安装时按顺序回退）。
    #[serde(default = "default_mirrors")]
    pub mirrors: Vec<String>,
    /// 已安装的扩展（包名 → 记录）。
    #[serde(default)]
    pub installed: BTreeMap<String, Installed>,
}

fn default_version() -> u32 {
    STATE_VERSION
}

fn default_registry() -> String {
    DEFAULT_REGISTRY.to_string()
}

fn default_mirrors() -> Vec<String> {
    DEFAULT_MIRRORS.iter().map(|mirror| mirror.to_string()).collect()
}

impl Default for State {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            indexes: default_indexes(),
            registry: default_registry(),
            mirrors: default_mirrors(),
            installed: BTreeMap::new(),
        }
    }
}

impl State {
    /// 读状态文件；不存在时返回默认状态（默认带一个索引源）。
    pub fn load(path: &Path) -> ExtResult<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        let value: Value = serde_json::from_str(&text)?;
        let version = value.get("version").and_then(Value::as_u64).unwrap_or(0) as u32;
        if version > STATE_VERSION {
            return Err(ExtError::State(format!(
                "{version} 比本程序支持的 {STATE_VERSION} 新，请升级 AI DAW"
            )));
        }
        let mut state: Self = serde_json::from_value(value)?;
        state.migrate(version);
        Ok(state)
    }

    fn migrate(&mut self, from: u32) {
        // v0（没有 version 字段）→ v1：补默认索引源与镜像。
        if from < 1 {
            if self.indexes.is_empty() {
                self.indexes = default_indexes();
            }
            if self.mirrors.is_empty() {
                self.mirrors = default_mirrors();
            }
        }
        self.version = STATE_VERSION;
    }

    /// 原子写回（先写 `.tmp` 再 rename）。
    pub fn save(&self, path: &Path) -> ExtResult<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// 可用的 registry 序列：配置的 registry 在前，镜像在后，去重。
    pub fn registries(&self) -> Vec<String> {
        let mut list = Vec::new();
        let mut push = |url: &str| {
            let trimmed = url.trim().trim_end_matches('/');
            if trimmed.is_empty() {
                return;
            }
            if !list.iter().any(|item: &String| item == trimmed) {
                list.push(trimmed.to_string());
            }
        };
        push(&self.registry);
        for mirror in &self.mirrors {
            push(mirror);
        }
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let state = State::load(&dir.path().join("extensions.json")).unwrap();
        // 占位地址不算配置：默认没有索引源（设置页或商店仓库建好后填）。
        assert_eq!(state.indexes, default_indexes());
        assert!(state.installed.is_empty());
    }

    #[test]
    fn default_index_is_the_official_pages_url() {
        assert_eq!(
            default_indexes(),
            vec!["https://heshengtao.github.io/ai-daw-plugins/index.json".to_string()]
        );
        assert!(!DEFAULT_INDEX_URL.contains('<'));
    }

    #[test]
    fn placeholder_index_is_treated_as_unset() {
        assert!(!DEFAULT_INDEX_URL.contains('<') || default_indexes().is_empty());
    }

    #[test]
    fn round_trips_and_preserves_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        let mut state = State::default();
        state.indexes = vec!["https://example.com/index.json".into()];
        state.installed.insert(
            "@aidaw/hello".into(),
            Installed {
                version: "0.1.0".into(),
                enabled: true,
                source: Source::npm("https://registry.npmjs.org", None),
                installed_at: 1,
                config: Value::Null,
                capabilities: vec!["tools".into()],
                permissions: vec![],
            },
        );
        state.save(&path).unwrap();
        let loaded = State::load(&path).unwrap();
        assert_eq!(loaded.indexes, state.indexes);
        assert_eq!(loaded.installed["@aidaw/hello"].version, "0.1.0");
    }

    #[test]
    fn newer_state_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        std::fs::write(&path, r#"{"version":99,"installed":{}}"#).unwrap();
        assert!(matches!(State::load(&path), Err(ExtError::State(_))));
    }

    #[test]
    fn registries_dedupe_and_keep_order() {
        let mut state = State::default();
        state.registry = "https://registry.npmjs.org/".into();
        state.mirrors = vec![
            "https://registry.npmmirror.com".into(),
            "https://registry.npmjs.org".into(),
            " ".into(),
        ];
        assert_eq!(
            state.registries(),
            vec![
                "https://registry.npmjs.org".to_string(),
                "https://registry.npmmirror.com".to_string()
            ]
        );
    }
}
