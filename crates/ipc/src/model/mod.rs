pub mod clip;
pub mod project;
pub mod track;
pub mod transport;

pub use clip::{
    clamp_beats, clamp_range, AudioSource, BeatFilter, Clip, ClipId, ClipPlacement, ClipRef, FitMode,
    Note, NoteEdit, NotePatch, NoteRange, PitchExpression, StrumDirection, VelocityShape,
};
pub use project::{note_tag_key, parse_note_tag_key, HarmonyBar, NoteTag, Project, Section};
pub use track::{
    lane_value_at, AutomationLane, AutomationPoint, MixerChannel, PitchRange, PluginInstance, Send,
    Track, TrackId, TrackMeta,
};
pub use transport::{Keyframe, Transport};
