use std::collections::HashMap;

use ai_daw_plugin_api::{
    IoLayout, ParamId, ParamInfo, Plugin, PluginDescriptor, PluginError, PluginKind,
    ProcessContext, TimedMidiEvent,
};

use crate::model::TrackId;

/// 内置合成器的插件 id（`PluginRegistry::with_builtins` 注册的就是它）。
///
/// 抽成常量是因为「新建工程的默认分轨」要挂乐器，而引擎不该散落魔法字符串。
pub const BUILTIN_SYNTH: &str = "builtin.synth";

/// 内置延迟的插件 id。
pub const BUILTIN_DELAY: &str = ai_daw_effects::DELAY_ID;
/// 内置混响的插件 id。
pub const BUILTIN_REVERB: &str = ai_daw_effects::REVERB_ID;
/// 内置合唱的插件 id。
pub const BUILTIN_CHORUS: &str = ai_daw_effects::CHORUS_ID;
/// 内置过载/失真的插件 id。
pub const BUILTIN_DRIVE: &str = ai_daw_effects::DRIVE_ID;
/// 内置两段 EQ 的插件 id。
pub const BUILTIN_EQ: &str = ai_daw_effects::EQ_ID;

/// 内置合成器的**出厂预设**：同一套信号链，不同的参数补丁。
///
/// 为什么把预设做成独立的注册项，而不是一个 `preset` 参数？
/// - 插件 id 是工程里唯一的「音色身份」，界面的音色选择器按 id 分组 / 搜索；
/// - 预设立即用（挂上就有合适的声音），无需再写一遍参数；
/// - 用户改过的参数照常保存在 `PluginInstance::params` 里，覆盖预设默认值。
///
/// **这里只放「默认分轨模板」需要的那 5 条**（见 [`crate::STARTER_TRACKS`]），每类音色的第 1 条。
/// 其余音色只是界面层的参数补丁（`data/plugins.ts`），不占注册表——它们挂在同一条内置合成器上，
/// 轨道只在 `params` 里记差异。
///
/// 这 5 条**故意都是干声**：注册表补丁只能带参数、带不了效果链，而前端的同名条目必须与它一致
/// （否则「默认模板的听感」和「点一下那个垫子的听感」会对不上）。带混响/延迟的湿版本是各类的第 2..4 条。
///
/// 前端的「出厂预设」列表（`data/plugins.ts`）里这 5 条的 id 与数值必须与这里完全一致。
pub const BUILTIN_PRESETS: &[Preset] = &[
    Preset { id: "builtin.synth", name: "内置合成器", patch: &[] },
    Preset {
        id: "builtin.synth.lead",
        name: "8-bit 主音",
        patch: &[
            // NES 2A03 脉冲通道：25% 占空比、无滤波、延迟颤音。数值与
            // 前端 `SYNTH_FACTORY_PRESETS` 里的同名条目**必须一致**。
            ("gain", 0.45),
            ("wave", 1.0),
            ("pwm", 0.222),
            ("attack", 0.002),
            ("decay", 0.25),
            ("sustain", 0.6),
            ("release", 0.08),
            ("cutoff", 1.0),
            ("resonance", 0.0),
            ("filterEnv", 0.0),
            ("keyFollow", 0.0),
            ("lfoRate", 0.8),
            ("lfoDelay", 0.35),
            ("lfoPitch", 0.16),
            ("hpf", 0.15),
        ],
    },
    Preset {
        id: "builtin.synth.bass",
        name: "低音 Bass",
        patch: &[
            ("gain", 0.75),
            ("wave", 0.0),
            ("sub", 0.35),
            ("attack", 0.005),
            ("decay", 0.5),
            ("sustain", 0.4),
            ("release", 0.12),
            ("cutoff", 0.45),
            ("resonance", 0.15),
            ("filterEnv", 0.35),
            ("keyFollow", 0.3),
            ("noise", 0.08),
        ],
    },
    Preset {
        id: "builtin.synth.drums",
        name: "底鼓 Kick",
        patch: &[
            // 808 式底鼓：音高约 8.4 半音的下扫（约 31 ms）+ 约 350 ms 的幅度衰减。
            ("gain", 0.85),
            ("wave", 2.0),
            ("attack", 0.0),
            ("decay", 0.35),
            ("sustain", 0.0),
            ("release", 0.05),
            ("cutoff", 1.0),
            ("resonance", 0.0),
            ("filterEnv", 0.0),
            ("keyFollow", 0.0),
            ("pitchEnv", 0.35),
            ("pitchDecay", 0.3),
        ],
    },
    Preset {
        id: "builtin.synth.strings",
        name: "弦乐 Strings",
        patch: &[
            ("gain", 0.55),
            ("wave", 0.0),
            ("detune", 0.35),
            ("attack", 0.18),
            ("decay", 1.5),
            ("sustain", 0.85),
            ("release", 0.5),
            ("cutoff", 0.72),
            ("resonance", 0.08),
            ("filterEnv", 0.1),
            ("keyFollow", 0.3),
        ],
    },
    Preset {
        id: "builtin.synth.pad",
        name: "铺底 Pad",
        patch: &[
            ("gain", 0.55),
            ("wave", 0.0),
            ("detune", 0.4),
            ("attack", 0.35),
            ("decay", 2.0),
            ("sustain", 0.8),
            ("release", 0.9),
            ("cutoff", 0.6),
            ("resonance", 0.1),
            ("filterEnv", 0.12),
            ("keyFollow", 0.2),
            ("lfoRate", 0.25),
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

/// 一条轨道上插件实例所在的位置：乐器位，还是效果链的第几个槽位。
///
/// 宿主侧据此把「正在发声的那个实例」登记进编辑器句柄表，界面才能为**指定槽位**打开原生
/// 编辑器——一轨只有一个乐器、但可以有多个效果器，只按轨道寻址会互相顶掉。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluginSlot {
    /// 轨道乐器。
    Instrument,
    /// 效果链中下标为 `n` 的效果器。
    Effect(usize),
}

/// 插件工厂：按「轨道 + 槽位」创建实例。
///
/// 之所以把轨道 id 与槽位交给工厂：宿主侧需要在实例创建时把它登记到「槽位 → 插件句柄」的
/// 表里，这样界面才能为**正在发声的那个实例**打开原生编辑器（编辑器必须附着在同一个实例上）。
/// 内置插件用不到它，忽略即可。
type Factory = Box<dyn Fn(TrackId, PluginSlot) -> Box<dyn Plugin> + Send + Sync>;

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
    /// 缺失插件是占位，不是真插件——重建播放器时不参与复用（见 trait 说明）。
    fn is_placeholder(&self) -> bool {
        true
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

    /// 内置插件：合成器本体 + 全部预设音色 + 内置效果器。
    pub fn with_builtins() -> Self {
        let mut r = PluginRegistry::new();
        for p in BUILTIN_PRESETS {
            r.register(p.id, move |_track, _slot| {
                Box::new(ai_daw_synth::SynthPlugin::preset(p.id, p.name, p.patch))
            });
        }
        // 采样内置乐器：钢琴 / 木吉他 / 电吉他 / 贝斯 / 原声鼓组。
        // 资产编译期内嵌；同类乐器的 SoundFont 在 crate 内共享，按轨道实例化合成器。
        for kind in ai_daw_instruments::Kind::ALL {
            r.register(kind.id(), move |_track, _slot| ai_daw_instruments::SoundFontPlugin::create(kind));
        }
        // 内置效果器：以前空间感只能靠外部 VST3，铺底/弦乐/管乐只能干着听。
        r.register(BUILTIN_DELAY, |_track, _slot| Box::new(ai_daw_effects::Delay::new()));
        r.register(BUILTIN_REVERB, |_track, _slot| Box::new(ai_daw_effects::Reverb::new()));
        r.register(BUILTIN_CHORUS, |_track, _slot| Box::new(ai_daw_effects::Chorus::new()));
        r.register(BUILTIN_DRIVE, |_track, _slot| Box::new(ai_daw_effects::Drive::new()));
        r.register(BUILTIN_EQ, |_track, _slot| Box::new(ai_daw_effects::Eq::new()));
        r
    }

    /// 注册一个插件工厂，同 id 覆盖旧值。工厂会收到「这条轨道的 id」与「槽位」。
    pub fn register(
        &mut self,
        id: &str,
        factory: impl Fn(TrackId, PluginSlot) -> Box<dyn Plugin> + Send + Sync + 'static,
    ) {
        self.factories.insert(id.to_string(), Box::new(factory));
    }

    /// 为「本机未安装的插件」注册静音占位（同 id 覆盖旧值）。
    ///
    /// 工程引用了某插件、但当前机器扫描不到时应当调用它：否则该轨会被静默跳过，
    /// 用户也分不清是「插件没装」还是「工程坏了」。
    pub fn register_missing(&mut self, id: &str) {
        let stored = id.to_string();
        self.register(id, move |_track, _slot| missing_plugin(&stored));
    }

    /// 按 id + 轨道 + 槽位创建插件实例；未知 id 返回 `None`。
    pub fn create(&self, id: &str, track: TrackId, slot: PluginSlot) -> Option<Box<dyn Plugin>> {
        self.factories.get(id).map(|f| f(track, slot))
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
    // `Plugin` 已由上方模块级 `use ai_daw_plugin_api::Plugin;` 经 `super::*` 引入。
    use super::*;

    #[test]
    fn builtins_include_synth() {
        let r = PluginRegistry::with_builtins();
        assert!(r.ids().contains(&"builtin.synth".to_string()));
    }

    #[test]
    fn builtins_include_the_sampled_instruments() {
        let r = PluginRegistry::with_builtins();
        for kind in ai_daw_instruments::Kind::ALL {
            let p = r
                .create(kind.id(), TrackId(1), PluginSlot::Instrument)
                .unwrap_or_else(|| panic!("{} 未注册", kind.id()));
            assert_eq!(p.descriptor().kind, PluginKind::Instrument);
            assert_eq!(p.descriptor().io.outputs, 2);
            assert!(!p.params().is_empty());
        }
    }

    /// 内置效果器必须在注册表里，而且是 `Effect` 类型（否则不会出现在效果器选择列表里）。
    #[test]
    fn builtins_include_the_effects() {
        let r = PluginRegistry::with_builtins();
        for id in [BUILTIN_DELAY, BUILTIN_REVERB, BUILTIN_CHORUS, BUILTIN_DRIVE, BUILTIN_EQ] {
            let p = r.create(id, TrackId(1), PluginSlot::Instrument).unwrap_or_else(|| panic!("{id} 未注册"));
            let d = p.descriptor();
            assert_eq!(d.kind, PluginKind::Effect, "{id} 应当是效果器");
            assert_eq!(d.io.inputs, 2, "{id} 需要立体声输入");
            assert!(!p.params().is_empty(), "{id} 应当有参数");
        }
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
        let p = r.create("builtin.synth.pad", TrackId(1), PluginSlot::Instrument).expect("pad preset exists");
        assert_eq!(p.descriptor().id, "builtin.synth.pad");
        // 断言「实例拿到的是**这条预设自己的补丁值**」：所以从补丁表里把值读出来比对，
        // 不把某个具体数字写死在这里——音色会调，这个测试不该跟着碎。
        let patched = BUILTIN_PRESETS
            .iter()
            .find(|preset| preset.id == "builtin.synth.pad")
            .and_then(|preset| {
                preset.patch.iter().find(|(id, _)| *id == "attack").map(|(_, value)| *value)
            })
            .expect("pad 补丁里应当有 attack");
        assert_eq!(p.get_param(&"attack".into()), Some(patched));
        assert_ne!(patched, 0.01, "补丁值等同于全局默认值时，上面的断言证明不了补丁被应用了");
    }

    /// 默认波形是正弦后，注册表里的预设必须**显式钉住自己的波形**，否则听感会跟着默认值漂移。
    #[test]
    fn factory_presets_pin_their_waveform() {
        let reg = PluginRegistry::with_builtins();
        let wave_of = |id: &str| -> Option<f32> {
            let p = reg.create(id, crate::model::TrackId(0), PluginSlot::Instrument)?;
            p.get_param(&ParamId::from("wave"))
        };
        assert_eq!(wave_of("builtin.synth"), Some(2.0), "默认音色 = 正弦");
        assert_eq!(wave_of("builtin.synth.lead"), Some(1.0), "8-bit 主音 = 方波");
        assert_eq!(wave_of("builtin.synth.bass"), Some(0.0));
        assert_eq!(wave_of("builtin.synth.pad"), Some(0.0));
        assert_eq!(wave_of("builtin.synth.strings"), Some(0.0));
        assert_eq!(wave_of("builtin.synth.drums"), Some(2.0), "底鼓 = 正弦");
    }

    /// 默认模板挂的每个 id 都必须真的注册过，否则那几条轨道会变成「缺失插件」占位。
    #[test]
    fn starter_tracks_point_at_registered_presets() {
        let r = PluginRegistry::with_builtins();
        let ids = r.ids();
        for track in crate::STARTER_TRACKS {
            assert!(
                ids.contains(&track.plugin_id.to_string()),
                "默认轨道 {} 挂了未注册的 {}",
                track.name,
                track.plugin_id
            );
        }
    }

    #[test]
    fn create_returns_configured_instance() {
        let r = PluginRegistry::with_builtins();
        let p = r.create("builtin.synth", TrackId(1), PluginSlot::Instrument).expect("synth exists");
        assert_eq!(p.descriptor().id, "builtin.synth");
        assert_eq!(p.get_param(&"gain".into()), Some(0.8));
    }

    #[test]
    fn create_unknown_returns_none() {
        let r = PluginRegistry::with_builtins();
        assert!(r.create("nope", TrackId(1), PluginSlot::Instrument).is_none());
    }

    #[test]
    fn missing_placeholder_is_silent_and_keeps_its_id() {
        let mut r = PluginRegistry::new();
        r.register_missing("vst3:deadbeef");
        let mut p = r.create("vst3:deadbeef", TrackId(1), PluginSlot::Instrument).expect("占位已注册");
        assert_eq!(p.descriptor().id, "vst3:deadbeef");
        assert!(p.is_placeholder(), "缺失插件必须自报为占位（重建时据此不复用）");

        let ctx = ProcessContext { sample_rate: 48_000.0, block_size: 4, tempo: 120.0, playing: true };
        let mut left = [1.0f32; 4];
        let mut right = [1.0f32; 4];
        p.process(&ctx, &mut [&mut left, &mut right], &[]);
        assert!(left.iter().all(|s| *s == 0.0) && right.iter().all(|s| *s == 0.0));
    }
}
