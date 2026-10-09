/**
 * 让二级窗口（内置合成器 / 内置效果器）跟工程保持一致。
 *
 * 这些窗口是独立的 Vue 实例，各自只加载一份工程快照。主窗口改动（换效果器顺序、切预设
 * 换掉整条效果链……）只通过 `engine://event` 广播出去，如果不重新拉取，二级窗口就会一直
 * 显示旧状态——「这条轨道上没有这个效果器」正是这么来的。这里订阅事件并**节流地**重新
 * 拉取工程快照，把窗口拉回真实状态。
 */
import { onMounted, onUnmounted } from "vue";
import { onEngineEvent } from "@/api/tauri";
import { useProjectStore } from "@/stores/project";

/**
 * @param intervalMs 事件节流窗口：连续事件只在安静这么久之后拉一次（拖旋钮时会连着发事件）。
 */
export function useProjectSync(intervalMs = 80): void {
  const store = useProjectStore();
  let timer: number | null = null;
  let unlisten: (() => void) | null = null;

  onMounted(async () => {
    // 自己确保工程已经载入：二级窗口不指望主窗口的装载链成功。
    if (!store.project) await store.reload();
    unlisten = await onEngineEvent(() => {
      if (timer !== null) return;
      timer = window.setTimeout(() => {
        timer = null;
        void store.reload();
      }, intervalMs);
    });
  });

  onUnmounted(() => {
    if (timer !== null) window.clearTimeout(timer);
    unlisten?.();
  });
}
