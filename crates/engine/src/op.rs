//! 操作记录：把一批命令描述成一条「人 / AI 可读」的时间线条目。
//!
//! 文案不在这里拼：引擎只给出 `kind` + `target` + `args`，界面按当前语言本地化。
//! 这样同一个操作在中文 / 英文界面里都能说清楚，也不必让引擎依赖任何界面文案。

pub use ai_daw_ipc::op::{OpArg, OpRecord, OpValue};

use crate::command::Command;
use crate::model::{Project, TrackId};

fn num(key: &str, value: f64) -> OpArg {
    OpArg {
        key: key.into(),
        value: OpValue::Num(value),
    }
}

fn text(key: &str, value: impl Into<String>) -> OpArg {
    OpArg {
        key: key.into(),
        value: OpValue::Text(value.into()),
    }
}

fn flag(key: &str, value: bool) -> OpArg {
    OpArg {
        key: key.into(),
        value: OpValue::Bool(value),
    }
}

/// 轨道显示名（操作发生时的那一版）。
fn track_name(project: &Project, track: TrackId) -> Option<String> {
    project
        .tracks
        .iter()
        .find(|t| t.id == track)
        .map(|t| t.name.clone())
}

/// 描述一批命令：`(kind, target, args)`。
///
/// 单条命令给出具体类型；多条命令合成一个撤销步骤时统一记为 `batch`（带步数），
/// 避免「切换预设写了 80 个参数」在时间线上刷出 80 行。
pub fn describe(project: &Project, commands: &[Command]) -> (String, Option<String>, Vec<OpArg>) {
    let [command] = commands else {
        let count = commands.len();
        return ("batch".into(), None, vec![num("count", count as f64)]);
    };
    describe_one(project, command)
}

fn describe_one(project: &Project, command: &Command) -> (String, Option<String>, Vec<OpArg>) {
    let (kind, target, args): (&str, Option<String>, Vec<OpArg>) = match command {
        Command::SetTempo(tempo) => ("set_tempo", None, vec![num("tempo", *tempo)]),
        Command::SetTimeSignature { num: n, den } => (
            "set_time_signature",
            None,
            vec![num("num", *n as f64), num("den", *den as f64)],
        ),
        Command::SetKeyframe { beat, tempo, .. } => (
            "set_keyframe",
            None,
            vec![num("beat", *beat), num("tempo", *tempo)],
        ),
        Command::RemoveKeyframe { beat } => ("remove_keyframe", None, vec![num("beat", *beat)]),

        Command::AddTrack { name } => ("add_track", Some(name.clone()), vec![]),
        Command::AddNoteTrack { name, notes, .. } => (
            "add_note_track",
            Some(name.clone()),
            vec![num("count", notes.len() as f64)],
        ),
        Command::AddAudioTrack { name, .. } => ("add_audio_track", Some(name.clone()), vec![]),
        Command::RemoveTrack(track) => ("remove_track", track_name(project, *track), vec![]),
        Command::RenameTrack { track, name } => (
            "rename_track",
            track_name(project, *track),
            vec![text("name", name.clone())],
        ),
        Command::SetTrackInstrument { track, plugin_id } => (
            "set_track_instrument",
            track_name(project, *track),
            vec![text("plugin", plugin_id.clone())],
        ),

        Command::AddClip {
            track,
            start,
            length,
        } => (
            "add_clip",
            track_name(project, *track),
            vec![num("start", *start), num("length", *length)],
        ),
        Command::AddAudioClip {
            track,
            start,
            length,
            ..
        } => (
            "add_audio_clip",
            track_name(project, *track),
            vec![num("start", *start), num("length", *length)],
        ),
        Command::SetClipAudio { track, .. } => {
            ("set_clip_audio", track_name(project, *track), vec![])
        }
        Command::SetClipRate {
            track,
            rate,
            pitch_semitones,
            ..
        } => (
            "set_clip_rate",
            track_name(project, *track),
            vec![num("rate", *rate), num("pitch", *pitch_semitones)],
        ),
        Command::RemoveClip { track, .. } => ("remove_clip", track_name(project, *track), vec![]),
        Command::MoveClip { track, start, .. } => (
            "move_clip",
            track_name(project, *track),
            vec![num("start", *start)],
        ),
        Command::MoveClips { items } => {
            ("move_clips", None, vec![num("count", items.len() as f64)])
        }
        Command::ResizeClip { track, length, .. } => (
            "resize_clip",
            track_name(project, *track),
            vec![num("length", *length)],
        ),

        Command::AddNote { track, .. } => ("add_note", track_name(project, *track), vec![]),
        Command::SetNote { track, .. } => ("set_note", track_name(project, *track), vec![]),
        Command::SetNotes { track, entries, .. } => (
            "set_notes",
            track_name(project, *track),
            vec![num("count", entries.len() as f64)],
        ),
        Command::RemoveNote { track, .. } => ("remove_note", track_name(project, *track), vec![]),
        Command::RemoveNotes { track, indices, .. } => (
            "remove_notes",
            track_name(project, *track),
            vec![num("count", indices.len() as f64)],
        ),
        Command::RemoveClips { items } => {
            ("remove_clips", None, vec![num("count", items.len() as f64)])
        }

        Command::SetGain { track, gain_db } => (
            "set_gain",
            track_name(project, *track),
            vec![num("db", *gain_db as f64)],
        ),
        Command::SetPan { track, pan } => (
            "set_pan",
            track_name(project, *track),
            vec![num("pan", *pan as f64)],
        ),
        Command::SetMute { track, mute } => (
            "set_mute",
            track_name(project, *track),
            vec![flag("on", *mute)],
        ),
        Command::SetSolo { track, solo } => (
            "set_solo",
            track_name(project, *track),
            vec![flag("on", *solo)],
        ),
        Command::SetMasterGain(gain_db) => {
            ("set_master_gain", None, vec![num("db", *gain_db as f64)])
        }
        Command::SetSends { track, sends } => (
            "set_sends",
            track_name(project, *track),
            vec![num("count", sends.len() as f64)],
        ),
        Command::SetAutomation {
            track,
            param,
            points,
        } => (
            "set_automation",
            track_name(project, *track),
            vec![
                text("param", param.clone()),
                num("count", points.len() as f64),
            ],
        ),
        Command::SetParam {
            track,
            param,
            value,
        } => (
            "set_param",
            track_name(project, *track),
            vec![text("param", param.clone()), num("value", *value as f64)],
        ),

        Command::AddEffect { track, plugin_id } => (
            "add_effect",
            track_name(project, *track),
            vec![text("plugin", plugin_id.clone())],
        ),
        Command::RemoveEffect { track, index } => (
            "remove_effect",
            track_name(project, *track),
            vec![num("index", *index as f64)],
        ),
        Command::MoveEffect { track, from, to } => (
            "move_effect",
            track_name(project, *track),
            vec![num("from", *from as f64), num("to", *to as f64)],
        ),
        Command::SetEffectParam {
            track,
            index,
            param,
            value,
        } => (
            "set_effect_param",
            track_name(project, *track),
            vec![
                num("index", *index as f64),
                text("param", param.clone()),
                num("value", *value as f64),
            ],
        ),

        Command::SplitClip { track, beat, .. } => (
            "split_clip",
            track_name(project, *track),
            vec![num("beat", *beat)],
        ),
        Command::RenameProject { name } => ("rename_project", Some(name.clone()), vec![]),
        Command::NewProject { name, .. } => ("new_project", Some(name.clone()), vec![]),
        Command::NewProjectWithTracks { name, track_names } => (
            "new_project",
            Some(name.clone()),
            vec![num("tracks", track_names.len() as f64)],
        ),

        Command::AddNotes { track, notes, .. } => (
            "add_notes",
            track_name(project, *track),
            vec![num("count", notes.len() as f64)],
        ),
        Command::ReplaceNotes { track, notes, .. } => (
            "replace_notes",
            track_name(project, *track),
            vec![num("count", notes.len() as f64)],
        ),
        Command::UpdateNotes { track, updates, .. } => (
            "update_notes",
            track_name(project, *track),
            vec![num("count", updates.len() as f64)],
        ),
        Command::RemoveNotesById { track, ids, .. } => (
            "remove_notes_by_id",
            track_name(project, *track),
            vec![num("count", ids.len() as f64)],
        ),
        Command::TransposeNotes {
            track, semitones, ..
        } => (
            "transpose_notes",
            track_name(project, *track),
            vec![num("semitones", *semitones as f64)],
        ),
        Command::SetNoteVelocity { track, .. } => {
            ("set_note_velocity", track_name(project, *track), vec![])
        }
        Command::QuantizeNotes {
            track,
            grid,
            strength,
            ..
        } => (
            "quantize_notes",
            track_name(project, *track),
            vec![num("grid", *grid), num("strength", *strength)],
        ),
        Command::StrumNotes { track, step_ms, .. } => (
            "strum_notes",
            track_name(project, *track),
            vec![num("step_ms", *step_ms)],
        ),
        Command::ApplySwing {
            track,
            subdivision,
            ratio,
            strength,
            ..
        } => (
            "apply_swing",
            track_name(project, *track),
            vec![
                num("subdivision", *subdivision),
                num("ratio", *ratio),
                num("strength", *strength),
            ],
        ),
        Command::HumanizeNotes { track, .. } => {
            ("humanize_notes", track_name(project, *track), vec![])
        }
        Command::SetNoteTags { track, ids, .. } => (
            "set_note_tags",
            track_name(project, *track),
            vec![num("count", ids.len() as f64)],
        ),
        Command::SetNoteTag { track, .. } => ("set_note_tag", track_name(project, *track), vec![]),
        Command::FitNotesToRange {
            track,
            min_pitch,
            max_pitch,
            ..
        } => (
            "fit_notes_to_range",
            track_name(project, *track),
            vec![num("min", *min_pitch as f64), num("max", *max_pitch as f64)],
        ),
        Command::DuplicateClip { track, .. } => {
            ("duplicate_clip", track_name(project, *track), vec![])
        }
        Command::MergeClips { track, clips } => (
            "merge_clips",
            track_name(project, *track),
            vec![num("count", clips.len() as f64)],
        ),

        Command::SetProjectHarmony { bars } => (
            "set_project_harmony",
            None,
            vec![num("count", bars.len() as f64)],
        ),
        Command::SetProjectSections { sections } => (
            "set_project_sections",
            None,
            vec![num("count", sections.len() as f64)],
        ),
        Command::ApplyCueSheet { plan } => {
            let clips: usize = plan.tracks.iter().map(|track| track.clips.len()).sum();
            let notes: usize = plan
                .tracks
                .iter()
                .flat_map(|track| track.clips.iter())
                .map(|clip| clip.notes.len())
                .sum();
            (
                "import_cue_sheet",
                plan.name.clone(),
                vec![
                    num("tracks", plan.tracks.len() as f64),
                    num("sections", plan.sections.len() as f64),
                    num("clips", clips as f64),
                    num("notes", notes as f64),
                ],
            )
        }
        Command::SetMpe { track, config } => {
            let args = match config {
                Some(config) => vec![
                    text("zone", format!("{:?}", config.zone).to_lowercase()),
                    num("channels", f64::from(config.channels)),
                ],
                None => vec![text("zone", "off")],
            };
            ("set_mpe", track_name(project, *track), args)
        }
        Command::SetMidiEvents { track, events } => (
            "set_midi_events",
            track_name(project, *track),
            vec![num("count", events.len() as f64)],
        ),
        Command::SetArticulations {
            track,
            articulations,
            default,
            ..
        } => {
            let mut args = vec![num("count", articulations.len() as f64)];
            if let Some(default) = default {
                args.push(text("default", default.clone()));
            }
            ("set_articulations", track_name(project, *track), args)
        }
        Command::SetTrackMeta { track, role, .. } => {
            let mut args = vec![];
            if let Some(role) = role {
                args.push(text("role", role.clone()));
            }
            ("set_track_meta", track_name(project, *track), args)
        }
    };
    (kind.into(), target, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Engine;
    use crate::event::Actor;

    #[test]
    fn single_command_resolves_target_at_describe_time() {
        let mut engine = Engine::new("demo");
        engine.execute(
            Command::AddTrack {
                name: "Bass".into(),
            },
            Actor::Human,
        );
        let track = engine.project().tracks[0].id;

        // 删除之后轨道名就查不到了——描述必须在应用命令之前生成。
        let (kind, target, _) = describe(engine.project(), &[Command::RemoveTrack(track)]);
        assert_eq!(kind, "remove_track");
        assert_eq!(target.as_deref(), Some("Bass"));
    }

    #[test]
    fn multiple_commands_collapse_into_batch() {
        let engine = Engine::new("demo");
        let (kind, _, args) = describe(
            engine.project(),
            &[Command::SetTempo(90.0), Command::SetTempo(100.0)],
        );
        assert_eq!(kind, "batch");
        assert_eq!(args.len(), 1);
        assert_eq!(args[0].key, "count");
        assert!(matches!(args[0].value, OpValue::Num(n) if n == 2.0));
    }
}
