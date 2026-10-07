use daw_party_plugin_api::{Plugin, ProcessContext, TimedMidiEvent};

use crate::registry::PluginRegistry;
use crate::schedule::{ScheduledEvents, ScheduledMidi};
use crate::transport::TransportCommand;

/// dB → 线性增益。
pub(crate) fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

struct TrackVoice {
    plugin: Box<dyn Plugin>,
    events: Vec<ScheduledMidi>,
    cursor: usize,
    left_gain: f32,
    right_gain: f32,
    muted: bool,
}

/// 实时安全的逐块播放器：构造后 `process_block` 不再分配内存。
pub struct Player {
    tracks: Vec<TrackVoice>,
    sample_rate: f64,
    tempo: f64,
    position: u64,
    end_frame: u64,
    playing: bool,
    scratch_left: Vec<f32>,
    scratch_right: Vec<f32>,
    scratch_events: Vec<TimedMidiEvent>,
}

impl Player {
    /// `max_block` 为回调可能请求的最大帧数；所有 scratch 缓冲按此预留，避免回调内分配。
    pub fn new(
        schedule: ScheduledEvents,
        registry: &PluginRegistry,
        sample_rate: f64,
        max_block: usize,
    ) -> Self {
        let tempo = schedule.tempo;
        let end_frame = schedule.end_frame;
        let mut tracks = Vec::with_capacity(schedule.tracks.len());
        for st in &schedule.tracks {
            let Some(instance) = &st.instrument else { continue };
            let Some(mut plugin) = registry.create(&instance.plugin_id) else { continue };
            if st.mixer.mute {
                continue;
            }
            plugin.prepare(sample_rate, max_block);
            for (id, value) in &instance.params {
                plugin.set_param(&id.clone().into(), *value);
            }
            let gain = db_to_linear(st.mixer.gain_db);
            let pan = st.mixer.pan.clamp(-1.0, 1.0);
            tracks.push(TrackVoice {
                plugin,
                events: st.events.clone(),
                cursor: 0,
                left_gain: gain * ((1.0 - pan) * 0.5).sqrt(),
                right_gain: gain * ((1.0 + pan) * 0.5).sqrt(),
                muted: st.mixer.mute,
            });
        }
        Player {
            tracks,
            sample_rate,
            tempo,
            position: 0,
            end_frame,
            playing: false,
            scratch_left: vec![0.0; max_block],
            scratch_right: vec![0.0; max_block],
            scratch_events: Vec::with_capacity(max_block.max(64)),
        }
    }

    /// 当前是否正在播放。
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// 当前位置（自播放起点起的帧数）。
    pub fn position_frames(&self) -> u64 {
        self.position
    }

    /// 定位到指定帧：重置所有插件并重算事件游标（已在播放中的音符不会被重新触发）。
    pub fn seek(&mut self, frame: u64) {
        self.position = frame;
        for track in self.tracks.iter_mut() {
            track.cursor = track.events.partition_point(|e| e.frame < frame);
            track.plugin.reset();
        }
    }

    /// 应用一条传输命令。
    pub fn apply_command(&mut self, command: TransportCommand) {
        match command {
            TransportCommand::Play => self.playing = true,
            TransportCommand::Pause => self.playing = false,
            TransportCommand::Stop => {
                self.playing = false;
                self.seek(0);
            }
            TransportCommand::Seek { frame } => self.seek(frame),
        }
    }

    /// 填充一个交错立体声块（`out.len() >= frames * 2`）。实时安全：不分配内存。
    pub fn process_block(&mut self, out: &mut [f32], frames: usize) {
        debug_assert!(out.len() >= frames * 2);
        let out = &mut out[..frames * 2];
        out.fill(0.0);
        if !self.playing {
            return;
        }

        let block_start = self.position;
        let block_end = block_start + frames as u64;
        let ctx = ProcessContext {
            sample_rate: self.sample_rate,
            block_size: frames,
            tempo: self.tempo,
            playing: true,
        };

        let Player { tracks, scratch_left, scratch_right, scratch_events, .. } = self;

        for track in tracks.iter_mut() {
            if track.muted {
                continue;
            }
            scratch_events.clear();
            while let Some(ev) = track.events.get(track.cursor).copied() {
                if ev.frame >= block_end {
                    break;
                }
                if ev.frame >= block_start {
                    scratch_events.push(TimedMidiEvent {
                        frame: (ev.frame - block_start) as u32,
                        event: ev.event,
                    });
                }
                track.cursor += 1;
            }

            if scratch_left.len() < frames {
                scratch_left.resize(frames, 0.0);
            }
            if scratch_right.len() < frames {
                scratch_right.resize(frames, 0.0);
            }
            let left = &mut scratch_left[..frames];
            let right = &mut scratch_right[..frames];
            left.fill(0.0);
            right.fill(0.0);

            track.plugin.process(&ctx, &mut [left, right], scratch_events);

            for i in 0..frames {
                out[i * 2] += left[i] * track.left_gain;
                out[i * 2 + 1] += right[i] * track.right_gain;
            }
        }

        self.position = block_end;
        if self.end_frame > 0 && self.position >= self.end_frame {
            self.position = self.end_frame;
            self.playing = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipId, Note, PluginInstance, Track, TrackId};
    use crate::registry::PluginRegistry;
    use crate::schedule::ScheduledEvents;
    use crate::transport::TransportCommand;
    use crate::model::Project;

    fn schedule() -> ScheduledEvents {
        let mut p = Project::new("p");
        p.transport.tempo = 120.0;
        p.tracks.push(Track {
            id: TrackId(1),
            name: "Lead".into(),
            instrument: Some(PluginInstance { plugin_id: "builtin.synth".into(), params: Default::default(), state: vec![] }),
            clips: vec![Clip {
                id: ClipId(1),
                start: 0.0,
                length: 1.0,
                notes: vec![Note { pitch: 69, start: 0.0, duration: 0.5, velocity: 100 }],
            }],
            mixer: Default::default(),
        });
        ScheduledEvents::from_project(&p, 48_000.0)
    }

    #[test]
    fn silent_until_play() {
        let mut player = Player::new(schedule(), &PluginRegistry::with_builtins(), 48_000.0, 512);
        let mut out = vec![1.0f32; 512 * 2];
        player.process_block(&mut out, 512);
        assert!(out.iter().all(|s| *s == 0.0));
        assert!(!player.is_playing());
    }

    #[test]
    fn plays_and_advances_position() {
        let mut player = Player::new(schedule(), &PluginRegistry::with_builtins(), 48_000.0, 512);
        player.apply_command(TransportCommand::Play);
        let mut out = vec![0.0f32; 512 * 2];
        player.process_block(&mut out, 512);
        assert!(out.iter().any(|s| s.abs() > 0.01), "should be audible");
        assert_eq!(player.position_frames(), 512);
    }

    #[test]
    fn stop_pauses_and_rewinds() {
        let mut player = Player::new(schedule(), &PluginRegistry::with_builtins(), 48_000.0, 512);
        player.apply_command(TransportCommand::Play);
        let mut out = vec![0.0f32; 512 * 2];
        player.process_block(&mut out, 512);
        player.apply_command(TransportCommand::Stop);
        assert!(!player.is_playing());
        assert_eq!(player.position_frames(), 0);
    }

    #[test]
    fn seek_moves_cursor_without_playing_early_notes() {
        let mut player = Player::new(schedule(), &PluginRegistry::with_builtins(), 48_000.0, 512);
        // Note ends at beat 0.5 = 0.25s = 12000 frames; seek past it -> silence.
        player.apply_command(TransportCommand::Seek { frame: 20_000 });
        player.apply_command(TransportCommand::Play);
        let mut out = vec![0.0f32; 512 * 2];
        player.process_block(&mut out, 512);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(player.position_frames(), 20_512);
    }
}
