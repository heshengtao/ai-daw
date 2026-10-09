use std::sync::Arc;

use daw_party_plugin_api::MidiEvent;

use crate::audio::{DecodedAudio, SampleProvider};
use crate::model::{MixerChannel, Note, PluginInstance, Project, TrackId};
use crate::tempo::TempoMap;

/// 一条已定绝对帧的 MIDI 事件。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScheduledMidi {
    /// 自播放起点起的绝对帧位置。
    pub frame: u64,
    /// 该帧时刻发出的 MIDI 事件。
    pub event: MidiEvent,
}

/// 一条已定绝对帧、且已接上素材的音频片段。
///
/// 语义（见 `specs/2026-10-09-audio-clip-design.md`）：片段的 `length`（拍）决定时间轴跨度，
/// 素材**铺满**该跨度（`ratio = source_window_frames / span_frames`）——缩放或改速度会按比例
/// 重采样，因此会变调（v1 不做保音高变速）。
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledAudioClip {
    /// 片段起始的绝对帧（含）。
    pub start_frame: u64,
    /// 片段结束的绝对帧（不含）。
    pub end_frame: u64,
    /// 素材内的起始帧（crop 起点，已按素材长度夹紧）。
    pub offset_frames: u64,
    /// 本次要铺开的源帧数（= 素材总帧数 − `offset_frames`）。
    pub source_window_frames: u64,
    /// 已解码素材（构造播放器前加载完毕，播放路径只读）。
    pub audio: Arc<DecodedAudio>,
}

/// 单轨的定帧事件表与播放所需信息（自带插件引用与混音参数，使 `Player` 不依赖 `Project`）。
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledTrack {
    /// 轨道标识。
    pub track_id: TrackId,
    /// 轨道使用的乐器插件实例（无则静音/留空）。
    pub instrument: Option<PluginInstance>,
    /// 轨道效果链（按顺序处理，接在乐器之后）。
    pub effects: Vec<PluginInstance>,
    /// 轨道混音参数。
    pub mixer: MixerChannel,
    /// 按帧稳定排序的事件列表。
    pub events: Vec<ScheduledMidi>,
    /// 音频片段（可能为空）。素材缺失的片段不会出现在这里（按静音处理）。
    pub audio_clips: Vec<ScheduledAudioClip>,
}

/// 整首工程的定帧事件表：离线渲染与实时播放共用。
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledEvents {
    /// 各轨道的定帧事件表。
    pub tracks: Vec<ScheduledTrack>,
    /// 总输出通道（增益在播放器里作用于各轨混合之后）。
    pub master: MixerChannel,
    /// 工程内容总长度（帧），取所有片段结束位置的最大值。
    pub end_frame: u64,
    /// 构建时使用的速度，供实时播放按拍推进时复用。
    pub tempo: f64,
    /// 速度分段在时间轴上的落点（帧 → BPM，按帧升序，首项为 `(0, tempo_at(0))`）。
    /// 供实时播放逐块取当前速度。
    pub tempo_points: Vec<(u64, f64)>,
    /// 构建该事件表时使用的采样率（Hz）；所有 `frame` 均以此刻度定点。宿主应在
    /// 采样率不一致时拒绝播放，否则事件会被静默错误定时。
    pub sample_rate: f64,
}

impl ScheduledEvents {
    /// 从工程构建定帧事件表。事件按帧稳定排序；同帧时保持（clip 顺序 → note 顺序 → NoteOn 先于 NoteOff）的原有次序，以保证与 Plan 1 的渲染一致。
    ///
    /// **不含音频素材**：工程里的音频片段一律按静音处理。要真正出声请用
    /// [`ScheduledEvents::from_project_with_assets`]。
    pub fn from_project(project: &Project, sample_rate: f64) -> Self {
        Self::from_project_with_assets(project, sample_rate, &crate::audio::no_assets())
    }

    /// 同 [`ScheduledEvents::from_project`]，但把音频片段接到真实素材上。
    ///
    /// 音频片段的绝对帧用与 MIDI **同一个** `to_frame` 闭包算出，保证两者严格对齐。
    /// 素材缺失（`assets.get` 返回 `None`）或窗口为空时该片段被跳过——与「缺失插件用静音占位」
    /// 同一策略：绝不让一段坏素材拖垮整首曲子。
    pub fn from_project_with_assets(
        project: &Project,
        sample_rate: f64,
        assets: &dyn SampleProvider,
    ) -> Self {
        let map = TempoMap::from_transport(&project.transport);
        let to_frame = |beats: f64| (map.beats_to_seconds(beats) * sample_rate) as u64;
        let mut tempo_points: Vec<(u64, f64)> = vec![(0, map.tempo_at(0.0))];
        for s in map.segments().iter().skip(1) {
            tempo_points.push((to_frame(s.start_beat), s.tempo));
        }

        let mut tracks = Vec::with_capacity(project.tracks.len());
        for track in &project.tracks {
            let mut events = Vec::new();
            let mut audio_clips = Vec::new();
            for clip in &track.clips {
                // 音频片段：接素材、定绝对帧。素材缺失 → 整段静音（跳过）。
                if let Some(source) = &clip.audio {
                    let Some(audio) = assets.get(&source.path) else { continue };
                    let total = audio.frames() as u64;
                    let offset = source.offset_frames.min(total);
                    let window = total - offset;
                    if window == 0 {
                        continue;
                    }
                    let start_frame = to_frame(clip.start);
                    let end_frame = to_frame(clip.start + clip.length);
                    if end_frame <= start_frame {
                        continue;
                    }
                    audio_clips.push(ScheduledAudioClip {
                        start_frame,
                        end_frame,
                        offset_frames: offset,
                        source_window_frames: window,
                        audio,
                    });
                    continue;
                }
                for note in &clip.notes {
                    let on = clip.start + note.start;
                    let off = on + note.duration;
                    events.push(ScheduledMidi {
                        frame: to_frame(on),
                        // 只送「演奏强度」。插件各自解释它（音色 / 振幅）——这是设计如此。
                        event: MidiEvent::NoteOn { note: note.pitch, velocity: note.velocity },
                    });
                    // 单音音量与力度**分开发**：支持 per-note 音量的插件独立处理它，不改变音色。
                    // 默认值 127（不衰减）不需要事件。
                    if note.volume != Note::MAX_VOLUME {
                        events.push(ScheduledMidi {
                            frame: to_frame(on),
                            event: MidiEvent::NoteVolume { note: note.pitch, value: note.volume },
                        });
                    }
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
                effects: track.effects.clone(),
                mixer: track.mixer,
                events,
                audio_clips,
            });
        }

        ScheduledEvents {
            tracks,
            master: project.master,
            end_frame: to_frame(project.length_beats()),
            tempo: map.tempo_at(0.0),
            tempo_points,
            sample_rate,
        }
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
            effects: vec![],
            clips: vec![Clip {
                id: ClipId(1),
                start: 0.0,
                length: 4.0,
                notes: vec![Note { pitch: 60, start: 1.0, duration: 1.0, velocity: 100, volume: 127 }],
                audio: None,
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

    #[test]
    fn carries_master_channel() {
        let mut p = project_with_note();
        p.master.gain_db = -4.5;
        p.master.pan = 0.25;
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        assert_eq!(s.master.gain_db, -4.5);
        assert_eq!(s.master.pan, 0.25);
    }

    #[test]
    fn records_the_build_sample_rate() {
        let s = ScheduledEvents::from_project(&project_with_note(), 44_100.0);
        assert_eq!(s.sample_rate, 44_100.0);
    }

    #[test]
    fn note_volume_travels_as_its_own_event() {
        let mut p = project_with_note();
        // 音量 64：力度保持原值，另发一条 NoteVolume。
        p.tracks[0].clips[0].notes[0].volume = 64;
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        assert!(matches!(
            s.tracks[0].events[0].event,
            MidiEvent::NoteOn { note: 60, velocity: 100 }
        ));
        assert!(matches!(
            s.tracks[0].events[1].event,
            MidiEvent::NoteVolume { note: 60, value: 64 }
        ));

        // 音量归零：仍要送达（插件据此把该音压到静音）。
        p.tracks[0].clips[0].notes[0].volume = 0;
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        assert!(matches!(
            s.tracks[0].events[1].event,
            MidiEvent::NoteVolume { note: 60, value: 0 }
        ));
    }

    #[test]
    fn keyframe_shifts_later_event_frames() {
        use crate::model::{Clip, ClipId, Keyframe, Note, Track, TrackId};
        let mut p = Project::new("p");
        p.transport.tempo = 120.0;
        p.transport.upsert_keyframe(Keyframe { beat: 2.0, tempo: 240.0, time_sig_num: 4, time_sig_den: 4 });
        p.tracks.push(Track {
            id: TrackId(1),
            name: "L".into(),
            instrument: None,
            effects: vec![],
            clips: vec![Clip {
                id: ClipId(1),
                start: 0.0,
                length: 4.0,
                notes: vec![Note { pitch: 60, start: 3.0, duration: 0.5, velocity: 100, volume: 127 }],
                audio: None,
            }],
            mixer: Default::default(),
        });
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        // 3 拍 = 0..2 @120 (1s) + 2..3 @240 (0.25s) = 1.25s = 60000 帧。
        assert_eq!(s.tracks[0].events[0].frame, 60_000);
        // tempo_points: 帧 0 → 120；帧 48000（2 拍 @120 = 1s）→ 240。
        assert_eq!(s.tempo_points[0], (0, 120.0));
        assert_eq!(s.tempo_points[1], (48_000, 240.0));
    }
}
