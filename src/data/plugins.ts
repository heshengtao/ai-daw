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

/** 内置合成器的插件 id（与引擎 `ai_daw_engine::BUILTIN_SYNTH` 一致）。 */
export const SYNTH_ID = "builtin.synth";
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
}

/**
 * 32 条出厂预设：**8 类各 4 条**，8-bit 是第一类。
 *
 * 每条都是**补丁**（只列改过的参数），未列出的取 `SYNTH_DEFAULTS`；`wave` 一律显式写出，
 * 免得将来改默认波形时听感跟着漂。
 *
 * **预设只改乐器参数，绝不碰效果链**：音色与效果是两件独立的事，切音色不该动用户自己搭的效果。
 * （早先版本会让预设「自带效果链」并在切换时整体替换轨道效果链——那会把用户的效果冲掉，已废弃。）
 *
 * 其中 5 条（8-bit 主音 / 低音 / 底鼓 / 弦乐 / 铺底）在引擎注册表里有同名条目，供**默认分轨模板**
 * 直接挂上；它们的 id 与数值必须与 `crates/engine/src/registry.rs::BUILTIN_PRESETS` 完全一致。
 * 其余 27 条只活在界面层：轨道始终挂在 `builtin.synth` 上，只记参数差异。
 */
export const SYNTH_FACTORY_PRESETS: FactoryPreset[] = [
  // ---- 8-bit 键盘（第一类）----
  //
  // 依据 NES 2A03（RP2A03）硬件的实际规格，不靠猜：
  // - 两个脉冲通道的**占空比固定为 12.5% / 25% / 50% / 75%**（见 nesdev wiki "APU Pulse"）。
  //   本合成器的 `pwm` 映射是 `占空比 = 0.5 + (pwm-0.5)*0.9`，所以四个占空比对应
  //   pwm = 0.083 / 0.222 / 0.5 / 0.778——之前的 0.125/0.25 并不是硬件值。
  // - **没有滤波器**：所以 cutoff 开到 1（≈18 kHz，等于旁通）、resonance / filterEnv 全 0。
  // - 一个三角波通道（用于低音与旋律主干），没有占空比概念。
  // - 音量是 4 bit、包络按 **60 Hz 帧**推进（FamiTracker 的乐器包络就是帧数）：
  //   下面把帧数换算成秒（1 帧 ≈ 16.7 ms）写进 ADSR。
  // - 硬件**不量化波形**，所以这里的 crush 一律为 0（早先拿它当「8 bit 味道」是错的，
  //   而且旧实现是降采样，会引入大量混叠）。
  // - 主音带**延迟颤音**（起音后才起振的 vibrato），这是芯片音乐主音的标志性奏法。
  {
    id: "builtin.synth.lead",
    name: "preset.bit.lead.name",
    note: "preset.bit.lead.note",
    category: "8bit",
    params: {
      gain: 0.45, wave: 1, pwm: 0.222, attack: 0.002, decay: 0.25, sustain: 0.6, release: 0.08,
      cutoff: 1.0, resonance: 0.0, filterEnv: 0.0, keyFollow: 0.0,
      // 25% 占空比的脉冲均值为 -0.5：一阶高通（约 41 Hz）按真机那样把直流耦合掉，
      // 否则包络一起伏就是「砰」的一声，还白吃动态余量。
      hpf: 0.15,
      lfoRate: 0.8, lfoDelay: 0.35, lfoPitch: 0.16,
    },
  },
  {
    id: "builtin.synth.bit.chord",
    name: "preset.bit.chord.name",
    note: "preset.bit.chord.note",
    category: "8bit",
    params: {
      // 50% 占空比（标准方波）+ 轻微失谐：两个脉冲通道齐奏的厚度。
      gain: 0.5, wave: 1, pwm: 0.5, detune: 0.15, attack: 0.002, decay: 0.12, sustain: 0.3,
      release: 0.06, cutoff: 1.0, resonance: 0.0, filterEnv: 0.0, keyFollow: 0.0,
    },
  },
  {
    id: "builtin.synth.bit.arp",
    name: "preset.bit.arp.name",
    note: "preset.bit.arp.note",
    category: "8bit",
    params: {
      // 12.5% 占空比：最薄最「芯片」的那个音色，配极短包络（约 3 帧）做琶音。
      gain: 0.45, wave: 1, pwm: 0.083, attack: 0.001, decay: 0.05, sustain: 0.0, release: 0.03,
      cutoff: 1.0, resonance: 0.0, filterEnv: 0.0, keyFollow: 0.0,
      // 12.5% 占空比直流更大（均值 -0.75），同样耦合掉。
      hpf: 0.15,
    },
  },
  {
    id: "builtin.synth.bit.tri",
    name: "preset.bit.tri.name",
    note: "preset.bit.tri.note",
    category: "8bit",
    params: {
      // 三角波通道：NES 的低音与旋律主干，长持续、无滤波。
      gain: 0.6, wave: 3, attack: 0.002, decay: 0.4, sustain: 0.85, release: 0.1,
      cutoff: 1.0, resonance: 0.0, filterEnv: 0.0, keyFollow: 0.0,
    },
  },

  // ---- 电钢琴（第二类）----
  //
  // 依据 DX7 的 **E.PIANO 1**（ROM1A）：算法 5，两组「载波 + 调制器」，频率比 **1:1**（琴体）
  // 与 **14:1**（音头那一下金属味的 tine）。公开资料里的具体数值多是**建模重建**而非文档记录
  // （见 musicproductionwiki 那篇自己也这么声明），所以这里按**结构**来搭，不假装逐值照抄：
  // 正弦载波 + FM、比值 1 或 14、长衰减 + 低延音（电钢的「越按越弱」）。
  //
  // 无法复现的部分要说清楚：本合成器的 FM 调制指数**不随时间衰减**，而真实 E.PIANO 1 的
  // 调制器包络是快速衰减的（那正是「音头亮一下、随后变暖」的来源）。这里的近似是
  // 「调制器全程常开」——音色会比真机更亮、更「电」，所以比值给得克制。
  {
    id: "builtin.synth.ep.rhodes",
    name: "preset.ep.rhodes.name",
    note: "preset.ep.rhodes.note",
    category: "epiano",
    params: {
      // 1:1 那对：琴体。长衰减 + 低延音是电钢的性格。
      gain: 0.7, wave: 2, attack: 0.004, decay: 1.6, sustain: 0.3, release: 0.35,
      cutoff: 0.88, resonance: 0.0, filterEnv: 0.0, keyFollow: 0.3,
      fm: 0.3, fmRatio: 1.0,
    },
  },
  {
    id: "builtin.synth.ep.tine",
    name: "preset.ep.tine.name",
    note: "preset.ep.tine.note",
    category: "epiano",
    params: {
      // 14:1 那对：音头。指数刻意压小，否则常开的高比值调制会变成刺耳的边。
      gain: 0.62, wave: 2, attack: 0.002, decay: 1.2, sustain: 0.2, release: 0.3,
      cutoff: 0.9, resonance: 0.0, filterEnv: 0.0, keyFollow: 0.3,
      fm: 0.12, fmRatio: 14.0,
    },
  },
  {
    id: "builtin.synth.ep.chorus",
    name: "preset.ep.chorus.name",
    note: "preset.ep.chorus.note",
    category: "epiano",
    params: {
      gain: 0.65, wave: 2, detune: 0.25, attack: 0.006, decay: 2.0, sustain: 0.35,
      release: 0.4, cutoff: 0.9, resonance: 0.0, filterEnv: 0.0, keyFollow: 0.3,
      fm: 0.35, fmRatio: 1.0,
    },
  },
  {
    id: "builtin.synth.ep.dx",
    name: "preset.ep.dx.name",
    note: "preset.ep.dx.note",
    category: "epiano",
    params: {
      // 3:1：介于琴体与铃音之间，80 年代那种透亮的 FM 电钢。
      gain: 0.66, wave: 2, attack: 0.003, decay: 1.4, sustain: 0.25, release: 0.3,
      cutoff: 0.92, resonance: 0.0, filterEnv: 0.0, keyFollow: 0.3,
      fm: 0.24, fmRatio: 3.0,
    },
  },

  // ---- 吉他 ----
  {
    id: "builtin.synth.guitar.acoustic",
    name: "preset.guitar.acoustic.name",
    note: "preset.guitar.acoustic.note",
    category: "guitar",
    params: {
      // 拨弦的起振噪声 + 快速衰减到低延音；截止约 4.5 kHz。
      gain: 0.72, wave: 0, attack: 0.001, decay: 0.6, sustain: 0.12, release: 0.3,
      cutoff: 0.72, resonance: 0.1, filterEnv: 0.4, keyFollow: 0.35, noise: 0.15,
      pitchEnv: 0.08, pitchDecay: 0.05,
    },
  },
  {
    id: "builtin.synth.guitar.electric",
    name: "preset.guitar.electric.name",
    note: "preset.guitar.electric.note",
    category: "guitar",
    params: {
      // 清音：截止收到约 2.5 kHz，配一点共振（拾音器的峰）。
      gain: 0.62, wave: 0, attack: 0.002, decay: 0.8, sustain: 0.3, release: 0.35,
      cutoff: 0.65, resonance: 0.28, filterEnv: 0.3, keyFollow: 0.3, noise: 0.08,
    },
  },
  {
    id: "builtin.synth.guitar.mute",
    name: "preset.guitar.mute.name",
    note: "preset.guitar.mute.note",
    category: "guitar",
    params: {
      // 闷音：极短的衰减 + 低截止（约 1.6 kHz），拨片噪声更明显。
      gain: 0.6, wave: 0, attack: 0.001, decay: 0.1, sustain: 0.0, release: 0.05,
      cutoff: 0.62, resonance: 0.2, filterEnv: 0.35, keyFollow: 0.3, noise: 0.22,
    },
  },
  {
    id: "builtin.synth.guitar.harmonic",
    name: "preset.guitar.harmonic.name",
    note: "preset.guitar.harmonic.note",
    category: "guitar",
    params: {
      // 泛音：正弦为主、长衰减、几乎不留延音；FM 只给一点点金属边。
      gain: 0.5, wave: 2, attack: 0.001, decay: 1.2, sustain: 0.05, release: 0.4,
      cutoff: 0.85, resonance: 0.1, filterEnv: 0.1, keyFollow: 0.3,
      fm: 0.18, fmRatio: 3.5,
    },
  },

  // ---- 贝斯 ----
  {
    id: "builtin.synth.bass",
    name: "preset.bass.name",
    note: "preset.bass.note",
    category: "bass",
    params: {
      // 指弹贝斯：截止约 900 Hz（贝斯的能量就在低频），包络推一点起振的「嗡」。
      gain: 0.75, wave: 0, sub: 0.35, attack: 0.005, decay: 0.5, sustain: 0.4, release: 0.12,
      cutoff: 0.45, resonance: 0.15, filterEnv: 0.35, keyFollow: 0.3, noise: 0.08,
    },
  },
  {
    id: "builtin.synth.bass.sub",
    name: "preset.bass.sub.name",
    note: "preset.bass.sub.note",
    category: "bass",
    params: {
      // 合成低音：正弦主体 + 大量低八度方波，几乎不要高通音色。
      gain: 0.6, wave: 2, sub: 0.7, attack: 0.008, decay: 0.6, sustain: 0.7, release: 0.15,
      cutoff: 0.42, resonance: 0.0, filterEnv: 0.1, keyFollow: 0.2,
    },
  },
  {
    id: "builtin.synth.bass.pick",
    name: "preset.bass.pick.name",
    note: "preset.bass.pick.note",
    category: "bass",
    params: {
      // 拨片：起振更硬（攻击噪声 + 高一点的包络量），截止约 1.6 kHz。
      gain: 0.7, wave: 0, sub: 0.2, attack: 0.002, decay: 0.35, sustain: 0.25, release: 0.1,
      cutoff: 0.55, resonance: 0.3, filterEnv: 0.45, keyFollow: 0.3, noise: 0.1,
    },
  },
  {
    id: "builtin.synth.bass.fm",
    name: "preset.bass.fm.name",
    note: "preset.bass.fm.note",
    category: "bass",
    params: {
      // FM 贝斯：2:1 的调制给低音一点金属质地，低八度补重量。
      gain: 0.62, wave: 2, sub: 0.4, attack: 0.004, decay: 0.5, sustain: 0.35, release: 0.12,
      cutoff: 0.5, resonance: 0.0, filterEnv: 0.2, keyFollow: 0.25,
      fm: 0.25, fmRatio: 2.0,
    },
  },

  // ---- 鼓 ----
  //
  // 参照 TR-808 的经典合成配方（Bridging 的电路分析那一类资料；具体数字是各家建模的重建值）：
  // 底鼓 = 正弦，音高从约 60 Hz 掉到 40 Hz（约 30 ms）+ 幅度衰减约 300–500 ms；
  // 军鼓 = 两个失谐正弦（约 180 / 330 Hz）叠噪声，音头短；闭镲 = 高通噪声，衰减极短（约 50 ms）。
  {
    id: "builtin.synth.drums",
    name: "preset.drum.kick.name",
    note: "preset.drum.kick.note",
    category: "drums",
    params: {
      // 音高包络 0.35 ≈ 8.4 半音（65 Hz → 约 40 Hz），扫描时长 pitchDecay 0.3 ≈ 31 ms。
      gain: 0.85, wave: 2, attack: 0.0, decay: 0.35, sustain: 0.0, release: 0.05,
      cutoff: 1.0, resonance: 0.0, filterEnv: 0.0, keyFollow: 0.0,
      pitchEnv: 0.35, pitchDecay: 0.3,
    },
  },
  {
    id: "builtin.synth.drums.snare",
    name: "preset.drum.snare.name",
    note: "preset.drum.snare.note",
    category: "drums",
    params: {
      // 噪声占大头（0.6），三角波给出约 180–330 Hz 的鼓皮音，**音高包络**是那记「裂」：
      // 0.2 ≈ 4.8 半音、pitchDecay 0.12 ≈ 10 ms 往下掉。低通约 4 kHz，一阶高通 53 Hz 去掉隆隆声。
      gain: 0.8, wave: 3, noise: 0.6, attack: 0.0, decay: 0.16, sustain: 0.0, release: 0.06,
      cutoff: 0.75, resonance: 0.15, filterEnv: 0.2, keyFollow: 0.0, hpf: 0.2,
      pitchEnv: 0.2, pitchDecay: 0.12,
    },
  },
  {
    id: "builtin.synth.drums.hat",
    name: "preset.drum.hat.name",
    note: "preset.drum.hat.note",
    category: "drums",
    params: {
      // 纯噪声 + 高通（滤波类型 2 = 高通，截止约 8.5 kHz）+ 一阶高通拉满 = 又薄又亮。
      gain: 0.45, wave: 0, noise: 1.0, attack: 0.0, decay: 0.045, sustain: 0.0, release: 0.02,
      cutoff: 0.9, resonance: 0.0, filterEnv: 0.0, keyFollow: 0.0, hpf: 1.0, filterType: 2,
    },
  },
  {
    id: "builtin.synth.drums.tom",
    name: "preset.drum.tom.name",
    note: "preset.drum.tom.note",
    category: "drums",
    params: {
      gain: 0.85, wave: 2, attack: 0.0, decay: 0.4, sustain: 0.0, release: 0.08,
      cutoff: 1.0, resonance: 0.0, filterEnv: 0.0, keyFollow: 0.0,
      pitchEnv: 0.25, pitchDecay: 0.25,
    },
  },

  // ---- 弦乐 ----
  {
    id: "builtin.synth.strings",
    name: "preset.strings.name",
    note: "preset.strings.note",
    category: "strings",
    params: {
      // 弦乐合奏：多个振荡器轻微失谐（0.35）撑出群感，慢起音 + 长释音。
      gain: 0.55, wave: 0, detune: 0.35, attack: 0.18, decay: 1.5, sustain: 0.85, release: 0.5,
      cutoff: 0.72, resonance: 0.08, filterEnv: 0.1, keyFollow: 0.3,
    },
  },
  {
    id: "builtin.synth.strings.solo",
    name: "preset.strings.solo.name",
    note: "preset.strings.solo.note",
    category: "strings",
    params: {
      // 独奏小提琴：起音比合奏快一点，带延迟颤音（弓弦的揉弦）。
      gain: 0.6, wave: 0, attack: 0.06, decay: 1.2, sustain: 0.8, release: 0.25,
      cutoff: 0.78, resonance: 0.1, filterEnv: 0.15, keyFollow: 0.4,
      lfoRate: 0.85, lfoDelay: 0.4, lfoPitch: 0.1,
    },
  },
  {
    id: "builtin.synth.strings.cello",
    name: "preset.strings.cello.name",
    note: "preset.strings.cello.note",
    category: "strings",
    params: {
      // 大提琴：更暗（截止约 2 kHz）、起音更慢、揉弦更宽。
      gain: 0.62, wave: 0, attack: 0.09, decay: 1.3, sustain: 0.82, release: 0.3,
      cutoff: 0.66, resonance: 0.1, filterEnv: 0.12, keyFollow: 0.35,
      lfoRate: 0.8, lfoDelay: 0.5, lfoPitch: 0.09,
    },
  },
  {
    id: "builtin.synth.strings.pizz",
    name: "preset.strings.pizz.name",
    note: "preset.strings.pizz.note",
    category: "strings",
    params: {
      // 拨弦（pizzicato）：极短、不留延音，包络推一点让起振有「啪」。
      gain: 0.6, wave: 0, attack: 0.001, decay: 0.25, sustain: 0.0, release: 0.12,
      cutoff: 0.75, resonance: 0.15, filterEnv: 0.3, keyFollow: 0.35,
    },
  },

  // ---- 管乐 ----
  {
    id: "builtin.synth.winds.flute",
    name: "preset.winds.flute.name",
    note: "preset.winds.flute.note",
    category: "winds",
    params: {
      // 长笛：接近正弦/三角（谐波极少），吹气噪声不可少，带一点延迟颤音。
      gain: 0.6, wave: 3, noise: 0.12, attack: 0.05, decay: 0.6, sustain: 0.85, release: 0.15,
      cutoff: 0.7, resonance: 0.0, filterEnv: 0.05, keyFollow: 0.3,
      lfoRate: 0.82, lfoDelay: 0.4, lfoPitch: 0.06,
    },
  },
  {
    id: "builtin.synth.winds.clarinet",
    name: "preset.winds.clarinet.name",
    note: "preset.winds.clarinet.note",
    category: "winds",
    params: {
      // 单簧管：**只有奇次谐波**，正是方波的频谱结构（50% 占空比），所以用方波。
      gain: 0.55, wave: 1, pwm: 0.5, attack: 0.045, decay: 0.5, sustain: 0.85, release: 0.12,
      cutoff: 0.62, resonance: 0.1, filterEnv: 0.08, keyFollow: 0.3,
    },
  },
  {
    id: "builtin.synth.winds.sax",
    name: "preset.winds.sax.name",
    note: "preset.winds.sax.note",
    category: "winds",
    params: {
      gain: 0.55, wave: 0, noise: 0.08, attack: 0.04, decay: 0.6, sustain: 0.8, release: 0.15,
      cutoff: 0.66, resonance: 0.18, filterEnv: 0.15, keyFollow: 0.3,
    },
  },
  {
    id: "builtin.synth.winds.brass",
    name: "preset.winds.brass.name",
    note: "preset.winds.brass.note",
    category: "winds",
    params: {
      // 铜管：慢起音（0.08 s）+ 滤波包络扫上去，就是那记「嗷」的由来。
      gain: 0.55, wave: 0, attack: 0.08, decay: 0.7, sustain: 0.8, release: 0.18,
      cutoff: 0.6, resonance: 0.12, filterEnv: 0.35, keyFollow: 0.3,
    },
  },

  // ---- 铺底 ----
  {
    id: "builtin.synth.pad",
    name: "preset.pad.name",
    note: "preset.pad.note",
    category: "pad",
    params: {
      // 暖垫：慢起慢落 + 大幅失谐 + 极慢的滤波起伏。
      gain: 0.55, wave: 0, detune: 0.4, attack: 0.35, decay: 2.0, sustain: 0.8, release: 0.9,
      cutoff: 0.6, resonance: 0.1, filterEnv: 0.12, keyFollow: 0.2,
      lfoRate: 0.25, lfoFilter: 0.15,
    },
  },
  {
    id: "builtin.synth.pad.strings",
    name: "preset.pad.strings.name",
    note: "preset.pad.strings.note",
    category: "pad",
    params: {
      gain: 0.5, wave: 0, detune: 0.5, attack: 0.5, decay: 2.0, sustain: 0.85, release: 1.2,
      cutoff: 0.65, resonance: 0.08, filterEnv: 0.1, keyFollow: 0.25,
    },
  },
  {
    id: "builtin.synth.pad.choir",
    name: "preset.pad.choir.name",
    note: "preset.pad.choir.note",
    category: "pad",
    params: {
      // 合唱垫：更多失谐 + 一点噪声当「气声」，滤波缓慢摆动。
      gain: 0.58, wave: 0, detune: 0.6, noise: 0.06, attack: 0.6, decay: 2.0, sustain: 0.8,
      release: 1.0, cutoff: 0.68, resonance: 0.1, filterEnv: 0.1, keyFollow: 0.25,
      lfoRate: 0.3, lfoFilter: 0.2,
    },
  },
  {
    id: "builtin.synth.pad.glass",
    name: "preset.pad.glass.name",
    note: "preset.pad.glass.note",
    category: "pad",
    params: {
      gain: 0.5, wave: 3, attack: 0.3, decay: 2.0, sustain: 0.6, release: 1.0,
      cutoff: 0.85, resonance: 0.0, filterEnv: 0.05, keyFollow: 0.3,
      fm: 0.2, fmRatio: 3.0,
    },
  },

  // ---- 音效 ----
  {
    id: "builtin.synth.fx.blip",
    name: "preset.fx.blip.name",
    note: "preset.fx.blip.note",
    category: "fx",
    params: {
      // 激光：正弦 + 极快的下行音高扫（0.7 ≈ 17 半音，扫完约 24 ms）。
      gain: 0.7, wave: 2, attack: 0.0, decay: 0.25, sustain: 0.0, release: 0.05,
      cutoff: 1.0, resonance: 0.0, filterEnv: 0.0, keyFollow: 0.0,
      pitchEnv: 0.7, pitchDecay: 0.12,
    },
  },
  {
    id: "builtin.synth.fx.boom",
    name: "preset.fx.boom.name",
    note: "preset.fx.boom.note",
    category: "fx",
    params: {
      // 爆炸：纯噪声 + 低通（截止约 1.6 kHz）扫出闷响的长尾巴。
      gain: 0.7, wave: 0, noise: 1.0, attack: 0.0, decay: 0.9, sustain: 0.0, release: 0.15,
      cutoff: 0.62, resonance: 0.2, filterEnv: 0.4, keyFollow: 0.0,
    },
  },
  {
    id: "builtin.synth.fx.riser",
    name: "preset.fx.riser.name",
    note: "preset.fx.riser.note",
    category: "fx",
    params: {
      // 上升音：音高包络拉满（+24 半音）慢慢扫上去，同时滤波也打开。
      gain: 0.5, wave: 0, attack: 0.0, decay: 1.2, sustain: 0.5, release: 0.3,
      cutoff: 0.55, resonance: 0.3, filterEnv: 0.5, keyFollow: 0.0,
      lfoRate: 0.6, lfoFilter: 0.4,
      pitchEnv: 0.5, pitchDecay: 1.0,
    },
  },
  {
    id: "builtin.synth.fx.wobble",
    name: "preset.fx.wobble.name",
    note: "preset.fx.wobble.note",
    category: "fx",
    params: {
      // 噪扫：噪声 + 高共振 + LFO 扫滤波，dubstep 那种「哇」。
      gain: 0.5, wave: 0, noise: 0.4, attack: 0.01, decay: 0.8, sustain: 0.7, release: 0.2,
      cutoff: 0.55, resonance: 0.6, filterEnv: 0.3, keyFollow: 0.2,
      lfoRate: 0.55, lfoFilter: 0.6,
    },
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
  [SYNTH_ID]: ["cutoff", "resonance", "attack", "decay", "sustain", "release", "detune", "lfoRate"],
};

/**
 * 某插件**默认绑定**的目标键（**只给参数 id**，`inst:` / `fx<i>:` 前缀由调用方拼）。
 *
 * 这就是「精选集」（`COMMON_INSTRUMENT_PARAMS`）——内置合成器 8 个，其他插件空。
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

/** 内置插件的参数表（音源 + 效果器都要预填，界面的通用参数列表按 id 取）。 */
export const BUILTIN_PARAMS: Record<string, ParamSpec[]> = {
  [SYNTH_ID]: SYNTH_PARAMS,
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
