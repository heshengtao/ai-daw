use daw_party_plugin_api::ProcessContext;

use crate::audio::AudioBuffer;
use crate::beats_to_seconds;
use crate::model::Project;
use crate::registry::PluginRegistry;
use crate::sequencer::collect_track_events;
use crate::BLOCK_SIZE;

/// 离线渲染选项。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderOptions {
    /// 采样率（Hz）。
    pub sample_rate: f64,
    /// 工程内容结束后的额外渲染时长（秒）。
    pub tail_seconds: f64,
}

/// dB → 线性增益。
fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// 把工程离线渲染为立体声交错缓冲。
///
/// 逐块渲染每条未静音且已设置乐器的轨道：轨道增益（dB → 线性）与声像按等功率
/// 分配到左右声道，块内的 MIDI 事件由 `collect_track_events` 收集。输出长度 =
/// （工程内容时长 + `tail_seconds`）× 采样率。相同工程与选项必然产出逐位相同的结果。
pub fn render(project: &Project, registry: &PluginRegistry, options: &RenderOptions) -> AudioBuffer {
    let sr = options.sample_rate;
    let tempo = project.transport.tempo;
    let body_sec = beats_to_seconds(project.length_beats(), tempo);
    let total_frames = ((body_sec + options.tail_seconds) * sr) as usize;

    let mut master = AudioBuffer::silence(2, total_frames);

    for track in &project.tracks {
        if track.mixer.mute {
            continue;
        }
        let Some(instance) = &track.instrument else { continue };
        let Some(mut plugin) = registry.create(&instance.plugin_id) else { continue };
        plugin.prepare(sr, BLOCK_SIZE);
        for (id, value) in &instance.params {
            plugin.set_param(&id.clone().into(), *value);
        }

        let gain = db_to_linear(track.mixer.gain_db);
        let pan = track.mixer.pan.clamp(-1.0, 1.0);
        let left_gain = gain * ((1.0 - pan) * 0.5).sqrt();
        let right_gain = gain * ((1.0 + pan) * 0.5).sqrt();

        let mut block_start = 0usize;
        while block_start < total_frames {
            let frames = BLOCK_SIZE.min(total_frames - block_start);
            let block_start_beats = (block_start as f64 / sr) * tempo / 60.0;
            let events = collect_track_events(track, block_start_beats, frames, sr, tempo);

            let mut left = vec![0.0f32; frames];
            let mut right = vec![0.0f32; frames];
            let ctx = ProcessContext {
                sample_rate: sr,
                block_size: frames,
                tempo,
                playing: true,
            };
            plugin.process(&ctx, &mut [&mut left, &mut right], &events);

            for i in 0..frames {
                master.set_sample(0, block_start + i, master.sample(0, block_start + i) + left[i] * left_gain);
                master.set_sample(1, block_start + i, master.sample(1, block_start + i) + right[i] * right_gain);
            }
            block_start += frames;
        }
    }

    master
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Engine;
    use crate::command::Command;
    use crate::event::{Actor, Event};
    use crate::model::Note;
    use crate::registry::PluginRegistry;

    fn demo() -> crate::model::Project {
        let mut e = Engine::new("demo");
        let ev = e.execute(Command::AddTrack { name: "Lead".into() }, Actor::Human);
        let track = match ev[0] { Event::TrackAdded { id, .. } => id, _ => unreachable!() };
        e.execute(Command::SetTrackInstrument { track, plugin_id: "builtin.synth".into() }, Actor::Human);
        let ev = e.execute(Command::AddClip { track, start: 0.0, length: 2.0 }, Actor::Human);
        let clip = match ev[0] { Event::ClipAdded { clip, .. } => clip, _ => unreachable!() };
        e.execute(
            Command::AddNote { track, clip, note: Note { pitch: 69, start: 0.0, duration: 1.0, velocity: 100 } },
            Actor::Human,
        );
        e.project().clone()
    }

    #[test]
    fn render_duration_matches_project_length_plus_tail() {
        let project = demo();
        let opts = RenderOptions { sample_rate: 48_000.0, tail_seconds: 0.5 };
        let buf = render(&project, &PluginRegistry::with_builtins(), &opts);
        let expected = (beats_to_seconds(project.length_beats(), project.transport.tempo) * 48_000.0) as usize
            + (0.5 * 48_000.0) as usize;
        assert!((buf.frames() as i64 - expected as i64).abs() <= 1);
        assert_eq!(buf.channels(), 2);
    }

    #[test]
    fn render_is_deterministic_and_audible() {
        let project = demo();
        let opts = RenderOptions { sample_rate: 48_000.0, tail_seconds: 0.2 };
        let r = PluginRegistry::with_builtins();
        let a = render(&project, &r, &opts);
        let b = render(&project, &r, &opts);
        assert_eq!(a.data(), b.data(), "render must be bit-identical");
        assert!(a.peak() > 0.05, "should be audible");
        assert!(a.data().iter().all(|s| s.is_finite()));
    }
}
