//! 扩展插件（Extensions）的 daemon 侧胶水。
//!
//! - 建管理器（数据目录布局、内置工具 / 面板保留名）；
//! - 把「执行宿主命令」接到 `/rpc` 的分发表上（[`RpcBridge`]）；
//! - 启动时装载已启用的扩展、把进程内事件总线转发给订阅者；
//! - 提供 MCP 派发用的 [`try_call`]。
//!
//! 真正的运行时在 `ai-daw-extensions` crate 里（QuickJS 宿主、注册表、市场、安装）。
//! 依赖方向：daemon → extensions。所以这里只做接线，不放业务。

use std::sync::Arc;

use serde_json::Value;

use ai_daw_extensions::bridge::{with_actor, Bridge};
use ai_daw_extensions::manager::{Manager as ExtManager, ManagerConfig};

use crate::state::AppState;
#[allow(unused_imports)] // 桌面端需要 `Manager` 才有 `app.state()`；无头端是固有方法
use crate::tauri_api::{AppHandle, Manager};

/// 内置面板 id：扩展不得占用（与 `src/stores/layout.ts` 的 `PANES` 一致）。
///
/// 前端那份是真相；这里是一份**保留名单**，用来在装载时拒绝冲突。改前端时记得同步。
pub const RESERVED_PANES: &[&str] = &[
    "inspector",
    "mixer",
    "editor",
    "plugins",
    "manager",
    "sounds",
    "ops",
    "agent",
    "ai",
];

/// 建扩展管理器（数据目录 `<数据目录>/extensions/`，状态 `<数据目录>/extensions.json`）。
///
/// 状态文件坏掉不能拦住整个应用启动：把它挪成 `.bad` 再用默认状态继续，并在日志里说清楚。
pub fn open_manager(host: &Arc<dyn crate::host::Host>) -> Arc<ExtManager> {
    let data = host.data_dir();
    let state_path = data.join("extensions.json");
    let config = ManagerConfig {
        data_dir: data.join("extensions"),
        state_path: state_path.clone(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        reserved_tools: builtin_tool_names(),
        reserved_panes: RESERVED_PANES.iter().map(|id| id.to_string()).collect(),
    };
    match ExtManager::open(config.clone()) {
        Ok(manager) => manager,
        Err(err) => {
            eprintln!("[extensions] 状态文件不可用（{err}），按默认状态继续");
            let _ = std::fs::rename(&state_path, data.join("extensions.json.bad"));
            ExtManager::open(config).unwrap_or_else(|err| {
                panic!("[extensions] 连默认状态都建不起来：{err}");
            })
        }
    }
}

fn builtin_tool_names() -> Vec<String> {
    crate::mcp::tools::definitions()
        .into_iter()
        .map(|def| def.name.to_string())
        .collect()
}

/// 把注册表里的工具名报给 MCP 之外的地方（CLI 帮助、测试）。
pub fn tool_names(state: &AppState) -> Vec<String> {
    state
        .extensions
        .registry()
        .tools()
        .into_iter()
        .map(|tool| tool.name)
        .collect()
}

/// 扩展发起的写操作用哪个署名。
///
/// 事件与撤销历史里显示成 `ext:<包名>`，这样「谁改的」一眼能看出来；不复用 `Agent("mcp")`
/// 是为了让审计与界面高亮能区分「外部 agent」与「装进来的扩展」。
pub fn actor_override() -> Option<ai_daw_engine::event::Actor> {
    ai_daw_extensions::bridge::current_actor()
        .map(|name| ai_daw_engine::event::Actor::Agent(format!("ext:{name}")))
}

/// MCP / `/rpc` 的派发兜底：命中扩展工具就交给它，否则返回 `None` 让内置表继续。
pub fn try_call(app: &AppHandle, name: &str, args: &str) -> Option<Result<String, String>> {
    let state = app.state::<Arc<AppState>>();
    let manager = Arc::clone(&state.extensions);
    let Some(tool) = manager.registry().tool(name) else {
        return None;
    };
    // 扩展在自己线程里同步调用自己的工具 → 会等自己，必须挡住（否则就是一个永久卡死）。
    if let Some(actor) = ai_daw_extensions::bridge::current_actor() {
        if actor == tool.plugin {
            return Some(Err(format!(
                "扩展「{}」不能在自己线程里同步调用自己的工具「{name}」（会死锁）；把逻辑抽成普通函数。",
                tool.plugin
            )));
        }
    }
    Some(manager.call_tool(name, args).map_err(|err| err.to_string()))
}

/// 执行宿主命令的桥：走 `/rpc` 那张分发表（与网页端、CLI 完全同一条实现）。
///
/// 它跑在扩展线程上（[`Bridge::call`] 是同步的），所以这里可以大胆阻塞。
pub struct RpcBridge {
    server: Arc<crate::server::ServerState>,
}

impl RpcBridge {
    /// 用 HTTP 服务的共享状态建桥。
    pub fn new(server: Arc<crate::server::ServerState>) -> Self {
        Self { server }
    }
}

impl Bridge for RpcBridge {
    fn call(&self, plugin: &str, cmd: &str, args: &str) -> Result<String, String> {
        let args: Value = if args.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(args).map_err(|err| format!("参数不是合法 JSON：{err}"))?
        };
        // 线程局部署名：编辑管线会把这次改动记成 `ext:<包名>`。
        //
        // 先查 `/rpc` 那张命令表（界面 / 浏览器 / CLI 同一条实现）；不在表里的
        // （`get_project_overview`、`add_notes` 这些是 **MCP 工具**，不是 rpc 命令）
        // 再落到 MCP 的派发表——两边的底层实现是同一份。
        let result = with_actor(plugin, || {
            crate::server::rpc::dispatch(&self.server, cmd, args.clone())
        });
        match result {
            Ok(value) => serde_json::to_string(&value).map_err(|err| err.to_string()),
            Err(crate::server::rpc::RpcError::Unknown(_)) => {
                let actor = ai_daw_engine::event::Actor::Agent(format!("ext:{plugin}"));
                with_actor(plugin, || {
                    crate::mcp::tools::dispatch(&self.server.app_handle(), &actor, cmd, args)
                })
            }
            Err(err) => Err(err.message()),
        }
    }

    fn log(&self, plugin: &str, level: &str, message: &str) {
        crate::logging::extension(plugin, level, message);
    }
}

/// 服务启动时的接线：接桥 → 装载已启用扩展 → 把总线事件转发给订阅者。
///
/// 返回装载失败清单（包名 + 原因），由调用方决定打到日志还是界面。
pub fn start(server: &Arc<crate::server::ServerState>) -> Vec<(String, String)> {
    let state = server.daemon.state_arc();
    state.extensions.set_bridge(Arc::new(RpcBridge::new(Arc::clone(server))));
    let failures = state.extensions.load_all();
    for (name, reason) in &failures {
        eprintln!("[extensions] 装载失败 {name}：{reason}");
    }
    spawn_event_fanout(Arc::clone(&state.extensions));
    failures
}

/// 订阅进程内事件总线 → 转发给订阅了该事件的扩展。
fn spawn_event_fanout(manager: Arc<ExtManager>) {
    let mut rx = crate::bus::subscribe();
    crate::tauri_api::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok((event, payload)) => manager.dispatch_event(&event, &payload.to_string()),
                // 慢消费者：丢掉积压（与 SSE 客户端同一套语义）。
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });
}

// ---------------------------------------------------------------------------
// IPC / 前端 API
// ---------------------------------------------------------------------------

/// 扩展状态变化事件（前端据此重载面板、管理器刷新列表）。
pub const EVENT_EXTENSIONS: &str = "extensions://changed";

/// 前端/CLI 的唯一入口：`extensions_rpc(op, args)`。
///
/// 为什么是一个名字 + `op` 而不是十几个命令：Tauri 的 `invoke_handler`、
/// `/rpc` 的命令表、CLI 的命令表是三份手工清单，每加一个命令要改三处；一个入口
/// 只加一次，后面加能力只是加一个 `op` 分支。
pub async fn api(app: &AppHandle, state: &Arc<AppState>, op: &str, args: Value) -> Result<Value, String> {
    match op {
        "list" => Ok(list_json(state)),
        "client_manifest" => client_manifest(state),
        "client_source" => {
            let name = arg_str(&args, "name")?;
            client_source(state, &name)
        }
        "set_enabled" => {
            let name = arg_str(&args, "name")?;
            let enabled = args.get("enabled").and_then(Value::as_bool).ok_or("缺 enabled")?;
            state
                .extensions
                .set_enabled(&name, enabled)
                .map_err(|err| err.to_string())?;
            notify(state);
            Ok(serde_json::json!({ "ok": true }))
        }
        "reload" => {
            let name = arg_str(&args, "name")?;
            state.extensions.reload(&name).map_err(|err| err.to_string())?;
            notify(state);
            Ok(serde_json::json!({ "ok": true }))
        }
        "uninstall" => {
            let name = arg_str(&args, "name")?;
            state.extensions.remove_installed(&name).map_err(|err| err.to_string())?;
            notify(state);
            Ok(serde_json::json!({ "ok": true }))
        }
        "install_local" => {
            let path = arg_str(&args, "path")?;
            let (app, owned) = (app.clone(), Arc::clone(state));
            let installed = crate::tauri_api::async_runtime::spawn_blocking(move || {
                let installer = installer(&app, &owned)?;
                installer
                    .install_local(&owned.extensions, std::path::Path::new(&path))
                    .map_err(|err| err.to_string())
            })
            .await
            .map_err(|err| err.to_string())??;
            notify(state);
            Ok(serde_json::json!({ "name": installed.name, "version": installed.version }))
        }
        "market" => {
            let (app, owned) = (app.clone(), Arc::clone(state));
            let refresh = args.get("refresh").and_then(Value::as_bool).unwrap_or(false);
            let text = args.get("text").and_then(Value::as_str).unwrap_or_default().to_string();
            let sort = ai_daw_extensions::search::Sort::parse(args.get("sort").and_then(Value::as_str).unwrap_or_default());
            let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
            let lang = args.get("lang").and_then(Value::as_str).unwrap_or("zh").to_string();
            let result = crate::tauri_api::async_runtime::spawn_blocking(move || {
                market_search(&app, &owned, refresh, &text, &sort, offset, limit, &lang)
            })
            .await
            .map_err(|err| err.to_string())??;
            Ok(result)
        }
        "install" => {
            let name = arg_str(&args, "name")?;
            let version = args.get("version").and_then(Value::as_str).map(str::to_string);
            let index = args.get("index").and_then(Value::as_str).map(str::to_string);
            let (app, owned) = (app.clone(), Arc::clone(state));
            crate::tauri_api::async_runtime::spawn_blocking(move || {
                install_package(&app, &owned, &name, version, index)
            })
            .await
            .map_err(|err| err.to_string())?
        }
        "call_tool" => {
            // 前端扩展调宿主：与 Host 侧 `aidaw.call` 同一条兜底路径（先 rpc，再 MCP 工具）。
            let name = arg_str(&args, "name")?;
            let payload = args.get("args").cloned().unwrap_or_else(|| serde_json::json!({}));
            let actor = ai_daw_engine::event::Actor::Agent("ext:ui".to_string());
            let text = crate::mcp::tools::dispatch(app, &actor, &name, payload)?;
            Ok(serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text)))
        }
        "set_indexes" => {
            let indexes = args
                .get("indexes")
                .and_then(Value::as_array)
                .ok_or("缺 indexes（数组）")?
                .iter()
                .map(|item| item.as_str().unwrap_or_default().trim().to_string())
                .collect::<Vec<_>>();
            for url in &indexes {
                validate_index_url(url).map_err(|err| format!("索引地址不可用：{err}"))?;
            }
            let mut deduped: Vec<String> = Vec::new();
            for url in indexes {
                if !deduped.iter().any(|item| item == &url) {
                    deduped.push(url);
                }
            }
            state
                .extensions
                .update_state(|state| state.indexes = deduped)
                .map_err(|err| err.to_string())?;
            notify(state);
            Ok(serde_json::json!({ "ok": true }))
        }
        "set_registry" => {
            let registry = args.get("registry").and_then(Value::as_str).unwrap_or_default().trim().to_string();
            validate_index_url(&registry).map_err(|err| format!("registry 不可用：{err}"))?;
            let mirrors: Vec<String> = args
                .get("mirrors")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|item| item.trim().to_string())
                        .filter(|item| !item.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            state
                .extensions
                .update_state(|state| {
                    state.registry = registry;
                    state.mirrors = mirrors;
                })
                .map_err(|err| err.to_string())?;
            Ok(serde_json::json!({ "ok": true }))
        }
        other => Err(format!("未知的扩展操作：{other}")),
    }
}

fn arg_str(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("缺 {key}"))
}

fn notify(state: &AppState) {
    crate::host::emit(&**state.host(), EVENT_EXTENSIONS, &serde_json::json!({ "at": now() }));
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// 已装扩展 + 注册表快照（MCP `list_extensions` 与前端共用）。
pub fn list_json(state: &AppState) -> Value {
    let snapshot = state.extensions.state();
    let statuses: std::collections::BTreeMap<String, _> = state
        .extensions
        .statuses()
        .into_iter()
        .map(|status| (status.plugin.clone(), status))
        .collect();
    let installed: Vec<Value> = snapshot
        .installed
        .iter()
        .map(|(name, record)| {
            let status = statuses.get(name);
            serde_json::json!({
                "name": name,
                "version": record.version,
                "enabled": record.enabled,
                "source": record.source,
                "capabilities": record.capabilities,
                "permissions": record.permissions,
                "config": record.config,
                "installedAt": record.installed_at,
                "state": status.map(|item| item.state).unwrap_or(ai_daw_extensions::registry::LoadState::Idle),
                "error": status.and_then(|item| item.error.clone()),
                "tools": status.map(|item| item.tools.clone()).unwrap_or_default(),
                "panes": status.map(|item| item.panes.clone()).unwrap_or_default(),
            })
        })
        .collect();
    serde_json::json!({
        "appVersion": env!("CARGO_PKG_VERSION"),
        "indexes": snapshot.indexes,
        "registry": snapshot.registry,
        "mirrors": snapshot.mirrors,
        "installed": installed,
        "panes": state.extensions.panes(),
        "tools": state.extensions.tools(),
        "contents": state.extensions.contents(),
    })
}

fn client_manifest(state: &AppState) -> Result<Value, String> {
    let snapshot = state.extensions.state();
    let mut items = Vec::new();
    for (name, record) in &snapshot.installed {
        if !record.enabled {
            continue;
        }
        let root = state.extensions.package_root(name, &record.version);
        let Ok(package) = ai_daw_extensions::manifest::load_package(&root) else {
            continue;
        };
        if package.client_entry.is_none() {
            continue;
        }
        items.push(serde_json::json!({
            "name": name,
            "version": record.version,
            "title": package.title("zh"),
            "panes": package.manifest.panes,
        }));
    }
    Ok(serde_json::json!({ "items": items }))
}

fn client_source(state: &AppState, name: &str) -> Result<Value, String> {
    let snapshot = state.extensions.state();
    let record = snapshot
        .installed
        .get(name)
        .ok_or_else(|| format!("没有安装「{name}」"))?;
    let root = state.extensions.package_root(name, &record.version);
    let package = ai_daw_extensions::manifest::load_package(&root).map_err(|err| err.to_string())?;
    let entry = package
        .client_entry
        .ok_or_else(|| format!("「{name}」没有 client 入口"))?;
    let source = std::fs::read_to_string(&entry).map_err(|err| err.to_string())?;
    Ok(serde_json::json!({ "name": name, "version": record.version, "source": source }))
}

/// 市场搜索（同步版；MCP 工具与 `api` 共用）。
pub fn market_search_json(
    app: &AppHandle,
    state: &AppState,
    refresh: bool,
    text: &str,
    sort: &ai_daw_extensions::search::Sort,
    offset: usize,
    limit: usize,
    lang: &str,
) -> Result<Value, String> {
    market_search(app, state, refresh, text, sort, offset, limit, lang)
}

/// 安装一个扩展（同步版）：`version` 为空时由 registry 选兼容的最新版。
pub fn install_package(
    app: &AppHandle,
    state: &AppState,
    name: &str,
    version: Option<String>,
    index: Option<String>,
) -> Result<Value, String> {
    let installer = installer(app, state)?;
    let request = match version {
        Some(version) => ai_daw_extensions::npm::VersionRequest::Exact(version),
        None => ai_daw_extensions::npm::VersionRequest::Latest,
    };
    let installed = installer
        .install_npm(&state.extensions, name, request, index)
        .map_err(|err| err.to_string())?;
    notify(state);
    Ok(serde_json::json!({ "name": installed.name, "version": installed.version }))
}

/// 卸载（同步版）。
pub fn uninstall_package(state: &AppState, name: &str) -> Result<Value, String> {
    state.extensions.remove_installed(name).map_err(|err| err.to_string())?;
    notify(state);
    Ok(serde_json::json!({ "ok": true, "name": name }))
}

/// 启用 / 停用（同步版）。
pub fn set_extension_enabled(state: &AppState, name: &str, enabled: bool) -> Result<Value, String> {
    state
        .extensions
        .set_enabled(name, enabled)
        .map_err(|err| err.to_string())?;
    notify(state);
    Ok(serde_json::json!({ "ok": true, "name": name, "enabled": enabled }))
}

/// 重新装载（同步版）。
pub fn reload_extension(state: &AppState, name: &str) -> Result<Value, String> {
    state.extensions.reload(name).map_err(|err| err.to_string())?;
    notify(state);
    Ok(serde_json::json!({ "ok": true, "name": name }))
}

fn installer(app: &AppHandle, state: &AppState) -> Result<ai_daw_extensions::installer::Installer, String> {
    let cache = crate::paths::cache_dir(app).map_err(|err| err.to_string())?;
    let snapshot = state.extensions.state();
    let npm = ai_daw_extensions::npm::NpmClient::new(snapshot.registries());
    Ok(ai_daw_extensions::installer::Installer::new(
        npm,
        cache.join("extensions").join("tmp"),
    ))
}

fn market_search(
    app: &AppHandle,
    state: &AppState,
    refresh: bool,
    text: &str,
    sort: &ai_daw_extensions::search::Sort,
    offset: usize,
    limit: usize,
    lang: &str,
) -> Result<Value, String> {
    let cache = crate::paths::cache_dir(app).map_err(|err| err.to_string())?;
    let snapshot = state.extensions.state();
    let market = ai_daw_extensions::market::Market::new(cache.join("extensions"));
    let loads = market.load_all(&snapshot.indexes, refresh);
    let installed: std::collections::BTreeMap<String, String> = snapshot
        .installed
        .iter()
        .map(|(name, record)| (name.clone(), record.version.clone()))
        .collect();
    let ctx = ai_daw_extensions::market::ListingContext {
        app_version: env!("CARGO_PKG_VERSION"),
        lang,
        installed: &installed,
    };
    let result = market.search(&loads, text, sort, offset, limit, &ctx);
    serde_json::to_value(result).map_err(|err| err.to_string())
}

/// 索引地址只接受 http(s)、不带凭据与片段。
pub fn validate_index_url(url: &str) -> Result<(), String> {
    if url.trim().is_empty() {
        return Err("地址是空的".into());
    }
    let lower = url.trim().to_ascii_lowercase();
    if !lower.starts_with("https://") && !lower.starts_with("http://") {
        return Err("必须是 http(s) 地址".into());
    }
    if url.contains('@') || url.contains('#') {
        return Err("不能带凭据或 # 片段".into());
    }
    Ok(())
}

/// Tauri / 前端的统一入口（`/rpc` 走的是 [`crate::server::mod::rpc_handler`] 里的同名分支）。
#[cfg_attr(feature = "desktop", tauri::command)]
pub async fn extensions_rpc(
    app: crate::tauri_api::AppHandle,
    op: String,
    args: Option<Value>,
) -> Result<Value, String> {
    let state = {
        let state = app.state::<Arc<AppState>>();
        Arc::clone(&state)
    };
    api(&app, &state, &op, args.unwrap_or(Value::Object(Default::default()))).await
}

/// 给 MCP `tools/list` 用的扩展工具定义（名字 / 说明 / schema）。
pub fn mcp_tool_defs(state: &AppState) -> Vec<(String, String, Value)> {
    state
        .extensions
        .registry()
        .tools()
        .into_iter()
        .map(|tool| (tool.name, tool.description, tool.parameters))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::Config;

    /// 起一个无头 daemon，往它的数据目录里放一个测试扩展，然后走真实的派发路径调它。
    ///
    /// 这条测试钉住三件事：管理器能从数据目录装载、`list_extensions` 的 JSON 里有它、
    /// `mcp::tools::dispatch` 在找不到内置工具时会落到扩展宿主。
    #[test]
    fn extension_tools_are_dispatched_and_listed() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::boot::HeadlessHost::new(dir.path()));
        let cfg = Config {
            data_dir: dir.path().to_path_buf(),
            lan: false,
            port: None,
            web_dir: None,
        };
        let daemon = crate::boot::bootstrap(host, cfg);
        let state = daemon.state_arc();

        // 扩展包：一个工具，返回配置里的问候语。
        let root = state.extensions.package_root("@aidaw/hello", "0.1.0");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"name":"@aidaw/hello","version":"0.1.0","aidaw":{"manifest":"./aidaw.json"},"engines":{"aidaw":">=1.9.0"}}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("aidaw.json"),
            r#"{"apiVersion":1,"id":"hello","host":"./host.cjs","capabilities":["tools"]}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("host.cjs"),
            r#"module.exports = { activate: function (aidaw) {
                aidaw.registerTool({ name: "hello_tool", description: "打招呼", parameters: { type: "object" }, risk: "read" },
                    function (args) { return { hello: String(args.who || "world") }; });
                aidaw.registerTool({ name: "hello_bridge", description: "转发宿主命令", parameters: { type: "object" }, risk: "read" },
                    function () { return aidaw.call("get_project_overview", {}); });
            } };"#,
        )
        .unwrap();

        // 测试桥：证明 `aidaw.call` 真的穿到了宿主命令层（daemon 里换成 `RpcBridge`）。
        struct TestBridge;
        impl ai_daw_extensions::bridge::Bridge for TestBridge {
            fn call(&self, _plugin: &str, cmd: &str, _args: &str) -> Result<String, String> {
                match cmd {
                    "get_project_overview" => Ok(r#"{"tempo":120,"tracks":[{},{}]}"#.into()),
                    other => Err(format!("测试桥不认识 {other}")),
                }
            }
            fn log(&self, _plugin: &str, _level: &str, _message: &str) {}
        }
        state.extensions.set_bridge(std::sync::Arc::new(TestBridge));

        state
            .extensions
            .record_installed("@aidaw/hello", "0.1.0", ai_daw_extensions::state::Source::local("test"), &root)
            .unwrap();
        state.extensions.load("@aidaw/hello").unwrap();

        // 1) list_extensions 的 JSON 里能看到它和它的工具。
        let listed = list_json(&state);
        assert_eq!(listed["installed"][0]["name"], "@aidaw/hello");
        assert_eq!(listed["installed"][0]["state"], "loaded");
        let tool_names: Vec<&str> = listed["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap_or_default())
            .collect();
        assert!(tool_names.contains(&"hello_tool"), "{tool_names:?}");
        assert!(tool_names.contains(&"hello_bridge"), "{tool_names:?}");

        // 2) 走 MCP 的派发：内置表没有 hello_tool，于是落到扩展宿主。
        let app = crate::tauri_api::rpc_app_handle();
        let out = crate::mcp::tools::dispatch(
            &app,
            &ai_daw_engine::event::Actor::Agent("mcp".into()),
            "hello_tool",
            serde_json::json!({ "who": "ai-daw" }),
        )
        .unwrap();
        assert_eq!(out, r#"{"hello":"ai-daw"}"#);

        // 2b) 扩展里调 `aidaw.call` 的路径也通（测试桥回了固定的工程概览）。
        let bridged = crate::mcp::tools::dispatch(
            &app,
            &ai_daw_engine::event::Actor::Agent("mcp".into()),
            "hello_bridge",
            serde_json::json!({}),
        )
        .unwrap();
        assert!(bridged.contains(r#""tempo":120"#), "{bridged}");

        // 2c) 安装必须有显式确认：没带 confirm 就拒绝（MCP 侧的高风险闸门）。
        let args: crate::mcp::InstallExtensionArgs = serde_json::from_value(serde_json::json!({
            "name": "@aidaw/does-not-matter",
            "confirm": false
        }))
        .unwrap();
        let refused = crate::mcp::tools::install_extension(&app, args).unwrap_err();
        assert!(refused.contains("confirm"), "{refused}");

        // 3) 停用后工具从注册表消失。
        state.extensions.set_enabled("@aidaw/hello", false).unwrap();
        assert!(state.extensions.registry().tool("hello_tool").is_none());
        assert!(crate::mcp::tools::dispatch(
            &app,
            &ai_daw_engine::event::Actor::Agent("mcp".into()),
            "hello_tool",
            serde_json::json!({}),
        )
        .is_err());
    }
}
