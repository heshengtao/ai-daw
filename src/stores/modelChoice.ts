import { ref, watch } from "vue";
import { defineStore } from "pinia";

/**
 * **每个 AI 功能上次选的那一档模型**（跨会话记住）。
 *
 * 为什么需要它：清单里常有好几档（fp16 / fp32、4 分轨 / 6 分轨、Q5K / Q6K / Q8 / bf16），
 * 绝大多数人只装其中一档。以前选中项是组件内的 `ref(null)`，再被「优先选已下载」的
 * watcher 一覆盖，用户刚点的选择立刻被弹回第一项——切个页面、重启一次更是全忘。
 *
 * 语义：
 * - **用户显式选过**就以它为准，**不管装没装**（没装就显示「下载并…」，这正是用户要的）；
 * - 只有这一档从清单里彻底消失（后端改名 / 下架）时，才回落到默认规则；
 * - 没选过（没有条目）时保持「已装的优先 → 清单第一个」。
 */
const KEY = "ai-daw-model-choice";

/** 读持久化的选择；坏数据 / 无痕模式都退回空表，不让它影响启动。 */
function read(): Record<string, string> {
  try {
    const raw = localStorage.getItem(KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : null;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    const out: Record<string, string> = {};
    for (const [feature, id] of Object.entries(parsed as Record<string, unknown>)) {
      if (typeof id === "string" && id.length > 0) out[feature] = id;
    }
    return out;
  } catch {
    return {};
  }
}

export const useModelChoiceStore = defineStore("modelChoice", () => {
  /** 功能 → 模型 id。没有条目 = 还没选过。 */
  const map = ref<Record<string, string>>(read());

  watch(
    map,
    (value) => {
      try {
        localStorage.setItem(KEY, JSON.stringify(value));
      } catch {
        // 无痕模式 / 存储被禁用：只影响「记住」，不影响本次使用。
      }
    },
    { deep: true },
  );

  function get(feature: string): string | null {
    return map.value[feature] ?? null;
  }

  function set(feature: string, id: string | null): void {
    const next = { ...map.value };
    if (id) next[feature] = id;
    else delete next[feature];
    map.value = next;
  }

  return { map, get, set };
});
