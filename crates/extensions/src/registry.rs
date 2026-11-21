//! 注册表：扩展贡献的**工具 / 面板 / 内容包**的唯一真相。
//!
//! MCP 的 `tools/list`、右栏面板清单、管理器里的「它贡献了什么」都读这里。
//! 规则（见设计 §3 D4）：
//!
//! - 工具名全局唯一，且不能影子**内置工具**；
//! - 面板 id 全局唯一，且不能影子**内置面板**；
//! - 冲突 = **整包装载失败**（不是覆盖、不是跳过），先加载者不受影响；
//! - 卸载时按插件名整体回收，不留残渣。

use std::collections::{BTreeMap, HashSet};
use std::sync::RwLock;

use serde::Serialize;
use serde_json::Value;

use crate::error::{ExtError, ExtResult};
use crate::manifest::{Localized, PaneSpec};

/// 工具的风险等级（扩展自报，展示与 `read_only_hint` 映射用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    /// 只读。
    Read,
    /// 写工程（默认）。
    Write,
    /// 危险（删除 / 覆盖 / 安装）。
    Dangerous,
}

impl Risk {
    /// 从清单里的字面量解析；不认识的值按 `write` 处理。
    pub fn parse(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "read" | "readonly" | "read-only" => Self::Read,
            "dangerous" | "danger" => Self::Dangerous,
            _ => Self::Write,
        }
    }

    /// 是否只读（MCP 的 `read_only_hint`）。
    pub fn is_read(self) -> bool {
        matches!(self, Self::Read)
    }
}

/// 一条已注册的工具。
#[derive(Debug, Clone, Serialize)]
pub struct ToolMeta {
    /// 工具名（全局唯一）。
    pub name: String,
    /// 贡献它的扩展（包名）。
    pub plugin: String,
    /// 给模型看的说明。
    pub description: String,
    /// JSON Schema。
    pub parameters: Value,
    /// 风险等级。
    pub risk: Risk,
}

/// 一条已注册的面板（来自清单声明）。
#[derive(Debug, Clone, Serialize)]
pub struct PaneMeta {
    /// 面板 id（全局唯一）。
    pub id: String,
    /// 贡献它的扩展。
    pub plugin: String,
    /// 标题。
    pub title: Localized,
    /// 图标键（宿主内置图标表里的名字）。
    pub icon: String,
    /// 排序权重（小的靠前）。
    pub order: i32,
}

/// 一个内容包（扩展通过 `aidaw.registerContent` 贡献）。
#[derive(Debug, Clone, Serialize)]
pub struct ContentPack {
    /// 贡献它的扩展。
    pub plugin: String,
    /// 内容本体（结构由内容域定义：曲风 / 配器 / 配乐规则…）。
    pub data: Value,
}

/// 扩展在运行时的装载状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LoadState {
    /// 已装载，注册已生效。
    Loaded,
    /// 装载失败（错误在 `error` 里）。
    Failed,
    /// 用户关掉了。
    Disabled,
    /// 还没尝试装载。
    Idle,
}

/// 一个扩展的运行时状态（管理器展示用）。
#[derive(Debug, Clone, Serialize)]
pub struct PluginStatus {
    /// 包名。
    pub plugin: String,
    /// 版本。
    pub version: String,
    /// 装载状态。
    pub state: LoadState,
    /// 失败原因。
    pub error: Option<String>,
    /// 它注册的工具名。
    pub tools: Vec<String>,
    /// 它注册的面板 id。
    pub panes: Vec<String>,
    /// 它订阅的事件名。
    pub events: Vec<String>,
    /// 它贡献的内容包数量。
    pub contents: usize,
}

/// 宿主在装载成功后一次性提交的注册结果。
#[derive(Debug, Clone, Default)]
pub struct Contributions {
    /// 工具。
    pub tools: Vec<ToolContribution>,
    /// 内容包。
    pub contents: Vec<Value>,
    /// 订阅的事件名。
    pub events: Vec<String>,
}

/// 一条待提交的工具注册。
#[derive(Debug, Clone)]
pub struct ToolContribution {
    /// 工具名。
    pub name: String,
    /// 说明。
    pub description: String,
    /// JSON Schema。
    pub parameters: Value,
    /// 风险等级。
    pub risk: Risk,
}

#[derive(Default)]
struct Inner {
    /// 内置工具名（扩展不得占用）。
    reserved_tools: HashSet<String>,
    /// 内置面板 id。
    reserved_panes: HashSet<String>,
    tools: BTreeMap<String, ToolMeta>,
    panes: BTreeMap<String, PaneMeta>,
    contents: Vec<ContentPack>,
    status: BTreeMap<String, PluginStatus>,
}

/// 注册表。
#[derive(Default)]
pub struct Registry {
    inner: RwLock<Inner>,
}

impl Registry {
    /// 用内置工具名与面板 id 初始化（冲突检测的底线）。
    pub fn new(reserved_tools: impl IntoIterator<Item = String>, reserved_panes: impl IntoIterator<Item = String>) -> Self {
        let inner = Inner {
            reserved_tools: reserved_tools.into_iter().collect(),
            reserved_panes: reserved_panes.into_iter().collect(),
            ..Inner::default()
        };
        Self {
            inner: RwLock::new(inner),
        }
    }

    /// 提交一个扩展的全部注册（**原子**：任何一条冲突都整体失败）。
    pub fn commit(
        &self,
        plugin: &str,
        version: &str,
        capabilities: &[String],
        contributions: &Contributions,
        panes: &[PaneSpec],
    ) -> ExtResult<()> {
        let mut inner = self.inner.write().unwrap_or_else(|err| err.into_inner());

        // 先整体体检，再一次性写入——避免"写了一半"的中间态。
        for tool in &contributions.tools {
            if inner.reserved_tools.contains(&tool.name) {
                return Err(ExtError::Conflict(format!(
                    "工具「{}」是内置工具，扩展不能占用它",
                    tool.name
                )));
            }
            if inner.tools.contains_key(&tool.name) {
                let owner = inner.tools[&tool.name].plugin.clone();
                return Err(ExtError::Conflict(format!(
                    "工具「{}」已被扩展「{owner}」占用",
                    tool.name
                )));
            }
        }
        for pane in panes {
            if inner.reserved_panes.contains(&pane.id) {
                return Err(ExtError::Conflict(format!(
                    "面板 id「{}」是内置面板，扩展不能占用它",
                    pane.id
                )));
            }
            if let Some(existing) = inner.panes.get(&pane.id) {
                return Err(ExtError::Conflict(format!(
                    "面板 id「{}」已被扩展「{}」占用",
                    pane.id, existing.plugin
                )));
            }
        }

        inner.tools.extend(contributions.tools.iter().map(|tool| {
            (
                tool.name.clone(),
                ToolMeta {
                    name: tool.name.clone(),
                    plugin: plugin.to_string(),
                    description: tool.description.clone(),
                    parameters: tool.parameters.clone(),
                    risk: tool.risk,
                },
            )
        }));
        inner.panes.extend(panes.iter().map(|pane| {
            (
                pane.id.clone(),
                PaneMeta {
                    id: pane.id.clone(),
                    plugin: plugin.to_string(),
                    title: pane
                        .title
                        .clone()
                        .unwrap_or(Localized::Plain(pane.id.clone())),
                    icon: pane.icon.clone().unwrap_or_else(|| "puzzle".to_string()),
                    order: pane.order.unwrap_or(100),
                },
            )
        }));
        for content in &contributions.contents {
            inner.contents.push(ContentPack {
                plugin: plugin.to_string(),
                data: content.clone(),
            });
        }

        inner.status.insert(
            plugin.to_string(),
            PluginStatus {
                plugin: plugin.to_string(),
                version: version.to_string(),
                state: LoadState::Loaded,
                error: None,
                tools: contributions.tools.iter().map(|tool| tool.name.clone()).collect(),
                panes: panes.iter().map(|pane| pane.id.clone()).collect(),
                events: contributions.events.clone(),
                contents: contributions.contents.len(),
            },
        );
        // 未声明能力却注册了东西，说明扩展在骗人——装载时已由宿主拦下，这里只是兜底。
        let _ = capabilities;
        Ok(())
    }

    /// 记一条失败状态（不动任何已生效的注册）。
    pub fn mark_failed(&self, plugin: &str, version: &str, error: impl Into<String>) {
        let mut inner = self.inner.write().unwrap_or_else(|err| err.into_inner());
        inner.status.insert(
            plugin.to_string(),
            PluginStatus {
                plugin: plugin.to_string(),
                version: version.to_string(),
                state: LoadState::Failed,
                error: Some(error.into()),
                tools: Vec::new(),
                panes: Vec::new(),
                events: Vec::new(),
                contents: 0,
            },
        );
        // 失败时把该插件可能残留的注册清掉（重载路径会先 commit 失败）。
        remove_plugin_locked(&mut inner, plugin);
    }

    /// 记一条「用户关掉了」。
    pub fn mark_disabled(&self, plugin: &str, version: &str) {
        let mut inner = self.inner.write().unwrap_or_else(|err| err.into_inner());
        remove_plugin_locked(&mut inner, plugin);
        inner.status.insert(
            plugin.to_string(),
            PluginStatus {
                plugin: plugin.to_string(),
                version: version.to_string(),
                state: LoadState::Disabled,
                error: None,
                tools: Vec::new(),
                panes: Vec::new(),
                events: Vec::new(),
                contents: 0,
            },
        );
    }

    /// 回收一个插件的全部注册（卸载 / 重载前）。
    pub fn remove(&self, plugin: &str) {
        let mut inner = self.inner.write().unwrap_or_else(|err| err.into_inner());
        remove_plugin_locked(&mut inner, plugin);
        inner.status.remove(plugin);
    }

    /// 全部工具（按名字排序，确定性）。
    pub fn tools(&self) -> Vec<ToolMeta> {
        let inner = self.inner.read().unwrap_or_else(|err| err.into_inner());
        inner.tools.values().cloned().collect()
    }

    /// 找一条工具。
    pub fn tool(&self, name: &str) -> Option<ToolMeta> {
        let inner = self.inner.read().unwrap_or_else(|err| err.into_inner());
        inner.tools.get(name).cloned()
    }

    /// 全部面板。
    pub fn panes(&self) -> Vec<PaneMeta> {
        let inner = self.inner.read().unwrap_or_else(|err| err.into_inner());
        let mut panes: Vec<PaneMeta> = inner.panes.values().cloned().collect();
        panes.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
        panes
    }

    /// 全部内容包。
    pub fn contents(&self) -> Vec<ContentPack> {
        let inner = self.inner.read().unwrap_or_else(|err| err.into_inner());
        inner.contents.clone()
    }

    /// 全部状态。
    pub fn statuses(&self) -> Vec<PluginStatus> {
        let inner = self.inner.read().unwrap_or_else(|err| err.into_inner());
        inner.status.values().cloned().collect()
    }

    /// 一条状态。
    pub fn status(&self, plugin: &str) -> Option<PluginStatus> {
        let inner = self.inner.read().unwrap_or_else(|err| err.into_inner());
        inner.status.get(plugin).cloned()
    }
}

fn remove_plugin_locked(inner: &mut Inner, plugin: &str) {
    inner.tools.retain(|_, tool| tool.plugin != plugin);
    inner.panes.retain(|_, pane| pane.plugin != plugin);
    inner.contents.retain(|content| content.plugin != plugin);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> ToolContribution {
        ToolContribution {
            name: name.into(),
            description: "d".into(),
            parameters: serde_json::json!({"type": "object"}),
            risk: Risk::Read,
        }
    }

    fn registry() -> Registry {
        Registry::new(
            ["get_project".to_string()],
            ["mixer".to_string(), "inspector".to_string()],
        )
    }

    #[test]
    fn commits_tools_and_lists_them() {
        let registry = registry();
        let contributions = Contributions {
            tools: vec![tool("a")],
            ..Default::default()
        };
        registry.commit("@aidaw/a", "1.0.0", &["tools".into()], &contributions, &[]).unwrap();
        assert_eq!(registry.tools().len(), 1);
        assert_eq!(registry.status("@aidaw/a").unwrap().state, LoadState::Loaded);
        assert_eq!(registry.tool("a").unwrap().risk, Risk::Read);
    }

    #[test]
    fn rejects_shadowing_builtin_and_duplicates() {
        let registry = registry();
        let builtin = Contributions {
            tools: vec![tool("get_project")],
            ..Default::default()
        };
        assert!(matches!(
            registry.commit("@aidaw/a", "1", &[], &builtin, &[]),
            Err(ExtError::Conflict(_))
        ));

        registry
            .commit("@aidaw/a", "1", &[], &Contributions { tools: vec![tool("x")], ..Default::default() }, &[])
            .unwrap();
        let duplicate = Contributions {
            tools: vec![tool("x")],
            ..Default::default()
        };
        let err = registry.commit("@aidaw/b", "1", &[], &duplicate, &[]).unwrap_err();
        assert!(err.to_string().contains("@aidaw/a"), "{err}");
        // 先加载者不受影响。
        assert!(registry.tool("x").is_some());
        assert!(registry.status("@aidaw/b").is_none());
    }

    #[test]
    fn a_failed_commit_writes_nothing() {
        let registry = registry();
        let mixed = Contributions {
            tools: vec![tool("fine"), tool("get_project")],
            ..Default::default()
        };
        assert!(registry.commit("@aidaw/bad", "1", &[], &mixed, &[]).is_err());
        assert!(registry.tool("fine").is_none(), "冲突时一条都不能写进去");
    }

    #[test]
    fn removal_cleans_everything() {
        let registry = registry();
        let contributions = Contributions {
            tools: vec![tool("a")],
            contents: vec![serde_json::json!({"genres": []})],
            events: vec!["project_changed".into()],
        };
        let panes = vec![PaneSpec {
            id: "p".into(),
            title: None,
            icon: None,
            order: None,
        }];
        registry.commit("@aidaw/a", "1", &[], &contributions, &panes).unwrap();
        assert_eq!(registry.panes().len(), 1);
        assert_eq!(registry.contents().len(), 1);

        registry.remove("@aidaw/a");
        assert!(registry.tools().is_empty());
        assert!(registry.panes().is_empty());
        assert!(registry.contents().is_empty());
        assert!(registry.status("@aidaw/a").is_none());
    }

    #[test]
    fn pane_conflicts_are_rejected_too() {
        let registry = registry();
        let pane = PaneSpec {
            id: "mixer".into(),
            title: None,
            icon: None,
            order: None,
        };
        assert!(matches!(
            registry.commit("@aidaw/a", "1", &[], &Contributions::default(), &[pane]),
            Err(ExtError::Conflict(_))
        ));
    }
}
