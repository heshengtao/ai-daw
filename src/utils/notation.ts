/**
 * 谱式（记谱形态）的公共定义。
 *
 * 一条轨道的谱式由**轨道自己**决定，而不是整个乐谱一刀切：
 * - 总谱里每条轨道按自己的谱式排在系统里（吉他带六线谱、鼓用打击乐谱表）；
 * - 单轨视图 / 下窗口也只是「看这一条」，同样读这里的值。
 *
 * 存储放在 `layout` 的 `trackNotations`（按轨道 id），是**显示偏好**，不进工程文件——
 * 同一份 MIDI 给不同的人看，想要的谱式本来就不同，没必要污染工程数据。
 */
import type { Track } from "@/api/types";

/** 谱式。 */
export type Notation = "score" | "guitar" | "bass" | "drums";

/** 全部谱式（顺序即下拉里的顺序）。 */
export const NOTATIONS: Notation[] = ["score", "guitar", "bass", "drums"];

/** 六线谱 / 四线谱的标准调弦（低 → 高，MIDI 音高）。 */
export const TAB_TUNINGS: Record<"guitar" | "bass", number[]> = {
  guitar: [40, 45, 50, 55, 59, 64],
  bass: [28, 33, 38, 43],
};

/** 谱式 → 六线谱参数；`score` 没有。 */
export function tabSpecFor(notation: Notation): { strings: number; tuning: number[] } | null {
  if (notation === "guitar") return { strings: 6, tuning: TAB_TUNINGS.guitar };
  if (notation === "bass") return { strings: 4, tuning: TAB_TUNINGS.bass };
  return null;
}

/** 谱式是不是六线谱（吉他 / 贝斯）。 */
export function isTabNotation(notation: Notation): boolean {
  return notation === "guitar" || notation === "bass";
}

/**
 * 没手动设置过时，按**乐器**猜一个合适的谱式：
 * 鼓组 → 打击乐谱表；贝斯 → 四线谱；吉他类 → 六线谱；其余 → 五线谱。
 *
 * 只看 `plugin_id` 与 `meta.role`：这两样是工程里已有的信号，不额外加字段。
 */
export function defaultNotationFor(track: Track): Notation {
  const id = (track.instrument?.plugin_id ?? "").toLowerCase();
  const role = (track.meta?.role ?? "").toLowerCase();
  if (role === "drums" || id.includes("drum")) return "drums";
  if (id.includes("bass")) return "bass";
  if (id.includes("guitar")) return "guitar";
  return "score";
}
