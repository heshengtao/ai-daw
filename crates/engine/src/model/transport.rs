use serde::{Deserialize, Serialize};

/// 时间轴上的一个关键点：从 `beat` 起，BPM 与拍号切换到给定值，直到下一个关键点。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub beat: f64,
    pub tempo: f64,
    pub time_sig_num: u8,
    pub time_sig_den: u8,
}

/// 传输设置。
///
/// 注意：因为 `keyframes` 是 `Vec`，`Transport` 不再是 `Copy`，仅实现 `Clone`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transport {
    pub tempo: f64,
    pub time_sig_num: u8,
    pub time_sig_den: u8,
    /// 速度/拍号关键点（按 beat 升序；beat 均 > 0）。第 0 拍由上面三个基准字段承载。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyframes: Vec<Keyframe>,
}

impl Default for Transport {
    fn default() -> Self {
        Transport { tempo: 120.0, time_sig_num: 4, time_sig_den: 4, keyframes: vec![] }
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

    /// 关键点合法的最小 beat：第 0 拍由基准字段承载，关键点必须在 0 之后。
    pub const MIN_KEYFRAME_BEAT: f64 = 1e-6;

    /// 归一化单个关键点；beat 非法时返回 `None`。
    pub fn normalize_keyframe(kf: Keyframe) -> Option<Keyframe> {
        if !kf.beat.is_finite() || kf.beat <= Self::MIN_KEYFRAME_BEAT {
            return None;
        }
        let (num, den) = Self::normalize_time_sig(kf.time_sig_num, kf.time_sig_den);
        let tempo = if kf.tempo.is_finite() { kf.tempo.clamp(1.0, 1000.0) } else { 120.0 };
        Some(Keyframe { beat: kf.beat, tempo, time_sig_num: num, time_sig_den: den })
    }

    /// 插入或覆盖关键点（同 beat 容差内视为同一个），并保持按 beat 升序。
    pub fn upsert_keyframe(&mut self, kf: Keyframe) {
        let Some(kf) = Self::normalize_keyframe(kf) else { return };
        if let Some(slot) = self
            .keyframes
            .iter_mut()
            .find(|k| (k.beat - kf.beat).abs() <= Self::MIN_KEYFRAME_BEAT)
        {
            *slot = kf;
        } else {
            self.keyframes.push(kf);
        }
        self.keyframes.sort_by(|a, b| a.beat.partial_cmp(&b.beat).unwrap_or(std::cmp::Ordering::Equal));
    }

    /// 删除指定 beat 处的关键点（容差内匹配）。
    pub fn remove_keyframe(&mut self, beat: f64) {
        self.keyframes.retain(|k| (k.beat - beat).abs() > Self::MIN_KEYFRAME_BEAT);
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

    #[test]
    fn keyframes_are_normalized_sorted_and_deduped() {
        use super::{Keyframe, Transport};
        let mut t = Transport::default();
        // 越界 beat / tempo / 拍号都要被处理。
        t.upsert_keyframe(Keyframe { beat: 8.0, tempo: 5000.0, time_sig_num: 0, time_sig_den: 0 });
        t.upsert_keyframe(Keyframe { beat: 2.0, tempo: 90.0, time_sig_num: 3, time_sig_den: 4 });
        t.upsert_keyframe(Keyframe { beat: 0.0, tempo: 100.0, time_sig_num: 4, time_sig_den: 4 }); // 被拒
        assert_eq!(t.keyframes.len(), 2, "beat<=0 的关键点不得进入列表");
        assert_eq!(t.keyframes[0].beat, 2.0, "必须按 beat 升序");
        assert_eq!(t.keyframes[1].beat, 8.0);
        assert_eq!(t.keyframes[1].tempo, 1000.0, "tempo 夹到 1000");
        assert_eq!((t.keyframes[1].time_sig_num, t.keyframes[1].time_sig_den), (1, 1));

        // 同 beat 覆盖（容差内视为同一个）。
        t.upsert_keyframe(Keyframe { beat: 2.0 + 1e-9, tempo: 60.0, time_sig_num: 6, time_sig_den: 8 });
        assert_eq!(t.keyframes.len(), 2);
        assert_eq!(t.keyframes[0].tempo, 60.0);
        assert_eq!((t.keyframes[0].time_sig_num, t.keyframes[0].time_sig_den), (6, 8));

        t.remove_keyframe(2.0);
        assert_eq!(t.keyframes.len(), 1);
        assert_eq!(t.keyframes[0].beat, 8.0);
    }

    #[test]
    fn transport_without_keyframes_roundtrips_and_omits_the_field() {
        let t = Transport::default();
        let json = serde_json::to_string(&t).unwrap();
        assert!(!json.contains("keyframes"), "空列表不该写进 JSON");
        let back: Transport = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }
}
