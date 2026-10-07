use crate::event::Event;
use crate::model::{Clip, PluginInstance, Project};

/// 把一个事件应用到工程状态。
pub fn apply(project: &mut Project, event: &Event) {
    match event {
        Event::TempoChanged { tempo, .. } => project.transport.tempo = *tempo,
        Event::TrackAdded { id, name, .. } => {
            project.tracks.push(crate::model::Track {
                id: *id,
                name: name.clone(),
                instrument: None,
                clips: vec![],
                mixer: crate::model::MixerChannel::default(),
            });
        }
        Event::TrackRemoved { id, .. } => project.tracks.retain(|t| t.id != *id),
        Event::TrackInstrumentSet { track, plugin_id, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.instrument = Some(PluginInstance {
                    plugin_id: plugin_id.clone(),
                    params: Default::default(),
                    state: vec![],
                });
            }
        }
        Event::ClipAdded { track, clip, start, length, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.clips.push(Clip { id: *clip, start: *start, length: *length, notes: vec![] });
            }
        }
        Event::ClipRemoved { track, clip, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.clips.retain(|c| c.id != *clip);
            }
        }
        Event::NoteAdded { track, clip, note, .. } => {
            if let Some(t) = project.track_mut(*track) {
                if let Some(c) = t.clips.iter_mut().find(|c| c.id == *clip) {
                    c.notes.push(*note);
                }
            }
        }
        Event::GainChanged { track, gain_db, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.mixer.gain_db = *gain_db;
            }
        }
        Event::PanChanged { track, pan, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.mixer.pan = *pan;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
    use crate::event::{Actor, Event};
    use crate::model::{ClipId, Note, Project, TrackId};

    #[test]
    fn add_track_then_clip_then_note() {
        let mut p = Project::new("t");
        let t = TrackId(1);
        apply(&mut p, &Event::TrackAdded { actor: Actor::Human, id: t, name: "Keys".into() });
        assert_eq!(p.tracks.len(), 1);
        assert_eq!(p.track(t).unwrap().name, "Keys");

        let c = ClipId(1);
        apply(&mut p, &Event::ClipAdded { actor: Actor::Human, track: t, clip: c, start: 0.0, length: 4.0 });
        assert_eq!(p.track(t).unwrap().clips.len(), 1);

        apply(
            &mut p,
            &Event::NoteAdded {
                actor: Actor::Human,
                track: t,
                clip: c,
                note: Note { pitch: 60, start: 0.0, duration: 1.0, velocity: 100 },
            },
        );
        assert_eq!(p.track(t).unwrap().clips[0].notes.len(), 1);

        let _ = Command::SetTempo(140.0);
    }

    #[test]
    fn set_tempo_event_updates_transport() {
        let mut p = Project::new("t");
        apply(&mut p, &Event::TempoChanged { actor: Actor::Human, tempo: 90.0 });
        assert_eq!(p.transport.tempo, 90.0);
    }
}
