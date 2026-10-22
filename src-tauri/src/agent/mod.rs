//! 内置「写歌智能体」：直连 OpenAI 兼容 API，通过对话操纵 DAW。
//!
//! 由 Rust 实现 agent 循环。工具实现复用 [`crate::mcp::tools`]，写操作以 `Agent("chat")`
//! 署名——与 MCP 入口共用同一条撤销 / 事件管线，因此代理的每一步在界面上原样可见、可撤销。
//!
//! 前端是「会话的唯一视图」：后端把一轮对话产生的节点与增量经 `engine://agent` 推给前端，
//! 会话本身落盘在 `agent/sessions/<id>.json`。

mod config;
mod openai;
mod store;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;

use ai_daw_engine::event::Actor;

pub use config::{AgentConfig, AgentConfigRuntime};
pub use store::{Node, Session, SessionSummary};

use openai::{StreamEvent, ToolCall};
use store::{new_id, now_millis, title_from, SessionStore, ToolCallRef};

/// 智能体事件通道。
pub const EVENT_AGENT: &str = "engine://agent";

/// 流式增量的合并窗口（毫秒）。
///
/// 模型按 token 推送，长思考时每秒能冒出上百条。原样逐条 emit，前端每收一条就要把整棵
/// 会话树重渲染一遍（`v-html` 每次 patch 都重建 DOM），主线程会被压死——用户看到的是
/// 「点开思考就卡死、计时不走」。合并成窗口后事件量降一个数量级，且不丢字。
const DELTA_FLUSH_MS: u128 = 33;

/// 单条工具结果的截断上限（避免把巨大的渲染报告塞进上下文）。
const MAX_TOOL_OUTPUT: usize = 20_000;

/// 智能体的写操作署名。
fn chat_actor() -> Actor {
    Actor::Agent(crate::mcp::tools::CHAT_ACTOR.into())
}

/// 系统提示词：角色 + skills 索引（细则让模型自己调 `aidaw_help` 取，渐进式披露）。
fn system_prompt() -> String {
    let index = crate::mcp::SKILL_DOCS.first().map(|(_, text)| *text).unwrap_or("");
    format!(
        "你是内嵌在 AI DAW 里的编曲助手，通过工具操纵当前工程（轨道、片段、音符、混音、\
生成、分析等）。先用 get_project_overview 了解现状（轨道 / 乐器 / 片段 id 与音符数），\
逐音核对用 get_clip_notes；只有确实需要全量字段时才调 get_project（不含插件二进制状态）。\
需要细则时调用 aidaw_help 取对应能力域的文档。\
挑乐器或效果器前先调 list_plugins 读它的分类与音色描述（别凭插件名猜——`JC303` 其实是\
单音酸性贝斯），单个插件的参数表用 describe_plugin 查。\
所有改动都会进入撤销历史（署名 chat），用户可随时撤销，所以放手去做、必要时也可撤销/重做。\
音色记忆：默认绝不自动保存。只有用户明确要求（「记住这个音色」），或用户表达满意后\
你询问并得到确认，才调用 save_sound_memory；不要每次调参后都存。删除记忆必须用户明确\
要求。用户要某类音色时，先 list_sound_memories 查他存过的。\
回复用简洁的中文，不要复述工具的原始 JSON。\n\n--- 能力域索引 ---\n{index}"
    )
}

/// 供前端渲染的事件。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// 一轮开始：用户节点已建。
    TurnStarted { session_id: String, user_node: Node },
    /// 正文增量。
    Delta { node_id: String, text: String },
    /// 思考增量。
    Reasoning { node_id: String, text: String },
    /// 一次工具调用已发出。
    ToolCall { node_id: String, call_id: String, name: String, args: serde_json::Value },
    /// 工具返回。
    ToolResult { node_id: String, call_id: String, ok: bool, output: String },
    /// 一个节点已定稿（助手 / 工具）。
    NodeAdded { node: Node },
    /// 一轮结束。
    TurnFinished { session_id: String, head: String, title: String, stopped: bool },
    /// 出错（网络 / 端点 / 未配置等）。
    Error { message: String },
}

/// 运行时：单飞标志 + 取消令牌 + 会话存储。
#[derive(Clone)]
pub struct AgentRuntime {
    running: Arc<AtomicBool>,
    cancel: Arc<Mutex<Option<CancellationToken>>>,
    sessions: Arc<SessionStore>,
}

impl AgentRuntime {
    pub fn new(sessions_dir: Option<PathBuf>) -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(Mutex::new(None)),
            sessions: Arc::new(SessionStore::new(sessions_dir)),
        }
    }

    /// 供命令内部把运行时移交给后台任务。
    fn handle(&self) -> Self {
        self.clone()
    }
}

// ---------------------------------------------------------------------------
// 命令
// ---------------------------------------------------------------------------

/// 读取智能体配置。
#[tauri::command]
pub fn agent_config(runtime: State<'_, AgentConfigRuntime>) -> AgentConfig {
    runtime.get()
}

/// 保存智能体配置。
#[tauri::command]
pub fn agent_set_config(
    runtime: State<'_, AgentConfigRuntime>,
    base_url: String,
    api_key: String,
    model: String,
) -> AgentConfig {
    runtime.set(AgentConfig { base_url, api_key, model })
}

/// 拉取模型的 id 列表（用**传入的**配置，不写盘——便于测试尚未保存的值）。
#[tauri::command]
pub async fn agent_models(base_url: String, api_key: String, model: String) -> Result<Vec<String>, String> {
    openai::list_models(&AgentConfig { base_url, api_key, model }).await
}

/// 测试连通性（同样用传入的配置，不写盘）。
#[tauri::command]
pub async fn agent_test(base_url: String, api_key: String, model: String) -> Result<openai::AgentProbe, String> {
    Ok(openai::test_connection(&AgentConfig { base_url, api_key, model }).await)
}

/// 全部会话摘要。
#[tauri::command]
pub fn agent_sessions(runtime: State<'_, AgentRuntime>) -> Vec<SessionSummary> {
    runtime.sessions.list()
}

/// 载入一个会话（含完整节点树）。
#[tauri::command]
pub fn agent_session(runtime: State<'_, AgentRuntime>, id: String) -> Result<Session, String> {
    runtime.sessions.load(&id).ok_or_else(|| format!("会话不存在：{id}"))
}

/// 删除一个会话。
#[tauri::command]
pub fn agent_delete_session(runtime: State<'_, AgentRuntime>, id: String) {
    runtime.sessions.delete(&id);
}

/// 切换某会话的活跃分支（`head`）。
#[tauri::command]
pub fn agent_set_head(runtime: State<'_, AgentRuntime>, id: String, node_id: String) -> Result<Session, String> {
    let mut session = runtime.sessions.load(&id).ok_or_else(|| format!("会话不存在：{id}"))?;
    session.head = Some(node_id);
    session.updated_at = now_millis();
    runtime.sessions.save(&session)?;
    Ok(session)
}

/// 停止当前一轮。返回「停下之前是否真的有一轮在跑」，供前端判断要不要本地收尾。
#[tauri::command]
pub fn agent_cancel(runtime: State<'_, AgentRuntime>) -> bool {
    let was_running = runtime.running.load(Ordering::SeqCst);
    if let Some(token) = runtime.cancel.lock().unwrap().as_ref() {
        token.cancel();
    }
    was_running
}

/// 当前是否有轮次在跑（面板重新挂载后用它跟后端对齐状态）。
#[tauri::command]
pub fn agent_running(runtime: State<'_, AgentRuntime>) -> bool {
    runtime.running.load(Ordering::SeqCst)
}

/// 发起一轮对话：在 `parent_id` 下挂一条用户输入并开始回答。
///
/// `session_id` 为空时新建会话；`parent_id` 为空时接在会话当前 `head` 之后。
/// 「编辑某条用户输入后重答」= 传入被编辑节点的 `parent` 作为 `parent_id`（产生分支）。
#[tauri::command]
pub fn agent_send(
    app: AppHandle,
    runtime: State<'_, AgentRuntime>,
    config: State<'_, AgentConfigRuntime>,
    session_id: Option<String>,
    parent_id: Option<String>,
    text: String,
) -> Result<String, String> {
    if text.trim().is_empty() {
        return Err("输入为空".into());
    }
    if runtime.running.swap(true, Ordering::SeqCst) {
        return Err("智能体正在回答，请先停止".into());
    }
    let cfg = config.get();
    if !cfg.is_configured() {
        runtime.running.store(false, Ordering::SeqCst);
        return Err("尚未配置 OpenAI 兼容端点（基础 URL / API Key / 模型）".into());
    }

    let session = match session_id {
        Some(id) => match runtime.sessions.load(&id) {
            Some(s) => s,
            None => {
                runtime.running.store(false, Ordering::SeqCst);
                return Err(format!("会话不存在：{id}"));
            }
        },
        None => Session {
            id: new_id(),
            title: String::new(),
            created_at: now_millis(),
            updated_at: now_millis(),
            nodes: Vec::new(),
            head: None,
        },
    };
    let id = session.id.clone();

    let token = CancellationToken::new();
    *runtime.cancel.lock().unwrap() = Some(token.clone());

    let handle = app.clone();
    let rt = runtime.handle();
    tauri::async_runtime::spawn(run_turn(handle, rt, cfg, token, session, parent_id, text));
    Ok(id)
}

// ---------------------------------------------------------------------------
// 一轮对话
// ---------------------------------------------------------------------------

/// 工具定义 → OpenAI function 数组。
fn tool_defs_json() -> Vec<serde_json::Value> {
    crate::mcp::tools::definitions()
        .into_iter()
        .map(|d| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": d.name,
                    "description": d.description,
                    "parameters": d.parameters,
                }
            })
        })
        .collect()
}

/// 从 `head` 沿 `parent` 链还原 OpenAI 消息数组。
fn build_messages(session: &Session) -> Vec<serde_json::Value> {
    let mut out = vec![serde_json::json!({ "role": "system", "content": system_prompt() })];
    let mut path: Vec<&Node> = Vec::new();
    let mut cursor = session.head.clone();
    while let Some(id) = cursor {
        let Some(node) = session.nodes.iter().find(|n| n.id == id) else { break };
        path.push(node);
        cursor = node.parent.clone();
    }
    path.reverse();
    // 只认「有调用、也有结果」的工具对。取消 / 中断可能在助手节点里留下半截
    // 调用（没有对应的 tool 消息），原样发给模型会被 OpenAI 兼容端点判成非法消息。
    let called: std::collections::HashSet<&str> =
        path.iter().filter(|n| n.role == "assistant").flat_map(|n| n.tool_calls.iter().map(|c| c.id.as_str())).collect();
    let answered: std::collections::HashSet<&str> =
        path.iter().filter(|n| n.role == "tool").filter_map(|n| n.tool_call_id.as_deref()).collect();
    let paired = |id: &str| called.contains(id) && answered.contains(id);

    for node in path {
        match node.role.as_str() {
            "user" => out.push(serde_json::json!({ "role": "user", "content": node.content })),
            "assistant" => {
                let calls: Vec<&ToolCallRef> =
                    node.tool_calls.iter().filter(|c| paired(c.id.as_str())).collect();
                if calls.is_empty() {
                    // 没有可用工具调用的节点退化成普通助手消息；连正文都没有就跳过。
                    if !node.content.is_empty() {
                        out.push(serde_json::json!({ "role": "assistant", "content": node.content }));
                    }
                } else {
                    let calls: Vec<serde_json::Value> = calls
                        .iter()
                        .map(|c| {
                            serde_json::json!({
                                "id": c.id,
                                "type": "function",
                                "function": {
                                    "name": c.name,
                                    "arguments": if c.arguments.trim().is_empty() { "{}" } else { c.arguments.as_str() },
                                }
                            })
                        })
                        .collect();
                    let content = if node.content.is_empty() {
                        serde_json::Value::Null
                    } else {
                        serde_json::Value::String(node.content.clone())
                    };
                    out.push(serde_json::json!({ "role": "assistant", "content": content, "tool_calls": calls }));
                }
            }
            "tool" => {
                let Some(id) = node.tool_call_id.as_deref() else { continue };
                if !paired(id) {
                    continue;
                }
                out.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": id,
                    "content": node.content,
                }));
            }
            _ => {}
        }
    }
    out
}

fn truncate_output(text: String) -> String {
    if text.chars().count() <= MAX_TOOL_OUTPUT {
        return text;
    }
    let head: String = text.chars().take(MAX_TOOL_OUTPUT).collect();
    format!(
        "{head}\n…（输出过长已截断。缩小范围再试：工程结构用 get_project_overview，\
         音符用 get_clip_notes(track, clip, offset, limit)，插件用 list_plugins(query=…)）"
    )
}

/// 执行一轮对话：追加用户节点 → 反复「流式回答 → 执行工具」直到不再有工具调用 → 落盘。
async fn run_turn(
    app: AppHandle,
    rt: AgentRuntime,
    cfg: AgentConfig,
    token: CancellationToken,
    mut session: Session,
    parent: Option<String>,
    text: String,
) {
    let actor = chat_actor();

    // 1. 用户节点。
    let user_node = Node {
        id: new_id(),
        parent: parent.clone(),
        role: "user".into(),
        content: text.clone(),
        reasoning: None,
        tool_calls: Vec::new(),
        tool_call_id: None,
        name: None,
        created_at: now_millis(),
        error: None,
    };
    session.nodes.push(user_node.clone());
    if session.title.trim().is_empty() {
        session.title = title_from(&text);
    }
    session.head = Some(user_node.id.clone());
    session.updated_at = now_millis();
    let _ = app.emit(EVENT_AGENT, AgentEvent::TurnStarted { session_id: session.id.clone(), user_node });
    let _ = rt.sessions.save(&session);

    let tools = tool_defs_json();

    // 2. 循环。
    loop {
        if token.is_cancelled() {
            break;
        }
        let messages = build_messages(&session);
        let asst_id = new_id();
        let mut content = String::new();
        let mut reasoning = String::new();

        // 增量按窗口合并：攒到的文字先缓冲，窗口到期（或这条流结束）再整段 emit。
        let mut pending_text = String::new();
        let mut pending_reasoning = String::new();
        let mut last_emit = Instant::now();

        let emit_app = app.clone();
        let emit_id = asst_id.clone();
        let result = openai::stream_chat(&cfg, &messages, &tools, &token, |ev| match ev {
            StreamEvent::Content(t) => {
                content.push_str(&t);
                pending_text.push_str(&t);
                if last_emit.elapsed().as_millis() >= DELTA_FLUSH_MS {
                    last_emit = Instant::now();
                    let _ = emit_app.emit(
                        EVENT_AGENT,
                        AgentEvent::Delta { node_id: emit_id.clone(), text: std::mem::take(&mut pending_text) },
                    );
                }
            }
            StreamEvent::Reasoning(t) => {
                reasoning.push_str(&t);
                pending_reasoning.push_str(&t);
                if last_emit.elapsed().as_millis() >= DELTA_FLUSH_MS {
                    last_emit = Instant::now();
                    let _ = emit_app.emit(
                        EVENT_AGENT,
                        AgentEvent::Reasoning { node_id: emit_id.clone(), text: std::mem::take(&mut pending_reasoning) },
                    );
                }
            }
        })
        .await;

        // 尾巴必须落地：否则最后一段文字只存在于内存里的 `content`，前端看不到。
        if !pending_text.is_empty() {
            let _ = app.emit(
                EVENT_AGENT,
                AgentEvent::Delta { node_id: asst_id.clone(), text: std::mem::take(&mut pending_text) },
            );
        }
        if !pending_reasoning.is_empty() {
            let _ = app.emit(
                EVENT_AGENT,
                AgentEvent::Reasoning { node_id: asst_id.clone(), text: std::mem::take(&mut pending_reasoning) },
            );
        }

        let calls = match result {
            Ok(calls) => calls,
            Err(err) => {
                let node = Node {
                    id: asst_id.clone(),
                    parent: session.head.clone(),
                    role: "assistant".into(),
                    content,
                    reasoning: non_empty(reasoning),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    name: None,
                    created_at: now_millis(),
                    error: Some(err.clone()),
                };
                session.nodes.push(node.clone());
                session.head = Some(asst_id);
                session.updated_at = now_millis();
                let _ = app.emit(EVENT_AGENT, AgentEvent::Error { message: err });
                let _ = app.emit(EVENT_AGENT, AgentEvent::NodeAdded { node });
                break;
            }
        };
        // 用户中途停止时，流里可能只攒到半个工具调用：别把它当成一次真实调用
        // 落盘（否则下一条消息会带着没有结果的 tool_calls，被端点拒绝）。
        let calls = if token.is_cancelled() { Vec::new() } else { calls };

        let node = Node {
            id: asst_id.clone(),
            parent: session.head.clone(),
            role: "assistant".into(),
            content,
            reasoning: non_empty(reasoning),
            tool_calls: calls
                .iter()
                .map(|c| ToolCallRef { id: c.id.clone(), name: c.name.clone(), arguments: c.arguments.clone() })
                .collect(),
            tool_call_id: None,
            name: None,
            created_at: now_millis(),
            error: None,
        };
        session.nodes.push(node.clone());
        session.head = Some(asst_id.clone());
        session.updated_at = now_millis();
        let _ = app.emit(EVENT_AGENT, AgentEvent::NodeAdded { node });
        let _ = rt.sessions.save(&session);

        if calls.is_empty() {
            break;
        }
        if token.is_cancelled() {
            break;
        }

        // 3. 执行工具。
        for call in calls {
            if token.is_cancelled() {
                break;
            }
            let args = parse_call_args(&call);
            let _ = app.emit(
                EVENT_AGENT,
                AgentEvent::ToolCall {
                    node_id: asst_id.clone(),
                    call_id: call.id.clone(),
                    name: call.name.clone(),
                    args: args.clone(),
                },
            );
            // 工具跑在阻塞线程池里（渲染 / 分析可能几十秒且无法打断）。这里**不等**它跑完：
            // 用户一按停止就立刻收尾，交给 blocker 自己在后台跑完、结果丢弃。
            let (ok, output) = tokio::select! {
                _ = token.cancelled() => break,
                out = dispatch_tool(&app, &actor, &call, args) => out,
            };
            let _ = app.emit(
                EVENT_AGENT,
                AgentEvent::ToolResult {
                    node_id: asst_id.clone(),
                    call_id: call.id.clone(),
                    ok,
                    output: output.clone(),
                },
            );
            let tool_node = Node {
                id: new_id(),
                parent: session.head.clone(),
                role: "tool".into(),
                content: truncate_output(output.clone()),
                reasoning: None,
                tool_calls: Vec::new(),
                tool_call_id: Some(call.id.clone()),
                name: Some(call.name.clone()),
                created_at: now_millis(),
                // 失败时把原因也留在 `error` 上，前端据此把卡片标红。
                error: if ok { None } else { Some(output) },
            };
            session.nodes.push(tool_node.clone());
            session.head = Some(tool_node.id.clone());
            session.updated_at = now_millis();
            let _ = app.emit(EVENT_AGENT, AgentEvent::NodeAdded { node: tool_node });
            let _ = rt.sessions.save(&session);
        }
    }

    // 4. 收尾。
    let stopped = token.is_cancelled();
    session.updated_at = now_millis();
    let _ = rt.sessions.save(&session);
    let head = session.head.clone().unwrap_or_default();
    // 先把「不在跑」落地，再通知前端：否则用户看到 `turn_finished` 立刻发下一条时，
    // `agent_send` 可能还撞上 `running == true`。取消令牌留在运行时里不清掉——它属于
    // **这一轮**，`take()` 会误拿走下一轮刚装上的新令牌并把它取消。
    rt.running.store(false, Ordering::SeqCst);
    let _ = app.emit(
        EVENT_AGENT,
        AgentEvent::TurnFinished {
            session_id: session.id.clone(),
            head,
            title: session.title.clone(),
            stopped,
        },
    );
}

/// 解析工具调用参数（失败当作空对象，交给工具自己报参数错误）。
fn parse_call_args(call: &ToolCall) -> serde_json::Value {
    if call.arguments.trim().is_empty() {
        return serde_json::json!({});
    }
    serde_json::from_str(&call.arguments).unwrap_or_else(|_| serde_json::json!({}))
}

/// 到阻塞线程池里执行工具。
async fn dispatch_tool(
    app: &AppHandle,
    actor: &Actor,
    call: &ToolCall,
    args: serde_json::Value,
) -> (bool, String) {
    let app = app.clone();
    let actor = actor.clone();
    let name = call.name.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::mcp::tools::dispatch(&app, &actor, &name, args)
    })
    .await;
    match result {
        Ok(Ok(out)) => (true, out),
        Ok(Err(err)) => (false, err),
        Err(err) => (false, format!("工具执行线程失败：{err}")),
    }
}

/// 空串 → `None`。
fn non_empty(s: String) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use store::ToolCallRef;

    fn node(id: &str, parent: Option<&str>, role: &str) -> Node {
        Node {
            id: id.into(),
            parent: parent.map(str::to_owned),
            role: role.into(),
            content: String::new(),
            reasoning: None,
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
            created_at: 0,
            error: None,
        }
    }

    fn call(id: &str) -> ToolCallRef {
        ToolCallRef { id: id.into(), name: "add_note".into(), arguments: "{}".into() }
    }

    fn session(nodes: Vec<Node>, head: &str) -> Session {
        Session {
            id: "s".into(),
            title: "t".into(),
            created_at: 0,
            updated_at: 0,
            nodes,
            head: Some(head.into()),
        }
    }

    #[test]
    fn build_messages_drops_tool_calls_without_results() {
        // 助手节点里有两个调用，但只有第一个有结果（取消 / 中断的典型形态）。
        let mut asst = node("a", Some("u"), "assistant");
        asst.content = "写两个音符。".into();
        asst.tool_calls = vec![call("c1"), call("c2")];
        let mut tool = node("t", Some("a"), "tool");
        tool.tool_call_id = Some("c1".into());
        tool.content = "ok".into();
        let s = session(vec![node("u", None, "user"), asst, tool], "t");

        let msgs = build_messages(&s);
        // system + user + assistant + tool
        assert_eq!(msgs.len(), 4);
        let calls = msgs[2]["tool_calls"].as_array().expect("tool_calls");
        assert_eq!(calls.len(), 1, "orphan call must be dropped: {msgs:#?}");
        assert_eq!(calls[0]["id"], "c1");
        assert_eq!(msgs[3]["tool_call_id"], "c1");
    }

    #[test]
    fn build_messages_keeps_plain_content_when_all_calls_orphaned() {
        let mut asst = node("a", Some("u"), "assistant");
        asst.content = "我先想一下。".into();
        asst.tool_calls = vec![call("c1")];
        let s = session(vec![node("u", None, "user"), asst], "a");

        let msgs = build_messages(&s);
        assert_eq!(msgs.len(), 3);
        assert!(msgs[2].get("tool_calls").is_none(), "no dangling call: {msgs:#?}");
        assert_eq!(msgs[2]["content"], "我先想一下。");
    }

    #[test]
    fn build_messages_drops_empty_assistant_without_calls() {
        let mut asst = node("a", Some("u"), "assistant");
        asst.tool_calls = vec![call("c1")];
        let s = session(vec![node("u", None, "user"), asst], "a");

        let msgs = build_messages(&s);
        assert_eq!(msgs.len(), 2, "empty assistant step should disappear: {msgs:#?}");
    }
}
