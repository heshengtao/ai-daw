/**
 * 插件清单 store：**界面侧唯一的插件数据源**。
 *
 * 它把两部分合并成一份视图：
 * - 内置音色（`data/plugins.ts`，随包发布、立即可用）；
 * - 外部插件（Tauri 后端扫描到的 VST3 / CLAP）。
 *
 * 参数表也是懒加载的：内置插件在本地就有；外部插件第一次需要时向后端要一次，之后缓存。
 */
import { defineStore } from "pinia";
import { computed, reactive, ref } from "vue";

import {
  BUILTIN_PARAMS,
  BUILTIN_PLUGINS,
  canonicalPluginId,
  type ParamSpec,
  type PluginInfo,
} from "@/data/plugins";
import { listPlugins, pluginParams, rescanPlugins } from "@/api/tauri";
import { useI18n } from "@/i18n";
import type { BackendParam, CatalogPlugin } from "@/api/types";

/**
 * 内置条目的名字 / 说明 / 分类存的是 **i18n key**（外部插件的名字后端给，不翻译）。
 * 在 store 的读取边界处翻译一次，显示侧照旧用 `info.name` / `p.category`。
 */
function localize(p: PluginInfo): PluginInfo {
  const { t } = useI18n();
  return p.builtin ? { ...p, name: t(p.name), note: t(p.note), category: t(p.category) } : p;
}

/** 外部插件条目的展示分类（i18n key）。 */
const EXTERNAL_INSTRUMENT_CATEGORY = "pluginCategory.externalInstrument";
const EXTERNAL_EFFECT_CATEGORY = "pluginCategory.externalEffect";

/** 后端参数 → 界面参数（补上步长/小数位的界面默认值）。 */
function toSpec(raw: BackendParam): ParamSpec {
  const span = raw.max - raw.min;
  return {
    id: raw.id,
    name: raw.name,
    min: raw.min,
    max: raw.max,
    step: span > 0 ? span / 100 : 0.01,
    default: raw.default,
    digits: 2,
  };
}

/** 扫描结果 → 界面条目。 */
function toInfo(p: CatalogPlugin): PluginInfo {
  const { t } = useI18n();
  return {
    id: p.id,
    name: p.name,
    kind: p.kind,
    category: t(
      p.kind === "instrument" ? EXTERNAL_INSTRUMENT_CATEGORY : EXTERNAL_EFFECT_CATEGORY,
    ),
    builtin: false,
    loadable: true,
    tags: [p.format, p.vendor, p.name],
    note: `${p.format} · ${p.vendor}${p.has_editor ? ` · ${t("plugins.hasEditor")}` : ""}`,
    format: p.format,
    hasEditor: p.has_editor,
  };
}

export const usePluginsStore = defineStore("plugins", () => {
  /** 后端扫描到的外部插件。 */
  const scanned = ref<CatalogPlugin[]>([]);
  /** 是否正在扫描。 */
  const loading = ref(false);
  /** 参数表缓存（内置预填 + 外部懒加载）。 */
  const params = reactive<Record<string, ParamSpec[]>>({ ...BUILTIN_PARAMS });

  const external = computed<PluginInfo[]>(() => scanned.value.map(toInfo));
  const all = computed<PluginInfo[]>(() => [...BUILTIN_PLUGINS.map(localize), ...external.value]);
  const instruments = computed<PluginInfo[]>(() => all.value.filter((p) => p.kind === "instrument"));
  const effects = computed<PluginInfo[]>(() => all.value.filter((p) => p.kind === "effect"));
  const categories = computed<string[]>(() => [...new Set(all.value.map((p) => p.category))]);

  /**
   * 条目信息。
   *
   * 老的「预设音色」插件 id（`builtin.synth.lead` 等）现在归内置合成器，所以先归一——
   * 否则老工程里的轨道会既没有名字也没有参数表。
   */
  function info(pluginId: string | undefined | null): PluginInfo | null {
    if (!pluginId) return null;
    const id = canonicalPluginId(pluginId);
    return all.value.find((p) => p.id === id) ?? null;
  }

  function name(pluginId: string | undefined | null): string {
    if (!pluginId) return useI18n().t("instrument.none");
    return info(pluginId)?.name ?? pluginId;
  }

  /** 参数表（同步；外部插件可能还没加载完，此时为空数组）。 */
  function paramsOf(pluginId: string | undefined | null): ParamSpec[] {
    if (!pluginId) return [];
    return params[canonicalPluginId(pluginId)] ?? [];
  }

  /** 确保某个插件的参数表已加载（外部插件会向后端要一次）。 */
  async function ensureParams(pluginId: string | undefined | null): Promise<void> {
    if (!pluginId) return;
    const id = canonicalPluginId(pluginId);
    if (params[id]) return;
    try {
      const raw = await pluginParams(id);
      params[id] = raw.map(toSpec);
    } catch {
      // 加载失败就记成空表，避免每次都重试。
      params[id] = [];
    }
  }

  /** 拉取一次插件目录。 */
  async function refresh(): Promise<void> {
    loading.value = true;
    try {
      scanned.value = await listPlugins();
    } finally {
      loading.value = false;
    }
  }

  /** 让后端重新扫描本机插件（会 dlopen 每个插件，明显更慢）。 */
  async function rescan(): Promise<void> {
    loading.value = true;
    try {
      scanned.value = await rescanPlugins();
    } finally {
      loading.value = false;
    }
  }

  return {
    scanned,
    loading,
    all,
    instruments,
    effects,
    categories,
    info,
    name,
    paramsOf,
    ensureParams,
    refresh,
    rescan,
  };
});
