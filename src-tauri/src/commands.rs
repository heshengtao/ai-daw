use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tauri::{Emitter, Manager, State};

use ai_daw_engine::audio::{AudioBuffer, DecodedAudio};
use ai_daw_engine::bus::Engine;
use ai_daw_engine::command::Command;
use ai_daw_engine::event::{Actor, Event};
use ai_daw_engine::model::{
    AudioSource, AutomationPoint, BeatFilter, ClipId, ClipPlacement, ClipRef, FitMode, HarmonyBar,
    Keyframe, Note, NoteEdit, NotePatch, NoteRange, PitchRange, Project, Section, Send,
    StrumDirection, TrackId, Transport, VelocityShape,
};
use ai_daw_engine::registry::{PluginRegistry, PluginSlot};
use ai_daw_engine::render::{render_with_assets_progress, RenderOptions};
use ai_daw_engine::tempo::TempoMap;
use ai_daw_engine::transport::TransportCommand;
use ai_daw_engine::wav::wav_bytes;
use ai_daw_host_vst3::{CatalogPlugin, EditorHandle};
use ai_daw_plugin_api::ParamInfo;

use crate::audio::{EditorSlot, TransportSnapshot};
use crate::plugins::{build_registry_for_offline_render, uses_external_plugins, LoadWarnings};
use crate::presets::Preset;
use crate::sound_memories::{MemoryMeta, PluginSnapshot, SoundMemory, SoundMixer, SoundSnapshot};
use crate::state::{with_editor_windows, AppState, EVENT_ENGINE, EVENT_EXPORT, EVENT_IMPORT};

/// 撤销/重做之后返回的历史状态（让界面能立刻更新按钮可用性）。
#[derive(serde::Serialize)]
pub struct HistoryState {
    pub project: Project,
    pub undo_depth: usize,
    pub redo_depth: usize,
    /// 是否有未保存的改动。
    pub dirty: bool,
}

/// 当前历史深度与是否未保存。
#[derive(serde::Serialize)]
pub struct HistoryDepths {
    pub undo: usize,
    pub redo: usize,
    pub dirty: bool,
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
    run_all(app, state, actor, vec![command])
}

/// 执行一批命令（**合成一个撤销步骤**）：应用状态、广播事件、把变更同步到音频线程。
fn run_all(
    app: &tauri::AppHandle,
    state: &State<'_, AppState>,
    actor: Actor,
    commands: Vec<Command>,
) -> (Project, Vec<Event>) {
    let (mut project, events) = {
        let mut engine = state.engine.lock().unwrap();
        let events: Vec<Event> = engine.execute_all(commands, actor);
        (engine.project().clone(), events)
    };
    for event in &events {
        let _ = app.emit(EVENT_ENGINE, event);
    }
    let structural = events.iter().any(is_structural);
    if structural {
        // 结构性变更会重建播放器、连带重建插件实例。**先**把实例里的插件状态抢救进工程，
        // 再交给音频线程重建：重建后的实例据此还原，于是改效果链 / 移动片段这类结构变更
        // 不会再重置乐器的配置。抢救后要重新取一份工程快照（状态已写进去）。
        capture_plugin_states(state);
        project = state.engine.lock().unwrap().project().clone();
    }
    state.audio.sync_events(&project, &events);
    // 结构性变更会重建播放器，但插件实例会被**搬到新播放器复用**，所以只有「乐器真的被换掉
    // / 轨道被删」的那些窗口才失效、才需要关——其余照旧开着。
    if structural {
        close_stale_plugin_windows(app, state);
    }
    (project, events)
}

/// 把「活着的插件实例」的内部状态抢救进工程。
///
/// 外部插件（VST3 / CLAP）的旋钮与预设只存在于实例内部，重建播放器就会丢。宿主在重建
/// **之前**调用它，把状态写进 `PluginInstance::state`；重建后的实例启动时会 `load_state`
/// 还原。于是「改效果链 / 移动片段」这类结构性变更不会再让乐器或效果器回到默认配置。
///
/// 只处理**登记过编辑器句柄**的实例——句柄是宿主与实例共享的那份引用，没有句柄就无从读取。
/// 读取失败返回空，会被忽略：绝不用空状态覆盖已有的好状态。
///
/// **效果器按 plugin id 回写、而不是按链内下标**：结构性变更（增删 / 移动效果器）会在写回
/// 之前就把下标挪了位，按键里的下标写回会串味。同一条轨道上出现多个同名效果器时，按出现
/// 次序一一对应。
pub(crate) fn capture_plugin_states(state: &AppState) {
    let entries: Vec<((u64, PluginSlot), EditorHandle, String)> = {
        let handles = state.editor_handles.lock().unwrap();
        handles
            .iter()
            .map(|(key, entry)| (*key, entry.handle.clone(), entry.plugin_id.clone()))
            .collect()
    };
    // `save_state` 可能派发到主线程，别在持锁时做。
    let captured: Vec<((u64, PluginSlot), String, Vec<u8>)> = entries
        .into_iter()
        .filter_map(|(key, handle, plugin_id)| {
            let bytes = handle.save_state();
            (!bytes.is_empty()).then_some((key, plugin_id, bytes))
        })
        .collect();
    if captured.is_empty() {
        return;
    }

    let mut engine = state.engine.lock().unwrap();
    // 各轨道当前效果链的 plugin id 序列（按下标），供效果器按 id 定位。
    let chain_ids: HashMap<u64, Vec<String>> = engine
        .project()
        .tracks
        .iter()
        .map(|t| (t.id.0, t.effects.iter().map(|fx| fx.plugin_id.clone()).collect()))
        .collect();
    // (轨道, plugin id) → 本次已匹配到的次数，用于消歧同名效果器。
    let mut cursor: HashMap<(u64, String), usize> = HashMap::new();
    for ((track, slot), plugin_id, bytes) in captured {
        match slot {
            // 带上**句柄自己的** plugin id：引擎会核对它是否还是轨道当前的乐器。
            // 换乐器时句柄要等这一轮重建后才清理，不核对就会把旧乐器的状态写进新乐器。
            PluginSlot::Instrument => engine.set_instrument_state(TrackId(track), &plugin_id, bytes),
            PluginSlot::Effect(_) => {
                let Some(ids) = chain_ids.get(&track) else { continue };
                let start = *cursor.get(&(track, plugin_id.clone())).unwrap_or(&0);
                if let Some(rel) = ids.iter().skip(start).position(|pid| *pid == plugin_id) {
                    let index = start + rel;
                    cursor.insert((track, plugin_id), index + 1);
                    engine.set_effect_state(TrackId(track), index, bytes);
                }
            }
        }
    }
}

/// 只把引擎当前状态写进历史；调用方负责先抢救插件状态。
///
/// 历史是「尽力而为」：拿不到配置目录、写盘失败都只打一行日志，绝不阻断保存 / 新建 / 打开。
fn remember_snapshot(app: &tauri::AppHandle, state: &AppState) {
    let Some(dir) = crate::history_dir(app) else {
        return;
    };
    let (project, saved) = {
        let engine = state.engine.lock().unwrap();
        (engine.project().clone(), engine.saved().clone())
    };
    if let Err(err) = crate::history::record(&dir, &project, &saved, state.session_path()) {
        eprintln!("记录历史工程失败：{err}");
    }
}

/// 把当前工程收进历史（工程即将被替换时调用）。
///
/// 外部插件的内部状态也要先抢救进工程，否则历史里的那份会丢掉插件补丁。
fn remember_current(app: &tauri::AppHandle, state: &AppState) {
    capture_plugin_states(state);
    remember_snapshot(app, state);
}

/// 覆盖已有工程文件前，把文件里的旧版本收进历史。
///
/// 这样「保存坏了的一版」不会把上一版顶掉——旧版还能从历史卡片里找回来。
/// 文件不存在、读不出来或不是合法工程都只是跳过（历史不能妨碍保存本身）。
fn remember_file_version(app: &tauri::AppHandle, path: &str) {
    let Some(dir) = crate::history_dir(app) else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(project) = serde_json::from_str::<Project>(&text) else {
        return;
    };
    let saved = project.clone();
    if let Err(err) = crate::history::record(&dir, &project, &saved, Some(path.to_string())) {
        eprintln!("记录旧版工程失败：{err}");
    }
}

/// 应用退出兜底：会话缓存已经落盘、插件状态也抢救过了，这里再记一份历史。
///
/// 程序坞「退出」/ Cmd+Q 不会走前端的关闭守卫，只能靠后端这一下。
pub(crate) fn remember_before_exit(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    remember_snapshot(app, &state);
}

/// 界面走的编辑入口（署名 `Actor::Human`）。
fn run(app: &tauri::AppHandle, state: &State<'_, AppState>, command: Command) -> Project {
    run_as(app, state, Actor::Human, command).0
}

/// MCP 服务器走的编辑入口：署名 `Actor::Agent("mcp")`，其余管线与界面**完全一致**。
///
/// 返回新工程与本次产生的事件，便于工具把手边新对象的 id 回给代理。
pub(crate) fn run_agent(app: &tauri::AppHandle, command: Command) -> (Project, Vec<Event>) {
    run_agent_as(app, Actor::Agent("mcp".into()), command)
}

/// 以**任意代理身份**执行单条命令：署名由 `actor` 决定（MCP 用 `"mcp"`，内置写歌智能体用 `"chat"`）。
///
/// 与 [`run_agent`] 同一条管线，只是署名不同——事件流与撤销历史因此能区分是哪个代理在写。
pub(crate) fn run_agent_as(
    app: &tauri::AppHandle,
    actor: Actor,
    command: Command,
) -> (Project, Vec<Event>) {
    let state = app.state::<AppState>();
    run_as(app, &state, actor, command)
}

/// 以代理身份执行**一批**命令（**合成一个撤销步骤**）：用于一次产出多条轨道的 AI 动作
/// （如音源分离一次生成鼓/贝斯/其他/人声四条分轨），一次撤销即可整体回退。
pub(crate) fn run_agent_all(app: &tauri::AppHandle, commands: Vec<Command>) -> (Project, Vec<Event>) {
    run_agent_all_as(app, Actor::Agent("mcp".into()), commands)
}

/// 同 [`run_agent_all`]，但由调用方决定署名（MCP 与内置写歌智能体都能用）。
pub(crate) fn run_agent_all_as(
    app: &tauri::AppHandle,
    actor: Actor,
    commands: Vec<Command>,
) -> (Project, Vec<Event>) {
    let state = app.state::<AppState>();
    run_all(app, &state, actor, commands)
}

/// 渲染一份快照所需的全部输入。
///
/// **音频素材在这里（命令线程）就解码好**：渲染与音频回调都不做任何 IO。
/// 缺失的素材会被跳过 → 该片段静音。
pub(crate) struct RenderInputs {
    pub project: Project,
    pub catalog: Vec<CatalogPlugin>,
    pub editor_handles: Arc<Mutex<HashMap<(u64, PluginSlot), EditorSlot>>>,
    pub samples: HashMap<String, Arc<DecodedAudio>>,
    /// 外部插件加载失败 / 崩溃的记录（渲染继续，坏轨道按静音处理）。
    pub warnings: LoadWarnings,
}

/// 取当前状态的可渲染快照（工程 + 插件目录 + 已解码素材）。
///
/// 只在锁内克隆工程，绝不把 `MutexGuard` 带过 `.await`。
///
/// **不带编辑器句柄表**：离线渲染（预览 / 导出 / 分析 / MCP）会为每条轨道**新建**一套插件实例，
/// 而插件工厂在创建实例时会把句柄登记进传进来的那张表。如果传的是实时播放那张表，
/// 刚渲染出来的**一次性实例**就会把正在发声的实例的句柄顶掉——之后「打开界面」开到的是一个
/// 不在音频图里的实例：界面能开、旋钮能动，但**听不到任何声音**。
pub(crate) fn render_inputs(state: &AppState) -> RenderInputs {
    let project = state.engine.lock().unwrap().project().clone();
    let samples = state.assets.snapshot_for(&project.audio_paths());
    RenderInputs {
        project,
        catalog: state.catalog_snapshot(),
        editor_handles: Arc::new(Mutex::new(HashMap::new())),
        samples,
        warnings: Arc::new(Mutex::new(Vec::new())),
    }
}

/// 离线渲染前留给**外部插件**的装载时间（秒）。
///
/// 采样流式音源（如 MusicLab RealGuitar）在 `prepare` 之后要在自己的线程里异步装载采样，这期间
/// 只出静音。实时播放感觉不到（它就按真实时间走）；离线渲染比实时快几十倍，同一段装载时间会被
/// 渲染成**几个小节**的静音——听感就是「导出文件开头两小节没声音」。
///
/// 实测本机 RealGuitar：等 0.4 s 仍有缺口、0.8 s 起就能从第一帧出声。取 1.5 s 留出余量（更重的
/// 采样库只会更慢，这个值兜不住它们，但至少不会把常见音源的开头吃掉）。代价只是一次渲染多等
/// 1.5 s，而且只在与外部插件有关时付。
const EXTERNAL_PLUGIN_SETTLE_SECONDS: f64 = 1.5;

/// 离线渲染一份快照（预览 / 导出 / 分析 / MCP 共用同一条路径）。
pub(crate) fn render_offline(inputs: RenderInputs, sample_rate: u32, tail_seconds: f64) -> AudioBuffer {
    render_offline_with_progress(inputs, sample_rate, tail_seconds, |_, _| {})
}

/// 同 [`render_offline`]，另外把渲染进度透给调用方（导出用它画进度条）。
pub(crate) fn render_offline_with_progress(
    inputs: RenderInputs,
    sample_rate: u32,
    tail_seconds: f64,
    on_progress: impl FnMut(usize, usize),
) -> AudioBuffer {
    let RenderInputs { project, catalog, editor_handles, samples, warnings } = inputs;
    // 离线渲染把 VST3 放进 helper 子进程：插件在加载 / 处理音频时崩溃只杀 helper，
    // 宿主记一条警告后继续（该轨道静音），不会再把整个应用带走。
    let registry = build_registry_for_offline_render(
        &catalog,
        &project,
        editor_handles,
        Arc::clone(&warnings),
    );
    let options = RenderOptions {
        sample_rate: sample_rate as f64,
        tail_seconds,
        // 用了外部插件才等：内置插件是纯 DSP，没有可装载的东西。
        settle_seconds: if uses_external_plugins(&project, &catalog) {
            EXTERNAL_PLUGIN_SETTLE_SECONDS
        } else {
            0.0
        },
    };
    let buffer = render_with_assets_progress(&project, &registry, &options, &samples, on_progress);

    // 预览 / 导出没有把警告回传界面的通道，至少写到 stderr（MCP 会把它们写进分析结果）。
    let warnings = warnings.lock().unwrap();
    let mut logged: Vec<&String> = Vec::new();
    for warning in warnings.iter() {
        if !logged.contains(&warning) {
            logged.push(warning);
            eprintln!("[离线渲染] {warning}");
        }
    }
    buffer
}

/// 该事件是否会导致播放器重建（混音与实时参数不会）。
fn is_structural(event: &Event) -> bool {
    !event.is_mix_only()
        && !matches!(event, Event::ParamChanged { .. } | Event::EffectParamChanged { .. })
}

/// 关掉指定槽位的插件窗口（须在主线程执行创建/销毁，所以走 `run_on_main_thread`）。
///
/// 只清**这几个槽位**的编辑器句柄：别处的实例会被重建**复用**，句柄仍然有效，不能一起清。
///
/// 清句柄**必须在主线程上做**：句柄可能正好是实例的最后一个引用，而释放会触发
/// `clap_plugin.destroy()`——那是主线程专属调用。
fn close_plugin_windows_for(app: &tauri::AppHandle, keys: Vec<(u64, PluginSlot)>) {
    if keys.is_empty() {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let app_for_main = app.clone();
    if app
        .run_on_main_thread(move || {
            let state = app_for_main.state::<AppState>();
            with_editor_windows(|windows| {
                for key in &keys {
                    // 走 `close()` 而不是直接丢：它会先 hide + destroy 插件的界面。
                    if let Some(window) = windows.remove(key) {
                        window.close();
                    }
                }
            });
            // 嵌入模式的宿主窗口也一并收掉，别留下一扇空窗。
            for key in &keys {
                if state.host_windows.lock().unwrap().remove(key) {
                    if let Some(window) =
                        app_for_main.get_webview_window(&host_window_label(key.0, key.1))
                    {
                        let _ = window.close();
                    }
                }
            }
            {
                let mut handles = state.editor_handles.lock().unwrap();
                for key in &keys {
                    handles.remove(key);
                }
            }
            {
                let mut open = state.open_editors.lock().unwrap();
                let mut meta = state.editor_plugins.lock().unwrap();
                for key in &keys {
                    open.remove(key);
                    meta.remove(key);
                }
            }
            let _ = tx.send(());
        })
        .is_ok()
    {
        let _ = rx.recv();
    }
}

/// 关掉**全部**插件窗口（整个工程被替换时用，例如打开另一个工程）。
fn close_all_plugin_windows(app: &tauri::AppHandle, state: &AppState) {
    let keys: Vec<(u64, PluginSlot)> =
        state.open_editors.lock().unwrap().iter().copied().collect();
    close_plugin_windows_for(app, keys);
}

/// 关掉**实例已经不在**的插件窗口，其余保持不动。
///
/// 重建播放器时插件实例会被搬进新播放器复用（见 `ReusableInstances`），所以只要某条轨道某
/// 个**槽位的插件没换**，它的窗口（以及插件内部状态）就照旧有效——**不要关**。只有插件被
/// 换掉、轨道/效果槽位被删、或整个工程被替换时，旧窗口才真的失效，必须关掉，否则窗口会指着
/// 一个已销毁的实例。
fn close_stale_plugin_windows(app: &tauri::AppHandle, state: &AppState) {
    let project = { state.engine.lock().unwrap().project().clone() };

    /// 某个槽位当前应当承载的 plugin id（轨道/槽位不存在时返回 None）。
    fn slot_plugin_id(project: &Project, track: u64, slot: PluginSlot) -> Option<String> {
        let t = project.track(TrackId(track))?;
        match slot {
            PluginSlot::Instrument => t.instrument.as_ref().map(|i| i.plugin_id.clone()),
            PluginSlot::Effect(index) => t.effects.get(index).map(|fx| fx.plugin_id.clone()),
        }
    }

    let mut stale: Vec<(u64, PluginSlot)> = {
        let opened = state.editor_plugins.lock().unwrap().clone();
        opened
            .into_iter()
            .filter(|((track, slot), opened_for)| {
                slot_plugin_id(&project, *track, *slot)
                    .map(|current| current != *opened_for)
                    .unwrap_or(true) // 轨道或槽位已经不在了
            })
            .map(|(key, _)| key)
            .collect()
    };
    // 已删除槽位遗留的编辑器句柄也一并收掉，免得实例（连同它的界面）迟迟不释放。
    {
        let handles = state.editor_handles.lock().unwrap();
        for key in handles.keys() {
            if slot_plugin_id(&project, key.0, key.1).is_none() && !stale.contains(key) {
                stale.push(*key);
            }
        }
    }
    close_plugin_windows_for(app, stale);
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

/// 改速度 / 拍号后，把每个**音频片段**的「拍长」重算成它在新速度下的**原真实时长**。
///
/// 引擎按 `length`（拍）画方框、素材按 `rate` 原速播放——**改 BPM 不改音频**，但方框会与素材
/// 错位。这里让方框秒数保持不变（= 素材的真实秒数），于是方框始终覆盖素材、音频原速、网格随
/// 速度走。返回需要追加的 `ResizeClip`（与速度命令同一批 → 一条撤销）。
pub(crate) fn resize_audio_clips_for_transport(
    project: &Project,
    old: &Transport,
    new: &Transport,
) -> Vec<Command> {
    if old == new {
        return Vec::new();
    }
    let old_map = TempoMap::from_transport(old);
    let new_map = TempoMap::from_transport(new);
    let mut out = Vec::new();
    for track in &project.tracks {
        for clip in &track.clips {
            if clip.audio.is_none() {
                continue;
            }
            let span_sec = old_map.beats_to_seconds(clip.start + clip.length)
                - old_map.beats_to_seconds(clip.start);
            if span_sec <= 1e-9 {
                continue;
            }
            let start_sec = new_map.beats_to_seconds(clip.start);
            let length = new_map.seconds_to_beats(start_sec + span_sec) - clip.start;
            if length > 1e-6 && (length - clip.length).abs() > 1e-6 {
                out.push(Command::ResizeClip { track: track.id, clip: clip.id, length });
            }
        }
    }
    out
}

/// 走 `run_all`（**一条撤销**）：先发 `command`，再把音频片段的方框重算到 `new_transport`。
fn run_tempo(
    app: &tauri::AppHandle,
    state: &State<'_, AppState>,
    actor: Actor,
    command: Command,
    new_transport: Transport,
) -> Project {
    let project = state.engine.lock().unwrap().project().clone();
    let old = project.transport.clone();
    let mut commands = vec![command];
    commands.extend(resize_audio_clips_for_transport(&project, &old, &new_transport));
    run_all(app, state, actor, commands).0
}

/// 设置速度（BPM）。
#[tauri::command]
pub fn set_tempo(app: tauri::AppHandle, bpm: f64) -> Project {
    set_tempo_as(&app, Actor::Human, bpm)
}

/// 设置速度的共用实现（界面命令与 MCP 工具共用）。
///
/// 音频片段的「拍长」会跟着重算——与界面改 BPM 的行为完全一致，否则导出会与工程对不上。
pub(crate) fn set_tempo_as(app: &tauri::AppHandle, actor: Actor, bpm: f64) -> Project {
    let state = app.state::<AppState>();
    let bpm = if bpm.is_finite() { bpm.clamp(1.0, 1000.0) } else { 120.0 };
    let mut t = state.engine.lock().unwrap().project().transport.clone();
    t.tempo = bpm;
    run_tempo(app, &state, actor, Command::SetTempo(bpm), t)
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

/// 应用一条预设的**内核**：一次事务写回整组乐器参数 + 整条效果链（**替换**效果链）。
///
/// 这是给 **MCP 工具**用的：代理可以显式要求「连效果链一起换」。界面侧的「切音色预设」
/// 不走这里——音色与效果已解耦，切音色只改参数（`set_params`），不碰效果链。
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

/// 应用一条预设（代理入口）：取状态 → 执行 → 广播 → 同步音频线程；署名由 `actor` 决定。
pub(crate) fn apply_preset_agent_as(
    app: &tauri::AppHandle,
    actor: Actor,
    track: TrackId,
    params: &HashMap<String, f32>,
    effects: &[EffectSpec],
) -> (Project, Vec<Event>) {
    let state = app.state::<AppState>();
    let (project, events) = apply_preset_inner(&state, track, params, effects, actor);
    for event in &events {
        let _ = app.emit(EVENT_ENGINE, event);
    }
    state.audio.sync_events(&project, &events);
    (project, events)
}

/// 从一条轨道抓音色快照：乐器 + 有序效果链 + 增益/声像。
///
/// 抓之前先抢救外部插件编辑器里的二进制状态，确保存下来的是用户**听到的**那份。
pub(crate) fn sound_snapshot_for(
    state: &AppState,
    track: TrackId,
) -> Result<SoundSnapshot, String> {
    capture_plugin_states(state);
    let engine = state.engine.lock().unwrap();
    let project = engine.project();
    let t = project
        .tracks
        .iter()
        .find(|t| t.id == track)
        .ok_or_else(|| format!("轨道 {} 不存在", track.0))?;
    Ok(SoundSnapshot {
        instrument: t.instrument.as_ref().map(PluginSnapshot::from_instance),
        effects: t.effects.iter().map(PluginSnapshot::from_instance).collect(),
        mixer: SoundMixer {
            gain_db: t.mixer.gain_db,
            pan: t.mixer.pan,
        },
        source_project: Some(project.name.clone()),
        role: t.meta.as_ref().and_then(|m| m.role.clone()),
    })
}

/// 应用一条音色记忆的**内核**：一次事务写回乐器、整条效果链、增益/声像。
///
/// 效果链是**替换**而不是叠加。插件二进制状态在事务后补写：`set_*_state` 不产生事件，
/// 因此不进入撤销/重做重放——这是引擎现状，内置合成器/效果器只用参数，不受影响。
pub(crate) fn apply_sound_memory_inner(
    state: &AppState,
    track: TrackId,
    memory: &SoundMemory,
    actor: Actor,
) -> Result<(Project, Vec<Event>), String> {
    capture_plugin_states(state);
    let mut engine = state.engine.lock().unwrap();
    if !engine.project().tracks.iter().any(|t| t.id == track) {
        return Err(format!("轨道 {} 不存在", track.0));
    }
    let existing = engine
        .project()
        .tracks
        .iter()
        .find(|t| t.id == track)
        .map_or(0, |t| t.effects.len());

    let mut commands: Vec<Command> = Vec::new();
    match &memory.instrument {
        Some(inst) => {
            commands.push(Command::SetTrackInstrument {
                track,
                plugin_id: inst.plugin_id.clone(),
            });
            for (param, value) in &inst.params {
                if value.is_finite() {
                    commands.push(Command::SetParam {
                        track,
                        param: param.clone(),
                        value: *value,
                    });
                }
            }
        }
        None => commands.push(Command::SetTrackInstrument {
            track,
            plugin_id: String::new(),
        }),
    }

    for _ in 0..existing {
        commands.push(Command::RemoveEffect { track, index: 0 });
    }
    for (index, fx) in memory.effects.iter().enumerate() {
        commands.push(Command::AddEffect {
            track,
            plugin_id: fx.plugin_id.clone(),
        });
        for (param, value) in &fx.params {
            if value.is_finite() {
                commands.push(Command::SetEffectParam {
                    track,
                    index,
                    param: param.clone(),
                    value: *value,
                });
            }
        }
    }

    if memory.mixer.gain_db.is_finite() {
        commands.push(Command::SetGain {
            track,
            gain_db: memory.mixer.gain_db,
        });
    }
    if memory.mixer.pan.is_finite() {
        commands.push(Command::SetPan {
            track,
            pan: memory.mixer.pan,
        });
    }

    let events = engine.execute_all(commands, actor);

    if let Some(inst) = &memory.instrument {
        let bytes = inst.state_bytes();
        if !bytes.is_empty() {
            engine.set_instrument_state(track, &inst.plugin_id, bytes);
        }
    }
    for (index, fx) in memory.effects.iter().enumerate() {
        let bytes = fx.state_bytes();
        if !bytes.is_empty() {
            engine.set_effect_state(track, index, bytes);
        }
    }

    Ok((engine.project().clone(), events))
}

/// 应用一条音色记忆（代理入口）：执行 → 广播 → 同步音频线程。
pub(crate) fn apply_sound_memory_agent_as(
    app: &tauri::AppHandle,
    actor: Actor,
    track: TrackId,
    memory: &SoundMemory,
) -> Result<(Project, Vec<Event>), String> {
    let state = app.state::<AppState>();
    let (project, events) = apply_sound_memory_inner(&state, track, memory, actor)?;
    for event in &events {
        let _ = app.emit(EVENT_ENGINE, event);
    }
    state.audio.sync_events(&project, &events);
    Ok((project, events))
}

/// 音色记忆库变更后通知前端刷新。
pub(crate) fn notify_sound_memories(app: &tauri::AppHandle) {
    let _ = app.emit(crate::state::EVENT_SOUND_MEMORIES, ());
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
    beat: f64,
    tempo: f64,
    num: u8,
    den: u8,
) -> Project {
    set_keyframe_as(&app, Actor::Human, beat, tempo, num, den)
}

/// 写入速度关键点的共用实现（界面命令与 MCP 工具共用）。
pub(crate) fn set_keyframe_as(
    app: &tauri::AppHandle,
    actor: Actor,
    beat: f64,
    tempo: f64,
    num: u8,
    den: u8,
) -> Project {
    let state = app.state::<AppState>();
    let mut t = state.engine.lock().unwrap().project().transport.clone();
    if let Some(kf) = Transport::normalize_keyframe(Keyframe {
        beat,
        tempo,
        time_sig_num: num,
        time_sig_den: den,
    }) {
        t.upsert_keyframe(kf);
    }
    run_tempo(app, &state, actor, Command::SetKeyframe { beat, tempo, num, den }, t)
}

/// 删除第 `beat` 拍的关键点。
#[tauri::command]
pub fn remove_keyframe(app: tauri::AppHandle, beat: f64) -> Project {
    remove_keyframe_as(&app, Actor::Human, beat)
}

/// 删除速度关键点的共用实现（界面命令与 MCP 工具共用）。
pub(crate) fn remove_keyframe_as(app: &tauri::AppHandle, actor: Actor, beat: f64) -> Project {
    let state = app.state::<AppState>();
    let mut t = state.engine.lock().unwrap().project().transport.clone();
    t.remove_keyframe(beat);
    run_tempo(app, &state, actor, Command::RemoveKeyframe { beat }, t)
}

/// 设置音频片段的**播放速度**（`rate`，1.0 = 原速）与**升降调**（半音，0 = 不变）。
///
/// 速度变了 → 素材真实时长变了 → 方框（拍长）也要跟着，否则方框与素材错位（与速度命令同一批，
/// 一条撤销）。升降调不改时长，只影响音高。
#[tauri::command]
pub fn set_clip_rate(
    app: tauri::AppHandle,
    track: u64,
    clip: u64,
    rate: f64,
    pitch_semitones: f64,
) -> Project {
    set_clip_rate_as(&app, Actor::Human, track, clip, rate, pitch_semitones)
}

/// 设置片段速度 / 升降调的共用实现（界面命令与 MCP 工具共用）。
pub(crate) fn set_clip_rate_as(
    app: &tauri::AppHandle,
    actor: Actor,
    track: u64,
    clip: u64,
    rate: f64,
    pitch_semitones: f64,
) -> Project {
    let state = app.state::<AppState>();
    let track_id = TrackId(track);
    let clip_id = ClipId(clip);
    let project = state.engine.lock().unwrap().project().clone();
    let rate = AudioSource::normalize_rate(rate);
    let mut commands = vec![Command::SetClipRate {
        track: track_id,
        clip: clip_id,
        rate,
        pitch_semitones,
    }];
    if let Some(c) = project.track(track_id).and_then(|t| t.clips.iter().find(|c| c.id == clip_id)) {
        if let Some(src) = &c.audio {
            if src.rate > 1e-9 {
                let map = TempoMap::from_transport(&project.transport);
                let span = map.beats_to_seconds(c.start + c.length) - map.beats_to_seconds(c.start);
                let target = span * (src.rate / rate);
                let start_sec = map.beats_to_seconds(c.start);
                let length = map.seconds_to_beats(start_sec + target) - c.start;
                if length > 1e-6 && (length - c.length).abs() > 1e-6 {
                    commands.push(Command::ResizeClip { track: track_id, clip: clip_id, length });
                }
            }
        }
    }
    run_all(app, &state, actor, commands).0
}

/// 裁剪 / 移动一个片段：`start` / `length` 是新的起点与拍长。
///
/// 音频片段按**资产采样率**把「拍」换算成源帧：拖左边缘推进 `offset_frames` 并缩短窗口，
/// 拖右边缘只改窗口长度——**不改速度、不变调**（速度 / 升降调是检查器里的独立参数）。
/// MIDI 片段只改起点 / 长度。同一批命令 → 一条撤销。
#[tauri::command]
pub fn trim_clip(
    app: tauri::AppHandle,
    track: u64,
    clip: u64,
    start: f64,
    length: f64,
) -> Result<Project, String> {
    trim_clip_as(&app, Actor::Human, track, clip, start, length)
}

/// 裁剪片段的共用实现（界面命令与 MCP 工具共用）。
pub(crate) fn trim_clip_as(
    app: &tauri::AppHandle,
    actor: Actor,
    track: u64,
    clip: u64,
    start: f64,
    length: f64,
) -> Result<Project, String> {
    let state = app.state::<AppState>();
    let track_id = TrackId(track);
    let clip_id = ClipId(clip);
    let project = state.engine.lock().unwrap().project().clone();
    let existing = project
        .track(track_id)
        .and_then(|t| t.clips.iter().find(|c| c.id == clip_id))
        .ok_or("片段不存在")?
        .clone();
    let start = ai_daw_engine::model::Clip::normalize_start(start);
    let length = ai_daw_engine::model::Clip::normalize_length(length);
    let mut commands: Vec<Command> = Vec::new();

    if let Some(src) = &existing.audio {
        let audio = state.assets.load(&src.path)?;
        let map = TempoMap::from_transport(&project.transport);
        // 方框起点移动了多少 = 素材跟着平移多少；窗口 = 方框对应的素材帧数。
        // 换算含片段速度 `rate`（见 `trimmed_window`）。
        let (offset, frames) = trimmed_window(
            &map,
            existing.start,
            start,
            length,
            src.offset_frames,
            audio.frames() as u64,
            audio.sample_rate,
            src.rate,
        );
        commands.push(Command::SetClipAudio {
            track: track_id,
            clip: clip_id,
            audio: Some(AudioSource {
                path: src.path.clone(),
                offset_frames: offset,
                frames: Some(frames),
                rate: src.rate,
                pitch_semitones: src.pitch_semitones,
            }),
        });
    } else if !existing.notes.is_empty() {
        // MIDI 片段同样统一为「收起 / 展开」：音符存的是**相对片段起点**的拍，直接挪起点会把
        // 音符一起拖走；这里把每个音符反向平移相同的量（停在**原本的时间线位置**）。
        // 落到方框左边（相对起点为负）的音符**保留不删**，由引擎按方框裁掉——往回拖即恢复。
        let shift = start - existing.start;
        if shift.abs() > 1e-9 {
            let entries: Vec<NoteEdit> = existing
                .notes
                .iter()
                .enumerate()
                .map(|(index, n)| NoteEdit {
                    index,
                    note: Note {
                        id: 0,
                        pitch: n.pitch,
                        start: n.start - shift,
                        duration: n.duration,
                        velocity: n.velocity,
                        volume: n.volume,
                        offset: 0.0,
                        legato: false, expr: ai_daw_engine::model::PitchExpression::NONE,
                    },
                })
                .collect();
            commands.push(Command::SetNotes { track: track_id, clip: clip_id, entries });
        }
    }
    commands.push(Command::MoveClip { track: track_id, clip: clip_id, start });
    commands.push(Command::ResizeClip { track: track_id, clip: clip_id, length });
    Ok(run_all(app, &state, actor, commands).0)
}

/// 片段能「展开」到的范围（拍）：左端 = 素材第一帧所在的位置，右端 = 素材最后一帧所在的位置。
#[derive(serde::Serialize)]
pub struct ClipBounds {
    /// 方框起点不能早于它。
    pub min_start: f64,
    /// 方框末尾不能晚于它。
    pub max_end: f64,
}

/// 左右拖拽的**夹取范围**：界面据此把边缘停在素材之内——「展开无法展开出原始的音频之外」。
///
/// **音频片段**用素材的实际首尾（左边缘往左拖最多露出素材第 0 帧，往右拖最多到素材最后一帧）。
/// **MIDI 片段是用户自定义的，没有素材边界**——长度随便改，所以不做夹取。
#[tauri::command]
pub fn clip_bounds(
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
) -> Result<ClipBounds, String> {
    let track_id = TrackId(track);
    let clip_id = ClipId(clip);
    let project = state.engine.lock().unwrap().project().clone();
    let c = project
        .track(track_id)
        .and_then(|t| t.clips.iter().find(|c| c.id == clip_id))
        .ok_or("片段不存在")?;
    let Some(src) = &c.audio else {
        // MIDI：不夹取（用一个足够大的有限值，避免 JSON 里的 Infinity）。
        return Ok(ClipBounds {
            min_start: 0.0,
            max_end: ai_daw_engine::model::Clip::MAX_START_BEATS,
        });
    };
    let map = TempoMap::from_transport(&project.transport);
    let audio = state.assets.load(&src.path)?;
    let (min_start, max_end) = clip_bounds_for(
        &map,
        c.start,
        src.offset_frames,
        audio.frames() as u64,
        audio.sample_rate,
        src.rate,
    );
    Ok(ClipBounds { min_start, max_end })
}

/// 新建工程：清空并铺上默认分轨模板。
///
/// `track_names` 由界面按**当前语言**给出（按模板顺序）；不传就用引擎的兜底名。
/// 名字写进工程后就是用户数据了，之后切换界面语言不会再动它。
#[tauri::command]
pub fn new_project(
    app: tauri::AppHandle,
    track_names: Option<Vec<String>>,
    tempo: Option<f64>,
) -> Project {
    new_project_as(&app, Actor::Human, track_names, tempo)
}

/// 新建工程的共用实现（界面署名 `Human`，MCP 用代理署名）。
pub(crate) fn new_project_as(
    app: &tauri::AppHandle,
    actor: Actor,
    track_names: Option<Vec<String>>,
    tempo: Option<f64>,
) -> Project {
    let state = app.state::<AppState>();
    // 新建会清空当前工程：先把它收进历史，用户随时能回去。
    remember_current(app, &state);
    let mut commands = vec![Command::NewProject { name: "untitled".into(), track_names }];
    // 设置里可配的「新工程默认速度」：与模板默认 120 不同才补一条命令（同一个撤销步骤）。
    if let Some(bpm) = tempo {
        if (bpm - ai_daw_engine::model::Transport::default().tempo).abs() > f64::EPSILON {
            commands.push(Command::SetTempo(bpm));
        }
    }
    let project = run_agent_all_as(app, actor, commands).0;
    // 刚新建的工程没有任何未保存的改动，还没有对应的文件。
    state.engine.lock().unwrap().mark_saved();
    state.set_session_path(None);
    project
}

/// 重命名当前工程。
#[tauri::command]
pub fn rename_project(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Project {
    run(&app, &state, Command::RenameProject { name })
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
    run(&app, &state, Command::RemoveClip { track: TrackId(track), clip: ai_daw_engine::model::ClipId(clip) })
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
        Command::MoveClip { track: TrackId(track), clip: ai_daw_engine::model::ClipId(clip), start },
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
        Command::ResizeClip { track: TrackId(track), clip: ai_daw_engine::model::ClipId(clip), length },
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
        Command::AddNote { track: TrackId(track), clip: ai_daw_engine::model::ClipId(clip), note },
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
        Command::SetNote { track: TrackId(track), clip: ai_daw_engine::model::ClipId(clip), index, note },
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
        Command::SetNotes { track: TrackId(track), clip: ai_daw_engine::model::ClipId(clip), entries },
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
        Command::RemoveNote { track: TrackId(track), clip: ai_daw_engine::model::ClipId(clip), index },
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
        Command::RemoveNotes { track: TrackId(track), clip: ai_daw_engine::model::ClipId(clip), indices },
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
// 编曲：批量音符、律动与音域变换
// ---------------------------------------------------------------------------

/// 一次新增一组音符（一次事务 = 一条撤销记录）。
#[tauri::command]
pub fn add_notes(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
    notes: Vec<Note>,
) -> Project {
    run(&app, &state, Command::AddNotes { track: TrackId(track), clip: ClipId(clip), notes })
}

/// 原子替换片段内的全部音符（空数组 = 清空片段）。
#[tauri::command]
pub fn replace_notes(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
    notes: Vec<Note>,
) -> Project {
    run(&app, &state, Command::ReplaceNotes { track: TrackId(track), clip: ClipId(clip), notes })
}

/// 按稳定 id 批量改写音符（只改给出的字段）。
#[tauri::command]
pub fn update_notes(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
    updates: Vec<NotePatch>,
) -> Project {
    run(&app, &state, Command::UpdateNotes { track: TrackId(track), clip: ClipId(clip), updates })
}

/// 按稳定 id 批量删除音符。
#[tauri::command]
pub fn remove_notes_by_id(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
    ids: Vec<u64>,
) -> Project {
    run(&app, &state, Command::RemoveNotesById { track: TrackId(track), clip: ClipId(clip), ids })
}

/// 批量移调（半音），可限定范围。
#[tauri::command]
pub fn transpose_notes(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
    semitones: i32,
    range: Option<NoteRange>,
) -> Project {
    run(
        &app,
        &state,
        Command::TransposeNotes { track: TrackId(track), clip: ClipId(clip), semitones, range },
    )
}

/// 批量设置力度（固定 / 渐变 / 缩放）。
#[tauri::command]
pub fn set_note_velocity(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
    shape: VelocityShape,
    range: Option<NoteRange>,
    beat_filter: Option<BeatFilter>,
) -> Project {
    run(
        &app,
        &state,
        Command::SetNoteVelocity {
            track: TrackId(track),
            clip: ClipId(clip),
            shape,
            range,
            beat_filter,
        },
    )
}

/// 量化：把起点吸到 `grid` 的整数倍。
#[tauri::command]
pub fn quantize_notes(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
    grid: f64,
    strength: f64,
    range: Option<NoteRange>,
    quantize_duration: Option<bool>,
) -> Project {
    run(
        &app,
        &state,
        Command::QuantizeNotes {
            track: TrackId(track),
            clip: ClipId(clip),
            grid,
            strength,
            range,
            quantize_duration: quantize_duration.unwrap_or(false),
        },
    )
}

/// Swing：把指定子拍的第二个往后推。
#[tauri::command]
pub fn apply_swing(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
    subdivision: f64,
    ratio: f64,
    strength: f64,
    range: Option<NoteRange>,
) -> Project {
    run(
        &app,
        &state,
        Command::ApplySwing {
            track: TrackId(track),
            clip: ClipId(clip),
            subdivision,
            ratio,
            strength,
            range,
        },
    )
}

/// 扫弦：把**起点相同**的音符按音高依次错开（`step_ms = 0` = 回到同时发声）。
#[tauri::command]
pub fn strum_notes(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
    step_ms: f64,
    direction: StrumDirection,
    range: Option<NoteRange>,
) -> Project {
    run(
        &app,
        &state,
        Command::StrumNotes {
            track: TrackId(track),
            clip: ClipId(clip),
            step_ms,
            direction,
            range,
        },
    )
}

/// 人性化：位置 / 力度的确定性抖动。
#[tauri::command]
pub fn humanize_notes(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
    timing: f64,
    velocity: f64,
    range: Option<NoteRange>,
    seed: Option<u64>,
    min_duration: Option<f64>,
    group_by_start: Option<bool>,
) -> Project {
    run(
        &app,
        &state,
        Command::HumanizeNotes {
            track: TrackId(track),
            clip: ClipId(clip),
            timing,
            velocity,
            range,
            seed,
            min_duration: min_duration.unwrap_or(0.0),
            group_by_start: group_by_start.unwrap_or(false),
        },
    )
}

/// 把音符收进目标音域（`fold` 八度折返 / `report` 只报告）。
#[tauri::command]
pub fn fit_notes_to_range(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
    min_pitch: u8,
    max_pitch: u8,
    mode: FitMode,
    range: Option<NoteRange>,
) -> Project {
    run(
        &app,
        &state,
        Command::FitNotesToRange {
            track: TrackId(track),
            clip: ClipId(clip),
            min_pitch,
            max_pitch,
            mode,
            range,
        },
    )
}

/// 复制片段（缺省贴在原片段之后）。
#[tauri::command]
pub fn duplicate_clip(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
    to_track: Option<u64>,
    start: Option<f64>,
) -> Project {
    run(
        &app,
        &state,
        Command::DuplicateClip {
            track: TrackId(track),
            clip: ClipId(clip),
            to_track: to_track.map(TrackId),
            start,
        },
    )
}

/// 合并同一轨道上的多个 MIDI 片段。
#[tauri::command]
pub fn merge_clips(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clips: Vec<u64>,
) -> Project {
    run(
        &app,
        &state,
        Command::MergeClips { track: TrackId(track), clips: clips.into_iter().map(ClipId).collect() },
    )
}

// ---------------------------------------------------------------------------
// 编曲分析（只读）：音域 / 密度 / 重叠 / 和声
// ---------------------------------------------------------------------------

/// 单条轨道（或其中某个片段）的音符统计。
#[tauri::command]
pub fn get_track_note_stats(
    state: State<'_, AppState>,
    track: u64,
    clip: Option<u64>,
) -> Result<ai_daw_engine::report::NoteStats, String> {
    let project = state.engine.lock().unwrap().project().clone();
    let found = project
        .track(TrackId(track))
        .ok_or_else(|| format!("没有轨道 {track}"))?;
    Ok(ai_daw_engine::compose::note_stats(
        found,
        clip.map(ClipId),
        project.transport.bar_beats(),
    ))
}

/// 多条轨道之间的音区重叠与分工冲突。
#[tauri::command]
pub fn analyze_track_overlap(
    state: State<'_, AppState>,
    tracks: Vec<u64>,
    clip: Option<u64>,
) -> Result<ai_daw_engine::report::OverlapReport, String> {
    let project = state.engine.lock().unwrap().project().clone();
    let picked: Vec<&ai_daw_engine::model::Track> = tracks
        .iter()
        .map(|id| project.track(TrackId(*id)).ok_or_else(|| format!("没有轨道 {id}")))
        .collect::<Result<_, _>>()?;

    let mut pairs = Vec::new();
    for (i, a) in picked.iter().enumerate() {
        for b in picked.iter().skip(i + 1) {
            pairs.push(ai_daw_engine::compose::track_overlap(a, b, clip.map(ClipId)));
        }
    }
    let summary: Vec<String> = pairs
        .iter()
        .filter(|p| p.conflict && !p.suggestion.is_empty())
        .map(|p| p.suggestion.clone())
        .collect();
    Ok(ai_daw_engine::report::OverlapReport { tracks, pairs, summary })
}

/// 和声检查：`bars` 缺省时用工程里存的和弦表；`tracks` 缺省时检查所有有音符的轨道。
#[tauri::command]
pub fn analyze_harmony(
    state: State<'_, AppState>,
    bars: Option<Vec<HarmonyBar>>,
    tracks: Option<Vec<u64>>,
    allow_extensions: Option<bool>,
    allow_approach_notes: Option<bool>,
    allow_non_chord_tone_duration: Option<f64>,
    time_tolerance: Option<f64>,
    key: Option<String>,
    scale: Option<String>,
) -> Result<ai_daw_engine::report::HarmonyReport, String> {
    let project = state.engine.lock().unwrap().project().clone();
    let bars = match bars {
        Some(bars) if !bars.is_empty() => bars,
        _ if !project.harmony.is_empty() => project.harmony.clone(),
        _ => {
            return Err(
                "没有和弦信息：传 bars（[{start, length, symbol}]），或先用 set_project_harmony 存一份"
                    .into(),
            )
        }
    };
    let picked: Vec<&ai_daw_engine::model::Track> = match tracks {
        Some(ids) => ids
            .iter()
            .map(|id| project.track(TrackId(*id)).ok_or_else(|| format!("没有轨道 {id}")))
            .collect::<Result<_, _>>()?,
        None => project.tracks.iter().filter(|t| t.clips.iter().any(|c| !c.notes.is_empty())).collect(),
    };
    if picked.is_empty() {
        return Err("没有可检查的音符：指定的轨道上没有任何音符".into());
    }
    let policy = ai_daw_engine::report::HarmonyPolicy {
        allow_extensions: allow_extensions.unwrap_or(true),
        allow_approach_notes: allow_approach_notes.unwrap_or(true),
        allow_non_chord_tone_duration: allow_non_chord_tone_duration.unwrap_or(0.5),
    };
    let scale = match (&key, &scale) {
        (Some(root), mode) => Some(ai_daw_engine::harmony::parse_scale(
            root,
            mode.as_deref().unwrap_or("major"),
        )?),
        (None, Some(_)) => {
            return Err("给了 scale 就必须同时给 key（例如 key=\"F\", scale=\"major\"）".into())
        }
        (None, None) => None,
    };
    let ornament_ids: Vec<(u64, u64, u64)> = project
        .note_tags
        .iter()
        .filter_map(|(key, tag)| {
            tag.ornament
                .as_deref()
                .filter(|v| !v.trim().is_empty())
                .and_then(|_| ai_daw_engine::model::parse_note_tag_key(key))
        })
        .collect();
    ai_daw_engine::harmony::analyze_with_options(
        &bars,
        &picked,
        policy,
        project.transport.bar_beats(),
        time_tolerance.unwrap_or(1.0 / 16.0),
        scale.as_deref(),
        if ornament_ids.is_empty() { None } else { Some(&ornament_ids) },
    )
}

/// 整体替换工程的小节和弦表（空 = 清除）。
#[tauri::command]
pub fn set_project_harmony(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    bars: Vec<HarmonyBar>,
) -> Project {
    run(&app, &state, Command::SetProjectHarmony { bars })
}

/// 整体替换工程的段落表（空 = 清除）。
#[tauri::command]
pub fn set_project_sections(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    sections: Vec<Section>,
) -> Project {
    run(&app, &state, Command::SetProjectSections { sections })
}

/// 设置轨道的编曲元数据（分工 / 目标音域 / 弯音范围）；三者都空 = 清除。
#[tauri::command]
pub fn set_track_meta(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    role: Option<String>,
    min_pitch: Option<u8>,
    max_pitch: Option<u8>,
    pitch_bend_range: Option<f32>,
) -> Project {
    let pitch_range = match (min_pitch, max_pitch) {
        (Some(min), Some(max)) => Some(PitchRange { min, max }),
        (None, None) => None,
        _ => return state.engine.lock().unwrap().project().clone(),
    };
    run(
        &app,
        &state,
        Command::SetTrackMeta { track: TrackId(track), role, pitch_range, pitch_bend_range },
    )
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

/// 设置轨道的发送列表（整体替换；空 = 清空）。
#[tauri::command]
pub fn set_sends(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    sends: Vec<Send>,
) -> Project {
    run(&app, &state, Command::SetSends { track: TrackId(track), sends })
}

/// 设置一条自动化车道（整体替换该参数的曲线；空 `points` = 删除该车道）。
#[tauri::command]
pub fn set_automation(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    param: String,
    points: Vec<AutomationPoint>,
) -> Project {
    run(&app, &state, Command::SetAutomation { track: TrackId(track), param, points })
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
    HistoryDepths { undo: engine.undo_depth(), redo: engine.redo_depth(), dirty: engine.is_dirty() }
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
                dirty: engine.is_dirty(),
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
                dirty: engine.is_dirty(),
            },
            engine.project().clone(),
        )
    };
    state.audio.load(&project);
    out
}

/// 撤销最后一次命令。
#[tauri::command]
pub fn undo(app: tauri::AppHandle, state: State<'_, AppState>) -> HistoryState {
    let out = undo_now(&state);
    // 撤销可能改回别的乐器：实例对不上的窗口要关掉（对得上的仍然保留）。
    close_stale_plugin_windows(&app, &state);
    out
}

/// 重做上一次被撤销的命令。
#[tauri::command]
pub fn redo(app: tauri::AppHandle, state: State<'_, AppState>) -> HistoryState {
    let out = redo_now(&state);
    close_stale_plugin_windows(&app, &state);
    out
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

/// 设定播放范围（帧）：开了循环时播到 `end` 无缝回到 `start`。
///
/// `end <= start` = 取消范围。范围本身不启动播放，也不改「是否循环」——那是
/// `transport_set_loop` 的事。
#[tauri::command]
pub fn transport_set_loop_range(
    state: State<'_, AppState>,
    start: u64,
    end: u64,
) -> Result<(), String> {
    state.audio.send(TransportCommand::SetLoopRange { start, end })
}

/// 开关节拍器：按工程速度 / 拍号在整数拍上打点（重音在小节头），只在播放时出声。
#[tauri::command]
pub fn transport_set_metronome(state: State<'_, AppState>, on: bool) -> Result<(), String> {
    state.audio.send(TransportCommand::SetMetronome(on))
}

// ---------------------------------------------------------------------------
// 文件与渲染
// ---------------------------------------------------------------------------

/// 把当前工程保存为 JSON 文件，返回写入的绝对路径。
#[tauri::command]
pub async fn save_project(app: tauri::AppHandle, path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || save_project_impl(&app, &path))
        .await
        .map_err(|e| e.to_string())?
}

/// 保存工程的同步实现（界面命令与 MCP 工具共用）。
pub(crate) fn save_project_impl(app: &tauri::AppHandle, path: &str) -> Result<String, String> {
    let state = app.state::<AppState>();
    // 落盘前先把插件实例的内部状态抢救进工程，否则存出去的工程会丢掉外部插件的补丁。
    capture_plugin_states(&state);
    // 覆盖已有文件前，先把旧版本留进历史（防止「保存坏了的一版」把上一版顶掉）。
    remember_file_version(app, path);
    let project = { state.engine.lock().unwrap().project().clone() };
    let json = serde_json::to_string_pretty(&project).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())?;
    // 写盘成功：当前工程成为新的「已保存」基线，并记下文件路径。
    state.engine.lock().unwrap().mark_saved();
    state.set_session_path(Some(path.to_string()));
    // 刚保存的这一版也进历史：界面列表里能直接看到已保存的工程。
    remember_snapshot(app, &state);
    Ok(path.to_string())
}

/// 当前工程对应的文件路径（尚未保存过时为 `null`）。
#[tauri::command]
pub fn current_project_path(state: State<'_, AppState>) -> Option<String> {
    state.session_path()
}

/// 把当前工作状态落进会话缓存（纯落盘，不改动内存里的工程）。
///
/// `cache_session` 命令与**退出兜底**共用这一条路径。后端在 `RunEvent::ExitRequested` 里也
/// 直接调它：macOS 从程序坞「退出」/ Cmd+Q 是进程级退出，前端的 `onCloseRequested` 守卫根本
/// 不触发，只靠 10 秒一次的自动保存会把「最近一次保存之后」的改动（比如刚添加的效果器）
/// 整段丢掉。
pub(crate) fn write_session(app: &tauri::AppHandle) -> Result<(), String> {
    let Some(file) = crate::session_file(app) else {
        return Ok(());
    };
    let state = app.state::<AppState>();
    // 会话缓存同样要记下插件实例的内部状态，下次启动才能还原外部插件的补丁。
    capture_plugin_states(&state);
    let path = state.session_path();
    let (project, saved) = {
        let engine = state.engine.lock().unwrap();
        (engine.project().clone(), engine.saved().clone())
    };
    let session = crate::session::SessionFile { path, project, saved };
    crate::session::write_session(&file, &session)
}

/// 把当前工程写入会话缓存（退出前 / 编辑过程中定期调用，供下次启动还原）。
///
/// 缓存同时记录工作副本与保存基线，因此带着未保存改动退出后，下次打开依旧显示「未保存」。
#[tauri::command]
pub fn cache_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: Option<String>,
) -> Result<(), String> {
    state.set_session_path(path);
    write_session(&app)
}

/// 历史工程列表（展示用元信息；完整快照留在配置目录里，按 id 恢复）。
#[tauri::command]
pub fn project_history(app: tauri::AppHandle) -> Vec<crate::history::HistoryEntry> {
    crate::history_dir(&app)
        .map(|dir| crate::history::load_index(&dir))
        .unwrap_or_default()
}

/// 删除一份历史快照（只动历史目录，不影响磁盘上的工程文件）。
#[tauri::command]
pub fn delete_project_history(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let dir = crate::history_dir(&app).ok_or_else(|| "拿不到应用配置目录".to_string())?;
    crate::history::delete(&dir, &id)
}

/// 回到一份历史工程。
///
/// 恢复前先把**当前**工程收进历史，所以来回切换不会丢东西；被恢复的那份则从列表里取出
/// （它已经是当前工程了，再留在列表里只会制造重复项）。
#[tauri::command]
pub fn restore_project_history(app: tauri::AppHandle, id: String) -> Result<Project, String> {
    restore_project_history_impl(&app, &id)
}

/// 从历史恢复工程的同步实现（界面命令与 MCP 工具共用）。
pub(crate) fn restore_project_history_impl(
    app: &tauri::AppHandle,
    id: &str,
) -> Result<Project, String> {
    let state = app.state::<AppState>();
    let dir = crate::history_dir(app).ok_or_else(|| "拿不到应用配置目录".to_string())?;
    let (snapshot, entry) = crate::history::take(&dir, id)?;
    remember_current(app, &state);

    let restored = normalize_project(snapshot.project);
    let saved = normalize_project(snapshot.saved);
    {
        let mut engine = state.engine.lock().unwrap();
        // 保存基线一并还原：当时有未保存改动，恢复后也照旧显示「未保存」。
        *engine = Engine::restore(restored.clone(), saved);
    }
    state.audio.load(&restored);
    // 整个工程被替换：已开的插件窗口一律作废。
    close_all_plugin_windows(app, &state);
    state.set_session_path(entry.path);
    Ok(restored)
}

/// 工程读入后的统一归一化：手改过的文件、旧版本工程都按引擎的合法范围夹紧。
fn normalize_project(mut project: Project) -> Project {
    // 与 `SetTempo` 一致地夹紧：非有限或 0 的速度会让离线渲染时长变成 Inf。
    if !project.transport.tempo.is_finite() {
        project.transport.tempo = 120.0;
    }
    project.transport.tempo = project.transport.tempo.clamp(1.0, 1000.0);
    // 拍号同样归一化：分母为 0 会让按小节换算的长度除不尽。
    let (num, den) = ai_daw_engine::model::Transport::normalize_time_sig(
        project.transport.time_sig_num,
        project.transport.time_sig_den,
    );
    project.transport.time_sig_num = num;
    project.transport.time_sig_den = den;
    // 混音参数与片段/音符同样要过一遍归一化：工程文件可能被手改过。
    project.master.gain_db =
        ai_daw_engine::model::MixerChannel::normalize_gain_db(project.master.gain_db);
    project.master.pan = ai_daw_engine::model::MixerChannel::normalize_pan(project.master.pan);
    for track in project.tracks.iter_mut() {
        track.mixer.gain_db =
            ai_daw_engine::model::MixerChannel::normalize_gain_db(track.mixer.gain_db);
        track.mixer.pan = ai_daw_engine::model::MixerChannel::normalize_pan(track.mixer.pan);
        for clip in track.clips.iter_mut() {
            clip.start = ai_daw_engine::model::Clip::normalize_start(clip.start);
            clip.length = ai_daw_engine::model::Clip::normalize_length(clip.length);
            for note in clip.notes.iter_mut() {
                *note = Note::normalize(*note);
            }
        }
    }
    // 旧工程文件可能没有音符 id；补号后当前工程与保存基线才可比。
    project.ensure_note_ids();
    project
}

/// 从 JSON 文件导入工程：替换当前工程并清空历史（导入是一个新的起点）。
#[tauri::command]
pub async fn open_project(app: tauri::AppHandle, path: String) -> Result<Project, String> {
    tauri::async_runtime::spawn_blocking(move || open_project_impl(&app, &path))
        .await
        .map_err(|e| e.to_string())?
}

/// 打开工程的同步实现（界面命令与 MCP 工具共用）。
pub(crate) fn open_project_impl(
    app: &tauri::AppHandle,
    path: &str,
) -> Result<Project, String> {
    let state = app.state::<AppState>();
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let project: Project =
        serde_json::from_str(&text).map_err(|e| format!("不是有效的工程文件：{e}"))?;
    let project = normalize_project(project);

    // 打开另一个工程同样会丢弃当前工程：先记进历史。
    remember_current(app, &state);
    let snapshot = {
        let mut engine = state.engine.lock().unwrap();
        *engine = Engine::from_project(project);
        engine.project().clone()
    };
    state.audio.load(&snapshot);
    // 整个工程被替换：已开的插件窗口一律作废（哪怕轨道 id 碰巧撞上，内容也换了）。
    close_all_plugin_windows(app, &state);
    // 打开的工程即为「已保存」状态，并记下它的文件路径。
    state.set_session_path(Some(path.to_string()));
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
) -> Result<ai_daw_analysis::Report, String> {
    validate_sample_rate(sample_rate)?;
    let inputs = render_inputs(&state);
    tauri::async_runtime::spawn_blocking(move || {
        // 分析只关心工程正文，不加尾音。
        let buffer = render_offline(inputs, sample_rate, 0.0);
        ai_daw_analysis::analyze(buffer.data(), buffer.channels(), sample_rate as f64)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 导出格式。
///
/// 由界面（导出对话框）选好再传进来。两种格式**共用同一条离线渲染路径**，只有最后一步的
/// 编码器不同：WAV 是 16-bit PCM 无损，MP3 走 LAME（有损，且首尾会有几毫秒的编码器补零）。
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// 16-bit PCM WAV。
    Wav,
    /// LAME 编码的 MP3。
    Mp3,
}

/// 导出进度（前端顶边那条进度条据此更新）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExportProgress {
    /// 目标文件路径。
    pub path: String,
    /// 阶段：`preparing`（建插件实例 / 等外部插件装载）→ `rendering`（离线渲染）→
    /// `encoding`（编码 + 写盘）→ `done` / `error`。
    pub phase: &'static str,
    /// 本阶段已完成的工作量（渲染阶段 = 已渲染帧数）。
    pub frames: u64,
    /// 本阶段的总量；未知时 `None`（进度条走「不确定」态）。
    pub total_frames: Option<u64>,
    /// 失败原因（只有 `error` 阶段带）。
    pub message: Option<String>,
}

/// 发一条导出进度事件（发后即忘；界面没在听也无所谓）。
fn emit_export(
    app: &tauri::AppHandle,
    path: &str,
    phase: &'static str,
    frames: u64,
    total_frames: Option<u64>,
    message: Option<String>,
) {
    let _ = app.emit(
        EVENT_EXPORT,
        ExportProgress {
            path: path.to_string(),
            phase,
            frames,
            total_frames,
            message,
        },
    );
}

/// 把当前工程导出为音频文件，返回写入的绝对路径。
///
/// `format` 决定编码器；`bitrate_kbps` 只对 MP3 有意义（缺省用
/// [`ai_daw_engine::mp3::DEFAULT_BITRATE_KBPS`]）。
///
/// 渲染与编码都在阻塞线程池上跑，并且**边走边发** `engine://export` 进度事件——界面由此画顶边
/// 进度条，同时继续响应其它操作（发事件是发后即忘，不会回头等界面）。
#[tauri::command]
pub async fn export_audio(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    sample_rate: u32,
    format: ExportFormat,
    bitrate_kbps: Option<u32>,
) -> Result<String, String> {
    validate_sample_rate(sample_rate)?;
    // 同样只在锁内克隆，渲染与写盘都放到阻塞线程池，避免卡住主线程。
    let inputs = render_inputs(&state);
    let progress_app = app.clone();
    let progress_path = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // 阶段一：建实例 +（引用了外部插件时）等它把素材装载完。这一段没有可细分的进度，
        // 就是「准备中」——但它是真实存在的等待（最长 EXTERNAL_PLUGIN_SETTLE_SECONDS）。
        emit_export(&progress_app, &progress_path, "preparing", 0, None, None);
        let buffer = render_offline_with_progress(inputs, sample_rate, 0.5, |done, total| {
            emit_export(
                &progress_app,
                &progress_path,
                "rendering",
                done as u64,
                Some(total as u64),
                None,
            );
        });

        emit_export(&progress_app, &progress_path, "encoding", 0, None, None);
        let written = match format {
            ExportFormat::Wav => {
                ai_daw_engine::wav::write_wav(&path, &buffer, sample_rate).map_err(|e| e.to_string())
            }
            ExportFormat::Mp3 => {
                let bitrate = bitrate_kbps.unwrap_or(ai_daw_engine::mp3::DEFAULT_BITRATE_KBPS);
                ai_daw_engine::mp3::write_mp3(&path, &buffer, sample_rate, bitrate)
                    .map_err(|e| e.to_string())
            }
        };

        match &written {
            Ok(()) => emit_export(&progress_app, &progress_path, "done", 0, None, None),
            Err(err) => {
                emit_export(&progress_app, &progress_path, "error", 0, None, Some(err.clone()))
            }
        }
        written?;
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

/// **时间轴秒 → 素材源帧**，含片段的播放速度 `rate`。
///
/// 音频片段的语义（见 `specs/2026-10-09-audio-clip-design.md` §3）：时间轴 1 秒 = 素材 **`rate`** 秒
/// （`rate = 2` 就是「一秒听两秒素材」）。所以**任何在「拍 / 时间轴秒」与「素材帧」之间换算的地方
/// 都必须带上它**——漏掉就会出现「改了播放速度以后再拖动片段，音频被砍掉一半（后半段静音）」这类错误。
pub(crate) fn span_to_source_frames(seconds: f64, sample_rate: f64, rate: f64) -> f64 {
    (seconds * sample_rate * rate).round().max(0.0)
}

/// **素材源帧 → 时间轴秒**，含片段的播放速度 `rate`（[`span_to_source_frames`] 的逆）。
pub(crate) fn source_frames_to_span(frames: f64, sample_rate: f64, rate: f64) -> f64 {
    frames / (sample_rate * rate.max(1e-9))
}

/// 裁剪 / 移动一个音频片段后，它引用的素材窗口：`(offset_frames, frames)`。
///
/// - `old_start` / `new_start`：方框起点（拍），两者之差就是要平移的素材量；
/// - `length`：新的方框长度（拍）；
/// - 返回值都已夹到素材范围内（窗口至少 1 帧）。
///
/// 抽成纯函数是为了**能被单测直接钉住**：这套换算漏掉 `rate` 时，改了播放速度以后再拖动片段
/// 就会把音频砍掉一半（后半段静音），而这类错误在界面里很难一眼看出来。
pub(crate) fn trimmed_window(
    map: &TempoMap,
    old_start: f64,
    new_start: f64,
    length: f64,
    offset_frames: u64,
    total: u64,
    sample_rate: f64,
    rate: f64,
) -> (u64, u64) {
    let shift = span_to_source_frames(
        map.beats_to_seconds(new_start) - map.beats_to_seconds(old_start),
        sample_rate,
        rate,
    ) as i64;
    let offset = (offset_frames as i64 + shift).clamp(0, total as i64) as u64;
    let visible = span_to_source_frames(
        map.beats_to_seconds(new_start + length) - map.beats_to_seconds(new_start),
        sample_rate,
        rate,
    ) as u64;
    (offset, visible.clamp(1, total.saturating_sub(offset).max(1)))
}

/// 音频片段能「展开」到的范围（拍）：左端 = 素材第一帧所在的位置，右端 = 素材最后一帧所在的位置。
///
/// **素材帧除以 `rate` 才是时间轴秒**：`rate = 2` 时两秒素材只占一秒时间轴——漏掉它，
/// 界面就能把右边缘拖到素材早已放完之后，露出一段没有声音的空档。
pub(crate) fn clip_bounds_for(
    map: &TempoMap,
    start: f64,
    offset_frames: u64,
    total: u64,
    sample_rate: f64,
    rate: f64,
) -> (f64, f64) {
    let t0 = map.beats_to_seconds(start);
    let consumed = source_frames_to_span(offset_frames as f64, sample_rate, rate);
    let available = source_frames_to_span(total.saturating_sub(offset_frames) as f64, sample_rate, rate);
    (map.seconds_to_beats(t0 - consumed), map.seconds_to_beats(t0 + available))
}

/// 音频导入的解码进度（与前端 `ImportProgress` 一一对应）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ImportProgress {
    /// 正在解码的素材路径。
    pub path: String,
    /// 阶段：`decoding` / `done` / `error`。
    pub state: String,
    /// 已解出的帧数（`decoding` 时有意义）。
    pub frames: u64,
    /// 总帧数；容器头预知不到时为 `None`（界面据此画不确定态进度条）。
    pub total_frames: Option<u64>,
    /// 失败原因（仅 `error` 有意义）。
    pub message: Option<String>,
}

/// 广播一次导入解码进度。
fn emit_import(
    app: &tauri::AppHandle,
    path: &str,
    state: &str,
    frames: u64,
    total_frames: Option<u64>,
    message: Option<String>,
) {
    let _ = app.emit(
        EVENT_IMPORT,
        ImportProgress {
            path: path.to_string(),
            state: state.to_string(),
            frames,
            total_frames,
            message,
        },
    );
}

/// 导入一个音频文件为音频片段（wav / flac / mp3 / ogg-vorbis / aac / m4a）。
///
/// 解码放到阻塞线程池：它既慢又可能失败，绝不能占住命令线程。解码途中经 `engine://import`
/// 推送进度（顶栏下边缘的进度条据此绘制）；解码完成后才落片段。
#[tauri::command]
pub async fn import_audio(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    track: u64,
    start: f64,
) -> Result<Project, String> {
    // 立刻发一条「开始解码」：总帧数还没探测出来，界面先画一条不确定态进度条。
    emit_import(&app, &path, "decoding", 0, None, None);

    let assets = Arc::clone(&state.assets);
    let path_for_load = path.clone();
    let progress_app = app.clone();
    let progress_path = path.clone();
    let loaded = tauri::async_runtime::spawn_blocking(move || {
        assets.load_with_progress(&path_for_load, |p| {
            emit_import(&progress_app, &progress_path, "decoding", p.frames, p.total_frames, None);
        })
    })
    .await
    .map_err(|e| e.to_string())?;

    let audio = match loaded {
        Ok(audio) => audio,
        Err(err) => {
            emit_import(&app, &path, "error", 0, None, Some(err.clone()));
            return Err(err);
        }
    };

    let frames = audio.frames() as u64;
    let seconds = audio.frames() as f64 / audio.sample_rate;
    if !(seconds.is_finite() && seconds > 0.0) {
        let err = format!("音频素材没有可用长度：{path}");
        emit_import(&app, &path, "error", 0, None, Some(err.clone()));
        return Err(err);
    }
    let start = ai_daw_engine::model::Clip::normalize_start(start);
    let length = {
        let engine = state.engine.lock().unwrap();
        natural_length_beats(engine.project(), start, seconds)
    };
    emit_import(&app, &path, "done", frames, Some(frames), None);
    Ok(run(
        &app,
        &state,
        Command::AddAudioClip {
            track: TrackId(track),
            start,
            length,
            // 记下素材总帧数：切分时才知道「左半段到哪停」（否则会一路播到结尾、被拉伸）。
            source: AudioSource { path, offset_frames: 0, frames: Some(frames), rate: 1.0, pitch_semitones: 0.0 },
        },
    ))
}

/// 把一个**已导入**的音频素材再放一份到轨道上（复制粘贴音频片段走它）。
///
/// 与 [`import_audio`] 的区别：素材早就解码过，长度也由调用方给（粘贴要的是**一模一样**的副本），
/// 所以这里不重新解码、也不重新按素材量长度——一次事务，一个撤销步骤。
#[tauri::command]
pub fn add_audio_clip(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    start: f64,
    length: f64,
    path: String,
    offset_frames: u64,
    frames: Option<u64>,
) -> Project {
    run(
        &app,
        &state,
        Command::AddAudioClip {
            track: TrackId(track),
            start,
            length,
            source: AudioSource { path, offset_frames, frames, rate: 1.0, pitch_semitones: 0.0 },
        },
    )
}

/// 在第 `beat` 拍切分片段（razor）：左半段沿用原 id，右半段是新片段。
///
/// 旧工程的音频片段可能没记素材窗口长度，切分需要它，所以先从素材库补上——补写与切分在
/// **同一批命令**里执行，因此仍然只占一个撤销步骤。
#[tauri::command]
pub async fn split_clip(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    clip: u64,
    beat: f64,
) -> Result<Project, String> {
    let stale_audio = {
        let engine = state.engine.lock().unwrap();
        engine
            .project()
            .track(TrackId(track))
            .and_then(|t| t.clips.iter().find(|c| c.id == ClipId(clip)))
            .and_then(|c| c.audio.clone())
            .filter(|a| a.frames.is_none())
    };

    let mut commands = Vec::new();
    if let Some(src) = stale_audio {
        let assets = Arc::clone(&state.assets);
        let path = src.path.clone();
        let audio = tauri::async_runtime::spawn_blocking(move || assets.load(&path))
            .await
            .map_err(|e| e.to_string())??;
        // 切分按「方框占素材窗口的比例」切，所以要补的是**方框对应的帧数**（不是到素材结尾）。
        let (clip_start, clip_len, transport) = {
            let engine = state.engine.lock().unwrap();
            let p = engine.project();
            let c = p
                .track(TrackId(track))
                .and_then(|t| t.clips.iter().find(|c| c.id == ClipId(clip)))
                .ok_or("片段不存在")?;
            (c.start, c.length, p.transport.clone())
        };
        let map = TempoMap::from_transport(&transport);
        let span = map.beats_to_seconds(clip_start + clip_len) - map.beats_to_seconds(clip_start);
        let total = audio.frames() as u64;
        let offset = src.offset_frames.min(total);
        // 左半段吃掉多少素材帧：同样要带上 `rate`（时间轴 1 秒 = 素材 `rate` 秒）。
        let visible = span_to_source_frames(span, audio.sample_rate, src.rate).max(1.0) as u64;
        commands.push(Command::SetClipAudio {
            track: TrackId(track),
            clip: ClipId(clip),
            audio: Some(AudioSource {
                path: src.path,
                offset_frames: offset,
                frames: Some(visible.clamp(1, total.saturating_sub(offset).max(1))),
                rate: src.rate,
                pitch_semitones: src.pitch_semitones,
            }),
        });
    }
    commands.push(Command::SplitClip { track: TrackId(track), clip: ClipId(clip), beat });

    Ok(run_all(&app, &state, Actor::Human, commands).0)
}

/// 片段波形：包络 + 素材采样率。
///
/// 界面需要采样率把「源帧」换算成秒 / 拍，才能把波形画在**它真实的时间位置**上、再按片段
/// 方框裁切——而不是把窗口拉伸铺满方框（拖动边缘时会看着像在拉伸/移动）。
#[derive(serde::Serialize)]
pub struct WaveformSlice {
    /// 每桶 min/max 各一个值（长度 = `buckets * 2`）。
    pub peaks: Vec<f32>,
    /// 素材采样率（Hz）。
    pub sample_rate: f64,
    /// 本窗口实际取到的源帧数（`frames` 夹紧后）——前端据此算窗口时长。
    pub frames: u64,
    /// 窗口在素材里的起点（源帧，夹紧后）。
    pub offset_frames: u64,
}

/// 取某个音频文件的波形包络（每桶 min/max 各一个值，长度 = `buckets * 2`），供界面绘制。
///
/// `offset_frames` / `frames` 用来只取素材的一个**窗口**（片段引用的是窗口而不是整段素材）；
/// 缺省表示整段。
#[tauri::command]
pub async fn audio_waveform(
    state: State<'_, AppState>,
    path: String,
    buckets: usize,
    offset_frames: Option<u64>,
    frames: Option<u64>,
) -> Result<WaveformSlice, String> {
    let assets = Arc::clone(&state.assets);
    tauri::async_runtime::spawn_blocking(move || {
        let audio = assets.load(&path)?;
        let total = audio.frames() as u64;
        let offset = offset_frames.unwrap_or(0).min(total);
        let window = frames.unwrap_or(total - offset).min(total - offset);
        let peaks = assets.waveform_range(&path, buckets, offset, Some(window))?;
        Ok(WaveformSlice {
            peaks,
            sample_rate: audio.sample_rate,
            frames: window,
            offset_frames: offset,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// 录音（C2）
// ---------------------------------------------------------------------------

/// 本机可用的音频输入设备。
#[tauri::command]
pub fn list_input_devices() -> Vec<ai_daw_engine::recorder::InputDeviceInfo> {
    ai_daw_engine::recorder::list_input_devices()
}

/// 选择输入设备（`None` = 关闭输入流）。选定后输入电平表即可工作。
///
/// **不等待**音频线程确认：这条命令在用户选设备 / 开录时被调用，那时音频线程可能正在
/// 装载带音频片段的工程，等它只会白等出「等待音频线程超时」。设备真打不开时电平表会一直是 0。
///
/// 界面**不会**在启动时自动调用它——麦克风只在用户显式选设备或开始录音时才被占用。
#[tauri::command]
pub fn set_input_device(state: State<'_, AppState>, device: Option<String>) -> Result<(), String> {
    match device {
        Some(name) => state.audio.arm_input(Some(name), false),
        None => {
            state.audio.disarm_input();
            Ok(())
        }
    }
}

/// 输入电平表：最近 `points` 个采样（供界面画条）。
#[tauri::command]
pub fn input_level(state: State<'_, AppState>, points: usize) -> Vec<f32> {
    state.audio.input_scope_snapshot(points)
}

/// 开始录音：打开输入设备、开始采集，并从播放头走带。
#[tauri::command]
pub fn record_start(
    state: State<'_, AppState>,
    track: u64,
    start: f64,
    device: Option<String>,
) -> Result<(), String> {
    record_start_impl(&state, track, start, device)
}

/// 开始录音的共用实现（界面命令与 MCP 工具共用）。
pub(crate) fn record_start_impl(
    state: &AppState,
    track: u64,
    start: f64,
    device: Option<String>,
) -> Result<(), String> {
    // 录音前挂载输入流：这里**要等**确认，设备打不开才好立刻提示。
    state.audio.arm_input(device, true)?;
    state.audio.start_recording()?;
    state.set_recording(Some(crate::state::RecordingTarget { track: TrackId(track), start }));
    let _ = state.audio.send(TransportCommand::Play);
    Ok(())
}

/// 结束录音：停带、写 WAV、把片段挂到轨道上（一个撤销步骤）。没录到内容返回 `None`。
#[tauri::command]
pub async fn record_stop(app: tauri::AppHandle) -> Result<Option<Project>, String> {
    tauri::async_runtime::spawn_blocking(move || record_stop_as(&app, Actor::Human))
        .await
        .map_err(|e| e.to_string())?
}

/// 结束录音的共用实现（界面署名 `Human`，MCP 用代理署名）。
pub(crate) fn record_stop_as(
    app: &tauri::AppHandle,
    actor: Actor,
) -> Result<Option<Project>, String> {
    let state = app.state::<AppState>();
    let _ = state.audio.send(TransportCommand::Pause);
    let target = state.take_recording();
    let captured = state.audio.stop_recording()?;
    let (Some(target), Some(audio)) = (target, captured) else {
        return Ok(None);
    };
    if audio.frames() == 0 {
        return Ok(None);
    }

    // 落盘到「应用数据目录 /recordings/rec-<epochms>.wav」。
    let path = recording_path(app)?;
    let audio = Arc::new(audio);
    let rate = audio.sample_rate.round().max(1.0) as u32;
    {
        let mut buffer = AudioBuffer::silence(audio.channels, audio.frames());
        buffer.data_mut().copy_from_slice(&audio.samples);
        ai_daw_engine::wav::write_wav(&path, &buffer, rate).map_err(|e| e.to_string())?;
    }

    // 免去立刻重解码，并追加一条可撤销的音频片段。
    state.assets.insert(&path, Arc::clone(&audio));
    let start = ai_daw_engine::model::Clip::normalize_start(target.start);
    let length = {
        let engine = state.engine.lock().unwrap();
        natural_length_beats(engine.project(), start, audio.frames() as f64 / audio.sample_rate)
    };
    Ok(Some(run_agent_as(
        app,
        actor,
        Command::AddAudioClip {
            track: target.track,
            start,
            length,
            source: AudioSource { path, offset_frames: 0, frames: Some(audio.frames() as u64), rate: 1.0, pitch_semitones: 0.0 },
        },
    ).0))
}

/// 录音文件路径：应用数据目录下的 `recordings/rec-<epochms>.wav`。
fn recording_path(app: &tauri::AppHandle) -> Result<String, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("拿不到应用数据目录：{e}"))?
        .join("recordings");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    Ok(dir.join(format!("rec-{stamp}.wav")).to_string_lossy().into_owned())
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
///
/// 一并扫描**插件管理器**的托管目录，所以「装完新插件点重扫」能看到新装的东西。
#[tauri::command]
pub async fn rescan_plugins(app: tauri::AppHandle) -> Result<Vec<CatalogPlugin>, String> {
    tauri::async_runtime::spawn_blocking(move || crate::manager::rescan_with_managed(&app))
        .await
        .map_err(|e| e.to_string())
}

/// 取某个插件的参数表（内置插件从注册表拿，外部插件按目录加载）。
///
/// **会加载插件**（外部插件要 dlopen），只能在阻塞线程上调用。抽成普通函数是为了让
/// `describe_plugin` 这个 MCP 工具复用同一份逻辑——两个入口看到同一张参数表。
pub(crate) fn plugin_params_with(
    catalog: &[CatalogPlugin],
    plugin_id: &str,
) -> Result<Vec<ParamInfo>, String> {
    if let Some(plugin) =
        PluginRegistry::with_builtins().create(plugin_id, TrackId(0), PluginSlot::Instrument)
    {
        return Ok(plugin.params());
    }
    let entry = catalog
        .iter()
        .find(|e| e.id == plugin_id)
        .ok_or_else(|| format!("未知插件：{plugin_id}"))?;
    let plugin = ai_daw_host_vst3::load_catalog_plugin(entry).map_err(|e| e.to_string())?;
    Ok(plugin.params())
}

/// 取某个插件的参数表（内置插件从注册表拿，外部插件按目录加载）。
#[tauri::command]
pub async fn plugin_params(
    state: State<'_, AppState>,
    plugin_id: String,
) -> Result<Vec<ParamInfo>, String> {
    let catalog = state.catalog_snapshot();
    tauri::async_runtime::spawn_blocking(move || plugin_params_with(&catalog, &plugin_id))
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

/// 一次设置效果器的**多个**参数（「恢复默认值」走它：一次事务，只占一个撤销步骤）。
#[tauri::command]
pub fn set_effect_params(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    index: usize,
    params: HashMap<String, f32>,
) -> Project {
    // 按参数名排序：`HashMap` 的遍历顺序随机，固定顺序才能让事件流与撤销历史可复现。
    let mut pairs: Vec<(String, f32)> = params.into_iter().collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    let id = TrackId(track);
    let commands: Vec<Command> = pairs
        .into_iter()
        .map(|(param, value)| Command::SetEffectParam { track: id, index, param, value })
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

// ---------------------------------------------------------------------------
// 插件原生界面（独立窗口）
// ---------------------------------------------------------------------------

/// 槽位在宿主窗口 label 里的短标记：`i` = 乐器，`f<n>` = 效果链第 n 个。
fn slot_tag(slot: PluginSlot) -> String {
    match slot {
        PluginSlot::Instrument => "i".to_string(),
        PluginSlot::Effect(index) => format!("f{index}"),
    }
}

/// 宿主给「只能嵌入」的插件开的那扇窗的 label（含槽位，一轨可同时开多个）。
pub fn host_window_label(track: u64, slot: PluginSlot) -> String {
    format!("plugin-ui-{track}-{}", slot_tag(slot))
}

/// [`host_window_label`] 的反函数：从窗口 label 认回 (轨道, 槽位)。
pub fn host_window_track(label: &str) -> Option<(u64, PluginSlot)> {
    let rest = label.strip_prefix("plugin-ui-")?;
    let (track, tag) = rest.split_once('-')?;
    let track: u64 = track.parse().ok()?;
    let slot = match tag.strip_prefix('f') {
        Some(n) => PluginSlot::Effect(n.parse().ok()?),
        None if tag == "i" => PluginSlot::Instrument,
        None => return None,
    };
    Some((track, slot))
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
    slot: PluginSlot,
    title: &str,
    width: u32,
    height: u32,
) -> Result<tauri::WebviewWindow, String> {
    let label = host_window_label(track, slot);
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

/// 轨道显示名（找不到就退回 `轨道 <id>`）。
fn track_name(state: &AppState, track: u64) -> String {
    state
        .engine
        .lock()
        .unwrap()
        .project()
        .track(TrackId(track))
        .map(|t| t.name.clone())
        .unwrap_or_else(|| format!("轨道 {track}"))
}

/// 把某个槽位已经开着的插件窗口带到最前面（**必须在主线程**）。
///
/// 原生窗口（VST3 的 `PluginWindow`、CLAP 的浮动窗口）由插件自己管理，没有统一的聚焦 API；
/// 这里保证**嵌入模式的宿主窗口**被抬起，插件自己的窗口是否跟随交给插件。
fn focus_plugin_window(app: &tauri::AppHandle, track: u64, slot: PluginSlot) {
    let (tx, rx) = std::sync::mpsc::channel();
    let app_for_main = app.clone();
    if app
        .run_on_main_thread(move || {
            if let Some(window) = app_for_main.get_webview_window(&host_window_label(track, slot)) {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
            let _ = tx.send(());
        })
        .is_ok()
    {
        let _ = rx.recv();
    }
}

/// 按 plugin id 在**权威的**效果链里现算序号。
///
/// 调用方（前端）可能拿着一条**过时的效果链**——别的窗口刚改过它。按 id 现算而不是听调用方
/// 给的序号，才能开到正确的槽位上。找不到就明确报错，而不是开出一扇空窗。
fn resolve_effect_index(state: &AppState, track: u64, plugin_id: &str) -> Result<usize, String> {
    let engine = state.engine.lock().unwrap();
    engine
        .project()
        .track(TrackId(track))
        .and_then(|t| t.effects.iter().position(|fx| fx.plugin_id == plugin_id))
        .ok_or_else(|| "这个效果器已经不在效果链上了".to_string())
}

/// 为某个槽位打开插件的原生界面（**已经开着就把它带到最前面**，不再开新窗）。
///
/// `title` 是插件展示名（界面按当前语言给出）；宿主再拼上轨道名，于是窗口标题是
/// 「轨道名 - 插件名」，用户一眼能看出这扇窗是从哪条轨道开的。
///
/// `plugin_id` 用来记录「这扇窗是为哪个插件开的」：重建后据此判断它是否还有效（插件没换就不关）。
///
/// 窗口必须在主线程创建（macOS 的 `NSWindow` 只能在那里建），所以走 `run_on_main_thread`，
/// 并把结果通过 channel 带回来。
fn open_slot_editor(
    app: tauri::AppHandle,
    state: &AppState,
    track: u64,
    slot: PluginSlot,
    plugin_id: String,
    title: String,
) -> Result<(), String> {
    let key = (track, slot);
    if state.open_editors.lock().unwrap().contains(&key) {
        // 已经开着：把它带到最前面，而不是无动作——否则用户点「打开界面」毫无反应。
        focus_plugin_window(&app, track, slot);
        return Ok(());
    }
    let handle = state
        .editor_handle(track, slot)
        .ok_or_else(|| "这个槽位没有可打开的插件界面".to_string())?;
    let title = format!("{} - {}", track_name(state, track), title);

    let (tx, rx) = std::sync::mpsc::channel();
    let app_for_main = app.clone();
    app.run_on_main_thread(move || {
        // 问插件：它的界面需要宿主提供一扇窗吗？
        // KR-106 的 CLAP 就是这种——它的 GUI 只支持「嵌入」，得我们给它一个原生视图。
        let host_size = handle.host_window_size();
        let host_window = match host_size {
            Some((width, height)) => {
                match build_host_window(&app_for_main, track, slot, &title, width, height) {
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
                    windows.insert(key, window);
                });
                let state = app_for_main.state::<AppState>();
                state.open_editors.lock().unwrap().insert(key);
                state.editor_plugins.lock().unwrap().insert(key, plugin_id);
                if host_size.is_some() {
                    state.host_windows.lock().unwrap().insert(key);
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

/// 为某条轨道打开**乐器**的原生界面（已经开着就聚焦）。
#[tauri::command]
pub fn open_plugin_window(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    title: String,
) -> Result<(), String> {
    // 记下这扇窗是**为哪个乐器**开的：重建后据此判断它是否还有效（乐器没换就不关）。
    let plugin_id = {
        let engine = state.engine.lock().unwrap();
        engine
            .project()
            .track(TrackId(track))
            .and_then(|t| t.instrument.as_ref())
            .map(|instrument| instrument.plugin_id.clone())
            .unwrap_or_default()
    };
    open_slot_editor(app, &state, track, PluginSlot::Instrument, plugin_id, title)
}

/// 为某条轨道上的**某个效果器**打开它**自带的**原生界面（已经开着就聚焦）。
///
/// 与 [`open_effect_window`]（内置面板窗口）不同：这条命令开的是插件自己的窗口。
#[tauri::command]
pub fn open_effect_plugin_window(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    plugin_id: String,
    title: String,
) -> Result<(), String> {
    let index = resolve_effect_index(&state, track, &plugin_id)?;
    open_slot_editor(app, &state, track, PluginSlot::Effect(index), plugin_id, title)
}

/// 关掉某个槽位的插件窗口，但**保留编辑器句柄**（用户只是关界面，之后还要能再开）。
///
/// 与 [`close_plugin_windows_for`] 的区别：那个用于「实例已经不在」的收尾，会连句柄一并清掉。
fn close_slot_window(app: &tauri::AppHandle, key: (u64, PluginSlot)) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let app_for_main = app.clone();
    app.run_on_main_thread(move || {
        let window = with_editor_windows(|windows| windows.remove(&key));
        if let Some(window) = window {
            window.close();
        }
        let state = app_for_main.state::<AppState>();
        state.open_editors.lock().unwrap().remove(&key);
        state.editor_plugins.lock().unwrap().remove(&key);
        // 嵌入模式的界面住在我们建的宿主窗口里，一并收掉。
        if state.host_windows.lock().unwrap().remove(&key) {
            if let Some(window) = app_for_main.get_webview_window(&host_window_label(key.0, key.1)) {
                let _ = window.close();
            }
        }
        let _ = tx.send(());
    })
    .map_err(|e| e.to_string())?;

    let _ = rx.recv();
    Ok(())
}

/// 关闭某条轨道的**乐器**窗口（保留编辑器句柄，之后还能再开）。
#[tauri::command]
pub fn close_plugin_window(app: tauri::AppHandle, track: u64) -> Result<(), String> {
    close_slot_window(&app, (track, PluginSlot::Instrument))
}

/// 关闭某条轨道上某个**效果器**的原生窗口（保留编辑器句柄，之后还能再开）。
#[tauri::command]
pub fn close_effect_plugin_window(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    plugin_id: String,
) -> Result<(), String> {
    let index = resolve_effect_index(&state, track, &plugin_id)?;
    close_slot_window(&app, (track, PluginSlot::Effect(index)))
}

/// 某条轨道的**乐器**窗口是否开着（用户自己点关闭按钮后会变回 false）。
#[tauri::command]
pub fn plugin_window_open(state: State<'_, AppState>, track: u64) -> bool {
    state.open_editors.lock().unwrap().contains(&(track, PluginSlot::Instrument))
}

/// 某条轨道上某个**效果器**的原生窗口是否开着。效果器不在链上时返回 false。
#[tauri::command]
pub fn effect_plugin_window_open(
    state: State<'_, AppState>,
    track: u64,
    plugin_id: String,
) -> bool {
    match resolve_effect_index(&state, track, &plugin_id) {
        Ok(index) => state.open_editors.lock().unwrap().contains(&(track, PluginSlot::Effect(index))),
        Err(_) => false,
    }
}

/// 开一扇「内置插件面板」窗口（已经开着就聚焦过去）。
///
/// 两块内置面板（合成器 / 效果器）共用这条路径：同一套前端换一条路由。
/// 面板是**流体布局**（铺满窗口），所以允许缩放——只给一个下限，免得控件被压扁变形。
async fn open_panel_window(
    app: tauri::AppHandle,
    label: String,
    url: String,
    title: String,
    size: (f64, f64),
    min: (f64, f64),
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(&label) {
        // 已经开着：抬起并聚焦，不再开新窗（最小化时也要先还原）。
        let _ = window.unminimize();
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
            tauri::WebviewUrl::App(url.into()),
        )
        .title(title)
        .inner_size(size.0, size.1)
        .min_inner_size(min.0, min.1)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string());
        let _ = tx.send(result);
    })
    .map_err(|e| e.to_string())?;

    rx.recv().map_err(|e| e.to_string())?
}

/// 打开内置合成器的**独立窗口**（已经开着就聚焦过去）。
///
/// 内置合成器没有原生编辑器，但它的面板也不该只能挤在侧边栏里——给它一个自己的窗口，
/// 「打开界面」这个动作对所有音色就都成立。窗口标题是「轨道名 - 插件名」。
#[tauri::command]
pub async fn open_synth_window(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    title: String,
) -> Result<(), String> {
    let title = format!("{} - {}", track_name(&state, track), title);
    open_panel_window(
        app,
        format!("synth-{track}"),
        format!("index.html#/synth?track={track}"),
        title,
        (960.0, 680.0),
        (780.0, 560.0),
    )
    .await
}

/// 打开某条轨道上**某个效果器**的独立界面。
///
/// 效果器参数不在检查器里就地调：链上效果一多，检查器会被几十个滑块塞满。
/// 与内置合成器同一个约定——检查器只给入口，编辑去它自己的界面。
#[tauri::command]
pub async fn open_effect_window(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    plugin_id: String,
    title: String,
) -> Result<(), String> {
    // **以 plugin id 为准在链里现算序号**，而不是听调用方的：调用方（主窗口的检查器）
    // 可能拿着一条**过时的效果链**——别的窗口（例如合成器窗口切预设）刚把整条链换掉，
    // 主窗口还没刷新。这时按它给的序号开窗就会开到不存在的槽位上。
    //
    // 找不到就直说，而不是开出一扇「这条轨道上没有这个效果器」的空窗。
    let position = {
        let engine = state.engine.lock().unwrap();
        engine
            .project()
            .track(TrackId(track))
            .and_then(|t| t.effects.iter().position(|fx| fx.plugin_id == plugin_id))
    };
    let Some(index) = position else {
        return Err("这个效果器已经不在效果链上了".into());
    };

    // 窗口 label 与 URL 都用 plugin id：链怎么变，同一个效果器都对应同一扇窗。
    let plugin_arg = format!("&plugin={}", encode_query_value(&plugin_id));
    let key = format!("{:016x}", fnv1a(&plugin_id));
    let title = format!("{} - {}", track_name(&state, track), title);
    open_panel_window(
        app,
        format!("effect-{track}-{key}"),
        format!("index.html#/effect?track={track}&index={index}{plugin_arg}"),
        title,
        (560.0, 400.0),
        (460.0, 320.0),
    )
    .await
}

/// 稳定的 64 位 FNV-1a 哈希（把插件 id 压成 label 里可用的十六进制）。
fn fnv1a(value: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// 把字符串安全地放进 URL 查询参数（保留 URL 的非保留字符，其余百分号转义）。
fn encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
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

// ---------------------------------------------------------------------------
// 音色记忆库（跨工程）
// ---------------------------------------------------------------------------

/// 全部音色记忆的摘要（不含参数/状态，界面列表用）。
#[tauri::command]
pub fn list_sound_memories(
    state: State<'_, AppState>,
) -> Vec<crate::sound_memories::SoundMemorySummary> {
    state.sound_memories.lock().unwrap().summaries()
}

/// 把当前轨道的音色链存成一条记忆；传 `id` 表示覆盖该条记忆的内容。
#[tauri::command]
pub fn save_sound_memory(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    name: String,
    description: Option<String>,
    tags: Option<Vec<String>>,
    id: Option<String>,
) -> Result<crate::sound_memories::SoundMemorySummary, String> {
    let snapshot = sound_snapshot_for(&state, TrackId(track))?;
    let meta = MemoryMeta {
        name,
        description: description.unwrap_or_default(),
        tags: tags.unwrap_or_default(),
        role: snapshot.role.clone(),
    };
    let memory = {
        let mut store = state.sound_memories.lock().unwrap();
        match id {
            Some(id) => store.replace(&id, meta, snapshot)?,
            None => store.create(meta, snapshot)?,
        }
    };
    notify_sound_memories(&app);
    Ok(memory.summary())
}

/// 只修改一条记忆的元数据（名称/描述/标签/分工），不动声音快照。
#[tauri::command]
pub fn update_sound_memory(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    name: Option<String>,
    description: Option<String>,
    tags: Option<Vec<String>>,
    role: Option<String>,
) -> Result<crate::sound_memories::SoundMemorySummary, String> {
    let memory = state.sound_memories.lock().unwrap().update_metadata(
        &id,
        name.as_deref(),
        description.as_deref(),
        tags,
        role.as_deref(),
    )?;
    notify_sound_memories(&app);
    Ok(memory.summary())
}

/// 删除一条音色记忆。
#[tauri::command]
pub fn delete_sound_memory(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state.sound_memories.lock().unwrap().remove(&id)?;
    notify_sound_memories(&app);
    Ok(())
}

/// 把一条音色记忆应用到轨道（一次事务：乐器 + 效果链 + 增益/声像）。
#[tauri::command]
pub fn apply_sound_memory(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    track: u64,
) -> Result<Project, String> {
    let memory = state
        .sound_memories
        .lock()
        .unwrap()
        .get(&id)
        .ok_or_else(|| format!("音色记忆不存在：{id}"))?;
    let (project, events) =
        apply_sound_memory_inner(&state, TrackId(track), &memory, Actor::Human)?;
    for event in &events {
        let _ = app.emit(EVENT_ENGINE, event);
    }
    state.audio.sync_events(&project, &events);
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 改速度时，音频片段的「拍长」重算成它的原真实秒数在新速度下的拍数——方框跟得上，
    /// 音频原速。
    #[test]
    fn tempo_change_resizes_audio_boxes_to_keep_seconds() {
        use ai_daw_engine::model::{Clip, MixerChannel, Track};
        let mut p = Project::new("t");
        p.transport.tempo = 120.0;
        p.tracks = vec![Track {
            meta: None,
            id: TrackId(1),
            name: "a".into(),
            instrument: None,
            effects: Vec::new(),
            clips: vec![Clip {
                id: ClipId(1),
                start: 0.0,
                length: 4.0, // 120 BPM 下 = 2 s
                notes: Vec::new(),
                audio: Some(AudioSource {
                    path: "x".into(),
                    offset_frames: 0,
                    frames: None,
                    rate: 1.0,
                    pitch_semitones: 0.0,
                }),
            }],
            mixer: MixerChannel { gain_db: 0.0, pan: 0.0, mute: false, solo: false },
            sends: Vec::new(),
            automation: Vec::new(),
        }];
        let old = p.transport.clone();
        let mut new = old.clone();
        new.tempo = 60.0; // 同样的 2 s 现在应占 2 拍
        let cmds = resize_audio_clips_for_transport(&p, &old, &new);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            Command::ResizeClip { length, .. } => assert!((length - 2.0).abs() < 1e-6, "length={length}"),
            other => panic!("expected ResizeClip, got {other:?}"),
        }
        // 速度不变时不动片段。
        assert!(resize_audio_clips_for_transport(&p, &old, &old).is_empty());
    }

    /// 离线渲染**绝不能**共用实时播放的编辑器句柄表。
    ///
    /// 插件工厂在创建实例时会把句柄登记进传进来的那张表。离线渲染（预览 / 导出 / 分析 / MCP）
    /// 会为每条轨道新建一套一次性实例——如果它拿的是实时那张表，就会把正在发声的实例的句柄顶掉，
    /// 之后「打开界面」开到的是个不在音频图里的实例：界面能开、旋钮能动，**却听不到声音**。
    #[test]
    fn offline_render_does_not_share_the_live_editor_handles() {
        let state = AppState::new("t");
        let inputs = render_inputs(&state);

        assert!(
            !Arc::ptr_eq(&inputs.editor_handles, &state.editor_handles),
            "离线渲染必须用自己的一次性句柄表"
        );
        assert!(
            inputs.editor_handles.lock().unwrap().is_empty(),
            "一次性句柄表应当是空的"
        );
    }

    // ---- 拍 ↔ 素材帧的换算（必须带上片段的播放速度 rate）----

    /// 120 BPM：1 拍 = 0.5 s，1 s = 2 拍。素材按 48 kHz 计。
    fn map120() -> TempoMap {
        TempoMap::from_transport(&Transport::default())
    }

    /// 回归：裁剪 / 移动时，方框吃掉的素材量 = 方框秒数 × `rate`。
    ///
    /// 早先这里漏了 `rate`，于是「改了播放速度以后再拖动片段」会把窗口设成一半
    /// （`rate = 2` 时只播前半段，后半段静音）——听感就是「变速之后没法用了」。
    #[test]
    fn trimming_consumes_material_in_proportion_to_playback_rate() {
        let map = map120();
        let total = 384_000; // 8 s 素材
        // 4 拍 = 2 s 时间轴。
        let (offset, frames) = trimmed_window(&map, 0.0, 0.0, 4.0, 0, total, 48_000.0, 2.0);
        assert_eq!(offset, 0);
        assert_eq!(frames, 192_000, "rate=2：2 s 时间轴要吃 4 s 素材");
        // 原速仍是 2 s（与改动前逐位一致）。
        let (_, frames1) = trimmed_window(&map, 0.0, 0.0, 4.0, 0, total, 48_000.0, 1.0);
        assert_eq!(frames1, 96_000);
        // 放慢到 0.5×：只吃 1 s。
        let (_, frames_half) = trimmed_window(&map, 0.0, 0.0, 4.0, 0, total, 48_000.0, 0.5);
        assert_eq!(frames_half, 48_000);
    }

    /// 整体平移片段时，素材偏移同样按 `rate` 走；拖出素材开头会被夹住。
    #[test]
    fn moving_a_clip_shifts_the_window_by_playback_rate() {
        let map = map120();
        let total = 384_000;
        // 从第 0 拍移到第 4 拍（时间轴 +2 s）：rate=2 → 素材偏移 +4 s。
        let (offset, _) = trimmed_window(&map, 0.0, 4.0, 4.0, 0, total, 48_000.0, 2.0);
        assert_eq!(offset, 192_000);
        let (offset1, _) = trimmed_window(&map, 0.0, 4.0, 4.0, 0, total, 48_000.0, 1.0);
        assert_eq!(offset1, 96_000);
        // 反向拖出素材开头：夹到 0，不出现负偏移。
        let (clamped, _) = trimmed_window(&map, 4.0, 0.0, 4.0, 0, total, 48_000.0, 2.0);
        assert_eq!(clamped, 0);
    }

    /// 回归：可拖范围也要按 `rate` 折算，否则右边缘能拖到素材早已放完之后，露出一段静音。
    #[test]
    fn clip_bounds_account_for_playback_rate() {
        let map = map120();
        // 第 0 拍的片段、8 s 素材：rate=2 时只能向右展开 4 s 时间轴 = 第 8 拍。
        let (min_start, max_end) = clip_bounds_for(&map, 0.0, 0, 384_000, 48_000.0, 2.0);
        assert_eq!(min_start, 0.0);
        assert!((max_end - 8.0).abs() < 1e-9, "rate=2 应到第 8 拍，实际 {max_end}");
        // 原速：展开到 8 s = 第 16 拍。
        let (_, max_end1) = clip_bounds_for(&map, 0.0, 0, 384_000, 48_000.0, 1.0);
        assert!((max_end1 - 16.0).abs() < 1e-9, "rate=1 应到第 16 拍，实际 {max_end1}");
        // 已消耗的素材也除以 rate：偏移 2 s 素材在 rate=2 下只占 1 s 时间轴。
        let (min2, _) = clip_bounds_for(&map, 4.0, 96_000, 384_000, 48_000.0, 2.0);
        assert!((min2 - 2.0).abs() < 1e-9, "第 4 拍（2 s）左移 1 s → 第 2 拍，实际 {min2}");
    }

    /// 音色记忆：从轨道抓快照 → 应用回去，乐器/效果链/增益声像逐值一致，且只占一个撤销步。
    #[test]
    fn sound_memory_snapshot_and_apply_roundtrip() {
        use std::collections::BTreeMap;

        let state = AppState::new("t");
        let track = {
            let engine = state.engine.lock().unwrap();
            engine.project().tracks[0].id
        };
        {
            let mut engine = state.engine.lock().unwrap();
            let events = engine.execute_all(
                vec![
                    Command::SetTrackInstrument {
                        track,
                        plugin_id: "builtin.synth".into(),
                    },
                    Command::SetParam {
                        track,
                        param: "attack".into(),
                        value: 0.25,
                    },
                    Command::AddEffect {
                        track,
                        plugin_id: "builtin.reverb".into(),
                    },
                    Command::SetEffectParam {
                        track,
                        index: 0,
                        param: "mix".into(),
                        value: 0.4,
                    },
                    Command::AddEffect {
                        track,
                        plugin_id: "builtin.delay".into(),
                    },
                    Command::SetGain {
                        track,
                        gain_db: -3.0,
                    },
                    Command::SetPan { track, pan: 0.2 },
                ],
                Actor::Human,
            );
            assert!(!events.is_empty());
        }

        let snapshot = sound_snapshot_for(&state, track).unwrap();
        assert_eq!(
            snapshot.instrument.as_ref().unwrap().plugin_id,
            "builtin.synth"
        );
        assert_eq!(snapshot.instrument.as_ref().unwrap().params["attack"], 0.25);
        assert_eq!(snapshot.effects.len(), 2);
        assert_eq!(snapshot.effects[0].plugin_id, "builtin.reverb");
        assert_eq!(snapshot.effects[1].plugin_id, "builtin.delay");
        assert_eq!(snapshot.mixer.gain_db, -3.0);
        assert_eq!(snapshot.mixer.pan, 0.2);

        let memory = SoundMemory {
            id: "s1".into(),
            name: "test".into(),
            description: String::new(),
            tags: Vec::new(),
            role: None,
            instrument: Some(PluginSnapshot {
                plugin_id: "builtin.synth".into(),
                params: BTreeMap::from([("cutoff".into(), 900.0)]),
                state: String::new(),
            }),
            effects: vec![PluginSnapshot {
                plugin_id: "builtin.chorus".into(),
                params: BTreeMap::from([("mix".into(), 0.5)]),
                state: String::new(),
            }],
            mixer: SoundMixer {
                gain_db: -9.0,
                pan: -0.5,
            },
            source_project: None,
            created_at: 0,
            updated_at: 0,
        };

        let depth_before = state.engine.lock().unwrap().undo_depth();
        let (project, events) =
            apply_sound_memory_inner(&state, track, &memory, Actor::Human).unwrap();
        assert!(!events.is_empty());
        assert_eq!(state.engine.lock().unwrap().undo_depth(), depth_before + 1);

        let t = project.tracks.iter().find(|t| t.id == track).unwrap();
        assert_eq!(t.instrument.as_ref().unwrap().plugin_id, "builtin.synth");
        assert_eq!(t.instrument.as_ref().unwrap().params["cutoff"], 900.0);
        assert_eq!(t.effects.len(), 1, "旧效果链必须被整体替换");
        assert_eq!(t.effects[0].plugin_id, "builtin.chorus");
        assert_eq!(t.effects[0].params["mix"], 0.5);
        assert_eq!(t.mixer.gain_db, -9.0);
        assert_eq!(t.mixer.pan, -0.5);

        // 不存在的轨道必须明确报错，不能静默成功。
        assert!(apply_sound_memory_inner(&state, TrackId(9999), &memory, Actor::Human).is_err());
    }
}
