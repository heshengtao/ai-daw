/**
 * **内置**插件的清单与参数元数据。
 *
 * 外部插件（VST3 / CLAP）不在这里——它们由 Tauri 后端扫描得到，见 `stores/plugins.ts`。
 * 界面侧**唯一**的插件数据源是那个 store：它把这里的内置音色与扫描结果合并成一份清单。
 *
 * 内置合成器的「预设」是同一套 DSP 配不同包络，但它们是各自独立的插件 id
 * （见 `crates/engine/src/registry.rs` 的 `BUILTIN_PRESETS`）——所以这里就是一条条真实的音色。
 */
import type { EffectSpec } from "@/api/types";

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
/** 内置延迟的插件 id（与引擎 `BUILTIN_DELAY` 一致）。 */
export const DELAY_ID = "builtin.delay";
/** 内置混响的插件 id（与引擎 `BUILTIN_REVERB` 一致）。 */
export const REVERB_ID = "builtin.reverb";

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
  /** 类别 slug：8bit | guitar | bass | drums | strings | winds | pad | fx（映射为 i18n 键 presetCategory.<slug>）。 */
  category: string;
  params: Record<string, number>;
  /**
   * 预设自带的**效果链**（可选）。
   *
   * 切预设会**整体替换**轨道效果链——「预设 = 整套音色」，与旋钮微调一样，效果微调也会被覆盖。
   * 没写这个字段的预设等于「效果链为空」，所以从一条带混响的铺底切到干声预设会清掉效果链。
   */
  effects?: EffectSpec[];
}

/**
 * 32 条出厂预设：**8 类各 4 条**，8-bit 是第一类。
 *
 * 每条都是**补丁**（只列改过的参数），未列出的取 `SYNTH_DEFAULTS`；`wave` 一律显式写出，
 * 免得将来改默认波形时听感跟着漂。
 *
 * `effects` 是预设**自带的效果链**（`builtin.reverb` / `builtin.delay`）。切预设会**整体替换**
 * 轨道效果链——「预设 = 整套音色」，跟旋钮微调一样，效果微调也会被覆盖。
 *
 * 其中 5 条（8-bit 主音 / 低音 / 底鼓 / 弦乐 / 铺底）在引擎注册表里有同名条目，供**默认分轨模板**
 * 直接挂上；它们的 id 与数值必须与 `crates/engine/src/registry.rs::BUILTIN_PRESETS` 完全一致，
 * 而且**不能带效果链**（注册表补丁只能带参数）——所以这 5 条刻意都是干声，
 * 各类的湿版本是第 2..4 条。其余 27 条只活在界面层：轨道始终挂在 `builtin.synth` 上，只记参数差异。
 */
export const SYNTH_FACTORY_PRESETS: FactoryPreset[] = [
  // ---- 8-bit 键盘（第一类）----
  {
    id: "builtin.synth.lead",
    name: "preset.bit.lead.name",
    note: "preset.bit.lead.note",
    category: "8bit",
    params: {
      gain: 0.7, wave: 1, pwm: 0.25, attack: 0.002, decay: 0.2, sustain: 0.7, release: 0.06,
      cutoff: 0.95, resonance: 0.05, filterEnv: 0.1, keyFollow: 0, crush: 0.35, hpf: 0.15,
    },
  },
  {
    id: "builtin.synth.bit.chord",
    name: "preset.bit.chord.name",
    note: "preset.bit.chord.note",
    category: "8bit",
    params: {
      gain: 0.55, wave: 1, pwm: 0.5, attack: 0.002, decay: 0.14, sustain: 0.55, release: 0.05,
      cutoff: 0.9, resonance: 0.05, filterEnv: 0.1, keyFollow: 0, crush: 0.4, hpf: 0.15,
    },
  },
  {
    id: "builtin.synth.bit.arp",
    name: "preset.bit.arp.name",
    note: "preset.bit.arp.note",
    category: "8bit",
    params: {
      gain: 0.6, wave: 1, pwm: 0.125, attack: 0.001, decay: 0.07, sustain: 0, release: 0.03,
      cutoff: 0.98, resonance: 0.05, filterEnv: 0.15, keyFollow: 0, crush: 0.5, hpf: 0.2,
    },
  },
  {
    id: "builtin.synth.bit.bell",
    name: "preset.bit.bell.name",
    note: "preset.bit.bell.note",
    category: "8bit",
    params: {
      gain: 0.6, wave: 1, pwm: 0.5, attack: 0.001, decay: 0.35, sustain: 0, release: 0.08,
      cutoff: 0.95, resonance: 0.05, filterEnv: 0.1, keyFollow: 0, crush: 0.25, hpf: 0.1,
      fm: 0.45, fmRatio: 3,
    },
  },

  // ---- 吉他 ----
  {
    id: "builtin.synth.guitar.acoustic",
    name: "preset.guitar.acoustic.name",
    note: "preset.guitar.acoustic.note",
    category: "guitar",
    params: {
      gain: 0.75, wave: 0, attack: 0.001, decay: 0.5, sustain: 0.15, release: 0.25,
      cutoff: 0.7, resonance: 0.1, filterEnv: 0.45, keyFollow: 0.35, noise: 0.18,
      pitchEnv: 0.08, pitchDecay: 0.05,
    },
    effects: [{ plugin_id: "builtin.reverb", params: { size: 0.45, damp: 0.45, mix: 0.22 } }],
  },
  {
    id: "builtin.synth.guitar.electric",
    name: "preset.guitar.electric.name",
    note: "preset.guitar.electric.note",
    category: "guitar",
    params: {
      gain: 0.65, wave: 0, attack: 0.002, decay: 0.6, sustain: 0.35, release: 0.3,
      cutoff: 0.45, resonance: 0.55, filterEnv: 0.6, keyFollow: 0.3, noise: 0.12,
    },
    effects: [{ plugin_id: "builtin.delay", params: { time: 0.3, feedback: 0.28, mix: 0.2 } }],
  },
  {
    id: "builtin.synth.guitar.mute",
    name: "preset.guitar.mute.name",
    note: "preset.guitar.mute.note",
    category: "guitar",
    params: {
      gain: 0.7, wave: 0, attack: 0.001, decay: 0.09, sustain: 0, release: 0.05,
      cutoff: 0.4, resonance: 0.3, filterEnv: 0.35, keyFollow: 0.3, noise: 0.25,
    },
  },
  {
    id: "builtin.synth.guitar.harmonic",
    name: "preset.guitar.harmonic.name",
    note: "preset.guitar.harmonic.note",
    category: "guitar",
    params: {
      gain: 0.6, wave: 2, attack: 0.001, decay: 0.9, sustain: 0.05, release: 0.4,
      cutoff: 0.85, resonance: 0.15, filterEnv: 0.2, keyFollow: 0.3, fm: 0.3, fmRatio: 3.5,
    },
    effects: [{ plugin_id: "builtin.reverb", params: { size: 0.6, damp: 0.3, mix: 0.3 } }],
  },

  // ---- 贝斯 ----
  {
    id: "builtin.synth.bass",
    name: "preset.bass.saw.name",
    note: "preset.bass.saw.note",
    category: "bass",
    params: {
      gain: 0.8, wave: 0, sub: 0.5, attack: 0.004, decay: 0.3, sustain: 0.5, release: 0.1,
      cutoff: 0.32, resonance: 0.2, filterEnv: 0.5, keyFollow: 0.3,
    },
  },
  {
    id: "builtin.synth.bass.sub",
    name: "preset.bass.sub.name",
    note: "preset.bass.sub.note",
    category: "bass",
    params: {
      gain: 0.9, wave: 2, sub: 0.75, attack: 0.006, decay: 0.5, sustain: 0.6, release: 0.12,
      cutoff: 0.2, resonance: 0.05, filterEnv: 0.25, keyFollow: 0.2,
    },
  },
  {
    id: "builtin.synth.bass.fm",
    name: "preset.bass.fm.name",
    note: "preset.bass.fm.note",
    category: "bass",
    params: {
      gain: 0.8, wave: 2, sub: 0.3, attack: 0.002, decay: 0.22, sustain: 0.4, release: 0.08,
      cutoff: 0.6, resonance: 0.1, filterEnv: 0.35, keyFollow: 0.25, fm: 0.55, fmRatio: 2,
    },
  },
  {
    id: "builtin.synth.bass.pick",
    name: "preset.bass.pick.name",
    note: "preset.bass.pick.note",
    category: "bass",
    params: {
      gain: 0.8, wave: 0, sub: 0.4, attack: 0.001, decay: 0.45, sustain: 0.25, release: 0.12,
      cutoff: 0.42, resonance: 0.28, filterEnv: 0.55, keyFollow: 0.3, noise: 0.12,
      pitchEnv: 0.1, pitchDecay: 0.08,
    },
  },

  // ---- 鼓组 ----
  {
    id: "builtin.synth.drums",
    name: "preset.drums.kick.name",
    note: "preset.drums.kick.note",
    category: "drums",
    params: {
      gain: 0.95, wave: 2, attack: 0, decay: 0.18, sustain: 0, release: 0.05,
      cutoff: 0.35, resonance: 0.05, filterEnv: 0.2, keyFollow: 0, pitchEnv: 0.55,
      pitchDecay: 0.35,
    },
  },
  {
    id: "builtin.synth.drums.snare",
    name: "preset.drums.snare.name",
    note: "preset.drums.snare.note",
    category: "drums",
    params: {
      gain: 0.8, wave: 3, attack: 0, decay: 0.16, sustain: 0, release: 0.06,
      cutoff: 0.75, resonance: 0.15, filterEnv: 0.2, keyFollow: 0, noise: 0.6,
      pitchEnv: 0.2, pitchDecay: 0.12, hpf: 0.2,
    },
    effects: [{ plugin_id: "builtin.reverb", params: { size: 0.35, damp: 0.5, mix: 0.18 } }],
  },
  {
    id: "builtin.synth.drums.hat",
    name: "preset.drums.hat.name",
    note: "preset.drums.hat.note",
    category: "drums",
    params: {
      gain: 0.5, wave: 1, attack: 0, decay: 0.045, sustain: 0, release: 0.02,
      cutoff: 0.5, resonance: 0.1, filterEnv: 0, keyFollow: 0, noise: 1, hpf: 0.55,
      filterType: 2,
    },
  },
  {
    id: "builtin.synth.drums.tom",
    name: "preset.drums.tom.name",
    note: "preset.drums.tom.note",
    category: "drums",
    params: {
      gain: 0.85, wave: 2, attack: 0, decay: 0.3, sustain: 0, release: 0.08,
      cutoff: 0.5, resonance: 0.08, filterEnv: 0.2, keyFollow: 0, noise: 0.08,
      pitchEnv: 0.3, pitchDecay: 0.22,
    },
    effects: [{ plugin_id: "builtin.reverb", params: { size: 0.4, damp: 0.5, mix: 0.15 } }],
  },

  // ---- 弦乐 ----
  {
    id: "builtin.synth.strings",
    name: "preset.strings.ensemble.name",
    note: "preset.strings.ensemble.note",
    category: "strings",
    params: {
      gain: 0.55, wave: 0, attack: 0.3, decay: 0.8, sustain: 0.85, release: 0.9,
      cutoff: 0.55, resonance: 0.1, filterEnv: 0.2, keyFollow: 0.4, detune: 0.6,
      lfoRate: 0.5, lfoDelay: 0.4, lfoPitch: 0.06,
    },
  },
  {
    id: "builtin.synth.strings.pizz",
    name: "preset.strings.pizz.name",
    note: "preset.strings.pizz.note",
    category: "strings",
    params: {
      gain: 0.7, wave: 0, attack: 0.001, decay: 0.22, sustain: 0.05, release: 0.15,
      cutoff: 0.65, resonance: 0.15, filterEnv: 0.4, keyFollow: 0.35, detune: 0.35,
      noise: 0.1,
    },
    effects: [{ plugin_id: "builtin.reverb", params: { size: 0.5, damp: 0.4, mix: 0.3 } }],
  },
  {
    id: "builtin.synth.strings.cello",
    name: "preset.strings.cello.name",
    note: "preset.strings.cello.note",
    category: "strings",
    params: {
      gain: 0.6, wave: 0, attack: 0.45, decay: 0.9, sustain: 0.85, release: 1,
      cutoff: 0.42, resonance: 0.12, filterEnv: 0.18, keyFollow: 0.3, detune: 0.5,
      lfoRate: 0.45, lfoDelay: 0.35, lfoPitch: 0.05,
    },
    effects: [{ plugin_id: "builtin.reverb", params: { size: 0.65, damp: 0.35, mix: 0.3 } }],
  },
  {
    id: "builtin.synth.strings.trem",
    name: "preset.strings.trem.name",
    note: "preset.strings.trem.note",
    category: "strings",
    params: {
      gain: 0.6, wave: 0, attack: 0.25, decay: 0.7, sustain: 0.85, release: 0.8,
      cutoff: 0.5, resonance: 0.12, filterEnv: 0.2, keyFollow: 0.35, detune: 0.7,
      lfoRate: 0.62, lfoDelay: 0.15, lfoPitch: 0.16,
    },
    effects: [{ plugin_id: "builtin.reverb", params: { size: 0.6, damp: 0.35, mix: 0.28 } }],
  },

  // ---- 管乐 ----
  {
    id: "builtin.synth.winds.flute",
    name: "preset.winds.flute.name",
    note: "preset.winds.flute.note",
    category: "winds",
    params: {
      gain: 0.65, wave: 3, attack: 0.09, decay: 0.5, sustain: 0.85, release: 0.35,
      cutoff: 0.75, resonance: 0.08, filterEnv: 0.15, keyFollow: 0.4, noise: 0.14,
      lfoRate: 0.6, lfoDelay: 0.3, lfoPitch: 0.08,
    },
    effects: [{ plugin_id: "builtin.reverb", params: { size: 0.5, damp: 0.3, mix: 0.3 } }],
  },
  {
    id: "builtin.synth.winds.clarinet",
    name: "preset.winds.clarinet.name",
    note: "preset.winds.clarinet.note",
    category: "winds",
    params: {
      gain: 0.65, wave: 1, pwm: 0.5, attack: 0.06, decay: 0.6, sustain: 0.9, release: 0.3,
      cutoff: 0.6, resonance: 0.1, filterEnv: 0.12, keyFollow: 0.4, noise: 0.05,
      lfoRate: 0.55, lfoDelay: 0.35, lfoPitch: 0.06,
    },
    effects: [{ plugin_id: "builtin.reverb", params: { size: 0.5, damp: 0.4, mix: 0.26 } }],
  },
  {
    id: "builtin.synth.winds.sax",
    name: "preset.winds.sax.name",
    note: "preset.winds.sax.note",
    category: "winds",
    params: {
      gain: 0.6, wave: 0, attack: 0.07, decay: 0.5, sustain: 0.8, release: 0.35,
      cutoff: 0.55, resonance: 0.22, filterEnv: 0.3, keyFollow: 0.35, noise: 0.1,
      lfoRate: 0.58, lfoDelay: 0.2, lfoPitch: 0.12,
    },
    effects: [{ plugin_id: "builtin.reverb", params: { size: 0.55, damp: 0.4, mix: 0.3 } }],
  },
  {
    id: "builtin.synth.winds.brass",
    name: "preset.winds.brass.name",
    note: "preset.winds.brass.note",
    category: "winds",
    params: {
      gain: 0.65, wave: 0, attack: 0.05, decay: 0.4, sustain: 0.8, release: 0.28,
      cutoff: 0.45, resonance: 0.18, filterEnv: 0.5, keyFollow: 0.35, detune: 0.25,
      noise: 0.06,
    },
    effects: [{ plugin_id: "builtin.reverb", params: { size: 0.45, damp: 0.45, mix: 0.2 } }],
  },

  // ---- 铺底 ----
  {
    id: "builtin.synth.pad",
    name: "preset.pad.basic.name",
    note: "preset.pad.basic.note",
    category: "pad",
    params: {
      gain: 0.5, wave: 0, attack: 0.6, decay: 1.2, sustain: 0.9, release: 1.6,
      cutoff: 0.45, resonance: 0.12, filterEnv: 0.25, keyFollow: 0.3, detune: 0.75,
      lfoRate: 0.35, lfoDelay: 0.5, lfoFilter: 0.25,
    },
  },
  {
    id: "builtin.synth.pad.warm",
    name: "preset.pad.warm.name",
    note: "preset.pad.warm.note",
    category: "pad",
    params: {
      gain: 0.55, wave: 3, attack: 0.8, decay: 1.4, sustain: 0.9, release: 1.8,
      cutoff: 0.34, resonance: 0.08, filterEnv: 0.2, keyFollow: 0.28, detune: 0.5,
      lfoRate: 0.3, lfoDelay: 0.6, lfoFilter: 0.2,
    },
    effects: [{ plugin_id: "builtin.reverb", params: { size: 0.8, damp: 0.5, mix: 0.38 } }],
  },
  {
    id: "builtin.synth.pad.glass",
    name: "preset.pad.glass.name",
    note: "preset.pad.glass.note",
    category: "pad",
    params: {
      gain: 0.42, wave: 2, attack: 0.5, decay: 1.5, sustain: 0.85, release: 1.8,
      cutoff: 0.8, resonance: 0.1, filterEnv: 0.15, keyFollow: 0.3, fm: 0.35, fmRatio: 4,
      lfoRate: 0.4, lfoDelay: 0.4, lfoFilter: 0.3,
    },
    effects: [
      { plugin_id: "builtin.reverb", params: { size: 0.85, damp: 0.25, mix: 0.4 } },
      { plugin_id: "builtin.delay", params: { time: 0.45, feedback: 0.4, mix: 0.22 } },
    ],
  },
  {
    id: "builtin.synth.pad.choir",
    name: "preset.pad.choir.name",
    note: "preset.pad.choir.note",
    category: "pad",
    params: {
      gain: 0.5, wave: 3, attack: 0.7, decay: 1.3, sustain: 0.9, release: 1.7,
      cutoff: 0.5, resonance: 0.15, filterEnv: 0.2, keyFollow: 0.3, detune: 0.6,
      noise: 0.06, lfoRate: 0.42, lfoDelay: 0.3, lfoPitch: 0.05, filterType: 1,
    },
    effects: [{ plugin_id: "builtin.reverb", params: { size: 0.8, damp: 0.4, mix: 0.38 } }],
  },

  // ---- 音效 ----
  {
    id: "builtin.synth.fx.riser",
    name: "preset.fx.riser.name",
    note: "preset.fx.riser.note",
    category: "fx",
    params: {
      gain: 0.55, wave: 2, attack: 1, decay: 2, sustain: 0.6, release: 1.2,
      cutoff: 0.1, resonance: 0.35, filterEnv: 1, keyFollow: 0, noise: 0.45,
    },
    effects: [{ plugin_id: "builtin.reverb", params: { size: 0.9, damp: 0.3, mix: 0.45 } }],
  },
  {
    id: "builtin.synth.fx.wobble",
    name: "preset.fx.wobble.name",
    note: "preset.fx.wobble.note",
    category: "fx",
    params: {
      gain: 0.6, wave: 0, sub: 0.6, attack: 0.01, decay: 1, sustain: 0.9, release: 0.3,
      cutoff: 0.25, resonance: 0.65, filterEnv: 0.2, keyFollow: 0.2, detune: 0.3,
      lfoRate: 0.55, lfoDelay: 0, lfoFilter: 0.85,
    },
    effects: [{ plugin_id: "builtin.delay", params: { time: 0.28, feedback: 0.35, mix: 0.25 } }],
  },
  {
    id: "builtin.synth.fx.drone",
    name: "preset.fx.drone.name",
    note: "preset.fx.drone.note",
    category: "fx",
    params: {
      gain: 0.45, wave: 0, attack: 1.2, decay: 2, sustain: 1, release: 3,
      cutoff: 0.3, resonance: 0.4, filterEnv: 0.3, keyFollow: 0.1, detune: 0.9,
      crush: 0.4, lfoRate: 0.15, lfoDelay: 0, lfoFilter: 0.5,
    },
    effects: [{ plugin_id: "builtin.reverb", params: { size: 1, damp: 0.2, mix: 0.5 } }],
  },
  {
    id: "builtin.synth.fx.blip",
    name: "preset.fx.blip.name",
    note: "preset.fx.blip.note",
    category: "fx",
    params: {
      gain: 0.55, wave: 0, attack: 0, decay: 0.05, sustain: 0, release: 0.03,
      cutoff: 0.8, resonance: 0.3, filterEnv: 0.4, keyFollow: 0, crush: 0.8,
      pitchEnv: 0.7, pitchDecay: 0.08,
    },
    effects: [{ plugin_id: "builtin.delay", params: { time: 0.18, feedback: 0.5, mix: 0.35 } }],
  },
];

/**
 * **在引擎注册表里**有同名条目的那 5 条预设（见 `crates/engine/src/registry.rs::BUILTIN_PRESETS`）。
 *
 * 它们存在的唯一理由是**默认分轨模板**要直接挂上去（`STARTER_TRACKS`）：模板里的
 * `plugin_id` 必须是引擎认得的 id，否则那几条轨道一新建就是「缺失插件」占位。
 *
 * 因此只有这 5 条走过 `canonicalPluginId`；其余 27 条只是「同一台合成器的一组参数」，
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

/** 内置插件的参数表（音源 + 效果器都要预填，界面的通用参数列表按 id 取）。 */
export const BUILTIN_PARAMS: Record<string, ParamSpec[]> = {
  [SYNTH_ID]: SYNTH_PARAMS,
  [DELAY_ID]: DELAY_PARAMS,
  [REVERB_ID]: REVERB_PARAMS,
};
