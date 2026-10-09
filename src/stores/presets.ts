/**
 * 用户预设 store：把「某个插件当前的参数」存成一条命名预设，之后一键切回。
 *
 * 出厂预设（内置合成器那几组）是随包发布的常量；用户预设存在后端（应用配置目录的
 * `presets.json`，跨工程）。这里把两者合并成同一份「可选预设」列表给界面用。
 */
import { defineStore } from "pinia";
import { ref } from "vue";

import { deletePreset, listPresets, savePreset, updatePreset } from "@/api/tauri";
import type { EffectSpec, Preset } from "@/api/types";
import { SYNTH_FACTORY_PRESETS, SYNTH_ID, canonicalPluginId, fullSynthParams } from "@/data/plugins";
import { useI18n } from "@/i18n";

/** 界面上的一条可选预设。 */
export interface PresetOption {
  id: string;
  name: string;
  note: string;
  /** 出厂预设的类别 slug（`8bit|guitar|bass|drums|strings|winds|pad|fx`）；用户自存预设留空。 */
  category?: string;
  params: Record<string, number>;
  /** 预设自带的效果链（出厂预设才有；没写 = 空链）。 */
  effects: EffectSpec[];
  /** 出厂预设：不能删，但可以「另存为」出自己的版本。 */
  factory: boolean;
}

/** 两组参数是否完全一致（用来判断「当前用的是哪个预设」）。 */
function sameParams(a: Record<string, number>, b: Record<string, number>): boolean {
  const keys = Object.keys(a);
  if (keys.length !== Object.keys(b).length) return false;
  return keys.every((k) => a[k] === b[k]);
}

/** 两条效果链是否一致（顺序、id、参数都要一样）。 */
function sameChain(a: EffectSpec[], b: EffectSpec[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((spec, i) => {
    const other = b[i];
    return !!other && spec.plugin_id === other.plugin_id && sameParams(spec.params, other.params);
  });
}

export const usePresetsStore = defineStore("presets", () => {
  const { t } = useI18n();
  const user = ref<Preset[]>([]);
  const loaded = ref(false);
  const loading = ref(false);

  /** 从后端拉一次用户预设（启动时调用一次即可）。 */
  async function refresh(): Promise<void> {
    loading.value = true;
    try {
      user.value = await listPresets();
      loaded.value = true;
    } finally {
      loading.value = false;
    }
  }

  /** 某个插件可选的预设：出厂（只有内置合成器有）+ 用户自存的。 */
  function forPlugin(pluginId: string | undefined | null): PresetOption[] {
    if (!pluginId) return [];
    const id = canonicalPluginId(pluginId);
    const factory: PresetOption[] =
      id === SYNTH_ID
        ? SYNTH_FACTORY_PRESETS.map((p) => ({
            id: p.id,
            name: t(p.name),
            note: t(p.note),
            category: p.category,
            // 出厂预设是「补丁」，这里摊成完整参数集：切预设要一次把整台合成器设好。
            params: fullSynthParams(p.params),
            effects: p.effects ?? [],
            factory: true,
          }))
        : [];
    // 用户自存预设没有类别（`category` 留空），界面按「我的预设」分组。
    const mine: PresetOption[] = user.value
      .filter((p) => canonicalPluginId(p.plugin_id) === id)
      .map((p, i) => ({
        id: p.id,
        name: p.name,
        note: i === 0 ? t("preset.mine") : "",
        params: p.params,
        // 自存预设只存乐器参数，不带效果链。
        effects: [],
        factory: false,
      }));
    return [...factory, ...mine];
  }

  /**
   * 当前参数命中的预设（没有就返回 null）。
   *
   * 传了 `chain` 就**连效果链一起比对**：带混响的铺底与「同一组参数 + 手动加的效果」不是同一条预设。
   * 不传则只比参数（调用方拿不到效果链时的退路）。
   *
   * 参数完全相同的可能不止一条（比如「另存为」出来的副本），此时取**最后**一条：
   * 自存预设排在出厂之后，所以用户自己刚存的那条会被选中——否则它既显示不出来也删不掉。
   */
  function match(
    pluginId: string | undefined | null,
    params: Record<string, number>,
    chain?: EffectSpec[],
  ): PresetOption | null {
    const options = forPlugin(pluginId);
    let hit: PresetOption | null = null;
    for (const option of options) {
      if (!sameParams(option.params, params)) continue;
      if (chain && !sameChain(option.effects, chain)) continue;
      hit = option;
    }
    if (hit) return hit;
    // 老工程：轨道直接挂在某个「预设音色」的 id 上、参数是空的（包络来自那个预设的默认值）。
    if (Object.keys(params).length === 0 && pluginId) {
      return options.find((p) => p.id === pluginId) ?? null;
    }
    return null;
  }

  /** 把当前参数存成一条新预设。 */
  async function save(
    name: string,
    pluginId: string,
    params: Record<string, number>,
  ): Promise<PresetOption> {
    const created = await savePreset(name, canonicalPluginId(pluginId), params);
    await refresh();
    return {
      id: created.id,
      name: created.name,
      note: "",
      params: created.params,
      // 自存预设只存乐器参数，不带效果链。
      effects: [],
      factory: false,
    };
  }

  /** 用当前参数覆盖一条自存预设。 */
  async function overwrite(id: string, name: string, params: Record<string, number>): Promise<void> {
    await updatePreset(id, name, params);
    await refresh();
  }

  /** 删除一条自存预设。 */
  async function remove(id: string): Promise<void> {
    await deletePreset(id);
    await refresh();
  }

  return { user, loaded, loading, refresh, forPlugin, match, save, overwrite, remove };
});
