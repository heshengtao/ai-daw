//! 进程内的**事件总线**：所有事实广播（编辑、传输、导入导出进度）都从这里出去。
//!
//! 桌面端的 webview、浏览器端的 SSE、未来的 PCM 推流元数据都面向这一条总线。
//! 之前每个模块各自 `app.emit`，于是"Tauri 里能收到、网页端收不到"成了默认状态；
//! 现在 Host 实现与 SSE 端点共用同一个 `Sender`。

use std::sync::OnceLock;

use serde_json::Value;
use tokio::sync::broadcast;

/// 环形缓冲容量：订阅者落后超过这个数就会丢事件（并收到 `Lagged`），
/// 而不是把整个进程拖慢。UI 事件本来就是"最新状态优先"，丢旧的没关系。
const CAPACITY: usize = 512;

static BUS: OnceLock<broadcast::Sender<(String, Value)>> = OnceLock::new();

fn bus() -> &'static broadcast::Sender<(String, Value)> {
    BUS.get_or_init(|| broadcast::channel(CAPACITY).0)
}

/// 发布一条事件。没有订阅者时静默丢弃（与 `emit` 语义一致）。
pub fn publish(event: &str, payload: Value) {
    let _ = bus().send((event.to_string(), payload));
}

/// 订阅事件流。每个 SSE / WebSocket 客户端各持有一个。
pub fn subscribe() -> broadcast::Receiver<(String, Value)> {
    bus().subscribe()
}

/// 当前订阅者数量（`/health` 用，便于判断"有没有客户端连着"）。
pub fn subscribers() -> usize {
    bus().receiver_count()
}
