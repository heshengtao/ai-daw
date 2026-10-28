//! **歌声合成**：MIDI 音符 + 歌词 + 参考音色 → 24 kHz 歌声。
//!
//! 三层：
//! - [`phones`] —— 音素表与中文 G2P（纯查表）。
//! - [`seq`] —— 音符 → token / F0 / mel2token（纯计算，CI 可测）。
//! - [`soulx`] —— SoulX-Singer 的 9 张 ONNX 图（加载、条件、采样、声码器）。
//!
//! 这里只做编排：[`assign_lyrics`] 把一串歌词按顺序贴到音符上，[`synthesize`] 把谱面切成
//! 可独立推理的区段、逐段编码/采样/声码，再按拍位拼回整段音频。
//!
//! **为什么是「一串歌词」而不是逐音符字段**：引擎的 `Note` 没有歌词（本阶段不动 IPC
//! schema），而 DAW 用户最自然的输入就是「一行歌词 + 一段旋律」。规则见 [`assign_lyrics`]：
//! 一字一音、`-` 连音、歌词不够就沿用上一个音节。

pub mod phones;
pub mod reference;
pub mod seq;
pub mod soulx;

pub use seq::SingNote;
pub use soulx::{Precision, SAMPLE_RATE};

use crate::Progress;

/// 一个区段最多多少帧（20 秒）。超过就在音符边界硬切。
pub const MAX_SEGMENT_FRAMES: usize = 1000;

/// 一次合成的结果。
#[derive(Debug, Clone)]
pub struct Take {
    /// 单声道采样（24 kHz）。
    pub samples: Vec<f32>,
    /// 采样率。
    pub sample_rate: f64,
    /// 时长（秒）。
    pub seconds: f64,
    /// 实际推理了几个区段。
    pub segments: usize,
    /// 参与演唱的音符数。
    pub notes: usize,
    /// 参考声线实际取自原音频的第几秒（自动裁剪）；没有参考时为 `None`。
    pub reference_offset: Option<f64>,
    /// 这次实际用的重量精度。
    pub precision: soulx::Precision,
}

/// 歌词分配的结果（给界面/MCP 的一句话反馈）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LyricsReport {
    /// 贴上的音节数。
    pub assigned: usize,
    /// 连音音符数。
    pub slurred: usize,
    /// 因为歌词用完了、沿用上一个音节的音符数。
    pub reused: usize,
    /// 没有任何歌词、用 `la` 兜底的音符数。
    pub filled: usize,
}

impl LyricsReport {
    /// 一句人话。
    pub fn summary(&self) -> String {
        let mut parts = vec![format!("{} 个音节", self.assigned)];
        if self.slurred > 0 {
            parts.push(format!("{} 个连音", self.slurred));
        }
        if self.reused > 0 {
            parts.push(format!("{} 个音沿用前一字", self.reused));
        }
        if self.filled > 0 {
            parts.push(format!("{} 个音用 la 兜底", self.filled));
        }
        parts.join("、")
    }
}

/// 模型是否就位。
pub fn installed() -> bool {
    soulx::installed()
}

/// 模型所在目录。
pub fn model_dir() -> Option<std::path::PathBuf> {
    soulx::model_dir()
}

/// 歌词里的一枚记号。
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Syllable { text: String, tone: Option<char> },
    Slur,
}

/// 解析一行歌词。
///
/// - 汉字：一字一音节；紧跟的 `1..5` 是强制声调（`小4`）。
/// - `-`、`—`、`_`：连音，下一个音符延长前一个音节。
/// - `<SP>` / `<AP>`：休止 / 吸气记号。
/// - ASCII 单词：整体作为一个记号（本阶段只按 `la` 兜底）。
/// - 空白与标点：分隔符。
fn tokenize(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if matches!(c, '-' | '—' | '–' | '_' | '～' | '~') {
            out.push(Token::Slur);
            i += 1;
            continue;
        }
        if c == '<' {
            if let Some(close) = chars[i..].iter().position(|ch| *ch == '>') {
                let token: String = chars[i..i + close + 1].iter().collect();
                out.push(Token::Syllable {
                    text: token,
                    tone: None,
                });
                i += close + 1;
                continue;
            }
        }
        if seq::is_han(c) {
            // 汉字 + 可选声调数字。
            let mut tone = None;
            let mut next = i + 1;
            if let Some(d) = chars.get(next).copied() {
                if ('1'..='5').contains(&d) {
                    tone = Some(d);
                    next += 1;
                }
            }
            out.push(Token::Syllable {
                text: c.to_string(),
                tone,
            });
            i = next;
            continue;
        }
        if c.is_ascii_alphanumeric() {
            let mut word = String::new();
            while let Some(d) = chars.get(i).copied() {
                if d.is_ascii_alphanumeric() || d == '\'' {
                    word.push(d);
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(Token::Syllable {
                text: word,
                tone: None,
            });
            continue;
        }
        // 标点 / 其他：跳过。
        i += 1;
    }
    out
}

/// 把歌词贴到音符上（`notes` 会被**原地修改**）。
///
/// 两条来源，**逐音歌词优先**：
/// 1. `SingNote::syllable` 已经有值（来自引擎的 `Note.lyric`）——直接用：
///    汉字可带 `1..5` 强制声调，`-` = 连音（沿用前一个音节）。
/// 2. 没有逐音歌词的音符，按顺序消费 `lyrics` 这串文本：一字一音、`-` 连音、
///    标点与空白跳过；歌词不够时沿用前一个音节，连前一个都没有就用 `la`。
///
/// 这样「整串歌词快速填」和「逐音改词」可以混用：改过的音不动，其余照旧按顺序补。
pub fn assign_lyrics(notes: &mut [SingNote], lyrics: &str) -> LyricsReport {
    let mut report = LyricsReport::default();
    let tokens = tokenize(lyrics);
    let mut cursor = 0usize;
    let mut last: Option<(String, Option<char>)> = None;

    let mut order: Vec<usize> = (0..notes.len()).collect();
    order.sort_by(|a, b| {
        notes[*a]
            .start
            .partial_cmp(&notes[*b].start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for idx in order {
        if notes[idx].pitch == 0 {
            continue;
        }

        // ---- 1. 逐音歌词 ----
        if let Some(raw) = notes[idx].syllable.clone() {
            let raw = raw.trim();
            if raw == "-" {
                notes[idx].slur = true;
                report.slurred += 1;
                if let Some((text, tone)) = &last {
                    notes[idx].syllable = Some(text.clone());
                    notes[idx].tone = *tone;
                } else {
                    notes[idx].syllable = Some("la".into());
                    report.filled += 1;
                }
                continue;
            }
            if raw == "<SP>" || raw.is_empty() {
                notes[idx].syllable = if raw == "<SP>" {
                    Some("<SP>".into())
                } else {
                    None
                };
                notes[idx].slur = false;
                continue;
            }
            let (text, tone) = parse_syllable(raw);
            notes[idx].syllable = Some(text.clone());
            notes[idx].tone = tone;
            notes[idx].slur = false;
            report.assigned += 1;
            last = Some((text, tone));
            continue;
        }

        // ---- 2. 整串歌词按顺序补 ----
        let mut slur = false;
        while let Some(token) = tokens.get(cursor) {
            match token {
                Token::Slur => {
                    slur = true;
                    cursor += 1;
                }
                Token::Syllable { .. } => break,
            }
        }

        if slur {
            notes[idx].slur = true;
            report.slurred += 1;
            if let Some((text, tone)) = &last {
                notes[idx].syllable = Some(text.clone());
                notes[idx].tone = *tone;
            } else {
                notes[idx].syllable = Some("la".into());
                report.filled += 1;
            }
            continue;
        }

        match tokens.get(cursor).cloned() {
            Some(Token::Syllable { text, tone }) => {
                cursor += 1;
                notes[idx].syllable = Some(text.clone());
                notes[idx].tone = tone;
                notes[idx].slur = false;
                report.assigned += 1;
                last = Some((text, tone));
            }
            _ => {
                if let Some((text, tone)) = &last {
                    notes[idx].syllable = Some(text.clone());
                    notes[idx].tone = *tone;
                    report.reused += 1;
                } else {
                    notes[idx].syllable = Some("la".into());
                    notes[idx].tone = None;
                    report.filled += 1;
                }
            }
        }
    }
    report
}

/// 解析一个显式音节：`小` / `小4` / 英文单词。返回（文本，强制声调）。
fn parse_syllable(raw: &str) -> (String, Option<char>) {
    let chars: Vec<char> = raw.chars().collect();
    if chars.len() >= 2 && seq::is_han(chars[0]) && ('1'..='5').contains(&chars[1]) {
        return (chars[0].to_string(), Some(chars[1]));
    }
    (raw.to_string(), None)
}

/// 合成一段歌声。
///
/// - `notes`：音符（拍；`pitch == 0` 视为休止）。返回的音频从**这些音符的第一拍**开始，
///   长度覆盖到最后一个音符结束（中间的空隙是静音）。
/// - `reference`：参考音色的单声道采样与采样率。`None` = 用零 prompt（模型给一个中性音色）。
/// - `steps`：采样步数（32 默认；16 更快、质量略降）。
/// - `progress`：0..1 进度；返回 `false` 取消。
pub fn synthesize(
    notes: &[SingNote],
    bpm: f64,
    steps: u32,
    seed: u64,
    reference: Option<(&[f32], f64)>,
    progress: Progress<'_>,
) -> Result<Take, String> {
    synthesize_with(notes, bpm, steps, seed, reference, None, progress)
}

/// 同 [`synthesize`]，但可以指定重量精度（`None` = 自动：fp32 优先，其次 fp16）。
#[allow(clippy::too_many_arguments)]
pub fn synthesize_with(
    notes: &[SingNote],
    bpm: f64,
    steps: u32,
    seed: u64,
    reference: Option<(&[f32], f64)>,
    precision: Option<soulx::Precision>,
    progress: Progress<'_>,
) -> Result<Take, String> {
    let (precision, dir) = soulx::resolve(precision)?;
    if notes.is_empty() {
        return Err("没有音符可唱".into());
    }
    let bpm = if bpm.is_finite() && bpm > 0.0 {
        bpm
    } else {
        120.0
    };
    let spb = 60.0 / bpm;

    let filled = seq::fill_gaps(notes);
    let first = filled.first().map(|n| n.start).unwrap_or(0.0);
    let last_end = filled
        .iter()
        .map(|n| n.start + n.duration)
        .fold(first, f64::max);
    let total_beats = (last_end - first).max(0.0);
    if total_beats <= 0.0 {
        return Err("音符总时长为 0".into());
    }

    let mut runner = soulx::Runner::load(&dir, &|p| progress(p * 0.25))?;

    // 参考音色：下混 + 重采样到 24 kHz，再取前 10 秒的 mel。
    let mut reference_offset = None;
    let prompt = match reference {
        Some((mono, rate)) if !mono.is_empty() && rate > 0.0 => {
            let resampled = crate::resample::resample(mono, rate, soulx::SAMPLE_RATE);
            // 整首歌 / 前奏静音都能当输入：先挑一段最像人声的 10 秒再提 mel。
            let (picked, choice) = reference::trim_to_best(&resampled, soulx::SAMPLE_RATE, 10.0);
            reference_offset = Some(choice.start_sec);
            runner.reference_mel(&picked, soulx::PROMPT_MAX_FRAMES)?
        }
        // 没有参考音色：**不是零帧**，而是一小段零 mel（参考实现的默认就是 10–50 帧）。
        // 模型按「前面有一段 prompt」训练；完全不给 prompt 会得到明显更亮、更噪的声音。
        _ => vec![0.0f32; 32 * soulx::MEL_DIM],
    };
    let prompt_frames = prompt.len() / soulx::MEL_DIM;

    let segments = seq::segments(&filled, bpm, MAX_SEGMENT_FRAMES);
    let total_samples = ((total_beats * spb) * soulx::SAMPLE_RATE).round().max(1.0) as usize;
    let mut out = vec![0.0f32; total_samples];
    let mut sung_notes = 0usize;

    for (index, segment) in segments.iter().enumerate() {
        let segment_notes = &filled[segment.notes.clone()];
        sung_notes += segment_notes.iter().filter(|n| !n.is_rest()).count();

        let sequence = seq::build(segment_notes, bpm);
        if sequence.frames == 0 {
            continue;
        }
        // 每个区段分到同样的进度区间：编码 10%，采样 75%，声码 15%。
        let base = 0.25 + 0.75 * (index as f32 / segments.len() as f32);
        let span = 0.75 / segments.len().max(1) as f32;

        if !progress(base) {
            return Err("已取消".into());
        }
        let cond = runner.condition(&sequence, prompt_frames)?;
        if !progress(base + span * 0.1) {
            return Err("已取消".into());
        }
        let mel = runner.sample(
            &prompt,
            prompt_frames,
            &cond,
            sequence.frames,
            steps,
            seed.wrapping_add(index as u64 * 7919),
            &|p| progress(base + span * (0.1 + 0.75 * p)),
        )?;
        let audio = runner.vocode(&mel, sequence.frames)?;

        // 按绝对拍位写回整段（区段之间是连续的，但留一点防爆音的淡入淡出）。
        let start = ((segment.start - first) * spb * soulx::SAMPLE_RATE)
            .round()
            .max(0.0) as usize;
        let limit = audio.len().min(out.len().saturating_sub(start));
        if limit == 0 {
            continue;
        }
        let fade = if segment.fade {
            (soulx::SAMPLE_RATE * 0.008) as usize
        } else {
            0
        };
        for i in 0..limit {
            let mut gain = 1.0f32;
            if fade > 0 {
                if i < fade {
                    gain *= i as f32 / fade as f32;
                }
                if limit - i <= fade {
                    gain *= (limit - i) as f32 / fade as f32;
                }
            }
            out[start + i] += audio[i] * gain;
        }

        if !progress(base + span) {
            return Err("已取消".into());
        }
    }

    // 峰值归一到 -1 dBFS。响度交给工程里的推子 / 母带链，不在这里压。
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if !(peak.is_finite() && peak > 0.0) {
        return Err("合成结果是静音（检查歌词与音高，或换一条参考音色）".into());
    }
    let gain = 0.891 / peak;
    for v in out.iter_mut() {
        *v *= gain;
    }

    Ok(Take {
        seconds: out.len() as f64 / soulx::SAMPLE_RATE,
        samples: out,
        sample_rate: soulx::SAMPLE_RATE,
        segments: segments.len(),
        notes: sung_notes,
        reference_offset,
        precision,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: f64, duration: f64, pitch: u8) -> SingNote {
        SingNote {
            start,
            duration,
            pitch,
            syllable: None,
            tone: None,
            slur: false,
        }
    }

    #[test]
    fn one_character_per_note_and_leftovers_reuse_the_last_syllable() {
        let mut notes = vec![note(0.0, 1.0, 60), note(1.0, 1.0, 62), note(2.0, 1.0, 64)];
        let report = assign_lyrics(&mut notes, "小河");
        assert_eq!(notes[0].syllable.as_deref(), Some("小"));
        assert_eq!(notes[1].syllable.as_deref(), Some("河"));
        assert_eq!(notes[2].syllable.as_deref(), Some("河"));
        assert_eq!(report.assigned, 2);
        assert_eq!(report.reused, 1);
    }

    #[test]
    fn dash_marks_a_slur() {
        let mut notes = vec![note(0.0, 1.0, 60), note(1.0, 1.0, 62), note(2.0, 1.0, 64)];
        let report = assign_lyrics(&mut notes, "小 - 河");
        assert_eq!(notes[0].syllable.as_deref(), Some("小"));
        assert!(notes[1].slur);
        assert_eq!(
            notes[1].syllable.as_deref(),
            Some("小"),
            "连音沿用前一个音节"
        );
        assert_eq!(notes[2].syllable.as_deref(), Some("河"));
        assert_eq!(report.slurred, 1);
    }

    #[test]
    fn tone_override_is_captured() {
        let mut notes = vec![note(0.0, 1.0, 60)];
        assign_lyrics(&mut notes, "小4");
        assert_eq!(notes[0].syllable.as_deref(), Some("小"));
        assert_eq!(notes[0].tone, Some('4'));
    }

    #[test]
    fn rests_do_not_consume_lyrics() {
        let mut notes = vec![note(0.0, 1.0, 0), note(1.0, 1.0, 60), note(2.0, 1.0, 0)];
        let report = assign_lyrics(&mut notes, "小");
        assert_eq!(notes[0].syllable, None);
        assert_eq!(notes[1].syllable.as_deref(), Some("小"));
        assert_eq!(notes[2].syllable, None);
        assert_eq!(report.assigned, 1);
        assert_eq!(report.filled, 0);
    }

    #[test]
    fn empty_lyrics_fall_back_to_la() {
        let mut notes = vec![note(0.0, 1.0, 60)];
        let report = assign_lyrics(&mut notes, "");
        assert_eq!(notes[0].syllable.as_deref(), Some("la"));
        assert_eq!(report.filled, 1);
    }

    #[test]
    fn punctuation_and_spaces_are_separators() {
        let mut notes = vec![note(0.0, 1.0, 60), note(1.0, 1.0, 62)];
        assign_lyrics(&mut notes, "小，河。");
        assert_eq!(notes[0].syllable.as_deref(), Some("小"));
        assert_eq!(notes[1].syllable.as_deref(), Some("河"));
    }

    #[test]
    fn per_note_lyrics_win_over_the_flat_string() {
        let mut notes = vec![note(0.0, 1.0, 60), note(1.0, 1.0, 62), note(2.0, 1.0, 64)];
        notes[0].syllable = Some("春".into());
        notes[1].syllable = Some("江".into());
        let report = assign_lyrics(&mut notes, "花月夜");
        assert_eq!(notes[0].syllable.as_deref(), Some("春"));
        assert_eq!(notes[1].syllable.as_deref(), Some("江"));
        // 只有第三个音没有逐音歌词 → 整串歌词从第一个 token 开始补给它。
        assert_eq!(notes[2].syllable.as_deref(), Some("花"));
        assert_eq!(report.assigned, 3);
    }

    #[test]
    fn per_note_lyric_supports_tone_and_slur() {
        let mut notes = vec![note(0.0, 1.0, 60), note(1.0, 1.0, 62), note(2.0, 1.0, 64)];
        notes[0].syllable = Some("小4".into());
        notes[1].syllable = Some("-".into());
        notes[2].syllable = Some("河".into());
        let report = assign_lyrics(&mut notes, "");
        assert_eq!(notes[0].syllable.as_deref(), Some("小"));
        assert_eq!(notes[0].tone, Some('4'));
        assert!(notes[1].slur);
        assert_eq!(notes[1].syllable.as_deref(), Some("小"));
        assert_eq!(notes[2].syllable.as_deref(), Some("河"));
        assert_eq!(report.slurred, 1);
        assert_eq!(report.filled, 0);
    }

    #[test]
    fn mixed_notes_only_consume_as_many_tokens_as_needed() {
        let mut notes = vec![note(0.0, 1.0, 60), note(1.0, 1.0, 62), note(2.0, 1.0, 64)];
        notes[1].syllable = Some("河".into());
        let report = assign_lyrics(&mut notes, "小 淌 水");
        assert_eq!(
            notes[0].syllable.as_deref(),
            Some("小"),
            "第一个空位拿第一个 token"
        );
        assert_eq!(notes[1].syllable.as_deref(), Some("河"), "显式歌词不被覆盖");
        assert_eq!(
            notes[2].syllable.as_deref(),
            Some("淌"),
            "第三个空位拿第二个 token"
        );
        assert_eq!(report.assigned, 3);
    }

    #[test]
    fn explicit_sp_clears_a_note() {
        let mut notes = vec![note(0.0, 1.0, 60), note(1.0, 1.0, 62)];
        notes[0].syllable = Some("<SP>".into());
        notes[1].syllable = Some("河".into());
        assign_lyrics(&mut notes, "");
        assert_eq!(notes[0].syllable.as_deref(), Some("<SP>"));
        assert!(notes[0].is_rest());
        assert_eq!(notes[1].syllable.as_deref(), Some("河"));
    }

    #[test]
    fn breath_marker_is_kept_as_a_token() {
        let mut notes = vec![note(0.0, 1.0, 60), note(1.0, 1.0, 62)];
        assign_lyrics(&mut notes, "<AP> 河");
        assert_eq!(notes[0].syllable.as_deref(), Some("<AP>"));
        assert!(!notes[0].is_rest(), "吸气是可听见的，不算休止");
        assert_eq!(notes[1].syllable.as_deref(), Some("河"));
    }
}
