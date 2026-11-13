//! **标准 MIDI 文件（SMF）→ 音符**：给 audio.cpp 的 MuScriptor `midi` artifact 用。
//!
//! audio.cpp 的转录任务可以直接吐 MIDI 文件（`kind = "midi"` 的 artifact）。工程里需要的
//! 是「音高 / 起点 / 时长 / 力度 / 乐器」这几样，所以这里只解析 SMF 的这一层，不认识的
//! 事件（sysex、歌词、控制器、弯音）一律跳过——**不要**为了未知事件让整份文件失败。
//!
//! 约定：
//! - 时间用「拍点 + tempo map」换算成秒（默认 500000 µs/四分音符，即 120 BPM）；
//! - 鼓通道（channel 9）的每个音都按 10 ms 的瞬时音符记录（与 MuScriptor 的事件语义一致）；
//! - note-on 与 note-off 按 `(channel, pitch)` 配对，先开先关；没等到 note-off 的音符丢弃；
//! - 乐器取**音符起点之前**该通道最后一次 program change。

/// 从 MIDI 文件里解析出来的一个音符。
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedNote {
    /// MIDI 音高。
    pub pitch: u8,
    /// 起点（秒）。
    pub start_sec: f64,
    /// 时长（秒，至少 10 ms）。
    pub duration_sec: f64,
    /// 力度（1..=127）。
    pub velocity: u8,
    /// 乐器（GM program；鼓通道给 128）。
    pub program: u8,
    /// 通道号（0..=15）。
    pub channel: u8,
    /// 是否鼓通道（channel 9）。
    pub is_drum: bool,
}

/// 默认 tempo：500000 µs / 四分音符。
const DEFAULT_TEMPO: u32 = 500_000;
/// 一份文件最多收这么多音符：畸形文件不该把内存打爆。
const MAX_NOTES: usize = 20_000;

/// 读大端 u16。
fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    let slice = bytes.get(at..at + 2)?;
    Some(u16::from_be_bytes([slice[0], slice[1]]))
}

/// 读大端 u32。
fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at + 4)?;
    Some(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// 读变长整数（VLQ，最多 4 字节）。
fn read_varlen(bytes: &[u8], at: &mut usize) -> Option<u32> {
    let mut value: u32 = 0;
    for _ in 0..4 {
        let byte = *bytes.get(*at)?;
        *at += 1;
        value = (value << 7) | u32::from(byte & 0x7F);
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

/// 一个原始事件（只保留解析音符需要的几类）。
enum RawEvent {
    /// `(tick, µs per quarter)`。
    Tempo(u64, u32),
    /// `(tick, channel, program)`。
    Program(u64, u8, u8),
    /// `(tick, channel, pitch, velocity)`；velocity = 0 表示 note-on 的关闭形式。
    NoteOn(u64, u8, u8, u8),
    /// `(tick, channel, pitch)`。
    NoteOff(u64, u8, u8),
}

/// 解析一份 SMF，返回按起点排序的音符。
pub fn parse_smf(bytes: &[u8]) -> Result<Vec<ImportedNote>, String> {
    if bytes.len() < 14 || &bytes[0..4] != b"MThd" {
        return Err("不是标准 MIDI 文件（缺 MThd）".into());
    }
    let header_len = u32_at(bytes, 4).ok_or("MIDI 头被截断")? as usize;
    let fmt = u16_at(bytes, 8).ok_or("MIDI 头被截断")?;
    let tracks = u16_at(bytes, 10).ok_or("MIDI 头被截断")? as usize;
    let division = u16_at(bytes, 12).ok_or("MIDI 头被截断")?;
    if fmt > 2 {
        return Err(format!("不支持的 MIDI 格式：{fmt}"));
    }
    if division & 0x8000 != 0 {
        return Err("不支持 SMPTE 时间码的 MIDI（需要每四分音符的 tick 数）".into());
    }
    let ppqn = u32::from(division).max(1);

    let mut events: Vec<RawEvent> = Vec::new();
    let mut pos = 8 + header_len;
    let mut seen = 0usize;
    while pos + 8 <= bytes.len() && seen < tracks.max(1) {
        let id = &bytes[pos..pos + 4];
        let len = u32_at(bytes, pos + 4).ok_or("MIDI 轨道长度被截断")? as usize;
        let body_start = pos + 8;
        let body_end = body_start.checked_add(len).ok_or("MIDI 轨道长度异常")?;
        if id == b"MTrk" && body_end <= bytes.len() {
            parse_track(&bytes[body_start..body_end], &mut events);
            seen += 1;
        }
        pos = body_end;
    }

    // tempo 段：默认值 + 文件里出现的变化，按拍点排序。
    let mut tempos: Vec<(u64, u32)> = events
        .iter()
        .filter_map(|event| match event {
            RawEvent::Tempo(tick, us) => Some((*tick, *us)),
            _ => None,
        })
        .collect();
    tempos.sort_by_key(|(tick, _)| *tick);
    if tempos.first().is_none_or(|(tick, _)| *tick > 0) {
        tempos.insert(0, (0, DEFAULT_TEMPO));
    }
    let seconds_of = |tick: u64| -> f64 {
        let mut seconds = 0.0;
        for (index, (start, us)) in tempos.iter().enumerate() {
            if *start > tick {
                break;
            }
            let end = tempos.get(index + 1).map(|(t, _)| *t).unwrap_or(u64::MAX);
            let span = tick.min(end) - start;
            seconds += span as f64 / f64::from(ppqn) * f64::from(*us) / 1_000_000.0;
        }
        seconds
    };

    // program change：按通道记下 (tick, program)，查音符起点之前的最后一个。
    let mut programs: Vec<(u8, u64, u8)> = events
        .iter()
        .filter_map(|event| match event {
            RawEvent::Program(tick, channel, program) => Some((*channel, *tick, *program)),
            _ => None,
        })
        .collect();
    programs.sort_by_key(|(_, tick, _)| *tick);
    let program_at = |channel: u8, tick: u64| -> u8 {
        programs
            .iter()
            .rev()
            .find(|(ch, at, _)| *ch == channel && *at <= tick)
            .map(|(_, _, program)| *program)
            .unwrap_or(0)
    };

    // note-on / note-off 配对：同一 (channel, pitch) 用栈，先开先关。
    let mut open: Vec<(u8, u8, u64, u8)> = Vec::new();
    let mut notes: Vec<ImportedNote> = Vec::new();
    for event in &events {
        match *event {
            RawEvent::NoteOn(tick, channel, pitch, velocity) if velocity > 0 => {
                if open.len() < MAX_NOTES * 4 {
                    open.push((channel, pitch, tick, velocity));
                }
            }
            RawEvent::NoteOn(tick, channel, pitch, 0) | RawEvent::NoteOff(tick, channel, pitch) => {
                if let Some(index) = open
                    .iter()
                    .rposition(|(ch, key, _, _)| *ch == channel && *key == pitch)
                {
                    let (_, _, start, velocity) = open.remove(index);
                    if notes.len() >= MAX_NOTES {
                        continue;
                    }
                    let is_drum = channel == 9;
                    let start_sec = seconds_of(start);
                    let end_sec = seconds_of(tick.max(start));
                    notes.push(ImportedNote {
                        pitch: pitch.min(127),
                        start_sec,
                        duration_sec: if is_drum {
                            0.01
                        } else {
                            (end_sec - start_sec).max(0.01)
                        },
                        velocity: velocity.clamp(1, 127),
                        program: if is_drum {
                            128
                        } else {
                            program_at(channel, start)
                        },
                        channel,
                        is_drum,
                    });
                }
            }
            _ => {}
        }
    }
    notes.sort_by(|a, b| {
        a.start_sec
            .partial_cmp(&b.start_sec)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.pitch.cmp(&b.pitch))
    });
    Ok(notes)
}

/// 扫一条轨道，把认识的事件推进 `events`（running status 在这里有状态，所以按轨道调用）。
fn parse_track(body: &[u8], events: &mut Vec<RawEvent>) {
    let mut at = 0usize;
    let mut tick: u64 = 0;
    let mut running: Option<u8> = None;
    while at < body.len() {
        let Some(delta) = read_varlen(body, &mut at) else {
            break;
        };
        tick += u64::from(delta);
        let Some(&first) = body.get(at) else {
            break;
        };
        if first == 0xFF {
            // meta：type, len, data
            at += 1;
            let Some(&kind) = body.get(at) else { break };
            at += 1;
            let Some(len) = read_varlen(body, &mut at) else {
                break;
            };
            let len = len as usize;
            let data = body.get(at..at + len);
            at += len;
            match (kind, data) {
                (0x51, Some(data)) if data.len() >= 3 => {
                    let us = u32::from(data[0]) << 16
                        | u32::from(data[1]) << 8
                        | u32::from(data[2]);
                    events.push(RawEvent::Tempo(tick, us.max(1)));
                }
                (0x2F, _) => break, // end of track
                _ => {}
            }
            continue;
        }
        if first == 0xF0 || first == 0xF7 {
            // sysex：只按长度跳过
            at += 1;
            let Some(len) = read_varlen(body, &mut at) else {
                break;
            };
            at += len as usize;
            continue;
        }
        // 通道消息：低半字节决定数据字节数。
        let status = if first & 0x80 != 0 {
            at += 1;
            running = Some(first);
            first
        } else {
            match running {
                Some(status) => status,
                None => break,
            }
        };
        let high = status & 0xF0;
        let channel = status & 0x0F;
        let data_len = match high {
            0xC0 | 0xD0 => 1,
            _ => 2,
        };
        let Some(data) = body.get(at..at + data_len) else {
            break;
        };
        at += data_len;
        let d0 = data[0] & 0x7F;
        let d1 = data.get(1).copied().unwrap_or(0) & 0x7F;
        match high {
            0x90 if d1 > 0 => events.push(RawEvent::NoteOn(tick, channel, d0, d1)),
            0x90 | 0x80 => events.push(RawEvent::NoteOff(tick, channel, d0)),
            0xC0 => events.push(RawEvent::Program(tick, channel, d0)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varlen(mut value: u32) -> Vec<u8> {
        let mut out = vec![(value & 0x7F) as u8];
        value >>= 7;
        while value > 0 {
            out.insert(0, ((value & 0x7F) as u8) | 0x80);
            value >>= 7;
        }
        out
    }

    fn smf(events: &[u8]) -> Vec<u8> {
        let mut track = Vec::new();
        track.extend_from_slice(b"MTrk");
        track.extend_from_slice(&(events.len() as u32).to_be_bytes());
        track.extend_from_slice(events);
        let mut out = Vec::new();
        out.extend_from_slice(b"MThd");
        out.extend_from_slice(&6u32.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes()); // format 0
        out.extend_from_slice(&1u16.to_be_bytes()); // one track
        out.extend_from_slice(&480u16.to_be_bytes()); // 480 PPQN
        out.extend_from_slice(&track);
        out
    }

    #[test]
    fn parses_a_note_with_tempo() {
        let mut events = Vec::new();
        events.extend(varlen(0));
        events.extend_from_slice(&[0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20]); // 120 BPM
        events.extend(varlen(0));
        events.extend_from_slice(&[0xC0, 0x00]); // piano
        events.extend(varlen(0));
        events.extend_from_slice(&[0x90, 60, 100]);
        events.extend(varlen(480)); // half a second at 120 BPM
        events.extend_from_slice(&[0x80, 60, 0]);
        events.extend(varlen(0));
        events.extend_from_slice(&[0xFF, 0x2F, 0x00]);

        let notes = parse_smf(&smf(&events)).expect("应能解析");
        assert_eq!(notes.len(), 1);
        let note = &notes[0];
        assert_eq!(note.pitch, 60);
        assert_eq!(note.program, 0);
        assert_eq!(note.velocity, 100);
        assert!((note.start_sec - 0.0).abs() < 1e-9);
        assert!((note.duration_sec - 0.5).abs() < 1e-9);
    }

    #[test]
    fn running_status_and_drum_channel_are_handled() {
        let mut events = Vec::new();
        events.extend(varlen(0));
        events.extend_from_slice(&[0x99, 36, 100]); // drum channel 9, kick
        events.extend(varlen(0));
        events.extend_from_slice(&[42, 90]); // running status → hi-hat
        events.extend(varlen(240));
        events.extend_from_slice(&[0x89, 36, 0]);
        events.extend(varlen(0));
        events.extend_from_slice(&[42, 0]);
        events.extend(varlen(0));
        events.extend_from_slice(&[0xFF, 0x2F, 0x00]);

        let notes = parse_smf(&smf(&events)).expect("应能解析");
        assert_eq!(notes.len(), 2);
        assert!(notes.iter().all(|note| note.is_drum && note.program == 128));
        assert!(notes.iter().any(|note| note.pitch == 36));
        assert!(notes.iter().any(|note| note.pitch == 42));
        assert!(notes.iter().all(|note| note.duration_sec >= 0.01));
    }

    #[test]
    fn rejects_non_midi_bytes() {
        assert!(parse_smf(b"RIFF....WAVE").is_err());
    }
}
