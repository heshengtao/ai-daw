use tauri::{Emitter, State};

use daw_party_engine::command::Command;
use daw_party_engine::event::{Actor, Event};
use daw_party_engine::model::{Note, Project};
use daw_party_engine::registry::PluginRegistry;
use daw_party_engine::render::{render, RenderOptions};
use daw_party_engine::wav::wav_bytes;

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
///
/// 全部经由 `run()`（命令/事件总线）构造：每次变更都会广播事件并进入撤销历史，
/// 不再直接替换引擎（那样会绕过命令/事件模型、重置 id 计数器，且不可撤销）。
#[tauri::command]
pub fn load_demo(app: tauri::AppHandle, state: State<'_, AppState>) -> Project {
    let _ = run(&app, &state, Command::Clear { name: "demo".into() });
    let after_track = run(&app, &state, Command::AddTrack { name: "Lead".into() });
    let track = after_track.tracks.last().expect("just added").id;
    let after_instrument = run(
        &app,
        &state,
        Command::SetTrackInstrument { track, plugin_id: "builtin.synth".into() },
    );
    let after_clip = run(&app, &state, Command::AddClip { track, start: 0.0, length: 4.0 });
    let clip = after_clip
        .tracks
        .iter()
        .find(|t| t.id == track)
        .and_then(|t| t.clips.last())
        .expect("just added")
        .id;
    let mut project = after_instrument;
    for (i, pitch) in [60u8, 64, 67, 72, 67, 64, 60, 64].into_iter().enumerate() {
        project = run(
            &app,
            &state,
            Command::AddNote {
                track,
                clip,
                note: Note { pitch, start: i as f64 * 0.5, duration: 0.5, velocity: 100 },
            },
        );
    }
    project
}

/// 采样率必须落在合理区间，避免渲染出无效 WAV 或触发超大分配。
fn validate_sample_rate(sample_rate: u32) -> Result<(), String> {
    if (8_000..=192_000).contains(&sample_rate) {
        Ok(())
    } else {
        Err(format!("unsupported sample rate: {sample_rate} (expected 8000..=192000)"))
    }
}

/// 离线渲染当前工程为 WAV 字节，供前端试听。
#[tauri::command]
pub async fn render_preview(
    state: State<'_, AppState>,
    sample_rate: u32,
) -> Result<tauri::ipc::Response, String> {
    validate_sample_rate(sample_rate)?;
    // 只在锁内克隆工程快照，绝不把 MutexGuard 带过 `.await`。
    let project = { state.engine.lock().unwrap().project().clone() };
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let registry = PluginRegistry::with_builtins();
        let options = RenderOptions { sample_rate: sample_rate as f64, tail_seconds: 0.5 };
        let buffer = render(&project, &registry, &options);
        wav_bytes(&buffer, sample_rate)
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// 把当前工程导出为 WAV 文件，返回写入的绝对路径。
#[tauri::command]
pub async fn export_wav(
    state: State<'_, AppState>,
    path: String,
    sample_rate: u32,
) -> Result<String, String> {
    validate_sample_rate(sample_rate)?;
    // 同样只在锁内克隆，渲染与写盘都放到阻塞线程池，避免卡住主线程。
    let project = { state.engine.lock().unwrap().project().clone() };
    tauri::async_runtime::spawn_blocking(move || {
        let registry = PluginRegistry::with_builtins();
        let options = RenderOptions { sample_rate: sample_rate as f64, tail_seconds: 0.5 };
        let buffer = render(&project, &registry, &options);
        daw_party_engine::wav::write_wav(&path, &buffer, sample_rate).map_err(|e| e.to_string())?;
        Ok::<String, String>(path)
    })
    .await
    .map_err(|e| e.to_string())?
}
