/**
 * **全局下载队列 + 「跳到下载管理器并定位到某一项」的聚焦总线。**
 *
 * 为什么要有它：模型 / ONNX 运行时 / audio.cpp 引擎原本各自管一个 `busyId`，
 * 「引擎装完再下权重」这种链式需求只能由调用方自己 `await` 串联；可一旦用户
 * 中途点了别的下载，后面那一步就会被 store 的「已有下载」守卫**静默吞掉**，
 * 界面表现为「确认过了，但什么都没发生」。
 *
 * 这里把三类下载收进一条串行队列：
 * - 同一时刻只跑一项，其余排队；
 * - 某一项失败就停下，并把「哪一项、为什么」留给界面重试；
 * - 队列跑完后调用方可以接着做「切后端 / 回原页面 / 提示重启」。
 *
 * `focus` 是给下载管理器用的：跳过去之后要自动切页签、切筛选、滚动定位并高亮，
 * 否则用户落在「已安装」页签上，只会看到一句「没有可下载的内容」。
 */
import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { useLayoutStore } from "@/stores/layout";
import { useModelsStore } from "@/stores/models";
import { useRuntimesStore } from "@/stores/runtimes";

/** 下载管理器里按「哪一类东西」定位（与 ManagerPane 的分类筛选对应）。 */
export type DownloadSection = "engine" | "runtime" | "model";

export interface DownloadTask {
  section: DownloadSection;
  /** 模型 id / 运行时 id / 引擎包 id。 */
  id: string;
  /** 界面文案用的名字（失败提示要指名道姓）。 */
  label: string;
  /** 预计要下多少字节（磁盘预检用；未知就省略）。 */
  bytes?: number;
}

export interface DownloadFocus {
  section: DownloadSection;
  id: string;
}

/** 失败的那一项（给「重试剩余」用）。 */
export interface DownloadFailure {
  task: DownloadTask;
  message: string;
  /** 失败时还没跑完的队列（含失败项本身）——「重试剩余」就重跑这一串。 */
  rest: DownloadTask[];
}

/** 取消是「用户的选择」，不是错误：调用方据此决定要不要弹红条。 */
export function isCancelled(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return message.includes("取消");
}

export const useDownloadsStore = defineStore("downloads", () => {
  const models = useModelsStore();
  const runtimes = useRuntimesStore();
  const layout = useLayoutStore();

  /** 还没开始的任务（第一项就是正在跑的）。 */
  const queue = ref<DownloadTask[]>([]);
  const current = ref<DownloadTask | null>(null);
  const failure = ref<DownloadFailure | null>(null);
  /** 本轮一共几项 / 已完成几项（「已完成 1/2」这种文案要用）。 */
  const total = ref(0);
  const done = ref(0);
  /** 上一次结束是不是用户主动取消（不要拿它弹「下载失败」）。 */
  const cancelled = ref(false);
  /** 跳到下载管理器时要定位到哪一项（消费后清空）。 */
  const focus = ref<DownloadFocus | null>(null);

  const running = computed(() => current.value !== null);
  const pending = computed(() => queue.value.length);
  /** 进度文案：`已完成 1/2`。 */
  const progressText = computed(() => ({ done: done.value, total: total.value }));

  /**
   * 当前这一项的进度（给「留在本页下载」的页内进度条用）。
   *
   * 三类进度分别由各自的 store 报：模型走 `models.progressPct`、运行时与引擎都走
   * `runtimes.progressPct`。总大小未知时 `pct` 为 `null`，界面画不定长条纹。
   */
  const progress = computed<{
    label: string;
    pct: number | null;
    index: number;
    total: number;
  } | null>(() => {
    const task = current.value;
    if (!task) return null;
    const pct = task.section === "model" ? models.progressPct : runtimes.progressPct;
    return {
      label: task.label,
      pct: pct ?? null,
      index: Math.min(done.value + 1, Math.max(1, total.value)),
      total: Math.max(1, total.value),
    };
  });

  function requestFocus(section: DownloadSection, id: string): void {
    focus.value = { section, id };
  }

  /** 取走（并清空）聚焦请求；返回是否曾经请求过。 */
  function consumeFocus(): DownloadFocus | null {
    const value = focus.value;
    focus.value = null;
    return value;
  }

  /**
   * 跳管理器 + 定位到具体条目。
   *
   * 所有入口（AI 面板 / 设置页 / 后端选择器 / 安装向导）都走这一个函数：以前
   * `openTab("manager")` + `requestFocus` 在各处重复了六七遍，漏传 id 就退化成
   * 「跳过去了，但没定位」。
   */
  function jumpTo(section: DownloadSection, id: string): void {
    layout.openTab("manager");
    requestFocus(section, id);
    // 管理器仍然消费两套 store 的 focus（它分「模型 / 运行时」两组渲染）。
    if (section === "model") models.requestFocus(id);
    else runtimes.requestFocus(id);
  }

  /** 跑一项：三套下载的失败语义统一成「抛异常」。 */
  async function one(task: DownloadTask): Promise<void> {
    if (task.section === "model") {
      await models.download(task.id);
      return;
    }
    const ok = await runtimes.installAndWait(task.id);
    if (!ok) throw new Error(runtimes.error ?? `「${task.label}」下载失败`);
  }

  /**
   * 按顺序下载一串任务，返回是否全部成功。
   *
   * `jump` 为真时先把用户带到下载管理器并定位第一项（**然后**开始下载）；为假时留在
   * 当前页面，由页内的下载条报进度——「在后台下载」走的正是这条。
   *
   * 失败时停下（不继续后面的项），并把失败项与剩余队列记在 `failure`；
   * 调用方可以拿 `failure.rest` 组一个「重试剩余」。
   */
  async function run(
    tasks: DownloadTask[],
    options: { jump?: boolean } = {},
  ): Promise<boolean> {
    if (running.value) return false;
    if (tasks.length === 0) return true;
    failure.value = null;
    cancelled.value = false;
    queue.value = [...tasks];
    total.value = tasks.length;
    done.value = 0;

    if (options.jump) {
      const first = tasks[0];
      if (first) jumpTo(first.section, first.id);
    }

    let ok = true;
    while (queue.value.length > 0) {
      const task = queue.value[0]!;
      current.value = task;
      try {
        await one(task);
        done.value += 1;
        queue.value = queue.value.slice(1);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        if (isCancelled(error)) {
          // 取消：保留已完成项，不记失败，也不留「重试剩余」。
          cancelled.value = true;
          queue.value = [];
          ok = false;
          break;
        }
        failure.value = { task, message, rest: [...queue.value] };
        queue.value = [];
        ok = false;
        break;
      }
    }
    current.value = null;
    return ok;
  }

  /** 失败之后接着下「没完成的那几项」。 */
  async function retry(options: { jump?: boolean } = {}): Promise<boolean> {
    const rest = failure.value?.rest ?? [];
    if (rest.length === 0) return false;
    return run(rest, options);
  }

  /** 取消当前项（排队中的会一并丢掉）。 */
  async function cancel(): Promise<void> {
    const task = current.value;
    queue.value = [];
    try {
      if (task?.section === "model") await models.cancel();
      else if (task) await runtimes.cancel();
    } finally {
      current.value = null;
    }
  }

  /** 「重试剩余」按钮要显示的数量。 */
  const retryCount = computed(() => failure.value?.rest.length ?? 0);

  return {
    queue,
    current,
    failure,
    total,
    done,
    cancelled,
    focus,
    running,
    pending,
    progress,
    progressText,
    retryCount,
    requestFocus,
    consumeFocus,
    jumpTo,
    run,
    retry,
    cancel,
  };
});
