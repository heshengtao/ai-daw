use daw_party_plugin_api::MidiEvent;

use crate::beats_to_seconds;
use crate::model::{MixerChannel, PluginInstance, Project, TrackId};

/// 一条已定绝对帧的 MIDI 事件。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScheduledMidi {
    /// 自播放起点起的绝对帧位置。
    pub frame: u64,
    /// 该帧时刻发出的 MIDI 事件。
    pub event: MidiEvent,
}

/// 单轨的定帧事件表与播放所需信息（自带插件引用与混音参数，使 `Player` 不依赖 `Project`）。
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledTrack {
    /// 轨道标识。
    pub track_id: TrackId,
    /// 轨道使用的乐器插件实例（无则静音/留空）。
    pub instrument: Option<PluginInstance>,
    /// 轨道混音参数。
    pub mixer: MixerChannel,
    /// 按帧稳定排序的事件列表。
    pub events: Vec<ScheduledMidi>,
}

/// 整首工程的定帧事件表：离线渲染与实时播放共用。
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledEvents {
    /// 各轨道的定帧事件表。
    pub tracks: Vec<ScheduledTrack>,
    /// 工程内容总长度（帧），取所有片段结束位置的最大值。
    pub end_frame: u64,
    /// 构建时使用的速度，供实时播放按拍推进时复用。
    pub tempo: f64,
}

impl ScheduledEvents {
    /// 从工程构建定帧事件表。事件按帧稳定排序；同帧时保持（clip 顺序 → note 顺序 → NoteOn 先于 NoteOff）的原有次序，以保证与 Plan 1 的渲染一致。
    pub fn from_project(project: &Project, sample_rate: f64) -> Self {
        let tempo = project.transport.tempo;
        let to_frame = |beats: f64| (beats_to_seconds(beats, tempo) * sample_rate) as u64;

        let mut tracks = Vec::with_capacity(project.tracks.len());
        for track in &project.tracks {
            let mut events = Vec::new();
            for clip in &track.clips {
                for note in &clip.notes {
                    let on = clip.start + note.start;
                    let off = on + note.duration;
                    events.push(ScheduledMidi {
                        frame: to_frame(on),
                        event: MidiEvent::NoteOn { note: note.pitch, velocity: note.velocity },
                    });
                    events.push(ScheduledMidi {
                        frame: to_frame(off),
                        event: MidiEvent::NoteOff { note: note.pitch },
                    });
                }
            }
            events.sort_by_key(|e| e.frame);
            tracks.push(ScheduledTrack {
                track_id: track.id,
                instrument: track.instrument.clone(),
                mixer: track.mixer,
                events,
            });
        }

        ScheduledEvents { tracks, end_frame: to_frame(project.length_beats()), tempo }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipId, Note, Track, TrackId};

    fn project_with_note() -> Project {
        let mut p = Project::new("p");
        p.transport.tempo = 120.0;
        p.tracks.push(Track {
            id: TrackId(1),
            name: "Lead".into(),
            instrument: None,
            clips: vec![Clip {
                id: ClipId(1),
                start: 0.0,
                length: 4.0,
                notes: vec![Note { pitch: 60, start: 1.0, duration: 1.0, velocity: 100 }],
            }],
            mixer: Default::default(),
        });
        p
    }

    #[test]
    fn builds_sorted_absolute_frame_events() {
        let s = ScheduledEvents::from_project(&project_with_note(), 48_000.0);
        assert_eq!(s.tracks.len(), 1);
        let ev = &s.tracks[0].events;
        assert_eq!(ev.len(), 2);
        // 120bpm, 48kHz: 1 beat = 0.5s = 24000 frames.
        assert_eq!(ev[0].frame, 24_000);
        assert!(matches!(ev[0].event, MidiEvent::NoteOn { note: 60, velocity: 100 }));
        assert_eq!(ev[1].frame, 48_000);
        assert!(matches!(ev[1].event, MidiEvent::NoteOff { note: 60 }));
    }

    #[test]
    fn end_frame_is_project_body_length() {
        let s = ScheduledEvents::from_project(&project_with_note(), 48_000.0);
        // clip end = 4 beats = 2.0 s = 96000 frames.
        assert_eq!(s.end_frame, 96_000);
    }

    #[test]
    fn carries_instrument_and_mixer() {
        let s = ScheduledEvents::from_project(&project_with_note(), 48_000.0);
        assert!(s.tracks[0].instrument.is_none());
        assert_eq!(s.tracks[0].mixer.gain_db, 0.0);
    }
}
