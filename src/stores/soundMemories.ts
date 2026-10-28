/**
 * 音色记忆库 store：列表、搜索、元数据编辑、删除。
 *
 * 保存只由 AI 触发（MCP），前端不提供保存入口；AI 保存后后端广播
 * `engine://sound-memories`，这里自动刷新。
 */
import { defineStore } from "pinia";
import { computed, ref } from "vue";

import {
  deleteSoundMemory,
  listSoundMemories,
  onSoundMemoriesEvent,
  updateSoundMemory,
} from "@/api/tauri";
import type { SoundMemorySummary } from "@/api/types";

export const useSoundMemoriesStore = defineStore("soundMemories", () => {
  const items = ref<SoundMemorySummary[]>([]);
  const loading = ref(false);
  const query = ref("");
  const activeTag = ref("");
  /** 库里的两类东西：乐器 / 效果链，和人声参考（同一个库，切换看）。 */
  const kind = ref<"instrument" | "voice">("instrument");
  /** 在音色库里点了「用作参考」的人声 id（AI 面板据此选中它并切过去）。 */
  const pickedVoice = ref<string | null>(null);
  /** 选择序号：重复选同一条也要能触发（值本身可能没变）。 */
  const pickSeq = ref(0);

  const tags = computed<string[]>(() => {
    const set = new Set<string>();
    for (const memory of items.value) {
      const isVoice = Boolean(memory.voice);
      if (kind.value === "voice" ? !isVoice : isVoice) continue;
      for (const tag of memory.tags) set.add(tag);
    }
    return [...set].sort();
  });

  const filtered = computed<SoundMemorySummary[]>(() => {
    const q = query.value.trim().toLowerCase();
    return items.value.filter((memory) => {
      const isVoice = Boolean(memory.voice);
      if (kind.value === "voice" ? !isVoice : isVoice) return false;
      if (activeTag.value && !memory.tags.includes(activeTag.value)) return false;
      if (!q) return true;
      const haystack = [
        memory.name,
        memory.description,
        memory.role ?? "",
        memory.instrument_plugin_id ?? "",
        ...memory.tags,
        ...memory.effect_plugin_ids,
      ]
        .join(" ")
        .toLowerCase();
      return haystack.includes(q);
    });
  });

  /** 把人声选作歌声合成的参考（由 AI 面板消费）。 */
  function pickVoice(id: string): void {
    pickedVoice.value = id;
    pickSeq.value += 1;
  }

  /** 取走（并清空）这次「用作参考」的选择：AI 面板只在挂载时消费一次。 */
  function consumeVoicePick(): string | null {
    const id = pickedVoice.value;
    pickedVoice.value = null;
    return id;
  }

  async function refresh(): Promise<void> {
    loading.value = true;
    try {
      items.value = await listSoundMemories();
    } finally {
      loading.value = false;
    }
  }

  async function updateMetadata(
    id: string,
    patch: { name?: string; description?: string; tags?: string[]; role?: string },
  ): Promise<void> {
    await updateSoundMemory(id, patch);
    await refresh();
  }

  async function remove(id: string): Promise<void> {
    await deleteSoundMemory(id);
    await refresh();
  }

  /** 订阅后端变更（返回取消订阅函数）。 */
  function watchChanges(): () => void {
    let stop: (() => void) | null = null;
    void onSoundMemoriesEvent(() => {
      void refresh();
    }).then((off) => {
      stop = off;
    });
    return () => stop?.();
  }

  return {
    items,
    loading,
    query,
    activeTag,
    kind,
    pickedVoice,
    pickSeq,
    pickVoice,
    consumeVoicePick,
    tags,
    filtered,
    refresh,
    updateMetadata,
    remove,
    watchChanges,
  };
});
