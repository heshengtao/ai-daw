use crate::event::Event;
use crate::model::{Clip, MixerChannel, Note, PluginInstance, Project};

/// 把一个事件应用到工程状态。
///
/// 这里是防御的第二道关：即使事件是手写 JSON 或旧版本工程反序列化出来的，也必须
/// 落在合法范围内（归一化与夹紧），否则越界值会污染整条时间轴或渲染时长。
pub fn apply(project: &mut Project, event: &Event) {
    match event {
        Event::TempoChanged { tempo, .. } => project.transport.tempo = tempo.clamp(1.0, 1000.0),
        Event::TimeSignatureChanged { num, den, .. } => {
            // 手写 / 加载来的事件同样要归一化，避免分母为 0 导致小节长度除不尽。
            let (num, den) = crate::model::Transport::normalize_time_sig(*num, *den);
            project.transport.time_sig_num = num;
            project.transport.time_sig_den = den;
        }
        Event::KeyframeSet { beat, tempo, num, den, .. } => {
            if let Some(kf) = crate::model::Transport::normalize_keyframe(crate::model::Keyframe {
                beat: *beat,
                tempo: *tempo,
                time_sig_num: *num,
                time_sig_den: *den,
            }) {
                project.transport.upsert_keyframe(kf);
            }
        }
        Event::KeyframeRemoved { beat, .. } => project.transport.remove_keyframe(*beat),
        Event::TrackAdded { id, name, at, .. } => {
            let track = crate::model::Track {
                meta: None,
                id: *id,
                name: name.clone(),
                instrument: None,
                effects: vec![],
                clips: vec![],
                mixer: MixerChannel::default(),
                sends: vec![],
                automation: vec![],
            };
            match at {
                // 事件自带插入位置：撤销/重放才能重现「插在中间」。
                Some(index) => project.tracks.insert((*index).min(project.tracks.len()), track),
                None => project.tracks.push(track),
            }
        }
        Event::TrackRemoved { id, .. } => project.tracks.retain(|t| t.id != *id),
        Event::TrackRenamed { track, name, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.name = name.clone();
            }
        }
        Event::TrackInstrumentSet { track, plugin_id, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.instrument = Some(PluginInstance {
                    plugin_id: plugin_id.clone(),
                    params: Default::default(),
                    state: vec![],
                });
            }
        }
        Event::TrackInstrumentCleared { track, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.instrument = None;
            }
        }
        Event::ClipAdded { track, clip, start, length, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.clips.push(Clip {
                    id: *clip,
                    start: Clip::normalize_start(*start),
                    length: Clip::normalize_length(*length),
                    notes: vec![],
                    audio: None,
                });
            }
        }
        Event::AudioClipAdded { track, clip, start, length, source, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.clips.push(Clip {
                    id: *clip,
                    start: Clip::normalize_start(*start),
                    length: Clip::normalize_length(*length),
                    notes: vec![],
                    audio: Some(source.clone()),
                });
            }
        }
        Event::ClipAudioChanged { track, clip, audio, .. } => {
            if let Some(c) = find_clip_mut(project, *track, *clip) {
                c.audio = audio.clone();
                // 变成音频片段时音符必须清空（两者不能共存）；变回 MIDI 片段时音符本来就是空的。
                if c.audio.is_some() {
                    c.notes.clear();
                }
            }
        }
        Event::ClipRateChanged { track, clip, rate, pitch_semitones, .. } => {
            if let Some(c) = find_clip_mut(project, *track, *clip) {
                if let Some(audio) = c.audio.as_mut() {
                    audio.rate = crate::model::AudioSource::normalize_rate(*rate);
                    audio.pitch_semitones =
                        crate::model::AudioSource::normalize_pitch(*pitch_semitones);
                }
            }
        }
        Event::ClipRemoved { track, clip, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.clips.retain(|c| c.id != *clip);
            }
        }
        Event::ClipMoved { track, clip, start, .. } => {
            if let Some(c) = find_clip_mut(project, *track, *clip) {
                c.start = Clip::normalize_start(*start);
            }
        }
        Event::ClipsMoved { items, .. } => {
            for item in items {
                // 目标轨道不存在 → 整条跳过：一次非法拖动不应该改变任何状态
                // （包括起点），更不应该把片段弄丢。
                if project.track(item.to_track).is_none() {
                    continue;
                }
                // 先从原轨道摘下来；摘不到说明片段已被删除，同样跳过。
                let Some(from) = project.track_mut(item.track) else { continue };
                let Some(index) = from.clips.iter().position(|c| c.id == item.clip) else { continue };
                let mut clip = from.clips.remove(index);
                clip.start = Clip::normalize_start(item.start);

                let dest = project
                    .track_mut(item.to_track)
                    .expect("目标轨道已在上面确认存在");
                // 按起点有序插入，保证轨道内片段顺序稳定（渲染不依赖顺序，但可读性与 diff 依赖）。
                let pos = dest.clips.partition_point(|c| c.start <= clip.start);
                dest.clips.insert(pos, clip);
            }
        }
        Event::ClipResized { track, clip, length, .. } => {
            if let Some(c) = find_clip_mut(project, *track, *clip) {
                c.length = Clip::normalize_length(*length);
            }
        }
        Event::ClipSplit { track, clip, new_clip, at_beat, left_notes, right_notes, left_audio, right_audio, .. } => {
            if let Some(t) = project.track_mut(*track) {
                let Some(pos) = t.clips.iter().position(|c| c.id == *clip) else { return };
                let end = t.clips[pos].start + t.clips[pos].length;
                {
                    let left = &mut t.clips[pos];
                    left.length = Clip::normalize_length(at_beat - left.start);
                    left.notes = left_notes.clone();
                    left.audio = left_audio.clone();
                }
                let right = Clip {
                    id: *new_clip,
                    start: Clip::normalize_start(*at_beat),
                    length: Clip::normalize_length(end - *at_beat),
                    notes: right_notes.clone(),
                    audio: right_audio.clone(),
                };
                // 按起点有序插入，维持轨道内片段顺序稳定。
                let at = t.clips.partition_point(|c| c.start <= right.start);
                t.clips.insert(at, right);
            }
        }
        Event::NoteAdded { track, clip, note, .. } => {
            if let Some(c) = find_clip_mut(project, *track, *clip) {
                // 事件里的 id 正常由命令层发放；手写 / 旧事件可能带 0，就地补一个。
                let mut note = Note::normalize(*note);
                if note.id == 0 {
                    note.id = c.next_note_id();
                }
                c.notes.push(note);
            }
        }
        Event::NotesAdded { track, clip, notes, .. } => {
            if let Some(c) = find_clip_mut(project, *track, *clip) {
                // 先算一次高水位，避免逐个 `next_note_id()` 反复扫描整段音符。
                let mut next = c.next_note_id();
                let mut seen: std::collections::HashSet<u64> =
                    c.notes.iter().map(|n| n.id).collect();
                for note in notes {
                    let mut note = Note::normalize(*note);
                    if note.id == 0 || !seen.insert(note.id) {
                        note.id = next;
                        next += 1;
                        seen.insert(note.id);
                    }
                    c.notes.push(note);
                }
            }
        }
        Event::NotesReplaced { track, clip, notes, .. } => {
            if let Some(c) = find_clip_mut(project, *track, *clip) {
                let mut next = 1u64;
                c.notes = notes
                    .iter()
                    .map(|note| {
                        let mut note = Note::normalize(*note);
                        if note.id == 0 {
                            note.id = next;
                        }
                        next = next.max(note.id + 1);
                        note
                    })
                    .collect();
                // 同一批里出现重复 id（手写事件）时补号，保证「片段内唯一」这条不变式。
                let mut seen = std::collections::HashSet::new();
                let mut next = c.notes.iter().map(|n| n.id).max().unwrap_or(0) + 1;
                for note in &mut c.notes {
                    if !seen.insert(note.id) {
                        note.id = next;
                        next += 1;
                        seen.insert(note.id);
                    }
                }
            }
        }
        Event::NoteChanged { track, clip, index, note, .. } => {
            if let Some(c) = find_clip_mut(project, *track, *clip) {
                if let Some(slot) = c.notes.get_mut(*index) {
                    *slot = Note::normalize(*note);
                }
            }
        }
        Event::NotesChanged { track, clip, entries, .. } => {
            if let Some(c) = find_clip_mut(project, *track, *clip) {
                for entry in entries {
                    if let Some(slot) = c.notes.get_mut(entry.index) {
                        *slot = Note::normalize(entry.note);
                    }
                }
            }
        }
        Event::NoteRemoved { track, clip, index, .. } => {
            if let Some(c) = find_clip_mut(project, *track, *clip) {
                if *index < c.notes.len() {
                    c.notes.remove(*index);
                }
            }
        }
        Event::NotesRemoved { track, clip, indices, .. } => {
            if let Some(c) = find_clip_mut(project, *track, *clip) {
                // 倒序删除：正序删除会让后面的序号整体前移，删错对象。
                let mut targets: Vec<usize> =
                    indices.iter().copied().filter(|i| *i < c.notes.len()).collect();
                targets.sort_unstable();
                targets.dedup();
                for index in targets.into_iter().rev() {
                    c.notes.remove(index);
                }
            }
        }
        Event::ClipsRemoved { items, .. } => {
            for item in items {
                if let Some(t) = project.track_mut(item.track) {
                    t.clips.retain(|c| c.id != item.clip);
                }
            }
        }
        Event::GainChanged { track, gain_db, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.mixer.gain_db = MixerChannel::normalize_gain_db(*gain_db);
            }
        }
        Event::PanChanged { track, pan, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.mixer.pan = MixerChannel::normalize_pan(*pan);
            }
        }
        Event::MuteChanged { track, mute, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.mixer.mute = *mute;
            }
        }
        Event::SoloChanged { track, solo, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.mixer.solo = *solo;
            }
        }
        Event::MasterGainChanged { gain_db, .. } => {
            project.master.gain_db = MixerChannel::normalize_gain_db(*gain_db);
        }
        Event::TrackSendsChanged { track, sends, .. } => {
            if let Some(t) = project.track_mut(*track) {
                // 防御：手写 JSON / 旧工程也可能带来越界增益；自发送没有意义，丢掉。
                t.sends = sends
                    .iter()
                    .filter(|s| s.target != *track)
                    .map(|s| s.normalize())
                    .collect();
            }
        }
        Event::AutomationChanged { track, param, points, .. } => {
            if let Some(t) = project.track_mut(*track) {
                let param = crate::model::AutomationLane::normalize_param(param);
                if param.is_empty() {
                    return;
                }
                t.automation.retain(|lane| lane.param != param);
                if !points.is_empty() {
                    // 点按 beat 升序；非有限值已被 translate 滤掉，这里再兜一层。
                    let mut points: Vec<crate::model::AutomationPoint> = points
                        .iter()
                        .copied()
                        .filter(|p| p.beat.is_finite() && p.value.is_finite())
                        .map(|p| crate::model::AutomationPoint { beat: p.beat.max(0.0), value: p.value })
                        .collect();
                    points.sort_by(|a, b| a.beat.partial_cmp(&b.beat).unwrap_or(std::cmp::Ordering::Equal));
                    t.automation.push(crate::model::AutomationLane { param, points });
                    t.automation.sort_by(|a, b| a.param.cmp(&b.param));
                }
            }
        }
        Event::ParamChanged { track, param, value, .. } => {
            if let Some(t) = project.track_mut(*track) {
                if let Some(inst) = t.instrument.as_mut() {
                    // 非有限值会让插件内部出现 NaN 并污染整条音频链，直接拒绝。
                    if value.is_finite() {
                        inst.params.insert(param.clone(), *value);
                    }
                }
            }
        }
        Event::EffectAdded { track, plugin_id, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.effects.push(PluginInstance {
                    plugin_id: plugin_id.clone(),
                    params: Default::default(),
                    state: vec![],
                });
            }
        }
        Event::EffectRemoved { track, index, .. } => {
            if let Some(t) = project.track_mut(*track) {
                if *index < t.effects.len() {
                    t.effects.remove(*index);
                }
            }
        }
        Event::EffectMoved { track, from, to, .. } => {
            if let Some(t) = project.track_mut(*track) {
                if *from < t.effects.len() {
                    let fx = t.effects.remove(*from);
                    // `to` 是「移除后」的目标下标，夹到合法范围（含末尾）。
                    let dest = (*to).min(t.effects.len());
                    t.effects.insert(dest, fx);
                }
            }
        }
        Event::EffectParamChanged { track, index, param, value, .. } => {
            if let Some(t) = project.track_mut(*track) {
                if let Some(fx) = t.effects.get_mut(*index) {
                    if value.is_finite() {
                        fx.params.insert(param.clone(), *value);
                    }
                }
            }
        }
        Event::ProjectRenamed { name, .. } => project.name = name.clone(),
        Event::ProjectCleared { name, .. } => *project = Project::new(name.clone()),
        Event::TrackMetaChanged { track, role, pitch_range, .. } => {
            if let Some(t) = project.track_mut(*track) {
                // 两者都空 → 元数据整个消失（JSON 里不再出现 `meta` 键）。
                let meta = crate::model::TrackMeta {
                    role: role.clone().filter(|r| !r.trim().is_empty()),
                    pitch_range: pitch_range.map(|r| r.normalize()),
                };
                t.meta = if meta.is_empty() { None } else { Some(meta) };
            }
        }
        Event::ProjectHarmonyChanged { bars, .. } => {
            let mut bars: Vec<crate::model::HarmonyBar> = bars
                .iter()
                .filter(|b| b.start.is_finite() && b.length.is_finite() && b.length > 0.0)
                .map(|b| crate::model::HarmonyBar {
                    start: b.start.max(0.0),
                    length: b.length.max(1e-6),
                    symbol: b.symbol.trim().to_string(),
                    source: b.source.clone(),
                })
                .filter(|b| !b.symbol.is_empty())
                .collect();
            bars.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
            project.harmony = bars;
        }
        Event::ProjectSectionsChanged { sections, .. } => {
            let mut sections: Vec<crate::model::Section> = sections
                .iter()
                .filter(|s| s.start.is_finite() && s.end.is_finite())
                .map(|s| crate::model::Section {
                    name: s.name.trim().to_string(),
                    start: s.start.max(0.0),
                    end: s.end.max(0.0),
                    role: s.role.clone(),
                })
                .filter(|s| !s.name.is_empty() && s.end > s.start)
                .collect();
            sections.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
            project.sections = sections;
        }
    }
}

/// 按「轨道 + 片段」定位片段；任一层缺失时返回 `None`（事件指向已删除对象是合法情形）。
fn find_clip_mut(project: &mut Project, track: crate::model::TrackId, clip: crate::model::ClipId) -> Option<&mut Clip> {
    project.track_mut(track)?.clips.iter_mut().find(|c| c.id == clip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Actor, Event};
    use crate::model::{ClipId, Note, Project, TrackId};

    fn project_with_clip() -> (Project, TrackId, ClipId) {
        let mut p = Project::new("t");
        let t = TrackId(1);
        apply(&mut p, &Event::TrackAdded { actor: Actor::Human, id: t, name: "Keys".into(), at: None });
        let c = ClipId(1);
        apply(&mut p, &Event::ClipAdded { actor: Actor::Human, track: t, clip: c, start: 0.0, length: 4.0 });
        (p, t, c)
    }

    #[test]
    fn add_track_then_clip_then_note() {
        let (mut p, t, c) = project_with_clip();
        assert_eq!(p.tracks.len(), 1);
        assert_eq!(p.track(t).unwrap().name, "Keys");
        assert_eq!(p.track(t).unwrap().clips.len(), 1);

        apply(
            &mut p,
            &Event::NoteAdded {
                actor: Actor::Human,
                track: t,
                clip: c,
                note: Note { id: 0, pitch: 60, start: 0.0, duration: 1.0, velocity: 100, volume: 127, offset: 0.0, legato: false },
            },
        );
        assert_eq!(p.track(t).unwrap().clips[0].notes.len(), 1);
    }

    #[test]
    fn set_tempo_event_updates_transport() {
        let mut p = Project::new("t");
        apply(&mut p, &Event::TempoChanged { actor: Actor::Human, tempo: 90.0 });
        assert_eq!(p.transport.tempo, 90.0);
    }

    #[test]
    fn time_signature_event_is_defensively_normalized() {
        // 手写 / 加载来的事件也必须落在合法范围，分母 0 会让小节长度除不尽。
        let mut p = Project::new("t");
        apply(&mut p, &Event::TimeSignatureChanged { actor: Actor::Human, num: 0, den: 0 });
        assert_eq!(p.transport.time_sig_num, 1);
        assert_eq!(p.transport.time_sig_den, 1);
        apply(&mut p, &Event::TimeSignatureChanged { actor: Actor::Human, num: 6, den: 8 });
        assert_eq!(p.transport.time_sig_num, 6);
        assert_eq!(p.transport.time_sig_den, 8);
    }

    #[test]
    fn tempo_event_is_defensively_clamped() {
        // 手写 / 加载来的事件也必须落在安全区间，否则渲染时长会溢出。
        let mut p = Project::new("t");
        apply(&mut p, &Event::TempoChanged { actor: Actor::Human, tempo: 0.0 });
        assert_eq!(p.transport.tempo, 1.0);
        apply(&mut p, &Event::TempoChanged { actor: Actor::Human, tempo: 100000.0 });
        assert_eq!(p.transport.tempo, 1000.0);
    }

    #[test]
    fn rename_move_resize_and_remove_clip() {
        let (mut p, t, c) = project_with_clip();
        apply(&mut p, &Event::TrackRenamed { actor: Actor::Human, track: t, name: "Bass".into() });
        assert_eq!(p.track(t).unwrap().name, "Bass");

        apply(&mut p, &Event::ClipMoved { actor: Actor::Human, track: t, clip: c, start: 2.5 });
        assert_eq!(p.track(t).unwrap().clips[0].start, 2.5);

        apply(&mut p, &Event::ClipResized { actor: Actor::Human, track: t, clip: c, length: 0.0 });
        // 零长度片段会让 NoteOn/NoteOff 同帧，必须被抬到最短长度。
        assert_eq!(p.track(t).unwrap().clips[0].length, Clip::MIN_LENGTH_BEATS);

        apply(&mut p, &Event::ClipRemoved { actor: Actor::Human, track: t, clip: c });
        assert!(p.track(t).unwrap().clips.is_empty());
    }

    #[test]
    fn note_change_and_remove_are_index_addressed_and_bounded() {
        let (mut p, t, c) = project_with_clip();
        for pitch in [60u8, 64, 67] {
            apply(
                &mut p,
                &Event::NoteAdded {
                    actor: Actor::Human,
                    track: t,
                    clip: c,
                    note: Note { id: 0, pitch, start: 0.0, duration: 1.0, velocity: 100, volume: 127, offset: 0.0, legato: false },
                },
            );
        }
        apply(
            &mut p,
            &Event::NoteChanged {
                actor: Actor::Human,
                track: t,
                clip: c,
                index: 1,
                note: Note { id: 0, pitch: 200, start: -1.0, duration: 0.0, velocity: 255, volume: 127, offset: 0.0, legato: false },
            },
        );
        let n = p.track(t).unwrap().clips[0].notes[1];
        assert_eq!(n.pitch, 127);
        // 音符起点是**相对片段起点**的拍，允许为负（左边缘收起时被裁掉的音符要留着）。
        assert_eq!(n.start, -1.0);
        assert_eq!(n.duration, Note::MIN_DURATION_BEATS);
        assert_eq!(n.velocity, 127);

        // 越界索引不得 panic，也不得改动任何东西。
        apply(&mut p, &Event::NoteRemoved { actor: Actor::Human, track: t, clip: c, index: 99 });
        assert_eq!(p.track(t).unwrap().clips[0].notes.len(), 3);

        apply(&mut p, &Event::NoteRemoved { actor: Actor::Human, track: t, clip: c, index: 0 });
        let pitches: Vec<u8> = p.track(t).unwrap().clips[0].notes.iter().map(|n| n.pitch).collect();
        assert_eq!(pitches, vec![127, 67]);
    }

    #[test]
    fn mixer_events_are_clamped_and_master_is_separate() {
        let (mut p, t, _) = project_with_clip();
        apply(&mut p, &Event::GainChanged { actor: Actor::Human, track: t, gain_db: 100.0 });
        assert_eq!(p.track(t).unwrap().mixer.gain_db, MixerChannel::MAX_GAIN_DB);
        apply(&mut p, &Event::PanChanged { actor: Actor::Human, track: t, pan: -9.0 });
        assert_eq!(p.track(t).unwrap().mixer.pan, -1.0);
        apply(&mut p, &Event::MuteChanged { actor: Actor::Human, track: t, mute: true });
        apply(&mut p, &Event::SoloChanged { actor: Actor::Human, track: t, solo: true });
        assert!(p.track(t).unwrap().mixer.mute);
        assert!(p.track(t).unwrap().mixer.solo);

        apply(&mut p, &Event::MasterGainChanged { actor: Actor::Human, gain_db: -6.0 });
        assert_eq!(p.master.gain_db, -6.0);
        // 总输出是独立通道，不能被轨道增益波及。
        assert_eq!(p.track(t).unwrap().mixer.gain_db, MixerChannel::MAX_GAIN_DB);
    }

    #[test]
    fn param_event_requires_an_instrument_and_rejects_non_finite() {
        let (mut p, t, _) = project_with_clip();
        // 还没有乐器：事件必须被安静地忽略。
        apply(&mut p, &Event::ParamChanged { actor: Actor::Human, track: t, param: "gain".into(), value: 0.5 });
        assert!(p.track(t).unwrap().instrument.is_none());

        apply(&mut p, &Event::TrackInstrumentSet { actor: Actor::Human, track: t, plugin_id: "builtin.synth".into() });
        apply(&mut p, &Event::ParamChanged { actor: Actor::Human, track: t, param: "gain".into(), value: 0.5 });
        apply(&mut p, &Event::ParamChanged { actor: Actor::Human, track: t, param: "attack".into(), value: f32::NAN });
        let params = &p.track(t).unwrap().instrument.as_ref().unwrap().params;
        assert_eq!(params.get("gain"), Some(&0.5));
        assert_eq!(params.get("attack"), None);
    }

    #[test]
    fn effect_chain_add_move_remove_and_param() {
        let (mut p, t, _) = project_with_clip();
        apply(&mut p, &Event::EffectAdded { actor: Actor::Human, track: t, plugin_id: "a".into() });
        apply(&mut p, &Event::EffectAdded { actor: Actor::Human, track: t, plugin_id: "b".into() });
        assert_eq!(p.track(t).unwrap().effects.len(), 2);

        apply(&mut p, &Event::EffectMoved { actor: Actor::Human, track: t, from: 0, to: 1 });
        assert_eq!(p.track(t).unwrap().effects[0].plugin_id, "b");
        assert_eq!(p.track(t).unwrap().effects[1].plugin_id, "a");

        apply(&mut p, &Event::EffectParamChanged { actor: Actor::Human, track: t, index: 0, param: "mix".into(), value: 0.5 });
        assert_eq!(p.track(t).unwrap().effects[0].params.get("mix"), Some(&0.5));

        apply(&mut p, &Event::EffectRemoved { actor: Actor::Human, track: t, index: 0 });
        assert_eq!(p.track(t).unwrap().effects.len(), 1);
        assert_eq!(p.track(t).unwrap().effects[0].plugin_id, "a");

        // 越界索引必须安静忽略，不得 panic。
        apply(&mut p, &Event::EffectRemoved { actor: Actor::Human, track: t, index: 99 });
        apply(&mut p, &Event::EffectMoved { actor: Actor::Human, track: t, from: 99, to: 0 });
        assert_eq!(p.track(t).unwrap().effects.len(), 1);
    }

    #[test]
    fn events_pointing_at_missing_objects_are_ignored() {
        let mut p = Project::new("t");
        let ghost = TrackId(42);
        apply(&mut p, &Event::ClipMoved { actor: Actor::Human, track: ghost, clip: ClipId(1), start: 4.0 });
        apply(&mut p, &Event::GainChanged { actor: Actor::Human, track: ghost, gain_db: -3.0 });
        apply(&mut p, &Event::TrackRenamed { actor: Actor::Human, track: ghost, name: "x".into() });
        assert!(p.tracks.is_empty());
    }
}
