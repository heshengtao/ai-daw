//! 扩展包格式：`package.json` 的 `aidaw` 字段 + `aidaw.json`。
//!
//! 规格见 `docs/superpowers/specs/2026-11-18-extensions-architecture-design.md` §5。要点：
//!
//! - `package.json` 里必须有 `aidaw.manifest`，否则这个包不是扩展；
//! - `aidaw.json` 的 `apiVersion` 必须是 1；
//! - `host` / `client` 至少要有一个，入口文件必须真实存在且**在包目录内**；
//! - `engines.aidaw`（或 `minAppVersion`）用 semver 对当前应用版本求值。

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{ExtError, ExtResult};

/// 当前扩展 API 版本。不兼容的变更才升它。
pub const API_VERSION: u32 = 1;

/// 本地化文本：`"标题"` 或 `{ "zh": "标题", "en": "Title" }`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Localized {
    /// 单一语言（当作英文/默认）。
    Plain(String),
    /// 按语言代码分组；键是小写（`zh` / `zh-cn` / `en`）。
    Map(BTreeMap<String, String>),
}

impl Localized {
    /// 取某个语言的文本：先精确匹配，再按主语言前缀（`zh-cn` → `zh`）回退，最后退回第一个。
    pub fn resolve(&self, lang: &str) -> String {
        match self {
            Self::Plain(text) => text.clone(),
            Self::Map(map) => {
                if let Some(text) = map.get(lang) {
                    return text.clone();
                }
                let primary = lang.split(['-', '_']).next().unwrap_or(lang);
                if let Some((_, text)) = map.iter().find(|(key, _)| key.split(['-', '_']).next() == Some(primary)) {
                    return text.clone();
                }
                map.get("en").cloned().or_else(|| map.values().next().cloned()).unwrap_or_default()
            }
        }
    }

    /// 是否为空（没有任何非空文本）。
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Plain(text) => text.trim().is_empty(),
            Self::Map(map) => map.values().all(|text| text.trim().is_empty()),
        }
    }
}

/// 扩展声明的能力。注册对应资源前必须声明，否则装载失败（fail-loud）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Capability {
    /// 注册 MCP 工具。
    Tools,
    /// 注册界面面板。
    Ui,
    /// 私有存储。
    Storage,
    /// 订阅引擎事件。
    Events,
    /// 注册内容包。
    Content,
    /// 注册供应商 / 模型条目（v1 只登记，不接线）。
    Providers,
}

impl Capability {
    /// 清单里的字面量。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tools => "tools",
            Self::Ui => "ui",
            Self::Storage => "storage",
            Self::Events => "events",
            Self::Content => "content",
            Self::Providers => "providers",
        }
    }
}

/// 一个面板声明：清单里说"我会往右栏贡献这个面板"，Client 侧再用同一个 id 注册组件。
///
/// 声明放在清单而不是运行时代码里，是为了让 daemon（MCP、无头模式、管理器）不用加载前端
/// 代码就知道"这个扩展有没有界面、叫什么"。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PaneSpec {
    /// 面板 id（全局唯一，且不能与内置面板撞名）。
    pub id: String,
    /// 标题。
    #[serde(default)]
    pub title: Option<Localized>,
    /// 图标键（宿主内置图标表里的名字，避免注入 SVG）。
    #[serde(default)]
    pub icon: Option<String>,
    /// 排序权重（小的靠前，缺省 100）。
    #[serde(default)]
    pub order: Option<i32>,
}

/// `aidaw.json` 的内容。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtManifest {
    /// 扩展 API 版本，必须是 [`API_VERSION`]。
    pub api_version: u32,
    /// 展示用短 id（包名才是身份）。小写字母/数字/连字符。
    pub id: String,
    /// 标题。
    #[serde(default)]
    pub title: Option<Localized>,
    /// 一句话说明。
    #[serde(default)]
    pub description: Option<Localized>,
    /// Host 侧入口（相对包根）。
    #[serde(default)]
    pub host: Option<String>,
    /// Client 侧入口（相对包根，ESM）。
    #[serde(default)]
    pub client: Option<String>,
    /// 声明的能力。
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    /// 预留的权限位（v1 恒为空；非空时安装需要逐项确认）。
    #[serde(default)]
    pub permissions: Vec<String>,
    /// 图标（相对包根，svg/png/jpeg/webp）。
    #[serde(default)]
    pub icon: Option<String>,
    /// 贡献的面板（需要 `ui` 能力与 `client` 入口）。
    #[serde(default)]
    pub panes: Vec<PaneSpec>,
    /// 兼容性下限兜底（`engines.aidaw` 缺失时才用）。
    #[serde(default)]
    pub min_app_version: Option<String>,
}

impl ExtManifest {
    /// 是否声明了某个能力。
    pub fn has(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability)
    }
}

/// `package.json` 里我们关心的字段（其余忽略，向前兼容）。
#[derive(Debug, Clone, Deserialize)]
pub struct PackageJson {
    /// npm 包名。
    pub name: String,
    /// 版本。
    pub version: String,
    /// 描述（`aidaw.json` 没有 description 时兜底）。
    #[serde(default)]
    pub description: Option<String>,
    /// `engines.aidaw`。
    #[serde(default)]
    pub engines: Option<Engines>,
    /// 扩展声明：`{ "manifest": "./aidaw.json" }`。
    #[serde(default)]
    pub aidaw: Option<AidawField>,
}

/// `package.json` 的 `engines`。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Engines {
    /// 对 AI DAW 版本的要求（npm semver range）。
    #[serde(default)]
    pub aidaw: Option<String>,
}

/// `package.json` 的 `aidaw` 字段。
#[derive(Debug, Clone, Deserialize)]
pub struct AidawField {
    /// 清单文件相对路径。
    pub manifest: String,
}

/// 一个已校验的扩展包。
#[derive(Debug, Clone)]
pub struct Package {
    /// 包根目录。
    pub root: PathBuf,
    /// npm 包名（身份）。
    pub name: String,
    /// 版本。
    pub version: String,
    /// 包描述。
    pub description: Option<String>,
    /// 清单。
    pub manifest: ExtManifest,
    /// Host 入口绝对路径。
    pub host_entry: Option<PathBuf>,
    /// Client 入口绝对路径。
    pub client_entry: Option<PathBuf>,
    /// 图标绝对路径。
    pub icon_entry: Option<PathBuf>,
    /// `engines.aidaw` 原始声明。
    pub engines_aidaw: Option<String>,
}

impl Package {
    /// 展示标题：清单 title → 包名。
    pub fn title(&self, lang: &str) -> String {
        self.manifest
            .title
            .as_ref()
            .map(|title| title.resolve(lang))
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| self.name.clone())
    }

    /// 是否声明了某个能力。
    pub fn has(&self, capability: Capability) -> bool {
        self.manifest.has(capability)
    }
}

/// 读一个包并校验（不做版本兼容判断，见 [`check_engine`]）。
pub fn load_package(root: &Path) -> ExtResult<Package> {
    let root = root
        .canonicalize()
        .map_err(|err| ExtError::Io(format!("{}: {err}", root.display())))?;
    let package_json = read_json::<PackageJson>(&root.join("package.json"))
        .map_err(|err| ExtError::InvalidPackage(format!("{}: {err}", root.join("package.json").display())))?;

    let aidaw = package_json.aidaw.ok_or_else(|| {
        ExtError::InvalidPackage(format!(
            "{} 的 package.json 没有 `aidaw.manifest`——它不是 AI DAW 扩展",
            package_json.name
        ))
    })?;

    let manifest_path = resolve_inside(&root, &aidaw.manifest)?;
    let manifest: ExtManifest = read_json(&manifest_path)?;

    if manifest.api_version != API_VERSION {
        return Err(ExtError::InvalidPackage(format!(
            "{} 的 apiVersion 是 {}，本版本只支持 {API_VERSION}",
            package_json.name, manifest.api_version
        )));
    }
    validate_id(&manifest.id)?;

    if manifest.host.is_none() && manifest.client.is_none() {
        return Err(ExtError::InvalidPackage(format!(
            "{} 既没有 host 也没有 client 入口（空包装了没有意义）",
            package_json.name
        )));
    }

    let host_entry = match &manifest.host {
        Some(path) => Some(resolve_inside(&root, path)?),
        None => None,
    };
    let client_entry = match &manifest.client {
        Some(path) => Some(resolve_inside(&root, path)?),
        None => None,
    };
    let icon_entry = match &manifest.icon {
        Some(path) => Some(resolve_inside(&root, path)?),
        None => None,
    };

    for (label, entry) in [("host", &host_entry), ("client", &client_entry)] {
        if let Some(path) = entry {
            if !path.is_file() {
                return Err(ExtError::InvalidPackage(format!(
                    "{} 的 {label} 入口不存在：{}",
                    package_json.name,
                    path.display()
                )));
            }
        }
    }

    let mut capabilities = manifest.capabilities.clone();
    capabilities.sort();
    capabilities.dedup();
    let mut manifest = manifest;
    manifest.capabilities = capabilities;

    if !manifest.panes.is_empty() {
        if client_entry.is_none() {
            return Err(ExtError::InvalidPackage(format!(
                "{} 声明了面板却没有 client 入口",
                package_json.name
            )));
        }
        if !manifest.has(Capability::Ui) {
            return Err(ExtError::InvalidPackage(format!(
                "{} 声明了面板却没有声明 ui 能力",
                package_json.name
            )));
        }
        let mut seen = std::collections::HashSet::new();
        for pane in &manifest.panes {
            validate_id(&pane.id)?;
            if !seen.insert(pane.id.clone()) {
                return Err(ExtError::InvalidPackage(format!(
                    "{} 重复声明了面板「{}」",
                    package_json.name, pane.id
                )));
            }
        }
    }

    let engines_aidaw = package_json
        .engines
        .as_ref()
        .and_then(|engines| engines.aidaw.clone())
        .or_else(|| manifest.min_app_version.clone());

    Ok(Package {
        root,
        name: package_json.name,
        version: package_json.version,
        description: package_json.description,
        manifest,
        host_entry,
        client_entry,
        icon_entry,
        engines_aidaw,
    })
}

/// 读一个包**但不做完整校验**：只取 `package.json` 的 name / version / engines。
///
/// 安装前（还没落盘时）用它做「版本兼容」预检。
pub fn peek_package_json(root: &Path) -> ExtResult<(String, String, Option<String>)> {
    let json: PackageJson = read_json(&root.join("package.json"))?;
    let engines = json.engines.and_then(|engines| engines.aidaw);
    Ok((json.name, json.version, engines))
}

/// 对当前应用版本检查 `engines.aidaw`。
///
/// 规则与 DSH 一致：**只认 `engines.aidaw`**，缺失视为不兼容（不猜、不回退 peerDeps）；
/// 空白串、非法 range 也是不兼容；预发布版本参与匹配。
pub fn check_engine(range: Option<&str>, app_version: &str) -> ExtResult<()> {
    let range = range.ok_or_else(|| {
        ExtError::Incompatible(format!(
            "包没有声明 engines.aidaw（AI DAW {app_version} 无法判断兼容性）"
        ))
    })?;
    let range = range.trim();
    if range.is_empty() {
        return Err(ExtError::Incompatible("engines.aidaw 是空串".into()));
    }
    // npm 风格的 range（空格分隔、`||`、连字符区间）由 [`crate::range`] 翻译后求值。
    let matched = crate::range::matches(range, app_version)
        .map_err(|err| ExtError::Incompatible(format!("engines.aidaw「{range}」不可用：{err}")))?;
    if matched {
        Ok(())
    } else {
        Err(ExtError::Incompatible(format!(
            "需要 AI DAW {range}，当前是 {app_version}"
        )))
    }
}

/// 包内相对路径 → 绝对路径，拒绝逃出包目录。
pub fn resolve_inside(root: &Path, relative: &str) -> ExtResult<PathBuf> {
    let relative = relative.trim();
    if relative.is_empty() {
        return Err(ExtError::InvalidPackage("入口路径是空串".into()));
    }
    let path = Path::new(relative);
    if path.is_absolute() {
        return Err(ExtError::InvalidPackage(format!("入口必须相对包根，不能是绝对路径：{relative}")));
    }
    for component in path.components() {
        match component {
            Component::ParentDir => {
                return Err(ExtError::InvalidPackage(format!("入口不能逃出包目录：{relative}")));
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(ExtError::InvalidPackage(format!("入口路径不合法：{relative}")));
            }
            _ => {}
        }
    }
    let joined = root.join(path);
    // 包内可能放软链：跟过去之后必须仍在包内。
    let canonical = joined
        .canonicalize()
        .map_err(|err| ExtError::InvalidPackage(format!("入口不存在 {}：{err}", joined.display())))?;
    if !canonical.starts_with(root) {
        return Err(ExtError::InvalidPackage(format!("入口指向包外：{relative}")));
    }
    Ok(canonical)
}

fn validate_id(id: &str) -> ExtResult<()> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
        && !id.starts_with('-')
        && !id.ends_with('-');
    if ok {
        Ok(())
    } else {
        Err(ExtError::InvalidPackage(format!(
            "id「{id}」不合法：只允许小写字母、数字与连字符，且不能以连字符开头/结尾"
        )))
    }
}

/// 读 JSON 文件。
pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> ExtResult<T> {
    let text = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_package(dir: &Path, package: &str, manifest: &str, host: Option<&str>) {
        std::fs::write(dir.join("package.json"), package).unwrap();
        std::fs::write(dir.join("aidaw.json"), manifest).unwrap();
        if let Some(host) = host {
            std::fs::write(dir.join(host), "module.exports={};").unwrap();
        }
    }

    #[test]
    fn loads_a_minimal_package() {
        let dir = tempfile::tempdir().unwrap();
        write_package(
            dir.path(),
            r#"{"name":"@aidaw/hello","version":"0.1.0","description":"hi","aidaw":{"manifest":"./aidaw.json"},"engines":{"aidaw":">=1.0.0"}}"#,
            r#"{"apiVersion":1,"id":"hello","host":"./host.cjs"}"#,
            Some("host.cjs"),
        );
        let package = load_package(dir.path()).unwrap();
        assert_eq!(package.name, "@aidaw/hello");
        assert_eq!(package.manifest.id, "hello");
        assert!(package.host_entry.is_some());
        assert_eq!(package.engines_aidaw.as_deref(), Some(">=1.0.0"));
        assert!(check_engine(package.engines_aidaw.as_deref(), "1.9.0").is_ok());
    }

    #[test]
    fn rejects_package_without_aidaw_field() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"name":"x","version":"1.0.0"}"#).unwrap();
        let err = load_package(dir.path()).unwrap_err();
        assert!(matches!(err, ExtError::InvalidPackage(_)), "{err}");
    }

    #[test]
    fn rejects_wrong_api_version_and_empty_entry_points() {
        let dir = tempfile::tempdir().unwrap();
        write_package(
            dir.path(),
            r#"{"name":"x","version":"1.0.0","aidaw":{"manifest":"./aidaw.json"}}"#,
            r#"{"apiVersion":2,"id":"x","host":"./h.js"}"#,
            Some("h.js"),
        );
        assert!(load_package(dir.path()).is_err());

        std::fs::write(dir.path().join("aidaw.json"), r#"{"apiVersion":1,"id":"x"}"#).unwrap();
        let err = load_package(dir.path()).unwrap_err();
        assert!(err.to_string().contains("host"), "{err}");
    }

    #[test]
    fn rejects_entry_escaping_the_package() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"name":"x","version":"1.0.0","aidaw":{"manifest":"./aidaw.json"}}"#).unwrap();
        std::fs::write(dir.path().join("aidaw.json"), r#"{"apiVersion":1,"id":"x","host":"../evil.js"}"#).unwrap();
        let err = load_package(dir.path()).unwrap_err();
        assert!(err.to_string().contains("逃出"), "{err}");
    }

    #[test]
    fn engine_check_is_strict() {
        assert!(check_engine(None, "1.9.0").is_err(), "缺声明必须不兼容");
        assert!(check_engine(Some("  "), "1.9.0").is_err());
        assert!(check_engine(Some("not-a-range"), "1.9.0").is_err());
        assert!(check_engine(Some(">=2.0.0"), "1.9.0").is_err());
        assert!(check_engine(Some(">=1.9.0 <2.0.0"), "1.9.0-rc.1").is_ok(), "预发布参与匹配");
    }

    #[test]
    fn localized_resolution_falls_back() {
        let text = Localized::Map(BTreeMap::from([
            ("en".to_string(), "Title".to_string()),
            ("zh".to_string(), "标题".to_string()),
        ]));
        assert_eq!(text.resolve("zh-CN"), "标题");
        assert_eq!(text.resolve("fr"), "Title");
        assert_eq!(Localized::Plain("x".into()).resolve("zh"), "x");
    }
}
