use daw_party_engine::bus::Engine;
use daw_party_engine::command::Command;
use daw_party_engine::event::{Actor, Event};
use daw_party_engine::model::Note;
use daw_party_engine::registry::PluginRegistry;
use daw_party_engine::render::{render, RenderOptions};
use daw_party_engine::wav::write_wav;

fn demo() -> daw_party_engine::model::Project {
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
                note: Note { pitch, start: i as f64 * 0.5, duration: 0.5, velocity: 100 },
            },
            Actor::Human,
        );
    }
    e.project().clone()
}

#[test]
fn renders_project_to_readable_wav() {
    let project = demo();
    let opts = RenderOptions { sample_rate: 44_100.0, tail_seconds: 0.5 };
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
