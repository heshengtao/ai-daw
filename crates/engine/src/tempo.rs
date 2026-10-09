//! 拍 ↔ 秒的分段换算：整首曲子的速度/拍号可以有关键点。

use crate::model::Transport;

/// 一段「分段常量」：从 `start_beat` 起用 `tempo`/`num`/`den`，直到下一段的 `start_beat`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub start_beat: f64,
    pub tempo: f64,
    pub num: u8,
    pub den: u8,
}

/// 拍 ↔ 秒的分段换算表。无关键点时退化为常量换算（与老公式逐位一致）。
#[derive(Debug, Clone, PartialEq)]
pub struct TempoMap {
    segments: Vec<Segment>,
}

impl TempoMap {
    /// 从工程传输设置构建；关键点按 beat 升序（第 0 段始终来自基准字段）。
    pub fn from_transport(t: &Transport) -> Self {
        let mut segments = vec![Segment {
            start_beat: 0.0,
            tempo: t.tempo.max(1e-6),
            num: t.time_sig_num.max(1),
            den: t.time_sig_den.max(1),
        }];
        let mut kfs = t.keyframes.clone();
        kfs.sort_by(|a, b| a.beat.partial_cmp(&b.beat).unwrap_or(std::cmp::Ordering::Equal));
        for kf in kfs {
            if kf.beat <= 0.0 {
                continue;
            }
            segments.push(Segment {
                start_beat: kf.beat,
                tempo: kf.tempo.max(1e-6),
                num: kf.time_sig_num.max(1),
                den: kf.time_sig_den.max(1),
            });
        }
        TempoMap { segments }
    }

    /// 所有分段（第 0 段起始拍恒为 0）。
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// `beat` 处生效的分段。
    pub fn segment_at(&self, beat: f64) -> &Segment {
        let beat = if beat > 0.0 { beat } else { 0.0 };
        let mut found = &self.segments[0];
        for s in &self.segments {
            if s.start_beat <= beat {
                found = s;
            } else {
                break;
            }
        }
        found
    }

    /// `beat` 处生效的 BPM。
    pub fn tempo_at(&self, beat: f64) -> f64 {
        self.segment_at(beat).tempo
    }

    /// `beat` 处生效的拍号。
    pub fn meter_at(&self, beat: f64) -> (u8, u8) {
        let s = self.segment_at(beat);
        (s.num, s.den)
    }

    /// 拍 → 秒（分段积分）。
    pub fn beats_to_seconds(&self, beats: f64) -> f64 {
        if !(beats > 0.0) {
            return 0.0;
        }
        let mut secs = 0.0;
        for (i, s) in self.segments.iter().enumerate() {
            let start = s.start_beat;
            if beats <= start {
                break;
            }
            let next = self.segments.get(i + 1).map(|n| n.start_beat);
            let end = match next {
                Some(n) if beats > n => n,
                _ => beats,
            };
            secs += (end - start) * 60.0 / s.tempo;
            if next.map(|n| beats <= n).unwrap_or(false) {
                break;
            }
        }
        secs
    }

    /// 秒 → 拍（分段反解）。最后一段视为无限长，因此任何有限 `secs` 都能落到某段内。
    pub fn seconds_to_beats(&self, secs: f64) -> f64 {
        if !(secs > 0.0) {
            return 0.0;
        }
        let mut acc = 0.0;
        for (i, s) in self.segments.iter().enumerate() {
            let next = self.segments.get(i + 1).map(|n| n.start_beat);
            let seg_secs = match next {
                Some(n) => (n - s.start_beat) * 60.0 / s.tempo,
                None => f64::INFINITY,
            };
            if secs <= acc + seg_secs {
                return s.start_beat + (secs - acc) * s.tempo / 60.0;
            }
            acc += seg_secs;
        }
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Keyframe, Transport};

    #[test]
    fn constant_transport_matches_the_old_formula() {
        let t = Transport::default(); // 120 BPM
        let m = TempoMap::from_transport(&t);
        assert!((m.beats_to_seconds(4.0) - 2.0).abs() < 1e-12);
        assert!((m.seconds_to_beats(2.0) - 4.0).abs() < 1e-12);
        assert_eq!(m.tempo_at(999.0), 120.0);
        assert_eq!(m.meter_at(999.0), (4, 4));
    }

    #[test]
    fn keyframe_switches_tempo_and_meter_from_its_beat() {
        let mut t = Transport::default(); // 120, 4/4
        t.upsert_keyframe(Keyframe { beat: 4.0, tempo: 240.0, time_sig_num: 3, time_sig_den: 4 });
        let m = TempoMap::from_transport(&t);
        assert_eq!(m.tempo_at(0.0), 120.0);
        assert_eq!(m.tempo_at(4.0), 240.0);
        assert_eq!(m.tempo_at(100.0), 240.0);
        assert_eq!(m.meter_at(4.0), (3, 4));
        // 0..4 拍 @120 = 2s；4..8 拍 @240 = 1s → 8 拍共 3s。
        assert!((m.beats_to_seconds(8.0) - 3.0).abs() < 1e-12);
        assert!((m.seconds_to_beats(3.0) - 8.0).abs() < 1e-9);
    }

    #[test]
    fn seconds_to_beats_inverts_beats_to_seconds() {
        let mut t = Transport::default();
        t.upsert_keyframe(Keyframe { beat: 2.0, tempo: 60.0, time_sig_num: 4, time_sig_den: 4 });
        t.upsert_keyframe(Keyframe { beat: 7.5, tempo: 180.0, time_sig_num: 4, time_sig_den: 4 });
        let m = TempoMap::from_transport(&t);
        for beat in [0.0, 0.5, 2.0, 3.25, 7.5, 12.0] {
            let back = m.seconds_to_beats(m.beats_to_seconds(beat));
            assert!((back - beat).abs() < 1e-9, "beat={beat} back={back}");
        }
    }
}
