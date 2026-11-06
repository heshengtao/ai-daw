/**
 * 应用偏好（与具体工程无关）：新工程默认速度、启动时要不要弹欢迎页。
 *
 * 与 layout / theme / locale 一样存 localStorage；`defaultTempo` 在**新建工程**时传给引擎，
 * 决定新工程的初始 BPM。`welcomeOnStart` 是纯界面偏好：关掉后启动直接进工作区，
 * 下一次启动生效（设置页里改了不会立刻弹 / 关掉当前这个欢迎页）。
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
  /** 启动时弹出欢迎页（默认开）。 */
  welcomeOnStart: boolean;
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
  // 老版本存的数据里没有这个字段：缺省当作「开」，新老用户都从下一次启动开始看到欢迎页。
  const welcomeOnStart = ref(saved.welcomeOnStart !== false);

  watch([defaultTempo, welcomeOnStart], () => {
    try {
      localStorage.setItem(
        KEY,
        JSON.stringify({
          defaultTempo: defaultTempo.value,
          welcomeOnStart: welcomeOnStart.value,
        } satisfies Persisted),
      );
    } catch {
      // 无痕模式 / 存储被禁用：记不住就记不住，不影响本次会话。
    }
  });

  return { defaultTempo, welcomeOnStart };
});
