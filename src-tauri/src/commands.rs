use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tauri::{Emitter, Manager, State};

use daw_party_engine::audio::{AudioBuffer, DecodedAudio};
use daw_party_engine::bus::Engine;
use daw_party_engine::command::Command;
use daw_party_engine::event::{Actor, Event};
use daw_party_engine::model::{AudioSource, ClipPlacement, ClipRef, Note, NoteEdit, Project, TrackId};
use daw_party_engine::registry::PluginRegistry;
use daw_party_engine::render::{render_with_assets, RenderOptions};
use daw_party_engine::tempo::TempoMap;
use daw_party_engine::transport::TransportCommand;
use daw_party_engine::wav::wav_bytes;
use daw_party_host_vst3::{CatalogPlugin, EditorHandle};
use daw_party_plugin_api::ParamInfo;

use crate::audio::TransportSnapshot;
use crate::plugins::build_registry;
use crate::presets::Preset;
use crate::state::{with_editor_windows, AppState, EVENT_ENGINE, EVENT_PLUGINS};

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

/// 执行一条命令：应用状态、广播事件、把变更同步到音频线程。
///
/// 这是**唯一**的编辑入口——界面与 AI 代理（MCP）都走这里，因此撤销历史、事件流与
/// 实时播放天然保持一致。`actor` 决定事件流里的署名是人还是代理。
fn run_as(
    app: &tauri::AppHandle,
    state: &State<'_, AppState>,
    actor: Actor,
    command: Command,
) -> (Project, Vec<Event>) {
    let (project, events) = {
        let mut engine = state.engine.lock().unwrap();
        let events: Vec<Event> = engine.execute(command, actor);
        (engine.project().clone(), events)
    };
    for event in &events {
        let _ = app.emit(EVENT_ENGINE, event);
    }
    state.audio.sync_events(&project, &events);
    // 结构性变更会重建播放器、连带重建插件实例：已开的窗口此时挂在**停用的旧实例**上，
    // 继续调它不会影响声音。直接关掉，让用户重新打开（下次可做成自动重挂）。
    if events.iter().any(is_structural) {
        close_all_plugin_windows(app, state);
    }
    (project, events)
}

/// 界面走的编辑入口（署名 `Actor::Human`）。
fn run(app: &tauri::AppHandle, state: &State<'_, AppState>, command: Command) -> Project {
    run_as(app, state, Actor::Human, command).0
}

/// MCP 服务器走的编辑入口：署名 `Actor::Agent`，其余管线与界面**完全一致**。
///
/// 返回新工程与本次产生的事件，便于工具把手边新对象的 id 回给代理。
pub(crate) fn run_agent(app: &tauri::AppHandle, command: Command) -> (Project, Vec<Event>) {
    let state = app.state::<AppState>();
    run_as(app, &state, Actor::Agent("mcp".into()), command)
}

/// 渲染一份快照所需的全部输入。
///
/// **音频素材在这里（命令线程）就解码好**：渲染与音频回调都不做任何 IO。
/// 缺失的素材会被跳过 → 该片段静音。
pub(crate) struct RenderInputs {
    pub project: Project,
    pub catalog: Vec<CatalogPlugin>,
    pub editor_handles: Arc<Mutex<HashMap<u64, EditorHandle>>>,
    pub samples: HashMap<String, Arc<DecodedAudio>>,
}

/// 取当前状态的可渲染快照（工程 + 插件目录 + 编辑器句柄表 + 已解码素材）。
///
/// 只在锁内克隆工程，绝不把 `MutexGuard` 带过 `.await`。
pub(crate) fn render_inputs(state: &AppState) -> RenderInputs {
    let project = state.engine.lock().unwrap().project().clone();
    let samples = state.assets.snapshot_for(&project.audio_paths());
    RenderInputs {
        project,
        catalog: state.catalog_snapshot(),
        editor_handles: Arc::clone(&state.editor_handles),
        samples,
    }
}

/// 离线渲染一份快照（预览 / 导出 / 分析 / MCP 共用同一条路径）。
pub(crate) fn render_offline(inputs: RenderInputs, sample_rate: u32, tail_seconds: f64) -> AudioBuffer {
    let RenderInputs { project, catalog, editor_handles, samples } = inputs;
    let registry = build_registry(&catalog, &project, editor_handles);
    let options = RenderOptions { sample_rate: sample_rate as f64, tail_seconds };
    render_with_assets(&project, &registry, &options, &samples)
}

/// 该事件是否会导致播放器重建（混音与实时参数不会）。
fn is_structural(event: &Event) -> bool {
    !event.is_mix_only()
        && !matches!(event, Event::ParamChanged { .. } | Event::EffectParamChanged { .. })
}

/// 关掉所有插件窗口（须在主线程执行创建/销毁，所以走 `run_on_main_thread`）。
///
/// 顺带清空 `editor_handles`：结构一变，那些编辑器句柄就指向即将被丢弃的实例。句柄持有实例
/// 的 `Arc`，不清就会让已销毁的插件实例一直留在内存里，也会让编辑器 ticker 去服务过期句柄。
/// 音频线程重建 Player 时会重新登记新句柄。
///
/// 清空**必须在主线程上做**：句柄可能正好是实例的最后一个引用，而释放会触发
/// `clap_plugin.destroy()`——那是主线程专属调用。
fn close_all_plugin_windows(app: &tauri::AppHandle, state: &AppState) {
    let tracks: Vec<u64> = state.open_editors.lock().unwrap().iter().copied().collect();
    let (tx, rx) = std::sync::mpsc::channel();
    let app_for_main = app.clone();
    if app
        .run_on_main_thread(move || {
            let state = app_for_main.state::<AppState>();
            with_editor_windows(|windows| {
                for track in &tracks {
                    // 走 `close()` 而不是直接丢：它会先 hide + destroy 插件的界面。
                    if let Some(window) = windows.remove(track) {
                        window.close();
                    }
                }
            });
            // 嵌入模式的宿主窗口也一并收掉，别留下一扇空窗。
            for track in &tracks {
                if state.host_windows.lock().unwrap().remove(track) {
                    if let Some(window) =
                        app_for_main.get_webview_window(&host_window_label(*track))
                    {
                        let _ = window.close();
                    }
                }
            }
            state.open_editors.lock().unwrap().clear();
            state.editor_handles.lock().unwrap().clear();
            let _ = tx.send(());
        })
        .is_ok()
    {
        let _ = rx.recv();
    }
}

// ---------------------------------------------------------------------------
// 工程与轨道
// ---------------------------------------------------------------------------

/// 新增一条轨道。
#[tauri::command]
pub fn add_track(app: tauri::AppHandle, state: State<'_, AppState>, name: String) -> Project {
    run(&app, &state, Command::AddTrack { name })
}

/// 删除一条轨道（含其片段与音符）。
#[tauri::command]
pub fn remove_track(app: tauri::AppHandle, state: State<'_, AppState>, track: u64) -> Project {
    run(&app, &state, Command::RemoveTrack(TrackId(track)))
}

/// 重命名一条轨道。
#[tauri::command]
pub fn rename_track(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    name: String,
) -> Project {
    run(&app, &state, Command::RenameTrack { track: TrackId(track), name })
}

/// 为轨道设置乐器插件。
#[tauri::command]
pub fn set_track_instrument(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    plugin_id: String,
) -> Project {
    run(&app, &state, Command::SetTrackInstrument { track: TrackId(track), plugin_id })
}

/// 设置轨道乐器的参数（参数 id 由插件自身定义）。
#[tauri::command]
pub fn set_param(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    param: String,
    value: f32,
) -> Project {
    run(&app, &state, Command::SetParam { track: TrackId(track), param, value })
}

/// 设置速度（BPM）。
#[tauri::command]
pub fn set_tempo(app: tauri::AppHandle, state: State<'_, AppState>, bpm: f64) -> Project {
    run(&app, &state, Command::SetTempo(bpm))
}

/// 一次性设置一条轨道上的多个参数——切换预设走的就是这条路。
///
/// 逐条调 `set_param` 会有 N 次 IPC、N 次工程克隆、N 个撤销步骤；预设动辄上百个参数，
/// 所以这里合成**一次**事务：一次应用、一次广播、一个撤销步骤。
#[tauri::command]
pub fn set_params(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    params: HashMap<String, f32>,
) -> Project {
    // 按参数名排序：`HashMap` 的遍历顺序随机，固定顺序才能让事件流与撤销历史可复现。
    let mut pairs: Vec<(String, f32)> = params.into_iter().collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    let commands: Vec<Command> = pairs
        .into_iter()
        .map(|(param, value)| Command::SetParam { track: TrackId(track), param, value })
        .collect();

    let (project, events) = {
        let mut engine = state.engine.lock().unwrap();
        let events = engine.execute_all(commands, Actor::Human);
        (engine.project().clone(), events)
    };
    for event in &events {
        let _ = app.emit(EVENT_ENGINE, event);
    }
    state.audio.sync_events(&project, &events);
    project
}

/// 预设自带的一条效果器声明（前端 `FactoryPreset.effects` 的形状）。
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct EffectSpec {
    pub plugin_id: String,
    /// 参数名 → 值；缺省表示全用插件自己的默认值。
    #[serde(default)]
    pub params: HashMap<String, f32>,
}

/// 应用一条预设的**内核**：一次事务写回整组乐器参数 + 整条效果链。
///
/// 抽出来是因为 Tauri 命令与 MCP 工具走的是同一件事，只有「谁来广播、谁来同步音频线程」不同。
pub(crate) fn apply_preset_inner(
    state: &AppState,
    track: TrackId,
    params: &HashMap<String, f32>,
    effects: &[EffectSpec],
    actor: Actor,
) -> (Project, Vec<Event>) {
    let mut engine = state.engine.lock().unwrap();
    let existing = engine
        .project()
        .tracks
        .iter()
        .find(|t| t.id == track)
        .map_or(0, |t| t.effects.len());

    let mut commands: Vec<Command> = Vec::new();
    // 参数与效果器一起排序：`HashMap` 的遍历顺序随机，固定顺序才能让事件流与撤销历史可复现。
    let mut pairs: Vec<(&String, &f32)> = params.iter().collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    for (param, value) in pairs {
        commands.push(Command::SetParam { track, param: param.clone(), value: *value });
    }
    // 先清空效果链（每一步都删第 0 个，链自己前移），再按顺序重建。
    for _ in 0..existing {
        commands.push(Command::RemoveEffect { track, index: 0 });
    }
    for (index, spec) in effects.iter().enumerate() {
        commands.push(Command::AddEffect { track, plugin_id: spec.plugin_id.clone() });
        let mut fx_pairs: Vec<(&String, &f32)> = spec.params.iter().collect();
        fx_pairs.sort_by(|a, b| a.0.cmp(b.0));
        for (param, value) in fx_pairs {
            commands.push(Command::SetEffectParam {
                track,
                index,
                param: param.clone(),
                value: *value,
            });
        }
    }
    let events = engine.execute_all(commands, actor);
    (engine.project().clone(), events)
}

/// 应用一条预设（MCP 入口）：取状态 → 执行 → 广播 → 同步音频线程。
pub(crate) fn apply_preset_agent(
    app: &tauri::AppHandle,
    track: TrackId,
    params: &HashMap<String, f32>,
    effects: &[EffectSpec],
) -> (Project, Vec<Event>) {
    let state = app.state::<AppState>();
    let (project, events) = apply_preset_inner(&state, track, params, effects, Actor::Agent("mcp".into()));
    for event in &events {
        let _ = app.emit(EVENT_ENGINE, event);
    }
    state.audio.sync_events(&project, &events);
    (project, events)
}

/// 应用一条预设：**一次事务**写回整组乐器参数 + 整条效果链。
///
/// 走 `execute_all`：一次应用、一次广播、**一个撤销步骤**。分两次调用（先 `set_params`
/// 再改效果链）会变成两个撤销步骤——按一下预设要按两次 Cmd+Z 才回得去。
///
/// 效果链是**替换**而不是叠加：预设 = 整套音色，切预设本来就会丢掉所有旋钮微调，
/// 丢掉效果微调是同一个契约。
#[tauri::command]
pub fn apply_preset(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    params: HashMap<String, f32>,
    effects: Vec<EffectSpec>,
) -> Project {
    let (project, events) =
        apply_preset_inner(&state, TrackId(track), &params, &effects, Actor::Human);
    for event in &events {
        let _ = app.emit(EVENT_ENGINE, event);
    }
    state.audio.sync_events(&project, &events);
    project
}

/// 设置拍号；越界值由引擎归一化（分子 1..=32，分母吸附到 2 的幂）。
#[tauri::command]
pub fn set_time_signature(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    num: u8,
    den: u8,
) -> Project {
    run(&app, &state, Command::SetTimeSignature { num, den })
}

/// 在第 `beat` 拍插入/更新关键点（BPM + 拍号）。
#[tauri::command]
pub fn set_keyframe(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    beat: f64,
    tempo: f64,
    num: u8,
    den: u8,
) -> Project {
    run(&app, &state, Command::SetKeyframe { beat, tempo, num, den })
}

/// 删除第 `beat` 拍的关键点。
#[tauri::command]
pub fn remove_keyframe(app: tauri::AppHandle, state: State<'_, AppState>, beat: f64) -> Project {
    run(&app, &state, Command::RemoveKeyframe { beat })
}

/// 新建工程：清空并铺上默认分轨模板。
///
/// `track_names` 由界面按**当前语言**给出（按模板顺序）；不传就用引擎的兜底名。
/// 名字写进工程后就是用户数据了，之后切换界面语言不会再动它。
#[tauri::command]
pub fn new_project(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track_names: Option<Vec<String>>,
) -> Project {
    run(
        &app,
        &state,
        Command::NewProject { name: "untitled".into(), track_names },
    )
}

/// 把**启动时**铺的默认分轨名改成当前语言的名字。
///
/// 启动时的初始工程是在界面起来之前建的，那时还不知道界面语言。这个方法只在工程仍是
/// 原封不动的默认模板时生效（引擎侧判断），所以对已经动过的工程完全无害。
///
/// 改名属于「初始状态」而不是用户编辑：它不进撤销栈，也不广播事件——界面直接用返回值刷新。
#[tauri::command]
pub fn localize_starter_tracks(state: State<'_, AppState>, names: Vec<String>) -> Project {
    let mut engine = state.engine.lock().unwrap();
    engine.localize_starter_tracks(&names);
    engine.project().clone()
}

// ---------------------------------------------------------------------------
// 片段与音符
// ---------------------------------------------------------------------------

/// 在轨道上新增一个 MIDI 片段。
#[tauri::command]
pub fn add_clip(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    start: f64,
    length: f64,
) -> Project {
    run(&app, &state, Command::AddClip { track: TrackId(track), start, length })
}

/// 删除一个片段。
#[tauri::command]
pub fn remove_clip(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
) -> Project {
    run(&app, &state, Command::RemoveClip { track: TrackId(track), clip: daw_party_engine::model::ClipId(clip) })
}

/// 把片段移动到新的起始拍。
#[tauri::command]
pub fn move_clip(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
    start: f64,
) -> Project {
    run(
        &app,
        &state,
        Command::MoveClip { track: TrackId(track), clip: daw_party_engine::model::ClipId(clip), start },
    )
}

/// 批量移动片段（多选拖动，一次提交一条撤销记录）。
#[tauri::command]
pub fn move_clips(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    items: Vec<ClipPlacement>,
) -> Project {
    run(&app, &state, Command::MoveClips { items })
}

/// 改变片段长度（拍）。
#[tauri::command]
pub fn resize_clip(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
    length: f64,
) -> Project {
    run(
        &app,
        &state,
        Command::ResizeClip { track: TrackId(track), clip: daw_party_engine::model::ClipId(clip), length },
    )
}

/// 在片段中新增一个音符。
#[tauri::command]
pub fn add_note(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
    note: Note,
) -> Project {
    run(
        &app,
        &state,
        Command::AddNote { track: TrackId(track), clip: daw_party_engine::model::ClipId(clip), note },
    )
}

/// 改写片段内的某个音符（按片段内索引定位）。
#[tauri::command]
pub fn set_note(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
    index: usize,
    note: Note,
) -> Project {
    run(
        &app,
        &state,
        Command::SetNote { track: TrackId(track), clip: daw_party_engine::model::ClipId(clip), index, note },
    )
}

/// 批量改写片段内的多个音符（多选拖动、属性面板一次改多个）。
#[tauri::command]
pub fn set_notes(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
    entries: Vec<NoteEdit>,
) -> Project {
    run(
        &app,
        &state,
        Command::SetNotes { track: TrackId(track), clip: daw_party_engine::model::ClipId(clip), entries },
    )
}

/// 删除片段内的某个音符（按片段内索引定位）。
#[tauri::command]
pub fn remove_note(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
    index: usize,
) -> Project {
    run(
        &app,
        &state,
        Command::RemoveNote { track: TrackId(track), clip: daw_party_engine::model::ClipId(clip), index },
    )
}

/// 批量删除片段内的多个音符（框选后删除）。
#[tauri::command]
pub fn remove_notes(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
    indices: Vec<usize>,
) -> Project {
    run(
        &app,
        &state,
        Command::RemoveNotes { track: TrackId(track), clip: daw_party_engine::model::ClipId(clip), indices },
    )
}

/// 批量删除片段（可跨轨道，用于框选删除）。
#[tauri::command]
pub fn remove_clips(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    items: Vec<ClipRef>,
) -> Project {
    run(&app, &state, Command::RemoveClips { items })
}

// ---------------------------------------------------------------------------
// 混音
// ---------------------------------------------------------------------------

/// 设置轨道增益（dB）。
#[tauri::command]
pub fn set_gain(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    gain_db: f32,
) -> Project {
    run(&app, &state, Command::SetGain { track: TrackId(track), gain_db })
}

/// 设置轨道声像（-1.0 最左，1.0 最右）。
#[tauri::command]
pub fn set_pan(app: tauri::AppHandle, state: State<'_, AppState>, track: u64, pan: f32) -> Project {
    run(&app, &state, Command::SetPan { track: TrackId(track), pan })
}

/// 设置轨道静音。
#[tauri::command]
pub fn set_mute(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    mute: bool,
) -> Project {
    run(&app, &state, Command::SetMute { track: TrackId(track), mute })
}

/// 设置轨道独奏。
#[tauri::command]
pub fn set_solo(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    solo: bool,
) -> Project {
    run(&app, &state, Command::SetSolo { track: TrackId(track), solo })
}

/// 设置总输出增益（dB）。
#[tauri::command]
pub fn set_master_gain(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    gain_db: f32,
) -> Project {
    run(&app, &state, Command::SetMasterGain(gain_db))
}

/// 实时监听用的混音预览：**不写入工程、不进入撤销历史**。
///
/// 拖动推子时界面以每秒数十次的频率调用它；松手时才发一条真正的 `set_gain`。
#[tauri::command]
pub fn monitor_track_mix(
    state: State<'_, AppState>,
    track: u64,
    gain_db: f32,
    pan: f32,
    mute: bool,
    solo: bool,
) {
    state.audio.monitor_track_mix(TrackId(track), gain_db, pan, mute, solo);
}

// ---------------------------------------------------------------------------
// 历史
// ---------------------------------------------------------------------------

/// 当前可撤销 / 可重做的次数。
#[tauri::command]
pub fn history_depths(state: State<'_, AppState>) -> HistoryDepths {
    let engine = state.engine.lock().unwrap();
    HistoryDepths { undo: engine.undo_depth(), redo: engine.redo_depth() }
}

/// 撤销最后一次命令（界面命令与 MCP 共用）。
pub(crate) fn undo_now(state: &AppState) -> HistoryState {
    let (out, project) = {
        let mut engine = state.engine.lock().unwrap();
        engine.undo();
        (
            HistoryState {
                project: engine.project().clone(),
                undo_depth: engine.undo_depth(),
                redo_depth: engine.redo_depth(),
            },
            engine.project().clone(),
        )
    };
    // 撤销是"重放历史"实现的，结果可能任意偏离当前事件表，因此整表重建。
    state.audio.load(&project);
    out
}

/// 重做上一次被撤销的命令（界面命令与 MCP 共用）。
pub(crate) fn redo_now(state: &AppState) -> HistoryState {
    let (out, project) = {
        let mut engine = state.engine.lock().unwrap();
        engine.redo();
        (
            HistoryState {
                project: engine.project().clone(),
                undo_depth: engine.undo_depth(),
                redo_depth: engine.redo_depth(),
            },
            engine.project().clone(),
        )
    };
    state.audio.load(&project);
    out
}

/// 撤销最后一次命令。
#[tauri::command]
pub fn undo(state: State<'_, AppState>) -> HistoryState {
    undo_now(&state)
}

/// 重做上一次被撤销的命令。
#[tauri::command]
pub fn redo(state: State<'_, AppState>) -> HistoryState {
    redo_now(&state)
}

// ---------------------------------------------------------------------------
// 传输（实时播放）
// ---------------------------------------------------------------------------

/// 当前传输状态（可用性、播放中、位置、采样率）。
#[tauri::command]
pub fn transport_status(state: State<'_, AppState>) -> TransportSnapshot {
    state.audio.snapshot()
}

/// 重新探测输出设备并重建实时宿主（设备恢复后无需重启应用）。
///
/// 底栏的「离线试听」指示离线时变成按钮，点击后走这条命令；探测结果仍由
/// `engine://transport` 事件推送回来，界面无需自己刷新。
#[tauri::command]
pub fn retry_audio(state: State<'_, AppState>) -> Result<(), String> {
    state.audio.retry_audio();
    Ok(())
}

/// 开始播放。
#[tauri::command]
pub fn transport_play(state: State<'_, AppState>) -> Result<(), String> {
    state.audio.send(TransportCommand::Play)
}

/// 暂停（保留当前位置）。
#[tauri::command]
pub fn transport_pause(state: State<'_, AppState>) -> Result<(), String> {
    state.audio.send(TransportCommand::Pause)
}

/// 停止并回到起点。
#[tauri::command]
pub fn transport_stop(state: State<'_, AppState>) -> Result<(), String> {
    state.audio.send(TransportCommand::Stop)
}

/// 定位到指定帧。
#[tauri::command]
pub fn transport_seek(state: State<'_, AppState>, frame: u64) -> Result<(), String> {
    state.audio.send(TransportCommand::Seek { frame })
}

/// 开关循环播放：播到工程结尾无缝回到起点继续。
#[tauri::command]
pub fn transport_set_loop(state: State<'_, AppState>, on: bool) -> Result<(), String> {
    state.audio.send(TransportCommand::SetLoop(on))
}

// ---------------------------------------------------------------------------
// 文件与渲染
// ---------------------------------------------------------------------------

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
    // 拍号同样归一化：分母为 0 会让按小节换算的长度除不尽。
    let (num, den) = daw_party_engine::model::Transport::normalize_time_sig(
        project.transport.time_sig_num,
        project.transport.time_sig_den,
    );
    project.transport.time_sig_num = num;
    project.transport.time_sig_den = den;
    // 混音参数与片段/音符同样要过一遍归一化：工程文件可能被手改过。
    project.master.gain_db = daw_party_engine::model::MixerChannel::normalize_gain_db(project.master.gain_db);
    project.master.pan = daw_party_engine::model::MixerChannel::normalize_pan(project.master.pan);
    for track in project.tracks.iter_mut() {
        track.mixer.gain_db = daw_party_engine::model::MixerChannel::normalize_gain_db(track.mixer.gain_db);
        track.mixer.pan = daw_party_engine::model::MixerChannel::normalize_pan(track.mixer.pan);
        for clip in track.clips.iter_mut() {
            clip.start = daw_party_engine::model::Clip::normalize_start(clip.start);
            clip.length = daw_party_engine::model::Clip::normalize_length(clip.length);
            for note in clip.notes.iter_mut() {
                *note = Note::normalize(*note);
            }
        }
    }

    let snapshot = {
        let mut engine = state.engine.lock().unwrap();
        *engine = Engine::from_project(project);
        engine.project().clone()
    };
    state.audio.load(&snapshot);
    Ok(snapshot)
}

/// 采样率必须落在合理区间，避免渲染出无效 WAV 或触发超大分配。
fn validate_sample_rate(sample_rate: u32) -> Result<(), String> {
    if (8_000..=192_000).contains(&sample_rate) {
        Ok(())
    } else {
        Err(format!("unsupported sample rate: {sample_rate} (expected 8000..=192000)"))
    }
}

/// 离线渲染当前工程为 WAV 字节。
///
/// 实时播放不可用（或用户只想要一次性试听）时的回退路径；导出同样走它。
#[tauri::command]
pub async fn render_preview(
    state: State<'_, AppState>,
    sample_rate: u32,
) -> Result<tauri::ipc::Response, String> {
    validate_sample_rate(sample_rate)?;
    let inputs = render_inputs(&state);
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let buffer = render_offline(inputs, sample_rate, 0.5);
        wav_bytes(&buffer, sample_rate)
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// 对当前工程做一次离线分析：响度（LUFS/LRA/真峰值）、RMS、平均幅度谱。
///
/// 与预览/导出走同一条渲染路径（同样的插件实例、同样的逐块推进），所以分析的是**真实输出**。
/// 这是 AI 层的「听感」入口：分析结果可以喂给代理做混音/母带决策，也可以给界面画频谱。
#[tauri::command]
pub async fn analyze_project(
    state: State<'_, AppState>,
    sample_rate: u32,
) -> Result<daw_party_analysis::Report, String> {
    validate_sample_rate(sample_rate)?;
    let inputs = render_inputs(&state);
    tauri::async_runtime::spawn_blocking(move || {
        // 分析只关心工程正文，不加尾音。
        let buffer = render_offline(inputs, sample_rate, 0.0);
        daw_party_analysis::analyze(buffer.data(), buffer.channels(), sample_rate as f64)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
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
    let inputs = render_inputs(&state);
    tauri::async_runtime::spawn_blocking(move || {
        let buffer = render_offline(inputs, sample_rate, 0.5);
        daw_party_engine::wav::write_wav(&path, &buffer, sample_rate).map_err(|e| e.to_string())?;
        Ok::<String, String>(path)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// 音频素材（导入 / 波形）
// ---------------------------------------------------------------------------

/// 把素材的**自然时长**（秒）换算成 `start` 处的拍跨度。
///
/// 片段的「拍长」是时间轴上的唯一真相；导入时让它正好等于自然时长，于是比值 ≈ 1（不变调）。
/// 之后用户拖宽窄或改工程速度才会按比例重采样（变调）。
pub(crate) fn natural_length_beats(project: &Project, start: f64, seconds: f64) -> f64 {
    let map = TempoMap::from_transport(&project.transport);
    let start_sec = map.beats_to_seconds(start);
    map.seconds_to_beats(start_sec + seconds) - start
}

/// 导入一个音频文件为音频片段（wav / flac / mp3 / ogg-vorbis / aac / m4a）。
///
/// 解码放到阻塞线程池：它既慢又可能失败，绝不能占住命令线程。
#[tauri::command]
pub async fn import_audio(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    track: u64,
    start: f64,
) -> Result<Project, String> {
    let assets = Arc::clone(&state.assets);
    let path_for_load = path.clone();
    let audio = tauri::async_runtime::spawn_blocking(move || assets.load(&path_for_load))
        .await
        .map_err(|e| e.to_string())??;

    let seconds = audio.frames() as f64 / audio.sample_rate;
    if !(seconds.is_finite() && seconds > 0.0) {
        return Err(format!("音频素材没有可用长度：{path}"));
    }
    let start = daw_party_engine::model::Clip::normalize_start(start);
    let length = {
        let engine = state.engine.lock().unwrap();
        natural_length_beats(engine.project(), start, seconds)
    };
    Ok(run(
        &app,
        &state,
        Command::AddAudioClip {
            track: TrackId(track),
            start,
            length,
            source: AudioSource { path, offset_frames: 0 },
        },
    ))
}

/// 取某个音频文件的波形包络（每桶 min/max 各一个值，长度 = `buckets * 2`），供界面绘制。
#[tauri::command]
pub async fn audio_waveform(
    state: State<'_, AppState>,
    path: String,
    buckets: usize,
) -> Result<Vec<f32>, String> {
    let assets = Arc::clone(&state.assets);
    tauri::async_runtime::spawn_blocking(move || assets.waveform(&path, buckets))
        .await
        .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// 插件：目录、参数、效果链
// ---------------------------------------------------------------------------

/// 已扫描到的外部插件目录（界面把它与内置音色合并展示）。
#[tauri::command]
pub fn list_plugins(state: State<'_, AppState>) -> Vec<CatalogPlugin> {
    state.catalog_snapshot()
}

/// 重新扫描本机插件（慢，放到阻塞线程池）；完成后广播事件。
#[tauri::command]
pub async fn rescan_plugins(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<CatalogPlugin>, String> {
    let catalog = tauri::async_runtime::spawn_blocking(daw_party_host_vst3::scan_catalog)
        .await
        .map_err(|e| e.to_string())?;
    state.install_catalog(catalog.clone());
    let _ = app.emit(EVENT_PLUGINS, &catalog);
    Ok(catalog)
}

/// 取某个插件的参数表（内置插件从注册表拿，外部插件按目录加载）。
#[tauri::command]
pub async fn plugin_params(
    state: State<'_, AppState>,
    plugin_id: String,
) -> Result<Vec<ParamInfo>, String> {
    let catalog = state.catalog_snapshot();
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(plugin) = PluginRegistry::with_builtins().create(&plugin_id, TrackId(0)) {
            return Ok(plugin.params());
        }
        let entry = catalog
            .into_iter()
            .find(|e| e.id == plugin_id)
            .ok_or_else(|| format!("未知插件：{plugin_id}"))?;
        let plugin = daw_party_host_vst3::load_catalog_plugin(&entry).map_err(|e| e.to_string())?;
        Ok(plugin.params())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 在轨道效果链末尾追加一个效果器。
#[tauri::command]
pub fn add_effect(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    plugin_id: String,
) -> Project {
    run(&app, &state, Command::AddEffect { track: TrackId(track), plugin_id })
}

/// 从轨道效果链移除一个效果器（按链内序号）。
#[tauri::command]
pub fn remove_effect(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    index: usize,
) -> Project {
    run(&app, &state, Command::RemoveEffect { track: TrackId(track), index })
}

/// 调整效果器在链中的位置。
#[tauri::command]
pub fn move_effect(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    from: usize,
    to: usize,
) -> Project {
    run(&app, &state, Command::MoveEffect { track: TrackId(track), from, to })
}

/// 设置效果器参数（参数 id 由插件自身定义）。
#[tauri::command]
pub fn set_effect_param(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    index: usize,
    param: String,
    value: f32,
) -> Project {
    run(&app, &state, Command::SetEffectParam { track: TrackId(track), index, param, value })
}

// ---------------------------------------------------------------------------
// 插件原生界面（独立窗口）
// ---------------------------------------------------------------------------

/// 宿主给「只能嵌入」的插件开的那扇窗的 label。
pub fn host_window_label(track: u64) -> String {
    format!("plugin-ui-{track}")
}

/// [`host_window_label`] 的反函数：从窗口 label 认回轨道号。
pub fn host_window_track(label: &str) -> Option<u64> {
    label.strip_prefix("plugin-ui-")?.parse().ok()
}

/// 建一扇宿主窗口来装「只能嵌入」插件的界面。
///
/// 用一扇普通的窗口 + **空白路由**：插件把自己的视图挂到窗口的内容视图上（盖在 webview
/// 之上），背后那层网页是空的，所以不会透出应用界面。
///
/// **标题栏样式必须显式指定**：Tauri 对 `TitleBarStyle::Visible`（也是新窗口的默认值）会额外
/// 打开 `fullsize_content_view`——内容视图一直铺到标题栏下面。对网页视图无所谓，但插件的
/// 原生视图会因此**盖住红绿灯**、插件自己的顶部栏也被顶到标题栏区域里。`Transparent` 是
/// 唯一不会打开该标志、又保留原生标题栏的选项。
///
/// 已经开着就复用（插件的尺寸由它自己定，窗口跟着走）。
fn build_host_window(
    app: &tauri::AppHandle,
    track: u64,
    title: &str,
    width: u32,
    height: u32,
) -> Result<tauri::WebviewWindow, String> {
    let label = host_window_label(track);
    if let Some(existing) = app.get_webview_window(&label) {
        return Ok(existing);
    }
    let mut builder = tauri::WebviewWindowBuilder::new(
        app,
        &label,
        tauri::WebviewUrl::App("index.html#/blank".into()),
    )
    .title(title)
    .decorations(true)
    .inner_size(width as f64, height as f64);

    #[cfg(target_os = "macos")]
    {
        builder = builder
            .title_bar_style(tauri::TitleBarStyle::Transparent)
            .hidden_title(false);
    }

    builder.build().map_err(|e| e.to_string())
}

/// 取窗口的原生视图句柄，交给插件 `set_parent`。
#[cfg(target_os = "macos")]
fn native_view_handle(window: &tauri::WebviewWindow) -> Option<usize> {
    // CLAP 的 cocoa 后端要的是 NSView（这里是窗口的内容视图）。
    window.ns_view().ok().map(|view| view as usize)
}

#[cfg(target_os = "windows")]
fn native_view_handle(window: &tauri::WebviewWindow) -> Option<usize> {
    // win32 后端要的是 HWND。
    window.hwnd().ok().map(|hwnd| hwnd.0 as usize)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn native_view_handle(_window: &tauri::WebviewWindow) -> Option<usize> {
    // X11 要 gdk 拿 window id，还没接。
    None
}

/// 为某条轨道打开插件的原生界面。
///
/// 窗口必须在主线程创建（macOS 的 `NSWindow` 只能在那里建），所以走 `run_on_main_thread`，
/// 并把结果通过 channel 带回来。
#[tauri::command]
pub fn open_plugin_window(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
) -> Result<(), String> {
    if state.open_editors.lock().unwrap().contains(&track) {
        return Ok(());
    }
    let handle = state
        .editor_handle(track)
        .ok_or_else(|| "这条轨道没有可打开的插件界面".to_string())?;

    // 窗口标题用插件的展示名（目录里有就用目录名，否则退回插件 id）。
    let plugin_id = {
        let engine = state.engine.lock().unwrap();
        engine
            .project()
            .track(TrackId(track))
            .and_then(|t| t.instrument.as_ref())
            .map(|i| i.plugin_id.clone())
            .unwrap_or_default()
    };
    let title = state
        .catalog_snapshot()
        .into_iter()
        .find(|e| e.id == plugin_id)
        .map(|e| e.name)
        .unwrap_or(plugin_id);

    let (tx, rx) = std::sync::mpsc::channel();
    let app_for_main = app.clone();
    app.run_on_main_thread(move || {
        // 问插件：它的界面需要宿主提供一扇窗吗？
        // KR-106 的 CLAP 就是这种——它的 GUI 只支持「嵌入」，得我们给它一个原生视图。
        let host_size = handle.host_window_size();
        let host_window = match host_size {
            Some((width, height)) => {
                match build_host_window(&app_for_main, track, &title, width, height) {
                    Ok(window) => Some(window),
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        return;
                    }
                }
            }
            None => None,
        };
        let host_view = host_window.as_ref().and_then(native_view_handle);
        if host_size.is_some() && host_view.is_none() {
            // 平台还没接（Linux 的 X11 窗口 id）：把刚建的窗收掉，把原因说清楚。
            if let Some(window) = host_window {
                let _ = window.close();
            }
            let _ = tx.send(Err("该插件的界面需要宿主窗口，当前平台还没接上".to_string()));
            return;
        }
        // 嵌入模式下插件会自己决定界面多大——它通过 `request_resize` 告诉我们，我们改窗口。
        if let Some(window) = host_window.clone() {
            handle.set_resize_handler(Box::new(move |width, height| {
                let _ = window.set_size(tauri::LogicalSize::new(width as f64, height as f64));
            }));
        }

        let result = match handle.open(&title, host_view) {
            Ok(window) => {
                with_editor_windows(|windows| {
                    windows.insert(track, window);
                });
                let state = app_for_main.state::<AppState>();
                state.open_editors.lock().unwrap().insert(track);
                if host_size.is_some() {
                    state.host_windows.lock().unwrap().insert(track);
                }
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        };
        let _ = tx.send(result);
    })
    .map_err(|e| e.to_string())?;

    rx.recv().map_err(|e| e.to_string())?
}

/// 关闭某条轨道的插件窗口。
#[tauri::command]
pub fn close_plugin_window(app: tauri::AppHandle, track: u64) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let app_for_main = app.clone();
    app.run_on_main_thread(move || {
        let window = with_editor_windows(|windows| windows.remove(&track));
        if let Some(window) = window {
            window.close();
        }
        let state = app_for_main.state::<AppState>();
        state.open_editors.lock().unwrap().remove(&track);
        // 嵌入模式的界面住在我们建的宿主窗口里，一并收掉。
        if state.host_windows.lock().unwrap().remove(&track) {
            if let Some(window) = app_for_main.get_webview_window(&host_window_label(track)) {
                let _ = window.close();
            }
        }
        let _ = tx.send(());
    })
    .map_err(|e| e.to_string())?;

    let _ = rx.recv();
    Ok(())
}

/// 该轨道是否开着插件窗口（用户自己点关闭按钮后会变回 false）。
#[tauri::command]
pub fn plugin_window_open(state: State<'_, AppState>, track: u64) -> bool {
    state.open_editors.lock().unwrap().contains(&track)
}

/// 打开内置合成器的**独立窗口**。
///
/// 内置合成器没有原生编辑器，但它的面板也不该只能挤在侧边栏里——给它一个自己的窗口，
/// 「打开界面」这个动作对所有音色就都成立。已经开着就聚焦过去。
#[tauri::command]
pub async fn open_synth_window(app: tauri::AppHandle, track: u64) -> Result<(), String> {
    let label = format!("synth-{track}");
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.set_focus();
        return Ok(());
    }

    // 建窗口必须在主线程上做。
    let (tx, rx) = std::sync::mpsc::channel();
    let app_for_main = app.clone();
    app.run_on_main_thread(move || {
        let result = tauri::WebviewWindowBuilder::new(
            &app_for_main,
            &label,
            // 同一个前端，换一条路由：窗口里只有合成器本体。
            tauri::WebviewUrl::App(format!("index.html#/synth?track={track}").into()),
        )
        .title("内置合成器")
        // 面板是固定尺寸的设备（900×560），窗口按「标题栏 + 面板」留够地方，并锁死尺寸：
        // 拖动窗口不会改变布局，键盘位置因此永远稳定。
        .inner_size(960.0, 700.0)
        .resizable(false)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string());
        let _ = tx.send(result);
    })
    .map_err(|e| e.to_string())?;

    rx.recv().map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// 实时波形与试听（合成器界面用的那条实时通道）
// ---------------------------------------------------------------------------

/// 取最近 `points` 个输出采样，供界面画实时波形。
#[tauri::command]
pub fn scope_data(state: State<'_, AppState>, points: usize) -> Vec<f32> {
    state.audio.scope_snapshot(points)
}

/// 试听一个音（合成器界面里的键盘）：不写工程、不进撤销历史、不等传输。
#[tauri::command]
pub fn audition_note(state: State<'_, AppState>, track: u64, note: u8, velocity: u8, on: bool) {
    state.audio.audition_note(TrackId(track), note, velocity, on);
}

// ---------------------------------------------------------------------------
// 用户预设（快速切音色）
// ---------------------------------------------------------------------------

/// 全部用户预设（跨工程）。
#[tauri::command]
pub fn list_presets(state: State<'_, AppState>) -> Vec<Preset> {
    state.presets.lock().unwrap().all()
}

/// 把「某个插件当前的参数快照」存成一条预设。
#[tauri::command]
pub fn save_preset(
    state: State<'_, AppState>,
    name: String,
    plugin_id: String,
    params: HashMap<String, f32>,
) -> Result<Preset, String> {
    state
        .presets
        .lock()
        .unwrap()
        .create(&name, &plugin_id, params)
}

/// 覆盖一条已存在的预设（改名或更新参数快照）。
#[tauri::command]
pub fn update_preset(
    state: State<'_, AppState>,
    id: String,
    name: String,
    params: HashMap<String, f32>,
) -> Result<Preset, String> {
    state.presets.lock().unwrap().update(&id, &name, params)
}

/// 删除一条预设。
#[tauri::command]
pub fn delete_preset(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.presets.lock().unwrap().remove(&id)
}
