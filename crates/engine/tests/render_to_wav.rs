use ai_daw_engine::bus::Engine;
use ai_daw_engine::command::Command;
use ai_daw_engine::event::{Actor, Event};
use ai_daw_engine::model::Note;
use ai_daw_engine::registry::PluginRegistry;
use ai_daw_engine::render::{render, RenderOptions};
use ai_daw_engine::wav::write_wav;

fn demo() -> ai_daw_engine::model::Project {
    let mut e = Engine::new("e2e");
    let ev = e.execute(Command::AddTrack { name: "Lead".into() }, Actor::Human);
    let track = match ev[0] { Event::TrackAdded { id, .. } => id, _ => unreachable!() };
    e.execute(Command::SetTrackInstrument { track, plugin_id: "builtin.synth".into() }, Actor::Human);
    let ev = e.execute(Command::AddClip { track, start: 0.0, length: 2.0 }, Actor::Human);
    let clip = match ev[0] { Event::ClipAdded { clip, .. } => clip, _ => unreachable!() };
    // a simple C major arpeggio
    for (i, pitch) in [60u8, 64, 67, 72].into_iter().enumerate() {
        e.execute(
            Command::AddNote {
                track,
                clip,
                note: Note { id: 0, pitch, start: i as f64 * 0.5, duration: 0.5, velocity: 100, volume: 127, offset: 0.0, legato: false },
            },
            Actor::Human,
        );
    }
    e.project().clone()
}

#[test]
fn renders_builtin_sampled_instruments() {
    for (plugin_id, pitch) in [
        ("builtin.piano", 60u8),
        ("builtin.strings", 60),
        ("builtin.slow_strings", 60),
        ("builtin.violin", 72),
        ("builtin.cello", 48),
        ("builtin.pizzicato", 60),
        ("builtin.trumpet", 67),
        ("builtin.french_horns", 60),
        ("builtin.synth_strings_2", 60),
        ("builtin.synth_brass_2", 60),
    ] {
        let mut e = Engine::new(plugin_id);
        let ev = e.execute(Command::AddTrack { name: plugin_id.into() }, Actor::Human);
        let track = match ev[0] { Event::TrackAdded { id, .. } => id, _ => unreachable!() };
        e.execute(
            Command::SetTrackInstrument { track, plugin_id: plugin_id.into() },
            Actor::Human,
        );
        let ev = e.execute(Command::AddClip { track, start: 0.0, length: 2.0 }, Actor::Human);
        let clip = match ev[0] { Event::ClipAdded { clip, .. } => clip, _ => unreachable!() };
        e.execute(
            Command::AddNote {
                track,
                clip,
                note: Note {
                    id: 0,
                    pitch,
                    start: 0.0,
                    duration: 1.0,
                    velocity: 100,
                    volume: 127,
                    offset: 0.0,
                    legato: false,
                },
            },
            Actor::Human,
        );

        let opts = RenderOptions { sample_rate: 44_100.0, tail_seconds: 0.2, settle_seconds: 0.0 };
        let buffer = render(e.project(), &PluginRegistry::with_builtins(), &opts);
        assert!(
            buffer.data().iter().any(|s| s.abs() > 1e-4),
            "{plugin_id} 必须能通过引擎渲染出非静音"
        );
    }
}

#[test]
fn renders_project_to_readable_wav() {
    let project = demo();
    let opts = RenderOptions { sample_rate: 44_100.0, tail_seconds: 0.5, settle_seconds: 0.0 };
    let buffer = render(&project, &PluginRegistry::with_builtins(), &opts);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.wav");
    write_wav(&path, &buffer, 44_100).unwrap();

    let mut reader = hound::WavReader::open(&path).unwrap();
    assert_eq!(reader.spec().channels, 2);
    assert_eq!(reader.spec().sample_rate, 44_100);
    assert_eq!(reader.len(), buffer.frames() as u32 * 2);
    assert!(
        reader.samples::<i16>().any(|s| s.unwrap() != 0),
        "rendered wav must contain audible (non-zero) samples"
    );
}
