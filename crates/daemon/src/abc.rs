//! **ABC 谱 → 工程 MIDI 音符**。
//!
//! YuE2 的 `/v1/tasks/run` 会顺带返回一份 ABC 谱（`meta.format = "abc"`）。乐谱视图读的是
//! 工程里的 MIDI，所以这里把 ABC 的常见子集解析成 `Note`，由宿主用 `AddNoteTrack` 写成
//! 一条（或每条声部一条）MIDI 轨——生成结果里就同时有音频和可编辑的谱。
//!
//! 支持的是 YuE2 实际会写出来的子集：`X/T/M/L/Q/V/K` 头、单声部/多声部、`|` 小节线、
//! `z` 休止、`Z` 多小节休止、`"C"` 和弦标记、八度 `,`/`'`、升降 `^`/`_`/`=`、时值
//! `4` / `3/2` / `/2`。装饰音、连音线、反复记号等不在子集里（遇到就跳过/忽略，不报错）。

use ai_daw_engine::model::Note;

/// 解析出来的一条声部。
#[derive(Debug, Clone)]
pub struct AbcVoice {
    /// 声部名（`V:` 后面的第一个 token；缺省 `ABC`）。
    pub name: String,
    pub notes: Vec<Note>,
    /// 最后一个音/休止的结束拍（相对声部开头）。
    pub end: f64,
}

struct VoiceState {
    name: String,
    notes: Vec<Note>,
    time: f64,
    end: f64,
    next_id: u64,
}

impl VoiceState {
    fn new(name: String) -> Self {
        Self {
            name,
            notes: Vec::new(),
            time: 0.0,
            end: 0.0,
            next_id: 1,
        }
    }
}

/// 把一份 ABC 文本解析成声部列表。没有任何音符时返回空表（由调用方报错）。
pub fn parse(text: &str) -> Result<Vec<AbcVoice>, String> {
    let mut meter = (4.0_f64, 4.0_f64);
    let mut unit = (1.0_f64, 8.0_f64);
    let mut voices: Vec<VoiceState> = Vec::new();
    let mut current = 0usize;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('%') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("M:") {
            meter = parse_ratio(rest).unwrap_or(meter);
            continue;
        }
        if let Some(rest) = line.strip_prefix("L:") {
            unit = parse_ratio(rest).unwrap_or(unit);
            continue;
        }
        if let Some(rest) = line.strip_prefix("V:") {
            let name = rest
                .split_whitespace()
                .next()
                .unwrap_or("ABC")
                .to_string();
            if let Some(index) = voices.iter().position(|voice| voice.name == name) {
                current = index;
            } else {
                voices.push(VoiceState::new(name));
                current = voices.len() - 1;
            }
            continue;
        }
        // 其他头（X/T/Q/K/...）跳过；`K:` 之后的行才通常是音符。
        if line.len() >= 2 && line.as_bytes()[1] == b':' {
            continue;
        }
        if voices.is_empty() {
            voices.push(VoiceState::new("ABC".into()));
            current = 0;
        }
        let bar_beats = meter.0 * 4.0 / meter.1;
        let unit_beats = 4.0 * unit.0 / unit.1;
        parse_line(
            line,
            &mut voices[current],
            bar_beats,
            unit_beats,
        );
    }

    Ok(voices
        .into_iter()
        .filter(|voice| !voice.notes.is_empty())
        .map(|voice| AbcVoice {
            name: voice.name,
            notes: voice.notes,
            end: voice.end.max(0.0),
        })
        .collect())
}

fn parse_ratio(text: &str) -> Option<(f64, f64)> {
    let text = text.split_whitespace().next()?;
    let (num, den) = text.split_once('/')?;
    let num = num.trim().parse::<f64>().ok()?;
    let den = den.trim().parse::<f64>().ok()?;
    if num <= 0.0 || den <= 0.0 {
        return None;
    }
    Some((num, den))
}

fn parse_line(line: &str, voice: &mut VoiceState, bar_beats: f64, unit_beats: f64) {
    let line = strip_chords_and_comments(line);
    let bytes = line.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        let ch = bytes[index] as char;
        match ch {
            ' ' | '\t' | '|' | ':' | '[' | ']' => {
                if ch == '|' {
                    // 小节线：把时间推到下一小节（正常谱面本来就对齐，浮点误差顺手修掉）。
                    let next = (voice.time / bar_beats - 1e-9).ceil() * bar_beats;
                    if next > voice.time {
                        voice.time = next;
                    }
                }
                index += 1;
            }
            'Z' => {
                index += 1;
                let (count, next) = read_number(&line, index);
                index = next;
                let bars = if count > 0.0 { count } else { 1.0 };
                voice.time += bars * bar_beats;
                voice.end = voice.end.max(voice.time);
            }
            'z' => {
                index += 1;
                let (mult, next) = read_duration(&line, index);
                index = next;
                voice.time += mult * unit_beats;
                voice.end = voice.end.max(voice.time);
            }
            'A'..='G' | 'a'..='g' | '^' | '_' | '=' => {
                let start = index;
                let (pitch, next) = read_pitch(&line, index);
                index = next;
                let (mult, next) = read_duration(&line, index);
                index = next;
                if let Some(pitch) = pitch {
                    let duration = (mult * unit_beats).clamp(1.0 / 32.0, 64.0);
                    voice.notes.push(Note {
                        id: voice.next_id,
                        pitch,
                        start: voice.time,
                        duration,
                        velocity: 100,
                        volume: 127,
                        offset: 0.0,
                        legato: false,
                        channel: None,
                        pressure: None,
                        expr: Default::default(),
                        lyric: None,
                    });
                    voice.next_id += 1;
                    voice.time += duration;
                    voice.end = voice.end.max(voice.time);
                } else {
                    // 不认识的 token：跳过一个字符，避免死循环。
                    index = start + 1;
                }
            }
            _ => {
                index += 1;
            }
        }
    }
}

/// 去掉 `"C"` 和弦标记与 `%` 注释；引号里的内容不影响时值。
fn strip_chords_and_comments(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut quoted = false;
    for ch in line.chars() {
        match ch {
            '"' => quoted = !quoted,
            '%' if !quoted => break,
            _ if !quoted => out.push(ch),
            _ => {}
        }
    }
    out
}

fn read_number(line: &str, mut index: usize) -> (f64, usize) {
    let bytes = line.as_bytes();
    let start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    let value = line[start..index].parse::<f64>().unwrap_or(0.0);
    (value, index)
}

/// 读时值：`4` = 4 个默认单位，`3/2` = 1.5，`/2` = 0.5，`//` = 0.25。
fn read_duration(line: &str, mut index: usize) -> (f64, usize) {
    let bytes = line.as_bytes();
    let mut numerator = 1.0;
    let mut has_number = false;
    let start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index > start {
        numerator = line[start..index].parse::<f64>().unwrap_or(1.0);
        has_number = true;
    }
    let mut denominator = 1.0;
    if index < bytes.len() && bytes[index] == b'/' {
        index += 1;
        if index < bytes.len() && bytes[index].is_ascii_digit() {
            let den_start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            denominator = line[den_start..index].parse::<f64>().unwrap_or(2.0);
        } else {
            denominator = 2.0;
            while index < bytes.len() && bytes[index] == b'/' {
                denominator *= 2.0;
                index += 1;
            }
        }
    }
    if !has_number && denominator == 1.0 {
        return (1.0, index);
    }
    ((numerator / denominator).max(1.0 / 64.0), index)
}

/// 读音高：升降号 + 字母 + 八度记号。返回 MIDI 音高。
fn read_pitch(line: &str, mut index: usize) -> (Option<u8>, usize) {
    let bytes = line.as_bytes();
    let mut accidental = 0i32;
    while index < bytes.len() {
        match bytes[index] {
            b'^' => {
                accidental += 1;
                index += 1;
            }
            b'_' => {
                accidental -= 1;
                index += 1;
            }
            b'=' => {
                accidental = 0;
                index += 1;
            }
            _ => break,
        }
    }
    if index >= bytes.len() {
        return (None, index);
    }
    let letter = bytes[index] as char;
    let base = match letter.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return (None, index + 1),
    };
    let mut octave = if letter.is_ascii_lowercase() { 5 } else { 4 };
    index += 1;
    while index < bytes.len() {
        match bytes[index] {
            b',' => {
                octave -= 1;
                index += 1;
            }
            b'\'' => {
                octave += 1;
                index += 1;
            }
            _ => break,
        }
    }
    let midi = (octave + 1) * 12 + base + accidental;
    if !(0..=127).contains(&midi) {
        return (None, index);
    }
    (Some(midi as u8), index)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"X:1
T:Demo
M:2/4
L:1/16
Q:1/4=95
V: Vocal clef=treble name="Vocal"
V: Ins clef=treble name="Ins"
K:C
% intro
V: Vocal
"C"C8|"C"z8|
V: Ins
z2c'2g2e2|f2ef2g3|
V: Vocal
"F"F8"G"G8|"C"C8|
V: Ins
f2ef2g3|z2c'2g2e2|f2ef2g3|"C"c'4e4|
"#;

    #[test]
    fn parses_two_voices_with_chords_and_rests() {
        let voices = parse(SAMPLE).unwrap();
        assert_eq!(voices.len(), 2, "{voices:?}");
        assert!(voices.iter().all(|voice| !voice.notes.is_empty()));
        assert!(voices.iter().all(|voice| voice.end > 0.0));
        // `c'` 是 C6（MIDI 84），`g` 是 G5（79）。
        let ins = voices.iter().find(|v| v.name == "Ins").unwrap();
        assert!(ins.notes.iter().any(|note| note.pitch == 84));
        assert!(ins.notes.iter().any(|note| note.pitch == 79));
        // 2/4 下每小节 2 拍；`c'4` = 4 个 1/16 = 1 拍。
        assert!(ins.notes.iter().any(|note| (note.duration - 1.0).abs() < 1e-9));
    }

    #[test]
    fn empty_or_header_only_is_empty() {
        assert!(parse("X:1\nM:4/4\nK:C\n").unwrap().is_empty());
    }

    #[test]
    fn durations_have_a_floor() {
        let voices = parse("X:1\nM:4/4\nL:1/16\nK:C\nV:1\nC/64|\n").unwrap();
        assert!(voices[0].notes[0].duration >= 1.0 / 32.0);
    }

    /// 真实的 YuE2 v0.9.3 输出（`gen4-out.json` 的 ABC payload）：两个声部、`Z4` 多小节
    /// 休止、`%` 注释、`"Em7"` 和弦标记都要能过。
    #[test]
    fn parses_the_real_yue2_artifact() {
        let abc = r#"X:1
T:
M:2/4
L:1/16
Q:1/4=95
V: Vocal clef=treble name="Vocal Melody" snm="Vocal"
V: Ins clef=treble name="Ins Melody" snm="Inst."
K:C
% intro
V: Vocal
z4"C"z4|"C"z8|"F"z4"G"z4|"C"z8|
V: Ins
Z|z2c'2g2e2|f2ef2g3|z2c'2g2e2|
V: Vocal
"F"z4"G"z4|
V: Ins
f2ef2g3|
% verse
V: Vocal
"C"c4e4|"Em7"B4e4|"Am"A4c4|"G"G4z4|
V: Ins
Z4|
"#;
        let voices = parse(abc).unwrap();
        assert_eq!(voices.len(), 2, "{voices:?}");
        let vocal = voices.iter().find(|v| v.name == "Vocal").unwrap();
        let ins = voices.iter().find(|v| v.name == "Ins").unwrap();
        assert!(vocal.notes.len() >= 4, "{vocal:?}");
        assert!(ins.notes.len() >= 8, "{ins:?}");
        assert!(vocal.notes.iter().any(|n| n.pitch == 72)); // c = C5
        assert!(ins.notes.iter().any(|n| n.pitch == 84)); // c' = C6
    }
}
