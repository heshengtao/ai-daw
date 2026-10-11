/**
 * 音频导入进度：订阅后端 `engine://import`，给顶栏下边缘的进度条提供状态。
 *
 * 与 `stores/project` 的分工：工程 store 管数据与编辑命令，本 store 只管「这次导入解码到哪了」
 * 这份瞬时界面状态（不持久化）。解码完成 / 出错时清空 —— 进度条随之隐藏。
 */
import { defineStore } from "pinia";
import { computed, ref } from "vue";

import { onImportEvent } from "@/api/tauri";
import type { ImportProgress } from "@/api/types";

export const useImportingStore = defineStore("importing", () => {
  /** 当前解码进度；`null` = 没在导入（进度条隐藏）。 */
  const progress = ref<ImportProgress | null>(null);

  /** 是否正在导入（进度条是否可见）。 */
  const active = computed(() => progress.value !== null);

  /** 进度百分比（0–100）；总帧数未知时给 `null`（该画不确定态）。 */
  const progressPct = computed(() => {
    const p = progress.value;
    if (!p || !p.total_frames || p.frames <= 0) return null;
    return Math.min(100, Math.round((p.frames / p.total_frames) * 100));
  });

  /** 订阅后端进度（返回取消订阅函数）。完成 / 出错一律清空。 */
  function watchProgress(): () => void {
    let stop: (() => void) | null = null;
    void onImportEvent((p) => {
      progress.value = p.state === "decoding" ? p : null;
    }).then((off) => {
      stop = off;
    });
    return () => stop?.();
  }

  return { progress, active, progressPct, watchProgress };
});
