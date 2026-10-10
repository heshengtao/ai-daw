pub mod clip;
pub mod project;
pub mod track;
pub mod transport;

pub use clip::{
    clamp_beats, clamp_range, AudioSource, Clip, ClipId, ClipPlacement, ClipRef, Note, NoteEdit,
};
pub use project::Project;
pub use track::{
    lane_value_at, AutomationLane, AutomationPoint, MixerChannel, PluginInstance, Send, Track,
    TrackId,
};
pub use transport::{Keyframe, Transport};
