//! 扩展与宿主之间的唯一桥：执行宿主命令、写日志。
//!
//! 扩展 crate 不认识 daemon（依赖方向是 daemon → extensions），所以这里只留一个 trait，
//! 由 daemon 注入实现。`Call` 是**同步**的：它跑在扩展线程上，阻塞那一条线程是设计的一部分
//! （见设计 §3 D2）。

use std::cell::RefCell;

/// 宿主命令桥。
pub trait Bridge: Send + Sync + 'static {
    /// 以扩展身份执行一条宿主命令（参数与返回值都是 JSON 字符串）。
    fn call(&self, plugin: &str, cmd: &str, args: &str) -> Result<String, String>;

    /// 写日志。
    fn log(&self, plugin: &str, level: &str, message: &str);
}

/// 什么都不做的桥（测试与「还没接上 daemon」时用）。
pub struct NoopBridge;

impl Bridge for NoopBridge {
    fn call(&self, _plugin: &str, _cmd: &str, _args: &str) -> Result<String, String> {
        Err("扩展宿主还没有接上命令层".into())
    }

    fn log(&self, _plugin: &str, _level: &str, _message: &str) {}
}

thread_local! {
    /// 当前线程正在代表哪个扩展执行命令。
    ///
    /// 命令层是同步执行的（`rpc::dispatch` → `commands::run_*`），所以用线程局部量把
    /// 「署名」传进编辑管线，比给 200 多个命令签名加一个 actor 参数便宜得多。
    static ACTOR: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// 在「代表某个扩展」的上下文里执行一段代码。
pub fn with_actor<T>(plugin: &str, code: impl FnOnce() -> T) -> T {
    struct Restore(Option<String>);
    impl Drop for Restore {
        fn drop(&mut self) {
            ACTOR.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let previous = ACTOR.with(|slot| slot.borrow_mut().replace(plugin.to_string()));
    let _restore = Restore(previous);
    code()
}

/// 当前线程是否代表某个扩展（命令层据此改署名）。
pub fn current_actor() -> Option<String> {
    ACTOR.with(|slot| slot.borrow().clone())
}
