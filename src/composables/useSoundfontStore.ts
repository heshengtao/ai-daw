/**
 * 在线音色库（SoundFont 索引）的搜索与安装逻辑。
 *
 * 只放**状态与动作**，不放排版：下载管理器用 `card` 卡片、插件面板用紧凑行，两处长得不一样，
 * 但搜索结果、加载态、安装中的 slug 必须共用同一套语义。
 */
import { ref } from "vue";
import { installSoundfontFromStore, searchSoundfontStore } from "@/api/tauri";
import { usePluginsStore } from "@/stores/plugins";
import { useLayoutStore } from "@/stores/layout";
import { useI18n } from "@/i18n";
import { openExternal } from "@/utils/external";
import type { SoundfontStoreMatch } from "@/api/types";

export function useSoundfontStore() {
  const plugins = usePluginsStore();
  const layout = useLayoutStore();
  const i18n = useI18n();

  const results = ref<SoundfontStoreMatch[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);
  /** 正在安装的 slug（一次只装一个：下载很重，并发只会让进度更难懂）。 */
  const installing = ref<string | null>(null);

  async function search(query: string): Promise<void> {
    loading.value = true;
    error.value = null;
    try {
      results.value = await searchSoundfontStore(query.trim());
    } catch (err) {
      error.value = String(err);
      results.value = [];
    } finally {
      loading.value = false;
    }
  }

  /** 装一个库：下载/校验/解压都在后端，完成后刷新本机清单并标记「已装」。 */
  async function install(bank: SoundfontStoreMatch): Promise<boolean> {
    if (!bank.installable || installing.value) return false;
    installing.value = bank.slug;
    try {
      const outcome = await installSoundfontFromStore(bank.slug);
      await plugins.refreshSoundfonts();
      layout.notify(
        i18n.t("plugins.soundfontStoreInstalled", {
          name: bank.name,
          count: outcome.imported.length,
        }),
      );
      const row = results.value.find((item) => item.slug === bank.slug);
      if (row) row.installed = true;
      return true;
    } catch (err) {
      layout.notify(i18n.t("error.load", { error: String(err) }));
      return false;
    } finally {
      installing.value = null;
    }
  }

  /** 不能一键装的条目：把用户送到作者页面。 */
  function openPage(bank: SoundfontStoreMatch): void {
    if (bank.homepage) void openExternal(bank.homepage);
  }

  return { results, loading, error, installing, search, install, openPage };
}
