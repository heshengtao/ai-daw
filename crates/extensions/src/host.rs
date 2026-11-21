//! 一个扩展一个线程的 QuickJS 宿主。
//!
//! 为什么是「一个扩展一个线程」：QuickJS 的 `Runtime` / `Context` 是 `!Send` 的，必须独占
//! 线程；顺带得到隔离——某个扩展死循环只会卡自己的线程（还有中断钩子兜底），不会碰音频线程。
//!
//! 线程里的契约：
//!
//! - 装载：创建 Runtime（内存 / 栈上限 + 中断钩子）→ 注入 `__aidaw_host` → 求值 bootstrap →
//!   求值扩展的 CJS 包 → 调 `activate(aidaw, config)`；
//! - 运行：只收三种消息——调工具、派事件、停止；
//! - 卸载：调 `deactivate()`（可选）→ 丢弃上下文 → 线程退出。
//!
//! JS 与 Rust 之间只传字符串：宿主函数返回**信封 JSON**（`{ ok, value | error }`），JS 侧
//! bootstrap 负责拆封与抛错。这样 rquickjs 的异常转换只出现在一处，扩展作者看到的是普通的
//! `throw`。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rquickjs::{Context, Function, Object, Runtime, Value as JsValue};
use serde_json::Value;

use crate::bridge::Bridge;
use crate::error::{ExtError, ExtResult};
use crate::manifest::Capability;
use crate::registry::{Risk, ToolContribution};

/// 单个扩展的内存上限（QuickJS 堆）。
pub const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
/// 单个扩展的 JS 栈上限。
pub const STACK_LIMIT: usize = 4 * 1024 * 1024;
/// `activate` 的时限。
pub const ACTIVATE_TIMEOUT: Duration = Duration::from_secs(10);
/// 一次工具调用的时限。
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// 宿主启动参数。
pub struct HostOptions {
    /// 包名。
    pub package: String,
    /// 版本。
    pub version: String,
    /// 应用版本（JS 侧 `aidaw.version`）。
    pub app_version: String,
    /// 扩展 JS 源码（已读文件）。
    pub source: String,
    /// 扩展配置（`activate` 第二个参数）。
    pub config: Value,
    /// 声明的能力。
    pub capabilities: Vec<Capability>,
    /// 私有存储文件。
    pub storage_path: PathBuf,
}

/// 装载成功后回报给管理器的注册结果。
#[derive(Debug, Clone, Default)]
pub struct LoadReport {
    /// 注册的工具。
    pub tools: Vec<ToolContribution>,
    /// 订阅的事件名。
    pub events: Vec<String>,
    /// 内容包。
    pub contents: Vec<Value>,
}

/// 发给宿主线程的消息。
enum HostMsg {
    /// 调一个工具。
    Call {
        /// 工具名。
        tool: String,
        /// 参数（JSON 字符串）。
        args: String,
        /// 回包。
        reply: SyncSender<ExtResult<String>>,
    },
    /// 派一条事件。
    Event {
        /// 事件名。
        event: String,
        /// 负载（JSON 字符串）。
        payload: String,
    },
    /// 停掉（可带一个回包用于等待）。
    Stop {
        /// 回包。
        reply: Option<SyncSender<()>>,
    },
}

/// 宿主线程句柄。
#[derive(Debug)]
pub struct HostHandle {
    /// 发给线程的消息通道。
    tx: Sender<HostMsg>,
    /// 线程 join 句柄（停的时候用）。
    join: Option<std::thread::JoinHandle<()>>,
    /// 装载时拿到的注册结果。
    pub report: LoadReport,
}

impl HostHandle {
    /// 调一个工具（阻塞等回包，带超时）。
    pub fn call(&self, tool: &str, args: &str) -> ExtResult<String> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.tx
            .send(HostMsg::Call {
                tool: tool.to_string(),
                args: args.to_string(),
                reply: tx,
            })
            .map_err(|_| ExtError::Host("扩展线程已经退出".into()))?;
        match rx.recv_timeout(CALL_TIMEOUT + Duration::from_secs(5)) {
            Ok(result) => result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                Err(ExtError::Host(format!("工具「{tool}」执行超时")))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Err(ExtError::Host("扩展线程崩溃了".into()))
            }
        }
    }

    /// 派一条事件（不等待处理结果）。
    pub fn dispatch_event(&self, event: &str, payload: &str) {
        let _ = self.tx.send(HostMsg::Event {
            event: event.to_string(),
            payload: payload.to_string(),
        });
    }

    /// 停掉线程并等它退出。
    pub fn stop(mut self) {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let _ = self.tx.send(HostMsg::Stop { reply: Some(tx) });
        let _ = rx.recv_timeout(Duration::from_secs(5));
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// 启动一个扩展宿主，等它装载完成（或失败）。
pub fn start(
    options: HostOptions,
    bridge: Arc<dyn Bridge>,
    package_for_log: String,
) -> ExtResult<HostHandle> {
    let (msg_tx, msg_rx) = channel::<HostMsg>();
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<ExtResult<LoadReport>>(1);
    let plugin = options.package.clone();
    let thread_name: String = format!("ext-{}", plugin.chars().take(24).collect::<String>());
    let join = std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run(options, bridge, msg_rx, ready_tx.clone(), package_for_log)
            }));
            if let Err(panic) = outcome {
                let message = panic
                    .downcast_ref::<&str>()
                    .map(|text| text.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "扩展线程 panic".into());
                let _ = ready_tx.send(Err(ExtError::Host(format!("扩展线程崩溃：{message}"))));
            }
        })
        .map_err(|err| ExtError::Host(format!("起不了扩展线程：{err}")))?;

    let report = match ready_rx.recv_timeout(ACTIVATE_TIMEOUT + Duration::from_secs(5)) {
        Ok(Ok(report)) => report,
        Ok(Err(err)) => {
            let _ = join.join();
            return Err(err);
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            return Err(ExtError::Host(format!(
                "装载超过 {} 秒还没完成（扩展在 activate 里死循环了？）",
                ACTIVATE_TIMEOUT.as_secs()
            )));
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            let _ = join.join();
            return Err(ExtError::Host("扩展线程在装载时退出".into()));
        }
    };

    Ok(HostHandle {
        tx: msg_tx,
        join: Some(join),
        report,
    })
}

fn run(
    options: HostOptions,
    bridge: Arc<dyn Bridge>,
    rx: std::sync::mpsc::Receiver<HostMsg>,
    ready: SyncSender<ExtResult<LoadReport>>,
    log_name: String,
) {
    let deadline = Arc::new(AtomicU64::new(0));

    let runtime = match Runtime::new() {
        Ok(runtime) => runtime,
        Err(err) => {
            let _ = ready.send(Err(ExtError::Host(format!("建不起 QuickJS 运行时：{err}"))));
            return;
        }
    };
    runtime.set_memory_limit(MEMORY_LIMIT);
    runtime.set_max_stack_size(STACK_LIMIT);
    {
        let deadline = Arc::clone(&deadline);
        runtime.set_interrupt_handler(Some(Box::new(move || {
            let limit = deadline.load(Ordering::Relaxed);
            limit != 0 && now_ms() > limit
        })));
    }

    let context = match Context::full(&runtime) {
        Ok(context) => context,
        Err(err) => {
            let _ = ready.send(Err(ExtError::Host(format!("建不起 JS 上下文：{err}"))));
            return;
        }
    };

    let state = Arc::new(Mutex::new(HostState {
        plugin: options.package.clone(),
        capabilities: options.capabilities.clone(),
        tools: Vec::new(),
        events: Vec::new(),
        contents: Vec::new(),
        storage: Storage::open(options.storage_path.clone()),
    }));

    // ---- 装载 ----
    let load = context.with(|ctx| load_into(&ctx, &options, state.clone(), bridge.clone(), &deadline));
    let report = match load {
        Ok(report) => report,
        Err(err) => {
            let _ = ready.send(Err(err));
            return;
        }
    };
    if ready.send(Ok(report)).is_err() {
        return;
    }

    // ---- 运行 ----
    loop {
        match rx.recv() {
            Ok(HostMsg::Call { tool, args, reply }) => {
                let result = context.with(|ctx| call_tool(&ctx, &tool, &args, &deadline));
                let _ = reply.send(result);
            }
            Ok(HostMsg::Event { event, payload }) => {
                context.with(|ctx| dispatch_event(&ctx, &event, &payload, &bridge, &log_name));
            }
            Ok(HostMsg::Stop { reply }) => {
                context.with(|ctx| deactivate(&ctx, &bridge, &log_name));
                // 上下文随 `Context` 的 drop 一起销毁（引擎、句柄、内存全部回收）。
                drop(context);
                if let Some(reply) = reply {
                    let _ = reply.send(());
                }
                break;
            }
            Err(_) => break,
        }
    }
}

struct HostState {
    plugin: String,
    capabilities: Vec<Capability>,
    tools: Vec<ToolContribution>,
    events: Vec<String>,
    contents: Vec<Value>,
    storage: Storage,
}

fn load_into(
    ctx: &rquickjs::Ctx<'_>,
    options: &HostOptions,
    state: Arc<Mutex<HostState>>,
    bridge: Arc<dyn Bridge>,
    deadline: &Arc<AtomicU64>,
) -> ExtResult<LoadReport> {
    install_host(ctx, state.clone(), bridge.clone(), deadline)?;

    // 配置用「设一个字符串全局，JS 侧 JSON.parse」的方式传入——避免在 Rust 里做 JS 值转换。
    let config_json = serde_json::to_string(&options.config).unwrap_or_else(|_| "null".into());
    ctx.globals()
        .set("__aidaw_config_json", config_json)
        .map_err(|err| ExtError::Host(format!("注入配置失败：{err}")))?;
    ctx.globals()
        .set("__aidaw_app_version", options.app_version.clone())
        .map_err(|err| ExtError::Host(format!("注入版本失败：{err}")))?;
    ctx.globals()
        .set("__aidaw_plugin_name", options.package.clone())
        .map_err(|err| ExtError::Host(format!("注入包名失败：{err}")))?;
    ctx.eval::<(), _>(BOOTSTRAP)
        .map_err(|err| ExtError::Host(format!("bootstrap 失败：{}", js_error(ctx, err))))?;

    // 包成 CJS：给扩展提供 module / exports / require（后者只用来给出友好报错）。
    let wrapped = format!(
        "(function(){{ \"use strict\";\nvar module = {{ exports: {{}} }};\nvar exports = module.exports;\nvar require = function (name) {{ throw new Error(\"扩展不能 require（\" + String(name) + \"）：请打包成自包含的单文件 CJS\"); }};\n{}\n;globalThis.__aidaw_module_exports = module.exports;\n}})();",
        options.source
    );
    set_deadline(deadline, ACTIVATE_TIMEOUT);
    let evaluated = ctx.eval::<(), _>(wrapped);
    clear_deadline(deadline);
    evaluated.map_err(|err| ExtError::Host(format!("求值扩展入口失败：{}", js_error(ctx, err))))?;

    // activate：CJS 的 `module.exports.activate`，或 IIFE 里 `aidaw.plugin(fn)`。
    let exports: JsValue = ctx
        .globals()
        .get("__aidaw_module_exports")
        .map_err(|err| ExtError::Host(format!("取不到 module.exports：{err}")))?;
    let activate: Option<Function> = exports
        .as_object()
        .and_then(|object| object.get::<_, JsValue>("activate").ok())
        .and_then(|value| value.into_function());
    let plugin_fn: Option<Function> = ctx
        .globals()
        .get::<_, JsValue>("__aidaw_plugin_fn")
        .ok()
        .and_then(|value| value.into_function());
    let activate = activate
        .or(plugin_fn)
        .ok_or_else(|| ExtError::Host("扩展没有导出 activate()（见扩展开发指南）".into()))?;

    let aidaw = ctx
        .globals()
        .get::<_, Object>("aidaw")
        .map_err(|err| ExtError::Host(format!("取不到 aidaw 对象：{err}")))?;
    let config: JsValue = ctx
        .globals()
        .get("__aidaw_config")
        .map_err(|err| ExtError::Host(format!("取不到配置：{err}")))?;

    set_deadline(deadline, ACTIVATE_TIMEOUT);
    let activated = activate.call::<_, JsValue>((aidaw, config));
    clear_deadline(deadline);
    activated.map_err(|err| ExtError::Host(format!("activate() 失败：{}", js_error(ctx, err))))?;

    let state = state.lock().unwrap_or_else(|err| err.into_inner());
    Ok(LoadReport {
        tools: state.tools.clone(),
        events: state.events.clone(),
        contents: state.contents.clone(),
    })
}

fn call_tool(ctx: &rquickjs::Ctx<'_>, tool: &str, args: &str, deadline: &Arc<AtomicU64>) -> ExtResult<String> {
    let invoke: Function = ctx
        .globals()
        .get("__aidaw_invoke")
        .map_err(|err| ExtError::Host(format!("取不到调用器：{err}")))?;
    set_deadline(deadline, CALL_TIMEOUT);
    let output = invoke.call::<_, JsValue>((tool.to_string(), args.to_string()));
    clear_deadline(deadline);
    let output = output.map_err(|err| ExtError::Host(format!("工具「{tool}」执行失败：{}", js_error(ctx, err))))?;
    let text = output
        .as_string()
        .and_then(|text| text.to_string().ok())
        .ok_or_else(|| ExtError::Host(format!("工具「{tool}」没有返回字符串")))?;

    let envelope: Value = serde_json::from_str(&text)
        .map_err(|err| ExtError::Host(format!("工具「{tool}」返回的不是合法 JSON：{err}")))?;
    if envelope.get("__ok").and_then(Value::as_bool).unwrap_or(false) {
        let value = envelope.get("value").cloned().unwrap_or(Value::Null);
        Ok(serde_json::to_string(&value).unwrap_or_else(|_| "null".into()))
    } else {
        let message = envelope
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("未知错误")
            .to_string();
        Err(ExtError::Host(format!("工具「{tool}」报错：{message}")))
    }
}

fn dispatch_event(
    ctx: &rquickjs::Ctx<'_>,
    event: &str,
    payload: &str,
    bridge: &Arc<dyn Bridge>,
    plugin: &str,
) {
    let Ok(dispatch) = ctx.globals().get::<_, Function>("__aidaw_dispatch") else {
        return;
    };
    let _ = dispatch.call::<_, JsValue>((event.to_string(), payload.to_string()));
    if let Some(exception) = ctx.catch().as_exception() {
        bridge.log(plugin, "error", &format!("事件 {event} 的处理器抛错：{}", exception.message().unwrap_or_default()));
    }
}

fn deactivate(ctx: &rquickjs::Ctx<'_>, bridge: &Arc<dyn Bridge>, plugin: &str) {
    let Ok(exports) = ctx.globals().get::<_, JsValue>("__aidaw_module_exports") else {
        return;
    };
    let Some(deactivate) = exports
        .as_object()
        .and_then(|object| object.get::<_, JsValue>("deactivate").ok())
        .and_then(|value| value.into_function())
    else {
        return;
    };
    if let Err(err) = deactivate.call::<_, JsValue>(()) {
        bridge.log(plugin, "error", &format!("deactivate() 抛错：{}", js_error(ctx, err)));
    }
}

fn install_host(
    ctx: &rquickjs::Ctx<'_>,
    state: Arc<Mutex<HostState>>,
    bridge: Arc<dyn Bridge>,
    deadline: &Arc<AtomicU64>,
) -> ExtResult<()> {
    let host = Object::new(ctx.clone()).map_err(|err| ExtError::Host(err.to_string()))?;

    // register_tool：校验 + 收进队列；返回空串表示成功，否则是给作者看的中文错误。
    {
        let state = Arc::clone(&state);
        let func = Function::new(ctx.clone(), move |spec: String| -> String {
            let mut guard = state.lock().unwrap_or_else(|err| err.into_inner());
            register_tool(&mut guard, &spec)
        })
        .map_err(|err| ExtError::Host(err.to_string()))?;
        host.set("register_tool", func).map_err(|err| ExtError::Host(err.to_string()))?;
    }

    // call：同步调一条宿主命令，返回信封 JSON。
    {
        let bridge = Arc::clone(&bridge);
        let plugin = state.lock().unwrap_or_else(|err| err.into_inner()).plugin.clone();
        let deadline = Arc::clone(deadline);
        let func = Function::new(ctx.clone(), move |cmd: String, args: String| -> String {
            // 调用命令本身可能慢：把中断时限放宽到调用时限，避免 JS 侧误判超时。
            set_deadline(&deadline, CALL_TIMEOUT);
            let result = bridge.call(&plugin, &cmd, &args);
            clear_deadline(&deadline);
            match result {
                // 宿主返回的是 JSON 文本：**解回值再嵌入**，否则 JS 拿到的是字符串而不是对象。
                Ok(value) => match serde_json::from_str::<Value>(&value) {
                    Ok(parsed) => serde_json::json!({ "__ok": true, "value": parsed }).to_string(),
                    Err(_) => serde_json::json!({ "__ok": true, "value": value }).to_string(),
                },
                Err(err) => serde_json::json!({ "__ok": false, "error": err }).to_string(),
            }
        })
        .map_err(|err| ExtError::Host(err.to_string()))?;
        host.set("call", func).map_err(|err| ExtError::Host(err.to_string()))?;
    }

    // storage：三个小函数，都返回信封。
    {
        let get_state = Arc::clone(&state);
        let func = Function::new(ctx.clone(), move |key: String| -> String {
            let guard = get_state.lock().unwrap_or_else(|err| err.into_inner());
            match guard.storage.get(&key) {
                Some(value) => serde_json::json!({ "__ok": true, "value": value }).to_string(),
                None => serde_json::json!({ "__ok": true, "value": null }).to_string(),
            }
        })
        .map_err(|err| ExtError::Host(err.to_string()))?;
        host.set("storage_get", func).map_err(|err| ExtError::Host(err.to_string()))?;

        let set_state = Arc::clone(&state);
        let func = Function::new(ctx.clone(), move |key: String, value: String| -> String {
            let mut guard = set_state.lock().unwrap_or_else(|err| err.into_inner());
            if !guard.capabilities.contains(&Capability::Storage) {
                return "扩展没有声明 storage 能力".to_string();
            }
            match serde_json::from_str::<Value>(&value) {
                Ok(value) => {
                    guard.storage.set(key, value);
                    String::new()
                }
                Err(err) => format!("storage.set 的值不是合法 JSON：{err}"),
            }
        })
        .map_err(|err| ExtError::Host(err.to_string()))?;
        host.set("storage_set", func).map_err(|err| ExtError::Host(err.to_string()))?;

        let remove_state = Arc::clone(&state);
        let func = Function::new(ctx.clone(), move |key: String| -> String {
            let mut guard = remove_state.lock().unwrap_or_else(|err| err.into_inner());
            if !guard.capabilities.contains(&Capability::Storage) {
                return "扩展没有声明 storage 能力".to_string();
            }
            guard.storage.remove(&key);
            String::new()
        })
        .map_err(|err| ExtError::Host(err.to_string()))?;
        host.set("storage_remove", func).map_err(|err| ExtError::Host(err.to_string()))?;
    }

    // log
    {
        let bridge = Arc::clone(&bridge);
        let plugin = state.lock().unwrap_or_else(|err| err.into_inner()).plugin.clone();
        let func = Function::new(ctx.clone(), move |level: String, message: String| -> String {
            let level = match level.as_str() {
                "error" | "warn" | "info" | "debug" => level,
                _ => "info".to_string(),
            };
            bridge.log(&plugin, &level, &message);
            String::new()
        })
        .map_err(|err| ExtError::Host(err.to_string()))?;
        host.set("log", func).map_err(|err| ExtError::Host(err.to_string()))?;
    }

    // on：记录订阅（事件的 JS 处理器由 bootstrap 自己保存）。
    {
        let state = Arc::clone(&state);
        let func = Function::new(ctx.clone(), move |event: String| -> String {
            let mut guard = state.lock().unwrap_or_else(|err| err.into_inner());
            if !guard.capabilities.contains(&Capability::Events) {
                return "扩展没有声明 events 能力".to_string();
            }
            if !guard.events.iter().any(|item| item == &event) {
                guard.events.push(event);
            }
            String::new()
        })
        .map_err(|err| ExtError::Host(err.to_string()))?;
        host.set("on", func).map_err(|err| ExtError::Host(err.to_string()))?;
    }

    // register_content
    {
        let state = Arc::clone(&state);
        let func = Function::new(ctx.clone(), move |pack: String| -> String {
            let mut guard = state.lock().unwrap_or_else(|err| err.into_inner());
            if !guard.capabilities.contains(&Capability::Content) {
                return "扩展没有声明 content 能力".to_string();
            }
            match serde_json::from_str::<Value>(&pack) {
                Ok(value) => {
                    guard.contents.push(value);
                    String::new()
                }
                Err(err) => format!("内容包不是合法 JSON：{err}"),
            }
        })
        .map_err(|err| ExtError::Host(err.to_string()))?;
        host.set("register_content", func).map_err(|err| ExtError::Host(err.to_string()))?;
    }

    ctx.globals()
        .set("__aidaw_host", host)
        .map_err(|err| ExtError::Host(err.to_string()))?;
    Ok(())
}

fn register_tool(state: &mut HostState, spec_json: &str) -> String {
    if !state.capabilities.contains(&Capability::Tools) {
        return "扩展没有声明 tools 能力（aidaw.json 的 capabilities）".to_string();
    }
    let spec: Value = match serde_json::from_str(spec_json) {
        Ok(spec) => spec,
        Err(err) => return format!("工具声明不是合法 JSON：{err}"),
    };
    let name = spec.get("name").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    if !valid_tool_name(&name) {
        return format!("工具名「{name}」不合法：只允许小写字母、数字与下划线，1–64 个字符");
    }
    if state.tools.iter().any(|tool| tool.name == name) {
        return format!("工具「{name}」在本扩展里重复注册");
    }
    let description = spec
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if description.is_empty() {
        return format!("工具「{name}」缺少 description（模型要靠它决定什么时候用）");
    }
    let parameters = spec.get("parameters").cloned().unwrap_or_else(|| serde_json::json!({}));
    if !parameters.is_object() {
        return format!("工具「{name}」的 parameters 必须是 JSON Schema 对象");
    }
    let risk = spec
        .get("risk")
        .and_then(Value::as_str)
        .map(Risk::parse)
        .unwrap_or(Risk::Write);
    state.tools.push(ToolContribution {
        name,
        description,
        parameters,
        risk,
    });
    String::new()
}

fn valid_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

fn js_error(ctx: &rquickjs::Ctx<'_>, err: rquickjs::Error) -> String {
    if !matches!(err, rquickjs::Error::Exception) {
        return err.to_string();
    }
    let caught = ctx.catch();
    if let Some(exception) = caught.as_exception() {
        let message = exception.message().unwrap_or_else(|| "（无消息）".into());
        if message.contains("interrupted") {
            return "执行超时（扩展死循环，已被中断）".into();
        }
        return match exception.stack() {
            Some(stack) if !stack.trim().is_empty() => format!("{message}\n{stack}"),
            _ => message,
        };
    }
    caught
        .as_string()
        .and_then(|text| text.to_string().ok())
        .unwrap_or_else(|| "（抛出了一个非字符串值）".into())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn set_deadline(deadline: &Arc<AtomicU64>, timeout: Duration) {
    deadline.store(now_ms() + timeout.as_millis() as u64, Ordering::Relaxed);
}

fn clear_deadline(deadline: &Arc<AtomicU64>) {
    deadline.store(0, Ordering::Relaxed);
}

/// 扩展的私有存储：`<data>/extensions/<pkg>/storage.json`。
///
/// 每次写入都落盘（小 JSON，不是热路径），崩溃不会丢用户的配置。
pub struct Storage {
    path: PathBuf,
    map: BTreeMap<String, Value>,
}

impl Storage {
    /// 打开（不存在时为空）。
    pub fn open(path: PathBuf) -> Self {
        let map = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<BTreeMap<String, Value>>(&text).ok())
            .unwrap_or_default();
        Self { path, map }
    }

    /// 读。
    pub fn get(&self, key: &str) -> Option<Value> {
        self.map.get(key).cloned()
    }

    /// 写（立即落盘）。
    pub fn set(&mut self, key: String, value: Value) {
        self.map.insert(key, value);
        self.flush();
    }

    /// 删（立即落盘）。
    pub fn remove(&mut self, key: &str) {
        self.map.remove(key);
        self.flush();
    }

    fn flush(&self) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(&self.map) {
            let tmp = self.path.with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }
}

/// 注入到 JS 侧的引导脚本。
///
/// 这里放「作者体验」：`aidaw.registerTool(spec, fn)` 这种自然写法全在这层，Rust 只看见字符串。
const BOOTSTRAP: &str = r#"
(function () {
  "use strict";
  var host = globalThis.__aidaw_host;
  var plugin = globalThis.__aidaw_plugin_name;
  var handlers = Object.create(null);
  var listeners = Object.create(null);
  var pluginFn = null;

  var aidaw = {
    version: globalThis.__aidaw_app_version,
    pluginName: plugin,
    plugin: function (fn) {
      if (typeof fn !== "function") throw new Error("aidaw.plugin(fn) 需要一个函数");
      pluginFn = fn;
    },
    registerTool: function (spec, handler) {
      if (!spec || typeof spec !== "object") throw new Error("registerTool 需要一个声明对象");
      if (typeof handler !== "function") throw new Error("registerTool 的第二个参数必须是函数：" + String(spec.name));
      var error = host.register_tool(JSON.stringify(spec));
      if (error) throw new Error(error);
      handlers[spec.name] = handler;
    },
    call: function (cmd, args) {
      var envelope = JSON.parse(host.call(String(cmd), JSON.stringify(args === undefined ? {} : args)));
      if (!envelope.__ok) throw new Error(String(envelope.error || "命令失败"));
      return envelope.value;
    },
    storage: {
      get: function (key) {
        var envelope = JSON.parse(host.storage_get(String(key)));
        return envelope.__ok ? envelope.value : null;
      },
      set: function (key, value) {
        var error = host.storage_set(String(key), JSON.stringify(value));
        if (error) throw new Error(error);
      },
      remove: function (key) {
        var error = host.storage_remove(String(key));
        if (error) throw new Error(error);
      }
    },
    log: function (level, message) {
      host.log(String(level || "info"), String(message));
    },
    on: function (event, handler) {
      if (typeof handler !== "function") throw new Error("on 的第二个参数必须是函数");
      var error = host.on(String(event));
      if (error) throw new Error(error);
      (listeners[String(event)] || (listeners[String(event)] = [])).push(handler);
    },
    registerContent: function (pack) {
      var error = host.register_content(JSON.stringify(pack));
      if (error) throw new Error(error);
    }
  };

  globalThis.aidaw = aidaw;
  globalThis.__aidaw_plugin_fn = null;  // 由 aidaw.plugin 改写，见下面 defineProperty
  globalThis.__aidaw_config = JSON.parse(globalThis.__aidaw_config_json || "null");

  Object.defineProperty(globalThis, "__aidaw_plugin_fn", {
    get: function () { return pluginFn; },
    configurable: true
  });

  globalThis.__aidaw_invoke = function (name, argsJson) {
    try {
      var fn = handlers[name];
      if (typeof fn !== "function") {
        return JSON.stringify({ __ok: false, error: "没有这个工具的处理器：" + String(name) });
      }
      var value = fn(JSON.parse(argsJson || "{}"));
      return JSON.stringify({ __ok: true, value: value === undefined ? null : value });
    } catch (error) {
      var text = error && error.message ? String(error.message) : String(error);
      if (error && error.stack) text += "\n" + String(error.stack);
      return JSON.stringify({ __ok: false, error: text });
    }
  };

  globalThis.__aidaw_dispatch = function (event, payloadJson) {
    var list = listeners[event];
    if (!list) return;
    var payload = JSON.parse(payloadJson || "null");
    for (var index = 0; index < list.length; index += 1) {
      list[index](payload);
    }
  };
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::NoopBridge;

    fn start_js(source: &str, capabilities: &[Capability]) -> ExtResult<HostHandle> {
        let dir = tempfile::tempdir().unwrap();
        start(
            HostOptions {
                package: "@aidaw/test".into(),
                version: "0.1.0".into(),
                app_version: "1.9.0".into(),
                source: source.into(),
                config: serde_json::json!({ "greeting": "hi" }),
                capabilities: capabilities.to_vec(),
                storage_path: dir.path().join("storage.json"),
            },
            Arc::new(NoopBridge),
            "test".into(),
        )
    }

    #[test]
    fn registers_and_calls_a_tool() {
        let host = start_js(
            r#"
module.exports = {
  activate: function (aidaw, config) {
    aidaw.registerTool({ name: "echo", description: "回声", parameters: { type: "object" }, risk: "read" },
      function (args) { return { echo: args.text, greeting: config.greeting }; });
  }
};
"#,
            &[Capability::Tools],
        )
        .unwrap();
        assert_eq!(host.report.tools.len(), 1);
        assert_eq!(host.report.tools[0].name, "echo");
        let out = host.call("echo", r#"{"text":"yo"}"#).unwrap();
        let value: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["echo"], "yo");
        assert_eq!(value["greeting"], "hi");
        host.stop();
    }

    #[test]
    fn tool_errors_are_reported_not_fatal() {
        let host = start_js(
            r#"module.exports = { activate: function (aidaw) {
                aidaw.registerTool({ name: "boom", description: "炸", parameters: {} }, function () { throw new Error("坏了"); });
                aidaw.registerTool({ name: "ok", description: "好的", parameters: {} }, function () { return 1; });
            } };"#,
            &[Capability::Tools],
        )
        .unwrap();
        let err = host.call("boom", "{}").unwrap_err();
        assert!(err.to_string().contains("坏了"), "{err}");
        // 出错之后线程还活着，别的工具照常。
        assert_eq!(host.call("ok", "{}").unwrap(), "1");
        host.stop();
    }

    #[test]
    fn capability_violation_fails_the_load() {
        let err = start_js(
            r#"module.exports = { activate: function (aidaw) { aidaw.registerTool({ name: "x", description: "d", parameters: {} }, function () {}); } };"#,
            &[],
        )
        .unwrap_err();
        assert!(err.to_string().contains("tools"), "{err}");
    }

    #[test]
    fn syntax_error_fails_the_load() {
        let err = start_js("module.exports = { activate: function ( { ", &[Capability::Tools]).unwrap_err();
        assert!(err.to_string().contains("求值"), "{err}");
    }

    #[test]
    fn missing_activate_fails_the_load() {
        let err = start_js("module.exports = {};", &[Capability::Tools]).unwrap_err();
        assert!(err.to_string().contains("activate"), "{err}");
    }

    #[test]
    fn activate_timeout_is_reported() {
        let err = start_js(
            "module.exports = { activate: function () { while (true) {} } };",
            &[Capability::Tools],
        )
        .unwrap_err();
        assert!(err.to_string().contains("超时") || err.to_string().contains("装载"), "{err}");
    }

    #[test]
    fn storage_round_trips_between_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let options = || HostOptions {
            package: "@aidaw/store".into(),
            version: "0.1.0".into(),
            app_version: "1.9.0".into(),
            source: "module.exports = { activate: function (aidaw) { aidaw.storage.set('k', { n: 1 }); } };".into(),
            config: Value::Null,
            capabilities: vec![Capability::Storage],
            storage_path: dir.path().join("storage.json"),
        };
        let host = start(options(), Arc::new(NoopBridge), "t".into()).unwrap();
        host.stop();
        let host = start(options(), Arc::new(NoopBridge), "t".into()).unwrap();
        host.stop();
        let text = std::fs::read_to_string(dir.path().join("storage.json")).unwrap();
        assert!(text.contains("\"k\""), "{text}");
    }

    #[test]
    fn events_reach_the_plugin() {
        let host = start_js(
            r#"module.exports = { activate: function (aidaw) {
                aidaw.on("project_changed", function (payload) { aidaw.storage.set("last", payload); });
                aidaw.registerTool({ name: "read", description: "读", parameters: {} }, function () { return aidaw.storage.get("last"); });
            } };"#,
            &[Capability::Tools, Capability::Events, Capability::Storage],
        )
        .unwrap();
        assert_eq!(host.report.events, vec!["project_changed".to_string()]);
        host.dispatch_event("project_changed", r#"{"rev":7}"#);
        // 给线程一点时间处理消息（同一个通道，顺序保证；这里只是等它处理完）。
        std::thread::sleep(Duration::from_millis(50));
        let value: Value = serde_json::from_str(&host.call("read", "{}").unwrap()).unwrap();
        assert_eq!(value["rev"], 7);
        host.stop();
    }

    #[test]
    fn iife_style_plugin_is_supported() {
        let host = start_js(
            r#"aidaw.plugin(function (api) {
                api.registerTool({ name: "iife", description: "IIFE", parameters: {} }, function () { return "ok"; });
            });"#,
            &[Capability::Tools],
        )
        .unwrap();
        assert_eq!(host.call("iife", "{}").unwrap(), "\"ok\"");
        host.stop();
    }
}
