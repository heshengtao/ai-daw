//! 内置效果器：延迟与混响。
//!
//! 合成器本身**不做空间感**——混响/延迟属于轨道效果链。以前只有外部 VST3 能提供，
//! 于是铺底、弦乐、管乐、音效这些最需要空间的音色只能干着听。这两个是最小的补齐。
//!
//! # 线程与分配契约
//!
//! 延迟线只在 [`prepare`](daw_party_plugin_api::Plugin::prepare) 里按采样率分配
//! （`prepare` 只在建宿主时调用，不在音频回调里），[`process`](daw_party_plugin_api::Plugin::process)
//! 内**不分配、不加锁、不查表字符串**。`reset()` 清空延迟线，保证「暂停 = 静音」之后不留尾音。

mod delay;
mod reverb;

pub use delay::Delay;
pub use reverb::Reverb;

/// 内置延迟的插件 id。
pub const DELAY_ID: &str = "builtin.delay";
/// 内置混响的插件 id。
pub const REVERB_ID: &str = "builtin.reverb";
