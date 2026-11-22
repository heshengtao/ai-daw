//! **ABC 谱改写：纯器乐工作流**（YuE2 官方做法）。
//!
//! YuE2 不会因为「歌词空着」就停止唱歌：它先写一份带 `Vocal` / `Ins` 两个声部的 ABC 谱，
//! 再按谱渲染音频。空歌词只是让 `Vocal` 声部少了词，模型照样给主旋律安排一个「人声」音色。
//! 上游给出的办法是**把 `Vocal` 的旋律搬进 `Ins`、`Vocal` 只留休止**，再拿这份改过的谱渲染
//! （见 `multimodal-art-projection/YuE` 的 `skills/yue2-music/instrumental`）。
//!
//! 这个模块只做那一步、且只做文本层：解析出每个声部的音符事件 → 把两个声部按时间合流
//! （旋律优先，空隙用伴奏补）→ 重新写回同一份 ABC。和弦标记跟着旋律走（YuE2 原生的谱里
//! 和弦就挂在 `Vocal` 那一行），所以搬完之后 `Ins` 同时担着「旋律 + 和弦」，和原生谱一致。
//!
//! 遇到的都是 YuE2 实际会写出来的子集：`X/T/M/L/Q/V/K` 头、`"和弦"`、`|` 小节线、`z` 休止、
//! `Z` 多小节休止、升降记号、八度 `,`/`'`、时值 `8` / `3/2` / `/2`。连音线 `-` 直接忽略
//! （YuE2 用它把音连到下一小节，忽略后每个音仍落在自己的时值上）；装饰音、`[K:]` 之类
//! 的内联字段原样跳过。

/// 浮点比较的容差：谱面时值都是 L 单位的整数倍，容差只用来吃掉解析出来的小数误差。
const EPS: f64 = 1e-6;

/// 谱面头里跟时间换算有关的两个字段。
struct Meta {
    /// 一小节等于多少个 L 单位（例：`M:2/4` + `L:1/32` → 16）。
    bar: f64,
}

impl Meta {
    fn parse(text: &str) -> Result<Self, String> {
        let mut meter = (4.0_f64, 4.0_f64);
        let mut unit_den = 8.0_f64;
        let mut has_unit = false;
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("M:") {
                if let Some((num, den)) = ratio(rest) {
                    if num > 0.0 && den > 0.0 {
                        meter = (num, den);
                    }
                }
            } else if let Some(rest) = line.strip_prefix("L:") {
                // `L:1/32` → 32 个「1/32」凑一整个全音符。
                if let Some((num, den)) = ratio(rest) {
                    if num > 0.0 && den > 0.0 {
                        unit_den = den / num;
                        has_unit = true;
                    }
                }
            }
        }
        if !has_unit {
            return Err("谱面缺少可用的 L: 头，算不出一小节多长".into());
        }
        let bar = (meter.0 / meter.1) * unit_den;
        Ok(Self { bar })
    }
}

/// `"2/4"` → `(2, 4)`；`"1/32"` → `(1, 32)`；`"3"` → `(3, 1)`。
fn ratio(text: &str) -> Option<(f64, f64)> {
    let text = text.trim();
    match text.split_once('/') {
        Some((a, b)) => Some((a.trim().parse().ok()?, b.trim().parse().ok()?)),
        None => Some((text.parse().ok()?, 1.0)),
    }
}

/// 一个音乐事件。`pitch = None` 表示休止。
#[derive(Clone, Debug)]
struct Ev {
    /// 起点（L 单位，从声部开头算）。
    t: f64,
    /// 时值（L 单位）。
    dur: f64,
    /// 音高记号（`^^c'` 这种原样字符串）。
    pitch: Option<String>,
    /// 这个位置上的和弦标记（`"Bbm7"` 里去引号的部分）。
    chords: Vec<String>,
}

impl Ev {
    fn rest(t: f64, dur: f64) -> Self {
        Self { t, dur, pitch: None, chords: Vec::new() }
    }
    fn end(&self) -> f64 {
        self.t + self.dur
    }
}

/// 把一段声部正文解析成事件表。
fn parse_body(body: &str, bar: f64) -> Result<Vec<Ev>, String> {
    let chars: Vec<char> = body.chars().collect();
    let mut i = 0usize;
    let mut t = 0.0f64;
    let mut chords: Vec<String> = Vec::new();
    let mut out: Vec<Ev> = Vec::new();

    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '|' | ':' => i += 1,
            '%' => break,
            '"' => {
                let mut text = String::new();
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    text.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    i += 1;
                }
                if !text.trim().is_empty() {
                    chords.push(text.trim().to_string());
                }
            }
            '[' => {
                // 内联字段 `[K:F]` / 和弦记法：跳过，不影响时值。
                while i < chars.len() && chars[i] != ']' {
                    i += 1;
                }
                i += 1;
            }
            '{' => {
                while i < chars.len() && chars[i] != '}' {
                    i += 1;
                }
                i += 1;
            }
            '-' | ')' | '(' => i += 1,
            'z' | 'x' => {
                i += 1;
                let dur = read_duration(&chars, &mut i);
                out.push(Ev { t, dur, pitch: None, chords: std::mem::take(&mut chords) });
                t += dur;
            }
            'Z' => {
                i += 1;
                let count = read_count(&chars, &mut i).unwrap_or(1);
                let dur = bar * count as f64;
                out.push(Ev { t, dur, pitch: None, chords: std::mem::take(&mut chords) });
                t += dur;
            }
            '^' | '_' | '=' | 'A'..='G' | 'a'..='g' => {
                let pitch = read_note(&chars, &mut i);
                let dur = read_duration(&chars, &mut i);
                out.push(Ev { t, dur, pitch: Some(pitch), chords: std::mem::take(&mut chords) });
                t += dur;
            }
            _ => i += 1,
        }
    }
    if !chords.is_empty() {
        // 行尾孤零零的和弦：挂到最后一个事件上，别丢。
        if let Some(last) = out.last_mut() {
            last.chords.append(&mut chords);
        }
    }
    Ok(out)
}

/// 读一个音高记号：`^^c'` / `B,` / `=f` / `d`。
fn read_note(chars: &[char], i: &mut usize) -> String {
    let mut text = String::new();
    // 升降记号
    while *i < chars.len() && matches!(chars[*i], '^' | '_' | '=') {
        text.push(chars[*i]);
        *i += 1;
    }
    if *i < chars.len() && chars[*i].is_ascii_alphabetic() {
        text.push(chars[*i]);
        *i += 1;
    }
    // 八度：`,` 往下、`'` 往上，可以连写
    while *i < chars.len() && matches!(chars[*i], ',' | '\'') {
        text.push(chars[*i]);
        *i += 1;
    }
    text
}

/// 读一个时值：`8` / `3/2` / `/2` / 空（= 1 个 L 单位）。
fn read_duration(chars: &[char], i: &mut usize) -> f64 {
    let mut num = String::new();
    while *i < chars.len() && chars[*i].is_ascii_digit() {
        num.push(chars[*i]);
        *i += 1;
    }
    let mut den = String::new();
    if *i < chars.len() && chars[*i] == '/' {
        *i += 1;
        while *i < chars.len() && chars[*i].is_ascii_digit() {
            den.push(chars[*i]);
            *i += 1;
        }
        let den: f64 = den.parse().unwrap_or(1.0);
        let num: f64 = num.parse().unwrap_or(1.0);
        return num / den;
    }
    num.parse().unwrap_or(1.0)
}

/// 读一个小整数（`Z4` 里的 4）。
fn read_count(chars: &[char], i: &mut usize) -> Option<u32> {
    let mut num = String::new();
    while *i < chars.len() && chars[*i].is_ascii_digit() {
        num.push(chars[*i]);
        *i += 1;
    }
    num.parse().ok()
}

/// 按时间合流两个声部：旋律声部优先，空隙用伴奏声部补，谁都没有的地方补休止。
///
/// 只搬「有音」的事件：休止由空隙自动补出来，否则旋律行的休止会把伴奏顶掉。
fn merge(melody: &[Ev], accompaniment: &[Ev], total: f64) -> Vec<Ev> {
    let mut out: Vec<Ev> = Vec::new();
    let mut t = 0.0f64;
    while t < total - EPS {
        let m = melody
            .iter()
            .find(|e| e.pitch.is_some() && (e.t - t).abs() < EPS);
        let a = accompaniment
            .iter()
            .find(|e| e.pitch.is_some() && (e.t - t).abs() < EPS);
        let ev = match (m, a) {
            (Some(m), _) => m.clone(),
            (None, Some(a)) => a.clone(),
            (None, None) => {
                let next = melody
                    .iter()
                    .chain(accompaniment.iter())
                    .filter(|e| e.pitch.is_some())
                    .map(|e| e.t)
                    .filter(|x| *x > t + EPS)
                    .fold(total, f64::min);
                out.push(Ev::rest(t, (next - t).max(EPS)));
                t = next;
                continue;
            }
        };
        let mut ev = ev;
        ev.t = t;
        ev.chords.clear();
        t += ev.dur;
        out.push(ev);
    }

    // 和弦标记跟旋律走：按时间就近挂到改写后的某个音上（没有音的，挂到最后一个音）。
    let mut chords: Vec<(f64, String)> = melody
        .iter()
        .chain(accompaniment.iter())
        .flat_map(|e| e.chords.iter().map(move |c| (e.t, c.clone())))
        .collect();
    chords.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    for (time, chord) in chords {
        // 只挂到「这个位置或之后」的音上：落在收尾休止里的和弦直接丢掉，
        // 否则它会被塞到最后一个音上，在小节中间冒出一个多余的标记。
        let target = out.iter().position(|e| e.pitch.is_some() && e.t + EPS >= time);
        if let Some(index) = target {
            if !out[index].chords.contains(&chord) {
                out[index].chords.push(chord);
            }
        }
    }
    out
}

/// 把事件表写回 ABC 正文：按小节插 `|`，跨小节的音拆成两段。
fn render(events: &[Ev], bar: f64) -> String {
    let mut out = String::new();
    let mut t = 0.0f64;
    let mut last_bar = 0i64;
    for ev in events {
        if ev.t > t + EPS {
            let gap = ev.t - t;
            write_span(&mut out, &mut t, gap, None, bar, &mut last_bar);
        }
        // 和弦标记要落在小节线**之后**，所以先把小节线补齐再写和弦。
        sync_bar(&mut out, t, bar, &mut last_bar);
        for chord in &ev.chords {
            out.push('"');
            out.push_str(chord);
            out.push('"');
        }
        write_span(&mut out, &mut t, ev.dur, ev.pitch.as_deref(), bar, &mut last_bar);
    }
    out.trim_end().to_string()
}

/// 如果 `t` 已经跨进新的小节，就把缺的小节线补上。
fn sync_bar(out: &mut String, t: f64, bar: f64, last_bar: &mut i64) {
    let index = (t / bar).floor() as i64;
    while *last_bar < index {
        out.push('|');
        *last_bar += 1;
    }
}

/// 写一段「有音 / 休止」的时长，遇到小节线就断开。
fn write_span(out: &mut String, t: &mut f64, dur: f64, pitch: Option<&str>, bar: f64, last_bar: &mut i64) {
    let mut left = dur;
    while left > EPS {
        let pos = *t % bar;
        let to_bar = if pos < EPS { bar } else { bar - pos };
        let step = left.min(to_bar);
        sync_bar(out, *t, bar, last_bar);
        match pitch {
            Some(p) => {
                out.push_str(p);
                push_duration(out, step);
            }
            None => {
                out.push('z');
                push_duration(out, step);
            }
        }
        *t += step;
        left -= step;
    }
}

/// 时值转文本：`4` / `3/2`（1 个 L 单位省略不写）。
fn push_duration(out: &mut String, units: f64) {
    if (units - 1.0).abs() < EPS {
        return;
    }
    let rounded = units.round();
    if (units - rounded).abs() < EPS {
        out.push_str(&(rounded as i64).to_string());
        return;
    }
    for den in 2..=16u32 {
        let num = units * den as f64;
        if (num - num.round()).abs() < EPS {
            out.push_str(&(num.round() as i64).to_string());
            out.push('/');
            out.push_str(&den.to_string());
            return;
        }
    }
    // 极端情况：写成两位小数，ABC 也认。
    out.push_str(&format!("{units:.2}"));
}

/// 一行里的一段（`V:` 头 + 正文）。
struct Section {
    header: String,
    body: String,
    id: String,
}

impl Section {
    fn id_of(header: &str) -> String {
        header
            .trim_start()
            .trim_start_matches("V:")
            .trim()
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string()
    }
}

enum Item {
    Line(String),
    Voice(Section),
}

fn split(text: &str) -> Vec<Item> {
    let lines: Vec<&str> = text.lines().collect();
    let mut items = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let line = lines[i];
        if line.trim_start().starts_with("V:") {
            let id = Section::id_of(line);
            let mut body = String::new();
            i += 1;
            // 正文：紧跟的一行（YuE2 的输出每个声部只有一行正文）。
            while i < lines.len() {
                let next = lines[i];
                let trimmed = next.trim();
                if trimmed.starts_with("V:") || trimmed.starts_with('%') || trimmed.is_empty() {
                    break;
                }
                if !body.is_empty() {
                    body.push(' ');
                }
                body.push_str(trimmed);
                i += 1;
            }
            items.push(Item::Voice(Section { header: line.to_string(), body, id }));
        } else {
            items.push(Item::Line(line.to_string()));
            i += 1;
        }
    }
    items
}

/// 原样输出一个声部：头 + 正文（正文空着就不占一行）。
fn push_section(out: &mut Vec<String>, section: &Section) {
    out.push(section.header.clone());
    if !section.body.trim().is_empty() {
        out.push(section.body.clone());
    }
}

/// 一整小节的休止（`z16`；小节长度不是整数时退回小数写法）。
fn bar_rest(bar: f64, count: usize) -> String {
    if count == 0 {
        return String::new();
    }
    let mut out = String::new();
    for i in 0..count {
        if i > 0 {
            out.push('|');
        }
        out.push('z');
        push_duration(&mut out, bar);
    }
    out
}

/// **把 `Vocal` 的旋律搬进 `Ins`，`Vocal` 只留休止。**
///
/// 返回改写后的整份 ABC。谱面里没有 `Vocal` / `Ins` 这一对、或 `Vocal` 本来就没有音时，
/// 原样返回（已经没啥可搬的了）。
pub fn move_vocal_to_ins(text: &str) -> Result<String, String> {
    let meta = Meta::parse(text)?;
    let items = split(text);
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < items.len() {
        let paired = match (&items[i], items.get(i + 1)) {
            (Item::Voice(a), Some(Item::Voice(b))) => a.id == "Vocal" && b.id == "Ins",
            _ => false,
        };
        if paired {
            let (Item::Voice(vocal), Some(Item::Voice(ins))) = (&items[i], items.get(i + 1)) else {
                unreachable!()
            };
            let melody = parse_body(&vocal.body, meta.bar)?;
            let accompaniment = parse_body(&ins.body, meta.bar)?;
            let total = melody
                .iter()
                .chain(accompaniment.iter())
                .map(|e| e.end())
                .fold(0.0f64, f64::max);
            let bars = (total / meta.bar - EPS).ceil().max(1.0) as usize;

            if melody.iter().all(|e| e.pitch.is_none()) {
                // 这一段的 Vocal 本来就全是休止：原样保留，别把伴奏也搬乱。
                push_section(&mut out, vocal);
                push_section(&mut out, ins);
            } else {
                let merged = merge(&melody, &accompaniment, bars as f64 * meta.bar);
                out.push(vocal.header.clone());
                out.push(bar_rest(meta.bar, bars));
                out.push(ins.header.clone());
                out.push(render(&merged, meta.bar));
            }
            i += 2;
            continue;
        }
        match &items[i] {
            Item::Line(line) => out.push(line.clone()),
            Item::Voice(section) => push_section(&mut out, section),
        }
        i += 1;
    }

    let mut text = out.join("\n");
    text.push('\n');
    // 自检：改写后 `Vocal` 不该再有任何音（这是这一步的全部意义）。
    let check = split(&text);
    for (index, item) in check.iter().enumerate() {
        let Item::Voice(section) = item else { continue };
        if section.id != "Vocal" {
            continue;
        }
        // 只有「跟着一对 Ins」的段落才应该被清空；孤儿段落原样保留。
        let paired = matches!(check.get(index + 1), Some(Item::Voice(next)) if next.id == "Ins");
        if !paired {
            continue;
        }
        if parse_body(&section.body, meta.bar)?.iter().any(|e| e.pitch.is_some()) {
            return Err("改写后 Vocal 声部仍有音符".into());
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实 YuE2 输出形状的一段谱（`Vocal` 主旋律 + `Ins` 伴奏）。
    const SAMPLE: &str = "X:1\nT:\nM:2/4\nL:1/32\nQ:1/4=96\n\
V: Vocal clef=treble name=\"Vocal Melody\" snm=\"Vocal\"\n\
V: Ins clef=treble name=\"Ins Melody\" snm=\"Ins\"\n\
K:Bbm\n\
% intro\n\
V: Vocal\n\
\"Ebm7\"z16|\"Ebm7\"z16|\n\
V: Ins\n\
z8z2f2a2b2|z2d'4d'2d'4d'4|\n\
% verse\n\
V: Vocal\n\
\"Bbm7\"f8f6f2-|\"Bbm7\"f4e4d4e4|\n\
V: Ins\n\
Z2|\n";

    #[test]
    fn melody_moves_into_ins_and_vocal_is_silent() {
        let out = move_vocal_to_ins(SAMPLE).expect("rewrite");
        let items = split(&out);
        let meta = Meta::parse(&out).unwrap();
        let mut vocal_notes = 0;
        let mut ins_notes = 0;
        for (index, item) in items.iter().enumerate() {
            let Item::Voice(section) = item else { continue };
            let events = parse_body(&section.body, meta.bar).unwrap();
            let notes = events.iter().filter(|e| e.pitch.is_some()).count();
            match section.id.as_str() {
                // verse 段（跟 Ins 配对）必须静音；intro 段 Vocal 本来就没音。
                "Vocal" => vocal_notes += notes,
                "Ins" => ins_notes += notes,
                _ => {}
            }
            let _ = index;
        }
        assert_eq!(vocal_notes, 0, "Vocal 不该再有音符：{out}");
        assert!(ins_notes >= 6, "旋律应该搬进 Ins：{out}");
    }

    #[test]
    fn bar_count_survives_the_rewrite() {
        let out = move_vocal_to_ins(SAMPLE).expect("rewrite");
        let meta = Meta::parse(&out).unwrap();
        let before = split(SAMPLE)
            .iter()
            .filter_map(|i| match i {
                Item::Voice(s) if s.id == "Ins" => Some(parse_body(&s.body, meta.bar).unwrap()),
                _ => None,
            })
            .flatten()
            .map(|e| e.end())
            .fold(0.0f64, f64::max);
        let after = split(&out)
            .iter()
            .filter_map(|i| match i {
                Item::Voice(s) if s.id == "Ins" => Some(parse_body(&s.body, meta.bar).unwrap()),
                _ => None,
            })
            .flatten()
            .map(|e| e.end())
            .fold(0.0f64, f64::max);
        assert!((before - after).abs() < EPS, "{before} vs {after}\n{out}");
    }

    #[test]
    fn chords_follow_the_melody() {
        let out = move_vocal_to_ins(SAMPLE).expect("rewrite");
        let items = split(&out);
        let meta = Meta::parse(&out).unwrap();
        let ins = items
            .iter()
            .filter_map(|i| match i {
                Item::Voice(s) if s.id == "Ins" => Some(s),
                _ => None,
            })
            .last()
            .expect("ins section");
        let chords: Vec<String> = parse_body(&ins.body, meta.bar)
            .unwrap()
            .iter()
            .flat_map(|e| e.chords.clone())
            .collect();
        assert!(chords.iter().any(|c| c.contains("Bbm7")), "{chords:?}");
    }

    #[test]
    fn unpaired_vocal_section_is_left_alone() {
        let text = "X:1\nM:4/4\nL:1/8\nV: Vocal\n\"C\"C8|\nV: Other\nD8|\n";
        let out = move_vocal_to_ins(text).expect("rewrite");
        assert!(out.contains("C8"), "{out}");
    }

    #[test]
    fn already_instrumental_input_is_unchanged() {
        let text = "X:1\nM:4/4\nL:1/8\nV: Vocal\nz8|\nV: Ins\nC8|\n";
        let out = move_vocal_to_ins(text).expect("rewrite");
        assert!(out.contains("z8"), "{out}");
        assert!(out.contains("C8"), "{out}");
    }

    #[test]
    fn missing_meta_is_an_error() {
        assert!(move_vocal_to_ins("V: Vocal\nC4|\nV: Ins\nD4|\n").is_err());
    }
}
