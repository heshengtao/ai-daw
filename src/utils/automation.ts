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
  COMMON_INSTRUMENT_PARAMS,
  type ParamSpec,
  SYNTH_ID,
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
}

type Translate = (key: string, vars?: Record<string, string | number>) => string;
type ParamsOf = (pluginId: string | null | undefined) => ParamSpec[];

/** 某插件**可自动化**的参数：有精选表就只取表里的（按表顺序），否则取全部。 */
function automatableParams(pluginId: string, paramsOf: ParamsOf): ParamSpec[] {
  const id = canonicalPluginId(pluginId);
  const all = paramsOf(id).filter((p) => p.automatable !== false);
  const common = COMMON_INSTRUMENT_PARAMS[id];
  if (!common) return all;
  const byId = new Map(all.map((p) => [p.id, p]));
  return common.map((pid) => byId.get(pid)).filter((p): p is ParamSpec => p !== undefined);
}

/** 参数显示名：内置合成器走 i18n（英文界面才不会是中文名），其余用插件自报的名字。 */
function paramLabel(pluginId: string, spec: ParamSpec, t: Translate): string {
  const key = SYNTH_PARAM_LABELS[spec.id];
  return canonicalPluginId(pluginId) === SYNTH_ID && key ? t(key) : spec.name;
}

function fromSpec(key: string, label: string, group: string, spec: ParamSpec): AutomationTarget {
  return {
    key,
    label,
    group,
    min: spec.min,
    max: spec.max,
    step: spec.step,
    digits: spec.digits ?? 2,
    unit: spec.unit,
  };
}

/**
 * 一条轨道当前可自动化的全部目标，顺序：轨道混音 → 乐器 → 效果器 1..n。
 *
 * `paramsOf` 传 `usePluginsStore().paramsOf`（内置已就绪、外部插件可能还没加载完，
 * 调用方负责对乐器调用一次 `ensureParams`）。
 */
export function automationTargetsFor(
  track: Track | null,
  paramsOf: ParamsOf,
  t: Translate,
): AutomationTarget[] {
  if (!track) return [];
  const out: AutomationTarget[] = [
    { key: "gain_db", label: t("auto.gain"), group: t("auto.group.track"), min: -60, max: 12, step: 0.5, digits: 1, unit: "dB" },
    { key: "pan", label: t("auto.pan"), group: t("auto.group.track"), min: -1, max: 1, step: 0.05, digits: 2 },
  ];

  const instrumentId = track.instrument?.plugin_id;
  if (instrumentId) {
    for (const spec of automatableParams(instrumentId, paramsOf)) {
      out.push(fromSpec(`inst:${spec.id}`, paramLabel(instrumentId, spec, t), t("auto.group.instrument"), spec));
    }
  }

  track.effects.forEach((fx, i) => {
    const group = t("auto.group.effect", { n: i + 1 });
    for (const spec of automatableParams(fx.plugin_id, paramsOf)) {
      out.push(fromSpec(`fx${i}:${spec.id}`, paramLabel(fx.plugin_id, spec, t), group, spec));
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
