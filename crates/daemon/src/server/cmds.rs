//! 浏览器端命令桥：直接调用按 Tauri 签名写的实现。
//!
//! 网页端与桌面端必须执行**同一份**实现（`commands::` / `manager::` / `ai::` …），否则
//! "桌面能用、浏览器不能用"会永远修不完。这些函数是按 Tauri 的约定签名的——第一个参数
//! 是 `AppHandle`，状态参数是 `State<'_, T>`——所以在无头进程里怎么拿到它们是个问题：
//!
//! * `AppHandle`：桌面端由外壳经 [`super::ServerState::app`] 传进来；无头进程用空壳
//!   （`tauri_api::rpc_app_handle()`），空壳的 `state()` / `emit()` 接的是全局托管表与
//!   事件总线（见 `tauri_api`）。
//! * `State<'_, T>`：`app.state::<T>()` 在两种构建里同形，桌面端从 `app.manage()` 取，
//!   无头进程从托管表取。
//!
//! 参数名按 Tauri 的约定：前端传 camelCase，Rust 侧是 snake_case，[`take`] 两者都认。
//!
//! **本文件由 `tools/gen_rpc_cmds.py` 生成**，手工改动请先改生成器。

use std::collections::HashMap;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde_json::Value;

use ai_daw_engine::model::HarmonyBar;
use ai_daw_plugin_api::MidiEvent;

use crate::ai::MasterArgs;
use crate::agent::{AgentConfigRuntime, AgentRuntime};
use crate::mcp::McpRuntime;
use crate::manager::{
    BackendPreference, InstallRequest, ModelSourcePreference, ProxyMode, StoreRuntime,
};
use crate::singing::SingArgs;
use crate::svc::SwapArgs;
use crate::state::AppState;
#[allow(unused_imports)] // 桌面端需要 `Manager` 才有 `app.state()`；无头端是固有方法
use crate::tauri_api::{AppHandle, Manager};
use crate::voices::VoiceSaveArgs;
use crate::yue_engine::ComposeArgs;
use crate::yue_lyrics::LyricsArgs;
use crate::yue_transcribe::TranscribeArgs;

use super::rpc::RpcError;

/// 前端按 Tauri 约定传 camelCase，Rust 参数是 snake_case：两个名字都试。
fn camel(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper = false;
    for ch in name.chars() {
        if ch == '_' {
            upper = true;
        } else if upper {
            out.extend(ch.to_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// 取一个参数。缺省按 `null` 处理，于是 `Option<T>` 可以省略、必填项会得到明确的报错。
pub(crate) fn take<T: DeserializeOwned>(args: &Value, name: &str) -> Result<T, RpcError> {
    let value = args
        .get(name)
        .or_else(|| args.get(&camel(name)))
        .cloned()
        .unwrap_or(Value::Null);
    serde_json::from_value(value).map_err(|err| RpcError::BadArgs(format!("{name}: {err}")))
}

/// 成功结果 → JSON。
fn plain<T: serde::Serialize>(value: T) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(|err| RpcError::Failed(err.to_string()))
}

/// `Result<T, String>` → JSON（错误走 500，与桌面端抛出的字符串一致）。
fn value<T: serde::Serialize>(value: Result<T, String>) -> Result<Value, RpcError> {
    plain(value.map_err(RpcError::Failed)?)
}

/// 只有桌面端能做到的事：浏览器里给出明确原因，而不是一句 `unknown command`。
fn unsupported(name: &str, why: &str) -> Result<Value, RpcError> {
    Err(RpcError::Unsupported(format!(
        "{name}：{why}只在桌面端可用；浏览器模式下请用页面内的对应操作"
    )))
}



/// 打开插件原生界面。
///
/// - 桌面构建（有真实窗口系统）：直接开在宿主上——浏览器里点「打开界面」就是桌面弹窗，
///   保真度 100%，零延迟。
/// - 无头构建（容器）：走 `plugin_gui` 的远程界面（虚拟显示 + 帧流 + 输入回传），
///   页面上是一块 canvas，看起来就是"插件窗口在网页里"。
fn window_open(
    app: &AppHandle,
    track: u64,
    plugin_id: Option<String>,
    title: String,
) -> Result<Value, RpcError> {
    #[cfg(feature = "desktop")]
    {
        let state = app.state::<Arc<AppState>>();
        let out = match plugin_id {
            Some(plugin_id) => {
                crate::commands::open_effect_plugin_window(app.clone(), state, track, plugin_id, title)
            }
            None => crate::commands::open_plugin_window(app.clone(), state, track, title),
        };
        return value(out);
    }
    #[cfg(not(feature = "desktop"))]
    {
        let state = app.state::<Arc<AppState>>();
        crate::plugin_gui::open(app.clone(), state, track, plugin_id, title)
    }
}

/// 关闭插件原生界面（桌面关窗口、容器停帧流）。
fn window_close(app: &AppHandle, track: u64, plugin_id: Option<String>) -> Result<Value, RpcError> {
    #[cfg(feature = "desktop")]
    {
        let state = app.state::<Arc<AppState>>();
        let out = match plugin_id {
            Some(plugin_id) => {
                crate::commands::close_effect_plugin_window(app.clone(), state, track, plugin_id)
            }
            None => crate::commands::close_plugin_window(app.clone(), track),
        };
        return value(out);
    }
    #[cfg(not(feature = "desktop"))]
    {
        let state = app.state::<Arc<AppState>>();
        crate::plugin_gui::close(app.clone(), state, track, plugin_id)
    }
}

/// 同步命令表；`None` = 这条命令不在这里。
pub fn call(app: &AppHandle, cmd: &str, args: Value) -> Option<Result<Value, RpcError>> {
    let handler: fn(&AppHandle, Value) -> Result<Value, RpcError> = match cmd {
    "set_params" => |app, args| {
        let track: u64 = take(&args, "track")?;
        let params: HashMap<String, f32> = take(&args, "params")?;        let out = crate::commands::set_params(app.clone(), app.state::<Arc<AppState>>(), track, params);
        plain(out)
    },
    "trim_clip" => |app, args| {
        let track: u64 = take(&args, "track")?;
        let clip: u64 = take(&args, "clip")?;
        let start: f64 = take(&args, "start")?;
        let length: f64 = take(&args, "length")?;        let out = crate::commands::trim_clip(app.clone(), track, clip, start, length);
        value(out)
    },
    "localize_starter_tracks" => |app, args| {
        let names: Vec<String> = take(&args, "names")?;        let out = crate::commands::localize_starter_tracks(app.state::<Arc<AppState>>(), names);
        plain(out)
    },
    "analyze_track_overlap" => |app, args| {
        let tracks: Vec<u64> = take(&args, "tracks")?;
        let clip: Option<u64> = take(&args, "clip")?;        let out = crate::commands::analyze_track_overlap(app.state::<Arc<AppState>>(), tracks, clip);
        value(out)
    },
    "analyze_harmony" => |app, args| {
        let bars: Option<Vec<HarmonyBar>> = take(&args, "bars")?;
        let tracks: Option<Vec<u64>> = take(&args, "tracks")?;
        let allow_extensions: Option<bool> = take(&args, "allow_extensions")?;
        let allow_approach_notes: Option<bool> = take(&args, "allow_approach_notes")?;
        let allow_non_chord_tone_duration: Option<f64> = take(&args, "allow_non_chord_tone_duration")?;
        let time_tolerance: Option<f64> = take(&args, "time_tolerance")?;
        let key: Option<String> = take(&args, "key")?;
        let scale: Option<String> = take(&args, "scale")?;        let out = crate::commands::analyze_harmony(app.state::<Arc<AppState>>(), bars, tracks, allow_extensions, allow_approach_notes, allow_non_chord_tone_duration, time_tolerance, key, scale);
        value(out)
    },
    "list_articulation_maps" => |app, args| {
        let query: Option<String> = take(&args, "query")?;
        let limit: Option<usize> = take(&args, "limit")?;        let out = crate::commands::list_articulation_maps(app.clone(), query, limit);
        plain(out)
    },
    "import_articulation_map" => |app, args| {
        let track: u64 = take(&args, "track")?;
        let id: String = take(&args, "id")?;
        let mode: Option<String> = take(&args, "mode")?;
        let default_articulation: Option<String> = take(&args, "default_articulation")?;        let out = crate::commands::import_articulation_map(app.clone(), app.state::<Arc<AppState>>(), track, id, mode, default_articulation);
        value(out)
    },
    "monitor_track_mix" => |app, args| {
        let track: u64 = take(&args, "track")?;
        let gain_db: f32 = take(&args, "gain_db")?;
        let pan: f32 = take(&args, "pan")?;
        let mute: bool = take(&args, "mute")?;
        let solo: bool = take(&args, "solo")?;        let out = crate::commands::monitor_track_mix(app.state::<Arc<AppState>>(), track, gain_db, pan, mute, solo);
        plain(out)
    },
    "ops_revert_to" => |app, args| {
        let id: u64 = take(&args, "id")?;        let out = crate::commands::ops_revert_to(app.clone(), app.state::<Arc<AppState>>(), id);
        value(out)
    },
    "ops_reapply_to" => |app, args| {
        let id: u64 = take(&args, "id")?;        let out = crate::commands::ops_reapply_to(app.clone(), app.state::<Arc<AppState>>(), id);
        value(out)
    },
    "app_ready" => |_app, _args| {
        // 首帧就绪：浏览器自己控制渲染时机，这里只回一句"收到了"。
        plain(())
    },
    "set_output_device" => |app, args| {
        let device: Option<String> = take(&args, "device")?;        let out = crate::commands::set_output_device(app.state::<Arc<AppState>>(), device);
        value(out)
    },
    "cache_session" => |app, args| {
        let path: Option<String> = take(&args, "path")?;        let out = crate::commands::cache_session(app.clone(), app.state::<Arc<AppState>>(), path);
        value(out)
    },
    "project_history" => |app, _args| {
        let out = crate::commands::project_history(app.clone());
        plain(out)
    },
    "delete_project_history" => |app, args| {
        let id: String = take(&args, "id")?;        let out = crate::commands::delete_project_history(app.clone(), id);
        value(out)
    },
    "restore_project_history" => |app, args| {
        let id: String = take(&args, "id")?;        let out = crate::commands::restore_project_history(app.clone(), id);
        value(out)
    },
    "set_input_device" => |app, args| {
        let device: Option<String> = take(&args, "device")?;        let out = crate::commands::set_input_device(app.state::<Arc<AppState>>(), device);
        value(out)
    },
    "input_level" => |app, args| {
        let points: usize = take(&args, "points")?;        let out = crate::commands::input_level(app.state::<Arc<AppState>>(), points);
        plain(out)
    },
    "set_midi_input" => |app, args| {
        let device: Option<String> = take(&args, "device")?;        let out = crate::commands::set_midi_input(app.clone(), app.state::<Arc<AppState>>(), device);
        value(out)
    },
    "midi_input_device" => |app, _args| {
        let out = crate::commands::midi_input_device(app.state::<Arc<AppState>>());
        plain(out)
    },
    "record_start" => |app, args| {
        let track: u64 = take(&args, "track")?;
        let start: f64 = take(&args, "start")?;
        let device: Option<String> = take(&args, "device")?;        let out = crate::commands::record_start(app.state::<Arc<AppState>>(), track, start, device);
        value(out)
    },
    "plugin_scan_status" => |app, _args| {
        let out = crate::commands::plugin_scan_status(app.state::<Arc<AppState>>());
        plain(out)
    },
    "set_effect_params" => |app, args| {
        let track: u64 = take(&args, "track")?;
        let index: usize = take(&args, "index")?;
        let params: HashMap<String, f32> = take(&args, "params")?;        let out = crate::commands::set_effect_params(app.clone(), app.state::<Arc<AppState>>(), track, index, params);
        plain(out)
    },
    "clear_meter_clip" => |app, _args| {
        let out = crate::commands::clear_meter_clip(app.state::<Arc<AppState>>());
        plain(out)
    },
    "plugin_window_open" => |app, args| {
        let track: u64 = take(&args, "track")?;
        #[cfg(feature = "desktop")]
        {
            let out = crate::commands::plugin_window_open(app.state::<Arc<AppState>>(), track);
            plain(out)
        }
        #[cfg(not(feature = "desktop"))]
        {
            plain(crate::plugin_gui::is_open(track, &None))
        }
    },
    "effect_plugin_window_open" => |app, args| {
        let track: u64 = take(&args, "track")?;
        let plugin_id: String = take(&args, "plugin_id")?;
        #[cfg(feature = "desktop")]
        {
            let out = crate::commands::effect_plugin_window_open(
                app.state::<Arc<AppState>>(),
                track,
                plugin_id,
            );
            plain(out)
        }
        #[cfg(not(feature = "desktop"))]
        {
            plain(crate::plugin_gui::is_open(track, &Some(plugin_id)))
        }
    },
    "audition_note" => |app, args| {
        let track: u64 = take(&args, "track")?;
        let note: u8 = take(&args, "note")?;
        let velocity: u8 = take(&args, "velocity")?;
        let on: bool = take(&args, "on")?;        let out = crate::commands::audition_note(app.state::<Arc<AppState>>(), track, note, velocity, on);
        plain(out)
    },
    "audition_midi_events" => |app, args| {
        let track: u64 = take(&args, "track")?;
        let events: Vec<MidiEvent> = take(&args, "events")?;        let out = crate::commands::audition_midi_events(app.state::<Arc<AppState>>(), track, events);
        plain(out)
    },
    "save_preset" => |app, args| {
        let name: String = take(&args, "name")?;
        let plugin_id: String = take(&args, "plugin_id")?;
        let params: HashMap<String, f32> = take(&args, "params")?;        let out = crate::commands::save_preset(app.state::<Arc<AppState>>(), name, plugin_id, params);
        value(out)
    },
    "update_preset" => |app, args| {
        let id: String = take(&args, "id")?;
        let name: String = take(&args, "name")?;
        let params: HashMap<String, f32> = take(&args, "params")?;        let out = crate::commands::update_preset(app.state::<Arc<AppState>>(), id, name, params);
        value(out)
    },
    "delete_preset" => |app, args| {
        let id: String = take(&args, "id")?;        let out = crate::commands::delete_preset(app.state::<Arc<AppState>>(), id);
        value(out)
    },
    "save_sound_memory" => |app, args| {
        let track: u64 = take(&args, "track")?;
        let name: String = take(&args, "name")?;
        let description: Option<String> = take(&args, "description")?;
        let tags: Option<Vec<String>> = take(&args, "tags")?;
        let id: Option<String> = take(&args, "id")?;        let out = crate::commands::save_sound_memory(app.clone(), app.state::<Arc<AppState>>(), track, name, description, tags, id);
        value(out)
    },
    "update_sound_memory" => |app, args| {
        let id: String = take(&args, "id")?;
        let name: Option<String> = take(&args, "name")?;
        let description: Option<String> = take(&args, "description")?;
        let tags: Option<Vec<String>> = take(&args, "tags")?;
        let role: Option<String> = take(&args, "role")?;        let out = crate::commands::update_sound_memory(app.clone(), app.state::<Arc<AppState>>(), id, name, description, tags, role);
        value(out)
    },
    "delete_sound_memory" => |app, args| {
        let id: String = take(&args, "id")?;        let out = crate::commands::delete_sound_memory(app.clone(), app.state::<Arc<AppState>>(), id);
        value(out)
    },
    "apply_sound_memory" => |app, args| {
        let id: String = take(&args, "id")?;
        let track: u64 = take(&args, "track")?;        let out = crate::commands::apply_sound_memory(app.clone(), app.state::<Arc<AppState>>(), id, track);
        value(out)
    },
    "agent_config" => |app, _args| {
        let out = crate::agent::agent_config(app.state::<AgentConfigRuntime>());
        plain(out)
    },
    "agent_set_config" => |app, args| {
        let base_url: String = take(&args, "base_url")?;
        let api_key: String = take(&args, "api_key")?;
        let model: String = take(&args, "model")?;        let out = crate::agent::agent_set_config(app.state::<AgentConfigRuntime>(), base_url, api_key, model);
        plain(out)
    },
    "agent_sessions" => |app, _args| {
        let out = crate::agent::agent_sessions(app.state::<AgentRuntime>());
        plain(out)
    },
    "agent_session" => |app, args| {
        let id: String = take(&args, "id")?;        let out = crate::agent::agent_session(app.state::<AgentRuntime>(), id);
        value(out)
    },
    "agent_delete_session" => |app, args| {
        let id: String = take(&args, "id")?;        let out = crate::agent::agent_delete_session(app.state::<AgentRuntime>(), id);
        plain(out)
    },
    "agent_set_head" => |app, args| {
        let id: String = take(&args, "id")?;
        let node_id: String = take(&args, "node_id")?;        let out = crate::agent::agent_set_head(app.state::<AgentRuntime>(), id, node_id);
        value(out)
    },
    "agent_cancel" => |app, _args| {
        let out = crate::agent::agent_cancel(app.state::<AgentRuntime>());
        plain(out)
    },
    "agent_running" => |app, _args| {
        let out = crate::agent::agent_running(app.state::<AgentRuntime>());
        plain(out)
    },
    "agent_send" => |app, args| {
        let session_id: Option<String> = take(&args, "session_id")?;
        let parent_id: Option<String> = take(&args, "parent_id")?;
        let text: String = take(&args, "text")?;        let out = crate::agent::agent_send(app.clone(), app.state::<AgentRuntime>(), app.state::<AgentConfigRuntime>(), session_id, parent_id, text);
        value(out)
    },
    "ai_status" => |app, _args| {
        let out = crate::ai::ai_status(app.state::<Arc<AppState>>());
        plain(out)
    },
    "ai_models" => |_app, _args| {
        let out = crate::ai::ai_models();
        plain(out)
    },
    "ai_cancel" => |app, _args| {
        let out = crate::ai::ai_cancel(app.state::<Arc<AppState>>());
        plain(out)
    },
    "ai_start" => |app, args| {
        let kind: String = take(&args, "kind")?;
        let track: u64 = take(&args, "track")?;
        let master: Option<MasterArgs> = take(&args, "master")?;
        let model: Option<String> = take(&args, "model")?;
        let clip: Option<u64> = take(&args, "clip")?;        let out = crate::ai::ai_start(app.clone(), app.state::<Arc<AppState>>(), kind, track, master, model, clip);
        value(out)
    },
    "ai_backend_info" => |app, _args| {
        let out = crate::backends::ai_backend_info(app.clone());
        plain(out)
    },
    "ai_set_backend" => |app, args| {
        let preference: BackendPreference = take(&args, "preference")?;        let out = crate::backends::ai_set_backend(app.clone(), preference);
        value(out)
    },
    "plugin_manager_installed" => |app, _args| {
        let out = crate::manager::plugin_manager_installed(app.clone());
        value(out)
    },
    "plugin_manager_resolve_url" => |app, args| {
        let url: String = take(&args, "url")?;        let out = crate::manager::plugin_manager_resolve_url(app.state::<StoreRuntime>(), url);
        plain(out)
    },
    "plugin_manager_remove" => |app, args| {
        let key: String = take(&args, "key")?;        let out = crate::manager::plugin_manager_remove(app.clone(), key);
        value(out)
    },
    "plugin_manager_foreign" => |app, _args| {
        let out = crate::manager::plugin_manager_foreign(app.clone());
        value(out)
    },
    "plugin_manager_network" => |app, _args| {
        let out = crate::manager::plugin_manager_network(app.state::<StoreRuntime>());
        plain(out)
    },
    "model_hf_token_status" => |app, _args| {
        let out = crate::manager::model_hf_token_status(app.state::<StoreRuntime>());
        plain(out)
    },
    "plugin_manager_set_network" => |app, args| {
        let registry_mirrors: Vec<String> = take(&args, "registry_mirrors")?;
        let download_prefix: String = take(&args, "download_prefix")?;
        let proxy_mode: ProxyMode = take(&args, "proxy_mode")?;
        let proxy_url: String = take(&args, "proxy_url")?;
        let model_mirrors: Option<Vec<String>> = take(&args, "model_mirrors")?;
        let model_source_preference: Option<ModelSourcePreference> = take(&args, "model_source_preference")?;        let out = crate::manager::plugin_manager_set_network(app.state::<StoreRuntime>(), registry_mirrors, download_prefix, proxy_mode, proxy_url, model_mirrors, model_source_preference);
        value(out)
    },
    "model_save_hf_token" => |app, args| {
        let token: String = take(&args, "token")?;        let out = crate::manager::model_save_hf_token(app.state::<StoreRuntime>(), token);
        value(out)
    },
    "plugin_manager_proxy_status" => |app, _args| {
        let out = crate::manager::plugin_manager_proxy_status(app.state::<StoreRuntime>());
        plain(out)
    },
    "mcp_status" => |app, _args| {
        let out = crate::mcp::mcp_status(app.state::<McpRuntime>());
        plain(out)
    },
    "set_mcp_config" => |app, args| {
        let enabled: bool = take(&args, "enabled")?;
        let port: u16 = take(&args, "port")?;        let out = crate::mcp::set_mcp_config(app.state::<McpRuntime>(), enabled, port);
        value(out)
    },
    "model_catalog" => |app, _args| {
        let out = crate::models::model_catalog(app.clone());
        plain(out)
    },
    "model_download" => |app, args| {
        let id: String = take(&args, "id")?;        let out = crate::models::model_download(app.clone(), id);
        value(out)
    },
    "model_cancel" => |_app, _args| {
        let out = crate::models::model_cancel();
        plain(out)
    },
    "model_remove" => |app, args| {
        let id: String = take(&args, "id")?;        let out = crate::models::model_remove(app.clone(), id);
        value(out)
    },
    "model_reveal" => |app, args| {
        let id: String = take(&args, "id")?;        let out = crate::models::model_reveal(app.clone(), id);
        value(out)
    },
    "disk_free_bytes" => |app, _args| {
        let out = crate::paths::disk_free_bytes(app.clone());
        plain(out)
    },
    "plugin_scan_dirs" => |app, _args| {
        let out = crate::plugin_dirs::plugin_scan_dirs(app.clone());
        plain(out)
    },
    "plugin_set_scan_dirs" => |app, args| {
        let dirs: Vec<String> = take(&args, "dirs")?;        let out = crate::plugin_dirs::plugin_set_scan_dirs(app.clone(), dirs);
        value(out)
    },
    "ai_runtime_catalog" => |app, _args| {
        let out = crate::runtimes::ai_runtime_catalog(app.clone());
        plain(out)
    },
    "ai_runtime_install" => |app, args| {
        let id: String = take(&args, "id")?;        let out = crate::runtimes::ai_runtime_install(app.clone(), id);
        value(out)
    },
    "ai_runtime_cancel" => |_app, _args| {
        let out = crate::runtimes::ai_runtime_cancel();
        plain(out)
    },
    "ai_runtime_busy" => |_app, _args| {
        let out = crate::runtimes::ai_runtime_busy();
        plain(out)
    },
    "ai_runtime_remove" => |app, args| {
        let id: String = take(&args, "id")?;        let out = crate::runtimes::ai_runtime_remove(app.clone(), id);
        value(out)
    },
    "ai_runtime_reveal" => |_app, args| {
        let id: String = take(&args, "id")?;        let out = crate::runtimes::ai_runtime_reveal(id);
        value(out)
    },
    "ai_runtime_discover" => |_app, _args| {
        let out = crate::runtimes::ai_runtime_discover();
        plain(out)
    },
    "ai_runtime_import" => |_app, args| {
        let dir: String = take(&args, "dir")?;        let out = crate::runtimes::ai_runtime_import(dir);
        value(out)
    },
    "singing_models" => |_app, _args| {
        let out = crate::singing::singing_models();
        plain(out)
    },
    "singing_status" => |app, _args| {
        let out = crate::singing::singing_status(app.state::<Arc<AppState>>());
        plain(out)
    },
    "singing_cancel" => |app, _args| {
        let out = crate::singing::singing_cancel(app.state::<Arc<AppState>>());
        plain(out)
    },
    "singing_render" => |app, args| {
        let args: SingArgs = take(&args, "args")?;        let out = crate::singing::singing_render(app.clone(), app.state::<Arc<AppState>>(), args);
        value(out)
    },
    "svc_models" => |_app, _args| {
        let out = crate::svc::svc_models();
        plain(out)
    },
    "svc_status" => |app, _args| {
        let out = crate::svc::svc_status(app.state::<Arc<AppState>>());
        plain(out)
    },
    "svc_cancel" => |app, _args| {
        let out = crate::svc::svc_cancel(app.state::<Arc<AppState>>());
        plain(out)
    },
    "svc_render" => |app, args| {
        let args: SwapArgs = take(&args, "args")?;        let out = crate::svc::svc_render(app.clone(), app.state::<Arc<AppState>>(), args);
        value(out)
    },
    "voice_list" => |app, _args| {
        let out = crate::voices::voice_list(app.state::<Arc<AppState>>());
        plain(out)
    },
    "voice_update" => |app, args| {
        let id: String = take(&args, "id")?;
        let name: Option<String> = take(&args, "name")?;
        let description: Option<String> = take(&args, "description")?;
        let tags: Option<Vec<String>> = take(&args, "tags")?;        let out = crate::voices::voice_update(app.state::<Arc<AppState>>(), id, name, description, tags);
        value(out)
    },
    "voice_delete" => |app, args| {
        let id: String = take(&args, "id")?;        let out = crate::voices::voice_delete(app.state::<Arc<AppState>>(), id);
        value(out)
    },
    "ai_yue_import" => |app, args| {
        let id: String = take(&args, "id")?;
        let dir: String = take(&args, "dir")?;        let out = crate::yue::ai_yue_import(app.clone(), id, dir);
        value(out)
    },
    "ai_yue_select" => |_app, args| {
        let id: Option<String> = take(&args, "id")?;        let out = crate::yue::ai_yue_select(id);
        value(out)
    },
    "ai_yue_engine_status" => |_app, _args| {
        let out = crate::yue_engine::ai_yue_engine_status();
        plain(out)
    },
    "ai_yue_engine_log" => |_app, args| {
        let lines: Option<usize> = take(&args, "lines")?;        let out = crate::yue_engine::ai_yue_engine_log(lines);
        plain(out)
    },
    "ai_yue_import_abc" => |app, args| {
        let path: String = take(&args, "path")?;        let out = crate::yue_engine::ai_yue_import_abc(app.clone(), path);
        value(out)
    },
    "ai_yue_cuda_perf" => |app, _args| {
        let out = crate::yue_engine::ai_yue_cuda_perf(app.clone());
        plain(out)
    },
    "ai_yue_set_cuda_perf" => |app, args| {
        let enabled: bool = take(&args, "enabled")?;        let out = crate::yue_engine::ai_yue_set_cuda_perf(app.clone(), enabled);
        value(out)
    },
    "ai_yue_score" => |_app, args| {
        let path: String = take(&args, "path")?;        let out = crate::yue_engine::ai_yue_score(path);
        value(out)
    },
    "ai_yue_record" => |_app, args| {
        let path: String = take(&args, "path")?;        let out = crate::yue_engine::ai_yue_record(path);
        value(out)
    },
    "ai_yue_continuable" => |app, _args| {
        let out = crate::yue_engine::ai_yue_continuable(app.clone());
        plain(out)
    },
    "ai_yue_lora" => |app, _args| {
        let out = crate::yue_engine::ai_yue_lora(app.clone());
        plain(out)
    },
    "ai_yue_set_lora" => |app, args| {
        let lora: crate::manager::YueLora = take(&args, "lora")?;        let out = crate::yue_engine::ai_yue_set_lora(app.clone(), lora);
        value(out)
    },
    "ai_yue_engine_start" => |app, args| {
        let model: Option<String> = take(&args, "model")?;        let out = crate::yue_engine::ai_yue_engine_start(app.clone(), model);
        value(out)
    },
    "ai_yue_engine_stop" => |app, _args| {
        let out = crate::yue_engine::ai_yue_engine_stop(app.clone());
        value(out)
    },
    "ai_yue_compose" => |app, args| {
        let args: ComposeArgs = take(&args, "args")?;        let out = crate::yue_engine::ai_yue_compose(app.clone(), args);
        value(out)
    },
    "ai_yue_compose_cancel" => |app, _args| {
        let out = crate::yue_engine::ai_yue_compose_cancel(app.clone());
        plain(out)
    },
    "ai_yue_lyrics_cancel" => |_app, _args| {
        let out = crate::yue_lyrics::ai_yue_lyrics_cancel();
        plain(out)
    },
    "ai_yue_midi_abc" => |app, args| {
        let track: u64 = take(&args, "track")?;
        let clip: Option<u64> = take(&args, "clip")?;        let out = crate::yue_midi::ai_yue_midi_abc(app.clone(), track, clip);
        value(out)
    },
    "ai_yue_transcribe_cancel" => |_app, _args| {
        let out = crate::yue_transcribe::ai_yue_transcribe_cancel();
        plain(out)
    },
    "cli_status" => |_app, _args| unsupported("cli_status", "命令行安装"),
    // 插件原生窗口：桌面壳有真实窗口，直接开在宿主上（浏览器里点「打开界面」= 桌面弹窗）。
    // 无头（容器）没有窗口系统，走 `plugin_gui` 的远程界面（Xvfb + 帧流），见下。
    "close_effect_plugin_window" => |app, args| {
        let track: u64 = take(&args, "track")?;
        let plugin_id: String = take(&args, "plugin_id")?;
        window_close(app, track, Some(plugin_id))
    },
    "close_plugin_window" => |app, args| {
        let track: u64 = take(&args, "track")?;
        window_close(app, track, None)
    },
    "install_cli" => |_app, _args| unsupported("install_cli", "命令行安装"),
    "open_effect_plugin_window" => |app, args| {
        let track: u64 = take(&args, "track")?;
        let plugin_id: String = take(&args, "plugin_id")?;
        let title: String = take(&args, "title")?;
        window_open(app, track, Some(plugin_id), title)
    },
    "open_effect_window" => |_app, _args| unsupported("open_effect_window", "原生效果器窗口"),
    "open_plugin_window" => |app, args| {
        let track: u64 = take(&args, "track")?;
        let title: String = take(&args, "title")?;
        window_open(app, track, None, title)
    },
    "open_synth_window" => |_app, _args| unsupported("open_synth_window", "原生合成器窗口"),
    // 容器画面链路的自检（需要 AIDAW_GUI_SELFTEST=1）：不碰插件，直接扫虚拟屏。
    "plugin_gui_selftest" => |_app, _args| crate::plugin_gui::selftest(),
    "plugin_manager_open" => |_app, _args| unsupported("plugin_manager_open", "打开外部浏览器"),
    "uninstall_cli" => |_app, _args| unsupported("uninstall_cli", "命令行安装"),
    "web_access_info" => |_app, _args| unsupported("web_access_info", "桌面端网络设置"),
    "web_access_set_lan" => |_app, _args| unsupported("web_access_set_lan", "桌面端网络设置"),
        _ => return None,
    };
    Some(handler(app, args))
}

/// 异步命令名（要 await 的那批）。
const ASYNC_CMDS: &[&str] = &[
    "open_project",
    "analyze_project",
    "record_stop",
    "rescan_plugins",
    "track_midi_capabilities",
    "plugin_params",
    "agent_models",
    "agent_test",
    "plugin_manager_registry",
    "plugin_manager_install",
    "plugin_manager_import",
    "plugin_manager_probe_proxy",
    "model_resolve_url",
    "model_probe",
    "data_dir_info",
    "data_dir_migrate",
    "data_dir_cleanup_previous",
    "voice_save",
    "ai_yue_lyrics",
    "ai_yue_transcribe",
];

/// 异步命令表（打开工程、渲染、重扫插件这类要 await 的）。
async fn run_async(app: &AppHandle, cmd: &str, args: Value) -> Result<Value, RpcError> {
    match cmd {
    "open_project" => {
        let path: String = take(&args, "path")?;        let out = crate::commands::open_project(app.clone(), path).await;
        value(out)
    },
    "analyze_project" => {
        let sample_rate: u32 = take(&args, "sample_rate")?;        let out = crate::commands::analyze_project(app.state::<Arc<AppState>>(), sample_rate).await;
        value(out)
    },
    "record_stop" => {
        let out = crate::commands::record_stop(app.clone()).await;
        value(out)
    },
    "rescan_plugins" => {
        let out = crate::commands::rescan_plugins(app.clone()).await;
        value(out)
    },
    "track_midi_capabilities" => {
        let track: u64 = take(&args, "track")?;        let out = crate::commands::track_midi_capabilities(app.state::<Arc<AppState>>(), track).await;
        value(out)
    },
    "plugin_params" => {
        let plugin_id: String = take(&args, "plugin_id")?;        let out = crate::commands::plugin_params(app.state::<Arc<AppState>>(), plugin_id).await;
        value(out)
    },
    "agent_models" => {
        let base_url: String = take(&args, "base_url")?;
        let api_key: String = take(&args, "api_key")?;
        let model: String = take(&args, "model")?;        let out = crate::agent::agent_models(base_url, api_key, model).await;
        value(out)
    },
    "agent_test" => {
        let base_url: String = take(&args, "base_url")?;
        let api_key: String = take(&args, "api_key")?;
        let model: String = take(&args, "model")?;        let out = crate::agent::agent_test(base_url, api_key, model).await;
        value(out)
    },
    "plugin_manager_registry" => {
        let force: bool = take(&args, "force")?;        let out = crate::manager::plugin_manager_registry(app.clone(), force).await;
        value(out)
    },
    "plugin_manager_install" => {
        let req: InstallRequest = take(&args, "req")?;        let out = crate::manager::plugin_manager_install(app.clone(), req).await;
        value(out)
    },
    "plugin_manager_import" => {
        let path: String = take(&args, "path")?;        let out = crate::manager::plugin_manager_import(app.clone(), path).await;
        value(out)
    },
    "plugin_manager_probe_proxy" => {
        let proxy_mode: ProxyMode = take(&args, "proxy_mode")?;
        let proxy_url: String = take(&args, "proxy_url")?;        let out = crate::manager::plugin_manager_probe_proxy(app.state::<StoreRuntime>(), proxy_mode, proxy_url).await;
        value(out)
    },
    "model_resolve_url" => {
        let id: String = take(&args, "id")?;        let out = crate::models::model_resolve_url(app.clone(), id).await;
        value(out)
    },
    "model_probe" => {
        let id: String = take(&args, "id")?;        let out = crate::models::model_probe(app.clone(), id).await;
        value(out)
    },
    "data_dir_info" => {
        let out = crate::paths::data_dir_info(app.clone()).await;
        value(out)
    },
    "data_dir_migrate" => {
        let target: String = take(&args, "target")?;        let out = crate::paths::data_dir_migrate(app.clone(), target).await;
        value(out)
    },
    "data_dir_cleanup_previous" => {
        let out = crate::paths::data_dir_cleanup_previous(app.clone()).await;
        value(out)
    },
    "voice_save" => {
        let args: VoiceSaveArgs = take(&args, "args")?;        let out = crate::voices::voice_save(app.clone(), args).await;
        value(out)
    },
    "ai_yue_lyrics" => {
        let args: LyricsArgs = take(&args, "args")?;        let out = crate::yue_lyrics::ai_yue_lyrics(app.clone(), args).await;
        value(out)
    },
    "ai_yue_transcribe" => {
        let args: TranscribeArgs = take(&args, "args")?;        let out = crate::yue_transcribe::ai_yue_transcribe(app.clone(), args).await;
        value(out)
    },
        _ => Err(RpcError::Unknown(cmd.to_string())),
    }
}

/// 全部命令名（`/commands` 自述用）。
pub const NAMES: &[&str] = &[
    "agent_cancel",
    "agent_config",
    "agent_delete_session",
    "agent_models",
    "agent_running",
    "agent_send",
    "agent_session",
    "agent_sessions",
    "agent_set_config",
    "agent_set_head",
    "agent_test",
    "ai_backend_info",
    "ai_cancel",
    "ai_models",
    "ai_runtime_busy",
    "ai_runtime_cancel",
    "ai_runtime_catalog",
    "ai_runtime_discover",
    "ai_runtime_import",
    "ai_runtime_install",
    "ai_runtime_remove",
    "ai_runtime_reveal",
    "ai_set_backend",
    "ai_start",
    "ai_status",
    "ai_yue_compose",
    "ai_yue_compose_cancel",
    "ai_yue_continuable",
    "ai_yue_cuda_perf",
    "ai_yue_engine_log",
    "ai_yue_engine_start",
    "ai_yue_engine_status",
    "ai_yue_engine_stop",
    "ai_yue_import",
    "ai_yue_import_abc",
    "ai_yue_lora",
    "ai_yue_lyrics",
    "ai_yue_lyrics_cancel",
    "ai_yue_midi_abc",
    "ai_yue_record",
    "ai_yue_score",
    "ai_yue_select",
    "ai_yue_set_cuda_perf",
    "ai_yue_set_lora",
    "ai_yue_transcribe",
    "ai_yue_transcribe_cancel",
    "analyze_harmony",
    "analyze_project",
    "analyze_track_overlap",
    "app_ready",
    "apply_sound_memory",
    "audition_midi_events",
    "audition_note",
    "cache_session",
    "clear_meter_clip",
    "cli_status",
    "close_effect_plugin_window",
    "close_plugin_window",
    "data_dir_cleanup_previous",
    "data_dir_info",
    "data_dir_migrate",
    "delete_preset",
    "delete_project_history",
    "delete_sound_memory",
    "disk_free_bytes",
    "effect_plugin_window_open",
    "import_articulation_map",
    "input_level",
    "install_cli",
    "list_articulation_maps",
    "localize_starter_tracks",
    "mcp_status",
    "midi_input_device",
    "model_cancel",
    "model_catalog",
    "model_download",
    "model_hf_token_status",
    "model_probe",
    "model_remove",
    "model_resolve_url",
    "model_reveal",
    "model_save_hf_token",
    "monitor_track_mix",
    "open_effect_plugin_window",
    "open_effect_window",
    "open_plugin_window",
    "open_project",
    "open_synth_window",
    "ops_reapply_to",
    "ops_revert_to",
    "plugin_manager_foreign",
    "plugin_manager_import",
    "plugin_manager_install",
    "plugin_manager_installed",
    "plugin_manager_network",
    "plugin_manager_open",
    "plugin_manager_probe_proxy",
    "plugin_manager_proxy_status",
    "plugin_manager_registry",
    "plugin_manager_remove",
    "plugin_manager_resolve_url",
    "plugin_manager_set_network",
    "plugin_params",
    "plugin_scan_dirs",
    "plugin_scan_status",
    "plugin_set_scan_dirs",
    "plugin_window_open",
    "project_history",
    "record_start",
    "record_stop",
    "render_preview",
    "rescan_plugins",
    "restore_project_history",
    "save_preset",
    "save_sound_memory",
    "set_effect_params",
    "set_input_device",
    "set_mcp_config",
    "set_midi_input",
    "set_output_device",
    "set_params",
    "singing_cancel",
    "singing_models",
    "singing_render",
    "singing_status",
    "svc_cancel",
    "svc_models",
    "svc_render",
    "svc_status",
    "track_midi_capabilities",
    "trim_clip",
    "uninstall_cli",
    "update_preset",
    "update_sound_memory",
    "voice_delete",
    "voice_list",
    "voice_save",
    "voice_update",
    "web_access_info",
    "web_access_set_lan",
];

/// 异步命令入口；`None` = 这条命令不在这里。
pub async fn call_async(
    app: &AppHandle,
    cmd: &str,
    args: Value,
) -> Option<Result<Value, RpcError>> {
    if !ASYNC_CMDS.contains(&cmd) {
        return None;
    }
    Some(run_async(app, cmd, args).await)
}
