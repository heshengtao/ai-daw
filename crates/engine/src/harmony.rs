//! 和弦符号解析与和声检查。
//!
//! 两件事分开：
//! - [`parse_chord`] 只做**符号 → 构成音**的翻译，无法识别就返回明确的错误（不静默放宽）；
//! - [`analyze`] 拿和弦表 + 轨道，逐音判定，输出**问题列表**——它**不改任何音符**。
//!   理论判断有上下文依赖（经过音、悬挂音、风格习惯），所以 v1 的定位是「指出问题 + 解释」，
//!   由 AI 或人决定怎么改。
//!
//! 支持的符号：大三 / 小三 / 减 / 增 / sus / 5 / 6 / 7 / maj7 / m7 / m7b5 / dim7 / 9 / 11 / 13 /
//! 6/9，以及常见的 `b5` `#5` `b9` `#9` `#11` `b13` `add9` `no3` 等修饰与斜杠低音。

use crate::model::{HarmonyBar, Track};
use crate::report::{
    pitch_class_name, pitch_name, HarmonyBarReport, HarmonyIssue, HarmonyIssueKind, HarmonyPolicy,
    HarmonyReport, Severity,
};

/// 一个已解析的和弦。
#[derive(Debug, Clone, PartialEq)]
pub struct Chord {
    /// 原始符号（原样回显）。
    pub symbol: String,
    /// 根音音级（0..=11，C=0）。
    pub root: u8,
    /// 低音音级（斜杠和弦；缺省 = 根音）。
    pub bass: u8,
    /// 相对根音的半音（升序，含 0）。
    pub intervals: Vec<i32>,
    /// 和弦类型名（用于报告）。
    pub quality: String,
}

impl Chord {
    /// 构成音的音级集合（去重、升序）。
    pub fn pitch_classes(&self) -> Vec<u8> {
        let mut out: Vec<u8> =
            self.intervals.iter().map(|i| ((self.root as i32 + i).rem_euclid(12)) as u8).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// 该音级是否属于和弦构成音。
    pub fn contains(&self, pc: u8) -> bool {
        self.pitch_classes().contains(&(pc % 12))
    }

    /// 相对根音的音程（0..=11）。
    pub fn degree(&self, pc: u8) -> u8 {
        ((pc as i32 - self.root as i32).rem_euclid(12)) as u8
    }

    /// 构成音的音名（报告里直接可读）。
    pub fn tone_names(&self) -> Vec<String> {
        self.pitch_classes().into_iter().map(pitch_class_name).collect()
    }

    /// 是否是属和弦（大三度 + 小七度）。
    pub fn is_dominant(&self) -> bool {
        self.intervals.contains(&4) && self.intervals.contains(&10)
    }

    /// 是否是小和弦（小三度）。
    pub fn is_minor(&self) -> bool {
        self.intervals.contains(&3)
    }

    /// 是否是大和弦（大三度且没有小七度）。
    pub fn is_major(&self) -> bool {
        self.intervals.contains(&4) && !self.intervals.contains(&10)
    }
}

/// 基础和弦表：**按前缀长度从长到短**排列（`maj7` 必须先于 `m`）。
const BASES: &[(&str, &[i32])] = &[
    ("maj13", &[0, 4, 7, 11, 14, 17, 21]),
    ("maj11", &[0, 4, 7, 11, 14, 17]),
    ("maj9", &[0, 4, 7, 11, 14]),
    ("maj7", &[0, 4, 7, 11]),
    ("m13", &[0, 3, 7, 10, 14, 17, 21]),
    ("m11", &[0, 3, 7, 10, 14, 17]),
    ("m9", &[0, 3, 7, 10, 14]),
    ("m7", &[0, 3, 7, 10]),
    ("m6", &[0, 3, 7, 9]),
    ("m", &[0, 3, 7]),
    ("13", &[0, 4, 7, 10, 14, 17, 21]),
    ("11", &[0, 4, 7, 10, 14, 17]),
    ("9", &[0, 4, 7, 10, 14]),
    ("7sus4", &[0, 5, 7, 10]),
    ("7", &[0, 4, 7, 10]),
    ("69", &[0, 4, 7, 9, 14]),
    ("6", &[0, 4, 7, 9]),
    ("dim7", &[0, 3, 6, 9]),
    ("dim", &[0, 3, 6]),
    ("aug", &[0, 4, 8]),
    ("sus2", &[0, 2, 7]),
    ("sus4", &[0, 5, 7]),
    ("sus", &[0, 5, 7]),
    ("5", &[0, 7]),
    ("", &[0, 4, 7]),
];

/// 和弦修饰符（按长度从长到短匹配）。
const MODIFIERS: &[&str] = &[
    "add13", "add11", "add9", "add2", "add4", "omit3", "omit5", "no3", "no5", "alt", "b13", "#11",
    "b9", "#9", "b5", "#5", "b6",
];

/// 解析根音（字母 + 任意个升降号）；返回音级与消耗的字节数。
fn parse_root(text: &str) -> Option<(u8, usize)> {
    let mut chars = text.char_indices();
    let (_, letter) = chars.next()?;
    let base = match letter.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let mut pc = base as i32;
    let mut consumed = letter.len_utf8();
    for (index, c) in chars {
        match c {
            '#' | '♯' => {
                pc += 1;
                consumed = index + c.len_utf8();
            }
            'b' | '♭' => {
                pc -= 1;
                consumed = index + c.len_utf8();
            }
            _ => break,
        }
    }
    Some(((pc.rem_euclid(12)) as u8, consumed))
}

/// 解析和弦符号。
pub fn parse_chord(symbol: &str) -> Result<Chord, String> {
    let raw = symbol.trim();
    if raw.is_empty() {
        return Err("和弦符号为空".into());
    }
    // 归一化：`6/9` 不是斜杠和弦；unicode 记号换成 ASCII；`M7` 是大七度。
    let mut normalized = raw
        .replace("6/9", "69")
        .replace('♭', "b")
        .replace('♯', "#")
        .replace('Δ', "maj")
        .replace('°', "dim")
        .replace('ø', "m7b5")
        .replace('−', "-");
    if let Some(rest) = normalized.strip_prefix('M') {
        normalized = format!("maj{rest}");
    }
    let normalized = normalized.to_ascii_lowercase();

    let (chord_part, bass_part) = match normalized.split_once('/') {
        Some((a, b)) => (a, Some(b)),
        None => (normalized.as_str(), None),
    };

    let (root, used) = parse_root(chord_part)
        .ok_or_else(|| format!("无法识别的根音：`{raw}`（应以 A–G 开头，可带 # / b）"))?;
    let rest = &chord_part[used..];
    let bass = match bass_part {
        Some(b) => parse_root(b)
            .map(|(pc, _)| pc)
            .ok_or_else(|| format!("无法识别的低音：`{b}`（斜杠后应为音名）"))?,
        None => root,
    };

    // 基础和弦：表已按长度降序，第一个匹配就是最长匹配。
    let (name, intervals) = BASES
        .iter()
        .copied()
        .find(|(name, _)| rest.starts_with(name))
        .ok_or_else(|| format!("无法识别的和弦类型：`{raw}`"))?;
    let mut intervals: Vec<i32> = intervals.to_vec();
    let mut tail = &rest[name.len()..];

    // 修饰符：逐个吃掉。
    while !tail.is_empty() {
        let token = MODIFIERS
            .iter()
            .find(|t| tail.starts_with(**t))
            .copied()
            .ok_or_else(|| {
                format!("无法识别的和弦修饰：`{tail}`（在 `{raw}` 里）；支持 {}", MODIFIERS.join(" / "))
            })?;
        intervals = apply_modifier(intervals, token);
        tail = &tail[token.len()..];
    }

    intervals.sort_unstable();
    intervals.dedup();
    let quality = if name.is_empty() { "major".to_string() } else { (*name).to_string() };

    Ok(Chord { symbol: raw.to_string(), root, bass, intervals, quality })
}

/// 把单个修饰符作用到音程集合上。
fn apply_modifier(mut intervals: Vec<i32>, token: &str) -> Vec<i32> {
    let replace = |list: &mut Vec<i32>, from: i32, to: i32| {
        if let Some(slot) = list.iter_mut().find(|i| **i == from) {
            *slot = to;
        } else {
            list.push(to);
        }
    };
    match token {
        "add9" | "add2" => intervals.push(14),
        "add11" | "add4" => intervals.push(17),
        "add13" => intervals.push(21),
        "no3" | "omit3" => intervals.retain(|i| *i != 3 && *i != 4),
        "no5" | "omit5" => intervals.retain(|i| *i != 6 && *i != 7 && *i != 8),
        "alt" => intervals = vec![0, 4, 8, 10, 13, 15],
        "b5" => replace(&mut intervals, 7, 6),
        "#5" => replace(&mut intervals, 7, 8),
        "b6" => replace(&mut intervals, 7, 8),
        "b9" => intervals.push(13),
        "#9" => intervals.push(15),
        "#11" => intervals.push(18),
        "b13" => intervals.push(20),
        _ => {}
    }
    intervals
}

/// 一个带绝对位置的和弦段。
struct BarChord {
    index: usize,
    start: f64,
    end: f64,
    chord: Chord,
}

/// 把和弦表整理成「按起点排序、带有效区间」的段落。
fn layout(bars: &[HarmonyBar], bar_beats: f64) -> Result<Vec<BarChord>, String> {
    let mut sorted: Vec<&HarmonyBar> = bars.iter().collect();
    sorted.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = Vec::with_capacity(sorted.len());
    for (index, bar) in sorted.iter().enumerate() {
        let chord = parse_chord(&bar.symbol)
            .map_err(|e| format!("第 {} 小节「{}」：{e}", index + 1, bar.symbol))?;
        let start = if bar.start.is_finite() { bar.start.max(0.0) } else { 0.0 };
        let end = if bar.length.is_finite() && bar.length > 0.0 {
            start + bar.length
        } else if let Some(next) = sorted.get(index + 1) {
            next.start.max(start)
        } else {
            start + bar_beats
        };
        out.push(BarChord { index, start, end, chord });
    }
    Ok(out)
}

/// 不推荐的延伸音：返回解释（`None` = 没有额外意见）。
fn unrecommended_extension(chord: &Chord, pc: u8) -> Option<&'static str> {
    let degree = chord.degree(pc);
    match degree {
        5 if chord.is_major() => Some("大三和弦上的纯四度（11 音）会与三音顶牛；除非当经过音或改成 sus"),
        11 if chord.is_minor() => Some("小三和弦上叠大七度极不协和"),
        10 if chord.is_major() => Some("大和弦上叠小七度（属七）会与七音打架"),
        1 if chord.is_major() => Some("大三和弦上的 b9 极不协和，通常只出现在属和弦上"),
        6 if chord.is_dominant() => Some("属和弦上的 #9 属于变化音，需要解决到目标和弦"),
        _ => None,
    }
}

/// 做一次和声检查。
///
/// `bar_beats` 是每小节拍数（4/4 = 4），用于给「没写长度」的和弦段兜底。
pub fn analyze(
    bars: &[HarmonyBar],
    tracks: &[&Track],
    policy: HarmonyPolicy,
    bar_beats: f64,
) -> Result<HarmonyReport, String> {
    if bars.is_empty() {
        return Err("没有和弦信息：请传入 bars，或先设置工程和弦表".into());
    }
    let policy = policy.normalize();
    let bar_beats = if bar_beats.is_finite() && bar_beats > 0.0 { bar_beats } else { 4.0 };
    let layout = layout(bars, bar_beats)?;
    let first_start = layout.first().map(|b| b.start).unwrap_or(0.0);
    let last_end = layout.last().map(|b| b.end).unwrap_or(0.0);

    let mut issues: Vec<HarmonyIssue> = Vec::new();
    let mut bar_issue_counts = vec![0usize; layout.len()];
    let mut bar_tension = vec![Severity::Info; layout.len()];
    let mut uncovered = 0usize;
    let mut per_track: Vec<(u64, usize, usize)> = Vec::new(); // (track, total, outside)

    for track in tracks {
        let mut total = 0usize;
        let mut outside = 0usize;
        // 每条轨道按绝对起点排序，导音解决判断要看「下一个音」。
        let mut notes: Vec<(f64, f64, u8)> = Vec::new();
        for clip in &track.clips {
            for n in &clip.notes {
                if n.start < 0.0 {
                    continue;
                }
                let start = clip.start + n.start;
                notes.push((start, start + n.duration, n.pitch));
            }
        }
        notes.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        for (index, (start, end, pitch)) in notes.iter().enumerate() {
            total += 1;
            let Some(bar) = layout.iter().position(|b| *start >= b.start && *start < b.end) else {
                outside += 1;
                continue;
            };
            let pc = pitch % 12;
            let chord = &layout[bar].chord;
            let beat_in_bar = *start - layout[bar].start;
            let duration = end - start;

            if chord.contains(pc) {
                continue;
            }

            // 1) 导音未解决：音级比根音低半音（= 根音上方大七度），且后面没有解决到根音。
            let is_leading = chord.degree(pc) == 11;
            if is_leading {
                let resolved = notes[index + 1..].iter().any(|(s, _, p)| {
                    *s < layout[bar].end && (p % 12) == chord.root
                });
                if !resolved {
                    push_issue(
                        &mut issues,
                        &mut bar_issue_counts,
                        &mut bar_tension,
                        HarmonyIssue {
                            kind: HarmonyIssueKind::UnresolvedLeadingTone,
                            severity: Severity::Warning,
                            bar,
                            bar_start: layout[bar].start,
                            beat: beat_in_bar,
                            track: track.id.0,
                            pitch: *pitch,
                            pitch_name: pitch_name(*pitch),
                            duration: round3(duration),
                            message: format!(
                                "{} 是 {} 的导音，但小节内没有解决到 {}",
                                pitch_name(*pitch),
                                chord.symbol,
                                pitch_class_name(chord.root)
                            ),
                        },
                    );
                    continue;
                }
            }

            // 2) 不推荐的延伸音。
            if let Some(reason) = unrecommended_extension(chord, pc) {
                if !policy.allow_extensions {
                    push_issue(
                        &mut issues,
                        &mut bar_issue_counts,
                        &mut bar_tension,
                        HarmonyIssue {
                            kind: HarmonyIssueKind::Extension,
                            severity: Severity::Warning,
                            bar,
                            bar_start: layout[bar].start,
                            beat: beat_in_bar,
                            track: track.id.0,
                            pitch: *pitch,
                            pitch_name: pitch_name(*pitch),
                            duration: round3(duration),
                            message: format!("{} 在 {} 上：{reason}", pitch_name(*pitch), chord.symbol),
                        },
                    );
                    continue;
                }
            }

            // 3) 和弦外音：短经过音只提示，长音才算冲突。
            let short = duration <= policy.allow_non_chord_tone_duration + 1e-9;
            let severity = if short && policy.allow_approach_notes {
                Severity::Info
            } else if duration >= 2.0 {
                Severity::Error
            } else {
                Severity::Warning
            };
            let message = if short && policy.allow_approach_notes {
                format!(
                    "{} 不属于 {}（时值 {:.2} 拍，按经过音放行）",
                    pitch_name(*pitch),
                    chord.symbol,
                    duration
                )
            } else {
                format!(
                    "{} 不属于 {}（持续 {:.2} 拍）",
                    pitch_name(*pitch),
                    chord.symbol,
                    duration
                )
            };
            push_issue(
                &mut issues,
                &mut bar_issue_counts,
                &mut bar_tension,
                HarmonyIssue {
                    kind: HarmonyIssueKind::NonChordTone,
                    severity,
                    bar,
                    bar_start: layout[bar].start,
                    beat: beat_in_bar,
                    track: track.id.0,
                    pitch: *pitch,
                    pitch_name: pitch_name(*pitch),
                    duration: round3(duration),
                    message,
                },
            );
        }

        // 4) 悬挂音：跨小节延续、且不属于新小节和弦的音。
        for (start, end, pitch) in &notes {
            if *start >= *end {
                continue;
            }
            for (bar, seg) in layout.iter().enumerate() {
                if *start < seg.start && *end > seg.start {
                    let pc = pitch % 12;
                    if !seg.chord.contains(pc) {
                        push_issue(
                            &mut issues,
                            &mut bar_issue_counts,
                            &mut bar_tension,
                            HarmonyIssue {
                                kind: HarmonyIssueKind::Suspension,
                                severity: Severity::Warning,
                                bar,
                                bar_start: seg.start,
                                beat: 0.0,
                                track: track.id.0,
                                pitch: *pitch,
                                pitch_name: pitch_name(*pitch),
                                duration: round3(*end - seg.start),
                                message: format!(
                                    "{} 从上一小节延续进来，但不属于 {}（悬挂音）",
                                    pitch_name(*pitch),
                                    seg.chord.symbol
                                ),
                            },
                        );
                    }
                }
            }
        }

        per_track.push((track.id.0, total, outside));
    }

    uncovered += per_track.iter().map(|(_, _, outside)| *outside).sum::<usize>();

    // 5) 轨道间同音区重叠。
    for (i, a) in tracks.iter().enumerate() {
        for b in tracks.iter().skip(i + 1) {
            let overlap = crate::compose::track_overlap(a, b, None);
            if !overlap.conflict {
                continue;
            }
            // 挂到「第一条轨道第一个落进和弦表的音」所在的小节上。
            let anchor = a
                .clips
                .iter()
                .flat_map(|c| c.notes.iter().map(move |n| c.start + n.start))
                .filter(|s| *s >= first_start && *s < last_end)
                .fold(f64::INFINITY, f64::min);
            let bar = layout
                .iter()
                .position(|seg| anchor >= seg.start && anchor < seg.end)
                .unwrap_or(0);
            push_issue(
                &mut issues,
                &mut bar_issue_counts,
                &mut bar_tension,
                HarmonyIssue {
                    kind: HarmonyIssueKind::TrackOverlap,
                    severity: Severity::Warning,
                    bar,
                    bar_start: layout[bar].start,
                    beat: (anchor - layout[bar].start).max(0.0),
                    track: a.id.0,
                    pitch: 0,
                    pitch_name: String::new(),
                    duration: overlap.overlap_beats,
                    message: overlap.suggestion.clone(),
                },
            );
        }
    }

    // 6) 全局提示。
    let mut notes_out: Vec<String> = Vec::new();
    for (track, total, outside) in &per_track {
        if *total >= 4 && *outside * 4 > *total {
            notes_out.push(format!(
                "轨道 {track} 有 {outside}/{total} 个音落在和弦表之外（未被检查）"
            ));
        }
    }
    if uncovered > 0 && notes_out.is_empty() {
        notes_out.push(format!("有 {uncovered} 个音落在和弦表覆盖范围之外，未参与检查"));
    }

    let conflict_level = issues.iter().map(|i| i.severity).max().unwrap_or(Severity::Info);

    let bars_report = layout
        .iter()
        .map(|seg| HarmonyBarReport {
            bar: seg.index,
            start: round3(seg.start),
            length: round3(seg.end - seg.start),
            symbol: seg.chord.symbol.clone(),
            chord_tones: seg.chord.tone_names(),
            tension: bar_tension[seg.index],
            issues: bar_issue_counts[seg.index],
        })
        .collect();

    Ok(HarmonyReport { bars: bars_report, issues, conflict_level, notes: notes_out })
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn push_issue(
    issues: &mut Vec<HarmonyIssue>,
    counts: &mut [usize],
    tension: &mut [Severity],
    issue: HarmonyIssue,
) {
    if let Some(count) = counts.get_mut(issue.bar) {
        *count += 1;
    }
    if let Some(level) = tension.get_mut(issue.bar) {
        *level = (*level).max(issue.severity);
    }
    issues.push(issue);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipId, Note, TrackId};

    fn pcs(chord: &Chord) -> Vec<u8> {
        chord.pitch_classes()
    }

    #[test]
    fn parses_common_triads_and_sevenths() {
        assert_eq!(pcs(&parse_chord("C").unwrap()), vec![0, 4, 7]);
        assert_eq!(pcs(&parse_chord("Cm").unwrap()), vec![0, 3, 7]);
        assert_eq!(pcs(&parse_chord("Cdim").unwrap()), vec![0, 3, 6]);
        assert_eq!(pcs(&parse_chord("Caug").unwrap()), vec![0, 4, 8]);
        assert_eq!(pcs(&parse_chord("Cmaj7").unwrap()), vec![0, 4, 7, 11]);
        assert_eq!(pcs(&parse_chord("C7").unwrap()), vec![0, 4, 7, 10]);
        assert_eq!(pcs(&parse_chord("Cm7").unwrap()), vec![0, 3, 7, 10]);
        assert_eq!(pcs(&parse_chord("Cm7b5").unwrap()), vec![0, 3, 6, 10]);
        assert_eq!(pcs(&parse_chord("Cdim7").unwrap()), vec![0, 3, 6, 9]);
    }

    #[test]
    fn parses_extensions_and_alterations() {
        // Dm9：D F A C E
        assert_eq!(pcs(&parse_chord("Dm9").unwrap()), vec![0, 2, 4, 5, 9]);
        // G13：G B D F A C E
        assert_eq!(pcs(&parse_chord("G13").unwrap()), vec![0, 2, 4, 5, 7, 9, 11]);
        // E7b9：E G# B D F
        assert_eq!(pcs(&parse_chord("E7b9").unwrap()), vec![2, 4, 5, 8, 11]);
        // Cmaj9：C E G B D
        assert_eq!(pcs(&parse_chord("Cmaj9").unwrap()), vec![0, 2, 4, 7, 11]);
        // Fmaj7：F A C E
        assert_eq!(pcs(&parse_chord("Fmaj7").unwrap()), vec![0, 4, 5, 9]);
        // Bm7b5：B D F A
        assert_eq!(pcs(&parse_chord("Bm7b5").unwrap()), vec![2, 5, 9, 11]);
    }

    #[test]
    fn parses_slash_chords_and_unicode() {
        let c = parse_chord("C/E").unwrap();
        assert_eq!(c.root, 0);
        assert_eq!(c.bass, 4);
        assert_eq!(pcs(&c), vec![0, 4, 7]);
        assert_eq!(pcs(&parse_chord("B♭maj7").unwrap()), vec![2, 5, 9, 10]);
        assert_eq!(pcs(&parse_chord("Cø").unwrap()), vec![0, 3, 6, 10]);
    }

    #[test]
    fn rejects_unknown_symbols_with_actionable_errors() {
        let err = parse_chord("H7").unwrap_err();
        assert!(err.contains("根音"), "{err}");
        let err = parse_chord("Cxyz").unwrap_err();
        assert!(err.contains("修饰") || err.contains("类型"), "{err}");
        assert!(parse_chord("").is_err());
    }

    fn track_with(notes: Vec<(u8, f64, f64)>) -> Track {
        Track {
            id: TrackId(1),
            name: "Lead".into(),
            instrument: None,
            effects: vec![],
            clips: vec![Clip {
                id: ClipId(1),
                start: 0.0,
                length: 16.0,
                notes: notes
                    .into_iter()
                    .enumerate()
                    .map(|(i, (pitch, start, duration))| Note {
                        id: i as u64 + 1,
                        pitch,
                        start,
                        duration,
                        velocity: 100,
                        volume: 127,
                    })
                    .collect(),
                audio: None,
            }],
            mixer: Default::default(),
            sends: vec![],
            automation: vec![],
            meta: None,
        }
    }

    fn bars() -> Vec<HarmonyBar> {
        vec![
            HarmonyBar { start: 0.0, length: 4.0, symbol: "Cmaj7".into(), source: None },
            HarmonyBar { start: 4.0, length: 4.0, symbol: "G7".into(), source: None },
        ]
    }

    #[test]
    fn chord_tones_are_clean_and_outsiders_are_reported() {
        let t = track_with(vec![(60, 0.0, 1.0), (61, 1.0, 1.0)]);
        let report = analyze(&bars(), &[&t], HarmonyPolicy::default(), 4.0).unwrap();
        // C4 是 Cmaj7 的根音；C#4 是长音外音。
        assert_eq!(report.issues.len(), 1, "{:?}", report.issues);
        let issue = &report.issues[0];
        assert_eq!(issue.kind, HarmonyIssueKind::NonChordTone);
        assert_eq!(issue.bar, 0);
        assert_eq!(issue.beat, 1.0);
        assert_eq!(issue.pitch, 61);
        assert_eq!(issue.pitch_name, "C#4");
        assert_eq!(issue.severity, Severity::Warning);
    }

    #[test]
    fn short_passing_tones_are_not_flagged_as_conflicts() {
        // C#4 是短经过音（0.25 拍），E4 是 Cmaj7 的三音。
        let t = track_with(vec![(60, 0.0, 1.0), (61, 1.0, 0.25), (64, 1.25, 1.0)]);
        let report = analyze(&bars(), &[&t], HarmonyPolicy::default(), 4.0).unwrap();
        let passing: Vec<&HarmonyIssue> = report
            .issues
            .iter()
            .filter(|i| i.kind == HarmonyIssueKind::NonChordTone)
            .collect();
        assert_eq!(passing.len(), 1);
        assert_eq!(passing[0].severity, Severity::Info, "短经过音只提示");
        assert_eq!(report.conflict_level, Severity::Info);
    }

    #[test]
    fn unknown_symbol_aborts_with_the_bar_number() {
        let bad = vec![HarmonyBar { start: 0.0, length: 4.0, symbol: "Hm".into(), source: None }];
        let t = track_with(vec![(60, 0.0, 1.0)]);
        let err = analyze(&bad, &[&t], HarmonyPolicy::default(), 4.0).unwrap_err();
        assert!(err.contains("第 1 小节"), "{err}");
        assert!(err.contains("Hm"), "{err}");
    }

    #[test]
    fn empty_harmony_is_an_error_not_a_silent_pass() {
        let t = track_with(vec![(60, 0.0, 1.0)]);
        assert!(analyze(&[], &[&t], HarmonyPolicy::default(), 4.0).is_err());
    }

    #[test]
    fn suspension_is_reported_at_the_bar_it_hangs_into() {
        // 4 拍长音从 Cmaj7 小节跨进 G7 小节：B 不属于 G7 的构成音？
        // G7 = G B D F，B 是构成音；改用 C（不属于 G7）来测。
        let t = track_with(vec![(60, 2.0, 4.0)]);
        let report = analyze(&bars(), &[&t], HarmonyPolicy::default(), 4.0).unwrap();
        assert!(
            report.issues.iter().any(|i| i.kind == HarmonyIssueKind::Suspension && i.bar == 1),
            "{:?}",
            report.issues
        );
    }
}
