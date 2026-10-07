#![forbid(unsafe_code)]

pub mod model;

/// 音频块帧数。
pub const BLOCK_SIZE: usize = 256;

/// 拍 → 秒。
pub fn beats_to_seconds(beats: f64, tempo: f64) -> f64 {
    beats * 60.0 / tempo
}
