import { computed, ref } from "vue";
import { defineStore } from "pinia";
import {
  modelCancel,
  modelCatalog,
  modelDownload,
  modelRemove,
  modelReveal,
  onModelEvent,
} from "@/api/tauri";
import type { ModelProgress, ModelStatus } from "@/api/types";

/**
 * 本地 AI 模型（Stage G）：列出清单、下载 / 取消 / 删除。
 *
 * 大模型（如音源分离 209 MB）不随包，用户在这里按需下载；小模型随安装包分发（`bundled`）。
 * 下载进度走 `engine://model`（`onModelEvent`）；网络配置与插件共用同一份（设置页「网络」）。
 */
export const useModelsStore = defineStore("models", () => {
  const catalog = ref<ModelStatus[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);
  /** 正在下载的那个模型 id（同一时刻最多一个）。 */
  const busyId = ref<string | null>(null);
  const progress = ref<ModelProgress | null>(null);
  /** 「管理器」的模型视图需要被聚焦（从别处跳过来看下载进度时用）。 */
  const focus = ref(false);

  /** 请求把「管理器」切到「可下载 → 模型」。 */
  function requestFocus(): void {
    focus.value = true;
  }

  /** 取走（并清空）聚焦请求；返回是否曾经请求过。 */
  function consumeFocus(): boolean {
    if (!focus.value) return false;
    focus.value = false;
    return true;
  }

  /** 下载百分比；阶段不是「下载中」或总大小未知时为 `null`（界面画不定长进度条）。 */
  const progressPct = computed<number | null>(() => {
    const p = progress.value;
    if (!p || p.state !== "downloading" || p.total <= 0) return null;
    return Math.min(100, Math.round((p.received / p.total) * 100));
  });

  /** 某个模型是否正在下载。 */
  function installing(id: string): boolean {
    return busyId.value === id;
  }

  /** 拉取清单 + 本地状态。 */
  async function load(): Promise<void> {
    loading.value = true;
    error.value = null;
    try {
      catalog.value = await modelCatalog();
    } catch (err) {
      error.value = String(err);
    } finally {
      loading.value = false;
    }
  }

  /** 开始下载（进度走事件；完成后自动刷新清单）。 */
  async function download(id: string): Promise<void> {
    if (busyId.value !== null) return;
    busyId.value = id;
    progress.value = { id, state: "downloading", received: 0, total: 0, message: null };
    try {
      await modelDownload(id);
    } catch (err) {
      busyId.value = null;
      progress.value = null;
      error.value = String(err);
    }
  }

  /** 取消当前下载。 */
  async function cancel(): Promise<void> {
    try {
      await modelCancel();
    } catch {
      // 状态事件会纠正；忽略。
    }
  }

  /** 删除已下载的模型文件。 */
  async function remove(id: string): Promise<void> {
    try {
      await modelRemove(id);
    } catch (err) {
      error.value = String(err);
    }
    await load();
  }

  /** 在系统文件管理器里显示模型文件。 */
  async function reveal(id: string): Promise<void> {
    try {
      await modelReveal(id);
    } catch (err) {
      error.value = String(err);
    }
  }

  /** 订阅下载进度（组件挂载时调用，返回取消订阅函数）。 */
  function watchProgress(): () => void {
    let stop: (() => void) | null = null;
    void onModelEvent((p) => {
      if (p.state === "done") {
        busyId.value = null;
        progress.value = null;
        void load();
      } else if (p.state === "error") {
        busyId.value = null;
        progress.value = null;
        if (p.message) error.value = p.message;
      } else {
        progress.value = p;
      }
    }).then((off) => {
      stop = off;
    });
    return () => stop?.();
  }

  return {
    catalog,
    loading,
    error,
    busyId,
    progress,
    focus,
    progressPct,
    installing,
    requestFocus,
    consumeFocus,
    load,
    download,
    cancel,
    remove,
    reveal,
    watchProgress,
  };
});
