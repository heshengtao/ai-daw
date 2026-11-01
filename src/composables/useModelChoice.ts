import { computed } from "vue";
import type { ModelStatus } from "@/api/types";
import { useI18n } from "@/i18n";
import { useModelChoiceStore } from "@/stores/modelChoice";
import { useModelsStore } from "@/stores/models";
import { humanSize, modelName } from "@/utils/models";

/**
 * 一个 AI 功能的「模型档」选择：**记住用户选的那一档**，卡片与子页共用同一份状态。
 *
 * 以前每个组件各自 `ref<string | null>(null)`，再配一个「优先选已下载」的 watcher：
 * 用户点了第二档，watcher 一看它还没下载又给弹回第一档——所以「选了白选」。
 * 这里把规则倒过来：
 *
 * - 用户选过就以它为准（装没装都算，没装就提示下载）；
 * - 只有它从清单里消失时才回落；
 * - 完全没选过 → 已装的优先 → 清单第一个。
 *
 * @param feature 功能键（首次选择后持久化在这个键下，例如 `sing` / `voice_swap`）
 * @param options 候选档（调用方按 `feature` 过滤；顺序即清单顺序）
 */
export function useModelChoice(feature: string, options: () => ModelStatus[]) {
  const store = useModelChoiceStore();
  const models = useModelsStore();
  const i18n = useI18n();

  const list = computed(options);

  /** 用户选过、且清单里还有的那一档。 */
  const chosen = computed(() => {
    const saved = store.get(feature);
    return saved ? (list.value.find((m) => m.id === saved) ?? null) : null;
  });

  /** 实际生效的那一档：用户选的 → 已装的 → 第一个。 */
  const current = computed(
    () => chosen.value ?? list.value.find((m) => m.installed) ?? list.value[0] ?? null,
  );

  /** 给 `v-model` 用的可写 id（写回持久化的选择）。 */
  const id = computed<string | null>({
    get: () => current.value?.id ?? null,
    set: (value) => store.set(feature, value),
  });

  const ready = computed(() => current.value?.installed ?? false);
  const downloading = computed(() =>
    current.value ? models.installing(current.value.id) : false,
  );

  /** 下拉选项：名称 + 「已下载 / 需下载 · 体积」。 */
  const selectOptions = computed(() =>
    list.value.map((m) => ({
      id: m.id,
      label: modelName(m),
      hint: m.installed
        ? i18n.t("ai.modelReady")
        : i18n.t("ai.modelNeedsDownload", { size: humanSize(m.approx_bytes) }),
    })),
  );

  return { list, current, id, ready, downloading, selectOptions };
}
