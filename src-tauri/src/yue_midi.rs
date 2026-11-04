//! **MIDI 片段 → ABC 谱**：翻唱的第二条参考来源。
//!
//! 音频要走 SheetSage2「听」一遍（2.5 GB 模型、分钟级），而工程里本来就有 MIDI 的话，
//! 旋律是**已知**的——直接写成 ABC 就行，零推理、瞬间完成，还顺手能把音符上的歌词
//! （`Note::lyric`）带回去填满歌词框。
//!
//! 输出刻意贴近 YuE2 自己吐的谱：`X/T/M/L/Q/K` 头 + `V: Vocal` / `V: Ins` 两个声部。
//! 单声部能不能喂给引擎没把握，所以按它的习惯给一个空的伴奏声部（`Z<n>` 全休止），
//! 这条在真机上验证过（见本模块测试与提交说明）。
//!
//! 局限（都是刻意的，别当 bug）：
//! - 拍号/速度取**工程基准值**，不追踪中途变速（`keyframes`）；
//! - 只写音高、时值、小节线、休止，不写和弦符号、力度、装饰音；
//! - 一个片段一个声部：轨道里如果有和声声部，请分别在两条轨上，或者先把旋律轨选出来。

use ai_daw_engine::model::{Note, Project};

/// 转换结果（界面一次拿全：谱、歌词、几个统计量）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct MidiScore {
    /// ABC 文本（直接喂 YuE2 的 `abc`）。
    pub abc: String,
    /// 从音符上收集的歌词（`-` 连音符号会被跳过）。
    pub lyrics: String,
    /// 小节数。
    pub bars: usize,
    /// 用到的音符数。
    pub notes: usize,
}

/// ABC 的一个八度字母表与它们对应的半音。
const LETTERS: [&str; 7] = ["C", "D", "E", "F", "G", "A", "B"];

/// MIDI 音高 → ABC 音名（含八度标记与必要的升降号）。
///
/// 一律用升号：`K:C` 下写成 `^C` 永远是对的，写成 `D_` 反而不一定。
fn abc_pitch(pitch: u8) -> String {
    let pitch = pitch.min(127) as i32;
    let octave = pitch / 12 - 1; // C4 = 60 → 4
    let semitone = pitch % 12;
    let (index, accidental) = match semitone {
        0 => (0, ""),
        1 => (0, "^"),
        2 => (1, ""),
        3 => (1, "^"),
        4 => (2, ""),
        5 => (3, ""),
        6 => (3, "^"),
        7 => (4, ""),
        8 => (4, "^"),
        9 => (5, ""),
        10 => (5, "^"),
        11 => (6, ""),
        _ => unreachable!(),
    };
    let base = LETTERS[index];
    // 中央八度（4）用大写，5 及以上用小写 + 撇号，3 及以下用大写 + 逗号。
    let (name, marks) = if octave >= 5 {
        (base.to_lowercase(), "'".repeat((octave - 5) as usize))
    } else {
        (base.to_string(), ",".repeat((4 - octave).max(0) as usize))
    };
    format!("{accidental}{name}{marks}")
}

/// 时值（以 `L:1/16` 为单位）→ ABC 长度后缀。1 个十六分音符用默认值（空串）。
fn abc_length(sixteenths: i64) -> String {
    let sixteenths = sixteenths.max(1);
    if sixteenths == 1 {
        String::new()
    } else {
        sixteenths.to_string()
    }
}

/// 把一个片段写成 ABC。
///
/// `track` / `clip` 用 id 定位；片段必须是 **MIDI**（有音符），音频片段会被明确拒绝。
pub fn clip_to_abc(
    project: &Project,
    track_id: u64,
    clip_id: Option<u64>,
) -> Result<MidiScore, String> {
    let track = project
        .tracks
        .iter()
        .find(|t| t.id.0 == track_id)
        .ok_or("没有这条轨道")?;
    let clip = track
        .clips
        .iter()
        .filter(|c| c.audio.is_none())
        .find(|c| clip_id.is_none_or(|id| c.id.0 == id))
        .ok_or("这条轨道上没有 MIDI 片段（翻唱的音频参考请用「工程内音频轨」）")?;
    if clip.notes.is_empty() {
        return Err("这个 MIDI 片段是空的".into());
    }

    let tempo = project.transport.tempo.max(1.0);
    let (num, den) = (
        project.transport.time_sig_num.max(1) as f64,
        project.transport.time_sig_den.max(1) as f64,
    );
    // 一小节几拍（按四分音符算）：4/4 → 4，6/8 → 3。
    let bar_beats = num * 4.0 / den;
    // 以 1/16 为最小单位：一拍 = 4 个。
    let bar_units = (bar_beats * 4.0).round().max(1.0) as i64;

    let mut notes: Vec<&Note> = clip.notes.iter().collect();
    notes.sort_by(|a, b| a.start.total_cmp(&b.start));

    let mut body = String::new();
    let mut lyrics: Vec<(i64, String)> = Vec::new();
    let mut cursor: i64 = 0; // 十六分音符为单位，从片段头开始
    let mut bars: i64 = 0;

    for note in &notes {
        let start = ((note.start.max(0.0)) * 4.0).round() as i64;
        let length = ((note.duration.max(0.0625)) * 4.0).round().max(1.0) as i64;
        // 前进到这一音；中间的空白写成休止。
        if start > cursor {
            let mut gap = start - cursor;
            while gap > 0 {
                let to_bar = bar_units - (cursor % bar_units);
                let take = gap.min(to_bar);
                body.push('z');
                body.push_str(&abc_length(take));
                cursor += take;
                gap -= take;
                if cursor % bar_units == 0 {
                    body.push('|');
                    bars += 1;
                }
            }
        }

        // 音符本身：跨小节的音用连线拆开（`c2-|c2`）。
        let token = abc_pitch(note.pitch);
        let mut remaining = length;
        let mut pos = cursor;
        while remaining > 0 {
            let to_bar = bar_units - (pos % bar_units);
            let take = remaining.min(to_bar);
            body.push_str(&token);
            body.push_str(&abc_length(take));
            pos += take;
            remaining -= take;
            if remaining > 0 {
                body.push('-');
                body.push('|');
                bars += 1;
            }
        }
        if let Some(lyric) = note.lyric.as_deref().map(str::trim) {
            if !lyric.is_empty() && lyric != "-" {
                lyrics.push((pos / bar_units, lyric.to_string()));
            }
        }
        cursor = pos.max(cursor);
    }

    // 收尾：补一个终止小节线。
    if !body.ends_with('|') {
        body.push('|');
        bars += 1;
    }

    let title = track.name.replace(['\n', '\r'], " ");
    let abc = format!(
        "X:1\nT:{title}\nM:{num:.0}/{den:.0}\nL:1/16\nQ:1/4={tempo:.0}\nK:C\n\
         V: Vocal clef=treble name=\"Vocal Melody\" snm=\"Vocal\"\n\
         V: Ins clef=treble name=\"Ins Melody\" snm=\"Inst.\"\n\
         V: Vocal\n{body}\n\
         V: Ins\nZ{bars}\n"
    );

    Ok(MidiScore {
        abc,
        lyrics: group_lyrics(&lyrics),
        bars: bars as usize,
        notes: notes.len(),
    })
}

/// 每 4 小节断一行（YuE2 的歌词按行唱，一口气一大段容易跑偏）。
fn group_lyrics(lyrics: &[(i64, String)]) -> String {
    if lyrics.is_empty() {
        return String::new();
    }
    let mut out: Vec<String> = Vec::new();
    let mut line: Vec<&str> = Vec::new();
    let mut bucket = lyrics[0].0 / 4;
    for (bar, text) in lyrics {
        if *bar / 4 != bucket && !line.is_empty() {
            out.push(line.join(" "));
            line.clear();
            bucket = *bar / 4;
        }
        line.push(text);
    }
    if !line.is_empty() {
        out.push(line.join(" "));
    }
    out.join("\n")
}

/// 把一个 MIDI 片段转成 ABC（界面「翻唱 → 工程内 MIDI」用；不改工程）。
#[tauri::command]
pub fn ai_yue_midi_abc(
    app: tauri::AppHandle,
    track: u64,
    clip: Option<u64>,
) -> Result<MidiScore, String> {
    use tauri::Manager;
    let state = app.state::<crate::state::AppState>();
    let project = {
        let engine = state.engine.lock().unwrap();
        engine.project().clone()
    };
    clip_to_abc(&project, track, clip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_daw_engine::model::{Clip, ClipId, MixerChannel, Project, Track, TrackId};

    fn track_with(name: &str, clips: Vec<Clip>) -> Track {
        Track {
            id: TrackId(1),
            name: name.into(),
            instrument: None,
            effects: Vec::new(),
            clips,
            mixer: MixerChannel::default(),
            sends: Vec::new(),
            automation: Vec::new(),
            meta: None,
        }
    }

    fn project_with(notes: Vec<Note>) -> Project {
        let mut project = Project::new("test");
        project.tracks.push(track_with(
            "Lead",
            vec![Clip {
                id: ClipId(1),
                start: 0.0,
                length: 4.0,
                notes,
                audio: None,
            }],
        ));
        project.transport.tempo = 120.0;
        project
    }

    fn note(pitch: u8, start: f64, duration: f64, lyric: Option<&str>) -> Note {
        Note {
            id: 0,
            pitch,
            start,
            duration,
            velocity: 100,
            volume: 127,
            offset: 0.0,
            legato: false,
            channel: None,
            pressure: None,
            expr: Default::default(),
            lyric: lyric.map(str::to_string),
        }
    }

    #[test]
    fn pitch_names_follow_abc_octaves() {
        assert_eq!(abc_pitch(60), "C"); // C4
        assert_eq!(abc_pitch(72), "c"); // C5
        assert_eq!(abc_pitch(48), "C,"); // C3
        assert_eq!(abc_pitch(84), "c'"); // C6
        assert_eq!(abc_pitch(61), "^C"); // C#4
        assert_eq!(abc_pitch(66), "^F"); // F#4
        assert_eq!(abc_pitch(70), "^A"); // A#4
    }

    #[test]
    fn a_simple_scale_becomes_bars_and_notes() {
        // C 大调音阶，8 个八分音符 = 4 拍 = 4/4 的一小节。
        let semitones = [0, 2, 4, 5, 7, 9, 11, 12];
        let notes = semitones
            .iter()
            .enumerate()
            .map(|(i, semi)| note(60 + semi, i as f64 * 0.5, 0.5, None))
            .collect();
        let score = clip_to_abc(&project_with(notes), 1, None).unwrap();
        assert!(score.abc.contains("M:4/4"), "{}", score.abc);
        assert!(score.abc.contains("V: Vocal"), "{}", score.abc);
        assert!(score.abc.contains("V: Ins"), "{}", score.abc);
        assert_eq!(score.bars, 1, "{}", score.abc);
        assert!(score.abc.contains("C2D2E2F2G2A2B2c2|"), "{}", score.abc);
    }

    /// 跨小节的音要拆成连音（`c16-|c4`），否则小节线会把时值弄错。
    #[test]
    fn long_notes_are_tied_across_the_bar_line() {
        let score = clip_to_abc(&project_with(vec![note(60, 0.0, 5.0, None)]), 1, None).unwrap();
        assert!(score.abc.contains("C16-|C4|"), "{}", score.abc);
        assert_eq!(score.bars, 2, "{}", score.abc);
    }

    #[test]
    fn rests_fill_the_gap_before_the_first_note() {
        let score = clip_to_abc(&project_with(vec![note(72, 2.0, 0.5, None)]), 1, None).unwrap();
        // 前面空了 2 拍 = 8 个十六分 = 半小节。
        assert!(score.abc.contains("z8"), "{}", score.abc);
    }

    #[test]
    fn lyrics_come_back_grouped_by_lines() {
        let notes = vec![
            note(60, 0.0, 0.5, Some("春")),
            note(62, 0.5, 0.5, Some("眠")),
            note(64, 1.0, 0.5, Some("-")),
            note(65, 1.5, 0.5, Some("不")),
        ];
        let score = clip_to_abc(&project_with(notes), 1, None).unwrap();
        assert_eq!(score.lyrics, "春 眠 不");
    }

    #[test]
    fn audio_clips_are_rejected_with_a_pointer() {
        let mut project = Project::new("test");
        project.tracks.push(track_with(
            "Vox",
            vec![Clip {
                id: ClipId(1),
                start: 0.0,
                length: 4.0,
                notes: Vec::new(),
                audio: Some(ai_daw_engine::model::AudioSource {
                    path: "/tmp/a.wav".into(),
                    offset_frames: 0,
                    frames: None,
                    rate: 1.0,
                    pitch_semitones: 0.0,
                }),
            }],
        ));
        let err = clip_to_abc(&project, 1, None).unwrap_err();
        assert!(err.contains("MIDI"), "{err}");
    }
}
