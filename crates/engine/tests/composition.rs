//! 编曲能力的端到端断言：批量写入、变换、片段复制/合并、元数据。
//!
//! 这些测试只经过 `Engine::execute`（命令 → 事件 → 应用），与 MCP / 界面走的是**同一条**
//! 管线，因此「一次事务 = 一条撤销记录」在这里被真正验证，而不是在工具层伪造。

use ai_daw_engine::command::Command;
use ai_daw_engine::event::{Actor, Event};
use ai_daw_engine::model::{
    ClipId, FitMode, HarmonyBar, Note, NotePatch, NoteRange, NoteTag, Section, TrackId,
    VelocityShape,
};
use ai_daw_engine::Engine;

fn agent() -> Actor {
    Actor::Agent("mcp".into())
}

fn note(pitch: u8, start: f64, duration: f64, velocity: u8) -> Note {
    Note {
        id: 0,
        channel: None,
        pressure: None,
        pitch,
        start,
        duration,
        velocity,
        volume: 127,
        offset: 0.0,
        legato: false,
        expr: ai_daw_engine::model::PitchExpression::NONE,
    }
}

/// 一条轨道 + 一个 16 拍的 MIDI 片段。
fn setup() -> (Engine, TrackId, ClipId) {
    let mut engine = Engine::new("test");
    engine.execute(
        Command::AddTrack {
            name: "Lead".into(),
        },
        agent(),
    );
    let track = engine.project().tracks[0].id;
    engine.execute(
        Command::AddClip {
            track,
            start: 0.0,
            length: 16.0,
        },
        agent(),
    );
    let clip = engine.project().tracks[0].clips[0].id;
    (engine, track, clip)
}

fn notes_of(engine: &Engine, track: TrackId, clip: ClipId) -> Vec<Note> {
    engine
        .project()
        .track(track)
        .unwrap()
        .clips
        .iter()
        .find(|c| c.id == clip)
        .unwrap()
        .notes
        .clone()
}

#[test]
fn a_thousand_notes_is_one_undo_step() {
    let (mut e, track, clip) = setup();
    let base = e.undo_depth();
    let notes: Vec<Note> = (0..1000)
        .map(|i| note(48 + (i % 24) as u8, i as f64 * 0.25, 0.25, 100))
        .collect();

    e.execute(Command::AddNotes { track, clip, notes }, agent());

    assert_eq!(e.undo_depth(), base + 1, "整批只占一条撤销记录");
    let added = notes_of(&e, track, clip);
    assert_eq!(added.len(), 1000);
    // id 唯一且都非零。
    let mut ids: Vec<u64> = added.iter().map(|n| n.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 1000);
    assert!(ids.iter().all(|id| *id > 0));

    e.undo();
    assert!(notes_of(&e, track, clip).is_empty(), "一次撤销回到空片段");
    e.redo();
    let again: Vec<u64> = notes_of(&e, track, clip).iter().map(|n| n.id).collect();
    assert_eq!(
        again,
        added.iter().map(|n| n.id).collect::<Vec<_>>(),
        "重做后 id 完全一致"
    );
}

#[test]
fn replace_notes_is_atomic_and_assigns_fresh_ids() {
    let (mut e, track, clip) = setup();
    e.execute(
        Command::AddNotes {
            track,
            clip,
            notes: vec![note(60, 0.0, 1.0, 100), note(62, 1.0, 1.0, 100)],
        },
        agent(),
    );
    let before = notes_of(&e, track, clip);

    e.execute(
        Command::ReplaceNotes {
            track,
            clip,
            notes: vec![
                note(64, 0.5, 2.0, 90),
                note(67, 2.5, 2.0, 90),
                note(72, 4.5, 1.0, 90),
            ],
        },
        agent(),
    );
    let after = notes_of(&e, track, clip);
    assert_eq!(after.len(), 3);
    assert_eq!(after[0].pitch, 64);
    assert_eq!(after[0].start, 0.5, "相对拍坐标原样保留");
    assert!(after.iter().all(|n| n.id > 0));

    // 一次撤销回到替换前的那两个音（id 也回到原来的）。
    e.undo();
    assert_eq!(notes_of(&e, track, clip), before);
}

#[test]
fn update_and_remove_by_stable_id() {
    let (mut e, track, clip) = setup();
    e.execute(
        Command::AddNotes {
            track,
            clip,
            notes: vec![
                note(60, 0.0, 1.0, 100),
                note(62, 1.0, 1.0, 100),
                note(64, 2.0, 1.0, 100),
            ],
        },
        agent(),
    );
    let ids: Vec<u64> = notes_of(&e, track, clip).iter().map(|n| n.id).collect();

    // 删掉中间那个，剩下的 id 不漂移。
    e.execute(
        Command::RemoveNotesById {
            track,
            clip,
            ids: vec![ids[1]],
        },
        agent(),
    );
    let left = notes_of(&e, track, clip);
    assert_eq!(
        left.iter().map(|n| n.id).collect::<Vec<_>>(),
        vec![ids[0], ids[2]]
    );

    // 用 id 改第三个音——下标已经变了，但 id 仍然指得准。
    e.execute(
        Command::UpdateNotes {
            track,
            clip,
            updates: vec![NotePatch {
                id: ids[2],
                pitch: Some(67),
                start: Some(3.5),
                duration: None,
                velocity: Some(40),
                volume: None,
                offset: None,
                legato: None,
                channel: None,
                pressure: None,
                expr: None,
            }],
        },
        agent(),
    );
    let updated = notes_of(&e, track, clip);
    assert_eq!(updated[1].id, ids[2]);
    assert_eq!(updated[1].pitch, 67);
    assert_eq!(updated[1].start, 3.5);
    assert_eq!(updated[1].duration, 1.0, "没给的字段保持原值");
    assert_eq!(updated[1].velocity, 40);

    // 两次编辑各自可撤销。
    e.undo();
    assert_eq!(notes_of(&e, track, clip)[1].pitch, 64);
    e.undo();
    assert_eq!(notes_of(&e, track, clip).len(), 3);
    e.redo();
    e.redo();
    assert_eq!(notes_of(&e, track, clip)[1].pitch, 67);
}

#[test]
fn transpose_velocity_quantize_swing_are_all_undoable() {
    let (mut e, track, clip) = setup();
    e.execute(
        Command::AddNotes {
            track,
            clip,
            notes: vec![
                note(60, 0.0, 0.5, 100),
                note(62, 0.5, 0.5, 100),
                note(64, 1.0, 0.5, 100),
            ],
        },
        agent(),
    );
    let original = notes_of(&e, track, clip);

    e.execute(
        Command::TransposeNotes {
            track,
            clip,
            semitones: 3,
            range: None,
        },
        agent(),
    );
    assert_eq!(
        notes_of(&e, track, clip)
            .iter()
            .map(|n| n.pitch)
            .collect::<Vec<_>>(),
        vec![63, 65, 67]
    );

    e.execute(
        Command::SetNoteVelocity {
            track,
            clip,
            shape: VelocityShape::Fixed(40),
            range: Some(NoteRange {
                start: 0.0,
                end: 0.5,
            }),
            beat_filter: None,
        },
        agent(),
    );
    let vel: Vec<u8> = notes_of(&e, track, clip)
        .iter()
        .map(|n| n.velocity)
        .collect();
    assert_eq!(vel, vec![40, 100, 100], "范围只覆盖第一个音");

    e.execute(
        Command::QuantizeNotes {
            track,
            clip,
            grid: 1.0,
            strength: 1.0,
            range: None,
            quantize_duration: false,
        },
        agent(),
    );
    let starts: Vec<f64> = notes_of(&e, track, clip).iter().map(|n| n.start).collect();
    assert_eq!(starts, vec![0.0, 1.0, 1.0], "0.5 吸到 1.0（四舍五入）");

    // 全部撤销 → 回到最初那一组。
    for _ in 0..3 {
        e.undo();
    }
    assert_eq!(notes_of(&e, track, clip), original);
}

#[test]
fn swing_at_two_thirds_lands_on_the_third_beat_fraction() {
    let (mut e, track, clip) = setup();
    e.execute(
        Command::AddNotes {
            track,
            clip,
            notes: vec![note(60, 0.0, 0.25, 100), note(60, 0.5, 0.25, 100)],
        },
        agent(),
    );
    e.execute(
        Command::ApplySwing {
            track,
            clip,
            subdivision: 0.5,
            ratio: 0.66,
            strength: 1.0,
            range: None,
        },
        agent(),
    );
    let starts: Vec<f64> = notes_of(&e, track, clip).iter().map(|n| n.start).collect();
    assert_eq!(starts[0], 0.0, "第一个子拍不动");
    assert!(
        (starts[1] - 0.66).abs() < 1e-9,
        "8 分 swing 的第二个子拍落在约 2/3 拍：{starts:?}"
    );
}

#[test]
fn humanize_with_the_same_seed_is_reproducible_across_undo() {
    let (mut e, track, clip) = setup();
    e.execute(
        Command::AddNotes {
            track,
            clip,
            notes: vec![note(60, 0.0, 0.25, 100), note(62, 1.0, 0.25, 100)],
        },
        agent(),
    );
    e.execute(
        Command::HumanizeNotes {
            track,
            clip,
            timing: 0.02,
            velocity: 8.0,
            range: None,
            seed: Some(42),
            min_duration: 0.0,
            group_by_start: false,
        },
        agent(),
    );
    let once = notes_of(&e, track, clip);
    e.undo();
    e.redo();
    assert_eq!(notes_of(&e, track, clip), once, "重做不能重新掷骰子");
}

#[test]
fn fit_to_range_folds_octaves_and_report_mode_changes_nothing() {
    let (mut e, track, clip) = setup();
    e.execute(
        Command::AddNotes {
            track,
            clip,
            notes: vec![note(84, 0.0, 1.0, 100), note(40, 1.0, 1.0, 100)],
        },
        agent(),
    );
    let before = notes_of(&e, track, clip);

    // report 模式：一条事件都不产生。
    let events = e.execute(
        Command::FitNotesToRange {
            track,
            clip,
            min_pitch: 48,
            max_pitch: 72,
            mode: FitMode::Report,
            range: None,
        },
        agent(),
    );
    assert!(events.is_empty(), "report 模式不得改动工程");
    assert_eq!(notes_of(&e, track, clip), before);

    e.execute(
        Command::FitNotesToRange {
            track,
            clip,
            min_pitch: 48,
            max_pitch: 72,
            mode: FitMode::Fold,
            range: None,
        },
        agent(),
    );
    let pitches: Vec<u8> = notes_of(&e, track, clip).iter().map(|n| n.pitch).collect();
    assert_eq!(pitches, vec![72, 52]);
    assert_eq!(pitches[0] % 12, 84 % 12, "八度折返不改变音级");
    assert_eq!(pitches[1] % 12, 40 % 12);
}

#[test]
fn duplicate_clip_copies_notes_with_fresh_ids_and_keeps_offsets() {
    let (mut e, track, clip) = setup();
    e.execute(
        Command::AddNotes {
            track,
            clip,
            notes: vec![note(60, 0.5, 1.0, 100), note(64, 1.5, 1.0, 100)],
        },
        agent(),
    );
    let source_ids: Vec<u64> = notes_of(&e, track, clip).iter().map(|n| n.id).collect();

    let events = e.execute(
        Command::DuplicateClip {
            track,
            clip,
            to_track: None,
            start: None,
        },
        agent(),
    );
    let new_clip = events
        .iter()
        .find_map(|ev| match ev {
            Event::ClipAdded { clip, .. } => Some(*clip),
            _ => None,
        })
        .expect("复制应产生一个新片段");

    let clips = &e.project().tracks[0].clips;
    assert_eq!(clips.len(), 2);
    let copy = clips.iter().find(|c| c.id == new_clip).unwrap();
    assert_eq!(copy.start, 16.0, "缺省贴在原片段之后");
    assert_eq!(copy.length, 16.0);
    assert_eq!(
        copy.notes.iter().map(|n| n.start).collect::<Vec<_>>(),
        vec![0.5, 1.5]
    );
    // id 是**片段内**唯一的：副本自己一套号，不重复即可。
    let mut copy_ids: Vec<u64> = copy.notes.iter().map(|n| n.id).collect();
    copy_ids.sort_unstable();
    copy_ids.dedup();
    assert_eq!(copy_ids.len(), 2);
    let _ = source_ids;

    e.undo();
    assert_eq!(e.project().tracks[0].clips.len(), 1);
}

#[test]
fn merge_clips_offsets_notes_and_drops_the_extras() {
    let (mut e, track, clip) = setup();
    e.execute(
        Command::AddNotes {
            track,
            clip,
            notes: vec![note(60, 1.0, 1.0, 100)],
        },
        agent(),
    );
    let events = e.execute(
        Command::AddClip {
            track,
            start: 4.0,
            length: 4.0,
        },
        agent(),
    );
    let second = events
        .iter()
        .find_map(|ev| match ev {
            Event::ClipAdded { clip, .. } => Some(*clip),
            _ => None,
        })
        .unwrap();
    e.execute(
        Command::AddNotes {
            track,
            clip: second,
            notes: vec![note(67, 0.5, 1.0, 100)],
        },
        agent(),
    );

    e.execute(
        Command::MergeClips {
            track,
            clips: vec![clip, second],
        },
        agent(),
    );

    let clips = &e.project().tracks[0].clips;
    assert_eq!(clips.len(), 1, "只留合并后的那一个片段");
    let merged = &clips[0];
    assert_eq!(merged.id, clip);
    assert_eq!(merged.start, 0.0);
    // 第一个片段本身有 16 拍，合并后的跨度取并集 = 0..16。
    assert_eq!(merged.length, 16.0);
    // 第二个片段的音移到 4.0 + 0.5 = 4.5 拍。
    let mut starts: Vec<f64> = merged.notes.iter().map(|n| n.start).collect();
    starts.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(starts, vec![1.0, 4.5]);

    e.undo();
    assert_eq!(e.project().tracks[0].clips.len(), 2);
    assert_eq!(
        e.project().tracks[0]
            .clips
            .iter()
            .find(|c| c.id == second)
            .unwrap()
            .notes
            .len(),
        1
    );
}

#[test]
fn merging_an_audio_clip_is_refused_instead_of_half_applied() {
    use ai_daw_engine::model::AudioSource;
    let (mut e, track, _clip) = setup();
    let events = e.execute(
        Command::AddAudioClip {
            track,
            start: 0.0,
            length: 4.0,
            source: AudioSource {
                path: "/tmp/a.wav".into(),
                offset_frames: 0,
                frames: Some(1000),
                rate: 1.0,
                pitch_semitones: 0.0,
            },
        },
        agent(),
    );
    let audio_clip = events
        .iter()
        .find_map(|ev| match ev {
            Event::AudioClipAdded { clip, .. } => Some(*clip),
            _ => None,
        })
        .unwrap();
    let midi_clip = e.project().tracks[0].clips[0].id;
    let before = e.project().clone();

    let produced = e.execute(
        Command::MergeClips {
            track,
            clips: vec![midi_clip, audio_clip],
        },
        agent(),
    );
    assert!(produced.is_empty(), "有音频片段时整体拒绝");
    assert_eq!(*e.project(), before, "拒绝时不得改动任何状态");
}

#[test]
fn midi_relative_coordinates_survive_move_resize_and_split() {
    let (mut e, track, clip) = setup();
    e.execute(
        Command::AddNotes {
            track,
            clip,
            notes: vec![note(60, 1.0, 2.0, 100)],
        },
        agent(),
    );
    // 移动片段：音符的相对坐标不变。
    e.execute(
        Command::MoveClip {
            track,
            clip,
            start: 8.0,
        },
        agent(),
    );
    let moved = &e.project().tracks[0].clips[0];
    assert_eq!(moved.start, 8.0);
    assert_eq!(moved.notes[0].start, 1.0);

    // 缩放：音符仍然相对片段起点。
    e.execute(
        Command::ResizeClip {
            track,
            clip,
            length: 20.0,
        },
        agent(),
    );
    assert_eq!(e.project().tracks[0].clips[0].notes[0].start, 1.0);

    // 切分：左半段保留原 id，右半段的音重新从 0 起算。
    let events = e.execute(
        Command::SplitClip {
            track,
            clip,
            beat: 9.5,
        },
        agent(),
    );
    assert!(matches!(events[0], Event::ClipSplit { .. }));
    let clips = &e.project().tracks[0].clips;
    assert_eq!(clips.len(), 2);
    let left = clips.iter().find(|c| c.id == clip).unwrap();
    let right = clips.iter().find(|c| c.id != clip).unwrap();
    assert_eq!(left.length, 1.5);
    assert_eq!(right.start, 9.5);
    // 跨过切点的音符被切成两半。
    assert_eq!(left.notes[0].start, 1.0);
    assert_eq!(left.notes[0].duration, 0.5);
    assert_eq!(right.notes[0].start, 0.0);
    assert_eq!(right.notes[0].duration, 1.5);
}

#[test]
fn composition_metadata_roundtrips_and_clears() {
    let (mut e, track, _clip) = setup();
    e.execute(
        Command::SetProjectHarmony {
            bars: vec![
                HarmonyBar {
                    start: 4.0,
                    length: 4.0,
                    symbol: "G13".into(),
                    source: Some("ai".into()),
                },
                HarmonyBar {
                    start: 0.0,
                    length: 4.0,
                    symbol: "Dm9".into(),
                    source: None,
                },
            ],
        },
        agent(),
    );
    // 事件应用时按起点排序。
    let harmony = &e.project().harmony;
    assert_eq!(
        harmony
            .iter()
            .map(|b| b.symbol.as_str())
            .collect::<Vec<_>>(),
        vec!["Dm9", "G13"]
    );

    e.execute(
        Command::SetProjectSections {
            sections: vec![Section {
                name: "主歌".into(),
                start: 0.0,
                end: 8.0,
                role: Some("verse".into()),
            }],
        },
        agent(),
    );
    assert_eq!(e.project().sections.len(), 1);

    e.execute(
        Command::SetTrackMeta {
            track,
            role: Some("lead".into()),
            pitch_range: Some(ai_daw_engine::model::PitchRange { min: 60, max: 84 }),
            pitch_bend_range: None,
        },
        agent(),
    );
    let t = e.project().track(track).unwrap();
    assert_eq!(t.role(), Some("lead"));
    assert_eq!(t.pitch_range().unwrap().min, 60);

    // 清空：两者都不写进 JSON。
    e.execute(
        Command::SetTrackMeta {
            track,
            role: None,
            pitch_range: None,
            pitch_bend_range: None,
        },
        agent(),
    );
    assert!(e.project().track(track).unwrap().meta.is_none());
    let json = serde_json::to_string(e.project()).unwrap();
    assert!(!json.contains("\"meta\""), "空元数据不该出现在 JSON 里");
    assert!(!json.contains("\"harmony\"") || json.contains("Dm9"));
}

#[test]
fn loading_an_old_project_assigns_note_ids_in_clip_order() {
    // 旧工程：音符没有 id 字段。
    let json = r#"{
        "name": "old",
        "transport": {"tempo": 120.0, "time_sig_num": 4, "time_sig_den": 4},
        "tracks": [{
            "id": 1, "name": "Keys", "instrument": null, "effects": [],
            "clips": [{"id": 1, "start": 0.0, "length": 4.0, "notes": [
                {"pitch": 60, "start": 0.0, "duration": 1.0, "velocity": 100},
                {"pitch": 64, "start": 1.0, "duration": 1.0, "velocity": 100}
            ]}],
            "mixer": {"gain_db": 0.0, "pan": 0.0, "mute": false, "solo": false}
        }],
        "master": {"gain_db": 0.0, "pan": 0.0, "mute": false, "solo": false}
    }"#;
    let project: ai_daw_engine::model::Project = serde_json::from_str(json).unwrap();
    let engine = Engine::from_project(project);
    let notes = &engine.project().tracks[0].clips[0].notes;
    assert_eq!(notes.iter().map(|n| n.id).collect::<Vec<_>>(), vec![1, 2]);
    // 补号后再存一次，id 就落进文件了（仍是合法 JSON，旧版本读它只是忽略 id）。
    let round = serde_json::to_string(engine.project()).unwrap();
    assert!(round.contains("\"id\":1"));
}

#[test]
fn note_tags_are_undoable_and_clearable() {
    let (mut e, track, clip) = setup();
    e.execute(
        Command::AddNotes {
            track,
            clip,
            notes: vec![note(60, 0.0, 1.0, 100)],
        },
        agent(),
    );
    let id = notes_of(&e, track, clip)[0].id;

    e.execute(
        Command::SetNoteTag {
            track,
            clip,
            id,
            tag: Some(NoteTag {
                ornament: Some("grace".into()),
                articulation: Some("staccato".into()),
            }),
        },
        agent(),
    );
    let tag = e.project().note_tag(track, clip, id).expect("标签应当写入");
    assert_eq!(tag.ornament.as_deref(), Some("grace"));
    assert_eq!(tag.articulation.as_deref(), Some("staccato"));

    e.undo();
    assert!(
        e.project().note_tag(track, clip, id).is_none(),
        "撤销应当移除标签"
    );
    e.redo();
    assert!(
        e.project().note_tag(track, clip, id).is_some(),
        "重做应当恢复标签"
    );

    e.execute(
        Command::SetNoteTag {
            track,
            clip,
            id,
            tag: None,
        },
        agent(),
    );
    assert!(
        e.project().note_tag(track, clip, id).is_none(),
        "None 应当清除标签"
    );
}

#[test]
fn batch_note_tags_are_one_undo_step() {
    let (mut e, track, clip) = setup();
    e.execute(
        Command::AddNotes {
            track,
            clip,
            notes: vec![note(60, 0.0, 1.0, 100), note(64, 0.0, 1.0, 100)],
        },
        agent(),
    );
    let ids: Vec<u64> = notes_of(&e, track, clip).iter().map(|n| n.id).collect();
    let before = e.undo_depth();

    e.execute(
        Command::SetNoteTags {
            track,
            clip,
            ids: ids.clone(),
            tag: Some(NoteTag {
                ornament: None,
                articulation: Some("harmonics".into()),
            }),
        },
        agent(),
    );
    assert_eq!(e.undo_depth(), before + 1, "多选写标签只能占一个撤销步");
    for id in &ids {
        assert_eq!(
            e.project()
                .note_tag(track, clip, *id)
                .and_then(|tag| tag.articulation.as_deref()),
            Some("harmonics")
        );
    }

    // 无效 id 不产生空撤销步。
    let before = e.undo_depth();
    e.execute(
        Command::SetNoteTags {
            track,
            clip,
            ids: vec![999_999],
            tag: None,
        },
        agent(),
    );
    assert_eq!(e.undo_depth(), before, "无效 id 不该产生撤销步");
}

#[test]
fn note_tags_are_pruned_on_remove_and_replace_but_come_back_on_undo() {
    let (mut e, track, clip) = setup();
    e.execute(
        Command::AddNotes {
            track,
            clip,
            notes: vec![note(60, 0.0, 1.0, 100)],
        },
        agent(),
    );
    let id = notes_of(&e, track, clip)[0].id;
    e.execute(
        Command::SetNoteTag {
            track,
            clip,
            id,
            tag: Some(NoteTag {
                ornament: Some("grace".into()),
                articulation: None,
            }),
        },
        agent(),
    );
    assert!(e.project().note_tag(track, clip, id).is_some());

    // 删除音符 → 标签必须一起清掉，避免 id 复用时误标。
    e.execute(
        Command::RemoveNotesById {
            track,
            clip,
            ids: vec![id],
        },
        agent(),
    );
    assert!(e.project().note_tags.is_empty(), "删除音符后不应留下标签");
    e.undo();
    assert!(
        e.project().note_tag(track, clip, id).is_some(),
        "撤销删除应当恢复标签"
    );

    // 整体替换会从 1 重新发 id：旧标签绝不能套到新音符上。
    e.execute(
        Command::ReplaceNotes {
            track,
            clip,
            notes: vec![note(64, 0.0, 1.0, 100)],
        },
        agent(),
    );
    assert!(e.project().note_tags.is_empty(), "整体替换后不应保留旧标签");
    e.undo();
    assert!(
        e.project().note_tag(track, clip, id).is_some(),
        "撤销替换应当恢复标签"
    );
}

#[test]
fn note_tags_follow_a_split_clip() {
    let (mut e, track, clip) = setup();
    e.execute(
        Command::AddNotes {
            track,
            clip,
            notes: vec![
                note(60, 0.0, 1.0, 100),
                note(64, 8.0, 1.0, 100),
                note(67, 3.0, 2.0, 100), // 跨过切点
            ],
        },
        agent(),
    );
    let ids: Vec<u64> = notes_of(&e, track, clip).iter().map(|n| n.id).collect();
    for id in &ids {
        e.execute(
            Command::SetNoteTag {
                track,
                clip,
                id: *id,
                tag: Some(NoteTag {
                    ornament: Some("grace".into()),
                    articulation: None,
                }),
            },
            agent(),
        );
    }

    e.execute(
        Command::SplitClip {
            track,
            clip,
            beat: 4.0,
        },
        agent(),
    );
    let new_clip = e
        .project()
        .track(track)
        .unwrap()
        .clips
        .iter()
        .find(|c| c.id != clip)
        .expect("切分后应当有新片段")
        .id;
    assert!(
        e.project().note_tag(track, clip, ids[0]).is_some(),
        "左半段标签应留在原片段"
    );
    assert!(
        e.project().note_tag(track, new_clip, ids[1]).is_some(),
        "右半段标签应迁到新片段"
    );
    assert!(
        e.project().note_tag(track, clip, ids[1]).is_none(),
        "旧片段不应残留右半段标签"
    );
    // 跨切点的音符在左右两半共用 id：标签应当两边都保留。
    assert!(
        e.project().note_tag(track, clip, ids[2]).is_some(),
        "跨切点音符的左半应保留标签"
    );
    assert!(
        e.project().note_tag(track, new_clip, ids[2]).is_some(),
        "跨切点音符的右半也应保留标签"
    );
}

#[test]
fn note_tags_follow_duplicate_and_merge() {
    let (mut e, track, clip) = setup();
    e.execute(
        Command::AddNotes {
            track,
            clip,
            notes: vec![note(60, 0.0, 1.0, 100)],
        },
        agent(),
    );
    let id = notes_of(&e, track, clip)[0].id;
    e.execute(
        Command::SetNoteTag {
            track,
            clip,
            id,
            tag: Some(NoteTag {
                ornament: Some("grace".into()),
                articulation: None,
            }),
        },
        agent(),
    );

    e.execute(
        Command::DuplicateClip {
            track,
            clip,
            to_track: None,
            start: Some(4.0),
        },
        agent(),
    );
    let duplicated = e
        .project()
        .track(track)
        .unwrap()
        .clips
        .iter()
        .find(|c| c.id != clip)
        .expect("复制后应当有新片段")
        .id;
    assert!(
        e.project().note_tag(track, duplicated, id).is_some(),
        "复制片段应当带上音符标签"
    );

    // 再来一段带标签的片段，合并后标签应落到目标片段。
    e.execute(
        Command::AddClip {
            track,
            start: 16.0,
            length: 4.0,
        },
        agent(),
    );
    let second = e
        .project()
        .track(track)
        .unwrap()
        .clips
        .iter()
        .max_by(|a, b| a.start.partial_cmp(&b.start).unwrap())
        .unwrap()
        .id;
    e.execute(
        Command::AddNotes {
            track,
            clip: second,
            notes: vec![note(67, 0.0, 1.0, 100)],
        },
        agent(),
    );
    let second_id = notes_of(&e, track, second)[0].id;
    e.execute(
        Command::SetNoteTag {
            track,
            clip: second,
            id: second_id,
            tag: Some(NoteTag {
                ornament: Some("mordent".into()),
                articulation: None,
            }),
        },
        agent(),
    );
    e.execute(
        Command::MergeClips {
            track,
            clips: vec![clip, second],
        },
        agent(),
    );
    let merged_ids: Vec<u64> = notes_of(&e, track, clip).iter().map(|n| n.id).collect();
    assert!(
        merged_ids
            .iter()
            .any(|id| e.project().note_tag(track, clip, *id).is_some()),
        "合并后目标片段应当保留标签"
    );
}
