use tauri::{Emitter, State};

use daw_party_engine::bus::Engine;
use daw_party_engine::command::Command;
use daw_party_engine::event::{Actor, Event};
use daw_party_engine::model::Project;
use daw_party_engine::registry::PluginRegistry;
use daw_party_engine::render::{render, RenderOptions};
use daw_party_engine::wav::wav_bytes;

use crate::state::{AppState, EVENT_ENGINE};

/// 撤销/重做之后返回的历史状态（让界面能立刻更新按钮可用性）。
#[derive(serde::Serialize)]
pub struct HistoryState {
    pub project: Project,
    pub undo_depth: usize,
    pub redo_depth: usize,
}

/// 当前历史深度。
#[derive(serde::Serialize)]
pub struct HistoryDepths {
    pub undo: usize,
    pub redo: usize,
}

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

/// 当前可撤销 / 可重做的次数。
#[tauri::command]
pub fn history_depths(state: State<'_, AppState>) -> HistoryDepths {
    let engine = state.engine.lock().unwrap();
    HistoryDepths { undo: engine.undo_depth(), redo: engine.redo_depth() }
}

/// 撤销最后一次命令。
#[tauri::command]
pub fn undo(state: State<'_, AppState>) -> HistoryState {
    let mut engine = state.engine.lock().unwrap();
    engine.undo();
    HistoryState {
        project: engine.project().clone(),
        undo_depth: engine.undo_depth(),
        redo_depth: engine.redo_depth(),
    }
}

/// 重做上一次被撤销的命令。
#[tauri::command]
pub fn redo(state: State<'_, AppState>) -> HistoryState {
    let mut engine = state.engine.lock().unwrap();
    engine.redo();
    HistoryState {
        project: engine.project().clone(),
        undo_depth: engine.undo_depth(),
        redo_depth: engine.redo_depth(),
    }
}

/// 把当前工程保存为 JSON 文件，返回写入的绝对路径。
#[tauri::command]
pub async fn save_project(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let project = { state.engine.lock().unwrap().project().clone() };
    let json = serde_json::to_string_pretty(&project).map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::write(&path, json).map_err(|e| e.to_string())?;
        Ok::<String, String>(path)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 从 JSON 文件导入工程：替换当前工程并清空历史（导入是一个新的起点）。
#[tauri::command]
pub async fn open_project(state: State<'_, AppState>, path: String) -> Result<Project, String> {
    let text = tauri::async_runtime::spawn_blocking(move || std::fs::read_to_string(&path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let mut project: Project =
        serde_json::from_str(&text).map_err(|e| format!("不是有效的工程文件：{e}"))?;
    // 与 `SetTempo` 一致地夹紧：非有限或 0 的速度会让离线渲染时长变成 Inf。
    if !project.transport.tempo.is_finite() {
        project.transport.tempo = 120.0;
    }
    project.transport.tempo = project.transport.tempo.clamp(1.0, 1000.0);

    let mut engine = state.engine.lock().unwrap();
    *engine = Engine::from_project(project);
    Ok(engine.project().clone())
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
