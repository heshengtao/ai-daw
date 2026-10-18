//! 插件目录的「人话」层：把 `id / name / vendor / kind` 翻译成**能读懂的语义**。
//!
//! 为什么需要这一层：扫描只能拿到名字。模型看到 `JC303`（厂商 midilab）时无从知道它是
//! TB-303 的克隆——**单音酸性贝斯**，于是会把它当铺底挂上、还写和弦，出来一片怪声。
//! 这里补三样扫描拿不到的东西：
//!
//! - `category`：用途分类（bass / drums / reverb / dynamics…），模型据此判断能干什么；
//! - `description`：一句话音色/效果描述（**具体型号写具体性格**，不是复读名字）；
//! - `roles` / `pitch_range` / `mono`：怎么用它编曲（分工、建议音域、能不能叠和弦）。
//!
//! 三级来源，越靠前越可信：
//!
//! 1. **内置插件表**（[`BUILTINS`]，id 固定、随代码发布）；
//! 2. **收录的第三方型号**（[`KNOWN`]，按名字片段匹配：`jc-303` → 酸性贝斯）；
//! 3. **兜底**：名字关键词 → VST3 `moduleinfo.json` 的 `Sub Categories` → 按乐器/效果给个
//!    诚实的「未收录」描述。**宁可说不知道，也不编造音色**。

use serde::Serialize;

use ai_daw_engine::registry::{
    BUILTIN_CHORUS, BUILTIN_DELAY, BUILTIN_DRIVE, BUILTIN_EQ, BUILTIN_REVERB, BUILTIN_SYNTH,
};
use ai_daw_host_vst3::CatalogPlugin;
use ai_daw_plugin_api::PluginKind;

/// 目录里的一条（内置插件 + 扫描到的外部插件，统一形状）。
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub kind: PluginKind,
    /// `builtin` / `vst3` / `clap` / `au`。
    pub format: String,
    pub builtin: bool,
    /// 外部插件的包路径（读 `moduleinfo.json` 用）；内置为 `None`。
    pub path: Option<std::path::PathBuf>,
}

/// 给模型看的插件卡片。
#[derive(Debug, Clone, Serialize)]
pub struct Brief {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub kind: PluginKind,
    pub format: String,
    pub builtin: bool,
    /// 用途分类 slug（见 [`category_label`]）。
    pub category: &'static str,
    /// 一句话描述：是什么、什么性格、怎么用。
    pub description: String,
    /// 适合的编曲分工（与 `set_track_meta` 的 `role` 同一套词）。
    pub roles: Vec<&'static str>,
    /// 建议写作音域（MIDI 音高，闭区间）；`None` = 没有明确建议。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pitch_range: Option<[u8; 2]>,
    /// 单音乐器：写音符时**不能**叠和弦。
    pub mono: bool,
    /// 搜索关键词（中英文都放，方便按习惯查）。
    pub tags: Vec<String>,
}

// ---------------------------------------------------------------------------
// 内置插件
// ---------------------------------------------------------------------------

/// 内置插件的一条（id 是引擎注册表里的真实 id）。
struct Builtin {
    id: &'static str,
    name: &'static str,
    kind: PluginKind,
    category: &'static str,
    description: &'static str,
    roles: &'static [&'static str],
    pitch_range: Option<[u8; 2]>,
    mono: bool,
    tags: &'static [&'static str],
}

/// 内置合成器 + 内置效果器 + 引擎里仍注册着的 5 条老预设音色。
///
/// 这 5 条预设 id（`builtin.synth.lead` 等）留着是因为老工程和默认分轨模板直接挂在它们上面；
/// 界面上的 32 条出厂音色不是独立插件，走 `set_param` / `apply_preset` 的 `params`。
const BUILTINS: &[Builtin] = &[
    Builtin {
        id: BUILTIN_SYNTH,
        name: "内置合成器",
        kind: PluginKind::Instrument,
        category: "synth",
        description: "内置复音减法合成器（16 复音）：4 种波形 + 低八度副振 + ADSR + 低通/共振/包络量 + LFO + FM + 噪声 + 位深压碎。\
出厂 32 条音色（8-bit / 电钢 / 吉他 / 贝斯 / 鼓 / 弦乐 / 管乐 / 铺底 / 音效）都跑在它上面，靠 set_param 或 apply_preset 的 params 调出——挂它就等于挂一台通用合成器。",
        roles: &["lead", "chords", "bass", "pad", "drums"],
        pitch_range: None,
        mono: false,
        tags: &["synth", "合成器", "内置", "减法", "8bit", "fm", "通用"],
    },
    Builtin {
        id: "builtin.synth.lead",
        name: "8-bit 主音（内置预设）",
        kind: PluginKind::Instrument,
        category: "lead",
        description: "内置合成器的 NES 主音预设：25% 占空比脉冲 + 延迟颤音，薄而亮，写单声部旋律。",
        roles: &["lead"],
        pitch_range: Some([60, 96]),
        mono: false,
        tags: &["8bit", "lead", "主音", "nes", "芯片"],
    },
    Builtin {
        id: "builtin.synth.bass",
        name: "低音 Bass（内置预设）",
        kind: PluginKind::Instrument,
        category: "bass",
        description: "内置合成器的指弹贝斯预设：低八度副振 + 约 900 Hz 的低通，写根音与八分音符 bass line。",
        roles: &["bass"],
        pitch_range: Some([28, 55]),
        mono: false,
        tags: &["bass", "贝斯", "低音", "预设"],
    },
    Builtin {
        id: "builtin.synth.drums",
        name: "底鼓 Kick（内置预设）",
        kind: PluginKind::Instrument,
        category: "drums",
        description: "内置合成器的 808 式底鼓预设：正弦 + 音高下扫（约 31 ms）+ 约 350 ms 衰减。固定写一个低音（36 附近）当四拍底鼓。",
        roles: &["drums"],
        pitch_range: Some([24, 48]),
        mono: false,
        tags: &["drums", "kick", "底鼓", "808"],
    },
    Builtin {
        id: "builtin.synth.strings",
        name: "弦乐 Strings（内置预设）",
        kind: PluginKind::Instrument,
        category: "strings",
        description: "内置合成器的弦乐合奏预设：多振荡器失谐 + 慢起音 + 长释音，适合铺和弦与长音。",
        roles: &["chords", "pad"],
        pitch_range: Some([36, 84]),
        mono: false,
        tags: &["strings", "弦乐", "合奏", "预设"],
    },
    Builtin {
        id: "builtin.synth.pad",
        name: "铺底 Pad（内置预设）",
        kind: PluginKind::Instrument,
        category: "pad",
        description: "内置合成器的暖垫预设：慢起慢落 + 大幅失谐 + 极慢的滤波起伏，铺和弦用。",
        roles: &["pad", "chords"],
        pitch_range: Some([36, 84]),
        mono: false,
        tags: &["pad", "铺底", "垫子", "预设"],
    },
    Builtin {
        id: BUILTIN_DELAY,
        name: "延迟",
        kind: PluginKind::Effect,
        category: "delay",
        description: "内置立体声延迟（时间 / 反馈 / 混合）。时间约 0.02–1 s：短到 45 ms 只把音变厚，长到 700 ms 是铺底的长尾巴。",
        roles: &[],
        pitch_range: None,
        mono: false,
        tags: &["delay", "echo", "延迟", "回声"],
    },
    Builtin {
        id: BUILTIN_REVERB,
        name: "混响",
        kind: PluginKind::Effect,
        category: "reverb",
        description: "内置施罗德混响（房间大小 / 阻尼 / 混合）。混响给的是「距离」：先定 size，再用 mix 收到听不出湿声的程度。",
        roles: &[],
        pitch_range: None,
        mono: false,
        tags: &["reverb", "hall", "room", "混响", "空间"],
    },
    Builtin {
        id: BUILTIN_CHORUS,
        name: "合唱",
        kind: PluginKind::Effect,
        category: "modulation",
        description: "内置立体声合唱（速率 / 深度 / 混合）：把单声道音色撑宽，浅一点就是「加厚」，深一点会听出音高晃动。",
        roles: &[],
        pitch_range: None,
        mono: false,
        tags: &["chorus", "modulation", "合唱", "加宽"],
    },
    Builtin {
        id: BUILTIN_DRIVE,
        name: "过载",
        kind: PluginKind::Effect,
        category: "distortion",
        description: "内置软削波过载（驱动 / 音色 / 混合）：给贝斯、吉他、鼓加谐波与「毛边」，也可当轻饱和用在总线上。",
        roles: &[],
        pitch_range: None,
        mono: false,
        tags: &["drive", "distortion", "overdrive", "过载", "失真", "饱和"],
    },
    Builtin {
        id: BUILTIN_EQ,
        name: "均衡",
        kind: PluginKind::Effect,
        category: "eq",
        description: "内置两段 EQ（低频 / 高频，可正可负，混合 100% 时最干净）：修音色重心，电话音（砍低频）也靠它。",
        roles: &[],
        pitch_range: None,
        mono: false,
        tags: &["eq", "tone", "均衡", "低频", "高频"],
    },
];

// ---------------------------------------------------------------------------
// 收录的第三方型号
// ---------------------------------------------------------------------------

/// 一条「认得出名字」的第三方插件知识。
struct Known {
    /// 归一化后的名字片段（小写、只留字母数字）。命中任意一个即算，取最长的那条。
    patterns: &'static [&'static str],
    category: &'static str,
    description: &'static str,
    roles: &'static [&'static str],
    pitch_range: Option<[u8; 2]>,
    mono: bool,
    tags: &'static [&'static str],
}

/// 收录清单：只写**有把握**的型号。
///
/// 判断标准是「这个名字对应一件明确的硬件/软件，且它的性格是公认的」——比如 JC303 是
/// TB-303 克隆（单音酸性贝斯），KR-106 是 Juno-106 克隆（复音、80 年代）。认不出的走兜底，
/// 不在这里瞎猜，否则模型会拿到一条看起来很像真的、其实是编的描述。
const KNOWN: &[Known] = &[
    // ---- 贝斯 ----
    Known {
        patterns: &["jc303", "tb303", "abl3", "phoscyon", "acidbox", "bassbot", "ransom"],
        category: "bass",
        description: "TB-303 克隆：**单音酸性贝斯**合成器（锯齿/方波 → 24 dB 低通 + 包络量 + 共振，accent / slide 做重音与滑音，另带过载）。写单音 bass line，不要叠和弦；典型音域 E1–E4。",
        roles: &["bass"],
        pitch_range: Some([28, 64]),
        mono: true,
        tags: &["acid", "303", "tb-303", "bass", "贝斯", "单音", "mono", "house", "techno"],
    },
    Known {
        patterns: &["trilian", "amplebass", "modobass", "subboombass", "bassroom", "bassynth"],
        category: "bass",
        description: "贝斯音源/合成器：低音区专用，写根音与律动。多数是采样或多层合成，音域压在 E1–G3 最结实。",
        roles: &["bass"],
        pitch_range: Some([28, 55]),
        mono: false,
        tags: &["bass", "贝斯", "低音"],
    },
    // ---- 复音合成器 ----
    Known {
        patterns: &["kr106", "juno106", "juno60", "talunolx", "taluno", "ju106", "juno"],
        category: "synth",
        description: "Roland Juno-106 风格的复音模拟合成器（DCO 锯齿/方波/脉冲 + 副振荡 + 合唱）。80 年代那种「一片」的 pad、弦垫、贝斯与主音都靠它，音色自带复古味。",
        roles: &["chords", "pad", "bass", "lead"],
        pitch_range: Some([36, 84]),
        mono: false,
        tags: &["juno", "analog", "poly", "80s", "复古", "合成器"],
    },
    Known {
        patterns: &["dx7", "dexed", "fm8", "operator"],
        category: "synth",
        description: "DX7 风格的 FM 合成器：电钢、金属 bell、FM 贝斯都是它的招牌；音色随预设变化极大，先看参数表再动手。",
        roles: &["keys", "lead", "bass"],
        pitch_range: Some([36, 96]),
        mono: false,
        tags: &["fm", "dx7", "合成器", "电钢", "bell"],
    },
    Known {
        patterns: &[
            "serum", "vital", "massive", "sylenth", "surge", "pigments", "phaseplant", "diva",
            "repro", "obxd", "hive", "spire", "avenger", "synthmaster", "prophet",
        ],
        category: "synth",
        description: "现代复音合成器（波表 / 减法 / 模拟建模）：音色完全由预设决定，适合 pad、lead、和弦与音效；先看参数表判断能调什么。",
        roles: &["chords", "lead", "pad", "bass"],
        pitch_range: Some([36, 96]),
        mono: false,
        tags: &["synth", "合成器", "wavetable", "analog", "pad", "lead"],
    },
    // ---- 键盘 / 钢琴 ----
    Known {
        patterns: &["rdpiano", "rhodes", "wurli", "epiano", "electricpiano", "lounge"],
        category: "epiano",
        description: "Rhodes / 电钢琴：柔和、带琴槌噪声与长衰减。抒情的和弦 comping、副歌铺底都合适；配一点合唱就是 80 年代情歌。",
        roles: &["chords", "lead"],
        pitch_range: Some([28, 96]),
        mono: false,
        tags: &["epiano", "rhodes", "电钢", "键盘", "keys"],
    },
    Known {
        patterns: &["pianoteq", "keyscape", "addictivekeys", "truepianos", "grand", "upright"],
        category: "piano",
        description: "原声钢琴音源：写和弦与分解和弦。注意钢琴左手本来就会占低音区，别再叠一条贝斯在同一个八度。",
        roles: &["chords", "lead"],
        pitch_range: Some([21, 96]),
        mono: false,
        tags: &["piano", "钢琴", "keys", "原声"],
    },
    Known {
        patterns: &["organ", "hammond", "b3", "drawbar"],
        category: "organ",
        description: "风琴：持续音不衰减，靠旋转喇叭（Leslie）与和声推子做表情；适合福音/爵士的铺底与和弦。",
        roles: &["chords", "pad"],
        pitch_range: Some([36, 84]),
        mono: false,
        tags: &["organ", "风琴", "hammond", "keys"],
    },
    // ---- 鼓 ----
    Known {
        patterns: &["jdrummer", "drummachine", "drumkit", "battery", "ezdrummer", "superiordrummer", "addictivedrums", "ssd", "kick", "808", "909", "707"],
        category: "drums",
        description: "鼓机 / 鼓组：按 GM 鼓键位写音符（36 底鼓、38 军鼓、42 闭镲、46 开镲…），一条轨写完整个节奏组；具体映射以插件参数/说明为准。",
        roles: &["drums"],
        pitch_range: Some([24, 60]),
        mono: false,
        tags: &["drums", "鼓", "鼓机", "kick", "snare", "808", "909", "gm"],
    },
    // ---- 弦乐 / 管乐 ----
    Known {
        patterns: &["spitfire", "bbcso", "sessionstrings", "cinestrings", "strings", "violin", "cello"],
        category: "strings",
        description: "弦乐音源：长音铺和弦、短音做节奏（pizz/spiccato）。弦乐吃音域——低音交给大提/贝斯，高音留给小提，别把和弦写得又低又厚。",
        roles: &["chords", "pad"],
        pitch_range: Some([36, 88]),
        mono: false,
        tags: &["strings", "弦乐", "violin", "cello", "orchestra"],
    },
    Known {
        patterns: &["brass", "trumpet", "trombone", "horn", "sax", "flute", "clarinet", "swam"],
        category: "winds",
        description: "管乐音源：单声部为主（一支乐器一条线），靠起音与滑音做语气；写和声要分层或换多支乐器。",
        roles: &["lead", "chords"],
        pitch_range: Some([36, 88]),
        mono: false,
        tags: &["winds", "brass", "管乐", "铜管", "sax", "flute"],
    },
    Known {
        patterns: &["choir", "vocal", "vox", "vocalsynth"],
        category: "vocal",
        description: "人声音源/人声处理：合唱音色铺底，或把人声当乐器用。人声频段很挤，注意别和主音抢 1–4 kHz。",
        roles: &["pad", "lead"],
        pitch_range: Some([36, 84]),
        mono: false,
        tags: &["vocal", "choir", "人声", "合唱"],
    },
    // ---- 采样器 ----
    Known {
        patterns: &["kontakt", "halion", "sampletank", "sforzando", "decentsampler", "tx16wx", "omnisphere", "sampler"],
        category: "sampler",
        description: "采样器/音色库宿主：本身没有固定性格，装什么库就是什么音色——先看它当前加载的库（或参数表）再决定怎么用。",
        roles: &["chords", "lead", "bass"],
        pitch_range: None,
        mono: false,
        tags: &["sampler", "采样器", "音色库"],
    },
    // ---- 效果：空间 ----
    Known {
        patterns: &["valhalla", "vintageverb", "supermassive", "shimmer", "lexicon", "altiverb", "reverb", "verb"],
        category: "reverb",
        description: "混响：给音色加空间与距离。先定 decay/size（房间大小），再把 mix 收到「听得出空间但听不出湿声」的程度；全曲只用一个主混响更容易听成一体。",
        roles: &[],
        pitch_range: None,
        mono: false,
        tags: &["reverb", "混响", "空间", "hall", "plate"],
    },
    Known {
        patterns: &["ambirr", "ambison", "binaural", "spatial", "surround", "imager", "width"],
        category: "spatial",
        description: "空间/声场类效果：处理宽度、环绕或双耳定位。别在单声道兼容性上翻车——加宽之后要检查单声道折叠是否还成立。",
        roles: &[],
        pitch_range: None,
        mono: false,
        tags: &["spatial", "空间", "ambisonic", "宽度", "stereo"],
    },
    Known {
        patterns: &["delay", "echo", "spaceecho", "re201", "pingpong", "timeless"],
        category: "delay",
        description: "延迟/回声：短延迟（30–60 ms）加厚，长延迟（1/4、1/8 拍）做节奏呼应。混音里延迟通常比混响更「干净」，因为它不糊。",
        roles: &[],
        pitch_range: None,
        mono: false,
        tags: &["delay", "echo", "延迟", "回声"],
    },
    // ---- 效果：动态 / 均衡 ----
    Known {
        patterns: &["proq", "proq3", "equalizer", "eq", "filterbank"],
        category: "eq",
        description: "均衡器：修音色重心与频段冲突（切比推更安全）。给贝斯切掉 200 Hz 以下的浑、给铺底挖掉中频，比一味加高频有效。",
        roles: &[],
        pitch_range: None,
        mono: false,
        tags: &["eq", "均衡", "filter"],
    },
    Known {
        patterns: &["proc", "prol", "compressor", "comp", "limiter", "maximizer", "glue", "ott", "multiband", "gate"],
        category: "dynamics",
        description: "动态处理（压缩 / 限幅 / 门）：压的是「音量差」不是「音量」。轻压 2–4 dB 让人声/贝斯稳，重压会听出「抽气」；限幅器留给总输出。",
        roles: &[],
        pitch_range: None,
        mono: false,
        tags: &["dynamics", "comp", "limiter", "压缩", "限幅", "动态"],
    },
    // ---- 效果：染色 ----
    Known {
        patterns: &["decapitator", "soundtoys", "saturator", "radiator", "distortion", "distort", "fuzz", "overdrive", "drive", "crush", "bitcrush", "tape"],
        category: "distortion",
        description: "失真/饱和：加谐波与毛边。贝斯与鼓上少量就很有效；失真会让高频变多，记得后面跟一个 EQ 收一下。",
        roles: &[],
        pitch_range: None,
        mono: false,
        tags: &["distortion", "drive", "失真", "饱和", "fuzz"],
    },
    Known {
        patterns: &["amplitube", "guitarrig", "neuraldsp", "archetype", "biasfx", "tonex", "amp", "cab"],
        category: "amp",
        description: "音箱/箱体模拟：吉他与贝斯的「音箱味」，通常自带箱体与麦克风。挂上之后不要再叠别的失真，容易糊。",
        roles: &[],
        pitch_range: None,
        mono: false,
        tags: &["amp", "cab", "音箱", "吉他"],
    },
    // ---- 效果：调制 / 音高 ----
    Known {
        patterns: &["chorus", "ensemble", "dimension", "flanger", "phaser", "tremolo", "vibrato", "rotary"],
        category: "modulation",
        description: "调制类效果（合唱 / 镶边 / 相位 / 颤音）：给音色加运动感与宽度。速率慢=铺底，速率快=明显晃动；单声道兼容性要留意。",
        roles: &[],
        pitch_range: None,
        mono: false,
        tags: &["modulation", "chorus", "flanger", "phaser", "调制"],
    },
    Known {
        patterns: &["pitch", "harmon", "autotune", "melodyne", "octav", "vocoder"],
        category: "pitch",
        description: "音高类效果：移调、和声、修音高。移调会同时改变共振峰（音色变「鸭」），人声上要谨慎。",
        roles: &[],
        pitch_range: None,
        mono: false,
        tags: &["pitch", "音高", "和声", "autotune"],
    },
];

// ---------------------------------------------------------------------------
// 分类与角色
// ---------------------------------------------------------------------------

/// 分类 slug → 中文标签（写进描述、给模型一个中文锚点）。
pub fn category_label(category: &str) -> &'static str {
    match category {
        "bass" => "贝斯/低音",
        "lead" => "主音",
        "synth" => "合成器",
        "keys" => "键盘",
        "epiano" => "电钢琴",
        "piano" => "原声钢琴",
        "organ" => "风琴",
        "drums" => "鼓/打击乐",
        "guitar" => "吉他",
        "strings" => "弦乐",
        "winds" => "管乐",
        "brass" => "铜管",
        "vocal" => "人声",
        "sampler" => "采样器",
        "pad" => "铺底",
        "fx" => "音效",
        "reverb" => "混响",
        "delay" => "延迟",
        "modulation" => "调制",
        "dynamics" => "动态处理",
        "eq" => "均衡",
        "distortion" => "失真/饱和",
        "amp" => "音箱模拟",
        "filter" => "滤波",
        "spatial" => "空间/声场",
        "pitch" => "音高",
        "utility" => "工具",
        _ => "其他",
    }
}

/// 分类 → 推荐分工（与 `set_track_meta` 的 `role` 同一套词）。效果器没有分工。
fn roles_for(category: &str) -> Vec<&'static str> {
    match category {
        "bass" => vec!["bass"],
        "drums" => vec!["drums"],
        "pad" => vec!["pad", "chords"],
        "lead" => vec!["lead"],
        "epiano" | "piano" | "organ" | "keys" => vec!["chords", "lead"],
        "strings" | "winds" | "brass" | "vocal" => vec!["chords", "pad", "lead"],
        "synth" | "sampler" => vec!["chords", "lead", "bass", "pad"],
        "fx" => vec!["fx"],
        _ => vec![],
    }
}

/// 名字里的关键词 → 分类。**顺序即优先级**：越具体的越靠前（`space echo` 是延迟不是混响）。
///
/// 只看名字，不看厂商：厂商名里常有 `Audio`、`Labs` 这类噪声词，拿它判断更容易错。
fn keyword_category(norm_name: &str, kind: PluginKind) -> Option<&'static str> {
    const INSTRUMENT_RULES: &[(&str, &str)] = &[
        ("303", "bass"),
        ("bassline", "bass"),
        ("bass", "bass"),
        ("subbass", "bass"),
        ("drum", "drums"),
        ("perc", "drums"),
        ("beat", "drums"),
        ("808", "drums"),
        ("909", "drums"),
        ("707", "drums"),
        ("kick", "drums"),
        ("snare", "drums"),
        ("rhodes", "epiano"),
        ("epiano", "epiano"),
        ("wurli", "epiano"),
        ("tine", "epiano"),
        ("piano", "piano"),
        ("organ", "organ"),
        ("hammond", "organ"),
        ("guitar", "guitar"),
        ("strat", "guitar"),
        ("violin", "strings"),
        ("cello", "strings"),
        ("viola", "strings"),
        ("string", "strings"),
        ("orchestr", "strings"),
        ("choir", "vocal"),
        ("vocal", "vocal"),
        ("voice", "vocal"),
        ("sax", "winds"),
        ("flute", "winds"),
        ("clarinet", "winds"),
        ("brass", "brass"),
        ("trumpet", "brass"),
        ("pad", "pad"),
        ("ambient", "pad"),
        ("atmos", "pad"),
        ("drone", "pad"),
        ("riser", "fx"),
        ("sweep", "fx"),
        ("impact", "fx"),
        ("boom", "fx"),
        ("blip", "fx"),
        ("glitch", "fx"),
        ("lead", "lead"),
        ("arp", "lead"),
        ("sampler", "sampler"),
        ("kontakt", "sampler"),
        ("sample", "sampler"),
        ("synth", "synth"),
        ("analog", "synth"),
        ("wavetable", "synth"),
        ("fm", "synth"),
    ];
    const EFFECT_RULES: &[(&str, &str)] = &[
        ("spaceecho", "delay"),
        ("delay", "delay"),
        ("echo", "delay"),
        ("pingpong", "delay"),
        ("reverb", "reverb"),
        ("verb", "reverb"),
        ("hall", "reverb"),
        ("plate", "reverb"),
        ("room", "reverb"),
        ("ambison", "spatial"),
        ("ambirr", "spatial"),
        ("binaural", "spatial"),
        ("spatial", "spatial"),
        ("imager", "spatial"),
        ("width", "spatial"),
        ("chorus", "modulation"),
        ("ensemble", "modulation"),
        ("flanger", "modulation"),
        ("phaser", "modulation"),
        ("tremolo", "modulation"),
        ("vibrato", "modulation"),
        ("rotary", "modulation"),
        ("compress", "dynamics"),
        ("comp", "dynamics"),
        ("limiter", "dynamics"),
        ("maximizer", "dynamics"),
        ("glue", "dynamics"),
        ("multiband", "dynamics"),
        ("gate", "dynamics"),
        ("equalizer", "eq"),
        ("eq", "eq"),
        ("filter", "filter"),
        ("distort", "distortion"),
        ("overdrive", "distortion"),
        ("fuzz", "distortion"),
        ("drive", "distortion"),
        ("satur", "distortion"),
        ("crush", "distortion"),
        ("amp", "amp"),
        ("cab", "amp"),
        ("pitch", "pitch"),
        ("harmon", "pitch"),
        ("autotune", "pitch"),
        ("vocoder", "pitch"),
        ("meter", "utility"),
        ("analyzer", "utility"),
        ("tuner", "utility"),
        ("gain", "utility"),
    ];
    let rules = match kind {
        PluginKind::Instrument => INSTRUMENT_RULES,
        PluginKind::Effect => EFFECT_RULES,
    };
    rules
        .iter()
        .find(|(needle, _)| norm_name.contains(needle))
        .map(|(_, category)| *category)
}

/// VST3 的 `Sub Categories` → 分类（认不出返回 `None`）。
fn subcategory_to_category(sub: &str) -> Option<&'static str> {
    match sub.trim().to_ascii_lowercase().as_str() {
        "instrument" | "synth" | "sampler" => Some("synth"),
        "drum" | "drums" => Some("drums"),
        "piano" => Some("piano"),
        "reverb" => Some("reverb"),
        "delay" => Some("delay"),
        "spatial" => Some("spatial"),
        "modulation" => Some("modulation"),
        "dynamics" => Some("dynamics"),
        "eq" => Some("eq"),
        "distortion" => Some("distortion"),
        "filter" => Some("filter"),
        "pitch shift" | "pitch" => Some("pitch"),
        "mastering" => Some("dynamics"),
        "tools" | "utility" => Some("utility"),
        "fx" => Some("fx"),
        _ => None,
    }
}

/// 读 VST3 包里的 `moduleinfo.json`，取出插件**自报**的 Sub Categories。
///
/// 为什么不用 `serde_json`：这几个文件的生成器会写**尾随逗号**（非法 JSON），严格解析必然失败。
/// 这里只做一件事——找到第一个 `"Sub Categories"` 数组，把里面的字符串捞出来，够用且不挑格式。
fn vst3_subcategories(path: &std::path::Path) -> Vec<String> {
    let file = path.join("Contents/Resources/moduleinfo.json");
    let Ok(text) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    let Some(at) = text.find("\"Sub Categories\"") else {
        return Vec::new();
    };
    let rest = &text[at..];
    let Some(open) = rest.find('[') else { return Vec::new() };
    let Some(close) = rest[open..].find(']').map(|i| i + open) else {
        return Vec::new();
    };
    let body = &rest[open + 1..close];
    body.split(',')
        .filter_map(|item| {
            let item = item.trim();
            let start = item.find('"')?;
            let end = item.rfind('"')?;
            (end > start).then(|| item[start + 1..end].to_string())
        })
        .collect()
}

/// 归一化：小写、只留 ASCII 字母数字（`JC-303` / `JC 303` / `jc303` 视为同一个名字）。
fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

// ---------------------------------------------------------------------------
// 目录装配与查询
// ---------------------------------------------------------------------------

/// 全部可用插件：内置（含 5 条老预设 id）+ 本机扫描到的外部插件。
pub fn entries(catalog: &[CatalogPlugin]) -> Vec<Entry> {
    let mut out: Vec<Entry> = BUILTINS
        .iter()
        .map(|b| Entry {
            id: b.id.to_string(),
            name: b.name.to_string(),
            vendor: "AI DAW".to_string(),
            kind: b.kind,
            format: "builtin".to_string(),
            builtin: true,
            path: None,
        })
        .collect();
    for item in catalog {
        // 扫描结果理论上不会与内置 id 撞车，撞了就以内置为准（内置的工厂一定可用）。
        if out.iter().any(|e| e.id == item.id) {
            continue;
        }
        out.push(Entry {
            id: item.id.clone(),
            name: item.name.clone(),
            vendor: item.vendor.clone(),
            kind: item.kind,
            format: item.format.clone(),
            builtin: false,
            path: Some(item.path.clone()),
        });
    }
    out
}

/// 把一条目录项翻译成给模型看的卡片。
pub fn brief(entry: &Entry) -> Brief {
    if entry.builtin {
        if let Some(b) = BUILTINS.iter().find(|b| b.id == entry.id) {
            return Brief {
                id: entry.id.clone(),
                name: entry.name.clone(),
                vendor: entry.vendor.clone(),
                kind: entry.kind,
                format: entry.format.clone(),
                builtin: true,
                category: b.category,
                description: b.description.to_string(),
                roles: b.roles.to_vec(),
                pitch_range: b.pitch_range,
                mono: b.mono,
                tags: b.tags.iter().map(|t| t.to_string()).collect(),
            };
        }
    }

    let norm = normalize(&entry.name);
    let known = best_known(&norm);
    if let Some(k) = known {
        return Brief {
            id: entry.id.clone(),
            name: entry.name.clone(),
            vendor: entry.vendor.clone(),
            kind: entry.kind,
            format: entry.format.clone(),
            builtin: false,
            category: k.category,
            description: k.description.to_string(),
            roles: k.roles.to_vec(),
            pitch_range: k.pitch_range,
            mono: k.mono,
            tags: k.tags.iter().map(|t| t.to_string()).collect(),
        };
    }

    let subs = entry
        .path
        .as_deref()
        .filter(|_| entry.format == "vst3")
        .map(vst3_subcategories)
        .unwrap_or_default();
    let from_sub = subs.iter().find_map(|s| subcategory_to_category(s));
    let category = keyword_category(&norm, entry.kind)
        .or(from_sub)
        .unwrap_or(match entry.kind {
            PluginKind::Instrument => "synth",
            PluginKind::Effect => "fx",
        });
    Brief {
        id: entry.id.clone(),
        name: entry.name.clone(),
        vendor: entry.vendor.clone(),
        kind: entry.kind,
        format: entry.format.clone(),
        builtin: false,
        category,
        description: unknown_description(entry, category, &subs),
        roles: roles_for(category),
        pitch_range: None,
        mono: false,
        tags: {
            let mut tags = vec![entry.format.clone(), entry.vendor.clone()];
            tags.extend(subs.iter().cloned());
            tags
        },
    }
}

/// 命中收录表里最长的那条（越长的名字片段越具体）。
fn best_known(norm_name: &str) -> Option<&'static Known> {
    let mut best: Option<(usize, &'static Known)> = None;
    for known in KNOWN {
        for pattern in known.patterns {
            if pattern.is_empty() || !norm_name.contains(pattern) {
                continue;
            }
            if best.map_or(true, |(len, _)| pattern.len() > len) {
                best = Some((pattern.len(), known));
            }
        }
    }
    best.map(|(_, known)| known)
}

/// 未收录型号的诚实描述：说清「是什么类别」+ 为什么没更多信息 + 下一步该干什么。
fn unknown_description(entry: &Entry, category: &str, subs: &[String]) -> String {
    let kind = match entry.kind {
        PluginKind::Instrument => "乐器",
        PluginKind::Effect => "效果器",
    };
    let mut text = format!(
        "外部 {kind}（{}，厂商 {}），未收录具体型号。按名称与插件自报信息判断属于「{}」。",
        entry.format.to_uppercase(),
        if entry.vendor.trim().is_empty() { "未知" } else { entry.vendor.as_str() },
        category_label(category),
    );
    if !subs.is_empty() {
        text.push_str(&format!("插件自报分类：{}。", subs.join(" / ")));
    }
    text.push_str("要判断实际音色/效果，用 describe_plugin 看参数表，或挂上后 analyze_project 确认听感——不要凭名字猜。");
    text
}

// ---------------------------------------------------------------------------
// 解析用户/模型给的名字
// ---------------------------------------------------------------------------

/// 把模型给的一串（id / 名字 / 名字片段）解析成目录里的一条。
///
/// 模型的常见错误是**直接拿名字当 id**（`set_track_instrument(plugin_id="JC303")`）。
/// 与其报「未知插件」，不如在这里认出来——认不出来时把现有名字列出来，让它自己改。
pub fn resolve(entries: &[Entry], query: &str) -> Result<Entry, String> {
    let raw = query.trim();
    if raw.is_empty() {
        return Err("插件 id / 名字不能为空".into());
    }
    // 1. 精确 id（`vst3:…` / `builtin.synth`）
    if let Some(hit) = entries.iter().find(|e| e.id == raw) {
        return Ok(hit.clone());
    }
    // 同一件东西可能装了两份（KR-106 同时有 VST3 与 CLAP）——这时必须让调用方选，别替它挑。
    let pick = |hits: Vec<&Entry>| -> Result<Option<Entry>, String> {
        match hits.len() {
            0 => Ok(None),
            1 => Ok(Some(hits[0].clone())),
            _ => {
                let ids: Vec<&str> = hits.iter().map(|e| e.id.as_str()).collect();
                Err(format!("「{raw}」匹配到多个插件：{}。请用完整 id。", ids.join(" / ")))
            }
        }
    };
    let lower = raw.to_lowercase();
    // 2. 名字精确（大小写不敏感）
    if let Some(hit) = pick(entries.iter().filter(|e| e.name.to_lowercase() == lower).collect())? {
        return Ok(hit);
    }
    // 3. 归一化相等：`jc-303` / `JC 303` / `jc303` 都算同一个
    let norm = normalize(raw);
    if !norm.is_empty() {
        if let Some(hit) =
            pick(entries.iter().filter(|e| normalize(&e.name) == norm).collect())?
        {
            return Ok(hit);
        }
        // 4. 唯一子串（名字或厂商）：`KR-106` 能命中 `Ultramaster KR-106`
        if let Some(hit) = pick(
            entries
                .iter()
                .filter(|e| normalize(&e.name).contains(&norm) || normalize(&e.vendor).contains(&norm))
                .collect(),
        )? {
            return Ok(hit);
        }
    }
    // 5. 中文名等非 ASCII 的原始子串匹配
    if let Some(hit) = pick(entries.iter().filter(|e| e.name.to_lowercase().contains(&lower)).collect())? {
        return Ok(hit);
    }

    let available: Vec<String> = entries.iter().map(|e| format!("{}（{}）", e.name, e.id)).collect();
    Err(format!(
        "没有插件「{raw}」。当前可用：{}。用 list_plugins 看完整清单与音色描述。",
        available.join("、")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn external(id: &str, name: &str, vendor: &str, kind: PluginKind) -> Entry {
        Entry {
            id: id.into(),
            name: name.into(),
            vendor: vendor.into(),
            kind,
            format: "vst3".into(),
            builtin: false,
            // 指向不存在的路径：读不到 moduleinfo 就走关键词兜底，正是要测的路径。
            path: Some(PathBuf::from("/nonexistent")),
        }
    }

    /// 这条是用户报的那个 bug：JC303 必须被认成贝斯，而不是「一台合成器」。
    #[test]
    fn jc303_is_known_as_a_mono_acid_bass() {
        let entry = external("vst3:abc", "JC303", "midilab", PluginKind::Instrument);
        let brief = brief(&entry);
        assert_eq!(brief.category, "bass");
        assert!(brief.mono, "TB-303 是单音乐器，不能叠和弦");
        assert_eq!(brief.roles, vec!["bass"]);
        assert_eq!(brief.pitch_range, Some([28, 64]));
        assert!(brief.description.contains("TB-303"), "描述要说清血统：{}", brief.description);
    }

    /// 名字里的空格/连字符/大小写都不影响识别。
    #[test]
    fn name_matching_ignores_case_spaces_and_dashes() {
        for name in ["JC-303", "jc 303", "JC303", "Jc_303"] {
            let entry = external("vst3:x", name, "midilab", PluginKind::Instrument);
            assert_eq!(brief(&entry).category, "bass", "{name} 应当认成贝斯");
        }
    }

    #[test]
    fn juno_clone_is_a_poly_synth() {
        let entry = external("vst3:k", "Ultramaster KR-106", "Kayrock", PluginKind::Instrument);
        let brief = brief(&entry);
        assert_eq!(brief.category, "synth");
        assert!(!brief.mono, "Juno-106 是复音乐器");
        assert!(brief.roles.contains(&"pad"), "应当建议铺底：{:?}", brief.roles);
    }

    #[test]
    fn rhodes_clone_is_an_electric_piano() {
        let entry = external("vst3:r", "RdPiano", "Giulio Zausa", PluginKind::Instrument);
        assert_eq!(brief(&entry).category, "epiano");
    }

    #[test]
    fn drum_machine_is_drums() {
        let entry = external("vst3:d", "jdrummer", "Justin Ehrlichman", PluginKind::Instrument);
        assert_eq!(brief(&entry).category, "drums");
    }

    /// 认不出的插件不能编音色：给「未收录」+ 分类兜底，并指向 describe_plugin。
    #[test]
    fn unknown_plugin_says_so_instead_of_inventing_a_timbre() {
        let entry = external("vst3:zz", "Zebra9000", "Acme", PluginKind::Effect);
        let brief = brief(&entry);
        assert!(brief.description.contains("未收录"), "{}", brief.description);
        assert!(brief.description.contains("describe_plugin"), "{}", brief.description);
    }

    #[test]
    fn effects_get_effect_categories_not_instrument_ones() {
        let entry = external("vst3:e", "SuperVerb XL", "Acme", PluginKind::Effect);
        let card = brief(&entry);
        assert_eq!(card.category, "reverb");
        assert!(card.roles.is_empty(), "效果器没有编曲分工");
    }

    #[test]
    fn builtin_catalog_is_complete_and_resolvable() {
        let entries = entries(&[]);
        // 内置合成器 + 5 条老预设 + 5 个内置效果器。
        assert_eq!(entries.len(), 11, "{:?}", entries.iter().map(|e| &e.id).collect::<Vec<_>>());
        assert_eq!(
            entries.iter().filter(|e| e.kind == PluginKind::Instrument).count(),
            6
        );
        assert!(entries.iter().all(|e| brief(e).description.len() > 10));
    }

    /// 模型把名字当 id 传是常态——必须认出来，而不是报「未知插件」。
    #[test]
    fn resolve_accepts_names_ids_and_fragments() {
        let entries = vec![
            external("vst3:1", "JC303", "midilab", PluginKind::Instrument),
            external("vst3:2", "Ultramaster KR-106", "Kayrock", PluginKind::Instrument),
        ];
        assert_eq!(resolve(&entries, "vst3:1").unwrap().name, "JC303");
        assert_eq!(resolve(&entries, "JC303").unwrap().id, "vst3:1");
        assert_eq!(resolve(&entries, "jc-303").unwrap().id, "vst3:1");
        assert_eq!(resolve(&entries, "kr-106").unwrap().id, "vst3:2");
        assert_eq!(resolve(&entries, "midilab").unwrap().id, "vst3:1");
    }

    #[test]
    fn resolve_reports_what_is_available_when_it_fails() {
        let entries = vec![external("vst3:1", "JC303", "midilab", PluginKind::Instrument)];
        let err = resolve(&entries, "Serum").unwrap_err();
        assert!(err.contains("JC303"), "报错要列出可用插件：{err}");
        assert!(err.contains("list_plugins"), "{err}");
    }

    #[test]
    fn resolve_is_ambiguous_when_several_names_match() {
        let entries = vec![
            external("vst3:1", "KR-106", "Kayrock", PluginKind::Instrument),
            external("clap:2", "KR-106", "Kayrock", PluginKind::Instrument),
        ];
        let err = resolve(&entries, "KR-106").unwrap_err();
        assert!(err.contains("多个插件"), "{err}");
    }

    #[test]
    fn normalize_keeps_only_ascii_alphanumerics() {
        assert_eq!(normalize("Ultramaster KR-106"), "ultramasterkr106");
        assert_eq!(normalize("JC-303 v0.12"), "jc303v012");
        assert_eq!(normalize("内置合成器"), "");
    }

    /// 手动诊断：打印本机插件的「人话卡片」+ 参数表，用来核对收录表与真实安装的型号对不对得上。
    ///
    /// ```text
    /// cargo test --manifest-path src-tauri/Cargo.toml --lib dump_installed_plugins -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "要扫本机插件（慢，且依赖本机装了什么）"]
    fn dump_installed_plugins() {
        // 测试里没有 AppHandle，按 macOS 的固定配置目录推出「插件管理器」的托管目录；
        // 其它平台就只扫系统标准目录（够用来核对收录表）。
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let managed = home
            .as_ref()
            .map(|h| h.join("Library/Application Support/com.aidaw.app/plugins"));
        let (vst3, clap) = match managed {
            Some(root) => (vec![root.join("vst3")], vec![root.join("clap")]),
            None => (Vec::new(), Vec::new()),
        };
        // AU 的托管目录是用户级 Components 目录（见 `manager::au_managed_dir`）。
        let au = home
            .map(|h| vec![h.join("Library/Audio/Plug-Ins/Components")])
            .unwrap_or_default();
        let catalog = ai_daw_host_vst3::scan_catalog_with(&vst3, &clap, &au);
        for entry in entries(&catalog) {
            let card = brief(&entry);
            println!(
                "\n{}（{}，{}）→ [{}] {}\n  roles={:?} pitch_range={:?} mono={}\n  {}",
                card.name,
                card.vendor,
                card.id,
                card.category,
                category_label(card.category),
                card.roles,
                card.pitch_range,
                card.mono,
                card.description
            );
            match crate::commands::plugin_params_with(&catalog, &entry.id) {
                Ok(params) => {
                    let preview: Vec<String> = params
                        .iter()
                        .take(8)
                        .map(|p| format!("{}={}", p.id.0, p.name))
                        .collect();
                    println!("  参数 {} 个（前 8）：{}", params.len(), preview.join(", "));
                }
                Err(err) => println!("  参数表加载失败：{err}"),
            }
        }
    }
}
