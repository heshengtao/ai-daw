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
 * 引擎仍然按这些 id 注册（老工程照旧能加载、听感不变，见 `registry.rs::BUILTIN_PRESETS`），
 * 所以数值必须与那边保持一致。
 */
export interface FactoryPreset {
  /** 稳定 id；同时也是老的插件 id。 */
  id: string;
  name: string;
  note: string;
  params: Record<string, number>;
}

export const SYNTH_FACTORY_PRESETS: FactoryPreset[] = [
  {
    id: "builtin.synth.lead",
    name: "preset.lead.name",
    note: "preset.lead.note",
    params: {
      gain: 0.75,
      wave: 0,
      attack: 0.005,
      release: 0.25,
      cutoff: 0.8,
      resonance: 0.22,
      filterEnv: 0.45,
      decay: 0.35,
      sustain: 0.7,
      lfoRate: 0.8,
      lfoDelay: 0.35,
      lfoPitch: 0.1,
    },
  },
  {
    id: "builtin.synth.pluck",
    name: "preset.pluck.name",
    note: "preset.pluck.note",
    params: {
      gain: 0.8,
      wave: 1,
      attack: 0.001,
      release: 0.08,
      cutoff: 0.55,
      filterEnv: 0.7,
      decay: 0.12,
      sustain: 0,
    },
  },
  {
    id: "builtin.synth.bass",
    name: "preset.bass.name",
    note: "preset.bass.note",
    params: {
      gain: 0.85,
      wave: 0,
      attack: 0.002,
      release: 0.12,
      sub: 0.6,
      cutoff: 0.35,
      resonance: 0.25,
      filterEnv: 0.5,
      decay: 0.25,
      sustain: 0.5,
    },
  },
  {
    id: "builtin.synth.pad",
    name: "preset.pad.name",
    note: "preset.pad.note",
    params: {
      gain: 0.55,
      wave: 0,
      attack: 0.35,
      release: 1.2,
      cutoff: 0.5,
      filterEnv: 0.3,
      decay: 1,
      sustain: 0.8,
      lfoRate: 0.6,
      lfoFilter: 0.15,
    },
  },
];

/** 老的「预设音色」插件 id：现在都归内置合成器。 */
export const LEGACY_SYNTH_IDS: string[] = SYNTH_FACTORY_PRESETS.map((p) => p.id);

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
