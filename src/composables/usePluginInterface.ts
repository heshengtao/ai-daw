/**
 * 「打开这个插件的界面」——插件清单页与检查器共用这一条路径。
 *
 * 两种落点：
 * - **自带原生界面的外部插件**：加载到目标轨道后开它自己的窗口（对着正在发声的那个实例）；
 * - **内置合成器**：开一台合成器的独立窗口（拿不到窗口时退回侧边栏面板）；
 * - **其余**（没有原生界面的外部插件）：切到侧边栏的「插件界面」标签看参数表。
 *
 * 引擎装载插件是异步的（音频线程重建播放器后才登记编辑器句柄），所以开窗前要重试几次。
 */
import { usePluginsStore } from "@/stores/plugins";
import { useProjectStore } from "@/stores/project";
import { useLayoutStore } from "@/stores/layout";
import { useI18n } from "@/i18n";
import { canonicalPluginId, CHORUS_ID, DELAY_ID, DRIVE_ID, EQ_ID, REVERB_ID, SYNTH_ID } from "@/data/plugins";
import { openEffectPluginWindow, openEffectWindow, openPluginWindow, openSynthWindow } from "@/api/tauri";
import { isTauri } from "@/utils/platform";

/** 有独立面板的内置效果器（点「界面」开我们自己的窗口，而不是原生窗口）。 */
const BUILTIN_EFFECT_IDS = [DELAY_ID, REVERB_ID, CHORUS_ID, DRIVE_ID, EQ_ID];

export function isBuiltinEffect(pluginId: string | undefined | null): boolean {
  return !!pluginId && BUILTIN_EFFECT_IDS.includes(canonicalPluginId(pluginId));
}

/** 开窗口的尝试次数与间隔：够覆盖一次播放器重建，又不至于让用户干等。 */
const OPEN_ATTEMPTS = 12;
const OPEN_RETRY_MS = 120;

export function usePluginInterface() {
  const store = useProjectStore();
  const plugins = usePluginsStore();
  const layout = useLayoutStore();
  const { t } = useI18n();

  /**
   * 打开某条轨道上某个插件的界面。
   *
   * @param load 该轨道当前不是这个插件时，是否先把它加载上去（插件清单页要，检查器不要）。
   */
  async function open(trackId: number, pluginId: string, load = false): Promise<void> {
    const info = plugins.info(pluginId);

    if (load) {
      const current = store.project?.tracks.find((t) => t.id === trackId)?.instrument?.plugin_id;
      if (current !== pluginId) {
        try {
          await store.setTrackInstrument(trackId, pluginId);
        } catch (err) {
          layout.notify(t("panel.loadFailed", { error: String(err) }));
          return;
        }
      }
    }

    if (!info?.hasEditor) {
      // 内置合成器：一台完整的合成器面板，开在它自己的窗口里。
      if (canonicalPluginId(pluginId) === SYNTH_ID) {
        try {
          await openSynthWindow(trackId, plugins.name(pluginId));
          return;
        } catch {
          // 没有独立窗口（浏览器预览 / 建窗失败）就退回侧边栏面板，功能不打折。
        }
      }
      layout.openTab("editor");
      return;
    }

    let lastError = "";
    for (let attempt = 0; attempt < OPEN_ATTEMPTS; attempt += 1) {
      try {
        await openPluginWindow(trackId, plugins.name(pluginId));
        return;
      } catch (err) {
        lastError = String(err);
        await new Promise((resolve) => setTimeout(resolve, OPEN_RETRY_MS));
      }
    }
    // 把最后一次的真实原因带出来：「引擎还没装载好」和「这里没有原生窗口」是两回事。
    layout.notify(t("panel.editorUnavailable", { error: lastError }));
  }

  /**
   * 打开**效果器**的界面。
   *
   * 三种落点，按「插件能提供什么」择优：
   * - **自带原生界面的外部效果器**（VST3 / CLAP）：开它自己的独立窗口，对着正在发声的实例，
   *   界面里的改动实时作用于声音；
   * - **内置效果器**：开我们自己的面板窗口；
   * - **没有原生界面的外部效果器**：同样开独立窗口，只是里面是参数表——不占用侧边栏。
   *
   * 浏览器预览里没有独立窗口，一律退回侧边栏。
   */
  async function openEffect(trackId: number, pluginId: string): Promise<void> {
    // 浏览器预览里没有独立窗口：退回侧边栏面板。
    if (!isTauri()) {
      layout.openTab("editor");
      return;
    }

    // 自带原生界面的**外部**效果器：开它自己的窗口（复用与乐器一致的「等引擎装载」重试）。
    if (!isBuiltinEffect(pluginId) && plugins.info(pluginId)?.hasEditor) {
      let lastError = "";
      for (let attempt = 0; attempt < OPEN_ATTEMPTS; attempt += 1) {
        try {
          await openEffectPluginWindow(trackId, pluginId, plugins.name(pluginId));
          return;
        } catch (err) {
          lastError = String(err);
          await new Promise((resolve) => setTimeout(resolve, OPEN_RETRY_MS));
        }
      }
      layout.notify(t("panel.editorUnavailable", { error: lastError }));
      return;
    }

    // 内置效果器，以及没有自带界面的外部效果器：开我们自己的参数窗口。
    try {
      await openEffectWindow(trackId, pluginId, plugins.name(pluginId));
    } catch (err) {
      // 后端只会在「这个效果器已经不在链上」时失败——明说，而不是开一扇空窗。
      layout.notify(String(err));
    }
  }

  return { open, openEffect };
}
