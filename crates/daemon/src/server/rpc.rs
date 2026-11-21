//! 命令名 → 实现函数的唯一映射表。
//!
//! 每迁移一组命令，就在这里加一组分支；**没有分支的命令返回 400**，不会静默成功——
//! 浏览器端拿到的是明确的「这条命令还没在 HTTP 上开放」，而不是一个空结果。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::Value;

use ai_daw_engine::command::Command;
use ai_daw_engine::model::{ClipId, Note, TrackId};

use crate::state::AppState;

/// `/rpc` 的错误。
pub enum RpcError {
    /// 命令名不在分发表里。
    Unknown(String),
    /// JSON 参数解不开（缺字段 / 类型不对）。
    BadArgs(String),
    /// 命令执行本身失败（例如没有音频设备）。
    Failed(String),
    /// 命令存在，但只有桌面端能做到（原生窗口 / 系统集成 / 后台运行时）。
    Unsupported(String),
    /// 令牌缺失或不匹配。
    Unauthorized,
}

impl RpcError {
    /// 给人/给扩展看的一句话错误（与 HTTP 响应体里的 `error` 同义）。
    pub fn message(&self) -> String {
        match self {
            Self::Unknown(cmd) => format!("unknown command: {cmd}"),
            Self::BadArgs(err) => format!("bad args: {err}"),
            Self::Failed(err) => err.clone(),
            Self::Unsupported(why) => why.clone(),
            Self::Unauthorized => "unauthorized".into(),
        }
    }
}

impl IntoResponse for RpcError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            RpcError::Unknown(cmd) => (StatusCode::BAD_REQUEST, format!("unknown command: {cmd}")),
            RpcError::BadArgs(err) => (StatusCode::BAD_REQUEST, format!("bad args: {err}")),
            RpcError::Failed(err) => (StatusCode::INTERNAL_SERVER_ERROR, err),
            RpcError::Unsupported(why) => (StatusCode::NOT_IMPLEMENTED, why),
            RpcError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".into()),
        };
        (status, axum::Json(serde_json::json!({ "error": message }))).into_response()
    }
}

/// 把 JSON 参数解析成具体类型。
fn parse<T: for<'de> Deserialize<'de>>(args: Value) -> Result<T, RpcError> {
    serde_json::from_value(args).map_err(|err| RpcError::BadArgs(err.to_string()))
}

/// 命令执行结果的序列化。
fn json<T: serde::Serialize>(value: T) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(|err| RpcError::Failed(err.to_string()))
}

/// 跑一条编辑命令（走与 Tauri 完全相同的编辑管线）。
fn edit(state: &AppState, command: Command) -> Result<Value, RpcError> {
    json(crate::commands::run_one(state, command))
}

#[derive(Deserialize)]
struct TrackArg {
    track: u64,
}

#[derive(Deserialize)]
struct AddTrackArgs {
    name: String,
}

#[derive(Deserialize)]
struct RenameTrackArgs {
    track: u64,
    name: String,
}

#[derive(Deserialize)]
struct InstrumentArgs {
    track: u64,
    plugin_id: String,
}

#[derive(Deserialize)]
struct TempoArgs {
    bpm: f64,
}

#[derive(Deserialize)]
struct TimeSigArgs {
    num: u8,
    den: u8,
}

#[derive(Deserialize)]
struct NameArgs {
    name: String,
}

#[derive(Deserialize)]
struct AddClipArgs {
    track: u64,
    start: f64,
    length: f64,
}

#[derive(Deserialize)]
struct ClipArgs {
    track: u64,
    clip: u64,
}

#[derive(Deserialize)]
struct ClipStartArgs {
    track: u64,
    clip: u64,
    start: f64,
}

#[derive(Deserialize)]
struct ClipLengthArgs {
    track: u64,
    clip: u64,
    length: f64,
}

#[derive(Deserialize)]
struct AddNoteArgs {
    track: u64,
    clip: u64,
    note: Note,
}

#[derive(Deserialize)]
struct NoteIndexArgs {
    track: u64,
    clip: u64,
    index: usize,
}

#[derive(Deserialize)]
struct SetNoteArgs {
    track: u64,
    clip: u64,
    index: usize,
    note: Note,
}

#[derive(Deserialize)]
struct GainArgs {
    track: u64,
    gain_db: f32,
}

#[derive(Deserialize)]
struct PanArgs {
    track: u64,
    pan: f32,
}

#[derive(Deserialize)]
struct MuteArgs {
    track: u64,
    mute: bool,
}

#[derive(Deserialize)]
struct SoloArgs {
    track: u64,
    solo: bool,
}

#[derive(Deserialize)]
struct MasterGainArgs {
    gain_db: f32,
}

#[derive(Deserialize)]
struct QuantizeArgs {
    track: u64,
    clip: u64,
    grid: f64,
    #[serde(default = "one")]
    strength: f64,
}

fn one() -> f64 {
    1.0
}

#[derive(Deserialize)]
struct TransposeArgs {
    track: u64,
    clip: u64,
    semitones: i32,
}


#[derive(Deserialize)]
struct LoopArgs {
    on: bool,
}

#[derive(Deserialize)]
struct LoopRangeArgs {
    start: u64,
    end: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WaveformArgs {
    path: String,
    #[serde(default = "default_buckets")]
    buckets: usize,
    #[serde(default)]
    offset_frames: Option<u64>,
    #[serde(default)]
    frames: Option<u64>,
}

fn default_buckets() -> usize {
    512
}

/// `import_audio` 的参数：解码是异步的，由 HTTP 处理器单独 await（见 `server::rpc_handler`）。
#[derive(Deserialize)]
pub struct ImportAudioArgs {
    pub path: String,
    pub track: u64,
    #[serde(default)]
    pub start: f64,
}

#[derive(Deserialize)]
struct ClipRefArgs {
    track: u64,
    clip: u64,
}

#[derive(Deserialize)]
struct TrackClipArgs {
    track: u64,
    #[serde(default)]
    clip: Option<u64>,
}

#[derive(Deserialize)]
struct PointsArgs {
    #[serde(default = "default_points")]
    points: usize,
}

fn default_points() -> usize {
    512
}

#[derive(Deserialize)]
struct SeekArgs {
    frame: u64,
}

/// 分发一条命令。**与 Tauri 命令同名同参**，前端的两套传输层可以共用一份调用代码。
pub fn dispatch(server: &super::ServerState, cmd: &str, args: Value) -> Result<Value, RpcError> {
    let state: &AppState = server.daemon.state();
    // 先问「按 Tauri 签名写的那张表」：工程 / 插件 / 设备 / 分析这些命令都躺在那里。
    // 它们和桌面端执行的是同一份实现，网页端不该有第二套行为。
    if let Some(result) = super::cmds::call(&server.app_handle(), cmd, args.clone()) {
        return result;
    }
    match cmd {
        // ---- 只读 ----
        "get_project" => json(crate::commands::get_project_impl(state)),
        "project_revision" => json(state.engine.lock().unwrap().revision()),
        "history_depths" => json(crate::commands::history_depths_impl(state)),
        "transport_status" => json(crate::commands::transport_status_impl(state)),
        "mixer_meters" => json(crate::commands::mixer_meters_impl(state)),
        "audio_waveform" => {
            let a: WaveformArgs = parse(args)?;
            let slice =
                crate::commands::audio_waveform_impl(state, &a.path, a.buckets, a.offset_frames, a.frames)
                    .map_err(RpcError::Failed)?;
            json(slice)
        }
        "current_project_path" => json(crate::commands::current_project_path_impl(state)),
        "session_restored" => json(crate::commands::session_restored_impl(state)),
        "list_plugins" => json(crate::commands::list_plugins_impl(state)),
        "list_presets" => json(crate::commands::list_presets_impl(state)),

        // ---- 工程 ----
        "add_track" => {
            let a: AddTrackArgs = parse(args)?;
            edit(state, Command::AddTrack { name: a.name })
        }
        "remove_track" => {
            let a: TrackArg = parse(args)?;
            edit(state, Command::RemoveTrack(TrackId(a.track)))
        }
        "rename_track" => {
            let a: RenameTrackArgs = parse(args)?;
            edit(
                state,
                Command::RenameTrack { track: TrackId(a.track), name: a.name },
            )
        }
        "set_track_instrument" => {
            let a: InstrumentArgs = parse(args)?;
            edit(
                state,
                Command::SetTrackInstrument { track: TrackId(a.track), plugin_id: a.plugin_id },
            )
        }
        "set_tempo" => {
            let a: TempoArgs = parse(args)?;
            edit(state, Command::SetTempo(a.bpm))
        }
        "set_time_signature" => {
            let a: TimeSigArgs = parse(args)?;
            edit(state, Command::SetTimeSignature { num: a.num, den: a.den })
        }
        "rename_project" => {
            let a: NameArgs = parse(args)?;
            edit(state, Command::RenameProject { name: a.name })
        }

        // ---- 片段 ----
        "add_clip" => {
            let a: AddClipArgs = parse(args)?;
            edit(
                state,
                Command::AddClip { track: TrackId(a.track), start: a.start, length: a.length },
            )
        }
        "remove_clip" => {
            let a: ClipArgs = parse(args)?;
            edit(
                state,
                Command::RemoveClip { track: TrackId(a.track), clip: ClipId(a.clip) },
            )
        }
        "move_clip" => {
            let a: ClipStartArgs = parse(args)?;
            edit(
                state,
                Command::MoveClip { track: TrackId(a.track), clip: ClipId(a.clip), start: a.start },
            )
        }
        "resize_clip" => {
            let a: ClipLengthArgs = parse(args)?;
            edit(
                state,
                Command::ResizeClip {
                    track: TrackId(a.track),
                    clip: ClipId(a.clip),
                    length: a.length,
                },
            )
        }

        // ---- 音符 ----
        "add_note" => {
            let a: AddNoteArgs = parse(args)?;
            edit(
                state,
                Command::AddNote { track: TrackId(a.track), clip: ClipId(a.clip), note: a.note },
            )
        }
        "set_note" => {
            let a: SetNoteArgs = parse(args)?;
            edit(
                state,
                Command::SetNote {
                    track: TrackId(a.track),
                    clip: ClipId(a.clip),
                    index: a.index,
                    note: a.note,
                },
            )
        }
        "remove_note" => {
            let a: NoteIndexArgs = parse(args)?;
            edit(
                state,
                Command::RemoveNote {
                    track: TrackId(a.track),
                    clip: ClipId(a.clip),
                    index: a.index,
                },
            )
        }
        "quantize_notes" => {
            let a: QuantizeArgs = parse(args)?;
            edit(
                state,
                Command::QuantizeNotes {
                    track: TrackId(a.track),
                    clip: ClipId(a.clip),
                    grid: a.grid,
                    strength: a.strength,
                    range: None,
                    quantize_duration: false,
                },
            )
        }
        "transpose_notes" => {
            let a: TransposeArgs = parse(args)?;
            edit(
                state,
                Command::TransposeNotes {
                    track: TrackId(a.track),
                    clip: ClipId(a.clip),
                    semitones: a.semitones,
                    range: None,
                },
            )
        }

        // ---- 混音 ----
        "set_gain" => {
            let a: GainArgs = parse(args)?;
            edit(state, Command::SetGain { track: TrackId(a.track), gain_db: a.gain_db })
        }
        "set_pan" => {
            let a: PanArgs = parse(args)?;
            edit(state, Command::SetPan { track: TrackId(a.track), pan: a.pan })
        }
        "set_mute" => {
            let a: MuteArgs = parse(args)?;
            edit(state, Command::SetMute { track: TrackId(a.track), mute: a.mute })
        }
        "set_solo" => {
            let a: SoloArgs = parse(args)?;
            edit(state, Command::SetSolo { track: TrackId(a.track), solo: a.solo })
        }
        "set_master_gain" => {
            let a: MasterGainArgs = parse(args)?;
            edit(state, Command::SetMasterGain(a.gain_db))
        }

        // ---- 历史 ----
        "undo" => json(crate::commands::undo_impl(state)),
        "redo" => json(crate::commands::redo_impl(state)),

        // ---- 传输 ----
        "transport_play" => crate::commands::transport_play_impl(state).map_err(RpcError::Failed).map(|_| Value::Null),
        "transport_pause" => crate::commands::transport_pause_impl(state).map_err(RpcError::Failed).map(|_| Value::Null),
        "transport_stop" => crate::commands::transport_stop_impl(state).map_err(RpcError::Failed).map(|_| Value::Null),
        "transport_seek" => {
            let a: SeekArgs = parse(args)?;
            crate::commands::transport_seek_impl(state, a.frame)
                .map_err(RpcError::Failed)
                .map(|_| Value::Null)
        }

        "transport_set_loop" => {
            let a: LoopArgs = parse(args)?;
            crate::commands::transport_set_loop_impl(state, a.on)
                .map_err(RpcError::Failed)
                .map(|_| Value::Null)
        }
        "transport_set_metronome" => {
            let a: LoopArgs = parse(args)?;
            crate::commands::transport_set_metronome_impl(state, a.on)
                .map_err(RpcError::Failed)
                .map(|_| Value::Null)
        }
        "transport_set_loop_range" => {
            let a: LoopRangeArgs = parse(args)?;
            crate::commands::transport_set_loop_range_impl(state, a.start, a.end)
                .map_err(RpcError::Failed)
                .map(|_| Value::Null)
        }
        "clip_bounds" => {
            let a: ClipRefArgs = parse(args)?;
            crate::commands::clip_bounds_impl(state, a.track, a.clip)
                .map_err(RpcError::Failed)
                .and_then(json)
        }
        "ops_list" => json(crate::commands::ops_list_impl(state)),
        "get_track_note_stats" => {
            let a: TrackClipArgs = parse(args)?;
            crate::commands::get_track_note_stats_impl(state, a.track, a.clip)
                .map_err(RpcError::Failed)
                .and_then(json)
        }
        "list_sound_memories" => json(crate::commands::list_sound_memories_impl(state)),
        "list_output_devices" => json(crate::commands::list_output_devices_impl()),
        "list_input_devices" => json(crate::commands::list_input_devices_impl()),
        "list_midi_inputs" => json(crate::commands::list_midi_inputs_impl()),
        "scope_data" => {
            let a: PointsArgs = parse(args)?;
            json(crate::commands::scope_data_impl(state, a.points))
        }
        "retry_audio" => crate::commands::retry_audio_impl(state)
            .map_err(RpcError::Failed)
            .map(|_| Value::Null),

        // ---- 通用兜底：命令名 → `Command` 变体 ----
        //
        // 编辑类命令有 ~70 条，逐条写参数结构体是纯样板。`Command` 已经能 serde 化
        // （字段 camelCase，与 Tauri 的参数命名一致），所以这里只要把名字映射成变体，
        // 再把 JSON 参数原样喂进去即可。
        other => {
            let Some(variant) = variant_of(other) else {
                return Err(RpcError::Unknown(other.to_string()));
            };
            let body = serde_json::json!({ variant: args });
            let command: Command =
                serde_json::from_value(body).map_err(|err| RpcError::BadArgs(err.to_string()))?;
            json(crate::commands::run_one(state, command))
        }
    }
}

/// 已开放的命令名（`/rpc` 的自述，方便调试与前端生成调用代码）。
pub const COMMANDS: &[&str] = &[
    "get_project",
    "project_revision",
    "history_depths",
    "transport_status",
    "mixer_meters",
    "read_import_file",
    "import_project",
    "audio_waveform",
    "export_audio",
    "export_midi",
    "export_score",
    "save_project",
    "import_audio",
    "current_project_path",
    "session_restored",
    "list_plugins",
    "list_presets",
    "add_track",
    "remove_track",
    "rename_track",
    "set_track_instrument",
    "set_tempo",
    "set_time_signature",
    "rename_project",
    "add_clip",
    "remove_clip",
    "move_clip",
    "resize_clip",
    "add_note",
    "set_note",
    "remove_note",
    "quantize_notes",
    "transpose_notes",
    "set_gain",
    "set_pan",
    "set_mute",
    "set_solo",
    "set_master_gain",
    "undo",
    "redo",
    "transport_play",
    "transport_pause",
    "transport_stop",
    "transport_seek",
    "transport_set_loop",
    "transport_set_metronome",
    "transport_set_loop_range",
    "clip_bounds",
    "ops_list",
    "list_sound_memories",
    "list_output_devices",
    "list_input_devices",
    "list_midi_inputs",
    "scope_data",
    "retry_audio",
    // 以下由 `Command` 通用兜底覆盖（名字 ↔ 变体）
    "set_keyframe",
    "remove_keyframe",
    "add_note_track",
    "add_audio_track",
    "add_midi_clip",
    "add_audio_clip",
    "set_clip_audio",
    "set_clip_rate",
    "move_clips",
    "set_notes",
    "remove_notes",
    "remove_clips",
    "set_sends",
    "set_automation",
    "set_param",
    "add_effect",
    "remove_effect",
    "move_effect",
    "set_effect_param",
    "set_effect_enabled",
    "set_effect_sidechain",
    "add_master_effect",
    "remove_master_effect",
    "move_master_effect",
    "set_master_effect_param",
    "set_master_effect_enabled",
    "set_master_effect_sidechain",
    "split_clip",
    "new_project",
    "new_project_with_tracks",
    "add_notes",
    "replace_notes",
    "update_notes",
    "remove_notes_by_id",
    "set_note_velocity",
    "strum_notes",
    "apply_swing",
    "humanize_notes",
    "set_note_tag",
    "set_note_tags",
    "fit_notes_to_range",
    "duplicate_clip",
    "merge_clips",
    "set_project_harmony",
    "set_project_sections",
    "set_track_meta",
    "set_mpe",
    "set_midi_events",
    "set_articulations",
    "apply_cue_sheet",
];

/// 命令名（snake_case，与前端 `invoke` 一致）→ `Command` 变体名。
///
/// 由 `crates/ipc/src/command.rs` 的变体表机械生成；新增变体时同步补一行。
fn variant_of(cmd: &str) -> Option<&'static str> {
    Some(match cmd {
        "set_tempo" => "SetTempo",
        "set_time_signature" => "SetTimeSignature",
        "set_keyframe" => "SetKeyframe",
        "remove_keyframe" => "RemoveKeyframe",
        "add_track" => "AddTrack",
        "add_note_track" => "AddNoteTrack",
        "add_audio_track" => "AddAudioTrack",
        "remove_track" => "RemoveTrack",
        "rename_track" => "RenameTrack",
        "set_track_instrument" => "SetTrackInstrument",
        "add_clip" => "AddClip",
        "add_midi_clip" => "AddMidiClip",
        "add_audio_clip" => "AddAudioClip",
        "set_clip_audio" => "SetClipAudio",
        "set_clip_rate" => "SetClipRate",
        "remove_clip" => "RemoveClip",
        "move_clip" => "MoveClip",
        "move_clips" => "MoveClips",
        "resize_clip" => "ResizeClip",
        "add_note" => "AddNote",
        "set_note" => "SetNote",
        "set_notes" => "SetNotes",
        "remove_note" => "RemoveNote",
        "remove_notes" => "RemoveNotes",
        "remove_clips" => "RemoveClips",
        "set_gain" => "SetGain",
        "set_pan" => "SetPan",
        "set_mute" => "SetMute",
        "set_solo" => "SetSolo",
        "set_master_gain" => "SetMasterGain",
        "set_sends" => "SetSends",
        "set_automation" => "SetAutomation",
        "set_param" => "SetParam",
        "add_effect" => "AddEffect",
        "remove_effect" => "RemoveEffect",
        "move_effect" => "MoveEffect",
        "set_effect_param" => "SetEffectParam",
        "set_effect_enabled" => "SetEffectEnabled",
        "set_effect_sidechain" => "SetEffectSidechain",
        "add_master_effect" => "AddMasterEffect",
        "remove_master_effect" => "RemoveMasterEffect",
        "move_master_effect" => "MoveMasterEffect",
        "set_master_effect_param" => "SetMasterEffectParam",
        "set_master_effect_enabled" => "SetMasterEffectEnabled",
        "set_master_effect_sidechain" => "SetMasterEffectSidechain",
        "split_clip" => "SplitClip",
        "rename_project" => "RenameProject",
        "new_project" => "NewProject",
        "new_project_with_tracks" => "NewProjectWithTracks",
        "add_notes" => "AddNotes",
        "replace_notes" => "ReplaceNotes",
        "update_notes" => "UpdateNotes",
        "remove_notes_by_id" => "RemoveNotesById",
        "transpose_notes" => "TransposeNotes",
        "set_note_velocity" => "SetNoteVelocity",
        "quantize_notes" => "QuantizeNotes",
        "strum_notes" => "StrumNotes",
        "apply_swing" => "ApplySwing",
        "humanize_notes" => "HumanizeNotes",
        "set_note_tag" => "SetNoteTag",
        "set_note_tags" => "SetNoteTags",
        "fit_notes_to_range" => "FitNotesToRange",
        "duplicate_clip" => "DuplicateClip",
        "merge_clips" => "MergeClips",
        "set_project_harmony" => "SetProjectHarmony",
        "set_project_sections" => "SetProjectSections",
        "set_track_meta" => "SetTrackMeta",
        "set_mpe" => "SetMpe",
        "set_midi_events" => "SetMidiEvents",
        "set_articulations" => "SetArticulations",
        "apply_cue_sheet" => "ApplyCueSheet",
        _ => return None,
    })
}
