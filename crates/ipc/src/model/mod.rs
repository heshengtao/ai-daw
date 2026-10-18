pub mod clip;
pub mod project;
pub mod track;
pub mod transport;

pub use clip::{
    clamp_beats, clamp_range, AudioSource, Clip, ClipId, ClipPlacement, ClipRef, FitMode, Note,
    NoteEdit, NotePatch, NoteRange, StrumDirection, VelocityShape,
};
pub use project::{HarmonyBar, Project, Section};
pub use track::{
    lane_value_at, AutomationLane, AutomationPoint, MixerChannel, PitchRange, PluginInstance, Send,
    Track, TrackId, TrackMeta,
};
pub use transport::{Keyframe, Transport};
