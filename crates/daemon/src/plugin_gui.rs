//! 无头环境下的**插件原生界面**：虚拟显示 + 帧流 + 输入回传。
//!
//! 桌面壳里插件界面是原生窗口（`host-vst3` 自己开 NSWindow / HWND / XCB 窗口），
//! 浏览器连的是同一个进程，所以点「打开界面」就让宿主弹窗——保真度 100%，见
//! `server::cmds::window_open` 的桌面分支。
//!
//! 容器里没有窗口系统，但**插件本身还是能画出界面的**：给它一个虚拟显示（Xvfb），
//! 它就会像在有桌面的 Linux 上一样开一扇 X11 窗口。这里做三件事：
//!
//! 1. 在专用线程上开编辑器并持续 `service()`（窗口事件的泵必须在开窗线程上跑）；
//! 2. `ffmpeg -f x11grab` 把整块虚拟屏编码成 MJPEG，经 WebSocket 推给页面；
//! 3. 页面把鼠标/键盘事件发回来，用 `xdotool` 注入到同一个 X 显示。
//!
//! 于是"插件窗口"在页面上就是一块 canvas：和桌面原生窗口是同一个界面，只是换了个出口。
//!
//! **只做容器需要的那条路**：macOS / Windows 的桌面构建有自己的窗口系统，走 `desktop`
//! 分支（C 方案），这里不会被执行。

use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::server::rpc::RpcError;
use ai_daw_engine::model::TrackId;
use ai_daw_engine::registry::PluginSlot;
use crate::state::AppState;
use crate::tauri_api::{AppHandle, Manager, State};

/// 一帧 JPEG 的广播（多个浏览器标签可以同时看同一个界面）。
pub type FrameSink = tokio::sync::broadcast::Sender<Arc<Vec<u8>>>;

/// 一次远程界面会话。
pub struct Session {
    pub id: String,
    /// 虚拟屏尺寸（页面按这个比例缩放鼠标坐标）。
    pub width: u32,
    pub height: u32,
    frames: FrameSink,
    stop: Arc<AtomicBool>,
    /// 抓帧进程（关闭会话时要杀掉，否则它一直占着一个 X 连接）。
    capture: Mutex<Option<Child>>,
    /// 注入输入用的命令队列（页面 → xdotool）。
    input: std::sync::mpsc::Sender<InputEvent>,
}

impl Session {
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Arc<Vec<u8>>> {
        self.frames.subscribe()
    }

    pub fn send_input(&self, event: InputEvent) {
        let _ = self.input.send(event);
    }
}

/// 页面发来的输入事件（坐标已换算到虚拟屏像素）。
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InputEvent {
    Move { x: f64, y: f64 },
    Down { x: f64, y: f64, button: u32 },
    Up { x: f64, y: f64, button: u32 },
    Wheel { x: f64, y: f64, delta: i32 },
    /// 按键：`key` 是浏览器 `event.key`（`a`、`Enter`、`ArrowUp`…），由前端给出。
    Key { key: String, down: bool },
    /// 输入文本（走 xdotool type，处理不了组合输入时前端会退回按键事件）。
    Text { text: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenResult {
    pub stream_id: String,
    pub width: u32,
    pub height: u32,
    /// 前端要连的 WebSocket 路径（同源，带上会话 id）。
    pub url: String,
}

fn sessions() -> &'static Mutex<HashMap<String, Arc<Session>>> {
    static SLOT: OnceLock<Mutex<HashMap<String, Arc<Session>>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 某个槽位的界面是否开着（页面的"是否已打开"查询用）。
pub fn is_open(track: u64, plugin_id: &Option<String>) -> bool {
    sessions()
        .lock()
        .map(|map| map.contains_key(&key_of(track, plugin_id)))
        .unwrap_or(false)
}

/// 按 id 取会话（WebSocket 处理器用）。
pub fn session(id: &str) -> Option<Arc<Session>> {
    sessions().lock().ok()?.get(id).cloned()
}

/// 会话 id：16 位十六进制（不需要全局唯一，只要同一进程内不撞）。
fn random_id() -> String {
    let mut bytes = [0u8; 8];
    let _ = getrandom::fill(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn key_of(track: u64, plugin_id: &Option<String>) -> String {
    format!("{track}:{}", plugin_id.clone().unwrap_or_default())
}

/// 虚拟屏尺寸：`AIDAW_GUI_SIZE=1920x1080` 可覆盖（默认与 Dockerfile 里的 Xvfb 一致）。
fn screen_size() -> (u32, u32) {
    std::env::var("AIDAW_GUI_SIZE")
        .ok()
        .and_then(|text| {
            let (w, h) = text.split_once('x')?;
            Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
        })
        .unwrap_or((1440, 900))
}

fn display() -> Option<String> {
    std::env::var("DISPLAY").ok().filter(|value| !value.trim().is_empty())
}

/// 打开（或复用）某条轨道的插件原生界面。
///
/// 返回 `{streamId, width, height, url}`；页面拿 `url` 连 WebSocket 取帧。
pub fn open(
    _app: AppHandle,
    state: State<'_, Arc<AppState>>,
    track: u64,
    plugin_id: Option<String>,
    title: String,
) -> Result<Value, RpcError> {
    let Some(display) = display() else {
        return Err(RpcError::Unsupported(
            "插件原生界面需要虚拟显示：容器镜像自带 Xvfb，请用镜像默认启动方式运行（会导出 DISPLAY）"
                .into(),
        ));
    };
    let key = key_of(track, &plugin_id);
    if let Some(existing) = session(&key) {
        // 已经开着：直接复用（页面上再点一次不该开第二个窗口）。
        return Ok(json!(OpenResult {
            stream_id: existing.id.clone(),
            width: existing.width,
            height: existing.height,
            url: format!("/plugin-gui/{}", existing.id),
        }));
    }

    let slot = match plugin_id.as_deref() {
        Some(id) => {
            let index = resolve_effect_index(&state, track, id)?;
            PluginSlot::Effect(index)
        }
        None => PluginSlot::Instrument,
    };
    let editor = state
        .editor_slot(track, slot)
        .ok_or_else(|| RpcError::Failed("这个槽位没有可打开的插件界面".into()))?;
    let (width, height) = screen_size();
    let id = random_id();
    let (frames, _) = tokio::sync::broadcast::channel(4);
    let (input_tx, input_rx) = std::sync::mpsc::channel::<InputEvent>();
    let stop = Arc::new(AtomicBool::new(false));

    // 1) 编辑器线程：开窗 + 持续泵事件。窗口与 XCB 连接都是线程本地的，只能待在这里。
    let editor_stop = Arc::clone(&stop);
    let editor_display = display.clone();
    let thread = std::thread::Builder::new()
        .name("aidaw-plugin-gui".into())
        .spawn(move || {
            if let Err(error) = run_editor(editor, title, Some(editor_display), editor_stop) {
                eprintln!("[plugin-gui] 编辑器线程退出：{error}");
            }
        })
        .map_err(|error| RpcError::Failed(format!("启动插件界面线程失败：{error}")))?;

    // 2) 抓帧：fulldisplay → MJPEG → 广播。
    start_capture(&display, width, height, frames.clone(), Arc::clone(&stop))
        .map_err(|error| RpcError::Failed(format!("虚拟显示抓帧失败：{error}")))?;

    // 3) 输入注入线程。
    let input_stop = Arc::clone(&stop);
    let input_display = display.clone();
    std::thread::Builder::new()
        .name("aidaw-plugin-input".into())
        .spawn(move || {
            while !input_stop.load(Ordering::Relaxed) {
                match input_rx.recv_timeout(std::time::Duration::from_millis(200)) {
                    Ok(event) => inject(&input_display, event),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .ok();

    let session = Arc::new(Session {
        id: id.clone(),
        width,
        height,
        frames,
        stop,
        capture: Mutex::new(None),
        input: input_tx,
    });
    if let Ok(mut map) = sessions().lock() {
        map.insert(key, Arc::clone(&session));
    }
    // 会话结束时回收：编辑器线程与抓帧进程都靠 `stop` 退出。
    // 编辑器线程在自己的 `stop` 置位后收尾；这里只是别让 handle 泄漏成 detach。
    let _ = thread;

    Ok(json!(OpenResult {
        stream_id: id.clone(),
        width,
        height,
        url: format!("/plugin-gui/{id}"),
    }))
}

/// 关闭界面。
pub fn close(
    _app: AppHandle,
    _state: State<'_, Arc<AppState>>,
    track: u64,
    plugin_id: Option<String>,
) -> Result<Value, RpcError> {
    let key = key_of(track, &plugin_id);
    if let Ok(mut map) = sessions().lock() {
        if let Some(session) = map.remove(&key) {
            session.stop.store(true, Ordering::Relaxed);
            if let Ok(mut capture) = session.capture.lock() {
                if let Some(mut child) = capture.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
            return Ok(json!({ "closed": true, "streamId": session.id }));
        }
    }
    Ok(json!({ "closed": false }))
}

fn resolve_effect_index(
    state: &State<'_, Arc<AppState>>,
    track: u64,
    plugin_id: &str,
) -> Result<usize, RpcError> {
    let engine = state.engine.lock().unwrap();
    engine
        .project()
        .track(TrackId(track))
        .and_then(|track| track.effects.iter().position(|fx| fx.plugin_id == plugin_id))
        .ok_or_else(|| RpcError::Failed(format!("效果器不在链上：{plugin_id}")))
}

/// 编辑器线程主体：开窗、泵事件、直到 `stop`。
fn run_editor(
    editor: crate::audio::EditorSlot,
    title: String,
    display: Option<String>,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    if let Some(display) = display {
        // 子线程不继承父线程临时改的 env 之外的设置；显式钉一次，避免受外部影响。
        std::env::set_var("DISPLAY", display);
    }
    // 需要宿主窗口的插件（界面只能"嵌入"）在无头环境里没有窗口可嵌：明确拒绝，
    // 而不是开一扇空白窗让用户以为插件坏了。
    if let Some((width, height)) = editor.handle.host_window_size() {
        return Err(format!(
            "这个插件的界面需要宿主窗口（{width}×{height}），无头环境暂不支持嵌入；\
             请换用自带浮动窗口的插件，或在桌面版里打开"
        ));
    }
    let window = editor
        .handle
        .open(&title, None, None)
        .map_err(|error| format!("打开插件界面失败：{error}"))?;
    while !stop.load(Ordering::Relaxed) {
        window.service();
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    drop(window);
    Ok(())
}

/// 抓帧线程：直接读 X11 根窗口（`GetImage`）→ RGB → JPEG。
///
/// 不用 ffmpeg：ai-daw 的镜像里没有它（音频渲染是自研的），而插件界面只需要
/// "把一块屏幕编码成 JPEG"。纯 Rust 的 `x11rb` + `jpeg-encoder` 足够，也少一个外部依赖。
fn start_capture(
    display: &str,
    width: u32,
    height: u32,
    sink: FrameSink,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let display = display.to_string();
    let handle = std::thread::Builder::new()
        .name("aidaw-plugin-frames".into())
        .spawn(move || {
            let Ok((connection, screen)) = x11rb::connect(Some(&display)) else {
                eprintln!("[plugin-gui] 连不上虚拟显示 {display}");
                return;
            };
            use x11rb::connection::Connection as _;
            let root = connection.setup().roots[screen].root;
            let period = std::time::Duration::from_millis(66); // ~15 fps
            while !stop.load(Ordering::Relaxed) {
                let started = std::time::Instant::now();
                if let Some(frame) = grab_jpeg(&connection, root, width, height) {
                    let _ = sink.send(Arc::new(frame));
                }
                let elapsed = started.elapsed();
                if elapsed < period {
                    std::thread::sleep(period - elapsed);
                }
            }
        })
        .map_err(|error| format!("启动抓帧线程失败：{error}"))?;
    let _ = handle;
    Ok(())
}

/// 读一帧根窗口像素并编码成 JPEG。
fn grab_jpeg(
    connection: &x11rb::rust_connection::RustConnection,
    root: x11rb::protocol::xproto::Window,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    use x11rb::protocol::xproto::{ConnectionExt, ImageFormat};
    let reply = connection
        .get_image(
            ImageFormat::Z_PIXMAP,
            root,
            0,
            0,
            width as u16,
            height as u16,
            !0,
        )
        .ok()?
        .reply()
        .ok()?;
    // 24/32 位 ZPixmap：BGRX（字节序 B,G,R,X）→ RGB。
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for pixel in reply.data.chunks_exact(4) {
        rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
    }
    let mut out = Vec::new();
    let encoder = jpeg_encoder::Encoder::new(&mut out, 80);
    encoder
        .encode(&rgb, width as u16, height as u16, jpeg_encoder::ColorType::Rgb)
        .ok()?;
    Some(out)
}

/// 把页面事件注入到 X 显示。`xdotool` 是镜像里随包的小工具。
fn inject(display: &str, event: InputEvent) {
    let mut command = Command::new("xdotool");
    command.env("DISPLAY", display);
    match event {
        InputEvent::Move { x, y } => {
            command.args(["mousemove", "--sync", &x.round().to_string(), &y.round().to_string()]);
        }
        InputEvent::Down { x, y, button } => {
            command.args([
                "mousemove",
                "--sync",
                &x.round().to_string(),
                &y.round().to_string(),
                "click",
                &button.to_string(),
            ]);
        }
        InputEvent::Up { .. } => return,
        // X11 的滚轮就是按钮 4（上）/ 5（下）。
        InputEvent::Wheel { x, y, delta } => {
            let button = if delta > 0 { "4" } else { "5" };
            command.args([
                "mousemove",
                "--sync",
                &x.round().to_string(),
                &y.round().to_string(),
                "click",
                button,
            ]);
        }
        InputEvent::Key { key, down } => {
            let keysym = keysym_of(&key);
            if !down {
                return;
            }
            command.args(["key", "--clearmodifiers", &keysym]);
        }
        InputEvent::Text { text } => {
            command.args(["type", "--delay", "12", &text]);
        }
    }
    let _ = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// 浏览器 `key` → X keysym 名（只覆盖常用键；其余原样传）。
fn keysym_of(key: &str) -> String {
    match key {
        " " => "space".into(),
        "Enter" => "Return".into(),
        "Backspace" => "BackSpace".into(),
        "Tab" => "Tab".into(),
        "Escape" => "Escape".into(),
        "Delete" => "Delete".into(),
        "ArrowUp" => "Up".into(),
        "ArrowDown" => "Down".into(),
        "ArrowLeft" => "Left".into(),
        "ArrowRight" => "Right".into(),
        "Home" => "Home".into(),
        "End" => "End".into(),
        "PageUp" => "Prior".into(),
        "PageDown" => "Next".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keysyms_cover_navigation() {
        assert_eq!(keysym_of("Enter"), "Return");
        assert_eq!(keysym_of("ArrowLeft"), "Left");
        assert_eq!(keysym_of("a"), "a");
    }

    #[test]
    fn screen_size_can_be_overridden() {
        std::env::set_var("AIDAW_GUI_SIZE", "800x600");
        assert_eq!(screen_size(), (800, 600));
        std::env::remove_var("AIDAW_GUI_SIZE");
        assert_eq!(screen_size(), (1440, 900));
    }
}
