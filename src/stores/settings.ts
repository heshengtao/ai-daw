/**
 * 应用偏好（与具体工程无关）：目前只有「新工程默认速度」。
 *
 * 与 layout / theme / locale 一样存 localStorage；`defaultTempo` 在**新建工程**时传给引擎，
 * 决定新工程的初始 BPM。
 */
import { defineStore } from "pinia";
import { ref, watch } from "vue";

const KEY = "ai-daw-settings";

/** 与工程的速度范围一致（`project.setTempo` / 引擎都夹在这个区间）。 */
export const MIN_DEFAULT_TEMPO = 20;
export const MAX_DEFAULT_TEMPO = 300;
/** 引擎 `Transport::default()` 的速度，也是设置的兜底值。 */
export const FALLBACK_DEFAULT_TEMPO = 120;

interface Persisted {
  defaultTempo: number;
}

function load(): Partial<Persisted> {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "{}") as Partial<Persisted>;
  } catch {
    return {};
  }
}

function clampTempo(value: number): number {
  if (!Number.isFinite(value)) return FALLBACK_DEFAULT_TEMPO;
  return Math.min(MAX_DEFAULT_TEMPO, Math.max(MIN_DEFAULT_TEMPO, Math.round(value)));
}

export const useSettingsStore = defineStore("settings", () => {
  const saved = load();
  const defaultTempo = ref(clampTempo(saved.defaultTempo ?? FALLBACK_DEFAULT_TEMPO));

  watch(defaultTempo, () => {
    localStorage.setItem(KEY, JSON.stringify({ defaultTempo: defaultTempo.value } satisfies Persisted));
  });

  return { defaultTempo };
});
