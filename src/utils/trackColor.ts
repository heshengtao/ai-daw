/** 轨道配色：按索引取一个稳定的色相，用于轨道条与片段，避免界面单调。 */
const HUES = [258, 205, 158, 28, 338, 96];

/** 返回第 index 条轨道的色相（度）。 */
export function trackHue(index: number): number {
  return HUES[((index % HUES.length) + HUES.length) % HUES.length]!;
}
