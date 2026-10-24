/**
 * 自动更新：共享状态（检查 / 下载 / 安装 / 重启）。
 *
 * 启动时由主窗口静默查一次；设置页「关于」里可以手动查、看进度、下载安装并重启。
 * 真正的网络与安装都在 `src/api/updater.ts`，这里只管状态机与界面要用的派生值。
 */
import { defineStore } from "pinia";
import { computed, ref } from "vue";
import {
  checkUpdate,
  currentVersion,
  downloadAndInstall,
  relaunchApp,
  type UpdateInfo,
} from "@/api/updater";
import { mockEnabled } from "@/api/mock";

/** 「上次运行过的版本」持久化键：用来判断这次启动是不是刚更新完。 */
const SEEN_VERSION_KEY = "ai-daw-seen-version";

/** 更新流程所处的阶段。 */
export type UpdatePhase =
  | "idle"
  | "checking"
  | "available"
  | "downloading"
  | "ready"
  | "error";

export const useUpdaterStore = defineStore("updater", () => {
  const phase = ref<UpdatePhase>("idle");
  const info = ref<UpdateInfo | null>(null);
  const error = ref("");
  const downloaded = ref(0);
  const total = ref(0);
  /** 当前运行版本号（启动时读一次）。 */
  const version = ref("");
  /** 「刚更新到」的版本号：非空时界面弹出更新完成提示；首次安装不弹。 */
  const justUpdatedVersion = ref<string | null>(null);

  /** 下载进度 0..1（总长未知时为 0）。 */
  const progress = computed(() => (total.value > 0 ? downloaded.value / total.value : 0));

  /** 读一次当前版本号（设置页加载时调用）。 */
  async function loadVersion(): Promise<void> {
    try {
      version.value = await currentVersion();
    } catch {
      version.value = "";
    }
  }

  /**
   * 启动时判断「这次是不是更新后的首次打开」：只有上次记录的版本存在且与当前版本不同才算，
   * 首次安装（没有记录）不提示。
   *
   * 无论是否弹窗都立刻记下当前版本——用户没点「查看更新日志」也不会下次再弹一遍。
   */
  async function detectJustUpdated(): Promise<void> {
    if (mockEnabled()) return;
    try {
      const current = await currentVersion();
      const seen = localStorage.getItem(SEEN_VERSION_KEY);
      localStorage.setItem(SEEN_VERSION_KEY, current);
      if (seen && current && seen !== current) justUpdatedVersion.value = current;
    } catch {
      // 读不到版本号（或存不住）就当没更新过，不打扰用户。
    }
  }

  /** 关掉「更新完成」提示。 */
  function dismissJustUpdated(): void {
    justUpdatedVersion.value = null;
  }

  /**
   * 查一次更新；返回是否有新版本。
   *
   * `silent` 用于启动时的后台检查：查不到 / 连不上（例如还没发过 release，端点 404）不该
   * 在界面上留下错误态，只当作「已是最新」。
   */
  async function check(silent = false): Promise<boolean> {
    phase.value = "checking";
    error.value = "";
    try {
      const found = await checkUpdate();
      info.value = found;
      phase.value = found ? "available" : "idle";
      return found !== null;
    } catch (err) {
      error.value = String(err);
      phase.value = silent ? "idle" : "error";
      return false;
    }
  }

  /** 下载并安装已发现的更新。 */
  async function install(): Promise<void> {
    if (!info.value) return;
    phase.value = "downloading";
    error.value = "";
    downloaded.value = 0;
    total.value = 0;
    try {
      await downloadAndInstall((done, all) => {
        downloaded.value = done;
        total.value = all;
      });
      phase.value = "ready";
    } catch (err) {
      error.value = String(err);
      phase.value = "error";
    }
  }

  /** 重启到新版本。 */
  async function restart(): Promise<void> {
    try {
      await relaunchApp();
    } catch (err) {
      error.value = String(err);
      phase.value = "error";
    }
  }

  return {
    phase,
    info,
    error,
    downloaded,
    total,
    version,
    justUpdatedVersion,
    progress,
    loadVersion,
    detectJustUpdated,
    dismissJustUpdated,
    check,
    install,
    restart,
  };
});
