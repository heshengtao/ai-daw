//! **标准 MIDI 文件（SMF type 1）导出**——把工程里的 MIDI 轨写成 `.mid`。
//!
//! 结构：
//! - 第 0 条是**指挥轨**：工程名、速度图（`transport.tempo` + 关键点）、拍号图、段落标记；
//! - 之后每条**带音符的轨道**一条 MIDI 轨：轨道名、音符、逐音歌词（`FF 05` 歌词元事件）、
//!   `cc:<n>` 自动化曲线、以及轨道通用 MIDI 事件表里的 CC / program change / 压力事件。
//!
//! 时间轴：工程里「一拍」= 四分音符（`bar_beats = num * 4 / den` 也按这个换算），所以
//! `tick = round(beat * PPQ)`，[`PPQ`] = 960（1/32 音符 = 120 tick，三连音也不会抖）。
//!
//! 导出是**只读派生**：不改工程、不进历史，与 `export_wav` / 乐谱导出同一口径。

use crate::model::{Project, Track};
use ai_daw_plugin_api::MidiEvent;

/// 每拍（四分音符）的 tick 数。
pub const PPQ: u16 = 960;

/// 同 tick 上的事件顺序：元事件 → NoteOff → 控制器 → NoteOn。
///
/// NoteOff 必须排在 NoteOn 前面，否则同一 tick 上「先关后开」会变成「先开新音、再关掉它」，
/// 长音被截断；歌词 / 轨道名这类元事件排最前，读谱器不会把它们当成音符。
const ORDER_META: u8 = 0;
const ORDER_OFF: u8 = 1;
const ORDER_CTRL: u8 = 2;
const ORDER_ON: u8 = 3;

/// 一个待写出的轨道事件。
struct Event {
    tick: u32,
    order: u8,
    data: Vec<u8>,
}

impl Event {
    fn meta(tick: u32, kind: u8, payload: &[u8]) -> Self {
        let mut data = Vec::with_capacity(payload.len() + 4);
        data.extend([0xFF, kind]);
        vlq(payload.len() as u32, &mut data);
        data.extend_from_slice(payload);
        Event {
            tick,
            order: ORDER_META,
            data,
        }
    }

    fn channel(tick: u32, order: u8, bytes: &[u8]) -> Self {
        Event {
            tick,
            order,
            data: bytes.to_vec(),
        }
    }
}

/// 把工程写成 SMF type 1 字节流。
///
/// `only_tracks` 给定时只导出这些轨道（`track` 的工程 id）；没有可导出的音符轨则报错
/// （只有指挥轨的空文件对用户没有意义）。
pub fn project_to_smf(project: &Project, only_tracks: Option<&[u64]>) -> Result<Vec<u8>, String> {
    let tracks: Vec<&Track> = project
        .tracks
        .iter()
        .filter(|track| only_tracks.is_none_or(|ids| ids.contains(&track.id.0)))
        .filter(|track| track.clips.iter().any(|clip| !clip.notes.is_empty()))
        .collect();

    if tracks.is_empty() {
        return Err("工程里没有可导出的 MIDI 音符（音频轨不进 MIDI 文件）".into());
    }

    let mut out = Vec::new();
    header(&mut out, (tracks.len() + 1) as u16);

    // ---- 指挥轨：名字 / 速度 / 拍号 / 段落 ----
    let mut conductor = Vec::new();
    let name = if project.name.trim().is_empty() {
        "AI DAW"
    } else {
        project.name.trim()
    };
    conductor.push(Event::meta(0, 0x03, name.as_bytes()));
    conductor.push(tempo_event(0, project.transport.tempo));
    conductor.push(time_sig_event(
        0,
        project.transport.time_sig_num,
        project.transport.time_sig_den,
    ));
    for keyframe in &project.transport.keyframes {
        let tick = beat_to_tick(keyframe.beat);
        conductor.push(tempo_event(tick, keyframe.tempo));
        conductor.push(time_sig_event(
            tick,
            keyframe.time_sig_num,
            keyframe.time_sig_den,
        ));
    }
    for section in &project.sections {
        if section.name.trim().is_empty() {
            continue;
        }
        conductor.push(Event::meta(
            beat_to_tick(section.start),
            0x06, // marker
            section.name.trim().as_bytes(),
        ));
    }
    write_track(&mut out, conductor);

    // ---- 内容轨 ----
    let channels = channels_for(&tracks);
    for (index, track) in tracks.iter().enumerate() {
        let channel = channels[index];
        let mut events = Vec::new();
        events.push(Event::meta(0, 0x03, track.name.as_bytes()));

        for clip in &track.clips {
            for note in &clip.notes {
                let start = (clip.start + note.start + note.offset.max(0.0)).max(0.0);
                let on = beat_to_tick(start);
                let off = beat_to_tick(start + note.duration.max(1.0 / f64::from(PPQ))).max(on + 1);
                let channel = note.channel.unwrap_or(channel) & 0x0F;

                if let Some(lyric) = note
                    .lyric
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                {
                    events.push(Event::meta(on, 0x05, lyric.as_bytes()));
                }
                let velocity = note.velocity.clamp(1, 127);
                events.push(Event::channel(
                    on,
                    ORDER_ON,
                    &[0x90 | channel, note.pitch & 0x7F, velocity],
                ));
                // 松键力度没有语义，统一 64（多数 DAW 的默认值）。
                events.push(Event::channel(
                    off,
                    ORDER_OFF,
                    &[0x80 | channel, note.pitch & 0x7F, 64],
                ));
            }
        }

        for lane in &track.automation {
            let Some(controller) = lane
                .param
                .strip_prefix("cc:")
                .and_then(|rest| rest.trim().parse::<u8>().ok())
                .filter(|cc| *cc <= 127)
            else {
                continue;
            };
            // 首点之前是常数：补一个 tick 0 的值，否则导出后开头会沿用「上一个文件」的状态。
            if let Some(first) = lane.points.first() {
                if first.beat > 0.0 {
                    events.push(cc_event(0, channel, controller, first.value));
                }
            }
            for point in &lane.points {
                if point.beat.is_finite() {
                    events.push(cc_event(
                        beat_to_tick(point.beat),
                        channel,
                        controller,
                        point.value,
                    ));
                }
            }
        }

        if let Some(meta) = &track.meta {
            for item in &meta.midi_events {
                let tick = beat_to_tick(item.beat);
                let channel = item.channel & 0x0F;
                let data: Vec<u8> = match item.event {
                    MidiEvent::ControlChange { controller, value } => {
                        vec![0xB0 | channel, controller & 0x7F, value & 0x7F]
                    }
                    MidiEvent::ProgramChange { program } => vec![0xC0 | channel, program & 0x7F],
                    MidiEvent::ChannelPressure { value } => vec![0xD0 | channel, value & 0x7F],
                    MidiEvent::PolyPressure { note, value } => {
                        vec![0xA0 | channel, note & 0x7F, value & 0x7F]
                    }
                    // 逐音表达 / 弯音在 SMF 里没有一一对应的表示（MPE 也不通用），跳过。
                    _ => continue,
                };
                events.push(Event::channel(tick, ORDER_CTRL, &data));
            }
        }

        write_track(&mut out, events);
    }

    Ok(out)
}

/// 每轨分到的 MIDI 通道：鼓轨固定 9（GM 打击乐），其余按顺序轮转并跳过 9。
fn channels_for(tracks: &[&Track]) -> Vec<u8> {
    let mut next = 0u8;
    let mut channels = Vec::with_capacity(tracks.len());
    for track in tracks {
        if track.role() == Some("drums") {
            channels.push(9);
            continue;
        }
        while next == 9 {
            next = (next + 1) % 16;
        }
        channels.push(next);
        next = (next + 1) % 16;
    }
    channels
}

fn cc_event(tick: u32, channel: u8, controller: u8, value: f32) -> Event {
    let value = if value.is_finite() {
        value.round().clamp(0.0, 127.0) as u8
    } else {
        0
    };
    Event::channel(
        tick,
        ORDER_CTRL,
        &[0xB0 | (channel & 0x0F), controller & 0x7F, value],
    )
}

fn tempo_event(tick: u32, bpm: f64) -> Event {
    let bpm = if bpm.is_finite() {
        bpm.clamp(1.0, 1000.0)
    } else {
        120.0
    };
    let micros = (60_000_000.0 / bpm).round().clamp(1.0, 0xFF_FFFF as f64) as u32;
    let bytes = [
        ((micros >> 16) & 0xFF) as u8,
        ((micros >> 8) & 0xFF) as u8,
        (micros & 0xFF) as u8,
    ];
    Event::meta(tick, 0x51, &bytes)
}

fn time_sig_event(tick: u32, num: u8, den: u8) -> Event {
    // 分母是 2 的幂：dd 存的是「2 的几次方」（4/4 → 2，6/8 → 3，3/2 → 1）。
    let dd = if den == 0 {
        2
    } else {
        den.trailing_zeros() as u8
    };
    // cc = 每拍的 MIDI clock 数（24），bb = 每拍的 32 分音符数（8）：DAW 的通用值。
    Event::meta(tick, 0x58, &[num, dd, 24, 8])
}

/// 拍 → tick；负值 / 非有限值夹到 0，超大工程夹到一个安全上界。
fn beat_to_tick(beat: f64) -> u32 {
    if !beat.is_finite() || beat <= 0.0 {
        return 0;
    }
    const MAX_TICK: f64 = 0x0FFF_FFFF as f64;
    (beat * f64::from(PPQ)).round().clamp(0.0, MAX_TICK) as u32
}

fn header(out: &mut Vec<u8>, ntrks: u16) {
    out.extend_from_slice(b"MThd");
    out.extend_from_slice(&6u32.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes()); // format 1：多轨、共享一条时间轴
    out.extend_from_slice(&ntrks.to_be_bytes());
    out.extend_from_slice(&PPQ.to_be_bytes());
}

fn write_track(out: &mut Vec<u8>, mut events: Vec<Event>) {
    events.sort_by_key(|event| (event.tick, event.order));
    let mut body = Vec::new();
    let mut last = 0u32;
    for event in events {
        let tick = event.tick.max(last);
        vlq(tick - last, &mut body);
        body.extend_from_slice(&event.data);
        last = tick;
    }
    // End of Track：与最后一个事件同 tick（delta 0）。
    vlq(0, &mut body);
    body.extend_from_slice(&[0xFF, 0x2F, 0x00]);

    out.extend_from_slice(b"MTrk");
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
}

/// MIDI 的可变长数值（1–4 字节，每字节 7 位、最高位是延续位）。
fn vlq(mut value: u32, out: &mut Vec<u8>) {
    let mut buffer = [0u8; 4];
    let mut len = 0;
    loop {
        buffer[len] = (value & 0x7F) as u8;
        len += 1;
        value >>= 7;
        if value == 0 {
            break;
        }
    }
    for index in (0..len).rev() {
        let mut byte = buffer[index];
        if index > 0 {
            byte |= 0x80;
        }
        out.push(byte);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        AudioSource, AutomationLane, AutomationPoint, Clip, ClipId, MixerChannel, Note,
        PitchExpression, Section, TrackId, Transport,
    };

    /// 解出来的一个事件（只够测试用：跳过 sysex / 其他元事件）。
    #[derive(Debug, Clone, PartialEq)]
    enum Parsed {
        Meta { kind: u8, payload: Vec<u8> },
        Channel { status: u8, data: Vec<u8> },
    }

    fn parse(bytes: &[u8]) -> (u16, Vec<Vec<(u32, Parsed)>>) {
        assert_eq!(&bytes[0..4], b"MThd");
        assert_eq!(
            u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            6
        );
        assert_eq!(u16::from_be_bytes([bytes[8], bytes[9]]), 1, "format 1");
        let ntrks = u16::from_be_bytes([bytes[10], bytes[11]]);
        let division = u16::from_be_bytes([bytes[12], bytes[13]]);
        let mut tracks = Vec::new();
        let mut pos = 14;
        for _ in 0..ntrks {
            assert_eq!(&bytes[pos..pos + 4], b"MTrk");
            let len = u32::from_be_bytes([
                bytes[pos + 4],
                bytes[pos + 5],
                bytes[pos + 6],
                bytes[pos + 7],
            ]) as usize;
            let end = pos + 8 + len;
            let mut cursor = pos + 8;
            let mut tick = 0u32;
            let mut events = Vec::new();
            while cursor < end {
                // delta
                let mut delta = 0u32;
                loop {
                    let byte = bytes[cursor];
                    cursor += 1;
                    delta = (delta << 7) | u32::from(byte & 0x7F);
                    if byte & 0x80 == 0 {
                        break;
                    }
                }
                tick += delta;
                let status = bytes[cursor];
                cursor += 1;
                match status {
                    0xFF => {
                        let kind = bytes[cursor];
                        cursor += 1;
                        let mut size = 0usize;
                        let mut size_bytes = 0;
                        loop {
                            let byte = bytes[cursor];
                            cursor += 1;
                            size = (size << 7) | usize::from(byte & 0x7F);
                            size_bytes += 1;
                            if byte & 0x80 == 0 {
                                break;
                            }
                            assert!(size_bytes < 4);
                        }
                        let payload = bytes[cursor..cursor + size].to_vec();
                        cursor += size;
                        events.push((tick, Parsed::Meta { kind, payload }));
                    }
                    0xF0 => panic!("不写 sysex"),
                    _ => {
                        let count = match status & 0xF0 {
                            0xC0 | 0xD0 => 1,
                            _ => 2,
                        };
                        let data = bytes[cursor..cursor + count].to_vec();
                        cursor += count;
                        events.push((tick, Parsed::Channel { status, data }));
                    }
                }
            }
            tracks.push(events);
            pos = end;
        }
        (division, tracks)
    }

    fn note(pitch: u8, start: f64, duration: f64) -> Note {
        Note {
            id: 1,
            channel: None,
            pressure: None,
            pitch,
            start,
            duration,
            velocity: 100,
            volume: Note::MAX_VOLUME,
            offset: 0.0,
            legato: false,
            expr: PitchExpression::NONE,
            lyric: None,
        }
    }

    fn project_with_track(notes: Vec<Note>) -> Project {
        let mut project = Project::new("demo");
        project.tracks.push(Track {
            id: TrackId(1),
            name: "主旋律".into(),
            instrument: None,
            effects: vec![],
            clips: vec![Clip {
                id: ClipId(7),
                start: 4.0,
                length: 16.0,
                notes,
                audio: None,
            }],
            mixer: MixerChannel::default(),
            sends: vec![],
            automation: vec![],
            meta: None,
        });
        project
    }

    #[test]
    fn writes_format_one_with_a_conductor_track() {
        let mut project = project_with_track(vec![note(60, 0.0, 1.0)]);
        project.transport = Transport {
            tempo: 90.0,
            time_sig_num: 3,
            time_sig_den: 4,
            keyframes: vec![],
        };
        project.sections.push(Section {
            name: "副歌".into(),
            start: 8.0,
            end: 16.0,
            role: None,
        });

        let bytes = project_to_smf(&project, None).unwrap();
        let (division, tracks) = parse(&bytes);
        assert_eq!(division, PPQ);
        assert_eq!(tracks.len(), 2, "指挥轨 + 一条内容轨");

        let meta = |kind: u8| {
            tracks[0]
                .iter()
                .find(|(_, event)| matches!(event, Parsed::Meta { kind: k, .. } if *k == kind))
                .map(|(tick, event)| (*tick, event.clone()))
        };
        assert!(meta(0x03).is_some(), "工程名");
        let (_, tempo) = meta(0x51).expect("tempo");
        let Parsed::Meta { payload, .. } = tempo else {
            unreachable!()
        };
        let micros =
            (u32::from(payload[0]) << 16) | (u32::from(payload[1]) << 8) | u32::from(payload[2]);
        assert_eq!(micros, 666_667, "90 BPM ≈ 666667 µs/拍");
        let (_, signature) = meta(0x58).expect("time sig");
        let Parsed::Meta { payload, .. } = signature else {
            unreachable!()
        };
        assert_eq!(payload[0], 3, "3/4");
        assert_eq!(payload[1], 2, "分母 4 = 2 的 2 次方");
        let (tick, marker) = meta(0x06).expect("marker");
        assert_eq!(tick, 8 * u32::from(PPQ));
        let Parsed::Meta { payload, .. } = marker else {
            unreachable!()
        };
        assert_eq!(payload, "副歌".as_bytes());
    }

    #[test]
    fn notes_land_on_ticks_and_respect_clip_start() {
        let project = project_with_track(vec![note(60, 1.0, 2.0)]);
        let bytes = project_to_smf(&project, None).unwrap();
        let (_, tracks) = parse(&bytes);
        let content = &tracks[1];

        let on = content
            .iter()
            .find(|(_, event)| matches!(event, Parsed::Channel { status: 0x90, .. }))
            .expect("NoteOn");
        // 片段从第 4 拍开始，音符在片段内第 1 拍：绝对第 5 拍。
        assert_eq!(on.0, 5 * u32::from(PPQ));
        let off = content
            .iter()
            .find(|(_, event)| matches!(event, Parsed::Channel { status: 0x80, .. }))
            .expect("NoteOff");
        assert_eq!(off.0, 7 * u32::from(PPQ));
    }

    #[test]
    fn lyrics_and_cc_automation_are_written() {
        let mut song = note(62, 0.0, 1.0);
        song.lyric = Some("春".into());
        let mut project = project_with_track(vec![song]);
        project.tracks[0].automation.push(AutomationLane {
            param: "cc:64".into(),
            points: vec![
                AutomationPoint {
                    beat: 0.0,
                    value: 0.0,
                },
                AutomationPoint {
                    beat: 2.0,
                    value: 127.0,
                },
            ],
        });

        let bytes = project_to_smf(&project, None).unwrap();
        let (_, tracks) = parse(&bytes);
        let content = &tracks[1];
        assert!(
            content.iter().any(|(_, event)| matches!(
                event,
                Parsed::Meta { kind: 0x05, payload } if payload == "春".as_bytes()
            )),
            "歌词元事件"
        );
        let cc: Vec<_> = content
            .iter()
            .filter(|(_, event)| matches!(event, Parsed::Channel { status: 0xB0, .. }))
            .collect();
        assert_eq!(cc.len(), 2, "两个 CC 点");
        assert_eq!(cc[0].0, 0);
        assert_eq!(cc[1].0, 2 * u32::from(PPQ));
        assert_eq!(
            cc[1].1,
            Parsed::Channel {
                status: 0xB0,
                data: vec![64, 127]
            }
        );
    }

    #[test]
    fn drums_use_channel_nine_and_track_filter_works() {
        let mut project = project_with_track(vec![note(60, 0.0, 1.0)]);
        project.tracks[0].meta = Some(crate::model::TrackMeta {
            role: Some("drums".into()),
            ..Default::default()
        });
        let bytes = project_to_smf(&project, None).unwrap();
        let (_, tracks) = parse(&bytes);
        assert!(tracks[1]
            .iter()
            .any(|(_, event)| matches!(event, Parsed::Channel { status: 0x99, .. })));

        // 过滤掉这条轨道 → 没有可导出的音符，报错而不是写空文件。
        let err = project_to_smf(&project, Some(&[99])).unwrap_err();
        assert!(err.contains("没有可导出"), "{err}");
    }

    #[test]
    fn audio_only_tracks_are_skipped() {
        let mut project = Project::new("audio");
        project.tracks.push(Track {
            id: TrackId(1),
            name: "人声".into(),
            instrument: None,
            effects: vec![],
            clips: vec![Clip {
                id: ClipId(1),
                start: 0.0,
                length: 4.0,
                notes: vec![],
                audio: Some(AudioSource {
                    path: "x.wav".into(),
                    offset_frames: 0,
                    frames: None,
                    rate: 1.0,
                    pitch_semitones: 0.0,
                }),
            }],
            mixer: MixerChannel::default(),
            sends: vec![],
            automation: vec![],
            meta: None,
        });
        assert!(project_to_smf(&project, None).is_err());
    }
}
