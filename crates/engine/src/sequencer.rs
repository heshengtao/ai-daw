use daw_party_plugin_api::{MidiEvent, TimedMidiEvent};

use crate::beats_to_seconds;
use crate::model::Track;

/// 收集某轨道在给定块内应发出的 MIDI 事件（带块内帧偏移）。
/// `block_start_beats` 为块起点的全局拍位置；`block_frames` 为块帧数。
///
/// 块窗口为半开区间 `[block_start, block_end)`：恰好落在 `block_end` 上的音符边沿
/// 属于下一块，会在下一块的帧 0 发出（不会重复发声，也不会丢事件）。因此本块内产出
/// 的每个事件帧偏移恒定满足 `frame < block_frames`，可直接用于按帧索引的消费者
/// （如宿主 / VST3 host）。
pub fn collect_track_events(
    track: &Track,
    block_start_beats: f64,
    block_frames: usize,
    sample_rate: f64,
    tempo: f64,
) -> Vec<TimedMidiEvent> {
    let block_start_sec = beats_to_seconds(block_start_beats, tempo);
    let block_end_sec = block_start_sec + block_frames as f64 / sample_rate;
    let mut out = Vec::new();

    for clip in &track.clips {
        for note in &clip.notes {
            let start_beat = clip.start + note.start;
            let end_beat = start_beat + note.duration;
            let start_sec = beats_to_seconds(start_beat, tempo);
            let end_sec = beats_to_seconds(end_beat, tempo);

            if start_sec >= block_start_sec && start_sec < block_end_sec {
                let frame = ((start_sec - block_start_sec) * sample_rate).round() as u32;
                out.push(TimedMidiEvent {
                    frame,
                    event: MidiEvent::NoteOn { note: note.pitch, velocity: note.velocity },
                });
            }
            if end_sec >= block_start_sec && end_sec < block_end_sec {
                let frame = ((end_sec - block_start_sec) * sample_rate).round() as u32;
                out.push(TimedMidiEvent { frame, event: MidiEvent::NoteOff { note: note.pitch } });
            }
        }
    }

    out.sort_by_key(|e| e.frame);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipId, Note, Track, TrackId};
    use daw_party_plugin_api::MidiEvent;

    /// Single one-beat-positioned note (`start = 1.0` beats) with a configurable duration.
    fn track_with_note_duration(duration: f64) -> Track {
        Track {
            id: TrackId(1),
            name: "t".into(),
            instrument: None,
            clips: vec![Clip {
                id: ClipId(1),
                start: 0.0,
                length: 4.0,
                notes: vec![Note { pitch: 60, start: 1.0, duration, velocity: 100 }],
            }],
            mixer: Default::default(),
        }
    }

    fn track_with_note() -> Track {
        track_with_note_duration(1.0)
    }

    #[test]
    fn note_on_and_off_land_in_correct_block() {
        // Note start beat 1.0 and end beat 1.5 both fall strictly inside the block.
        let t = track_with_note_duration(0.5);
        // 120bpm, 48kHz: 1 beat = 0.5s = 24000 frames.
        let events = collect_track_events(&t, 0.0, 48_000, 48_000.0, 120.0);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].frame, 24_000);
        assert!(matches!(events[0].event, MidiEvent::NoteOn { note: 60, velocity: 100 }));
        assert_eq!(events[1].frame, 36_000);
        assert!(matches!(events[1].event, MidiEvent::NoteOff { note: 60 }));
        // Half-open window `[block_start, block_end)`: frames are always `< block_frames`.
        assert!(events.iter().all(|e| e.frame < 48_000));
    }

    #[test]
    fn note_off_at_block_boundary_lands_in_next_block() {
        // Note ends at beat 2.0 = 1.0s = the exact end of block N (0.0..1.0s).
        let t = track_with_note();
        // Query the NEXT block: no `frame == block_frames` out-of-range copy is produced,
        // and the boundary NoteOff is closed at frame 0 of the following block instead.
        let events = collect_track_events(&t, 2.0, 48_000, 48_000.0, 120.0);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].frame, 0);
        assert!(matches!(events[0].event, MidiEvent::NoteOff { note: 60 }));
    }

    #[test]
    fn notes_outside_block_are_excluded() {
        let t = track_with_note();
        let events = collect_track_events(&t, 0.25, 1_000, 48_000.0, 120.0);
        assert!(events.is_empty());
    }
}
