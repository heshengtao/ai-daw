/** 引擎事件流：只被开发者抽屉消费，用于观察引擎/未来 AI 的动作。 */
import { defineStore } from "pinia";
import { ref } from "vue";
import { onEngineEvent } from "@/api/tauri";

const MAX_LINES = 200;

export const useEngineEventsStore = defineStore("engineEvents", () => {
  const lines = ref<string[]>([]);
  const lastAt = ref(0);
  let unlisten: (() => void) | null = null;

  /** 开始订阅（幂等）。 */
  async function start(): Promise<void> {
    if (unlisten) return;
    const off = await onEngineEvent((payload) => {
      lines.value.push(JSON.stringify(payload));
      if (lines.value.length > MAX_LINES) lines.value.splice(0, lines.value.length - MAX_LINES);
      lastAt.value = Date.now();
    });
    unlisten = off;
  }

  /** 停止订阅。 */
  function stop(): void {
    unlisten?.();
    unlisten = null;
  }

  /** 清空。 */
  function clear(): void {
    lines.value = [];
  }

  /**
   * 最近 `withinMs` 毫秒内是否有事件流入（用于 AI 状态胶囊的高亮）。
   * 注意：这是渲染时的取值，非响应式；需要响应性的地方请配合定时器使用。
   */
  function recentlyActive(withinMs = 800): boolean {
    return Date.now() - lastAt.value < withinMs;
  }

  return { lines, lastAt, start, stop, clear, recentlyActive };
});
