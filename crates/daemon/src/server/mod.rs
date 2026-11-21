//! HTTP 服务：`/rpc/<cmd>`、`/health` 与（M3 的）`/events`、`/audio`。
//!
//! axum 0.8 的路径参数语法是 `{name}`（不是旧版的 `:name`）。
//!
//! **不鉴权**：这个服务的定位是"无头版本"——给没有显示器的机器用，打开端口就是界面。
//! 访问控制交给部署侧（局域网、容器网络、反向代理/网关）。要对外暴露就自己加一层。

pub mod cmds;
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
    /// 桌面外壳的应用句柄。
    ///
    /// 浏览器客户端发来的命令要和 webview 里点按钮**走同一条实现**——那些实现按 Tauri
    /// 签名写着（`AppHandle` / `State`），所以得把句柄带进来。无头进程没有句柄，用
    /// `tauri_api::rpc_app_handle()` 的空壳（它的 `state()` 背后是全局托管表）。
    pub app: Option<crate::tauri_api::AppHandle>,
}

impl ServerState {
    /// 命令层要的 `AppHandle`：桌面端用外壳给的，无头进程用空壳。
    pub fn app_handle(&self) -> crate::tauri_api::AppHandle {
        self.app.clone().unwrap_or_else(crate::tauri_api::rpc_app_handle)
    }
}


/// 组装路由。
pub fn router(state: Arc<ServerState>) -> Router {
    Router::new()
        .route("/rpc/{cmd}", post(rpc_handler))
        .route("/events", get(events_handler))
        .route("/audio", get(audio_handler))
        // 插件原生界面的帧流（容器里"原生界面进网页"）：帧走二进制，输入走文本。
        .route("/plugin-gui/{id}", get(plugin_gui_handler))
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
    // 扩展插件：装 / 卸 / 市场 / 状态都走一个入口（前端与桌面端同一份实现）。
    if cmd == "extensions_rpc" {
        let op = args.get("op").and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
        let payload = args
            .get("args")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Object(Default::default()));
        let app = state.app_handle();
        let state_arc = state.daemon.state_arc();
        return match crate::extensions::api(&app, &state_arc, &op, payload).await {
            Ok(value) => axum::Json(value).into_response(),
            Err(err) => RpcError::Failed(err).into_response(),
        };
    }
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
            Ok(path) => axum::Json(serde_json::Value::String(path)).into_response(),
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
        // 走桌面端同一条实现：覆盖旧文件前留历史、写盘后记一份快照。
        let app = state.app_handle();
        return match crate::commands::save_project_impl(&app, &path) {
            Ok(path) => axum::Json(serde_json::Value::String(path)).into_response(),
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
            Ok(path) => axum::Json(serde_json::Value::String(path)).into_response(),
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
            Ok(path) => axum::Json(serde_json::Value::String(path)).into_response(),
            Err(err) => RpcError::Failed(err).into_response(),
        };
    }
    // 离线试听：返回 WAV 原始字节（前端拿去当 Blob 播）。没有声卡的容器里这是唯一的播放路径。
    if cmd == "render_preview" {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Args {
            #[serde(default = "default_preview_rate")]
            sample_rate: u32,
        }
        fn default_preview_rate() -> u32 {
            44_100
        }
        let parsed: Args = match serde_json::from_value(args) {
            Ok(value) => value,
            Err(err) => return RpcError::BadArgs(err.to_string()).into_response(),
        };
        let daemon = Arc::clone(&state.daemon);
        return match tokio::task::spawn_blocking(move || {
            crate::commands::render_preview_wav(daemon.state(), parsed.sample_rate)
        })
        .await
        {
            Ok(Ok(bytes)) => {
                ([(axum::http::header::CONTENT_TYPE, "audio/wav")], bytes).into_response()
            }
            Ok(Err(err)) => RpcError::Failed(err).into_response(),
            Err(err) => RpcError::Failed(format!("render failed: {err}")).into_response(),
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
        // **只读数据目录里的文件**：`read_import_file` 原本会把任意绝对路径读出来还给你
        // （`--lan` + 端口映射下等于把整台机器的可读文件交出去）。浏览器的导入流程本来就
        // 是「先 `POST /upload` 落进 `<data>/imports`，再读回来」，所以这条限制不影响任何
        // 现有功能；桌面壳走原生对话框，不经过这里。
        if let Err(reason) = ensure_data_scoped(&state, &parsed.path) {
            return RpcError::Failed(reason).into_response();
        }
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
    // 要 await 的命令（打开工程、重扫插件、录音收尾、参数枚举…）：单独在这里 await。
    if let Some(result) = cmds::call_async(&state.app_handle(), &cmd, args.clone()).await {
        return match result {
            Ok(value) => axum::Json(value).into_response(),
            Err(err) => err.into_response(),
        };
    }
    // 其余命令实现里有渲染、读盘、加锁：绝不能放在 async 任务的 poll 里。
    let server = Arc::clone(&state);
    let result = tokio::task::spawn_blocking(move || rpc::dispatch(&server, &cmd, args)).await;
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

/// 插件原生界面的 WebSocket：`id` 是 `open_plugin_window` 返回的 streamId。
async fn plugin_gui_handler(
    State(_state): State<Arc<ServerState>>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> Response {
    let Some(session) = crate::plugin_gui::session(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    ws.on_upgrade(move |socket| stream_plugin_gui(socket, session))
}

/// 帧下行（二进制 JPEG）+ 输入上行（JSON 文本）。
async fn stream_plugin_gui(mut socket: WebSocket, session: Arc<crate::plugin_gui::Session>) {
    let mut frames = session.subscribe();
    loop {
        tokio::select! {
            frame = frames.recv() => match frame {
                Ok(bytes) => {
                    if socket.send(Message::Binary((*bytes).clone().into())).await.is_err() {
                        break;
                    }
                }
                // 落后太多（标签页被挂起过）：跳过这批，等下一帧——界面要的是最新画面。
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            },
            message = socket.recv() => match message {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(event) = serde_json::from_str::<crate::plugin_gui::InputEvent>(&text) {
                        session.send_input(event);
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
            },
        }
    }
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


/// 把路径限制在数据目录里（`read_import_file` 用）。
///
/// 用 `canonicalize` 而不是字符串前缀比较：`/data/../etc/passwd` 这类绕过必须挡住。
/// 目标不存在时直接报错——调用方本来也会拿到「读取失败」。
fn ensure_data_scoped(state: &ServerState, path: &str) -> Result<(), String> {
    let root = std::fs::canonicalize(&state.daemon.config().data_dir)
        .map_err(|error| format!("数据目录不可用：{error}"))?;
    let target = std::fs::canonicalize(path).map_err(|error| format!("读取失败：{error}"))?;
    if target.starts_with(&root) {
        Ok(())
    } else {
        Err("只能读取数据目录内的文件；浏览器请先上传".to_string())
    }
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
    let (bytes, served) = match tokio::fs::read(&file).await {
        Ok(bytes) => (bytes, relative.to_string()),
        // 不认识的路由回落到 index.html（history 路由直接访问 `/settings` 这类深链时走这里）。
        Err(_) => match tokio::fs::read(dir.join("index.html")).await {
            // MIME 要按**实际服务的文件**算：`/settings` 没有扩展名，若按请求路径猜会变成
            // application/octet-stream，浏览器直接当下载处理（页面白着不动）。
            Ok(bytes) => (bytes, "index.html".to_string()),
            Err(_) => return (StatusCode::NOT_FOUND, "not found").into_response(),
        },
    };
    let mime = guess_mime(&served);
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
    // 两张表合在一起：`rpc::COMMANDS` 是手写的分发分支，`cmds::NAMES` 是直接调用
    // Tauri 命令实现的那批。前端据此知道哪些名字可用（其余会拿到明确的 501）。
    let mut all: Vec<&str> = rpc::COMMANDS.to_vec();
    for name in cmds::NAMES {
        if !all.contains(name) {
            all.push(name);
        }
    }
    axum::Json(serde_json::json!({ "commands": all }))
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
        // 命令层按 Tauri 签名写，靠托管表取状态（无头进程里由这里填）。
        crate::tauri_api::share_rpc_state(&state);
        // 数据目录指到临时目录：`save_project` 会往里写 `projects/*.json`，
        // 用相对路径会把测试产物落在源码树里。
        let dir = std::env::temp_dir().join("ai-daw-rpc-tests");
        let _ = std::fs::create_dir_all(&dir);
        let daemon = Daemon::new(
            state,
            host.clone(),
            Config { data_dir: dir, ..Config::default() },
        );
        (router(Arc::new(ServerState {
            daemon,
            static_dir: None,
            app: None,
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

    /// 工程相关命令必须走「按 Tauri 签名写的那张表」（`cmds`），不能是 unknown。
    ///
    /// 这几条以前在网页端全报 `unknown command: …`，回归时最容易再掉一个。
    #[tokio::test]
    async fn project_commands_are_not_unknown() {
        let (app, _) = server();
        for cmd in [
            "project_history",
            "cache_session",
            "ops_list",
            "trim_clip",
            "audition_note",
            "plugin_scan_status",
        ] {
            let (_status, body) = call(app.clone(), &format!("/rpc/{cmd}"), "{}").await;
            // 少参数会得到 400 `bad args`（正常）；只有 `unknown command` 才是没接上。
            assert!(
                !body.contains("unknown command"),
                "{cmd} 又变成未知命令了：{body}"
            );
        }
    }

    /// 保存工程返回的是**裸路径字符串**（与桌面端 `Result<String, String>` 一致）。
    ///
    /// 以前返回 `{"path": …}`，前端 `currentPath` 会拿到一个对象，之后再保存就整条坏掉。
    #[tokio::test]
    async fn save_project_returns_a_bare_path_string() {
        let (app, _) = server();
        let (status, body) = call(app, "/rpc/save_project", r#"{"path":""}"#).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.starts_with('"'), "应当是 JSON 字符串：{body}");
        assert!(body.contains("project-"), "应当是服务端分配的路径：{body}");
    }

    /// 只有桌面端能做到的命令给出明确原因（501），而不是「未知命令」。
    #[tokio::test]
    async fn desktop_only_command_is_501_with_reason() {
        let (app, _) = server();
        let (status, body) = call(app, "/rpc/install_cli", r#"{}"#).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert!(body.contains("桌面端"), "{body}");
    }

    /// 插件原生界面在无头环境里不是"不支持"，而是需要虚拟显示：没有 `DISPLAY` 时
    /// 必须把原因说清楚（容器镜像自带 Xvfb，入口脚本会导出 DISPLAY）。
    #[tokio::test]
    async fn plugin_window_explains_missing_display() {
        let (app, _) = server();
        std::env::remove_var("DISPLAY");
        let (status, body) =
            call(app, "/rpc/open_plugin_window", r#"{"track":1,"title":"X"}"#).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert!(body.contains("Xvfb") || body.contains("虚拟显示"), "{body}");
    }

    /// 离线试听回的是 WAV 原始字节（没有声卡的容器里，这是唯一的播放路径）。
    #[tokio::test]
    async fn render_preview_returns_wav_bytes() {
        let (app, _) = server();
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/rpc/render_preview")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"sampleRate":22050}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[axum::http::header::CONTENT_TYPE],
            "audio/wav"
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert!(bytes.len() > 44, "至少要有 WAV 头 + 数据");
        assert_eq!(&bytes[0..4], b"RIFF");
    }
}
