/**
 * 「从哪里打开哪个 AI 子页」的意图总线。
 *
 * 右键菜单里每一种 AI 操作都对应一套明确的输入（这一块音频 / 这一段 MIDI / 一条轨道），
 * 但 AI 子页原本只认「当前选中」，从菜单直接切页签时选中态可能还没刷新。这里把意图先
 * 记下来：`AiPanel` 切到对应子页，子页挂载时按意图把参考来源、轨道、片段都填好。
 *
 * 意图是**一次性**的：子页读完后由 `AiPanel` 清掉，下次从卡片进来仍按默认行为。
 */
import { ref } from "vue";
import { defineStore } from "pinia";
import { useLayoutStore } from "@/stores/layout";

/** 三个 AI 子页：写歌 / 歌声合成 / 音色替换。 */
export type AiPageId = "yue" | "singing" | "svc";

/** 写歌页的两种入口：从零写 / 按参考重唱。 */
export type AiYueMode = "create" | "cover";

export interface AiIntent {
  page: AiPageId;
  /** 写歌页：创作还是翻唱。 */
  mode?: AiYueMode;
  /** 翻唱参考来源：工程内音频轨 / 工程内 MIDI。 */
  source?: "track" | "midi";
  /** 作为输入的轨道（翻唱参考 / 歌声合成的 MIDI / 音色替换的目标）。 */
  trackId?: number | null;
  /** 作为输入的片段（音频片段或 MIDI 片段）。 */
  clipId?: number | null;
}

export const useAiIntentStore = defineStore("aiIntent", () => {
  const layout = useLayoutStore();
  /** 待处理的意图；`null` = 没有（从卡片正常进入）。 */
  const request = ref<AiIntent | null>(null);

  /** 记下意图并打开 AI 面板。 */
  function open(intent: AiIntent): void {
    request.value = intent;
    layout.openTab("ai");
  }

  /** 子页消费完就清掉，避免下次从卡片进来又套用上一次的上下文。 */
  function clear(): void {
    request.value = null;
  }

  return { request, open, clear };
});
