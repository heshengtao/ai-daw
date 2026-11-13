//! 宿主抽象：后端需要的一切"进程外部"能力都从这里拿。
//!
//! 桌面端提供原生对话框与插件编辑器窗口；无窗口进程（`aidawd`）把这些能力
//! 明确降级为 `None`——**不静默失败**，调用方据此给出「此操作需要桌面端」的提示。

use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;

/// 一次"选文件"请求。桌面端据此弹原生对话框，无窗口进程直接拒绝。
#[derive(Debug, Clone)]
pub struct PickRequest {
    /// 判断用的语义（`project` / `audio` / `midi` / `export`），不是给用户看的文案。
    pub kind: &'static str,
    /// 允许的扩展名；空表示不限。
    pub extensions: Vec<&'static str>,
}

/// 原生能力的抽象。**只有桌面端能实现成有意义的行为**。
pub trait UiHost: Send + Sync {
    /// 弹系统对话框选一个文件；用户取消返回 `Ok(None)`。
    fn pick_file(&self, request: PickRequest) -> Result<Option<PathBuf>, String>;

    /// 首帧画完后把主窗口显示出来（窗口创建时是隐藏的，避免白屏）。
    fn show_main_window(&self);

    /// 把一段活派到 UI 主线程（CLAP 的 init / 参数枚举必须在主线程）。
    fn run_on_main_thread(&self, task: Box<dyn FnOnce() + Send>);
}

/// 后端与外壳之间唯一的契约。
pub trait Host: Send + Sync + 'static {
    /// 广播一个事件。桌面端发给 webview，无窗口进程转发给 SSE（M3）。
    fn emit_value(&self, event: &'static str, payload: Value);

    /// 应用数据目录（会话缓存、插件缓存、token 都落在这里）。
    fn data_dir(&self) -> PathBuf;

    /// 没有原生能力时返回 `None`。
    fn ui(&self) -> Option<&dyn UiHost>;
}

/// 便捷包装：序列化失败只记日志，绝不 panic —— 事件广播失败不该让一次编辑失败。
pub fn emit<S: Serialize + ?Sized>(host: &dyn Host, event: &'static str, payload: &S) {
    match serde_json::to_value(payload) {
        Ok(value) => host.emit_value(event, value),
        Err(err) => eprintln!("[host] 序列化事件 {event} 失败：{err}"),
    }
}

/// 测试用宿主：只记事件，不碰磁盘、不开窗口。
#[derive(Default)]
pub struct FakeHost {
    /// 收到的全部事件（`(通道名, 载荷)`），测试据此断言"有没有通知界面"。
    pub events: std::sync::Mutex<Vec<(String, Value)>>,
    /// 数据目录；为空时用 `fake-data`。
    pub dir: PathBuf,
}

impl FakeHost {
    /// 指定数据目录。
    pub fn with_dir(dir: impl Into<PathBuf>) -> Self {
        Self { events: Default::default(), dir: dir.into() }
    }

    /// 已收到的事件条数。
    pub fn event_count(&self) -> usize {
        self.events.lock().unwrap().len()
    }

    /// 某个通道上的事件载荷。
    pub fn events_on(&self, channel: &str) -> Vec<Value> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == channel)
            .map(|(_, payload)| payload.clone())
            .collect()
    }
}

impl Host for FakeHost {
    fn emit_value(&self, event: &'static str, payload: Value) {
        self.events.lock().unwrap().push((event.to_string(), payload));
    }

    fn data_dir(&self) -> PathBuf {
        if self.dir.as_os_str().is_empty() {
            PathBuf::from("fake-data")
        } else {
            self.dir.clone()
        }
    }

    fn ui(&self) -> Option<&dyn UiHost> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct Payload {
        value: u32,
    }

    #[test]
    fn fake_host_records_events_with_json_payload() {
        let host = FakeHost::default();
        emit(&host, "engine://event", &Payload { value: 7 });
        assert_eq!(host.event_count(), 1);
        let events = host.events.lock().unwrap();
        assert_eq!(events[0].0, "engine://event");
        assert_eq!(events[0].1["value"], 7);
    }

    #[test]
    fn fake_host_has_data_dir_and_no_ui() {
        let host = FakeHost::default();
        assert!(host.data_dir().ends_with("fake-data"));
        assert!(host.ui().is_none());
    }
}

/// 默认宿主：丢弃事件、数据目录为 `.`、没有原生能力。
///
/// 用于测试与"只关心结果、不关心通知"的入口；桌面端与 `aidawd` 各自注入真实现。
pub struct NoopHost;

impl Host for NoopHost {
    fn emit_value(&self, _event: &'static str, _payload: Value) {}

    fn data_dir(&self) -> PathBuf {
        PathBuf::from(".")
    }

    fn ui(&self) -> Option<&dyn UiHost> {
        None
    }
}
