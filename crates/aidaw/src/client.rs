//! 极小的 MCP 客户端：只覆盖 `initialize` / `tools/list` / `tools/call` 三条消息。
//!
//! 只连本机回环地址，故不需要 TLS。rmcp 的 Streamable HTTP 服务器默认以 **SSE** 回请求，
//! 所以这里**流式**读响应：拿到与请求 id 匹配的那条消息就停，不等连接关闭——SSE 是长连接，
//! 读到底会把调用一直挂住。

use std::io::{BufRead, BufReader};
use std::time::Duration;

use serde_json::{json, Value};

/// 握手时上报的客户端名。
const CLIENT_NAME: &str = "aidaw";
/// 我们支持的 MCP 协议版本（与 `src-tauri` 所用 rmcp 一致）。
const PROTOCOL_VERSION: &str = "2025-06-18";
/// 会话头：`initialize` 的响应里给出，之后每个请求都要带上（rmcp 默认的 legacy 会话模式）。
const SESSION_HEADER: &str = "Mcp-Session-Id";
/// 协议版本头（2025-06-18 规范引入）。
const VERSION_HEADER: &str = "MCP-Protocol-Version";
/// 常规请求超时：渲染 / 导出这类工具会跑很久，给足。
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
/// 探测「软件在不在」用的短超时。
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// `initialize` 的请求 id。
const INIT_ID: i64 = 0;
/// `tools/list` 的请求 id。
const LIST_ID: i64 = 1;
/// `tools/call` 的请求 id。
const CALL_ID: i64 = 2;

/// 一个工具的名字与描述。
pub struct ToolInfo {
    pub name: String,
    pub description: String,
}

/// `tools/call` 的结果。
pub struct CallOutcome {
    /// 工具自己报的失败（结果里的 `isError`）。
    pub is_error: bool,
    /// 汇成一段文本的内容。
    pub text: String,
}

/// 一次 HTTP 往返的结果。
struct Reply {
    status: u16,
    session: Option<String>,
    /// 解析出的 JSON-RPC 消息（通知类响应没有正文时为 `None`）。
    message: Option<Value>,
    /// 原始正文，仅在报错时用来带出服务器原文。
    raw: String,
}

/// MCP 薄客户端：持有连接池与当前会话 id。
pub struct McpClient {
    agent: ureq::Agent,
    url: String,
    session: Option<String>,
}

impl McpClient {
    /// 连接并完成握手；成功即代表软件在运行且 MCP 可用。
    pub fn connect(url: &str) -> Result<Self, String> {
        let mut client = Self {
            agent: agent(DEFAULT_TIMEOUT),
            url: url.to_owned(),
            session: None,
        };
        client.handshake()?;
        Ok(client)
    }

    /// 只做一次握手，用来判断该地址是否可用（短超时，不抛错）。
    pub fn probe(url: &str) -> bool {
        let mut client = Self {
            agent: agent(PROBE_TIMEOUT),
            url: url.to_owned(),
            session: None,
        };
        client.handshake().is_ok()
    }

    /// 列出软件当前暴露的全部工具。
    pub fn list_tools(&self) -> Result<Vec<ToolInfo>, String> {
        let result = self.rpc("tools/list", json!({}), LIST_ID)?;
        let tools = result
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(tools
            .into_iter()
            .filter_map(|tool| {
                let name = tool.get("name")?.as_str()?.to_owned();
                let description = tool
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                Some(ToolInfo { name, description })
            })
            .collect())
    }

    /// 调用一个工具，参数是 JSON 对象。
    pub fn call_tool(&self, tool: &str, arguments: Value) -> Result<CallOutcome, String> {
        let result = self.rpc(
            "tools/call",
            json!({ "name": tool, "arguments": arguments }),
            CALL_ID,
        )?;
        let is_error = result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Ok(CallOutcome {
            is_error,
            text: collect_text(&result),
        })
    }

    /// 握手：`initialize` + `notifications/initialized`。
    fn handshake(&mut self) -> Result<(), String> {
        let reply = self.post(
            &json!({
                "jsonrpc": "2.0",
                "id": INIT_ID,
                "method": "initialize",
                "params": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": CLIENT_NAME, "version": env!("CARGO_PKG_VERSION") },
                },
            }),
            Some(INIT_ID),
        )?;
        if reply.status >= 400 {
            return Err(format!("握手失败：HTTP {}", reply.status));
        }
        if let Some(error) = reply.message.as_ref().and_then(|m| m.get("error")) {
            return Err(format!("握手被拒绝：{}", compact(error)));
        }
        self.session = reply.session;
        // 握手完成后必须通知服务器，会话才进入可用状态。
        self.post(
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            None,
        )?;
        Ok(())
    }

    /// 发一条带 id 的请求，返回 `result`（出错时转成人类可读的消息）。
    fn rpc(&self, method: &str, params: Value, id: i64) -> Result<Value, String> {
        let reply = self.post(
            &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
            Some(id),
        )?;
        if reply.status >= 400 {
            return Err(format!(
                "服务器返回 HTTP {}：{}",
                reply.status,
                truncate(reply.raw.trim())
            ));
        }
        let message = reply
            .message
            .ok_or_else(|| "服务器没有返回消息".to_owned())?;
        if let Some(error) = message.get("error") {
            return Err(format!("MCP 错误：{}", compact(error)));
        }
        Ok(message.get("result").cloned().unwrap_or(Value::Null))
    }

    /// 发一次 POST；`expect_id` 指定要在 SSE 流里等的响应 id。
    fn post(&self, body: &Value, expect_id: Option<i64>) -> Result<Reply, String> {
        let mut request = self
            .agent
            .post(&self.url)
            .header("Content-Type", "application/json")
            // rmcp 要求同时接受这两种类型。
            .header("Accept", "application/json, text/event-stream")
            .header(VERSION_HEADER, PROTOCOL_VERSION);
        if let Some(session) = &self.session {
            request = request.header(SESSION_HEADER, session);
        }
        let mut response = request
            .send(body.to_string())
            .map_err(|e| format!("请求 {} 失败：{e}", self.url))?;

        let (status, session, content_type) = {
            let headers = response.headers();
            (
                response.status().as_u16(),
                headers
                    .get(SESSION_HEADER)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned),
                headers
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default()
                    .to_owned(),
            )
        };

        let (message, raw) = if content_type.contains("text/event-stream") {
            let mut reader = BufReader::new(response.body_mut().as_reader());
            (read_sse(&mut reader, expect_id)?, String::new())
        } else {
            let text = response
                .body_mut()
                .read_to_string()
                .map_err(|e| format!("读取响应失败：{e}"))?;
            let message =
                if text.trim().is_empty() {
                    None
                } else {
                    Some(serde_json::from_str(&text).map_err(|e| {
                        format!("响应不是合法 JSON：{e}（原文：{}）", truncate(&text))
                    })?)
                };
            (message, text)
        };

        Ok(Reply {
            status,
            session,
            message,
            raw,
        })
    }
}

/// 建一个连接器：定超时、自己看状态码、不经代理（只连本机）。
fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false) // 自己看状态码，好把服务器的报错原文带出来
        .proxy(None) // 只连本机，别被 HTTP_PROXY 带偏
        .build()
        .into()
}

/// 从 SSE 流里读出一条 `data:` 消息；`expect_id` 用于指名要哪条响应。
///
/// 找到匹配的消息就返回（不等流结束）；流先结束则交出手里最后一段。
fn read_sse<R: BufRead>(reader: &mut R, expect_id: Option<i64>) -> Result<Option<Value>, String> {
    let mut data = String::new();
    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .map_err(|e| format!("读取响应流失败：{e}"))?;
        if read == 0 {
            // 流结束：把还没成事件的一段 data 交出去。
            return finish_event(&data, expect_id);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            // 空行 = 一个事件结束。
            if let Some(value) = finish_event(&data, expect_id)? {
                return Ok(Some(value));
            }
            data.clear();
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
        // 其它字段（event / id / retry / 注释）一概忽略。
    }
}

/// 解析一条 SSE 事件的 `data`；只在 id 匹配（或未指定 id）时返回。
fn finish_event(data: &str, expect_id: Option<i64>) -> Result<Option<Value>, String> {
    let data = data.trim();
    if data.is_empty() {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(data).map_err(|e| format!("响应不是合法 JSON：{e}"))?;
    let matched = match expect_id {
        Some(id) => value.get("id").and_then(Value::as_i64) == Some(id),
        None => true,
    };
    Ok(matched.then_some(value))
}

/// 把工具结果里的文本内容汇成一段；没有文本就退回整个结果的 JSON。
fn collect_text(result: &Value) -> String {
    let texts: Vec<String> = result
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
                .map(|item| {
                    item.get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                })
                .collect()
        })
        .unwrap_or_default();
    if texts.is_empty() {
        compact(result)
    } else {
        texts.join("\n")
    }
}

/// 紧凑 JSON（单行）。
fn compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

/// 报错时带出服务器原文：过长就截断，避免刷屏。
fn truncate(text: &str) -> String {
    const MAX: usize = 400;
    if text.chars().count() <= MAX {
        return text.to_owned();
    }
    let head: String = text.chars().take(MAX).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;

    /// 桩服务器下发的会话 id。
    const SESSION: &str = "test-session";

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    /// 读一条 HTTP/1.1 请求，返回 (JSON-RPC 正文, 请求里的会话头)。
    fn read_request(stream: &mut TcpStream) -> Option<(Value, Option<String>)> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        let header_end = loop {
            let read = stream.read(&mut chunk).ok()?;
            if read == 0 {
                return None;
            }
            buf.extend_from_slice(&chunk[..read]);
            if let Some(pos) = find(&buf, b"\r\n\r\n") {
                break pos + 4;
            }
        };
        // 头名大小写不敏感：统一小写后再匹配。
        let head = String::from_utf8_lossy(&buf[..header_end]).to_ascii_lowercase();
        let mut content_length = 0usize;
        let mut session = None;
        for line in head.split("\r\n") {
            if let Some(value) = line.strip_prefix("content-length:") {
                content_length = value.trim().parse().unwrap_or(0);
            }
            if let Some(value) = line.strip_prefix("mcp-session-id:") {
                session = Some(value.trim().to_owned());
            }
        }
        while buf.len() < header_end + content_length {
            let read = stream.read(&mut chunk).ok()?;
            if read == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..read]);
        }
        let body = &buf[header_end..(header_end + content_length).min(buf.len())];
        Some((serde_json::from_slice(body).ok()?, session))
    }

    /// 写一条响应（带 Content-Length，靠 `Connection: close` 收尾）。
    fn respond(
        stream: &mut TcpStream,
        status: &str,
        content_type: Option<&str>,
        session: Option<&str>,
        body: &str,
    ) {
        let mut head = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        if let Some(content_type) = content_type {
            head.push_str(&format!("Content-Type: {content_type}\r\n"));
        }
        if let Some(session) = session {
            head.push_str(&format!("Mcp-Session-Id: {session}\r\n"));
        }
        head.push_str("\r\n");
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(body.as_bytes());
        let _ = stream.flush();
    }

    /// SSE 正文：一条注释 + 一条 data（逼客户端忽略非 data 行）。
    fn sse(payload: &str) -> String {
        format!(": priming\r\n\r\ndata: {payload}\r\n\r\n")
    }

    /// 桩服务器：按 MCP 的次序回应 initialize / initialized / tools/list / tools/call。
    fn spawn_stub() -> (String, mpsc::Receiver<(String, Option<String>)>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定回环临时端口");
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let Some((message, session)) = read_request(&mut stream) else {
                    continue;
                };
                let method = message
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let id = message.get("id").and_then(Value::as_i64).unwrap_or(0);
                let _ = tx.send((method.clone(), session));
                let payload = match method.as_str() {
                    "initialize" => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "protocolVersion": "2025-06-18",
                            "capabilities": { "tools": {} },
                            "serverInfo": { "name": "stub", "version": "0" },
                        },
                    }),
                    "notifications/initialized" => {
                        respond(&mut stream, "202 Accepted", None, None, "");
                        continue;
                    }
                    "tools/list" => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "tools": [
                                { "name": "get_project", "description": "读工程" },
                                { "name": "set_tempo", "description": "设速度" },
                            ],
                        },
                    }),
                    "tools/call" => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [ { "type": "text", "text": "速度已设为 120 BPM" } ],
                            "isError": false,
                        },
                    }),
                    _ => {
                        respond(&mut stream, "404 Not Found", None, None, "unknown method");
                        continue;
                    }
                };
                let session = if method == "initialize" {
                    Some(SESSION)
                } else {
                    None
                };
                respond(
                    &mut stream,
                    "200 OK",
                    Some("text/event-stream"),
                    session,
                    &sse(&payload.to_string()),
                );
            }
        });
        (format!("http://{addr}/mcp"), rx)
    }

    #[test]
    fn walks_the_full_mcp_handshake_and_calls() {
        let (url, requests) = spawn_stub();
        let client = McpClient::connect(&url).expect("应能连上桩服务器");

        let tools = client.list_tools().expect("应能列出工具");
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "get_project");
        assert_eq!(tools[0].description, "读工程");

        let outcome = client
            .call_tool("set_tempo", json!({ "bpm": 120 }))
            .expect("应能调用工具");
        assert!(!outcome.is_error);
        assert_eq!(outcome.text, "速度已设为 120 BPM");

        // 请求次序、「会话 id 只在握手之后带上」是协议契约，逐条校验。
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.push(
                requests
                    .recv_timeout(Duration::from_secs(5))
                    .expect("应收到 4 个请求"),
            );
        }
        assert_eq!(seen[0], ("initialize".to_owned(), None));
        assert_eq!(
            seen[1],
            (
                "notifications/initialized".to_owned(),
                Some(SESSION.to_owned())
            )
        );
        assert_eq!(seen[2], ("tools/list".to_owned(), Some(SESSION.to_owned())));
        assert_eq!(seen[3], ("tools/call".to_owned(), Some(SESSION.to_owned())));
    }

    #[test]
    fn sse_reader_waits_for_the_matching_id() {
        let body = concat!(
            ": priming\r\n\r\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":99,\"result\":{}}\r\n\r\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\r\n\r\n",
        );
        let mut reader = BufReader::new(body.as_bytes());
        let message = read_sse(&mut reader, Some(7))
            .expect("解析 SSE")
            .expect("应拿到 id=7 的消息");
        assert_eq!(message.get("id"), Some(&json!(7)));
    }

    #[test]
    fn probe_reports_a_dead_endpoint() {
        // 绑一个端口再立刻释放，得到一个确定没人监听的地址。
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        assert!(!McpClient::probe(&format!("http://{addr}/mcp")));
    }
}
