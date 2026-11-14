//! HTTP 服务：`/rpc/<cmd>`、`/health` 与（M3 的）`/events`、`/audio`。
//!
//! axum 0.8 的路径参数语法是 `{name}`（不是旧版的 `:name`）。
//!
//! **不鉴权**：这个服务的定位是"无头版本"——给没有显示器的机器用，打开端口就是界面。
//! 访问控制交给部署侧（局域网、容器网络、反向代理/网关）。要对外暴露就自己加一层。

pub mod rpc;

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
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
    /// 前端构建产物目录；`None` = 只提供 API，不托管界面。
    pub static_dir: Option<PathBuf>,
}


/// 组装路由。
pub fn router(state: Arc<ServerState>) -> Router {
    Router::new()
        .route("/rpc/{cmd}", post(rpc_handler))
        .route("/events", get(events_handler))
        .route("/audio", get(audio_handler))
        .route("/health", get(health))
        .route("/commands", get(commands))
        .route("/upload", post(upload_handler))
        .route("/download", get(download_handler))
        // 其余路径交给前端产物（hash 路由，不需要 SPA 兜底规则，但仍兜一下更稳）。
        .fallback(get(static_handler))
        // axum 默认请求体上限是 2 MB，真实的音频/视频素材随便就超了。
        .layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024 * 1024))
        .with_state(state)
}

async fn rpc_handler(
    State(state): State<Arc<ServerState>>,
    Path(cmd): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    let args: Value = if body.is_empty() {
        Value::Object(Default::default())
    } else {
        match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(err) => return RpcError::BadArgs(err.to_string()).into_response(),
        }
    };
    // 导出：渲染 + 编码要几秒到几十秒，单独 await。路径留空时由服务端生成暂存文件。
    if cmd == "export_audio" {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Args {
            #[serde(default)]
            path: String,
            #[serde(default = "default_rate")]
            sample_rate: u32,
            format: crate::commands::ExportFormat,
            #[serde(default)]
            bitrate_kbps: Option<u32>,
        }
        fn default_rate() -> u32 {
            44_100
        }
        let parsed: Args = match serde_json::from_value(args) {
            Ok(value) => value,
            Err(err) => return RpcError::BadArgs(err.to_string()).into_response(),
        };
        let path = if parsed.path.trim().is_empty() {
            let ext = match parsed.format {
                crate::commands::ExportFormat::Wav => "wav",
                crate::commands::ExportFormat::Mp3 => "mp3",
            };
            export_staging_path(&state, ext).to_string_lossy().to_string()
        } else {
            parsed.path
        };
        let daemon = Arc::clone(&state.daemon);
        return match crate::commands::export_audio_impl(
            daemon.state(),
            path,
            parsed.sample_rate,
            parsed.format,
            parsed.bitrate_kbps,
        )
        .await
        {
            Ok(path) => axum::Json(serde_json::json!({ "path": path })).into_response(),
            Err(err) => RpcError::Failed(err).into_response(),
        };
    }
    if cmd == "save_project" {
        #[derive(serde::Deserialize)]
        struct Args {
            #[serde(default)]
            path: String,
        }
        let parsed: Args = match serde_json::from_value(args) {
            Ok(value) => value,
            Err(err) => return RpcError::BadArgs(err.to_string()).into_response(),
        };
        let path = if parsed.path.trim().is_empty() {
            project_staging_path(&state)
        } else {
            parsed.path
        };
        let daemon = Arc::clone(&state.daemon);
        return match crate::commands::save_project_state(daemon.state(), &path) {
            Ok(path) => axum::Json(serde_json::json!({ "path": path })).into_response(),
            Err(err) => RpcError::Failed(err).into_response(),
        };
    }
    if cmd == "export_midi" {
        #[derive(serde::Deserialize)]
        struct Args {
            #[serde(default)]
            path: String,
            #[serde(default)]
            tracks: Option<Vec<u64>>,
        }
        let parsed: Args = match serde_json::from_value(args) {
            Ok(value) => value,
            Err(err) => return RpcError::BadArgs(err.to_string()).into_response(),
        };
        let path = if parsed.path.trim().is_empty() {
            export_staging_path(&state, "mid").to_string_lossy().to_string()
        } else {
            parsed.path
        };
        let daemon = Arc::clone(&state.daemon);
        return match crate::commands::export_midi_impl(daemon.state(), path, parsed.tracks).await {
            Ok(path) => axum::Json(serde_json::json!({ "path": path })).into_response(),
            Err(err) => RpcError::Failed(err).into_response(),
        };
    }
    if cmd == "export_score" {
        #[derive(serde::Deserialize)]
        struct Args {
            #[serde(default)]
            path: String,
            xml: String,
        }
        let parsed: Args = match serde_json::from_value(args) {
            Ok(value) => value,
            Err(err) => return RpcError::BadArgs(err.to_string()).into_response(),
        };
        let path = if parsed.path.trim().is_empty() {
            export_staging_path(&state, "musicxml").to_string_lossy().to_string()
        } else {
            parsed.path
        };
        return match crate::commands::export_score(path, parsed.xml).await {
            Ok(path) => axum::Json(serde_json::json!({ "path": path })).into_response(),
            Err(err) => RpcError::Failed(err).into_response(),
        };
    }
    // 整体替换工程（MIDI / MusicXML / 工程 JSON 导入的最后一步）。
    if cmd == "import_project" {
        #[derive(serde::Deserialize)]
        struct Args {
            project: ai_daw_engine::model::Project,
        }
        let parsed: Args = match serde_json::from_value(args) {
            Ok(value) => value,
            Err(err) => return RpcError::BadArgs(err.to_string()).into_response(),
        };
        let daemon = Arc::clone(&state.daemon);
        return match crate::commands::import_project_state(daemon.state(), parsed.project) {
            Ok(project) => axum::Json(project).into_response(),
            Err(err) => RpcError::Failed(err).into_response(),
        };
    }
    // 读文件字节（MIDI / MusicXML 导入）：返回原始二进制，不是 JSON。
    if cmd == "read_import_file" {
        #[derive(serde::Deserialize)]
        struct Args {
            path: String,
        }
        let parsed: Args = match serde_json::from_value(args) {
            Ok(value) => value,
            Err(err) => return RpcError::BadArgs(err.to_string()).into_response(),
        };
        return match tokio::fs::read(&parsed.path).await {
            Ok(bytes) => (
                [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
                bytes,
            )
                .into_response(),
            Err(err) => RpcError::Failed(format!("读取失败：{err}")).into_response(),
        };
    }
    // 导入音频是异步的（解码要几秒），单独 await，不能塞进 spawn_blocking。
    if cmd == "import_audio" {
        let parsed: rpc::ImportAudioArgs = match serde_json::from_value(args) {
            Ok(value) => value,
            Err(err) => return RpcError::BadArgs(err.to_string()).into_response(),
        };
        let daemon = Arc::clone(&state.daemon);
        return match crate::commands::import_audio_impl(
            daemon.state(),
            parsed.path,
            parsed.track,
            parsed.start,
        )
        .await
        {
            Ok(project) => axum::Json(project).into_response(),
            Err(err) => RpcError::Failed(err).into_response(),
        };
    }
    // 其余命令实现里有渲染、读盘、加锁：绝不能放在 async 任务的 poll 里。
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
/// 每条事件都带着原始通道名
/// （`engine://event` 之类），前端按名字过滤即可——与桌面端的 `listen` 用法一致。
async fn events_handler(
    State(state): State<Arc<ServerState>>,
) -> Response {
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
    ws: WebSocketUpgrade,
) -> Response {
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

#[derive(Deserialize)]
struct DownloadQuery {
    path: String,
}

/// `GET /download?path=…`：把服务端生成的导出文件发给浏览器。
///
/// 只允许数据目录内的文件（导出暂存与导入素材都落在那里），避免变成任意文件读取。
async fn download_handler(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<DownloadQuery>,
) -> Response {
    let root = state.daemon.config().data_dir.clone();
    let path = std::path::PathBuf::from(&query.path);
    let resolved = match path.canonicalize() {
        Ok(path) => path,
        Err(err) => return (StatusCode::NOT_FOUND, format!("找不到文件：{err}")).into_response(),
    };
    let root = root.canonicalize().unwrap_or(root);
    if !resolved.starts_with(&root) {
        return (StatusCode::FORBIDDEN, "只能下载数据目录内的文件").into_response();
    }
    match tokio::fs::read(&resolved).await {
        Ok(bytes) => {
            let name = resolved
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "download".into());
            (
                [
                    (axum::http::header::CONTENT_TYPE, "application/octet-stream".to_string()),
                    (
                        axum::http::header::CONTENT_DISPOSITION,
                        format!("attachment; filename=\"{name}\""),
                    ),
                ],
                bytes,
            )
                .into_response()
        }
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, format!("读取失败：{err}")).into_response(),
    }
}

#[derive(Deserialize)]
struct UploadQuery {
    /// 原始文件名（用来决定落盘名与扩展名）。
    name: String,
}

/// `POST /upload?name=foo.wav`：把浏览器选中的文件落到数据目录，返回可导入的绝对路径。
///
/// 浏览器的文件选择拿不到真实路径（安全限制），所以只能把字节传上来，
/// 由服务端写进 `<data_dir>/imports/`，再交给 `import_audio` 解码。
async fn upload_handler(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<UploadQuery>,
    body: axum::body::Bytes,
) -> Response {
    upload_to_data_dir(&state, &query.name, &body).await
}

/// 工程文件的暂存位置：`<数据目录>/projects/<时间戳>.json`。
fn project_staging_path(state: &ServerState) -> String {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let dir = state.daemon.config().data_dir.join("projects");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("project-{stamp}.json")).to_string_lossy().to_string()
}

/// 导出文件的暂存位置：`<数据目录>/exports/<时间戳>.<ext>`。
fn export_staging_path(state: &ServerState, ext: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let dir = state.daemon.config().data_dir.join("exports");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("export-{stamp}.{ext}"))
}

async fn upload_to_data_dir(state: &ServerState, name: &str, body: &[u8]) -> Response {
    // 只留最后一段文件名，挡掉 `../` 之类的路径注入。
    let safe: String = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("upload.bin")
        .chars()
        .filter(|c| !matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect();
    let safe = if safe.is_empty() { "upload.bin".to_string() } else { safe };
    let dir = state.daemon.config().data_dir.join("imports");
    if let Err(err) = tokio::fs::create_dir_all(&dir).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败：{err}")).into_response();
    }
    let path = dir.join(&safe);
    if let Err(err) = tokio::fs::write(&path, body).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("写入失败：{err}")).into_response();
    }
    axum::Json(serde_json::json!({
        "path": path.to_string_lossy(),
        "bytes": body.len(),
    }))
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
    let mut response = ([(axum::http::header::CONTENT_TYPE, mime)], bytes).into_response();
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
            static_dir: None,
        })), host)
    }

    async fn call(app: Router, uri: &str, body: &str) -> (StatusCode, String) {
        let builder = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json");
        let response = app
            .oneshot(builder.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    #[tokio::test]
    async fn get_project_returns_project_json() {
        let (app, _) = server();
        let (status, body) = call(app, "/rpc/get_project", "{}").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("\"tracks\""));
    }

    #[tokio::test]
    async fn add_track_returns_updated_project_and_emits_event() {
        let (app, host) = server();
        let before = host.event_count();
        let (status, body) = call(app, "/rpc/add_track", r#"{"name":"Lead"}"#).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Lead"));
        assert!(host.event_count() > before, "编辑必须广播事件");
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
        let (status, body) = call(app, "/rpc/no_such_cmd", "{}").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("unknown command"));
    }

    #[tokio::test]
    async fn bad_args_is_400() {
        let (app, _) = server();
        let (status, _) = call(app, "/rpc/add_track", "{}").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
