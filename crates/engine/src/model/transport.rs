use serde::{Deserialize, Serialize};

/// 传输设置。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transport {
    pub tempo: f64,
    pub time_sig_num: u8,
    pub time_sig_den: u8,
}

impl Default for Transport {
    fn default() -> Self {
        Transport { tempo: 120.0, time_sig_num: 4, time_sig_den: 4 }
    }
}
