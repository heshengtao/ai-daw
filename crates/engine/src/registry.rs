use std::collections::HashMap;

use daw_party_plugin_api::{
    IoLayout, ParamId, ParamInfo, Plugin, PluginDescriptor, PluginError, PluginKind,
    ProcessContext, TimedMidiEvent,
};

use crate::model::TrackId;

/// 内置合成器的插件 id（`PluginRegistry::with_builtins` 注册的就是它）。
///
/// 抽成常量是因为「新建工程的默认分轨」要挂乐器，而引擎不该散落魔法字符串。
pub const BUILTIN_SYNTH: &str = "builtin.synth";

/// 内置合成器的**出厂预设**：同一套信号链，不同的参数补丁。
///
/// 为什么把预设做成独立的注册项，而不是一个 `preset` 参数？
/// - 插件 id 是工程里唯一的「音色身份」，界面的音色选择器按 id 分组 / 搜索；
/// - 预设立即用（挂上就有合适的声音），无需再写一遍参数；
/// - 用户改过的参数照常保存在 `PluginInstance::params` 里，覆盖预设默认值。
///
/// 前端的「出厂预设」列表（`data/plugins.ts`）必须与这里的数值保持一致。
pub const BUILTIN_PRESETS: &[Preset] = &[
    Preset { id: "builtin.synth", name: "内置合成器", patch: &[] },
    Preset {
        id: "builtin.synth.lead",
        name: "主音 Lead",
        patch: &[
            ("gain", 0.75),
            ("wave", 0.0),
            ("attack", 0.005),
            ("release", 0.25),
            ("cutoff", 0.80),
            ("resonance", 0.22),
            ("filterEnv", 0.45),
            ("decay", 0.35),
            ("sustain", 0.70),
            ("lfoRate", 0.80),
            ("lfoDelay", 0.35),
            ("lfoPitch", 0.10),
        ],
    },
    Preset {
        id: "builtin.synth.pluck",
        name: "拨弦 Pluck",
        patch: &[
            ("gain", 0.80),
            ("wave", 1.0),
            ("attack", 0.001),
            ("release", 0.08),
            ("cutoff", 0.55),
            ("filterEnv", 0.70),
            ("decay", 0.12),
            ("sustain", 0.0),
        ],
    },
    Preset {
        id: "builtin.synth.bass",
        name: "低音 Bass",
        patch: &[
            ("gain", 0.85),
            ("wave", 0.0),
            ("attack", 0.002),
            ("release", 0.12),
            ("sub", 0.60),
            ("cutoff", 0.35),
            ("resonance", 0.25),
            ("filterEnv", 0.50),
            ("decay", 0.25),
            ("sustain", 0.50),
        ],
    },
    Preset {
        id: "builtin.synth.pad",
        name: "铺底 Pad",
        patch: &[
            ("gain", 0.55),
            ("wave", 0.0),
            ("attack", 0.35),
            ("release", 1.20),
            ("cutoff", 0.50),
            ("filterEnv", 0.30),
            ("decay", 1.0),
            ("sustain", 0.80),
            ("lfoRate", 0.60),
            ("lfoFilter", 0.15),
        ],
    },
];

/// 一条内置预设的定义。
#[derive(Debug, Clone, Copy)]
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    /// 参数补丁（参数 id → 值）；没提到的参数取合成器默认值。
    pub patch: &'static [(&'static str, f32)],
}

/// 插件工厂：按「轨道」创建实例。
///
/// 之所以把轨道 id 交给工厂：宿主侧需要在实例创建时把它登记到「轨道 → 插件句柄」的表里，
/// 这样界面才能为**正在发声的那个实例**打开原生编辑器（编辑器必须附着在同一个实例上）。
/// 内置插件用不到它，忽略即可。
type Factory = Box<dyn Fn(TrackId) -> Box<dyn Plugin> + Send + Sync>;

/// 未安装插件的占位：静音、惰性。
///
/// 工程里引用了某个插件、但这台机器上没装（或扫描不到）时，用它顶上——这样工程仍能打开，
/// 其余轨道照常播放，而不是整首打不开。
struct MissingPlugin {
    id: String,
}

impl Plugin for MissingPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: self.id.clone(),
            name: format!("缺失插件（{}）", self.id),
            kind: PluginKind::Instrument,
            io: IoLayout { inputs: 0, outputs: 2 },
            capabilities: Default::default(),
        }
    }
    fn params(&self) -> Vec<ParamInfo> {
        Vec::new()
    }
    fn get_param(&self, _id: &ParamId) -> Option<f32> {
        None
    }
    fn set_param(&mut self, _id: &ParamId, _value: f32) {}
    fn prepare(&mut self, _sample_rate: f64, _block_size: usize) {}
    fn reset(&mut self) {}
    fn process(&mut self, _ctx: &ProcessContext, audio: &mut [&mut [f32]], _events: &[TimedMidiEvent]) {
        for channel in audio.iter_mut() {
            channel.fill(0.0);
        }
    }
    fn save_state(&self) -> Vec<u8> {
        Vec::new()
    }
    fn load_state(&mut self, _state: &[u8]) -> Result<(), PluginError> {
        Ok(())
    }
}

/// 构造一个「缺失插件」静音占位实例。
///
/// 外部插件工厂在加载失败时返回它，而不是 panic——一条坏轨不该拖垮整个工程。
pub fn missing_plugin(id: &str) -> Box<dyn Plugin> {
    Box::new(MissingPlugin { id: id.to_string() })
}

/// 插件注册表：按 id 创建插件实例。
#[derive(Default)]
pub struct PluginRegistry {
    factories: HashMap<String, Factory>,
}

impl PluginRegistry {
    /// 创建一个空注册表。
    pub fn new() -> Self {
        PluginRegistry { factories: HashMap::new() }
    }

    /// 内置插件：合成器本体 + 全部预设音色。
    pub fn with_builtins() -> Self {
        let mut r = PluginRegistry::new();
        for p in BUILTIN_PRESETS {
            r.register(p.id, move |_track| {
                Box::new(daw_party_synth::SynthPlugin::preset(p.id, p.name, p.patch))
            });
        }
        r
    }

    /// 注册一个插件工厂，同 id 覆盖旧值。工厂会收到「这条轨道的 id」。
    pub fn register(
        &mut self,
        id: &str,
        factory: impl Fn(TrackId) -> Box<dyn Plugin> + Send + Sync + 'static,
    ) {
        self.factories.insert(id.to_string(), Box::new(factory));
    }

    /// 为「本机未安装的插件」注册静音占位（同 id 覆盖旧值）。
    ///
    /// 工程引用了某插件、但当前机器扫描不到时应当调用它：否则该轨会被静默跳过，
    /// 用户也分不清是「插件没装」还是「工程坏了」。
    pub fn register_missing(&mut self, id: &str) {
        let stored = id.to_string();
        self.register(id, move |_track| missing_plugin(&stored));
    }

    /// 按 id + 轨道创建插件实例；未知 id 返回 `None`。
    pub fn create(&self, id: &str, track: TrackId) -> Option<Box<dyn Plugin>> {
        self.factories.get(id).map(|f| f(track))
    }

    /// 已注册的全部插件 id（升序排列）。
    pub fn ids(&self) -> Vec<String> {
        let mut v: Vec<String> = self.factories.keys().cloned().collect();
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    // `Plugin` 已由上方模块级 `use daw_party_plugin_api::Plugin;` 经 `super::*` 引入。
    use super::*;

    #[test]
    fn builtins_include_synth() {
        let r = PluginRegistry::with_builtins();
        assert!(r.ids().contains(&"builtin.synth".to_string()));
    }

    #[test]
    fn builtins_include_every_preset() {
        let r = PluginRegistry::with_builtins();
        let ids = r.ids();
        for p in BUILTIN_PRESETS {
            assert!(ids.contains(&p.id.to_string()), "预设 {} 未注册", p.id);
        }
    }

    #[test]
    fn preset_instances_carry_their_own_identity_and_defaults() {
        let r = PluginRegistry::with_builtins();
        let p = r.create("builtin.synth.pad", TrackId(1)).expect("pad preset exists");
        assert_eq!(p.descriptor().id, "builtin.synth.pad");
        assert_eq!(p.get_param(&"attack".into()), Some(0.35));
    }

    /// 默认改成正弦后，老预设必须仍保持原来的波形，否则听感会变。
    #[test]
    fn factory_presets_pin_their_waveform() {
        let reg = PluginRegistry::with_builtins();
        let wave_of = |id: &str| -> Option<f32> {
            let p = reg.create(id, crate::model::TrackId(0))?;
            p.get_param(&ParamId::from("wave"))
        };
        assert_eq!(wave_of("builtin.synth"), Some(2.0), "默认音色 = 正弦");
        assert_eq!(wave_of("builtin.synth.lead"), Some(0.0));
        assert_eq!(wave_of("builtin.synth.bass"), Some(0.0));
        assert_eq!(wave_of("builtin.synth.pad"), Some(0.0));
        assert_eq!(wave_of("builtin.synth.pluck"), Some(1.0));
    }

    #[test]
    fn create_returns_configured_instance() {
        let r = PluginRegistry::with_builtins();
        let p = r.create("builtin.synth", TrackId(1)).expect("synth exists");
        assert_eq!(p.descriptor().id, "builtin.synth");
        assert_eq!(p.get_param(&"gain".into()), Some(0.8));
    }

    #[test]
    fn create_unknown_returns_none() {
        let r = PluginRegistry::with_builtins();
        assert!(r.create("nope", TrackId(1)).is_none());
    }

    #[test]
    fn missing_placeholder_is_silent_and_keeps_its_id() {
        let mut r = PluginRegistry::new();
        r.register_missing("vst3:deadbeef");
        let mut p = r.create("vst3:deadbeef", TrackId(1)).expect("占位已注册");
        assert_eq!(p.descriptor().id, "vst3:deadbeef");

        let ctx = ProcessContext { sample_rate: 48_000.0, block_size: 4, tempo: 120.0, playing: true };
        let mut left = [1.0f32; 4];
        let mut right = [1.0f32; 4];
        p.process(&ctx, &mut [&mut left, &mut right], &[]);
        assert!(left.iter().all(|s| *s == 0.0) && right.iter().all(|s| *s == 0.0));
    }
}
