/** 布局偏好与轻量通知（snackbar）；布局持久化到 localStorage。 */
import { defineStore } from "pinia";
import { ref, watch } from "vue";

/** 界面密度模式。 */
export type LayoutMode = "simple" | "pro";

const KEY = "daw-party-layout";

interface Persisted {
  mode: LayoutMode;
  showTrackHeaders: boolean;
  showInspector: boolean;
  trackWidth: number;
  inspectorWidth: number;
}

function load(): Partial<Persisted> {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "{}") as Partial<Persisted>;
  } catch {
    return {};
  }
}

export const useLayoutStore = defineStore("layout", () => {
  const saved = load();
  const mode = ref<LayoutMode>(saved.mode ?? "simple");
  const showTrackHeaders = ref(saved.showTrackHeaders ?? true);
  const showInspector = ref(saved.showInspector ?? false);
  const trackWidth = ref(saved.trackWidth ?? 220);
  const inspectorWidth = ref(saved.inspectorWidth ?? 280);
  const devDrawer = ref(false);

  /** 当前提示文案（自动消失）。 */
  const toast = ref<string | null>(null);
  let toastTimer: number | null = null;

  watch(
    [mode, showTrackHeaders, showInspector, trackWidth, inspectorWidth],
    () => {
      localStorage.setItem(
        KEY,
        JSON.stringify({
          mode: mode.value,
          showTrackHeaders: showTrackHeaders.value,
          showInspector: showInspector.value,
          trackWidth: trackWidth.value,
          inspectorWidth: inspectorWidth.value,
        } satisfies Persisted),
      );
    },
    { deep: true },
  );

  /** 切换密度模式；简单模式收起检查器。 */
  function setMode(next: LayoutMode): void {
    mode.value = next;
    showInspector.value = next === "pro";
  }

  /** 在简单/专业之间切换。 */
  function togglePro(): void {
    setMode(mode.value === "pro" ? "simple" : "pro");
  }

  /** 开关开发者抽屉。 */
  function toggleDevDrawer(): void {
    devDrawer.value = !devDrawer.value;
  }

  /** 弹出一条提示（3.5 秒后自动消失）。 */
  function notify(message: string): void {
    toast.value = message;
    if (toastTimer !== null) window.clearTimeout(toastTimer);
    toastTimer = window.setTimeout(() => {
      toast.value = null;
      toastTimer = null;
    }, 3500);
  }

  return {
    mode,
    showTrackHeaders,
    showInspector,
    trackWidth,
    inspectorWidth,
    devDrawer,
    toast,
    setMode,
    togglePro,
    toggleDevDrawer,
    notify,
  };
});
