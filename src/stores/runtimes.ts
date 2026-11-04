import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { mockEnabled } from "@/api/mock";
import {
  aiBackendInfo,
  aiRuntimeBusy,
  aiRuntimeCancel,
  aiRuntimeCatalog,
  aiRuntimeDiscover,
  aiRuntimeImport,
  aiRuntimeReveal,
  aiRuntimeInstall,
  aiRuntimeRemove,
  aiYueSelect,
  onRuntimeEvent,
} from "@/api/tauri";
import type { DiscoveredRuntime, RuntimeProgress, RuntimeStatus } from "@/api/types";

/**
 * 运行时（下载管理器 → 运行时）：ONNX Runtime 的各份 dylib，以及 audio.cpp 引擎。
 *
 * ONNX Runtime 是 `load-dynamic` 的 dylib：**一个进程只能加载一份**，所以装好 / 换后端之后
 * 要重启才生效（界面另给提示）。audio.cpp 引擎是独立进程（`audiocpp_server`），每次启动现挑后端，
 * 不需要重启。两者共用同一张列表，靠 `group` 区分。进度走 `engine://runtime`。
 */
export const useRuntimesStore = defineStore("runtimes", () => {
  const items = ref<RuntimeStatus[]>([]);
  /** 本机已有的、可以直接借用的 CUDA 版运行时。 */
  const discovered = ref<DiscoveredRuntime[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);
  /** 正在下载的运行时 id（同一时刻最多一个）。 */
  const busyId = ref<string | null>(null);
  const progress = ref<RuntimeProgress | null>(null);
  /** 「下载管理器」需要被聚焦到「运行时」标签（从设置页跳过来时用）。 */
  const focus = ref<string | null>(null);
  /** 运行时装完后要回设置页弹「重启生效」（设置页可能已被跳走卸载）。 */
  const restartPrompt = ref(false);

  function requestRestartPrompt(): void {
    restartPrompt.value = true;
  }

  function consumeRestartPrompt(): boolean {
    if (!restartPrompt.value) return false;
    restartPrompt.value = false;
    return true;
  }

  /**
   * 装 / 删之后问一次后端「要不要重启才生效」。
   *
   * 只比较「已加载的运行时路径」是不够的：CUDA 组件是在加载那一刻一次性预载的，
   * 之后再补装 cuDNN / cuFFT，路径没变但仍然要重启。后端为此单独记了标记
   * （`pending-restart`），这里只是把它翻译成一个界面提示。
   */
  async function refreshRestartPrompt(): Promise<void> {
    try {
      const info = await aiBackendInfo();
      if (info.restart_required) restartPrompt.value = true;
    } catch {
      // 预览 / 后端不可用：这只是个提示，失败就不提。
    }
  }

  /** 下载百分比；不在下载中或总大小未知时为 `null`。 */
  const progressPct = computed<number | null>(() => {
    const p = progress.value;
    if (!p || p.state !== "downloading" || p.total <= 0) return null;
    return Math.min(100, Math.round((p.received / p.total) * 100));
  });

  /** 请求把管理器切到「运行时」标签；传 id 时会滚到那一行。 */
  function requestFocus(id = "*"): void {
    focus.value = id;
  }

  /** 取走（并清空）聚焦请求；返回要定位的 id（`"*"` = 只切页签）。 */
  function consumeFocus(): string | null {
    const value = focus.value;
    focus.value = null;
    return value;
  }

  async function load(): Promise<void> {
    loading.value = true;
    error.value = null;
    try {
      items.value = await aiRuntimeCatalog();
      // 下载线程可能在管理器没打开时就已经结束：别让界面一直卡在「下载中」。
      // （后端没有这个命令时按「还在下载」处理，宁可不纠正也不要误清状态。）
      let busy = true;
      try {
        busy = await aiRuntimeBusy();
      } catch {
        busy = true;
      }
      if (!busy) {
        busyId.value = null;
        progress.value = null;
      }
      discovered.value = await aiRuntimeDiscover();
    } catch (err) {
      error.value = String(err);
    } finally {
      loading.value = false;
    }
  }

  /** 把本机已有的那一份复制进应用目录（省掉 153 MB 下载）。 */
  async function importFrom(dir: string): Promise<void> {
    try {
      await aiRuntimeImport(dir);
    } catch (err) {
      error.value = String(err);
    }
    await load();
  }

  async function install(id: string): Promise<void> {
    if (busyId.value !== null && busyId.value !== id) {
      error.value = "已有运行时正在下载，请先等它结束或取消。";
      return;
    }
    busyId.value = id;
    progress.value = { id, state: "downloading", received: 0, total: 0, message: null };
    error.value = null;
    try {
      await aiRuntimeInstall(id);
    } catch (err) {
      busyId.value = null;
      progress.value = null;
      error.value = String(err);
    }
  }

  async function cancel(): Promise<void> {
    try {
      await aiRuntimeCancel();
    } catch {
      // 状态事件会纠正。
    } finally {
      // 立刻收起进度条；线程会在下一个数据块处停下。
      busyId.value = null;
      progress.value = null;
    }
  }

  /** 在系统文件管理器里显示运行时目录。 */
  async function reveal(id: string): Promise<void> {
    try {
      await aiRuntimeReveal(id);
    } catch (err) {
      error.value = String(err);
    }
  }

  /**
   * 挑 audio.cpp 引擎用哪个后端（`null` = 按显卡自动挑）。
   *
   * 返回是否真的切成功：「点了没装的后端 → 下载 → 自动切过去」这条链路只有在
   * 切换没被后端拒绝时才该弹「已切换」，否则界面会谎报成功。
   */
  async function select(id: string | null): Promise<boolean> {
    let failure: string | null = null;
    try {
      await aiYueSelect(id);
    } catch (err) {
      failure = String(err);
    }
    // `load()` 会把 `error` 清空：先记下来，刷新完再放回去。
    await load();
    if (failure) {
      error.value = failure;
      return false;
    }
    return true;
  }

  /**
   * 开始下载一个运行时 / 引擎后端，并**等到装完**。
   *
   * `install()` 只负责发起后台下载，完成状态原本只有管理器订阅 `engine://runtime`
   * 才回得来；设置页 / AI 面板的「点了没装的后端 → 去下载 → 直接切过去」需要等结果，
   * 所以这里自己订阅一次（不依赖管理器是否已经挂载）。
   *
   * 返回是否真的装好（取消 / 出错都是 `false`）。
   */
  async function installAndWait(id: string): Promise<boolean> {
    if (busyId.value !== null && busyId.value !== id) {
      error.value = "已有运行时正在下载，请先等它结束或取消。";
      return false;
    }
    busyId.value = id;
    progress.value = { id, state: "downloading", received: 0, total: 0, message: null };
    error.value = null;

    // 用对象保存 resolve：TS 不会跟踪闭包里的赋值，直接写成可空变量会被收窄成 never。
    const completer: { settle?: (ok: boolean) => void } = {};
    let finishedOk = false;
    let reason: string | null = null;
    let stop: (() => void) | null = null;
    const finished = new Promise<boolean>((resolve) => {
      completer.settle = (ok) => {
        if (finishedOk) return;
        finishedOk = true;
        resolve(ok);
      };
    });
    const release = (): void => {
      stop?.();
      stop = null;
    };
    void onRuntimeEvent((p) => {
      if (p.id !== id) return;
      if (p.state === "done") {
        completer.settle?.(true);
        release();
      } else if (p.state === "error") {
        reason = p.message ?? "下载失败";
        completer.settle?.(false);
        release();
      }
    }).then((off) => {
      if (finishedOk) off();
      else stop = off;
    }).catch(() => {
      // 订阅失败（例如后端刚退出）：别留下未处理的 rejection，也别让界面一直等。
      reason = "无法订阅下载进度";
      completer.settle?.(false);
      release();
    });

    try {
      await aiRuntimeInstall(id);
    } catch (err) {
      reason = String(err);
      completer.settle?.(false);
      release();
    }
    // 预览模式是同步装好、且不发进度事件：别在这里死等。
    if (mockEnabled()) {
      completer.settle?.(true);
      release();
    }

    const ok = await finished;
    busyId.value = null;
    progress.value = null;
    if (!ok) {
      // 失败 / 取消：磁盘状态没变，**不刷新**——刷新会把管理器刚弹出的原因清掉。
      if (!error.value && reason) error.value = reason;
      return false;
    }
    await load();
    return true;
  }

  async function remove(id: string): Promise<void> {
    let failure: string | null = null;
    try {
      await aiRuntimeRemove(id);
    } catch (err) {
      failure = String(err);
    }
    // `load()` 会把 `error` 清空：先记下来，刷新完再放回去并抛给调用方去弹提示。
    // （以前删除失败的信息就这样被静默吃掉了，界面看起来像「点了删除但没反应」。）
    await load();
    await refreshRestartPrompt();
    if (failure) {
      error.value = failure;
      throw failure;
    }
  }

  /** 订阅下载进度（挂载时调用，返回取消订阅函数）。 */
  function watchProgress(): () => void {
    let stop: (() => void) | null = null;
    void onRuntimeEvent((p) => {
      if (p.state === "done") {
        busyId.value = null;
        progress.value = null;
        void load();
        void refreshRestartPrompt();
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
    items,
    discovered,
    loading,
    error,
    busyId,
    progress,
    progressPct,
    focus,
    requestFocus,
    consumeFocus,
    restartPrompt,
    requestRestartPrompt,
    consumeRestartPrompt,
    refreshRestartPrompt,
    load,
    install,
    installAndWait,
    cancel,
    remove,
    reveal,
    select,
    importFrom,
    watchProgress,
  };
});
