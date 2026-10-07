use tauri::{Emitter, State};

use daw_party_engine::bus::Engine;
use daw_party_engine::command::Command;
use daw_party_engine::event::{Actor, Event};
use daw_party_engine::model::{Note, Project};

use crate::state::{AppState, EVENT_ENGINE};

/// 读取当前工程快照。
#[tauri::command]
pub fn get_project(state: State<'_, AppState>) -> Project {
    state.engine.lock().unwrap().project().clone()
}

fn run(app: &tauri::AppHandle, state: &State<'_, AppState>, command: Command) -> Project {
    let mut engine = state.engine.lock().unwrap();
    let events: Vec<Event> = engine.execute(command, Actor::Human);
    let project = engine.project().clone();
    drop(engine);
    for event in events {
        let _ = app.emit(EVENT_ENGINE, event);
    }
    project
}

/// 新增一条轨道。
#[tauri::command]
pub fn add_track(app: tauri::AppHandle, state: State<'_, AppState>, name: String) -> Project {
    run(&app, &state, Command::AddTrack { name })
}

/// 设置速度（BPM）。
#[tauri::command]
pub fn set_tempo(app: tauri::AppHandle, state: State<'_, AppState>, bpm: f64) -> Project {
    run(&app, &state, Command::SetTempo(bpm))
}

/// 清空为新工程。
#[tauri::command]
pub fn clear_project(app: tauri::AppHandle, state: State<'_, AppState>) -> Project {
    run(&app, &state, Command::Clear { name: "untitled".into() })
}

/// 载入内置示例（C 大调琶音），让界面立刻有内容可看可听。
#[tauri::command]
pub fn load_demo(app: tauri::AppHandle, state: State<'_, AppState>) -> Project {
    // 只锁一次：重置引擎，并建出示例轨道、乐器、片段与音符。
    let (project, events) = {
        let mut engine = state.engine.lock().unwrap();
        *engine = Engine::new("demo");
        let mut events: Vec<Event> = Vec::new();
        events.extend(engine.execute(Command::AddTrack { name: "Lead".into() }, Actor::Human));
        let track = match events[0] {
            Event::TrackAdded { id, .. } => id,
            _ => unreachable!(),
        };
        events.extend(engine.execute(
            Command::SetTrackInstrument { track, plugin_id: "builtin.synth".into() },
            Actor::Human,
        ));
        let added = engine.execute(Command::AddClip { track, start: 0.0, length: 4.0 }, Actor::Human);
        let clip = match added[0] {
            Event::ClipAdded { clip, .. } => clip,
            _ => unreachable!(),
        };
        events.extend(added);
        for (i, pitch) in [60u8, 64, 67, 72, 67, 64, 60, 64].into_iter().enumerate() {
            events.extend(engine.execute(
                Command::AddNote {
                    track,
                    clip,
                    note: Note { pitch, start: i as f64 * 0.5, duration: 0.5, velocity: 100 },
                },
                Actor::Human,
            ));
        }
        (engine.project().clone(), events)
    };
    for event in events {
        let _ = app.emit(EVENT_ENGINE, event);
    }
    project
}
