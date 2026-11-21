//! 管理器：把包、状态、注册表、宿主缝在一起。
//!
//! 它是 daemon 侧唯一要认识的东西：装载 / 卸载 / 重载 / 调工具 / 派事件 / 列表。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::bridge::Bridge;
use crate::error::{ExtError, ExtResult};
use crate::host::{self, HostHandle, HostOptions};
use crate::manifest;
use crate::registry::{Contributions, ContentPack, PaneMeta, PluginStatus, Registry, ToolMeta};
use crate::state::{Installed, Source, State};

/// 管理器配置。
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    /// 包内容根目录（通常是 `<数据目录>/extensions`）。
    pub data_dir: PathBuf,
    /// 状态文件（通常是 `<配置目录>/extensions.json`）。
    pub state_path: PathBuf,
    /// 当前应用版本（用于 `engines.aidaw`）。
    pub app_version: String,
    /// 内置工具名（扩展不得占用）。
    pub reserved_tools: Vec<String>,
    /// 内置面板 id（扩展不得占用）。
    pub reserved_panes: Vec<String>,
}

impl ManagerConfig {
    /// 以数据目录 / 配置目录为基准的常用布局。
    pub fn new(data_dir: impl AsRef<Path>, config_dir: impl AsRef<Path>, app_version: impl Into<String>) -> Self {
        let data_dir = data_dir.as_ref().join("extensions");
        let state_path = config_dir.as_ref().join("extensions.json");
        Self {
            data_dir,
            state_path,
            app_version: app_version.into(),
            reserved_tools: Vec::new(),
            reserved_panes: Vec::new(),
        }
    }
}

/// 扩展管理器。
pub struct Manager {
    data_dir: PathBuf,
    state_path: PathBuf,
    app_version: String,
    state: Mutex<State>,
    registry: Arc<Registry>,
    hosts: Mutex<BTreeMap<String, HostHandle>>,
    bridge: Mutex<Option<Arc<dyn Bridge>>>,
}

impl Manager {
    /// 打开管理器（读状态文件；不存在就用默认状态）。
    pub fn open(config: ManagerConfig) -> ExtResult<Arc<Self>> {
        let state = State::load(&config.state_path)?;
        std::fs::create_dir_all(&config.data_dir)?;
        let registry = Arc::new(Registry::new(config.reserved_tools.clone(), config.reserved_panes.clone()));
        Ok(Arc::new(Self {
            data_dir: config.data_dir,
            state_path: config.state_path,
            app_version: config.app_version,
            state: Mutex::new(state),
            registry,
            hosts: Mutex::new(BTreeMap::new()),
            bridge: Mutex::new(None),
        }))
    }

    /// 接上宿主命令桥（daemon 启动时注入一次）。
    pub fn set_bridge(&self, bridge: Arc<dyn Bridge>) {
        *self.bridge.lock().unwrap_or_else(|err| err.into_inner()) = Some(bridge);
    }

    /// 当前应用版本。
    pub fn app_version(&self) -> &str {
        &self.app_version
    }

    /// 注册表（MCP / 界面直接读）。
    pub fn registry(&self) -> Arc<Registry> {
        Arc::clone(&self.registry)
    }

    /// 状态快照。
    pub fn state(&self) -> State {
        self.state.lock().unwrap_or_else(|err| err.into_inner()).clone()
    }

    /// 改状态并落盘。
    pub fn update_state<T>(&self, mutate: impl FnOnce(&mut State) -> T) -> ExtResult<T> {
        let cloned = {
            let mut state = self.state.lock().unwrap_or_else(|err| err.into_inner());
            let value = mutate(&mut state);
            (value, state.clone())
        };
        cloned.1.save(&self.state_path)?;
        Ok(cloned.0)
    }

    /// 全部工具。
    pub fn tools(&self) -> Vec<ToolMeta> {
        self.registry.tools()
    }

    /// 全部面板。
    pub fn panes(&self) -> Vec<PaneMeta> {
        self.registry.panes()
    }

    /// 全部内容包。
    pub fn contents(&self) -> Vec<ContentPack> {
        self.registry.contents()
    }

    /// 全部扩展状态。
    pub fn statuses(&self) -> Vec<PluginStatus> {
        self.registry.statuses()
    }

    /// 包目录：`<data>/extensions/<包名编码>/<版本>`。
    pub fn package_root(&self, package: &str, version: &str) -> PathBuf {
        self.data_dir.join(package_dir_name(package)).join(version)
    }

    /// 某个扩展的私有存储路径。
    pub fn storage_path(&self, package: &str) -> PathBuf {
        self.data_dir.join(package_dir_name(package)).join("storage.json")
    }

    /// 装载全部已启用的扩展（启动时调用一次；单个失败不影响其他）。
    ///
    /// 返回失败清单（包名, 原因），供启动日志与管理器展示。
    pub fn load_all(&self) -> Vec<(String, String)> {
        let installed = self.state().installed;
        let mut failures = Vec::new();
        for (package, record) in installed {
            if !record.enabled {
                self.registry.mark_disabled(&package, &record.version);
                continue;
            }
            if let Err(err) = self.load(&package) {
                failures.push((package, err.to_string()));
            }
        }
        failures
    }

    /// 装载（或重载）一个扩展。
    pub fn load(&self, package: &str) -> ExtResult<()> {
        // 先卸掉旧的（重载路径），保证"注册表里不会有半套"。
        self.unload(package);

        let record = self
            .state()
            .installed
            .get(package)
            .cloned()
            .ok_or_else(|| ExtError::InvalidPackage(format!("「{package}」没有安装记录")))?;
        let root = self.package_root(package, &record.version);
        let loaded = manifest::load_package(&root)?;
        manifest::check_engine(loaded.engines_aidaw.as_deref(), &self.app_version)?;

        let capabilities: Vec<String> = loaded.manifest.capabilities.iter().map(|item| item.as_str().to_string()).collect();

        // 只有界面（没有 Host）的扩展：清单里的面板照样要出现在注册表里。
        let Some(host_entry) = loaded.host_entry.clone() else {
            if loaded.client_entry.is_none() {
                return Err(ExtError::InvalidPackage(format!("「{package}」没有任何入口")));
            }
            self.registry
                .commit(package, &record.version, &capabilities, &Contributions::default(), &loaded.manifest.panes)?;
            return Ok(());
        };

        let source = std::fs::read_to_string(&host_entry)?;
        // 桥没接上也要能装载：扩展的工具逻辑本身可能不调宿主命令（纯计算），只是
        // `aidaw.call` 会在运行时拿到「桥还没接上」的错误。这样 HTTP 服务没起来时
        // 也不会把所有扩展一起判死。
        let bridge = self
            .bridge
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
            .unwrap_or_else(|| Arc::new(crate::bridge::NoopBridge));

        let handle = host::start(
            HostOptions {
                package: package.to_string(),
                version: record.version.clone(),
                app_version: self.app_version.clone(),
                source,
                config: record.config.clone(),
                capabilities: loaded.manifest.capabilities.clone(),
                storage_path: self.storage_path(package),
            },
            bridge,
            package.to_string(),
        )?;

        let contributions = Contributions {
            tools: handle.report.tools.clone(),
            contents: handle.report.contents.clone(),
            events: handle.report.events.clone(),
        };
        if let Err(err) = self.registry.commit(
            package,
            &record.version,
            &capabilities,
            &contributions,
            &loaded.manifest.panes,
        ) {
            handle.stop();
            self.registry.mark_failed(package, &record.version, err.to_string());
            return Err(err);
        }
        self.hosts
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(package.to_string(), handle);
        Ok(())
    }

    /// 卸载（停线程 + 回收注册）。
    pub fn unload(&self, package: &str) {
        let handle = self
            .hosts
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .remove(package);
        if let Some(handle) = handle {
            handle.stop();
        }
        self.registry.remove(package);
    }

    /// 重载（改文件后不重启应用）。
    pub fn reload(&self, package: &str) -> ExtResult<()> {
        self.load(package)
    }

    /// 启用 / 停用一个扩展。
    pub fn set_enabled(&self, package: &str, enabled: bool) -> ExtResult<()> {
        {
            let mut state = self.state.lock().unwrap_or_else(|err| err.into_inner());
            let record = state
                .installed
                .get_mut(package)
                .ok_or_else(|| ExtError::InvalidPackage(format!("「{package}」没有安装记录")))?;
            record.enabled = enabled;
        }
        self.save_state()?;
        if enabled {
            self.load(package).map(|_| ()).map_err(|err| {
                // 装载失败也要让人看见：状态表里留一条 failed。
                let version = self
                    .state()
                    .installed
                    .get(package)
                    .map(|record| record.version.clone())
                    .unwrap_or_default();
                self.registry.mark_failed(package, &version, err.to_string());
                err
            })
        } else {
            self.unload(package);
            let version = self
                .state()
                .installed
                .get(package)
                .map(|record| record.version.clone())
                .unwrap_or_default();
            self.registry.mark_disabled(package, &version);
            Ok(())
        }
    }

    /// 调一个工具（同步；MCP 层负责放到阻塞线程里）。
    pub fn call_tool(&self, name: &str, args: &str) -> ExtResult<String> {
        let tool = self
            .registry
            .tool(name)
            .ok_or_else(|| ExtError::Host(format!("没有扩展工具「{name}」")))?;
        let hosts = self.hosts.lock().unwrap_or_else(|err| err.into_inner());
        let handle = hosts
            .get(&tool.plugin)
            .ok_or_else(|| ExtError::Host(format!("扩展「{}」没有在运行", tool.plugin)))?;
        handle.call(name, args)
    }

    /// 派一条事件给订阅了它的扩展。
    pub fn dispatch_event(&self, event: &str, payload: &str) {
        let hosts = self.hosts.lock().unwrap_or_else(|err| err.into_inner());
        for handle in hosts.values() {
            if handle.report.events.iter().any(|item| item == event) {
                handle.dispatch_event(event, payload);
            }
        }
    }

    /// 把某个包登记成已安装（安装流水线最后一步调用）。
    pub fn record_installed(&self, package: &str, version: &str, source: Source, package_dir: &Path) -> ExtResult<()> {
        let loaded = manifest::load_package(package_dir)?;
        let capabilities: Vec<String> = loaded.manifest.capabilities.iter().map(|item| item.as_str().to_string()).collect();
        let permissions = loaded.manifest.permissions.clone();
        self.update_state(|state| {
            let record = Installed {
                version: version.to_string(),
                enabled: true,
                source,
                installed_at: unix_now(),
                config: Value::Null,
                capabilities,
                permissions,
            };
            state.installed.insert(package.to_string(), record);
        })?;
        Ok(())
    }

    /// 删掉一个扩展的目录与记录（卸载）。
    pub fn remove_installed(&self, package: &str) -> ExtResult<()> {
        self.unload(package);
        self.update_state(|state| state.installed.remove(package))?;
        let dir = self.data_dir.join(package_dir_name(package));
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }

    fn save_state(&self) -> ExtResult<()> {
        let state = self.state().clone();
        state.save(&self.state_path)
    }
}

/// 包名 → 目录名（把 `/` 编成 `__`，其余字符做白名单校验）。
pub fn package_dir_name(package: &str) -> String {
    let encoded = package.replace('/', "__");
    let safe = encoded
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '@' | '+') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    // 目录名不能是 `.` / `..`，也不该以点开头（隐藏目录 + 遍历观感）。
    let safe = safe.trim_start_matches('.').to_string();
    if safe.is_empty() {
        "unnamed".to_string()
    } else {
        safe
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::NoopBridge;

    fn manager() -> (tempfile::TempDir, Arc<Manager>) {
        let dir = tempfile::tempdir().unwrap();
        let config = ManagerConfig::new(dir.path().join("data"), dir.path().join("config"), "1.9.0");
        let manager = Manager::open(config).unwrap();
        manager.set_bridge(Arc::new(NoopBridge));
        (dir, manager)
    }

    fn install_fixture(manager: &Manager, package: &str, version: &str, manifest: &str, host: Option<&str>) {
        let root = manager.package_root(package, version);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("package.json"),
            format!(
                r#"{{"name":"{package}","version":"{version}","aidaw":{{"manifest":"./aidaw.json"}},"engines":{{"aidaw":">=1.9.0"}}}}"#
            ),
        )
        .unwrap();
        std::fs::write(root.join("aidaw.json"), manifest).unwrap();
        if let Some(host) = host {
            std::fs::write(root.join("host.cjs"), host).unwrap();
        }
        manager
            .record_installed(package, version, Source::local("fixture"), &root)
            .unwrap();
    }

    #[test]
    fn loads_a_plugin_and_exposes_its_tool() {
        let (_dir, manager) = manager();
        install_fixture(
            &manager,
            "@aidaw/hello",
            "0.1.0",
            r#"{"apiVersion":1,"id":"hello","host":"./host.cjs","capabilities":["tools"]}"#,
            Some("module.exports = { activate: function (aidaw) { aidaw.registerTool({ name: 'hello_world', description: '打招呼', parameters: {} }, function () { return 'hi'; }); } };"),
        );
        manager.load_all();
        assert_eq!(manager.tools().len(), 1);
        assert_eq!(manager.call_tool("hello_world", "{}").unwrap(), "\"hi\"");
        assert_eq!(manager.statuses()[0].state, crate::registry::LoadState::Loaded);
    }

    #[test]
    fn disabled_plugins_do_not_load() {
        let (_dir, manager) = manager();
        install_fixture(
            &manager,
            "@aidaw/hello",
            "0.1.0",
            r#"{"apiVersion":1,"id":"hello","host":"./host.cjs","capabilities":["tools"]}"#,
            Some("module.exports = { activate: function (aidaw) { aidaw.registerTool({ name: 'hello_world', description: 'd', parameters: {} }, function () { return 'hi'; }); } };"),
        );
        manager.set_enabled("@aidaw/hello", false).unwrap();
        assert!(manager.tools().is_empty());
        manager.set_enabled("@aidaw/hello", true).unwrap();
        assert_eq!(manager.tools().len(), 1);
    }

    #[test]
    fn unload_removes_registrations() {
        let (_dir, manager) = manager();
        install_fixture(
            &manager,
            "@aidaw/hello",
            "0.1.0",
            r#"{"apiVersion":1,"id":"hello","host":"./host.cjs","capabilities":["tools"]}"#,
            Some("module.exports = { activate: function (aidaw) { aidaw.registerTool({ name: 'hello_world', description: 'd', parameters: {} }, function () { return 'hi'; }); } };"),
        );
        manager.load_all();
        manager.unload("@aidaw/hello");
        assert!(manager.tools().is_empty());
        assert!(manager.statuses().is_empty());
    }

    #[test]
    fn incompatible_engine_is_reported_not_loaded() {
        let (_dir, manager) = manager();
        let root = manager.package_root("@aidaw/old", "0.1.0");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"name":"@aidaw/old","version":"0.1.0","aidaw":{"manifest":"./aidaw.json"},"engines":{"aidaw":">=2.0.0"}}"#,
        )
        .unwrap();
        std::fs::write(root.join("aidaw.json"), r#"{"apiVersion":1,"id":"old","host":"./h.cjs"}"#).unwrap();
        std::fs::write(root.join("h.cjs"), "module.exports = { activate: function () {} };").unwrap();
        manager.record_installed("@aidaw/old", "0.1.0", Source::local("x"), &root).unwrap();

        let failures = manager.load_all();
        assert_eq!(failures.len(), 1);
        assert!(failures[0].1.contains("2.0.0"), "{}", failures[0].1);
        assert!(manager.tools().is_empty());
    }

    #[test]
    fn package_dir_name_encodes_scopes() {
        assert_eq!(package_dir_name("@aidaw/hello"), "@aidaw__hello");
        let nasty = package_dir_name("../evil");
        assert!(!nasty.contains('/'), "{nasty}");
        assert!(!nasty.starts_with('.'), "{nasty}");
    }
}
