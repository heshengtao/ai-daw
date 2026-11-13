/**
 * 浏览器端的实时监听开关：播放时连上 `/audio` 收 PCM，暂停时断开。
 *
 * 桌面端不启用——声音由本机声卡直接出，再叠一路浏览器播放只会双重发声。
 */
import { onUnmounted, watch } from "vue";
import { WebMonitor } from "@/api/audio";
import { useTransportStore } from "@/stores/transport";
import { isTauri } from "@/utils/platform";

export function useWebMonitor(): void {
  if (isTauri()) return;
  const transport = useTransportStore();
  const monitor = new WebMonitor();

  watch(
    () => transport.playing,
    (playing) => {
      if (playing) {
        void monitor.start();
      } else {
        monitor.stop();
      }
    },
    { immediate: true },
  );

  // iOS Safari：切后台会挂起 AudioContext，回来必须显式恢复。
  const onVisible = () => {
    if (document.visibilityState === "visible") void monitor.resume();
  };
  document.addEventListener("visibilitychange", onVisible);
  onUnmounted(() => {
    document.removeEventListener("visibilitychange", onVisible);
    monitor.stop();
  });
}
