/**
 * 布局偏好与轻量通知（snackbar）：面板可见性与宽度，持久化到 localStorage。
 * 面板开关直接由顶栏的两个按钮控制，不再有"简单/专业模式"这层概念。
 */
import { defineStore } from "pinia";
import { ref, watch } from "vue";

const KEY = "daw-party-layout";

interface Persisted {
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
  const showTrackHeaders = ref(saved.showTrackHeaders ?? true);
  const showInspector = ref(saved.showInspector ?? false);
  const trackWidth = ref(saved.trackWidth ?? 220);
  const inspectorWidth = ref(saved.inspectorWidth ?? 280);
  const devDrawer = ref(false);

  /** 当前提示文案（自动消失）。 */
  const toast = ref<string | null>(null);
  let toastTimer: number | null = null;

  watch(
    [showTrackHeaders, showInspector, trackWidth, inspectorWidth],
    () => {
      localStorage.setItem(
        KEY,
        JSON.stringify({
          showTrackHeaders: showTrackHeaders.value,
          showInspector: showInspector.value,
          trackWidth: trackWidth.value,
          inspectorWidth: inspectorWidth.value,
        } satisfies Persisted),
      );
    },
    { deep: true },
  );

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
    showTrackHeaders,
    showInspector,
    trackWidth,
    inspectorWidth,
    devDrawer,
    toast,
    toggleDevDrawer,
    notify,
  };
});
