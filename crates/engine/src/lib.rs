#![forbid(unsafe_code)]

pub mod apply;
pub mod bus;
pub mod command;
pub mod event;
pub mod model;
pub mod registry;
pub mod sequencer;

pub use bus::Engine;
pub use registry::PluginRegistry;

/// 音频块帧数。
pub const BLOCK_SIZE: usize = 256;

/// 拍 → 秒。
pub fn beats_to_seconds(beats: f64, tempo: f64) -> f64 {
    beats * 60.0 / tempo
}
