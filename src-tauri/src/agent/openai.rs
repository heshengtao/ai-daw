//! OpenAI 兼容客户端：模型列表、连通性探测、以及流式 Chat Completions（含工具调用）。
//!
//! 只依赖标准 OpenAI 形状的 HTTP 接口：`GET {base}/models`、`POST {base}/chat/completions`。
//! 流式解析自己实现（`data:` 行按空行分帧、`[DONE]` 结束），因为不同兼容网关在 SSE 细节上
//! 有出入，自己控更稳。

use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::config::AgentConfig;

/// 连通性探测结果（形状对齐 `plugin_manager_probe_proxy`）。
#[derive(Debug, Clone, Serialize)]
pub struct AgentProbe {
    /// 是否真的连上了。
    pub ok: bool,
    /// 成功=延迟说明，失败=原因。
    pub detail: String,
    /// 耗时（毫秒）。
    pub ms: u64,
}

/// 一次工具调用（流式累积后）。
#[derive(Debug, Clone, Serialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// 原始 arguments JSON 文本。
    pub arguments: String,
}

/// 流式回调里的事件。
pub enum StreamEvent {
    /// 正文增量。
    Content(String),
    /// 思考（reasoning）增量。
    Reasoning(String),
}

/// 把用户填的地址规整成「根」：去掉结尾斜杠，并容忍直接粘贴完整端点。
fn normalize_base(base_url: &str) -> String {
    let mut s = base_url.trim().trim_end_matches('/').to_string();
    for suffix in ["/chat/completions", "/models", "/completions"] {
        if let Some(stripped) = s.strip_suffix(suffix) {
            s = stripped.trim_end_matches('/').to_string();
            break;
        }
    }
    s
}

fn client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder().timeout(timeout).build().map_err(|e| e.to_string())
}

/// 截断过长的响应体，避免把整页 HTML 塞进错误信息。
fn brief(text: &str) -> String {
    let t = text.trim();
    if t.chars().count() <= 400 {
        return t.to_string();
    }
    t.chars().take(400).collect::<String>() + "…"
}

/// `GET {base}/models` → 模型 id 列表。
pub async fn list_models(cfg: &AgentConfig) -> Result<Vec<String>, String> {
    let root = normalize_base(&cfg.base_url);
    if root.is_empty() {
        return Err("尚未填写基础 URL".into());
    }
    let url = format!("{root}/models");
    let resp = client(Duration::from_secs(30))?
        .get(&url)
        .bearer_auth(&cfg.api_key)
        .send()
        .await
        .map_err(|e| format!("请求失败：{e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("HTTP {status}：{}", brief(&text)));
    }
    #[derive(Deserialize)]
    struct Models {
        #[serde(default)]
        data: Vec<ModelEntry>,
    }
    #[derive(Deserialize)]
    struct ModelEntry {
        id: String,
    }
    let parsed: Models =
        serde_json::from_str(&text).map_err(|e| format!("解析模型列表失败：{e}；响应：{}", brief(&text)))?;
    let mut ids: Vec<String> = parsed.data.into_iter().map(|m| m.id).collect();
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Err("端点没有返回任何模型".into());
    }
    Ok(ids)
}

/// 探测连通性：先试 `GET /models`，不行再用一条最短的 `POST /chat/completions` 兜底。
pub async fn test_connection(cfg: &AgentConfig) -> AgentProbe {
    let started = Instant::now();
    match list_models(cfg).await {
        Ok(models) => AgentProbe {
            ok: true,
            detail: format!("连接成功，发现 {} 个模型", models.len()),
            ms: started.elapsed().as_millis() as u64,
        },
        Err(list_err) => match ping_chat(cfg).await {
            Ok(detail) => AgentProbe { ok: true, detail, ms: started.elapsed().as_millis() as u64 },
            Err(chat_err) => AgentProbe {
                ok: false,
                detail: format!("模型列表：{list_err}；对话接口：{chat_err}"),
                ms: started.elapsed().as_millis() as u64,
            },
        },
    }
}

/// 最短的一次对话请求，用于探测「没有 /models 的网关」。
async fn ping_chat(cfg: &AgentConfig) -> Result<String, String> {
    let root = normalize_base(&cfg.base_url);
    if root.is_empty() {
        return Err("尚未填写基础 URL".into());
    }
    if cfg.model.trim().is_empty() {
        return Err("尚未填写模型名".into());
    }
    let url = format!("{root}/chat/completions");
    let body = serde_json::json!({
        "model": cfg.model,
        "messages": [{ "role": "user", "content": "ping" }],
        "max_tokens": 1,
        "stream": false,
    });
    let resp = client(Duration::from_secs(30))?
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求失败：{e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if status.is_success() {
        Ok(format!("对话接口可用（模型 {}）", cfg.model))
    } else {
        Err(format!("HTTP {status}：{}", brief(&text)))
    }
}

// ---------------------------------------------------------------------------
// 流式对话
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct Chunk {
    #[serde(default)]
    choices: Vec<ChunkChoice>,
}

#[derive(Deserialize, Default)]
struct ChunkChoice {
    #[serde(default)]
    delta: Delta,
}

#[derive(Deserialize, Default)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    /// DeepSeek / Qwen 等的思考字段。
    #[serde(default)]
    reasoning_content: Option<String>,
    /// 部分网关用的另一个名字。
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<DeltaToolCall>>,
}

#[derive(Deserialize, Default)]
struct DeltaToolCall {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<DeltaFunction>,
}

#[derive(Deserialize, Default)]
struct DeltaFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

/// SSE 帧缓冲：按**字节**累积，只在完整帧边界解码 UTF-8。
///
/// 网络分块可能把多字节字符劈成两半；逐块 `from_utf8_lossy` 会把半个汉字变成 `�`。
/// 帧边界（空行）永远是 ASCII，所以攒到空行再整帧解码才是安全的。
#[derive(Default)]
struct SseBuffer {
    buf: Vec<u8>,
}

impl SseBuffer {
    fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// 取下一帧（不含分隔空行）。兼容 `\n\n` 与 `\r\n\r\n` 两种 SSE 换行。
    fn next_frame(&mut self) -> Option<String> {
        let (end, sep) = find_frame_end(&self.buf)?;
        let frame = String::from_utf8_lossy(&self.buf[..end]).into_owned();
        self.buf.drain(..end + sep);
        Some(frame)
    }

    /// 流结束时处理没有以空行收尾的最后一帧（有些兼容网关会这样）。
    fn finish(&mut self) -> Option<String> {
        if self.buf.is_empty() {
            return None;
        }
        let frame = String::from_utf8_lossy(&self.buf).into_owned();
        self.buf.clear();
        Some(frame)
    }
}

/// 找空行分隔符：返回（帧尾偏移，分隔符字节数）。
fn find_frame_end(buf: &[u8]) -> Option<(usize, usize)> {
    for i in 0..buf.len().saturating_sub(1) {
        if buf[i] == b'\n' && buf[i + 1] == b'\n' {
            return Some((i, 2));
        }
        if i + 3 < buf.len()
            && buf[i] == b'\r'
            && buf[i + 1] == b'\n'
            && buf[i + 2] == b'\r'
            && buf[i + 3] == b'\n'
        {
            return Some((i, 4));
        }
    }
    None
}

/// 解析一帧 SSE：拆 `data:` 行、累积正文 / 思考 / 工具调用。
fn ingest_frame<F>(frame: &str, calls: &mut Vec<ToolCall>, on_event: &mut F)
where
    F: FnMut(StreamEvent),
{
    for line in frame.lines() {
        let Some(data) = line.strip_prefix("data:") else { continue };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let parsed: Chunk = match serde_json::from_str(data) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let Some(choice) = parsed.choices.into_iter().next() else { continue };
        let delta = choice.delta;
        if let Some(text) = delta.reasoning_content.or(delta.reasoning) {
            if !text.is_empty() {
                on_event(StreamEvent::Reasoning(text));
            }
        }
        if let Some(text) = delta.content {
            if !text.is_empty() {
                on_event(StreamEvent::Content(text));
            }
        }
        if let Some(parts) = delta.tool_calls {
            for part in parts {
                while calls.len() <= part.index {
                    calls.push(ToolCall { id: String::new(), name: String::new(), arguments: String::new() });
                }
                let slot = &mut calls[part.index];
                if let Some(id) = part.id {
                    if !id.is_empty() {
                        slot.id = id;
                    }
                }
                if let Some(func) = part.function {
                    if let Some(name) = func.name {
                        if !name.is_empty() {
                            slot.name = name;
                        }
                    }
                    if let Some(args) = func.arguments {
                        slot.arguments.push_str(&args);
                    }
                }
            }
        }
    }
}

/// 流式发一轮对话。把正文 / 思考增量通过 `on_event` 抛给调用方；
/// 返回本轮累积出的工具调用（可能为空）。取消令牌命中时立刻断开连接。
pub async fn stream_chat<F>(
    cfg: &AgentConfig,
    messages: &[serde_json::Value],
    tools: &[serde_json::Value],
    cancel: &CancellationToken,
    mut on_event: F,
) -> Result<Vec<ToolCall>, String>
where
    F: FnMut(StreamEvent),
{
    let root = normalize_base(&cfg.base_url);
    if root.is_empty() {
        return Err("尚未配置基础 URL".into());
    }
    let url = format!("{root}/chat/completions");
    let mut body = serde_json::json!({
        "model": cfg.model,
        "messages": messages,
        "stream": true,
    });
    if !tools.is_empty() {
        body["tools"] = serde_json::Value::Array(tools.to_vec());
        body["tool_choice"] = serde_json::Value::String("auto".into());
    }

    // 模型排队 / 网关慢的时候响应头可能几十秒才回来；这一段也必须能被停止，
    // 否则界面会在「思考中」卡到超时（300 秒）才响应。
    let send = client(Duration::from_secs(300))?
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .json(&body)
        .send();
    let resp = tokio::select! {
        _ = cancel.cancelled() => return Ok(Vec::new()),
        resp = send => resp.map_err(|e| format!("请求失败：{e}"))?,
    };
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP {status}：{}", brief(&text)));
    }

    let mut stream = resp.bytes_stream();
    let mut sse = SseBuffer::default();
    // 工具调用按 index 累积。
    let mut calls: Vec<ToolCall> = Vec::new();

    loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => return Ok(calls),
            next = stream.next() => next,
        };
        let Some(chunk) = chunk else { break };
        let chunk = chunk.map_err(|e| format!("读取流失败：{e}"))?;
        sse.push(&chunk);

        // 只在完整帧（空行）边界解码，避免把跨块的多字节字符劈坏。
        while let Some(frame) = sse.next_frame() {
            ingest_frame(&frame, &mut calls, &mut on_event);
        }
    }

    // 有些兼容网关的最后一帧没有以空行结尾：别把尾巴丢掉。
    if let Some(frame) = sse.finish() {
        ingest_frame(&frame, &mut calls, &mut on_event);
    }

    calls.retain(|c| !c.name.is_empty());
    Ok(calls)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 按给定分块喂入，收集所有完整帧（含收尾帧）。
    fn frames(chunks: &[&[u8]]) -> Vec<String> {
        let mut buf = SseBuffer::default();
        let mut out = Vec::new();
        for chunk in chunks {
            buf.push(chunk);
            while let Some(frame) = buf.next_frame() {
                out.push(frame);
            }
        }
        if let Some(frame) = buf.finish() {
            out.push(frame);
        }
        out
    }

    #[test]
    fn multibyte_content_survives_chunk_boundaries() {
        // 逐字节切块：每个汉字都会被劈开；整帧解码后不应出现替换字符。
        let payload = "data: {\"choices\":[{\"delta\":{\"content\":\"思考\"}}]}\n\n";
        let bytes = payload.as_bytes().to_vec();
        let chunks: Vec<&[u8]> = bytes.chunks(1).collect();
        let got = frames(&chunks);
        assert_eq!(got.len(), 1);
        assert!(got[0].contains("思考"), "frame = {got:?}");
        assert!(!got[0].contains('\u{FFFD}'), "frame = {got:?}");
    }

    #[test]
    fn crlf_frames_are_supported() {
        let got = frames(&[b"data: a\r\n\r\ndata: b\r\n\r\n"]);
        assert_eq!(got, vec!["data: a".to_string(), "data: b".to_string()]);
    }

    #[test]
    fn trailing_frame_without_blank_line_is_flushed() {
        let got = frames(&[b"data: a\n\ndata: b"]);
        assert_eq!(got, vec!["data: a".to_string(), "data: b".to_string()]);
    }

    #[test]
    fn utf8_split_across_frames_is_not_reassembled() {
        // 空行是帧边界；帧内内容永远是完整 UTF-8（帧边界本身是 ASCII）。
        // 这里确认缓冲区只把完整帧交出去，剩下半个字符留在缓冲里。
        let bytes = "data: 思考".as_bytes().to_vec();
        let (head, tail) = bytes.split_at(bytes.len() - 1);
        let mut buf = SseBuffer::default();
        buf.push(head);
        assert!(buf.next_frame().is_none());
        buf.push(tail);
        buf.push(b"\n\n");
        let frame = buf.next_frame().expect("complete frame");
        assert!(frame.contains("思考"));
        assert!(!frame.contains('\u{FFFD}'));
    }
}
