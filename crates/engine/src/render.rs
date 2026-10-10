use crate::audio::{AudioBuffer, SampleProvider};
use crate::model::Project;
use crate::player::Player;
use crate::registry::PluginRegistry;
use crate::schedule::ScheduledEvents;
use crate::tempo::TempoMap;
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
    render_with_assets(project, registry, options, &crate::audio::no_assets())
}

/// 同 [`render`]，但把音频片段接到真实素材上（缺失素材的片段按静音处理）。
pub fn render_with_assets(
    project: &Project,
    registry: &PluginRegistry,
    options: &RenderOptions,
    assets: &dyn SampleProvider,
) -> AudioBuffer {
    let sr = options.sample_rate;
    let map = TempoMap::from_transport(&project.transport);
    let body_sec = map.beats_to_seconds(project.length_beats());
    let total_frames = ((body_sec + options.tail_seconds) * sr) as usize;

    let schedule = ScheduledEvents::from_project_with_assets(project, sr, assets);
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
            Command::AddNote { track, clip, note: Note { pitch: 69, start: 0.0, duration: 1.0, velocity: 100, volume: 127 } },
            Actor::Human,
        );
        e.project().clone()
    }

    #[test]
    fn render_duration_matches_project_length_plus_tail() {
        let project = demo();
        let opts = RenderOptions { sample_rate: 48_000.0, tail_seconds: 0.5 };
        let buf = render(&project, &PluginRegistry::with_builtins(), &opts);
        let expected = (TempoMap::from_transport(&project.transport).beats_to_seconds(project.length_beats())
            * 48_000.0) as usize
            + (0.5 * 48_000.0) as usize;
        assert!((buf.frames() as i64 - expected as i64).abs() <= 1);
        assert_eq!(buf.channels(), 2);
    }

    /// 无关键点时，渲染结果必须与本计划之前逐位一致（回归由现有用例覆盖）。
    /// 有关键点时，时长按分段计算。
    #[test]
    fn render_length_follows_tempo_keyframes() {
        use crate::model::Keyframe;
        let mut project = demo();
        project.transport.upsert_keyframe(Keyframe { beat: 1.0, tempo: 240.0, time_sig_num: 4, time_sig_den: 4 });
        let map = crate::tempo::TempoMap::from_transport(&project.transport);
        let expected_body = map.beats_to_seconds(project.length_beats());
        let opts = RenderOptions { sample_rate: 48_000.0, tail_seconds: 0.1 };
        let buf = render(&project, &PluginRegistry::with_builtins(), &opts);
        let expected = (expected_body * 48_000.0) as usize + (0.1 * 48_000.0) as usize;
        assert!((buf.frames() as i64 - expected as i64).abs() <= 1);
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

    /// 回归测试（Plan 2 审查项 1）：正文很短、音符在正文末尾附近结束时，`render`
    /// 必须继续把音符释放/衰减渲染进 `tail_seconds`。早期 `Player::process_block`
    /// 会在到达 `end_frame` 时自动停止（`playing = false`），把尾音截断，导致每次
    /// 导出结尾出现爆音；离线 `render` 现在循环调用 `process_block`，必须越过正文继续渲染。
    #[test]
    fn render_keeps_note_release_into_tail_after_body_end() {
        use crate::model::{Clip, ClipId, PluginInstance, Track, TrackId};

        // 长释放，确保尾音一直延续到缓冲末尾的「最后 0.1 s」窗口。
        let mut params = std::collections::BTreeMap::new();
        params.insert("release".to_string(), 1.0f32);

        let mut p = Project::new("short");
        p.transport.tempo = 120.0;
        p.tracks.push(Track {
            id: TrackId(1),
            name: "Lead".into(),
            instrument: Some(PluginInstance {
                plugin_id: "builtin.synth".into(),
                params,
                state: vec![],
            }),
            effects: vec![],
            clips: vec![Clip {
                id: ClipId(1),
                start: 0.0,
                length: 0.05,
                notes: vec![Note { pitch: 69, start: 0.0, duration: 0.05, velocity: 100, volume: 127 }],
                audio: None,
            }],
            mixer: Default::default(),
            sends: vec![],
            automation: vec![],
        });

        let opts = RenderOptions { sample_rate: 48_000.0, tail_seconds: 0.5 };
        let buffer = render(&p, &PluginRegistry::with_builtins(), &opts);
        let data = buffer.data();
        let frames = buffer.frames();

        // 工程正文结束 = 0.05 拍 @120bpm = 0.025s = 1200 帧；其后全部属于 tail。
        let end_frame = 1_200usize;
        assert!(frames > end_frame, "buffer must include the tail region");

        // 尾音区域 [end_frame, frames) 不能全零：释放/衰减确实被渲染了。
        assert!(
            data[end_frame * 2..].iter().any(|s| *s != 0.0),
            "note release must be rendered into the tail region"
        );
        // 最强断言：缓冲「最后 0.1 s」也必须非零 —— 自动停止会在正文末尾截断尾音。
        let last_100ms = (0.1 * 48_000.0) as usize;
        assert!(
            data[(frames - last_100ms) * 2..].iter().any(|s| *s != 0.0),
            "note release must survive into the final 100ms of the tail"
        );
    }
}
