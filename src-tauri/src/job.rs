//! 后台 AI 任务的 **panic 兜底**。
//!
//! 这些任务都跑在独立线程里（`std::thread::Builder::new().name("ai-daw-…")`）：线程一旦
//! panic——第三方库的 `expect`、越界、`unwrap` 之类——收尾的 `finish()` 就不会执行，
//! 界面上的 `running` 永远为真、进度条永远停在原地，用户只能重启应用。
//!
//! 典型例子：`ort` 在 CUDA 预加载失败时构造 `ort::Error`，顺手去拿还没初始化的 API，
//! 结果在系统目录里抓到一份旧版 `onnxruntime.dll` 并 `expect` panic（已修，见
//! `crates/local-ai/src/backend/runtime.rs`）——但**兜底仍然要有**：这类崩溃不该表现为
//! 「卡住不动」，而应该是一句能读懂的失败原因。

/// 在后台线程里跑一段任务，把 panic 变成 `Err`。
///
/// `catch_unwind` 需要 `UnwindSafe`；这里的闭包捕获的都是共享状态（`Arc` / `&AppHandle`），
/// 组合起来并不满足，所以用 `AssertUnwindSafe`——任务本身要么跑完要么崩，
/// 崩溃后也不会有代码继续用那些被"污染"的状态（`finish()` 只读自己的原子量）。
pub fn guard<F>(body: F) -> Result<String, String>
where
    F: FnOnce() -> Result<String, String>,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(result) => result,
        Err(payload) => Err(format!("后台任务崩溃：{}", panic_text(&*payload))),
    }
}

/// 从 panic 载荷里取一句能显示的话（`&str` / `String` 两种是标准做法）。
fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "未知错误（详见日志）".into())
}
