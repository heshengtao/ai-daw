import { computed, ref } from "vue";
import { defineStore } from "pinia";
import {
  modelCancel,
  modelCatalog,
  modelDownload,
  modelProbe,
  modelRemove,
  modelReveal,
  onModelEvent,
} from "@/api/tauri";
import type { ModelProgress, ModelStatus, SourceProbe } from "@/api/types";

/**
 * 本地 AI 模型（Stage G）：列出清单、下载 / 取消 / 删除、测源。
 *
 * 大模型（如音源分离 209 MB、写歌权重几 GB）不随包，用户在这里按需下载；小模型随安装包
 * 分发（`bundled`）。下载进度走 `engine://model`（`onModelEvent`）；网络配置与插件共用同一份
 * （设置页「网络」）。
 *
 * **`download()` 会等到真正下完才 resolve**：队列 / 向导要靠它串「装完引擎再下权重」；
 * 已有下载时**抛错**而不是静默返回（以前直接 `return`，链式下载被无声吞掉，界面看起来
 * 就是「点了确认但什么都没发生」）。
 */
export const useModelsStore = defineStore("models", () => {
  const catalog = ref<ModelStatus[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);
  /** 正在下载的那个模型 id（同一时刻最多一个）。 */
  const busyId = ref<string | null>(null);
  const progress = ref<ModelProgress | null>(null);
  /** 「管理器」的模型视图需要被聚焦（从别处跳过来看下载进度时用）。 */
  const focus = ref<string | null>(null);
  /** 「测试源」的结果（按模型 id 缓存，下一次测覆盖）。 */
  const probes = ref<Record<string, SourceProbe[]>>({});
  /** 正在测源的模型 id。 */
  const probingId = ref<string | null>(null);

  /** 当前这次下载的收尾函数（取消时主动结算，避免队列一直等）。 */
  let settleCurrent: ((result: { ok: boolean; message?: string }) => void) | null = null;
  /** 同一个 id 重复调用时共用这一次 Promise。 */
  let inflight: { id: string; promise: Promise<void> } | null = null;

  /**
   * 请求把「管理器」切到「可下载 → 模型」。
   *
   * 传 id 时会顺带滚到那一行并高亮；不传就是切个页签。
   */
  function requestFocus(id = "*"): void {
    focus.value = id;
  }

  /** 取走（并清空）聚焦请求；返回要定位的 id（`"*"` = 只切页签）。 */
  function consumeFocus(): string | null {
    const value = focus.value;
    focus.value = null;
    return value;
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

  /**
   * 开始下载并**等到下完**。
   *
   * - 已有别的下载在跑 → 抛错（调用方可以据此排队或提示）；
   * - 同一个 id 正在下 → 共用同一次 Promise，不重复发起；
   * - 失败 / 取消 → 抛错（取消的 message 含「取消」，调用方据此不弹红条）。
   */
  async function download(id: string): Promise<void> {
    if (inflight?.id === id) return inflight.promise;
    if (busyId.value !== null && busyId.value !== id) {
      throw new Error("已有模型正在下载，请先等它结束或取消。");
    }

    const promise = (async () => {
      let settle!: (result: { ok: boolean; message?: string }) => void;
      const finished = new Promise<{ ok: boolean; message?: string }>((resolve) => {
        settle = resolve;
      });
      settleCurrent = settle;

      // 先订阅再发命令：下载线程可能在命令返回前就报完 `done`，反过来的顺序会丢事件。
      const off = await onModelEvent((p) => {
        if (p.id !== id) return;
        if (p.state === "done") settle({ ok: true });
        else if (p.state === "error") settle({ ok: false, message: p.message ?? "下载失败" });
        else progress.value = p;
      });

      busyId.value = id;
      progress.value = {
        id,
        state: "downloading",
        received: 0,
        total: 0,
        message: null,
        source: null,
        source_kind: null,
        region: null,
      };
      error.value = null;

      try {
        // 命令立即返回的失败（未知模型 / 后端已有下载）在这里就抛出来。
        await modelDownload(id);
        const result = await finished;
        if (!result.ok) {
          const message = result.message ?? "下载失败";
          if (!message.includes("取消")) error.value = message;
          throw new Error(message);
        }
        await load();
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        if (!message.includes("取消")) error.value = message;
        throw err instanceof Error ? err : new Error(message);
      } finally {
        settleCurrent = null;
        busyId.value = null;
        progress.value = null;
        off();
      }
    })().finally(() => {
      inflight = null;
    });

    inflight = { id, promise };
    return promise;
  }

  /** 取消当前下载（主动结算 Promise，队列不会一直等一个再也不会来的事件）。 */
  async function cancel(): Promise<void> {
    settleCurrent?.({ ok: false, message: "已取消" });
    settleCurrent = null;
    try {
      await modelCancel();
    } catch {
      // 状态事件 / 主动结算已经纠正；忽略。
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

  /** 逐源测速（「这条线通不通、多少毫秒」），结果缓存到 `probes`。 */
  async function probe(id: string): Promise<void> {
    if (probingId.value !== null) return;
    probingId.value = id;
    try {
      probes.value = { ...probes.value, [id]: await modelProbe(id) };
    } catch (err) {
      error.value = String(err);
    } finally {
      probingId.value = null;
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
        // 取消不是错误：别在管理器顶部留一条红条。
        if (p.message && !p.message.includes("取消")) error.value = p.message;
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
    probes,
    probingId,
    progressPct,
    installing,
    requestFocus,
    consumeFocus,
    load,
    download,
    cancel,
    remove,
    reveal,
    probe,
    watchProgress,
  };
});
