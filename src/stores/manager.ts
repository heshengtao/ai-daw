/**
 * 插件管理器 store：注册表（可下载 / 可更新）与已安装记录，外加安装 / 导入 / 删除动作。
 *
 * 与 `stores/plugins` 的分工：
 * - `stores/plugins` 是「把插件加载到轨道」的清单（宿主**扫描**到的结果）；
 * - 本 store 管的是插件**文件本身**（浏览注册表、下载、导入、更新、删除）。
 *
 * 任何改动都会让后端重扫，结果经 `engine://plugins` 回到 `stores/plugins`——所以两个 store
 * 不需要互相调用，只管各自那份数据。
 */
import { defineStore } from "pinia";
import { computed, ref } from "vue";

import {
  managerForeign,
  managerImport,
  managerInstalled,
  managerInstall,
  managerOpen,
  managerRegistry,
  managerRemove,
  onManagerEvent,
} from "@/api/tauri";
import type {
  ForeignPlugin,
  ManagerFile,
  ManagerInstallRequest,
  ManagerInstalled,
  ManagerPlugin,
  ManagerProgress,
} from "@/api/types";

export const useManagerStore = defineStore("manager", () => {
  /** 注册表里的可下载插件。 */
  const registry = ref<ManagerPlugin[]>([]);
  /** 本管理器装过的记录。 */
  const installed = ref<ManagerInstalled[]>([]);
  /** 用户自己装的插件（扫描到但不在托管库；只读）。 */
  const foreign = ref<ForeignPlugin[]>([]);
  const platform = ref("");
  const arch = ref("");
  const fetchedAt = ref(0);
  const fromCache = ref(false);

  const loading = ref(false);
  /** 正在处理的那一条（slug 或 `import` / 删除的 key）；用于禁用按钮与转圈。 */
  const busy = ref<string | null>(null);
  const error = ref<string | null>(null);

  const installedByKey = computed(() => new Map(installed.value.map((r) => [r.key, r])));

  /** 某个注册表插件的已装记录（没装过为 `undefined`）。 */
  function installedFor(slug: string): ManagerInstalled | undefined {
    return installedByKey.value.get(slug);
  }

  /** 已装版本与注册表版本不同 → 可以更新（本地导入的记录没有版本号，不算）。 */
  function updateAvailable(p: ManagerPlugin): boolean {
    const rec = installedFor(p.slug);
    return !!rec && rec.version !== "" && rec.version !== p.version;
  }

  /** 拉取注册表与已装 / 外部插件清单（`force` 强制联网刷新注册表）。 */
  async function load(force = false): Promise<void> {
    loading.value = true;
    error.value = null;
    try {
      const [reg, inst, ext] = await Promise.all([
        managerRegistry(force),
        managerInstalled(),
        managerForeign(),
      ]);
      registry.value = reg.plugins;
      platform.value = reg.platform;
      arch.value = reg.arch;
      fetchedAt.value = reg.fetched_at;
      fromCache.value = reg.from_cache;
      installed.value = inst;
      foreign.value = ext;
    } catch (e) {
      error.value = String(e);
    } finally {
      loading.value = false;
    }
  }

  /**
   * 重新拉「已装 / 外部」两份清单（装完 / 删完调用）。
   *
   * 不必重拉注册表；同理装了新插件后外部清单也会少一条（它进了托管库）。
   */
  async function refreshLocal(): Promise<void> {
    const [inst, ext] = await Promise.all([managerInstalled(), managerForeign()]);
    installed.value = inst;
    foreign.value = ext;
  }

  /**
   * 当前安装的进度（`null` = 没在下）。
   *
   * 由后端 `engine://manager` 事件驱动；界面据此在**卡片下边缘**画进度条。
   */
  const progress = ref<ManagerProgress | null>(null);

  /** 进度百分比（0–100）；总大小未知时给 `null`（该画不确定态）。 */
  const progressPct = computed(() => {
    const p = progress.value;
    if (!p || p.phase !== "downloading" || p.total <= 0) return null;
    return Math.min(100, Math.round((p.received / p.total) * 100));
  });

  /** 订阅后端进度（返回取消订阅函数）。 */
  function watchProgress(): () => void {
    let stop: (() => void) | null = null;
    void onManagerEvent((p) => {
      if (p.phase === "error") {
        progress.value = null;
        return;
      }
      progress.value = p;
    }).then((off) => {
      stop = off;
    });
    return () => stop?.();
  }

  /** 从注册表安装（也是更新）。 */
  async function install(p: ManagerPlugin, file: ManagerFile): Promise<void> {
    busy.value = p.slug;
    error.value = null;
    progress.value = { slug: p.slug, phase: "downloading", received: 0, total: file.size, message: null };
    try {
      const req: ManagerInstallRequest = {
        slug: p.slug,
        name: p.name,
        version: p.version,
        url: file.url,
        sha256: file.sha256,
        size: file.size,
      };
      await managerInstall(req);
      await refreshLocal();
    } catch (e) {
      error.value = String(e);
      throw e;
    } finally {
      busy.value = null;
      progress.value = null;
    }
  }

  /** 导入本地 `.vst3` / `.clap`。 */
  async function importFile(path: string): Promise<void> {
    busy.value = "import";
    error.value = null;
    try {
      await managerImport(path);
      await refreshLocal();
    } catch (e) {
      error.value = String(e);
      throw e;
    } finally {
      busy.value = null;
    }
  }

  /** 删除一条托管安装。 */
  async function remove(key: string): Promise<void> {
    busy.value = key;
    error.value = null;
    try {
      await managerRemove(key);
      await refreshLocal();
    } catch (e) {
      error.value = String(e);
      throw e;
    } finally {
      busy.value = null;
    }
  }

  /** 在浏览器里打开下载页（安装器类型用）。 */
  async function open(url: string): Promise<void> {
    await managerOpen(url);
  }

  return {
    registry,
    installed,
    foreign,
    platform,
    arch,
    fetchedAt,
    fromCache,
    loading,
    busy,
    error,
    progress,
    progressPct,
    installedFor,
    updateAvailable,
    load,
    refreshLocal,
    watchProgress,
    install,
    importFile,
    remove,
    open,
  };
});
