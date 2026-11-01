//! 内置效果器：空间、失真、EQ 与动态处理。
//!
//! 合成器本身**不做空间感**——混响/延迟属于轨道效果链。以前只有外部 VST3 能提供，
//! 于是铺底、弦乐、管乐、音效这些最需要空间的音色只能干着听。这两个是最小的补齐。
//! 压缩器与限幅器补的是「压得住」：没有它们，人声 / 鼓组 / 贝斯在总线上根本站不住。
//!
//! # 线程与分配契约
//!
//! 延迟线只在 [`prepare`](ai_daw_plugin_api::Plugin::prepare) 里按采样率分配
//! （`prepare` 只在建宿主时调用，不在音频回调里），[`process`](ai_daw_plugin_api::Plugin::process)
//! 内**不分配、不加锁、不查表字符串**。`reset()` 清空延迟线，保证「暂停 = 静音」之后不留尾音。

mod chorus;
mod compressor;
mod delay;
mod drive;
mod eq;
mod limiter;
mod reverb;

pub use chorus::Chorus;
pub use compressor::Compressor;
pub use delay::Delay;
pub use drive::Drive;
pub use eq::Eq;
pub use limiter::Limiter;
pub use reverb::Reverb;

/// 内置延迟的插件 id。
pub const DELAY_ID: &str = "builtin.delay";
/// 内置混响的插件 id。
pub const REVERB_ID: &str = "builtin.reverb";
/// 内置合唱的插件 id。
pub const CHORUS_ID: &str = "builtin.chorus";
/// 内置过载/失真的插件 id。
pub const DRIVE_ID: &str = "builtin.drive";
/// 内置两段 EQ 的插件 id。
pub const EQ_ID: &str = "builtin.eq";
/// 内置压缩器的插件 id。
pub const COMPRESSOR_ID: &str = "builtin.compressor";
/// 内置限幅器的插件 id。
pub const LIMITER_ID: &str = "builtin.limiter";
