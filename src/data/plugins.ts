/**
 * **内置**插件的清单与参数元数据。
 *
 * 外部插件（VST3 / CLAP）不在这里——它们由 Tauri 后端扫描得到，见 `stores/plugins.ts`。
 * 界面侧**唯一**的插件数据源是那个 store：它把这里的内置音色与扫描结果合并成一份清单。
 *
 * 内置合成器的「预设」是同一套 DSP 配不同参数；其中 5 条在引擎注册表里有独立 id
 * （见 `crates/engine/src/registry.rs` 的 `BUILTIN_PRESETS`），其余 31 条只是参数补丁。
 */
import factoryPresets from "./synth-factory-presets.json";

export type PluginKind = "instrument" | "effect";

export interface PluginInfo {
  id: string;
  name: string;
  /** 顶层类别：乐器 / 效果。 */
  kind: PluginKind;
  /** 细分分类：音色选择器按它分组、浏览器按它筛选。 */
  category: string;
  /** 是否为内置插件。 */
  builtin: boolean;
  /** 是否可加载（宿主能力未接入的条目为 false）。 */
  loadable: boolean;
  /** 搜索关键词（中英文都放，方便按习惯搜到）。 */
  tags: string[];
  /** 一句话说明。 */
  note: string;
  /** 外部插件的格式标识（`"vst3"` / `"clap"`）；内置为 `undefined`。 */
  format?: string;
  /** 是否自带原生编辑器界面。 */
  hasEditor?: boolean;
}

/** 内置合成器的插件 id（与引擎 `ai_daw_engine::BUILTIN_SYNTH` 一致）。 */
export const SYNTH_ID = "builtin.synth";
/** 采样内置乐器（与 `crates/instruments` 的 id 一一对应）。 */
export const PIANO_ID = "builtin.piano";
export const NYLON_GUITAR_ID = "builtin.nylon_guitar";
export const ELECTRIC_GUITAR_ID = "builtin.electric_guitar";
export const BASS_ID = "builtin.bass";
export const DRUMS_ID = "builtin.drums";
export const STRINGS_ID = "builtin.strings";
export const SLOW_STRINGS_ID = "builtin.slow_strings";
export const VIOLIN_ID = "builtin.violin";
export const VIOLA_ID = "builtin.viola";
export const CELLO_ID = "builtin.cello";
export const CONTRABASS_ID = "builtin.contrabass";
export const PIZZICATO_ID = "builtin.pizzicato";
export const TRUMPET_ID = "builtin.trumpet";
export const FRENCH_HORNS_ID = "builtin.french_horns";
export const SYNTH_BRASS_1_ID = "builtin.synth_brass_1";
export const SYNTH_BRASS_2_ID = "builtin.synth_brass_2";
export const SYNTH_STRINGS_1_ID = "builtin.synth_strings_1";
export const SYNTH_STRINGS_2_ID = "builtin.synth_strings_2";
/** 内置延迟的插件 id（与引擎 `BUILTIN_DELAY` 一致）。 */
export const DELAY_ID = "builtin.delay";
/** 内置混响的插件 id（与引擎 `BUILTIN_REVERB` 一致）。 */
export const REVERB_ID = "builtin.reverb";
/** 内置合唱的插件 id（与引擎 `BUILTIN_CHORUS` 一致）。 */
export const CHORUS_ID = "builtin.chorus";
/** 内置过载/失真的插件 id（与引擎 `BUILTIN_DRIVE` 一致）。 */
export const DRIVE_ID = "builtin.drive";
/** 内置两段 EQ 的插件 id（与引擎 `BUILTIN_EQ` 一致）。 */
export const EQ_ID = "builtin.eq";

/**
 * 内置合成器的**出厂预设**。
 *
 * 这些 id 曾经是独立的插件条目（`builtin.synth.lead` 等）。现在它们只是「内置合成器的一组
 * 参数」：插件清单里只留内置合成器一条，切预设即切音色。
 *
 * 其中 5 条（8-bit 主音 / 低音 / 底鼓 / 弦乐 / 铺底）在引擎里仍然按原 id 注册
 * （老工程照旧能加载，见 `registry.rs::BUILTIN_PRESETS`），所以它们的数值必须与那边一致；
 * 其余 31 条只存在于界面这一层。
 */
export interface FactoryPreset {
  /** 稳定 id；其中 5 条同时也是老的插件 id（见 `ENGINE_PRESET_IDS`）。 */
  id: string;
  /** i18n key（界面显示用）。 */
  name: string;
  /** i18n key（界面显示用）。 */
  note: string;
  /** 给 MCP 的中文名快照（界面不用；由 `synth-factory-presets.json` 提供）。 */
  label?: string;
  /** 给 MCP 的中文一句话描述（界面不用）。 */
  description?: string;
  /** 类别 slug：8bit | epiano | guitar | bass | drums | strings | winds | pad | fx（映射为 i18n 键 presetCategory.<slug>）。 */
  category: string;
  params: Record<string, number>;
}

/**
 * 36 条出厂预设：**9 类各 4 条**。
 *
 * 数据在 `synth-factory-presets.json`：前端与 Rust/MCP **共用同一份**，避免两边的参数漂移。
 * 每条都是**补丁**（只列改过的参数），未列出的取 `SYNTH_DEFAULTS`；`wave` 一律显式写出，
 * 免得将来改默认波形时听感跟着漂。
 *
 * **预设只改乐器参数，绝不碰效果链**：音色与效果是两件独立的事，切音色不该动用户自己搭的效果。
 * （早先版本会让预设「自带效果链」并在切换时整体替换轨道效果链——那会把用户的效果冲掉，已废弃。）
 *
 * 其中 5 条（8-bit 主音 / 低音 / 底鼓 / 弦乐 / 铺底）在引擎注册表里有同名条目，供**默认分轨模板**
 * 直接挂上；它们的 id 与数值必须与 `crates/engine/src/registry.rs::BUILTIN_PRESETS` 完全一致。
 * 其余 31 条只活在界面层：轨道始终挂在 `builtin.synth` 上，只记参数差异。
 */
// JSON import 会把每个对象的可选键推断成 `undefined` 的联合类型；数据形状由 Rust 侧测试
// （`factory_presets::tests`）与 `SYNTH_PARAMS` 兜底，这里直接按接口断言。
export const SYNTH_FACTORY_PRESETS: FactoryPreset[] = factoryPresets as unknown as FactoryPreset[];

/**
 * **在引擎注册表里**有同名条目的那 5 条预设（见 `crates/engine/src/registry.rs::BUILTIN_PRESETS`）。
 *
 * 它们存在的唯一理由是**默认分轨模板**要直接挂上去（`STARTER_TRACKS`）：模板里的
 * `plugin_id` 必须是引擎认得的 id，否则那几条轨道一新建就是「缺失插件」占位。
 *
 * 因此只有这 5 条走过 `canonicalPluginId`；其余 31 条只是「同一台合成器的一组参数」，
 * id 仅用于界面匹配，不参与归一键。
 */
export const ENGINE_PRESET_IDS: string[] = [
  "builtin.synth.lead",
  "builtin.synth.strings",
  "builtin.synth.bass",
  "builtin.synth.drums",
  "builtin.synth.pad",
];

/** 把插件 id 归一到「哪个插件」这一层：引擎注册表里的预设音色都算内置合成器。 */
export function canonicalPluginId(pluginId: string): string {
  return ENGINE_PRESET_IDS.includes(pluginId) ? SYNTH_ID : pluginId;
}

/**
 * 面板要显示的参数。
 *
 * 老工程（以及默认分轨）直接挂在某个「预设音色」的 id 上，工程里存的 `params` 是**空的**——
 * 那些包络/滤波值在引擎侧的预设补丁里。这里把出厂补丁摊平，再让工程里真正存过的值覆盖，
 * 于是「预设名」和「旋钮位置」永远一致。
 */
export function effectiveParams(
  pluginId: string | undefined | null,
  params: Record<string, number>,
): Record<string, number> {
  if (!pluginId) return params;
  const factory = SYNTH_FACTORY_PRESETS.find((p) => p.id === pluginId);
  if (factory) return fullSynthParams(factory.params, params);
  // 采样内置乐器（以及其它自带参数表的插件）：工程里空 params 时按参数表默认值显示/保存。
  const specs = BUILTIN_PARAMS[canonicalPluginId(pluginId)];
  if (!specs) return params;
  return { ...Object.fromEntries(specs.map((p) => [p.id, p.default])), ...params };
}

/** 全部内置插件：内置合成器 + 18 件采样乐器 + 5 个内置效果器。 */
export const BUILTIN_PLUGINS: PluginInfo[] = [
  {
    id: SYNTH_ID,
    // 内置条目的名字 / 说明 / 分类都是 **i18n key**，由 `stores/plugins` 在读取时翻译。
    name: "synth.name",
    kind: "instrument",
    category: "pluginCategory.synth",
    builtin: true,
    loadable: true,
    tags: ["synth", "sine", "正弦", "基础", "默认", "合成器"],
    note: "synth.note",
  },
  {
    id: PIANO_ID,
    name: "inst.piano.name",
    kind: "instrument",
    category: "pluginCategory.piano",
    builtin: true,
    loadable: true,
    tags: ["piano", "钢琴", "原声", "keys", "采样"],
    note: "inst.piano.note",
  },
  {
    id: NYLON_GUITAR_ID,
    name: "inst.nylon.name",
    kind: "instrument",
    category: "pluginCategory.guitar",
    builtin: true,
    loadable: true,
    tags: ["guitar", "吉他", "木吉他", "尼龙", "classical"],
    note: "inst.nylon.note",
  },
  {
    id: ELECTRIC_GUITAR_ID,
    name: "inst.electric.name",
    kind: "instrument",
    category: "pluginCategory.guitar",
    builtin: true,
    loadable: true,
    tags: ["guitar", "吉他", "电吉他", "clean"],
    note: "inst.electric.note",
  },
  {
    id: BASS_ID,
    name: "inst.bass.name",
    kind: "instrument",
    category: "pluginCategory.bass",
    builtin: true,
    loadable: true,
    tags: ["bass", "贝斯", "低音", "finger"],
    note: "inst.bass.note",
  },
  {
    id: DRUMS_ID,
    name: "inst.drums.name",
    kind: "instrument",
    category: "pluginCategory.drums",
    builtin: true,
    loadable: true,
    tags: ["drums", "鼓", "原声鼓", "鼓组", "GM"],
    note: "inst.drums.note",
  },
  {
    id: STRINGS_ID,
    name: "inst.strings.name",
    kind: "instrument",
    category: "pluginCategory.strings",
    builtin: true,
    loadable: true,
    tags: ["strings", "弦乐", "群奏", "ensemble", "采样"],
    note: "inst.strings.note",
  },
  {
    id: SLOW_STRINGS_ID,
    name: "inst.slow_strings.name",
    kind: "instrument",
    category: "pluginCategory.strings",
    builtin: true,
    loadable: true,
    tags: ["strings", "弦乐", "慢弦乐", "pad", "铺底", "采样"],
    note: "inst.slow_strings.note",
  },
  {
    id: VIOLIN_ID,
    name: "inst.violin.name",
    kind: "instrument",
    category: "pluginCategory.strings",
    builtin: true,
    loadable: true,
    tags: ["violin", "小提琴", "独奏", "strings", "采样"],
    note: "inst.violin.note",
  },
  {
    id: VIOLA_ID,
    name: "inst.viola.name",
    kind: "instrument",
    category: "pluginCategory.strings",
    builtin: true,
    loadable: true,
    tags: ["viola", "中提琴", "独奏", "strings", "采样"],
    note: "inst.viola.note",
  },
  {
    id: CELLO_ID,
    name: "inst.cello.name",
    kind: "instrument",
    category: "pluginCategory.strings",
    builtin: true,
    loadable: true,
    tags: ["cello", "大提琴", "独奏", "strings", "采样"],
    note: "inst.cello.note",
  },
  {
    id: CONTRABASS_ID,
    name: "inst.contrabass.name",
    kind: "instrument",
    category: "pluginCategory.strings",
    builtin: true,
    loadable: true,
    tags: ["contrabass", "低音提琴", "double bass", "strings", "采样"],
    note: "inst.contrabass.note",
  },
  {
    id: PIZZICATO_ID,
    name: "inst.pizzicato.name",
    kind: "instrument",
    category: "pluginCategory.strings",
    builtin: true,
    loadable: true,
    tags: ["pizzicato", "拨奏", "弦乐", "strings", "采样"],
    note: "inst.pizzicato.note",
  },
  {
    id: TRUMPET_ID,
    name: "inst.trumpet.name",
    kind: "instrument",
    category: "pluginCategory.brass",
    builtin: true,
    loadable: true,
    tags: ["trumpet", "小号", "brass", "铜管", "采样"],
    note: "inst.trumpet.note",
  },
  {
    id: FRENCH_HORNS_ID,
    name: "inst.french_horns.name",
    kind: "instrument",
    category: "pluginCategory.brass",
    builtin: true,
    loadable: true,
    tags: ["french horn", "圆号", "号乐", "brass", "铜管", "采样"],
    note: "inst.french_horns.note",
  },
  {
    id: SYNTH_STRINGS_1_ID,
    name: "inst.synth_strings_1.name",
    kind: "instrument",
    category: "pluginCategory.strings",
    builtin: true,
    loadable: true,
    tags: ["synth strings", "合成弦乐", "strings", "pad", "采样"],
    note: "inst.synth_strings_1.note",
  },
  {
    id: SYNTH_STRINGS_2_ID,
    name: "inst.synth_strings_2.name",
    kind: "instrument",
    category: "pluginCategory.strings",
    builtin: true,
    loadable: true,
    tags: ["synth strings", "合成弦乐", "strings", "pad", "采样"],
    note: "inst.synth_strings_2.note",
  },
  {
    id: SYNTH_BRASS_1_ID,
    name: "inst.synth_brass_1.name",
    kind: "instrument",
    category: "pluginCategory.brass",
    builtin: true,
    loadable: true,
    tags: ["synth brass", "合成铜管", "brass", "铜管", "采样"],
    note: "inst.synth_brass_1.note",
  },
  {
    id: SYNTH_BRASS_2_ID,
    name: "inst.synth_brass_2.name",
    kind: "instrument",
    category: "pluginCategory.brass",
    builtin: true,
    loadable: true,
    tags: ["synth brass", "合成铜管", "dx7", "brass", "铜管", "采样"],
    note: "inst.synth_brass_2.note",
  },
  {
    id: DELAY_ID,
    name: "delay.name",
    kind: "effect",
    category: "pluginCategory.builtinEffect",
    builtin: true,
    loadable: true,
    tags: ["delay", "echo", "延迟", "回声"],
    note: "delay.note",
  },
  {
    id: REVERB_ID,
    name: "reverb.name",
    kind: "effect",
    category: "pluginCategory.builtinEffect",
    builtin: true,
    loadable: true,
    tags: ["reverb", "hall", "room", "混响", "空间"],
    note: "reverb.note",
  },
  {
    id: CHORUS_ID,
    name: "chorus.name",
    kind: "effect",
    category: "pluginCategory.builtinEffect",
    builtin: true,
    loadable: true,
    tags: ["chorus", "modulation", "width", "合唱", "调制", "加宽"],
    note: "chorus.note",
  },
  {
    id: DRIVE_ID,
    name: "drive.name",
    kind: "effect",
    category: "pluginCategory.builtinEffect",
    builtin: true,
    loadable: true,
    tags: ["drive", "distortion", "overdrive", "saturation", "过载", "失真", "饱和"],
    note: "drive.note",
  },
  {
    id: EQ_ID,
    name: "eq.name",
    kind: "effect",
    category: "pluginCategory.builtinEffect",
    builtin: true,
    loadable: true,
    tags: ["eq", "tone", "bass", "treble", "均衡", "音色", "低频", "高频"],
    note: "eq.note",
  },
];

/** 参数元信息（与 `plugin-api` 的 `ParamInfo` 同构，另加界面用的步长与单位）。 */
export interface ParamSpec {
  id: string;
  name: string;
  min: number;
  max: number;
  step: number;
  default: number;
  /** 展示单位（可选）。 */
  unit?: string;
  /** 展示小数位。 */
  digits?: number;
  /** 是否可自动化（缺省视为可自动化）。外部插件由后端 `ParamInfo` 给出。 */
  automatable?: boolean;
}

/**
 * 内置合成器的参数表。
 *
 * 与 `crates/synth` 的 `PARAM_TABLE` **一一对应**（id、范围、默认值都必须一致）。
 * 面板上的分组与显示名在 `SynthPanel.vue` 里；这里保留通用元数据，供预设条与通用参数列表使用。
 */
export const SYNTH_PARAMS: ParamSpec[] = [
  // 前三个与老工程的状态布局一致（gain / attack / release）。
  { id: "gain", name: "音量", min: 0, max: 1, step: 0.01, default: 0.8, digits: 2 },
  { id: "attack", name: "起音", min: 0, max: 1, step: 0.005, default: 0.01, unit: "s", digits: 3 },
  { id: "release", name: "释音", min: 0, max: 4, step: 0.01, default: 0.3, unit: "s", digits: 2 },
  { id: "wave", name: "波形", min: 0, max: 3, step: 1, default: 2, digits: 0 },
  { id: "sub", name: "低八度", min: 0, max: 1, step: 0.01, default: 0, digits: 2 },
  { id: "pwm", name: "脉宽", min: 0, max: 1, step: 0.01, default: 0.5, digits: 2 },
  { id: "glide", name: "滑音", min: 0, max: 0.5, step: 0.005, default: 0, unit: "s", digits: 3 },
  { id: "cutoff", name: "截止", min: 0, max: 1, step: 0.01, default: 0.7, digits: 2 },
  { id: "resonance", name: "共振", min: 0, max: 1, step: 0.01, default: 0.15, digits: 2 },
  { id: "filterEnv", name: "包络量", min: 0, max: 1, step: 0.01, default: 0.35, digits: 2 },
  { id: "keyFollow", name: "键跟随", min: 0, max: 1, step: 0.01, default: 0.5, digits: 2 },
  { id: "hpf", name: "高通", min: 0, max: 1, step: 0.01, default: 0, digits: 2 },
  { id: "decay", name: "衰减", min: 0, max: 2, step: 0.01, default: 0.4, unit: "s", digits: 2 },
  { id: "sustain", name: "延音", min: 0, max: 1, step: 0.01, default: 0.75, digits: 2 },
  { id: "lfoRate", name: "速率", min: 0, max: 1, step: 0.01, default: 0.75, digits: 2 },
  { id: "lfoDelay", name: "延迟", min: 0, max: 1, step: 0.01, default: 0, digits: 2 },
  { id: "lfoPitch", name: "音高", min: 0, max: 1, step: 0.01, default: 0, digits: 2 },
  { id: "lfoFilter", name: "滤波", min: 0, max: 1, step: 0.01, default: 0, digits: 2 },
  // ---- 扩展音源能力（与 Rust 侧同顺序，见 `crates/synth` 的 PARAM_TABLE） ----
  { id: "noise", name: "噪声", min: 0, max: 1, step: 0.01, default: 0, digits: 2 },
  { id: "crush", name: "位深压碎", min: 0, max: 1, step: 0.01, default: 0, digits: 2 },
  { id: "detune", name: "失谐", min: 0, max: 1, step: 0.01, default: 0, digits: 2 },
  { id: "fm", name: "FM 深度", min: 0, max: 1, step: 0.01, default: 0, digits: 2 },
  { id: "fmRatio", name: "FM 比值", min: 0.5, max: 8, step: 0.05, default: 2, digits: 2 },
  { id: "pitchEnv", name: "音高包络", min: 0, max: 1, step: 0.01, default: 0, digits: 2 },
  { id: "pitchDecay", name: "音高扫时长", min: 0, max: 1, step: 0.01, default: 0.1, digits: 2 },
  { id: "filterType", name: "滤波类型", min: 0, max: 2, step: 1, default: 0, digits: 0 },
  // ---- 音色设计扩展（与 Rust 侧 PARAM_TABLE 同顺序；默认值 = 老行为） ----
  { id: "filterEnvMode", name: "滤波包络模式", min: 0, max: 1, step: 1, default: 0, digits: 0 },
  { id: "fAttack", name: "滤波起音", min: 0, max: 1, step: 0.005, default: 0.01, unit: "s", digits: 3 },
  { id: "fDecay", name: "滤波衰减", min: 0, max: 2, step: 0.01, default: 0.4, unit: "s", digits: 2 },
  { id: "fSustain", name: "滤波延音", min: 0, max: 1, step: 0.01, default: 0.75, digits: 2 },
  { id: "fRelease", name: "滤波释音", min: 0, max: 4, step: 0.01, default: 0.3, unit: "s", digits: 2 },
  { id: "lfoMode", name: "LFO 模式", min: 0, max: 1, step: 1, default: 0, digits: 0 },
  { id: "pitchEnvDir", name: "音高包络方向", min: 0, max: 1, step: 1, default: 0, digits: 0 },
  { id: "spread", name: "声部宽度", min: 0, max: 1, step: 0.01, default: 0, digits: 2 },
  { id: "unison", name: "振荡器副本", min: 1, max: 4, step: 1, default: 1, digits: 0 },
  { id: "oversample", name: "2x 过采样", min: 0, max: 1, step: 1, default: 0, digits: 0 },
];

/**
 * 内置合成器参数 id → 显示名（i18n 键）。
 *
 * 与 `SynthPanel.vue` 共用同一张表（那边直接 `import { SYNTH_PARAM_LABELS as LABELS }`）——
 * 参数下拉与旋钮面板显示的名字必须一致。
 */
export const SYNTH_PARAM_LABELS: Record<string, string> = {
  wave: "synth.wave",
  sub: "synth.sub",
  pwm: "synth.pwm",
  glide: "synth.glide",
  hpf: "synth.hpf",
  cutoff: "synth.brightness",
  resonance: "synth.sharpness",
  filterEnv: "synth.filterEnvName",
  keyFollow: "synth.keyFollow",
  attack: "synth.attack",
  decay: "synth.decay",
  sustain: "synth.sustain",
  release: "synth.release",
  lfoRate: "synth.lfoRate",
  lfoDelay: "synth.lfoDelay",
  lfoPitch: "synth.lfoPitch",
  lfoFilter: "synth.lfoFilter",
  gain: "synth.gain",
  noise: "synth.noise",
  crush: "synth.crush",
  detune: "synth.detune",
  fm: "synth.fm",
  fmRatio: "synth.fmRatio",
  pitchEnv: "synth.pitchEnv",
  pitchDecay: "synth.pitchDecay",
  filterType: "synth.filterType",
  filterEnvMode: "synth.filterEnvMode",
  fAttack: "synth.fAttack",
  fDecay: "synth.fDecay",
  fSustain: "synth.fSustain",
  fRelease: "synth.fRelease",
  lfoMode: "synth.lfoMode",
  pitchEnvDir: "synth.pitchEnvDir",
  spread: "synth.spread",
  unison: "synth.unison",
  oversample: "synth.oversample",
};

/**
 * 内置合成器参数的**说明文案**（参数 id → i18n 键）。
 *
 * 与 `SYNTH_PARAM_LABELS` 并列：那张表给「名字」，这张给「这句参数是干什么的」。
 * 只覆盖精选参数（`COMMON_INSTRUMENT_PARAMS`）——它们最常出现在下拉里；其余参数
 * 由 `automationTargetsFor` 自动拼「名称 · 单位 · 范围（默认 x）」。
 */
export const SYNTH_PARAM_HINTS: Record<string, string> = {
  cutoff: "param.hint.inst.cutoff",
  resonance: "param.hint.inst.resonance",
  attack: "param.hint.inst.attack",
  decay: "param.hint.inst.decay",
  sustain: "param.hint.inst.sustain",
  release: "param.hint.inst.release",
  detune: "param.hint.inst.detune",
  lfoRate: "param.hint.inst.lfoRate",
};

/**
 * 自动化下拉里**精选**的参数（插件 id → 参数 id 数组）。
 *
 * 有表就只列表里的（顺序即展示顺序），没有表的插件列全部可自动化参数。
 * 想加/减内置合成器的可选参数，**只改这里**。
 */
export const COMMON_INSTRUMENT_PARAMS: Record<string, string[]> = {
  [SYNTH_ID]: [
    "cutoff",
    "resonance",
    "attack",
    "decay",
    "sustain",
    "release",
    "fDecay",
    "detune",
    "lfoRate",
  ],
  [PIANO_ID]: ["gain", "tone"],
  [NYLON_GUITAR_ID]: ["gain", "tone"],
  [ELECTRIC_GUITAR_ID]: ["gain", "tone"],
  [BASS_ID]: ["gain", "tone"],
  [DRUMS_ID]: ["gain", "tone"],
  [STRINGS_ID]: ["gain", "tone"],
  [SLOW_STRINGS_ID]: ["gain", "tone"],
  [VIOLIN_ID]: ["gain", "tone"],
  [VIOLA_ID]: ["gain", "tone"],
  [CELLO_ID]: ["gain", "tone"],
  [CONTRABASS_ID]: ["gain", "tone"],
  [PIZZICATO_ID]: ["gain", "tone"],
  [TRUMPET_ID]: ["gain", "tone"],
  [FRENCH_HORNS_ID]: ["gain", "tone"],
  [SYNTH_STRINGS_1_ID]: ["gain", "tone"],
  [SYNTH_STRINGS_2_ID]: ["gain", "tone"],
  [SYNTH_BRASS_1_ID]: ["gain", "tone"],
  [SYNTH_BRASS_2_ID]: ["gain", "tone"],
};

/**
 * 某插件**默认绑定**的目标键（**只给参数 id**，`inst:` / `fx<i>:` 前缀由调用方拼）。
 *
 * 这就是「精选集」（`COMMON_INSTRUMENT_PARAMS`）——内置合成器 9 个，采样乐器各 2 个，其他插件空。
 * 走 `canonicalPluginId`，所以老工程的预设音色 id（`builtin.synth.lead` 等）也算内置合成器。
 */
export function defaultBoundParamIds(pluginId: string): string[] {
  return COMMON_INSTRUMENT_PARAMS[canonicalPluginId(pluginId)] ?? [];
}

/** 内置合成器的参数默认值（摊平预设补丁用）。 */
export const SYNTH_DEFAULTS: Record<string, number> = Object.fromEntries(
  SYNTH_PARAMS.map((p) => [p.id, p.default]),
);

/**
 * 把「补丁」摊成完整参数集。
 *
 * 预设是**补丁**（只列改过的参数），但写回工程时必须写全：否则从 A 预设切到 B 预设时，
 * 只有 B 提到的参数会变，A 独有的参数会残留下来。
 */
export function fullSynthParams(
  patch: Record<string, number>,
  overrides: Record<string, number> = {},
): Record<string, number> {
  return { ...SYNTH_DEFAULTS, ...patch, ...overrides };
}

/** 内置延迟的参数表（与 `crates/effects` 的 `delay.rs` 一一对应）。 */
export const DELAY_PARAMS: ParamSpec[] = [
  { id: "time", name: "时间", min: 0, max: 1, step: 0.01, default: 0.35, digits: 2 },
  { id: "feedback", name: "反馈", min: 0, max: 0.95, step: 0.01, default: 0.35, digits: 2 },
  { id: "mix", name: "混合", min: 0, max: 1, step: 0.01, default: 0.35, digits: 2 },
];

/** 内置混响的参数表（与 `crates/effects` 的 `reverb.rs` 一一对应）。 */
export const REVERB_PARAMS: ParamSpec[] = [
  { id: "size", name: "房间大小", min: 0, max: 1, step: 0.01, default: 0.6, digits: 2 },
  { id: "damp", name: "阻尼", min: 0, max: 1, step: 0.01, default: 0.4, digits: 2 },
  { id: "mix", name: "混合", min: 0, max: 1, step: 0.01, default: 0.3, digits: 2 },
];

/** 内置合唱的参数表（与 `crates/effects` 的 `chorus.rs` 一一对应）。 */
export const CHORUS_PARAMS: ParamSpec[] = [
  { id: "rate", name: "速率", min: 0, max: 1, step: 0.01, default: 0.35, digits: 2 },
  { id: "depth", name: "深度", min: 0, max: 1, step: 0.01, default: 0.5, digits: 2 },
  { id: "mix", name: "混合", min: 0, max: 1, step: 0.01, default: 0.5, digits: 2 },
];

/** 内置过载/失真的参数表（与 `crates/effects` 的 `drive.rs` 一一对应）。 */
export const DRIVE_PARAMS: ParamSpec[] = [
  { id: "drive", name: "驱动", min: 0, max: 1, step: 0.01, default: 0.3, digits: 2 },
  { id: "tone", name: "音色", min: 0, max: 1, step: 0.01, default: 0.6, digits: 2 },
  { id: "mix", name: "混合", min: 0, max: 1, step: 0.01, default: 1, digits: 2 },
];

/** 内置两段 EQ 的参数表（与 `crates/effects` 的 `eq.rs` 一一对应；低频/高频可正可负）。 */
export const EQ_PARAMS: ParamSpec[] = [
  { id: "bass", name: "低频", min: -1, max: 1, step: 0.01, default: 0, digits: 2 },
  { id: "treble", name: "高频", min: -1, max: 1, step: 0.01, default: 0, digits: 2 },
  { id: "mix", name: "混合", min: 0, max: 1, step: 0.01, default: 1, digits: 2 },
];

/**
 * 采样内置乐器的参数表。`gain` 的默认值按试听峰值校准，与 Rust 侧
 * `Kind::default_gain()` 必须一致；`tone` 是一阶低通，1 = 旁通。
 */
function sampledInstrumentParams(gain: number): ParamSpec[] {
  return [
    { id: "gain", name: "音量", min: 0, max: 4, step: 0.01, default: gain, digits: 2 },
    { id: "tone", name: "音色", min: 0, max: 1, step: 0.01, default: 1, digits: 2 },
  ];
}

export const PIANO_PARAMS = sampledInstrumentParams(2.5);
export const NYLON_GUITAR_PARAMS = sampledInstrumentParams(3.0);
export const ELECTRIC_GUITAR_PARAMS = sampledInstrumentParams(2.2);
export const BASS_PARAMS = sampledInstrumentParams(3.0);
export const DRUMS_PARAMS = sampledInstrumentParams(2.8);
export const STRINGS_PARAMS = sampledInstrumentParams(3.0);
export const SLOW_STRINGS_PARAMS = sampledInstrumentParams(3.0);
export const VIOLIN_PARAMS = sampledInstrumentParams(2.2);
export const VIOLA_PARAMS = sampledInstrumentParams(2.1);
export const CELLO_PARAMS = sampledInstrumentParams(2.1);
export const CONTRABASS_PARAMS = sampledInstrumentParams(2.5);
export const PIZZICATO_PARAMS = sampledInstrumentParams(1.2);
export const TRUMPET_PARAMS = sampledInstrumentParams(2.2);
export const FRENCH_HORNS_PARAMS = sampledInstrumentParams(2.1);
export const SYNTH_STRINGS_1_PARAMS = sampledInstrumentParams(2.1);
export const SYNTH_STRINGS_2_PARAMS = sampledInstrumentParams(2.3);
export const SYNTH_BRASS_1_PARAMS = sampledInstrumentParams(1.4);
export const SYNTH_BRASS_2_PARAMS = sampledInstrumentParams(1.0);

/** 内置插件的参数表（音源 + 效果器都要预填，界面的通用参数列表按 id 取）。 */
export const BUILTIN_PARAMS: Record<string, ParamSpec[]> = {
  [SYNTH_ID]: SYNTH_PARAMS,
  [PIANO_ID]: PIANO_PARAMS,
  [NYLON_GUITAR_ID]: NYLON_GUITAR_PARAMS,
  [ELECTRIC_GUITAR_ID]: ELECTRIC_GUITAR_PARAMS,
  [BASS_ID]: BASS_PARAMS,
  [DRUMS_ID]: DRUMS_PARAMS,
  [STRINGS_ID]: STRINGS_PARAMS,
  [SLOW_STRINGS_ID]: SLOW_STRINGS_PARAMS,
  [VIOLIN_ID]: VIOLIN_PARAMS,
  [VIOLA_ID]: VIOLA_PARAMS,
  [CELLO_ID]: CELLO_PARAMS,
  [CONTRABASS_ID]: CONTRABASS_PARAMS,
  [PIZZICATO_ID]: PIZZICATO_PARAMS,
  [TRUMPET_ID]: TRUMPET_PARAMS,
  [FRENCH_HORNS_ID]: FRENCH_HORNS_PARAMS,
  [SYNTH_STRINGS_1_ID]: SYNTH_STRINGS_1_PARAMS,
  [SYNTH_STRINGS_2_ID]: SYNTH_STRINGS_2_PARAMS,
  [SYNTH_BRASS_1_ID]: SYNTH_BRASS_1_PARAMS,
  [SYNTH_BRASS_2_ID]: SYNTH_BRASS_2_PARAMS,
  [DELAY_ID]: DELAY_PARAMS,
  [REVERB_ID]: REVERB_PARAMS,
  [CHORUS_ID]: CHORUS_PARAMS,
  [DRIVE_ID]: DRIVE_PARAMS,
  [EQ_ID]: EQ_PARAMS,
};

/**
 * 内置效果器的参数默认值。
 *
 * 效果器实例刚加进链上时 `params` 是**空的**（`EffectAdded` 不带参数），界面按参数表回落到默认值。
 * 所以「命中哪条预设」不能拿实例里存的残缺参数去比，要先补齐成这一份完整参数集。
 */
export const EFFECT_DEFAULTS: Record<string, Record<string, number>> = Object.fromEntries(
  [DELAY_ID, REVERB_ID, CHORUS_ID, DRIVE_ID, EQ_ID].map((id) => [
    id,
    Object.fromEntries((BUILTIN_PARAMS[id] ?? []).map((p) => [p.id, p.default])),
  ]),
);

/** 一条内置效果器的出厂预设。 */
export interface EffectFactoryPreset {
  id: string;
  /** i18n 键。 */
  name: string;
  note: string;
  /** 完整参数集（不写补丁，直接给全，免得切换时残留上一条的值）。 */
  params: Record<string, number>;
}

/**
 * 内置效果器的出厂预设：`效果器 id → 几条命名参数组`。
 *
 * 与合成器预设同一个机制（`stores/presets` 把出厂与用户自存合并成一份列表），
 * 只是效果器没有「类别 / 垫子」那一层——四个值全在一条条上。
 *
 * 延迟的 `time` 是归一值：`秒数 = 0.02 + 0.98 × time`（见 `crates/effects/src/delay.rs`）。
 * 所以 120ms → 0.10，360ms → 0.35，700ms → 0.69。
 */
export const EFFECT_FACTORY_PRESETS: Record<string, EffectFactoryPreset[]> = {
  [DELAY_ID]: [
    {
      id: "builtin.delay.thicken",
      name: "preset.delay.thicken.name",
      note: "preset.delay.thicken.note",
      // 45ms：听不出回声，只把音变厚（double tracking）。
      params: { time: 0.03, feedback: 0.05, mix: 0.16 },
    },
    {
      id: "builtin.delay.slap",
      name: "preset.delay.slap.name",
      note: "preset.delay.slap.note",
      // 120ms：老式板回声（slapback），一次就完。
      params: { time: 0.1, feedback: 0.15, mix: 0.22 },
    },
    {
      id: "builtin.delay.tape",
      name: "preset.delay.tape.name",
      note: "preset.delay.tape.note",
      // 360ms：磁带机那档，三四次回声。
      params: { time: 0.35, feedback: 0.45, mix: 0.32 },
    },
    {
      id: "builtin.delay.ambient",
      name: "preset.delay.ambient.name",
      note: "preset.delay.ambient.note",
      // 700ms：长回声，垫底/音效用的尾巴。
      params: { time: 0.69, feedback: 0.62, mix: 0.38 },
    },
  ],
  [REVERB_ID]: [
    {
      id: "builtin.reverb.room",
      name: "preset.reverb.room.name",
      note: "preset.reverb.room.note",
      // 小房间：短、暗、贴脸。
      params: { size: 0.28, damp: 0.55, mix: 0.18 },
    },
    {
      id: "builtin.reverb.plate",
      name: "preset.reverb.plate.name",
      note: "preset.reverb.plate.note",
      // 板式：亮、密、不拖泥（阻尼最小）。
      params: { size: 0.5, damp: 0.22, mix: 0.25 },
    },
    {
      id: "builtin.reverb.hall",
      name: "preset.reverb.hall.name",
      note: "preset.reverb.hall.note",
      // 大厅：长、宽，乐器通用的那一档。
      params: { size: 0.78, damp: 0.4, mix: 0.32 },
    },
    {
      id: "builtin.reverb.cathedral",
      name: "preset.reverb.cathedral.name",
      note: "preset.reverb.cathedral.note",
      // 教堂：最长、最暗，铺底与音效的尾巴。
      params: { size: 0.95, damp: 0.62, mix: 0.4 },
    },
  ],
  [CHORUS_ID]: [
    {
      id: "builtin.chorus.subtle",
      name: "preset.chorus.subtle.name",
      note: "preset.chorus.subtle.note",
      // 轻微加宽：慢速、浅调制，几乎听不出「颤」，只把单声道音色撑开。
      params: { rate: 0.3, depth: 0.3, mix: 0.28 },
    },
    {
      id: "builtin.chorus.lush",
      name: "preset.chorus.lush.name",
      note: "preset.chorus.lush.note",
      // 经典合唱：中速、较深，80 年代那种「一片」的合唱。
      params: { rate: 0.35, depth: 0.65, mix: 0.5 },
    },
    {
      id: "builtin.chorus.vibrato",
      name: "preset.chorus.vibrato.name",
      note: "preset.chorus.vibrato.note",
      // 颤音：又快又深，接近纯延迟（全湿）→ 明显的音高晃动。
      params: { rate: 0.75, depth: 1.0, mix: 0.7 },
    },
  ],
  [DRIVE_ID]: [
    {
      id: "builtin.drive.warm",
      name: "preset.drive.warm.name",
      note: "preset.drive.warm.note",
      // 温暖：轻驱动 + 偏暗的音色，只添一点谐波。
      params: { drive: 0.25, tone: 0.45, mix: 1 },
    },
    {
      id: "builtin.drive.crunch",
      name: "preset.drive.crunch.name",
      note: "preset.drive.crunch.note",
      // 破音：中等驱动，适合贝斯/吉他的节奏声部。
      params: { drive: 0.6, tone: 0.6, mix: 1 },
    },
    {
      id: "builtin.drive.fuzz",
      name: "preset.drive.fuzz.name",
      note: "preset.drive.fuzz.note",
      // 法兹：驱动拉满，音色偏暗一点免得刺耳。
      params: { drive: 1.0, tone: 0.4, mix: 1 },
    },
  ],
  [EQ_ID]: [
    {
      id: "builtin.eq.bass",
      name: "preset.eq.bass.name",
      note: "preset.eq.bass.note",
      // 低频增强：给贝斯/底鼓加重量。
      params: { bass: 0.6, treble: 0, mix: 1 },
    },
    {
      id: "builtin.eq.bright",
      name: "preset.eq.bright.name",
      note: "preset.eq.bright.note",
      // 提亮：给铺底/弦乐加空气感。
      params: { bass: 0, treble: 0.55, mix: 1 },
    },
    {
      id: "builtin.eq.telephone",
      name: "preset.eq.telephone.name",
      note: "preset.eq.telephone.note",
      // 电话音：砍掉低频、留一点高频，做「复古/收音机」效果。
      params: { bass: -1, treble: 0.6, mix: 1 },
    },
  ],
};
