#![forbid(unsafe_code)]

pub mod apply;
pub mod audio;
pub mod bus;
pub mod command;
pub mod command_queue;
pub mod event;
pub mod model;
pub mod player;
pub mod registry;
pub mod render;
#[cfg(feature = "realtime")]
pub mod realtime;
#[cfg(feature = "realtime")]
pub mod recorder;
pub mod schedule;
pub mod scope;
pub mod stretch;
pub mod tempo;
pub mod transport;
pub mod wav;

pub use bus::Engine;
pub use registry::PluginRegistry;
/// 采集环的 SPSC 类型：`capture_block` / `drain_into` 的公开签名用到它，集成测试也要用。
pub use rtrb;

/// 界面实时波形缓冲的容量（采样数）。
///
/// 8192 个采样在 48 kHz 下约 170 ms——够画一个稳定的示波器画面。
pub const SCOPE_CAPACITY: usize = 8192;

/// 音频块帧数。
pub const BLOCK_SIZE: usize = 256;

/// 新建工程的一条默认轨道：显示名 + 默认挂载的乐器 id。
#[derive(Debug, Clone, Copy)]
pub struct StarterTrack {
    pub name: &'static str,
    /// 默认乐器。用预设 id 而不是 `BUILTIN_SYNTH`，让每条功能位一上来就有合适的音色。
    pub plugin_id: &'static str,
}

/// 新建工程的默认分轨（数组顺序即自上而下的轨道顺序）。
///
/// 素材从上往下、由主到次：**主旋律 → 和弦 → 低音 → 鼓组 → 铺底**。
/// 每条轨道挂一个内置合成器预设：鼓组用拨弦（最短促的包络）、低音用 Bass、
/// 和弦与铺底用 Pad、主旋律用 Lead。鼓组专用音源（采样器）属于 Stage C，
/// 接入后这里只需改 `plugin_id`。
///
/// `name` 只是**兜底默认名**：界面会按当前语言把一批名字随新建命令传进来
/// （见 `Command::NewProject::track_names`），落进工程后就不再随语言变化。
pub const STARTER_TRACKS: [StarterTrack; 5] = [
    StarterTrack { name: "主旋律", plugin_id: "builtin.synth.lead" },
    StarterTrack { name: "和弦", plugin_id: "builtin.synth.strings" },
    StarterTrack { name: "低音", plugin_id: "builtin.synth.bass" },
    StarterTrack { name: "鼓组", plugin_id: "builtin.synth.drums" },
    StarterTrack { name: "铺底", plugin_id: "builtin.synth.pad" },
];

/// 拍 → 秒。
pub fn beats_to_seconds(beats: f64, tempo: f64) -> f64 {
    beats * 60.0 / tempo
}
