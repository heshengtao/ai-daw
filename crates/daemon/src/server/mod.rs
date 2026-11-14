//! HTTP 服务：`/rpc/<cmd>`、`/health` 与（M3 的）`/events`、`/audio`。
//!
//! axum 0.8 的路径参数语法是 `{name}`（不是旧版的 `:name`）。
//! 鉴权：`Authorization: Bearer <token>`；`EventSource` / `WebSocket` 带不了 header，
//! 所以 query 参数 `?token=` 同样有效。

pub mod rpc;

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;
use serde_json::Value;

use rpc::RpcError;

use crate::daemon::Daemon;

/// axum 的共享状态。
pub struct ServerState {
    /// 后端实例。
    pub daemon: Arc<Daemon>,
    /// 访问令牌。
    pub token: String,
    /// 前端构建产物目录；`None` = 只提供 API，不托管界面。
    pub static_dir: Option<PathBuf>,
    /// 浏览器会话：打开过界面就发一个，之后 `/rpc`、`/events`、`/audio` 认它。
    ///
    /// 这样浏览器地址栏里**不需要带 token**（与 ExamBot 的体验一致）；
    /// 命令行 / 脚本仍然可以用 `Authorization: Bearer <token>`。
    pub sessions: std::sync::Mutex<std::collections::HashSet<String>>,
}

/// 浏览器会话 cookie 名。
const SESSION_COOKIE: &str = "ai-daw-session";

/// 组装路由。
pub fn router(state: Arc<ServerState>) -> Router {
    Router::new()
        .route("/rpc/{cmd}", post(rpc_handler))
        .route("/events", get(events_handler))
        .route("/audio", get(audio_handler))
        .route("/health", get(health))
        .route("/commands", get(commands))
        // 其余路径交给前端产物（hash 路由，不需要 SPA 兜底规则，但仍兜一下更稳）。
        .fallback(get(static_handler))
        .with_state(state)
}

#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

/// 请求是否被授权。
///
/// 三条通道，任一成立即可：
/// 1. `Authorization: Bearer <token>`（命令行、脚本）；
/// 2. `?token=<token>`（`EventSource` / `WebSocket` 带不了 header 时的兜底）；
/// 3. 浏览器会话 cookie（打开过界面就有，地址栏不必带 token）。
fn authorized(state: &ServerState, headers: &HeaderMap, query: &TokenQuery) -> bool {
    let token = state.token.as_str();
    let bearer = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if bearer == Some(token) || query.token.as_deref() == Some(token) {
        return true;
    }
    let Some(session) = session_cookie(headers) else {
        return false;
    };
    state.sessions.lock().map(|set| set.contains(&session)).unwrap_or(false)
}

/// 从 `Cookie` 头里取出会话标识。
fn session_cookie(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|part| {
        let part = part.trim();
        part.strip_prefix(&format!("{SESSION_COOKIE}="))
            .map(|value| value.to_string())
    })
}

/// 给浏览器发一个会话（已存在就复用）。
fn ensure_session(state: &ServerState, headers: &HeaderMap) -> Option<String> {
    if let Some(existing) = session_cookie(headers) {
        if state
            .sessions
            .lock()
            .map(|set| set.contains(&existing))
            .unwrap_or(false)
        {
            return None;
        }
    }
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        return None;
    }
    let session: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    if let Ok(mut set) = state.sessions.lock() {
        set.insert(session.clone());
    }
    Some(session)
}

async fn rpc_handler(
    State(state): State<Arc<ServerState>>,
    Path(cmd): Path<String>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !authorized(&state, &headers, &query) {
        return RpcError::Unauthorized.into_response();
    }
    let args: Value = if body.is_empty() {
        Value::Object(Default::default())
    } else {
        match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(err) => return RpcError::BadArgs(err.to_string()).into_response(),
        }
    };
    // 命令实现里有渲染、读盘、加锁：绝不能放在 async 任务的 poll 里。
    let daemon = Arc::clone(&state.daemon);
    let result = tokio::task::spawn_blocking(move || {
        rpc::dispatch(daemon.state(), &cmd, args)
    })
    .await;
    match result {
        Ok(Ok(value)) => axum::Json(value).into_response(),
        Ok(Err(err)) => err.into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("rpc task failed: {err}"),
        )
            .into_response(),
    }
}

async fn health(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    axum::Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "audio": state.daemon.state().audio.snapshot().available,
    }))
}

/// 事件流（Server-Sent Events）。
///
/// `EventSource` 带不了自定义 header，所以 token 走 query。每条事件都带着原始通道名
/// （`engine://event` 之类），前端按名字过滤即可——与桌面端的 `listen` 用法一致。
async fn events_handler(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&state, &headers, &query) {
        return RpcError::Unauthorized.into_response();
    }
    let rx = crate::bus::subscribe();
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok((name, payload)) => {
                    let data = serde_json::json!({ "event": name, "payload": payload }).to_string();
                    return Some((
                        Ok::<_, std::convert::Infallible>(Event::default().event("engine").data(data)),
                        rx,
                    ));
                }
                // 客户端跟不上：丢旧事件，继续（UI 关心的是最新状态）。
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// 实时监听：把音频线程的混音抽头按 ~10 ms 一块推到浏览器。
///
/// 帧格式（小端）：
/// `[epoch u32][start_frame u64][frames u16][channels u16][f32 × frames × channels]`
///
/// `epoch` 是流代数：播放器重建（任何编辑）都会换一版，客户端据此丢弃旧流并重新同步。
/// `start_frame` 是这一块第一帧在时间轴上的绝对位置，客户端用「起点 + 已消费帧数」
/// 推算播放头，不依赖服务器时间戳。
async fn audio_handler(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !authorized(&state, &headers, &query) {
        return RpcError::Unauthorized.into_response();
    }
    let tap = state.daemon.state().audio.web_tap();
    ws.on_upgrade(move |socket| stream_tap(socket, tap))
}

/// 一块 PCM 的线上编码。
fn encode_block(epoch: u32, start_frame: u64, interleaved: &[f32]) -> Vec<u8> {
    let frames = interleaved.len() / 2;
    let mut out = Vec::with_capacity(16 + interleaved.len() * 4);
    out.extend_from_slice(&epoch.to_le_bytes());
    out.extend_from_slice(&start_frame.to_le_bytes());
    out.extend_from_slice(&(frames as u16).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    for sample in interleaved {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

async fn stream_tap(mut socket: WebSocket, tap: Arc<ai_daw_engine::tap::AudioTap>) {
    // 从「现在」开始，不补历史：监听要的是低延迟，不是完整的回放。
    let mut cursor = tap.cursor();
    let mut buffer: Vec<f32> = Vec::new();
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(10));
    loop {
        ticker.tick().await;
        let start = cursor;
        let frames = tap.read_since(&mut cursor, &mut buffer);
        if frames == 0 {
            // 暂停 / 停播时没有新数据；顺便探测一下客户端还在不在。
            if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                break;
            }
            continue;
        }
        let packet = encode_block(tap.epoch(), start as u64, &buffer);
        if socket.send(Message::Binary(packet.into())).await.is_err() {
            break;
        }
    }
}

/// 前端产物：`dist/` 下的静态文件。
async fn static_handler(
    State(state): State<Arc<ServerState>>,
    uri: axum::http::Uri,
    headers: HeaderMap,
) -> Response {
    let Some(dir) = &state.static_dir else {
        return (
            StatusCode::NOT_FOUND,
            "这个 daemon 没有配置前端产物目录（--web-dir）",
        )
            .into_response();
    };
    let mut relative = uri.path().trim_start_matches('/');
    if relative.is_empty() {
        relative = "index.html";
    }
    // 目录穿越防护：只接受普通相对路径。
    if relative.split('/').any(|part| part == "..") {
        return (StatusCode::BAD_REQUEST, "bad path").into_response();
    }
    let file = dir.join(relative);
    let bytes = match tokio::fs::read(&file).await {
        Ok(bytes) => bytes,
        // 不认识的路由回落到 index.html（hash 路由用不到，但直接访问 `/` 之外也顺手）。
        Err(_) => match tokio::fs::read(dir.join("index.html")).await {
            Ok(bytes) => bytes,
            Err(_) => return (StatusCode::NOT_FOUND, "not found").into_response(),
        },
    };
    let mime = guess_mime(relative);
    let mut response = ([(axum::http::header::CONTENT_TYPE, mime)], bytes).into_response();
    // 打开界面就发一个会话：地址栏不必带 token，后续请求靠 cookie。
    if let Some(session) = ensure_session(&state, &headers) {
        let cookie = format!("{SESSION_COOKIE}={session}; Path=/; HttpOnly; SameSite=Lax; Max-Age=31536000");
        if let Ok(value) = cookie.parse() {
            response.headers_mut().insert(axum::http::header::SET_COOKIE, value);
        }
    }
    response
}

/// 够用就行的 MIME 表：前端产物只有这几类文件。
fn guess_mime(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("wasm") => "application/wasm",
        Some("map") => "application/json",
        _ => "application/octet-stream",
    }
}

async fn commands() -> impl IntoResponse {
    axum::Json(serde_json::json!({ "commands": rpc::COMMANDS }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::Config;
    use crate::host::FakeHost;
    use crate::state::AppState;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn server() -> (Router, Arc<FakeHost>) {
        let host = Arc::new(FakeHost::default());
        let state = Arc::new(AppState::bootstrap_with_host(None, Vec::new(), host.clone()));
        let daemon = Daemon::new(state, host.clone(), Config::default());
        (router(Arc::new(ServerState {
            daemon,
            token: "t0ken".into(),
            static_dir: None,
            sessions: Default::default(),
        })), host)
    }

    async fn call(
        app: Router,
        uri: &str,
        auth: Option<&str>,
        body: &str,
    ) -> (StatusCode, String) {
        let mut builder = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(auth) = auth {
            builder = builder.header("authorization", format!("Bearer {auth}"));
        }
        let response = app
            .oneshot(builder.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    #[tokio::test]
    async fn get_project_requires_token() {
        let (app, _) = server();
        let (status, _) = call(app, "/rpc/get_project", None, "{}").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn get_project_returns_project_json() {
        let (app, _) = server();
        let (status, body) = call(app, "/rpc/get_project", Some("t0ken"), "{}").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("\"tracks\""));
    }

    #[tokio::test]
    async fn add_track_returns_updated_project_and_emits_event() {
        let (app, host) = server();
        let before = host.event_count();
        let (status, body) = call(app, "/rpc/add_track", Some("t0ken"), r#"{"name":"Lead"}"#).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Lead"));
        assert!(host.event_count() > before, "编辑必须广播事件");
    }

    #[tokio::test]
    async fn query_token_also_works() {
        let (app, _) = server();
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/health?token=t0ken")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn block_encoding_has_expected_layout() {
        let encoded = encode_block(7, 1024, &[0.5, -0.5, 0.25, -0.25]);
        assert_eq!(&encoded[0..4], &7u32.to_le_bytes());
        assert_eq!(&encoded[4..12], &1024u64.to_le_bytes());
        assert_eq!(&encoded[12..14], &2u16.to_le_bytes(), "2 帧");
        assert_eq!(&encoded[14..16], &2u16.to_le_bytes(), "2 声道");
        let first = f32::from_le_bytes(encoded[16..20].try_into().unwrap());
        assert!((first - 0.5).abs() < 1e-6);
    }

    #[tokio::test]
    async fn unknown_command_is_400() {
        let (app, _) = server();
        let (status, body) = call(app, "/rpc/no_such_cmd", Some("t0ken"), "{}").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("unknown command"));
    }

    #[tokio::test]
    async fn bad_args_is_400() {
        let (app, _) = server();
        let (status, _) = call(app, "/rpc/add_track", Some("t0ken"), "{}").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
