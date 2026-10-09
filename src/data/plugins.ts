/**
 * **内置**插件的清单与参数元数据。
 *
 * 外部插件（VST3 / CLAP）不在这里——它们由 Tauri 后端扫描得到，见 `stores/plugins.ts`。
 * 界面侧**唯一**的插件数据源是那个 store：它把这里的内置音色与扫描结果合并成一份清单。
 *
 * 内置合成器的「预设」是同一套 DSP 配不同包络，但它们是各自独立的插件 id
 * （见 `crates/engine/src/registry.rs` 的 `BUILTIN_PRESETS`）——所以这里就是一条条真实的音色。
 */
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

/** 内置合成器的插件 id（与引擎 `daw_party_engine::BUILTIN_SYNTH` 一致）。 */
export const SYNTH_ID = "builtin.synth";

/**
 * 内置合成器的**出厂预设**。
 *
 * 这些 id 曾经是独立的插件条目（`builtin.synth.lead` 等）。现在它们只是「内置合成器的一组
 * 参数」：插件清单里只留内置合成器一条，切预设即切音色。
 *
 * 其中**前四条**（lead / pluck / bass / pad）在引擎里仍然按原 id 注册
 * （老工程照旧能加载、听感不变，见 `registry.rs::BUILTIN_PRESETS`），所以它们的数值必须与
 * 那边保持一致；后 20 条是新加的音色，只存在于界面这一层。
 */
export interface FactoryPreset {
  /** 稳定 id；前四条同时也是老的插件 id（见 `LEGACY_SYNTH_IDS`）。 */
  id: string;
  name: string;
  note: string;
  /** 类别 slug：bass | lead | keys | pad | pluck | fx（界面映射为 i18n 键 presetCategory.<slug>）。 */
  category: string;
  params: Record<string, number>;
}

/**
 * 24 条出厂预设：6 类各 4 条。
 *
 * 每条都是**补丁**（只列改过的参数），未列出的取 `SYNTH_DEFAULTS`；`params` 里显式写出的
 * `wave` 是为了不受默认波形变化影响（老四条与引擎补丁一致）。
 * 顺序按类别排列：主音 → 拨弦 → 低音 → 铺底 → 键盘 → 音效。
 */
export const SYNTH_FACTORY_PRESETS: FactoryPreset[] = [
  // ---- 主音 Lead ----
  {
    id: "builtin.synth.lead",
    name: "preset.lead.name",
    note: "preset.lead.note",
    category: "lead",
    params: {
      gain: 0.75, wave: 0, attack: 0.005, release: 0.25, cutoff: 0.8, resonance: 0.22,
      filterEnv: 0.45, decay: 0.35, sustain: 0.7, lfoRate: 0.8, lfoDelay: 0.35, lfoPitch: 0.1,
    },
  },
  {
    id: "builtin.synth.lead.soft",
    name: "preset.lead.soft.name",
    note: "preset.lead.soft.note",
    category: "lead",
    params: {
      gain: 0.7, wave: 2, attack: 0.02, release: 0.3, cutoff: 0.7, resonance: 0.1,
      decay: 0.4, sustain: 0.75,
    },
  },
  {
    id: "builtin.synth.lead.square",
    name: "preset.lead.square.name",
    note: "preset.lead.square.note",
    category: "lead",
    params: {
      gain: 0.72, wave: 1, attack: 0.004, release: 0.2, cutoff: 0.65, resonance: 0.25,
      filterEnv: 0.4, decay: 0.3, sustain: 0.7,
    },
  },
  {
    id: "builtin.synth.lead.sync",
    name: "preset.lead.sync.name",
    note: "preset.lead.sync.note",
    category: "lead",
    params: {
      gain: 0.7, wave: 3, attack: 0.003, release: 0.18, cutoff: 0.85, resonance: 0.35,
      filterEnv: 0.5, decay: 0.25, sustain: 0.65, lfoRate: 0.85, lfoPitch: 0.15,
    },
  },
  // ---- 拨弦 Pluck ----
  {
    id: "builtin.synth.pluck",
    name: "preset.pluck.name",
    note: "preset.pluck.note",
    category: "pluck",
    params: {
      gain: 0.8, wave: 1, attack: 0.001, release: 0.08, cutoff: 0.55, filterEnv: 0.7,
      decay: 0.12, sustain: 0,
    },
  },
  {
    id: "builtin.synth.pluck.soft",
    name: "preset.pluck.soft.name",
    note: "preset.pluck.soft.note",
    category: "pluck",
    params: {
      gain: 0.75, wave: 3, attack: 0.002, release: 0.2, cutoff: 0.5, resonance: 0.1,
      filterEnv: 0.5, decay: 0.2, sustain: 0.1,
    },
  },
  {
    id: "builtin.synth.pluck.bell",
    name: "preset.pluck.bell.name",
    note: "preset.pluck.bell.note",
    category: "pluck",
    params: {
      gain: 0.72, wave: 2, attack: 0.001, release: 0.35, cutoff: 0.85, resonance: 0.25,
      filterEnv: 0.6, decay: 0.45, sustain: 0,
    },
  },
  {
    id: "builtin.synth.pluck.mute",
    name: "preset.pluck.mute.name",
    note: "preset.pluck.mute.note",
    category: "pluck",
    params: {
      gain: 0.78, wave: 0, attack: 0.001, release: 0.06, cutoff: 0.4, resonance: 0.2,
      filterEnv: 0.55, decay: 0.1, sustain: 0,
    },
  },
  // ---- 低音 Bass ----
  {
    id: "builtin.synth.bass",
    name: "preset.bass.name",
    note: "preset.bass.note",
    category: "bass",
    params: {
      gain: 0.85, wave: 0, attack: 0.002, release: 0.12, sub: 0.6, cutoff: 0.35,
      resonance: 0.25, filterEnv: 0.5, decay: 0.25, sustain: 0.5,
    },
  },
  {
    id: "builtin.synth.bass.sub",
    name: "preset.bass.sub.name",
    note: "preset.bass.sub.note",
    category: "bass",
    params: {
      gain: 0.8, wave: 0, attack: 0.004, release: 0.2, sub: 0.9, cutoff: 0.28,
      decay: 0.35, sustain: 0.6,
    },
  },
  {
    id: "builtin.synth.bass.round",
    name: "preset.bass.round.name",
    note: "preset.bass.round.note",
    category: "bass",
    params: {
      gain: 0.8, wave: 2, attack: 0.006, release: 0.18, sub: 0.4, cutoff: 0.3,
      resonance: 0.1, decay: 0.3, sustain: 0.55,
    },
  },
  {
    id: "builtin.synth.bass.pick",
    name: "preset.bass.pick.name",
    note: "preset.bass.pick.note",
    category: "bass",
    params: {
      gain: 0.8, wave: 1, attack: 0.001, release: 0.08, cutoff: 0.55, filterEnv: 0.7,
      decay: 0.12, sustain: 0.1,
    },
  },
  // ---- 铺底 Pad ----
  {
    id: "builtin.synth.pad",
    name: "preset.pad.name",
    note: "preset.pad.note",
    category: "pad",
    params: {
      gain: 0.55, wave: 0, attack: 0.35, release: 1.2, cutoff: 0.5, filterEnv: 0.3,
      decay: 1, sustain: 0.8, lfoRate: 0.6, lfoFilter: 0.15,
    },
  },
  {
    id: "builtin.synth.pad.strings",
    name: "preset.pad.strings.name",
    note: "preset.pad.strings.note",
    category: "pad",
    params: {
      gain: 0.6, wave: 0, attack: 0.25, release: 1, sub: 0.2, cutoff: 0.55,
      resonance: 0.15, filterEnv: 0.25, decay: 0.9, sustain: 0.85, lfoRate: 0.5, lfoFilter: 0.1,
    },
  },
  {
    id: "builtin.synth.pad.glass",
    name: "preset.pad.glass.name",
    note: "preset.pad.glass.note",
    category: "pad",
    params: {
      gain: 0.5, wave: 2, attack: 0.5, release: 1.5, cutoff: 0.7, resonance: 0.18,
      filterEnv: 0.2, decay: 1.2, sustain: 0.8, lfoRate: 0.4, lfoPitch: 0.05,
    },
  },
  {
    id: "builtin.synth.pad.choir",
    name: "preset.pad.choir.name",
    note: "preset.pad.choir.note",
    category: "pad",
    params: {
      gain: 0.52, wave: 3, attack: 0.4, release: 1.4, sub: 0.15, cutoff: 0.45,
      filterEnv: 0.35, decay: 1.1, sustain: 0.85, lfoRate: 0.55, lfoFilter: 0.2,
    },
  },
  // ---- 键盘 Keys ----
  {
    id: "builtin.synth.keys.ep",
    name: "preset.keys.ep.name",
    note: "preset.keys.ep.note",
    category: "keys",
    params: {
      gain: 0.75, wave: 2, attack: 0.008, release: 0.45, cutoff: 0.6, resonance: 0.12,
      filterEnv: 0.35, decay: 0.5, sustain: 0.5,
    },
  },
  {
    id: "builtin.synth.keys.bell",
    name: "preset.keys.bell.name",
    note: "preset.keys.bell.note",
    category: "keys",
    params: {
      gain: 0.7, wave: 2, attack: 0.001, release: 0.8, cutoff: 0.9, resonance: 0.2,
      filterEnv: 0.6, decay: 0.9, sustain: 0.2,
    },
  },
  {
    id: "builtin.synth.keys.organ",
    name: "preset.keys.organ.name",
    note: "preset.keys.organ.note",
    category: "keys",
    params: {
      gain: 0.72, wave: 1, attack: 0.003, release: 0.12, sub: 0.3, cutoff: 0.75,
      sustain: 0.95,
    },
  },
  {
    id: "builtin.synth.keys.pluck",
    name: "preset.keys.pluck.name",
    note: "preset.keys.pluck.note",
    category: "keys",
    params: {
      gain: 0.78, wave: 3, attack: 0.001, release: 0.25, cutoff: 0.55, filterEnv: 0.65,
      decay: 0.3, sustain: 0.15,
    },
  },
  // ---- 音效 FX ----
  {
    id: "builtin.synth.fx.riser",
    name: "preset.fx.riser.name",
    note: "preset.fx.riser.note",
    category: "fx",
    params: {
      gain: 0.6, wave: 0, attack: 0.6, release: 0.8, cutoff: 0.4, resonance: 0.4,
      filterEnv: 0.8, decay: 1.5, sustain: 0.9, lfoRate: 0.9, lfoFilter: 0.35, lfoPitch: 0.2,
    },
  },
  {
    id: "builtin.synth.fx.wobble",
    name: "preset.fx.wobble.name",
    note: "preset.fx.wobble.note",
    category: "fx",
    params: {
      gain: 0.65, wave: 1, attack: 0.01, release: 0.3, sub: 0.4, pwm: 0.5, cutoff: 0.35,
      resonance: 0.5, filterEnv: 0.5, decay: 0.6, sustain: 0.7, lfoRate: 0.35, lfoFilter: 0.8,
    },
  },
  {
    id: "builtin.synth.fx.drone",
    name: "preset.fx.drone.name",
    note: "preset.fx.drone.note",
    category: "fx",
    params: {
      gain: 0.5, wave: 0, attack: 0.8, release: 2, sub: 0.7, cutoff: 0.35,
      resonance: 0.3, decay: 2, sustain: 0.95, lfoRate: 0.25, lfoFilter: 0.4,
    },
  },
  {
    id: "builtin.synth.fx.blip",
    name: "preset.fx.blip.name",
    note: "preset.fx.blip.note",
    category: "fx",
    params: {
      gain: 0.7, wave: 1, attack: 0.001, release: 0.05, pwm: 0.8, cutoff: 0.8,
      resonance: 0.5, filterEnv: 0.8, decay: 0.06, sustain: 0,
    },
  },
];

/**
 * 老的「预设音色」插件 id：现在都归内置合成器。
 *
 * **只有**这四条走过 `canonicalPluginId`（老工程里轨道直接挂在它们身上）。上面新增的 20 条
 * 只是「同一台合成器的一组参数」，id 仅用于界面匹配，不进这份名单、不参与归一键。
 */
export const LEGACY_SYNTH_IDS: string[] = [
  "builtin.synth.lead",
  "builtin.synth.pluck",
  "builtin.synth.bass",
  "builtin.synth.pad",
];

/** 把插件 id 归一到「哪个插件」这一层：老的预设音色都算内置合成器。 */
export function canonicalPluginId(pluginId: string): string {
  return LEGACY_SYNTH_IDS.includes(pluginId) ? SYNTH_ID : pluginId;
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
  if (!factory) return params;
  return fullSynthParams(factory.params, params);
}

/** 全部内置插件：内置合成器家族现在只剩它一个（其余是预设，不再是独立条目）。 */
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
];

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

/** 内置插件的参数表（外部插件的参数表由后端提供）。 */
export const BUILTIN_PARAMS: Record<string, ParamSpec[]> = Object.fromEntries(
  BUILTIN_PLUGINS.filter((p) => p.kind === "instrument").map((p) => [p.id, SYNTH_PARAMS]),
);
