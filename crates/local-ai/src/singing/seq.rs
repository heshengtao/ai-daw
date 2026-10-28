//! **音符 → 模型张量序列**（纯计算，不碰 ONNX）。
//!
//! 与 SXSEditor 的 `Preprocessing.notesToSequences` / `_buildMel2token` 逐项对齐：
//!
//! ```text
//! token 序列： [PAD] [BOW ph… EOW] [BOW ph… EOW] …
//! 音高/类型： 与 token 等长，每个 token 都属于某个音符
//! 帧序列：     Σ 音符秒数 × 50（24 kHz / hop 480），逐帧铺 F0
//! mel2token：  token 的时间展开表——每个 mel 帧对应哪一个 token
//! ```
//!
//! `mel2token` 是整条管线里最容易写错的一处（差一帧 → 整句节奏漂移），所以这里把参考
//! 实现的「首帧给 token、`inner = 下一音符起点 − 起点 − 2`、末帧给下一个 token、scan
//! 跳过已占用帧」逐行搬过来，并用测试锁住总帧数守恒。

use super::phones;

/// 24 kHz 模型帧率（`SAMPLE_RATE / HOP_SIZE`）。
pub const FRAME_RATE: f64 = 24_000.0 / 480.0;
/// F0 量化档位数（bin 0 = 清音）。
pub const F0_BIN: usize = 361;
/// 最低 F0（C1 = 32.7031956625 Hz，按 f32 精度截断）。
pub const F0_MIN: f32 = 32.703_197;

/// 一个待唱音符（拍为单位；`pitch == 0` 或没有音节 = 休止）。
#[derive(Debug, Clone, PartialEq)]
pub struct SingNote {
    /// 相对片段起点的拍。
    pub start: f64,
    /// 时长（拍）。
    pub duration: f64,
    /// MIDI 音高；0 = 休止。
    pub pitch: u8,
    /// 歌词音节（汉字；`None` = 休止 / 连音）。
    pub syllable: Option<String>,
    /// 强制声调（`1..=5`），来自 `小3` 这种写法。
    pub tone: Option<char>,
    /// 连音：延续前一个音节的发音（模型 note type = 3）。
    pub slur: bool,
}

impl SingNote {
    /// 休止判定（与参考实现的 `isRestNote` 一致）：`<AP>` 是**可听见**的吸气，不算休止；
    /// `pitch <= 0` 优先判休止；连音即使没歌词也不算休止。
    pub fn is_rest(&self) -> bool {
        if self.syllable.as_deref() == Some("<AP>") {
            return false;
        }
        if self.pitch == 0 {
            return true;
        }
        if self.slur {
            return false;
        }
        matches!(self.syllable.as_deref(), None | Some("") | Some("<SP>"))
    }

    /// 这个音符送进模型的音素 id 列表。
    fn phones(&self) -> Vec<u16> {
        if self.syllable.as_deref() == Some("<AP>") {
            return vec![phones::breath_phone()];
        }
        if self.is_rest() {
            return vec![phones::silence_phone()];
        }
        // 连音且没写歌词：送 <SP> + note type 3，模型按「延续」处理（与参考实现一致）。
        let Some(text) = self.syllable.as_deref().filter(|s| !s.is_empty()) else {
            return vec![phones::silence_phone()];
        };
        let first = text.chars().find(|c| is_han(*c));
        match first {
            Some(c) => vec![phones::char_phone(c, self.tone)],
            None => vec![phones::la_phone()],
        }
    }
}

/// 是否汉字（基本区 + 扩展 A）。
pub fn is_han(c: char) -> bool {
    matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF)
}

/// 给音符序列补空隙（空隙 = 休止），让它成为一条**连续**的谱面时间线。
///
/// 模型按「Σ 时长」算帧数，谱面上的空隙如果不补出来，后面所有音符都会整体前移。
pub fn fill_gaps(notes: &[SingNote]) -> Vec<SingNote> {
    let mut sorted = notes.to_vec();
    sorted.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let Some(first) = sorted.first().cloned() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(sorted.len() + 4);
    let mut cursor = first.start + first.duration;
    out.push(first);
    for note in sorted.into_iter().skip(1) {
        if note.start - cursor > 0.01 {
            out.push(SingNote {
                start: cursor,
                duration: note.start - cursor,
                pitch: 0,
                syllable: None,
                tone: None,
                slur: false,
            });
        }
        cursor = cursor.max(note.start + note.duration);
        out.push(note);
    }
    out
}

/// 一条完整的模型输入序列。
#[derive(Debug, Clone, Default)]
pub struct Sequence {
    /// 音素 token id。
    pub tokens: Vec<i64>,
    /// 每个 token 的 MIDI 音高（休止 0）。
    pub pitches: Vec<i64>,
    /// 每个 token 的音符类型：1 休止 / 2 唱 / 3 连音。
    pub types: Vec<i64>,
    /// 每个 mel 帧的量化 F0（0..=360）。
    pub f0_ids: Vec<i64>,
    /// 每个 mel 帧的 F0（Hz，0 = 清音）——声码器/调试用。
    pub f0_hz: Vec<f32>,
    /// 每个 mel 帧对应的 token。
    pub mel2token: Vec<i64>,
    /// 总帧数。
    pub frames: usize,
}

/// 中文逐字音节的分配规则（`j == 1`）；英文多音素也走线性分配。
fn allocate(j: usize, inner: usize) -> Vec<usize> {
    if j == 0 {
        return Vec::new();
    }
    if inner >= j {
        let base = inner / j;
        let extra = inner % j;
        (0..j).map(|p| base + usize::from(p < extra)).collect()
    } else {
        // 帧数不足：每个音素最多 1 帧（元音优先的完整策略在中文单音素下用不到）。
        (0..j).map(|p| usize::from(p < inner)).collect()
    }
}

/// 音符 → 序列。输入应当已经过 [`fill_gaps`]（连续谱面）。
pub fn build(notes: &[SingNote], bpm: f64) -> Sequence {
    let bpm = if bpm.is_finite() && bpm > 0.0 {
        bpm
    } else {
        120.0
    };
    let seconds_per_beat = 60.0 / bpm;
    let total_seconds: f64 = notes.iter().map(|n| n.duration * seconds_per_beat).sum();
    let frames = (total_seconds * FRAME_RATE).floor().max(0.0) as usize;

    let pad = phones::phone_id("<PAD>") as i64;
    let bow = phones::phone_id("<BOW>") as i64;
    let eow = phones::phone_id("<EOW>") as i64;

    let mut seq = Sequence {
        tokens: vec![pad],
        pitches: vec![0],
        types: vec![1],
        f0_ids: Vec::new(),
        f0_hz: vec![0.0; frames],
        mel2token: vec![0; frames],
        frames,
    };
    if frames == 0 {
        return seq;
    }

    // (首帧, 音素数, 音素 id)
    let mut locations: Vec<(usize, usize, Vec<u16>)> = Vec::with_capacity(notes.len());
    let mut elapsed = 0.0f64;
    let mut frame_cursor = 0usize;

    for note in notes {
        let start_frame = ((elapsed * FRAME_RATE).round() as usize).min(frames.saturating_sub(1));
        let kind = if note.slur {
            3
        } else if note.is_rest() {
            1
        } else {
            2
        };
        let pitch = i64::from(note.pitch);

        seq.tokens.push(bow);
        seq.pitches.push(pitch);
        seq.types.push(kind);

        let ids = note.phones();
        locations.push((start_frame, ids.len().max(1), ids.clone()));
        for id in &ids {
            seq.tokens.push(i64::from(*id));
            seq.pitches.push(pitch);
            seq.types.push(kind);
        }

        seq.tokens.push(eow);
        seq.pitches.push(pitch);
        seq.types.push(kind);

        // F0：休止 / 换气 / 无音高 → 0（清音）。
        let hz = if note.is_rest() || note.pitch == 0 {
            0.0
        } else {
            440.0 * 2f32.powf((f32::from(note.pitch) - 69.0) / 12.0)
        };
        let note_frames = (note.duration * seconds_per_beat * FRAME_RATE).round() as usize;
        for f in 0..note_frames {
            if frame_cursor + f < frames {
                seq.f0_hz[frame_cursor + f] = hz;
            }
        }
        frame_cursor += note_frames;
        elapsed += note.duration * seconds_per_beat;
    }

    build_mel2token(&locations, frames, &mut seq.mel2token, seq.tokens.len());
    seq.f0_ids = quantize(&seq.f0_hz);
    seq
}

/// 每个 mel 帧属于哪个 token（见模块头部的逐行说明）。
fn build_mel2token(
    locations: &[(usize, usize, Vec<u16>)],
    frames: usize,
    mel2token: &mut [i64],
    token_count: usize,
) {
    let mut token_base: i64 = 1;
    for (idx, (start, count, _)) in locations.iter().enumerate() {
        let j = *count;
        let mut i = *start;
        let next_start = locations.get(idx + 1).map_or(frames, |(s, _, _)| *s);
        if i >= frames {
            break;
        }
        if mel2token[i] > 0 {
            while i < frames && mel2token[i] > 0 {
                i += 1;
            }
        }
        if i >= frames {
            break;
        }
        mel2token[i] = token_base;
        let inner = next_start.saturating_sub(i + 2);
        let alloc = allocate(j, inner);

        let mut offset = 0usize;
        for (p, frames_for_phone) in alloc.iter().enumerate() {
            let start = i + 1 + offset;
            let end = (start + frames_for_phone).min(frames);
            for cell in mel2token.iter_mut().take(end).skip(start) {
                *cell = token_base + 1 + p as i64;
            }
            offset += frames_for_phone;
        }
        if next_start > i + 1 && next_start - 1 < frames {
            mel2token[next_start - 1] = token_base + j as i64 + 1;
        }
        token_base += j as i64 + 2;
    }

    // 参考实现同款：越界的 token id 夹到词表末尾，避免 gather 越界。
    let limit = token_count.saturating_sub(1) as i64;
    for v in mel2token.iter_mut() {
        if *v > limit {
            *v = limit;
        }
    }
}

/// F0（Hz）→ 20 cent 一档的量化 id（0 = 清音）。
pub fn quantize(f0: &[f32]) -> Vec<i64> {
    f0.iter()
        .map(|hz| {
            if *hz <= 0.0 {
                0
            } else {
                let cents = 1200.0 * (hz.max(F0_MIN) / F0_MIN).log2();
                let bin = (cents / 20.0).round() as i64 + 1;
                bin.clamp(1, F0_BIN as i64 - 1)
            }
        })
        .collect()
}

/// 一个可独立推理的区段（拍区间 + 音符下标区间）。
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// 区段起点（拍，绝对）。
    pub start: f64,
    /// 区段时长（拍）。
    pub length: f64,
    /// 音符下标区间（`notes[start..end]`）。
    pub notes: std::ops::Range<usize>,
    /// 是否需要淡入 / 淡出（长区段被硬切时的防爆音）。
    pub fade: bool,
}

/// 把连续谱面切成可独立推理的区段。
///
/// 规则：
/// 1. 在**休止**处切（休止 ≥ 0.5 拍，约 0.25 秒）；休止归下一段，给下一段留出起唱前的气口。
/// 2. 单个区段超过 `max_frames`（默认 1000 帧 = 20 秒）时，在最近的音符边界硬切，
///    两端各 8 ms 淡入淡出——宁可有一个几乎听不见的接缝，也不要一段 60 秒的注意力。
pub fn segments(notes: &[SingNote], bpm: f64, max_frames: usize) -> Vec<Segment> {
    let mut out = Vec::new();
    if notes.is_empty() {
        return out;
    }
    let spb = 60.0 / bpm.max(1e-6);
    let frames_of = |n: &SingNote| (n.duration * spb * FRAME_RATE).round() as usize;

    let mut begin = 0usize;
    let mut frames = 0usize;
    // 左侧是不是硬切出来的（硬切的两端都要淡，休止处不用）。
    let mut fade_in = false;
    for (i, note) in notes.iter().enumerate() {
        let note_frames = frames_of(note);
        let is_break = i > begin && note.is_rest() && note.duration >= 0.5 && frames > 0;
        let over_cap = i > begin && frames + note_frames > max_frames;
        if is_break || over_cap {
            let range = begin..i;
            out.push(Segment {
                start: notes[begin].start,
                length: notes[range.clone()].iter().map(|n| n.duration).sum(),
                notes: range,
                fade: fade_in || over_cap,
            });
            begin = i;
            frames = 0;
            fade_in = over_cap;
        }
        frames += note_frames;
    }
    if begin < notes.len() {
        let range = begin..notes.len();
        out.push(Segment {
            start: notes[begin].start,
            length: notes[range.clone()].iter().map(|n| n.duration).sum(),
            notes: range,
            fade: fade_in,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: f64, duration: f64, pitch: u8, lyric: Option<&str>) -> SingNote {
        SingNote {
            start,
            duration,
            pitch,
            syllable: lyric.map(str::to_string),
            tone: None,
            slur: false,
        }
    }

    #[test]
    fn frames_follow_total_duration() {
        let notes = vec![
            note(0.0, 1.0, 60, Some("小")),
            note(1.0, 1.0, 62, Some("河")),
        ];
        // 2 拍 @120 BPM = 1 秒 = 50 帧
        let seq = build(&notes, 120.0);
        assert_eq!(seq.frames, 50);
        assert_eq!(seq.f0_hz.len(), 50);
        assert_eq!(seq.mel2token.len(), 50);
        assert_eq!(seq.tokens.len(), 1 + 3 * 2);
    }

    #[test]
    fn f0_is_quantized_to_20_cent_bins() {
        // A4 = 440 Hz → 0 cent 相对 A4 的 bin。
        let seq = build(&[note(0.0, 1.0, 69, Some("啊"))], 120.0);
        let expected = (1200.0 * (440.0f32 / F0_MIN).log2() / 20.0).round() as i64 + 1;
        assert!(seq.f0_ids.iter().all(|v| *v == expected));
        assert!(seq.f0_hz.iter().all(|hz| (*hz - 440.0).abs() < 0.01));
        // 休止 → 0。
        let rest = build(&[note(0.0, 1.0, 0, None)], 120.0);
        assert!(rest.f0_ids.iter().all(|v| *v == 0));
    }

    #[test]
    fn rest_note_is_type_one_and_silence_phone() {
        let seq = build(&[note(0.0, 1.0, 0, None)], 120.0);
        assert_eq!(seq.types[1], 1);
        assert_eq!(seq.tokens[2], i64::from(phones::silence_phone()));
    }

    #[test]
    fn mel2token_covers_every_frame_and_never_exceeds_vocab() {
        let notes = vec![
            note(0.0, 2.0, 60, Some("小")),
            note(2.0, 2.0, 62, Some("河")),
            note(4.0, 4.0, 64, Some("淌")),
        ];
        let seq = build(&notes, 120.0);
        assert_eq!(seq.mel2token.len(), seq.frames);
        for (f, token) in seq.mel2token.iter().enumerate() {
            assert!(
                *token >= 1,
                "第 {f} 帧没有归属 token（0 号是 PAD，不应该出现在正文里）"
            );
            assert!(
                (*token as usize) < seq.tokens.len(),
                "第 {f} 帧指向不存在的 token {token}"
            );
        }
        // 后面的音符必须比前面的音符晚（mel2token 单调不减）。
        assert!(seq.mel2token.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn gaps_are_filled_with_rests_so_later_notes_do_not_shift() {
        let notes = vec![
            note(0.0, 1.0, 60, Some("小")),
            note(4.0, 1.0, 62, Some("河")),
        ];
        let filled = fill_gaps(&notes);
        assert_eq!(filled.len(), 3);
        assert!(filled[1].is_rest());
        assert!((filled[1].start - 1.0).abs() < 1e-9);
        assert!((filled[1].duration - 3.0).abs() < 1e-9);
        assert!((filled[2].start - 4.0).abs() < 1e-9);
        // 总帧数 = 5 拍 = 2.5 秒 = 125 帧（补空隙之后才成立）。
        assert_eq!(build(&filled, 120.0).frames, 125);
    }

    #[test]
    fn segments_split_on_rests_and_cap_long_regions() {
        // 8 拍唱 + 2 拍休止 + 2 拍唱 @120 → 两个区段。
        let notes = vec![
            note(0.0, 8.0, 60, Some("小")),
            note(8.0, 2.0, 0, None),
            note(10.0, 2.0, 62, Some("河")),
        ];
        let segs = segments(&notes, 120.0, 1000);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].notes, 0..1);
        assert_eq!(segs[1].notes, 1..3);
        assert!((segs[1].start - 8.0).abs() < 1e-9);
        assert!(!segs[0].fade);

        // 连续 40 拍（20 秒）没有休止 → 必须按音符边界切开，并带淡入淡出标记。
        let long: Vec<SingNote> = (0..40)
            .map(|i| note(f64::from(i), 1.0, 60, Some("小")))
            .collect();
        let segs = segments(&long, 120.0, 200);
        assert!(segs.len() > 1, "超过上限的区段必须切开");
        assert!(
            segs.iter().all(|s| s.notes.len() <= 8),
            "每段不超过 8 个音（200 帧）"
        );
        assert!(
            segs.iter().skip(1).all(|s| s.fade),
            "硬切出来的接缝要淡入淡出"
        );
        // 段与段首尾相接，不丢音符。
        let covered: usize = segs.iter().map(|s| s.notes.len()).sum();
        assert_eq!(covered, 40);
    }

    #[test]
    fn slur_notes_are_not_rests() {
        let mut n = note(0.0, 1.0, 60, None);
        n.slur = true;
        assert!(!n.is_rest());
        let seq = build(&[n], 120.0);
        assert_eq!(seq.types[1], 3);
    }
}
