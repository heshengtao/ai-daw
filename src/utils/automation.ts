/**
 * 自动化目标的**唯一来源**：编排视图的自动化车道与钢琴卷帘下方的参数车道共用它。
 *
 * 键的格式见 `specs/2026-10-10-plugin-param-automation-design.md` §4：
 * `gain_db` / `pan` / `inst:<param_id>` / `fx<i>:<param_id>`（`i` 从 **0** 起；
 * 下拉里的分组标题从 1 起显示，换算只在这里做一次）。
 */
import type { AutomationPoint, Track } from "@/api/types";
import {
  canonicalPluginId,
  defaultBoundParamIds,
  type ParamSpec,
  SYNTH_ID,
  SYNTH_PARAM_HINTS,
  SYNTH_PARAM_LABELS,
} from "@/data/plugins";

/** 一条可自动化的目标（下拉项 + 值域）。 */
export interface AutomationTarget {
  /** 写进 `Track.automation[].param` 的键。 */
  key: string;
  /** 显示名（已翻译）。 */
  label: string;
  /** 下拉里的分组标题（已翻译）。 */
  group: string;
  min: number;
  max: number;
  step: number;
  digits: number;
  unit?: string;
  /**
   * 一句话说明（构建时**已本地化**，和 `label` 一样）。
   *
   * 内置目标（增益 / 声像）与合成器精选参数走手写文案；插件参数自动拼
   * 「名称 · 单位 · 范围（默认 x）」。`velocity` 不在本列表里——它是逐音符属性，车道表头
   * 只留一行「自动化 + 目标 + 网格」，不再显示说明文字。
   */
  hint?: string;
  /**
   * 插件是否**声明**这个参数可自动化（缺省视为可自动化）。
   *
   * `false` 的目标会在参数页里**可见但按钮禁用**（要能解释「为什么它不能自动化」）。注意
   * `curateTargets` **不**据它过滤：已绑定的、已经有曲线的照常收下——MCP 的 `set_automation`
   * 接受任何键，那里可能已经有曲线；参数页那颗禁用按钮才是守卫，车道下拉不是。
   */
  automatable?: boolean;
}

type Translate = (key: string, vars?: Record<string, string | number>) => string;
type ParamsOf = (pluginId: string | null | undefined) => ParamSpec[];

/**
 * 某插件的参数：**永远是全量**——这是参数页与车道下拉共用的唯一口径（设计 §4.2）。
 *
 * 精选表之外的参数（例如合成器的 `wave` / `lfoPitch`）也在这里，"插件声明不可自动化"的也在
 * （在目标上标 `automatable: false`，参数页把它们的按钮禁用）。给车道下拉看的收窄是
 * `curateTargets` 的事，与这里无关；精选表剩下的唯一用处是经 `defaultBoundKeys` 当**默认绑定集**。
 */
function paramsFor(pluginId: string, paramsOf: ParamsOf): ParamSpec[] {
  return paramsOf(canonicalPluginId(pluginId));
}

/** 参数显示名：内置合成器走 i18n（英文界面才不会是中文名），其余用插件自报的名字。 */
function paramLabel(pluginId: string, spec: ParamSpec, t: Translate): string {
  const key = SYNTH_PARAM_LABELS[spec.id];
  return canonicalPluginId(pluginId) === SYNTH_ID && key ? t(key) : spec.name;
}

function fromSpec(
  key: string,
  label: string,
  group: string,
  spec: ParamSpec,
  hint: string,
): AutomationTarget {
  return {
    key,
    label,
    group,
    min: spec.min,
    max: spec.max,
    step: spec.step,
    digits: spec.digits ?? 2,
    unit: spec.unit,
    hint,
    automatable: spec.automatable !== false,
  };
}

/** 插件参数的自动说明：「名称 · 单位 · 范围（默认 x）」（没有单位就不带那一段）。 */
function pluginHint(label: string, spec: ParamSpec, t: Translate): string {
  const vars = {
    name: label,
    unit: spec.unit ?? "",
    min: spec.min,
    max: spec.max,
    default: spec.default,
  };
  return spec.unit ? t("param.hint.plugin", vars) : t("param.hint.pluginNoUnit", vars);
}

/** 目标的说明：合成器**精选参数**用手写文案，其余插件参数自动拼。 */
function specHint(pluginId: string, label: string, spec: ParamSpec, t: Translate): string {
  const key = SYNTH_PARAM_HINTS[spec.id];
  if (canonicalPluginId(pluginId) === SYNTH_ID && key) return t(key);
  return pluginHint(label, spec, t);
}

/** 常用 MIDI CC：键 `cc:<编号>`，值与 MIDI 一致（0..127）。 */
const MIDI_CC_TARGETS: ReadonlyArray<{ cc: number; label: string; hint: string }> = [
  { cc: 1, label: "auto.cc.mod", hint: "param.hint.cc.mod" },
  { cc: 2, label: "auto.cc.breath", hint: "param.hint.cc.breath" },
  { cc: 11, label: "auto.cc.expression", hint: "param.hint.cc.expression" },
  { cc: 64, label: "auto.cc.sustain", hint: "param.hint.cc.sustain" },
  { cc: 74, label: "auto.cc.brightness", hint: "param.hint.cc.brightness" },
];

/**
 * 一条轨道当前可自动化的全部目标，顺序：轨道混音 → MIDI CC → 乐器 → 效果器 1..n。
 *
 * **唯一口径就是"全量"**（设计 §4.2）：每个插件的**全部**参数都在，含精选表之外的，含插件声明
 * 不可自动化的（后者在目标上标 `automatable: false`）。参数页要用它列出一切；给车道下拉看的
 * 收窄是 `curateTargets` 的事。
 *
 * `paramsOf` 传 `usePluginsStore().paramsOf`（内置已就绪、外部插件可能还没加载完，
 * 调用方负责对乐器与每个效果器调用一次 `ensureParams`）。
 */
export function automationTargetsFor(
  track: Track | null,
  paramsOf: ParamsOf,
  t: Translate,
): AutomationTarget[] {
  if (!track) return [];
  const out: AutomationTarget[] = [
    { key: "gain_db", label: t("auto.gain"), group: t("auto.group.track"), min: -60, max: 12, step: 0.5, digits: 1, unit: "dB", hint: t("param.hint.gain_db"), automatable: true },
    { key: "pan", label: t("auto.pan"), group: t("auto.group.track"), min: -1, max: 1, step: 0.05, digits: 2, hint: t("param.hint.pan"), automatable: true },
  ];

  // 常用 CC 永远在列表里（车道下拉与参数页共用这份口径）。
  for (const cc of MIDI_CC_TARGETS) {
    out.push({
      key: `cc:${cc.cc}`,
      label: t(cc.label),
      group: t("auto.group.midi"),
      min: 0,
      max: 127,
      step: 1,
      digits: 0,
      hint: t(cc.hint),
      automatable: true,
    });
  }
  // 触后只有一条默认车道（通道 0），和常规 DAW 的 Aftertouch 一致。
  // MPE 的成员通道车道（`at:1`…`at:15`）如果工程里已经有，会在下面自动补进下拉。
  out.push({
    key: "at:0",
    label: t("auto.at.aftertouch"),
    group: t("auto.group.midi"),
    min: 0,
    max: 127,
    step: 1,
    digits: 0,
    hint: t("auto.at.hint"),
    automatable: true,
  });
  // MCP 可能写了常用表之外的 CC / 触后通道：把它补进列表，否则车道下拉里选不到已存在的曲线。
  for (const lane of track.automation ?? []) {
    const match = /^cc:(\d+)$/.exec(lane.param);
    if (match) {
      const n = Number(match[1]);
      if (n > 127 || out.some((target) => target.key === lane.param)) continue;
      out.push({
        key: lane.param,
        label: t("auto.cc.generic", { n }),
        group: t("auto.group.midi"),
        min: 0,
        max: 127,
        step: 1,
        digits: 0,
        automatable: true,
      });
      continue;
    }
    const at = /^at:(\d+)$/.exec(lane.param);
    if (at) {
      const n = Number(at[1]);
      if (n > 15 || out.some((target) => target.key === lane.param)) continue;
      out.push({
        key: lane.param,
        label: t("auto.at.channel", { n: n + 1 }),
        group: t("auto.group.midi"),
        min: 0,
        max: 127,
        step: 1,
        digits: 0,
        hint: t("auto.at.hint"),
        automatable: true,
      });
    }
  }

  const instrumentId = track.instrument?.plugin_id;
  if (instrumentId) {
    for (const spec of paramsFor(instrumentId, paramsOf)) {
      const label = paramLabel(instrumentId, spec, t);
      out.push(fromSpec(`inst:${spec.id}`, label, t("auto.group.instrument"), spec, specHint(instrumentId, label, spec, t)));
    }
  }

  track.effects.forEach((fx, i) => {
    const group = t("auto.group.effect", { n: i + 1 });
    for (const spec of paramsFor(fx.plugin_id, paramsOf)) {
      const label = paramLabel(fx.plugin_id, spec, t);
      out.push(fromSpec(`fx${i}:${spec.id}`, label, group, spec, specHint(fx.plugin_id, label, spec, t)));
    }
  });

  return out;
}

/** 折线求值（镜像引擎 `lane_value_at`：相邻点线性插值、两端夹住；空返回 `null`）。 */
export function laneValueAtOrNull(points: AutomationPoint[], beat: number): number | null {
  const first = points[0];
  if (!first) return null;
  if (beat <= first.beat) return first.value;
  const last = points[points.length - 1]!;
  if (beat >= last.beat) return last.value;
  for (let i = 0; i + 1 < points.length; i += 1) {
    const a = points[i]!;
    const b = points[i + 1]!;
    if (beat >= a.beat && beat <= b.beat) {
      const span = b.beat - a.beat;
      if (span <= 1e-9) return b.value;
      return a.value + (b.value - a.value) * ((beat - a.beat) / span);
    }
  }
  return last.value;
}

/** 该参数在这条轨道上是否**已经有曲线**（「已用」判定：车道存在且至少一个点）。 */
export function laneHasPoints(track: Track | null | undefined, key: string): boolean {
  return (track?.automation ?? []).some((l) => l.param === key && l.points.length > 0);
}

/**
 * 一条轨道**默认绑定**的目标键：把乐器（`inst:<id>`）与每个效果器（`fx<i>:<id>`）的
 * 精选参数 id 拼上前缀（`defaultBoundParamIds` 只给 id）。
 *
 * `curateTargets` 的 `fallbackBound` 输入就是它——Task 3 直接调用，不要另写一份拼前缀的逻辑。
 */
export function defaultBoundKeys(track: Track | null | undefined): string[] {
  if (!track) return [];
  const out: string[] = [];
  const instrumentId = track.instrument?.plugin_id;
  if (instrumentId) {
    for (const id of defaultBoundParamIds(instrumentId)) out.push(`inst:${id}`);
  }
  track.effects.forEach((fx, i) => {
    for (const id of defaultBoundParamIds(fx.plugin_id)) out.push(`fx${i}:${id}`);
  });
  return out;
}

/**
 * 收窄给车道下拉用的目标列表：增益 / 声像 + 已绑定 + **已经有曲线的**。
 *
 * 规则（设计 §3.3）：
 * 1. `gain_db` / `pan` 永远在最前（按 `all` 里的原始顺序）。
 * 2. 然后 `bound ?? fallbackBound` 中能在 `all` 里找到的键（**保持绑定顺序**；表里不存在的键忽略）。
 * 3. 最后 `usedKeys` 里还没出现的键（已用但已解绑）——同样要在 `all` 里存在。
 * 4. 目标对象原样透传（含 `hint`）。
 *
 * `bound === null` 表示「没有显式记录」→ 用 `fallbackBound`（默认精选）；
 * `bound === []` 表示「用户显式全解绑」→ **只剩增益 / 声像**，绝不回落默认。
 */
export function curateTargets(
  all: AutomationTarget[],
  opts: { bound: string[] | null; fallbackBound: string[]; usedKeys: string[] },
): AutomationTarget[] {
  const byKey = new Map(all.map((target) => [target.key, target]));
  const out: AutomationTarget[] = [];
  const seen = new Set<string>();

  /** 按 `all` 里的目标原样收下（表里没有的键忽略、重复的忽略）。 */
  const push = (key: string): void => {
    if (seen.has(key)) return;
    const target = byKey.get(key);
    if (!target) return;
    seen.add(key);
    out.push(target);
  };

  for (const target of all) {
    if (
      target.key === "gain_db" ||
      target.key === "pan" ||
      target.key.startsWith("cc:") ||
      target.key.startsWith("at:")
    ) {
      push(target.key);
    }
  }
  for (const key of opts.bound ?? opts.fallbackBound) push(key);
  for (const key of opts.usedKeys) push(key);

  return out;
}
