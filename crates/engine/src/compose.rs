//! 编曲变换与统计：**纯函数**，不碰工程、不碰实时线程。
//!
//! 所有变换都是 `(片段, 参数) → 一串音符改写项`，由命令层在**编辑线程**上算好，再把结果
//! 写进事件。这样做的两个好处：
//!
//! 1. 播放调度拿到的永远是已经归一化的数据，实时线程里没有任何分配或分支；
//! 2. 事件自带结果（`NotesChanged { entries }`），撤销/重做靠重放即可，不必再算一遍——
//!    随机化的 `humanize` 也因此**不会在重做时漂移**。
//!
//! 约束（全部在 [`clamp_into_clip`] 里实现）：起点不得早于片段起点、时值必须为正、
//! 变换后不得越出片段范围。被夹住的音符数量会返回给调用方（`clamped`）。

use crate::model::{Clip, ClipId, Note, NoteEdit, NotePatch, NoteRange, Track, VelocityShape};

/// 一次变换的结果。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Transform {
    /// 需要写回的音符改写项（按片段内下标升序）。
    pub edits: Vec<NoteEdit>,
    /// 被夹进片段范围的音符数（起点或时值被改动）。
    pub clamped: usize,
}

impl Transform {
    /// 是否真的改了东西。
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }
}

/// 量化网格的合法下界（1/64 拍）与上界（4 拍）。
pub const MIN_GRID: f64 = 1.0 / 64.0;
pub const MAX_GRID: f64 = 4.0;

/// 把一个音符夹进片段范围；返回是否改动过。
///
/// 只对**本来就在片段内**（起点 >= 0）的音符动手：被左边缘裁掉的音符（起点为负）是
/// 用户「收起来」的内容，变换不该把它们拽回来。
fn clamp_into_clip(note: Note, clip_length: f64) -> (Note, bool) {
    if note.start < 0.0 {
        return (note, false);
    }
    let mut out = note;
    let mut clamped = false;
    let max_start = (clip_length - Note::MIN_DURATION_BEATS).max(0.0);
    if out.start > max_start {
        out.start = max_start;
        clamped = true;
    }
    let max_duration = (clip_length - out.start).max(Note::MIN_DURATION_BEATS);
    if out.duration > max_duration {
        out.duration = max_duration;
        clamped = true;
    }
    (out, clamped)
}

/// 范围过滤：`range` 为 `None` 时作用于整个片段。
fn in_range(note: &Note, range: Option<NoteRange>) -> bool {
    match range {
        Some(r) => r.normalize().contains(note),
        None => true,
    }
}

/// 把一串「新音符」收成改写项，同时统计被夹住的个数。
fn collect(clip: &Clip, updates: impl IntoIterator<Item = (usize, Note)>) -> Transform {
    let mut out = Transform::default();
    for (index, note) in updates {
        let (note, clamped) = clamp_into_clip(note, clip.length);
        if clamped {
            out.clamped += 1;
        }
        // 值没变就不写：一条空改写会让事件流噪声变大，也会让撤销历史里出现「什么都没改」。
        if clip.notes.get(index) != Some(&note) {
            out.edits.push(NoteEdit { index, note });
        }
    }
    out
}

/// 批量移调（半音）。音高夹在 0..=127。
pub fn transpose(clip: &Clip, semitones: i32, range: Option<NoteRange>) -> Transform {
    if semitones == 0 {
        return Transform::default();
    }
    collect(
        clip,
        clip.notes.iter().enumerate().filter(|(_, n)| in_range(n, range)).map(|(i, n)| {
            let pitch = (n.pitch as i32 + semitones).clamp(0, 127) as u8;
            (i, Note { pitch, ..*n })
        }),
    )
}

/// 按 **id** 批量改写：只改给出的字段。
pub fn update_by_id(clip: &Clip, updates: &[NotePatch]) -> Transform {
    collect(
        clip,
        updates.iter().filter_map(|patch| {
            let index = clip.note_index(patch.id)?;
            Some((index, patch.apply_to(clip.notes[index])))
        }),
    )
}

/// 批量设置力度。
///
/// - `Fixed(v)`：全部设为 `v`；
/// - `Ramp { from, to }`：在**范围内**按拍位线性变化（没有范围时按音符首尾跨度）；
/// - `Scale(f)`：相对缩放。
pub fn set_velocity(clip: &Clip, shape: VelocityShape, range: Option<NoteRange>) -> Transform {
    let selected: Vec<usize> =
        clip.notes.iter().enumerate().filter(|(_, n)| in_range(n, range)).map(|(i, _)| i).collect();
    if selected.is_empty() {
        return Transform::default();
    }
    // 渐变需要一个起止拍：优先用显式范围，否则用这批音符自己的跨度。
    let (lo, hi) = match (shape, range.map(|r| r.normalize())) {
        (VelocityShape::Ramp { .. }, Some(r)) => (r.start, r.end),
        (VelocityShape::Ramp { .. }, None) => {
            let lo = selected.iter().map(|i| clip.notes[*i].start).fold(f64::INFINITY, f64::min);
            let hi = selected.iter().map(|i| clip.notes[*i].start).fold(f64::NEG_INFINITY, f64::max);
            (lo, hi)
        }
        _ => (0.0, 1.0),
    };
    collect(
        clip,
        selected.into_iter().map(|i| {
            let note = clip.notes[i];
            let velocity = match shape {
                VelocityShape::Fixed(v) => v as f64,
                VelocityShape::Scale(f) => note.velocity as f64 * f,
                VelocityShape::Ramp { from, to } => {
                    let span = hi - lo;
                    let t = if span > 0.0 { ((note.start - lo) / span).clamp(0.0, 1.0) } else { 0.0 };
                    from as f64 + (to as f64 - from as f64) * t
                }
            };
            (i, Note { velocity: clamp_velocity(velocity), ..note })
        }),
    )
}

/// 浮点力度 → 0..=127 的整数（四舍五入，越界夹住）。
fn clamp_velocity(value: f64) -> u8 {
    if !value.is_finite() {
        return 0;
    }
    value.round().clamp(0.0, 127.0) as u8
}

/// 量化：把起点吸到 `grid` 的整数倍。`strength` 是「吸多少」（0 = 不动，1 = 完全对齐）。
pub fn quantize(clip: &Clip, grid: f64, strength: f64, range: Option<NoteRange>) -> Transform {
    let grid = if grid.is_finite() { grid.clamp(MIN_GRID, MAX_GRID) } else { MIN_GRID };
    let strength = if strength.is_finite() { strength.clamp(0.0, 1.0) } else { 0.0 };
    if strength <= 0.0 {
        return Transform::default();
    }
    collect(
        clip,
        clip.notes.iter().enumerate().filter(|(_, n)| in_range(n, range)).map(|(i, n)| {
            let target = (n.start / grid).round() * grid;
            let start = n.start + (target - n.start) * strength;
            (i, Note { start, ..*n })
        }),
    )
}

/// Swing：把每对子拍里的**第二个**往后推。
///
/// `subdivision` 是子拍长度（8 分 swing = 0.5 拍），`ratio` 是第二个子拍落在一对子拍里的
/// 位置比例（0.5 = 平直，0.66 ≈ 三连音感，0.75 = 硬 shuffle）。`strength` 是应用程度。
///
/// 位置公式：一对子拍占 `2 * subdivision`，第二个子拍的目标位置是
/// `pair_start + ratio * 2 * subdivision`——8 分 swing 在 ratio=0.66 时正好落到约 2/3 拍。
pub fn swing(
    clip: &Clip,
    subdivision: f64,
    ratio: f64,
    strength: f64,
    range: Option<NoteRange>,
) -> Transform {
    let subdivision =
        if subdivision.is_finite() { subdivision.clamp(MIN_GRID, MAX_GRID) } else { 0.5 };
    let ratio = if ratio.is_finite() { ratio.clamp(0.5, 0.8) } else { 0.5 };
    let strength = if strength.is_finite() { strength.clamp(0.0, 1.0) } else { 0.0 };
    if strength <= 0.0 || (ratio - 0.5).abs() < 1e-9 {
        return Transform::default();
    }
    let pair = 2.0 * subdivision;
    collect(
        clip,
        clip.notes.iter().enumerate().filter(|(_, n)| in_range(n, range)).map(|(i, n)| {
            if n.start < 0.0 {
                return (i, *n);
            }
            // 落在第几个子拍；偶数位（第一个子拍）不动。
            let slot = (n.start / subdivision).floor() as i64;
            if slot % 2 == 0 {
                return (i, *n);
            }
            let pair_start = (slot - 1) as f64 * subdivision;
            let target = pair_start + ratio * pair;
            let start = n.start + (target - n.start) * strength;
            (i, Note { start, ..*n })
        }),
    )
}

/// 确定性伪随机（splitmix64）：同一个 `seed` 永远给同一串偏移。
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `-1.0..=1.0` 的均匀分布。
    fn bipolar(&mut self) -> f64 {
        let unit = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        unit * 2.0 - 1.0
    }
}

/// 人性化：给位置与力度加**确定性**抖动。
///
/// `timing` 是位置抖动的最大绝对值（拍），`velocity` 是力度抖动的最大绝对值。
/// 音符按片段内顺序取随机数，`seed` 相同则结果逐字相同——因此重做不会漂。
pub fn humanize(
    clip: &Clip,
    timing: f64,
    velocity: f64,
    range: Option<NoteRange>,
    seed: Option<u64>,
) -> Transform {
    let timing = if timing.is_finite() { timing.clamp(0.0, 0.5) } else { 0.0 };
    let velocity = if velocity.is_finite() { velocity.clamp(0.0, 127.0) } else { 0.0 };
    if timing <= 0.0 && velocity <= 0.0 {
        return Transform::default();
    }
    let mut rng = Rng(seed.unwrap_or(0x5EED_1234_ABCD_0001));
    let mut updates = Vec::new();
    for (i, n) in clip.notes.iter().enumerate() {
        // 每个音符都取两个随机数（即使不在范围内），这样「范围」不会改变其他音符的抖动。
        let dt = rng.bipolar() * timing;
        let dv = rng.bipolar() * velocity;
        if !in_range(n, range) || n.start < 0.0 {
            continue;
        }
        let start = n.start + dt;
        let vel = clamp_velocity(n.velocity as f64 + dv);
        updates.push((i, Note { start, velocity: vel, ..*n }));
    }
    collect(clip, updates)
}

/// 音域约束的结果。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FitReport {
    pub transform: Transform,
    /// 越界（需要处理）的音符数。
    pub out_of_range: usize,
}

/// 把音符收进 `[min_pitch, max_pitch]`。
///
/// `Fold` 做八度折返：加减整八度，**音级不变**（C 还是 C）；音域窄于一个八度时退化为夹到边界。
/// `Report` 只统计越界数量，不改任何音符。
pub fn fit_to_range(
    clip: &Clip,
    min_pitch: u8,
    max_pitch: u8,
    mode: crate::model::FitMode,
    range: Option<NoteRange>,
) -> FitReport {
    let (lo, hi) = if min_pitch <= max_pitch {
        (min_pitch.min(127), max_pitch.min(127))
    } else {
        (max_pitch.min(127), min_pitch.min(127))
    };
    let selected: Vec<usize> =
        clip.notes.iter().enumerate().filter(|(_, n)| in_range(n, range)).map(|(i, _)| i).collect();
    let out_of_range =
        selected.iter().filter(|i| !(lo..=hi).contains(&clip.notes[**i].pitch)).count();
    if mode == crate::model::FitMode::Report || out_of_range == 0 {
        return FitReport { transform: Transform::default(), out_of_range };
    }
    let mut transform = Transform::default();
    for index in selected {
        let note = clip.notes[index];
        if (lo..=hi).contains(&note.pitch) {
            continue;
        }
        let mut pitch = note.pitch as i32;
        // 八度折返：整八度上下挪，音级（pitch % 12）保持不变。
        while pitch > hi as i32 && pitch - 12 >= 0 {
            pitch -= 12;
        }
        while (pitch as u8) < lo && pitch + 12 <= 127 {
            pitch += 12;
        }
        // 音域窄于八度 / 已经贴边：只能夹到最近的边界（音级会变，但至少合法）。
        let pitch = pitch.clamp(lo as i32, hi as i32) as u8;
        let edited = Note { pitch, ..note };
        if edited != note {
            transform.edits.push(NoteEdit { index, note: edited });
        }
    }
    FitReport { transform, out_of_range }
}

// ---------------------------------------------------------------------------
// 统计
// ---------------------------------------------------------------------------

/// 音符的绝对起止拍（工程坐标）。
fn absolute(note: &Note, clip_start: f64) -> (f64, f64) {
    let start = clip_start + note.start;
    (start, start + note.duration)
}

/// 取出参与统计的片段。
fn clips_of<'a>(track: &'a Track, clip: Option<ClipId>) -> impl Iterator<Item = &'a Clip> {
    track.clips.iter().filter(move |c| clip.map_or(true, |id| c.id == id))
}

/// 每小节的音符密度。
fn bar_density(
    intervals: &[(f64, f64)],
    bar_beats: f64,
) -> (Vec<crate::report::BarDensity>, f64) {
    let bar_beats = if bar_beats.is_finite() && bar_beats > 0.0 { bar_beats } else { 4.0 };
    let mut counts: Vec<usize> = Vec::new();
    for (start, _) in intervals {
        if *start < 0.0 {
            continue;
        }
        let bar = (*start / bar_beats).floor() as usize;
        if counts.len() <= bar {
            counts.resize(bar + 1, 0);
        }
        counts[bar] += 1;
    }
    let bars = counts
        .iter()
        .enumerate()
        .map(|(bar, count)| crate::report::BarDensity {
            bar,
            start: bar as f64 * bar_beats,
            count: *count,
        })
        .collect::<Vec<_>>();
    let mean = if bars.is_empty() {
        0.0
    } else {
        counts.iter().sum::<usize>() as f64 / bars.len() as f64
    };
    (bars, mean)
}

/// 单轨（或单片段）的音符统计。只读，不改工程。
pub fn note_stats(track: &Track, clip: Option<ClipId>, bar_beats: f64) -> crate::report::NoteStats {
    let mut intervals: Vec<(f64, f64)> = Vec::new();
    let mut pitches: Vec<u8> = Vec::new();
    let mut velocities: Vec<u8> = Vec::new();
    let mut outside = 0usize;

    for c in clips_of(track, clip) {
        for n in &c.notes {
            intervals.push(absolute(n, c.start));
            pitches.push(n.pitch);
            velocities.push(n.velocity);
            // 起点在片段之外（负的、或超过片段长度）→ 播放时听不见。
            if n.start < 0.0 || n.start >= c.length {
                outside += 1;
            }
        }
    }

    // 同音高、时间重叠：按音高分组后扫一遍（O(n log n)）。
    let mut by_pitch: Vec<Vec<(f64, f64)>> = vec![Vec::new(); 128];
    for (pitch, span) in pitches.iter().zip(intervals.iter()) {
        by_pitch[*pitch as usize].push(*span);
    }
    let mut overlapping = 0usize;
    for group in &mut by_pitch {
        if group.len() < 2 {
            continue;
        }
        group.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut end = group[0].1;
        for span in &group[1..] {
            if span.0 < end - 1e-9 {
                overlapping += 1;
            }
            end = end.max(span.1);
        }
    }

    let count = intervals.len();
    let lowest = pitches.iter().copied().min();
    let highest = pitches.iter().copied().max();
    let mean_pitch = if count == 0 {
        None
    } else {
        Some(pitches.iter().map(|p| *p as f64).sum::<f64>() / count as f64)
    };
    let mean_velocity = if count == 0 {
        None
    } else {
        Some(velocities.iter().map(|v| *v as f64).sum::<f64>() / count as f64)
    };
    let (bars, mean_notes_per_bar) = bar_density(&intervals, bar_beats);
    let span_beats = intervals
        .iter()
        .map(|(_, e)| *e)
        .fold(f64::NEG_INFINITY, f64::max)
        .max(0.0)
        - intervals.iter().map(|(s, _)| *s).fold(f64::INFINITY, f64::min).min(0.0);

    crate::report::NoteStats {
        track: track.id.0,
        clip: clip.map(|c| c.0),
        note_count: count,
        lowest_pitch: lowest,
        highest_pitch: highest,
        mean_pitch: mean_pitch.map(|v| (v * 100.0).round() / 100.0),
        range_semitones: match (lowest, highest) {
            (Some(lo), Some(hi)) => Some((hi - lo) as u16),
            _ => None,
        },
        mean_velocity: mean_velocity.map(|v| (v * 100.0).round() / 100.0),
        span_beats: (span_beats * 1000.0).round() / 1000.0,
        mean_notes_per_bar: (mean_notes_per_bar * 100.0).round() / 100.0,
        bars,
        overlapping_notes: overlapping,
        notes_outside_clip: outside,
    }
}

/// 合并重叠区间（用于求「两条轨道同时发声」的总时长）。
fn merge(intervals: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut sorted: Vec<(f64, f64)> = intervals.iter().copied().filter(|(s, e)| e > s).collect();
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (s, e) in sorted {
        match out.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

/// 两组区间的交集总长。
fn intersection_len(a: &[(f64, f64)], b: &[(f64, f64)]) -> f64 {
    let (a, b) = (merge(a), merge(b));
    let (mut i, mut j, mut total) = (0usize, 0usize, 0.0);
    while i < a.len() && j < b.len() {
        let start = a[i].0.max(b[j].0);
        let end = a[i].1.min(b[j].1);
        if end > start {
            total += end - start;
        }
        if a[i].1 < b[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    total
}

/// 两条轨道之间的音区重叠与分工冲突判断。
pub fn track_overlap(a: &Track, b: &Track, clip: Option<ClipId>) -> crate::report::TrackOverlap {
    let spans = |track: &Track| -> Vec<(f64, f64, u8)> {
        clips_of(track, clip)
            .flat_map(|c| {
                c.notes.iter().filter(|n| n.start >= 0.0).map(move |n| {
                    let (s, e) = absolute(n, c.start);
                    (s, e, n.pitch)
                })
            })
            .collect()
    };
    let sa = spans(a);
    let sb = spans(b);

    let intervals = |v: &[(f64, f64, u8)]| -> Vec<(f64, f64)> {
        v.iter().map(|(s, e, _)| (*s, *e)).collect()
    };
    let overlap_beats = intersection_len(&intervals(&sa), &intervals(&sb));

    // 完全同音、且同时发声：最容易糊在一起的那种重叠。
    let mut unison = 0usize;
    for (s1, e1, p1) in &sa {
        for (s2, e2, p2) in &sb {
            if p1 == p2 && s1 < e2 && s2 < e1 {
                unison += 1;
            }
        }
    }

    let pcs = |v: &[(f64, f64, u8)]| -> Vec<u8> {
        let mut out: Vec<u8> = v.iter().map(|(_, _, p)| p % 12).collect();
        out.sort_unstable();
        out.dedup();
        out
    };
    let (pa, pb) = (pcs(&sa), pcs(&sb));
    let shared: Vec<u8> = pa.iter().copied().filter(|p| pb.contains(p)).collect();

    let mean = |v: &[(f64, f64, u8)]| -> Option<f64> {
        if v.is_empty() {
            None
        } else {
            Some(v.iter().map(|(_, _, p)| *p as f64).sum::<f64>() / v.len() as f64)
        }
    };
    let gap = match (mean(&sa), mean(&sb)) {
        (Some(x), Some(y)) => Some((x - y).round() as i32),
        _ => None,
    };

    // 同音区 + 大量同时发声就是分工冲突：即使音高不同，两条线挤在一个八度里也会互相盖住。
    let same_register = gap.map_or(false, |g| g.abs() < 12);
    let conflict = overlap_beats >= 4.0 && same_register;

    let suggestion = if conflict {
        let role_a = a.role().unwrap_or("未标注分工");
        let role_b = b.role().unwrap_or("未标注分工");
        format!(
            "轨道 {}（{}）与轨道 {}（{}）在相近音区同时发声 {:.1} 拍（其中同音 {} 处）——\
             建议把其中一条上下移一个八度、或把其中一条的音域收窄到不同音区。",
            a.id.0, role_a, b.id.0, role_b, overlap_beats, unison
        )
    } else if !shared.is_empty() && same_register {
        format!(
            "轨道 {} 与 {} 音区接近但不构成明显冲突（同时发声 {:.1} 拍）。",
            a.id.0, b.id.0, overlap_beats
        )
    } else {
        String::new()
    };

    crate::report::TrackOverlap {
        a: a.id.0,
        b: b.id.0,
        overlap_beats: (overlap_beats * 1000.0).round() / 1000.0,
        unison_notes: unison,
        shared_pitch_classes: shared,
        pitch_gap_semitones: gap,
        conflict,
        suggestion,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ClipId, FitMode, TrackId, TrackMeta};

    fn clip(notes: Vec<Note>, length: f64) -> Clip {
        Clip { id: ClipId(1), start: 0.0, length, notes, audio: None }
    }

    fn note(id: u64, pitch: u8, start: f64) -> Note {
        Note { id, pitch, start, duration: 0.25, velocity: 100, volume: 127 }
    }

    fn track(notes: Vec<Note>) -> Track {
        Track {
            id: TrackId(1),
            name: "Lead".into(),
            instrument: None,
            effects: vec![],
            clips: vec![clip(notes, 16.0)],
            mixer: Default::default(),
            sends: vec![],
            automation: vec![],
            meta: None,
        }
    }

    #[test]
    fn quantize_strength_zero_is_a_no_op_and_one_snaps() {
        let c = clip(vec![note(1, 60, 0.1), note(2, 62, 1.4)], 4.0);
        assert!(quantize(&c, 0.5, 0.0, None).is_empty());
        let t = quantize(&c, 0.5, 1.0, None);
        assert_eq!(t.edits[0].note.start, 0.0);
        assert_eq!(t.edits[1].note.start, 1.5);
        // 一半强度：落在原值与目标之间。
        let half = quantize(&c, 0.5, 0.5, None);
        assert!((half.edits[0].note.start - 0.05).abs() < 1e-9);
    }

    #[test]
    fn swing_moves_only_the_second_subdivision() {
        let c = clip(vec![note(1, 60, 0.0), note(2, 62, 0.5), note(3, 64, 1.0)], 4.0);
        let t = swing(&c, 0.5, 0.66, 1.0, None);
        // 第一个子拍（0.0 / 1.0）不动。
        assert!(t.edits.iter().all(|e| e.index != 0 && e.index != 2), "{:?}", t.edits);
        // 第二个子拍落到约 2/3 拍。
        assert!((t.edits[0].note.start - 0.66).abs() < 1e-9, "{:?}", t.edits[0].note.start);
    }

    #[test]
    fn swing_strength_scales_the_shift() {
        let c = clip(vec![note(1, 60, 0.5)], 4.0);
        let t = swing(&c, 0.5, 0.75, 0.5, None);
        // 目标 0.75，起点 0.5 → 一半强度 = 0.625。
        assert!((t.edits[0].note.start - 0.625).abs() < 1e-9);
    }

    #[test]
    fn humanize_is_deterministic_and_bounded() {
        let c = clip(vec![note(1, 60, 1.0), note(2, 62, 2.0)], 8.0);
        let a = humanize(&c, 0.05, 10.0, None, Some(7));
        let b = humanize(&c, 0.05, 10.0, None, Some(7));
        assert_eq!(a, b, "同一个 seed 必须逐字一致");
        for e in &a.edits {
            assert!((e.note.start - c.notes[e.index].start).abs() <= 0.05 + 1e-9);
        }
        assert_ne!(humanize(&c, 0.05, 10.0, None, Some(8)), a, "换 seed 应该不一样");
    }

    #[test]
    fn fit_to_range_folds_by_octaves_without_changing_pitch_class() {
        let c = clip(vec![note(1, 80, 0.0), note(2, 30, 1.0), note(3, 60, 2.0)], 4.0);
        let report = fit_to_range(&c, 55, 72, FitMode::Fold, None);
        assert_eq!(report.out_of_range, 2);
        let pitches: Vec<u8> = report.transform.edits.iter().map(|e| e.note.pitch).collect();
        assert_eq!(pitches, vec![68, 66], "80→68、30→66（+12 直到落进音域），音级不变");
        assert_eq!(68 % 12, 80 % 12);
        assert_eq!(66 % 12, 30 % 12);
    }

    #[test]
    fn fit_to_range_report_mode_never_edits() {
        let c = clip(vec![note(1, 80, 0.0)], 4.0);
        let report = fit_to_range(&c, 55, 72, FitMode::Report, None);
        assert_eq!(report.out_of_range, 1);
        assert!(report.transform.is_empty());
    }

    #[test]
    fn transforms_clamp_into_the_clip() {
        // 起点 3.9 的 1 拍音符在 4 拍的片段里放不下：时值会被夹到 0.1。
        let c = clip(vec![Note { id: 1, pitch: 60, start: 3.9, duration: 1.0, velocity: 100, volume: 127 }], 4.0);
        let t = quantize(&c, 1.0, 1.0, None);
        assert_eq!(t.clamped, 1);
        let n = t.edits[0].note;
        assert!((n.start + n.duration - 4.0).abs() < 1e-9, "{n:?}");
    }

    #[test]
    fn transpose_clamps_pitch_and_skips_unchanged_notes() {
        let c = clip(vec![note(1, 120, 0.0), note(2, 60, 1.0)], 4.0);
        let t = transpose(&c, 12, None);
        assert_eq!(t.edits.len(), 2);
        assert_eq!(t.edits[0].note.pitch, 127, "越界音高被夹住");
        assert!(transpose(&c, 0, None).is_empty());
    }

    #[test]
    fn range_limits_which_notes_move() {
        let c = clip(vec![note(1, 60, 0.0), note(2, 62, 2.0)], 4.0);
        let t = transpose(&c, 12, Some(NoteRange { start: 1.0, end: 4.0 }));
        assert_eq!(t.edits.len(), 1);
        assert_eq!(t.edits[0].index, 1);
    }

    #[test]
    fn velocity_shapes() {
        let c = clip(vec![note(1, 60, 0.0), note(2, 62, 1.0)], 4.0);
        let fixed = set_velocity(&c, VelocityShape::Fixed(64), None);
        assert!(fixed.edits.iter().all(|e| e.note.velocity == 64));

        let scaled = set_velocity(&c, VelocityShape::Scale(0.5), None);
        assert_eq!(scaled.edits[0].note.velocity, 50);

        let ramp = set_velocity(
            &c,
            VelocityShape::Ramp { from: 20, to: 100 },
            Some(NoteRange { start: 0.0, end: 2.0 }),
        );
        // 范围 0..2 拍：0 拍处 = from，1 拍处 = 中点 60。
        assert_eq!(ramp.edits[0].note.velocity, 20);
        assert_eq!(ramp.edits[1].note.velocity, 60);
    }

    #[test]
    fn stats_report_range_density_and_overlap() {
        let t = track(vec![
            Note { id: 1, pitch: 60, start: 0.0, duration: 2.0, velocity: 100, volume: 127 },
            Note { id: 2, pitch: 60, start: 1.0, duration: 1.0, velocity: 80, volume: 127 },
            Note { id: 3, pitch: 72, start: 5.0, duration: 1.0, velocity: 90, volume: 127 },
        ]);
        let s = note_stats(&t, None, 4.0);
        assert_eq!(s.note_count, 3);
        assert_eq!(s.lowest_pitch, Some(60));
        assert_eq!(s.highest_pitch, Some(72));
        assert_eq!(s.range_semitones, Some(12));
        assert_eq!(s.overlapping_notes, 1, "同音高 60 重叠一次");
        assert_eq!(s.bars.len(), 2, "第 1 小节 2 个、第 2 小节 1 个");
        assert_eq!(s.bars[0].count, 2);
        assert_eq!(s.bars[1].count, 1);
    }

    #[test]
    fn overlap_flags_two_tracks_in_the_same_register() {
        let a = track(vec![Note { id: 1, pitch: 60, start: 0.0, duration: 8.0, velocity: 100, volume: 127 }]);
        let mut b = track(vec![Note { id: 1, pitch: 64, start: 0.0, duration: 8.0, velocity: 100, volume: 127 }]);
        b.id = TrackId(2);
        b.meta = Some(TrackMeta { role: Some("chords".into()), pitch_range: None });
        let o = track_overlap(&a, &b, None);
        assert!((o.overlap_beats - 8.0).abs() < 1e-9);
        assert!(o.conflict, "同音区同时发声 8 拍应当判为冲突：{o:?}");
        assert!(!o.suggestion.is_empty());
    }
}
