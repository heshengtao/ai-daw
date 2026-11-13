//! HTTP 服务：`/rpc/<cmd>`、`/health` 与（M3 的）`/events`、`/audio`。
//!
//! axum 0.8 的路径参数语法是 `{name}`（不是旧版的 `:name`）。
//! 鉴权：`Authorization: Bearer <token>`；`EventSource` / `WebSocket` 带不了 header，
//! 所以 query 参数 `?token=` 同样有效。

pub mod rpc;

use std::path::PathBuf;
use std::sync::Arc;

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
}

/// 组装路由。
pub fn router(state: Arc<ServerState>) -> Router {
    Router::new()
        .route("/rpc/{cmd}", post(rpc_handler))
        .route("/events", get(events_handler))
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

fn authorized(headers: &HeaderMap, query: &TokenQuery, token: &str) -> bool {
    let bearer = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    bearer == Some(token) || query.token.as_deref() == Some(token)
}

async fn rpc_handler(
    State(state): State<Arc<ServerState>>,
    Path(cmd): Path<String>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !authorized(&headers, &query, &state.token) {
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
    if !authorized(&headers, &query, &state.token) {
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

/// 前端产物：`dist/` 下的静态文件。
async fn static_handler(
    State(state): State<Arc<ServerState>>,
    uri: axum::http::Uri,
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
    ([(axum::http::header::CONTENT_TYPE, mime)], bytes).into_response()
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
        (router(Arc::new(ServerState { daemon, token: "t0ken".into(), static_dir: None })), host)
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
