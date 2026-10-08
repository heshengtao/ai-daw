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

impl Transport {
    /// 合法的拍号分母（2 的幂，从全音符到 32 分音符）。
    pub const TIME_SIG_DENS: [u8; 6] = [1, 2, 4, 8, 16, 32];

    /// 归一化拍号：分子夹到 `1..=32`，分母吸附到最近的合法值（并列时取较小者）。
    ///
    /// 手写工程、旧文件或 AI 传入的越界拍号都必须经过这里，避免下游按
    /// `num/den` 计算小节长度时出现 0 或除不尽的情况。
    pub fn normalize_time_sig(num: u8, den: u8) -> (u8, u8) {
        let num = num.clamp(1, 32);
        let den = if Self::TIME_SIG_DENS.contains(&den) {
            den
        } else {
            let mut best = 4u8;
            let mut best_dist = u32::MAX;
            for &candidate in &Self::TIME_SIG_DENS {
                let dist = (candidate as i32 - den as i32).unsigned_abs();
                if dist < best_dist {
                    best_dist = dist;
                    best = candidate;
                }
            }
            best
        };
        (num, den)
    }
}

#[cfg(test)]
mod tests {
    use super::Transport;

    #[test]
    fn keeps_valid_time_signatures_untouched() {
        assert_eq!(Transport::normalize_time_sig(4, 4), (4, 4));
        assert_eq!(Transport::normalize_time_sig(7, 8), (7, 8));
        assert_eq!(Transport::normalize_time_sig(3, 2), (3, 2));
    }

    #[test]
    fn clamps_numerator_and_snaps_denominator() {
        assert_eq!(Transport::normalize_time_sig(0, 4), (1, 4));
        assert_eq!(Transport::normalize_time_sig(200, 4), (32, 4));
        // 5 到 4 与 8 等距，取较小者 4。
        assert_eq!(Transport::normalize_time_sig(4, 5), (4, 4));
        assert_eq!(Transport::normalize_time_sig(4, 0), (4, 1));
        assert_eq!(Transport::normalize_time_sig(4, 9), (4, 8));
        assert_eq!(Transport::normalize_time_sig(4, 100), (4, 32));
    }
}
