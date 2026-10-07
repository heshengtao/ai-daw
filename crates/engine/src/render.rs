use crate::audio::AudioBuffer;
use crate::beats_to_seconds;
use crate::model::Project;
use crate::player::Player;
use crate::registry::PluginRegistry;
use crate::schedule::ScheduledEvents;
use crate::transport::TransportCommand;
use crate::BLOCK_SIZE;

/// 离线渲染选项。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderOptions {
    /// 采样率（Hz）。
    pub sample_rate: f64,
    /// 工程内容结束后的额外渲染时长（秒）。
    pub tail_seconds: f64,
}

/// 把工程离线渲染为立体声交错缓冲。
///
/// 与实时播放共用同一条代码路径：先把工程构建为 [`ScheduledEvents`]，再交给
/// [`Player`] 逐块推进，输出因此与实时播放逐位一致（块大小无关）。输出长度 =
/// （工程内容时长 + `tail_seconds`）× 采样率。相同工程与选项必然产出逐位相同的结果。
pub fn render(project: &Project, registry: &PluginRegistry, options: &RenderOptions) -> AudioBuffer {
    let sr = options.sample_rate;
    let tempo = project.transport.tempo;
    let body_sec = beats_to_seconds(project.length_beats(), tempo);
    let total_frames = ((body_sec + options.tail_seconds) * sr) as usize;

    let schedule = ScheduledEvents::from_project(project, sr);
    let mut player = Player::new(schedule, registry, sr, BLOCK_SIZE);
    player.apply_command(TransportCommand::Play);

    let mut buffer = AudioBuffer::silence(2, total_frames);
    let data = buffer.data_mut();
    let mut block_start = 0usize;
    while block_start < total_frames {
        let frames = BLOCK_SIZE.min(total_frames - block_start);
        let start = block_start * 2;
        let end = start + frames * 2;
        player.process_block(&mut data[start..end], frames);
        block_start += frames;
    }
    buffer
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

    #[test]
    fn render_matches_player_at_different_block_size() {
        let project = demo();
        let opts = RenderOptions { sample_rate: 48_000.0, tail_seconds: 0.2 };
        let registry = PluginRegistry::with_builtins();
        let buffer = render(&project, &registry, &opts);

        let schedule = crate::schedule::ScheduledEvents::from_project(&project, opts.sample_rate);
        let mut player = crate::player::Player::new(schedule, &registry, opts.sample_rate, 4096);
        player.apply_command(crate::transport::TransportCommand::Play);

        let total = buffer.frames();
        let mut out = vec![0.0f32; total * 2];
        let mut off = 0usize;
        while off < total {
            let frames = 128usize.min(total - off);
            player.process_block(&mut out[off * 2..(off + frames) * 2], frames);
            off += frames;
        }
        assert_eq!(buffer.data(), out.as_slice(), "offline render must equal realtime Player output");
    }
}
