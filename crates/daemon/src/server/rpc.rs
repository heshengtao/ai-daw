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
    /// 令牌缺失或不匹配。
    Unauthorized,
}

impl IntoResponse for RpcError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            RpcError::Unknown(cmd) => (StatusCode::BAD_REQUEST, format!("unknown command: {cmd}")),
            RpcError::BadArgs(err) => (StatusCode::BAD_REQUEST, format!("bad args: {err}")),
            RpcError::Failed(err) => (StatusCode::INTERNAL_SERVER_ERROR, err),
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
struct SeekArgs {
    frame: u64,
}

/// 分发一条命令。**与 Tauri 命令同名同参**，前端的两套传输层可以共用一份调用代码。
pub fn dispatch(state: &AppState, cmd: &str, args: Value) -> Result<Value, RpcError> {
    match cmd {
        // ---- 只读 ----
        "get_project" => json(crate::commands::get_project_impl(state)),
        "history_depths" => json(crate::commands::history_depths_impl(state)),
        "transport_status" => json(crate::commands::transport_status_impl(state)),
        "mixer_meters" => json(crate::commands::mixer_meters_impl(state)),

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

        other => Err(RpcError::Unknown(other.to_string())),
    }
}

/// 已开放的命令名（`/rpc` 的自述，方便调试与前端生成调用代码）。
pub const COMMANDS: &[&str] = &[
    "get_project",
    "history_depths",
    "transport_status",
    "mixer_meters",
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
];
