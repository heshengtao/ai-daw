use std::thread;
use std::time::Duration;

use daw_party_engine::bus::Engine;
use daw_party_engine::command::Command;
use daw_party_engine::event::{Actor, Event};
use daw_party_engine::model::Note;
use daw_party_engine::registry::PluginRegistry;
use daw_party_engine::realtime::{AudioHost, RealtimeOptions, SharedStatus};
use daw_party_engine::schedule::ScheduledEvents;

fn main() {
    let mut engine = Engine::new("realtime demo");
    let ev = engine.execute(Command::AddTrack { name: "Lead".into() }, Actor::Human);
    let track = match ev[0] { Event::TrackAdded { id, .. } => id, _ => unreachable!() };
    engine.execute(Command::SetTrackInstrument { track, plugin_id: "builtin.synth".into() }, Actor::Human);
    let ev = engine.execute(Command::AddClip { track, start: 0.0, length: 4.0 }, Actor::Human);
    let clip = match ev[0] { Event::ClipAdded { clip, .. } => clip, _ => unreachable!() };
    for (i, pitch) in [60u8, 64, 67, 72, 67, 64, 60].into_iter().enumerate() {
        engine.execute(
            Command::AddNote { track, clip, note: Note { pitch, start: i as f64 * 0.5, duration: 0.5, velocity: 100, volume: 127 } },
            Actor::Human,
        );
    }

    let schedule = ScheduledEvents::from_project(engine.project(), 48_000.0);
    let registry = PluginRegistry::with_builtins();
    let options = RealtimeOptions {
        max_block: 2048,
        command_capacity: 64,
        scope: std::sync::Arc::new(daw_party_engine::scope::ScopeBuffer::new(8192)),
        status: std::sync::Arc::new(SharedStatus::new()),
    };

    let mut host = match AudioHost::start(schedule, &registry, options) {
        Ok(host) => host,
        Err(e) => {
            eprintln!("could not start audio: {e}");
            return;
        }
    };

    println!("playing 2 bars...");
    host.play();
    thread::sleep(Duration::from_secs(4));
    host.stop();
    println!("stopped at frame {}", host.position_frames());
}
