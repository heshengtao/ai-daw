use ai_daw_engine::bus::Engine;
use ai_daw_engine::command::Command;
use ai_daw_engine::event::{Actor, Event};
use ai_daw_engine::model::Note;
use ai_daw_engine::registry::PluginRegistry;
use ai_daw_engine::render::{render, RenderOptions};
use ai_daw_engine::wav::write_wav;

fn main() {
    let mut e = Engine::new("demo");
    let ev = e.execute(
        Command::AddTrack {
            name: "Lead".into(),
        },
        Actor::Human,
    );
    let track = match ev[0] {
        Event::TrackAdded { id, .. } => id,
        _ => unreachable!(),
    };
    e.execute(
        Command::SetTrackInstrument {
            track,
            plugin_id: "builtin.synth".into(),
        },
        Actor::Human,
    );
    let ev = e.execute(
        Command::AddClip {
            track,
            start: 0.0,
            length: 4.0,
        },
        Actor::Human,
    );
    let clip = match ev[0] {
        Event::ClipAdded { clip, .. } => clip,
        _ => unreachable!(),
    };
    for (i, pitch) in [60u8, 64, 67, 72, 67, 64].into_iter().enumerate() {
        e.execute(
            Command::AddNote {
                track,
                clip,
                note: Note {
                    id: 0,
                    channel: None,
                    pressure: None,
                    pitch,
                    start: i as f64 * 0.5,
                    duration: 0.5,
                    velocity: 100,
                    volume: 127,
                    offset: 0.0,
                    legato: false,
                    expr: ai_daw_engine::model::PitchExpression::NONE,
                    lyric: None,
                },
            },
            Actor::Human,
        );
    }
    let buffer = render(
        e.project(),
        &PluginRegistry::with_builtins(),
        &RenderOptions {
            sample_rate: 44_100.0,
            tail_seconds: 0.5,
            settle_seconds: 0.0,
        },
    );
    write_wav("demo.wav", &buffer, 44_100).unwrap();
    println!("wrote demo.wav ({} frames)", buffer.frames());
}
