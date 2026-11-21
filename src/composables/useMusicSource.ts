/**
 * 「音乐生成」的来源下拉：**本地权重 + 云端官方 API** 共用一份列表。
 *
 * 「AI 写歌」卡片与「灵感」页要的是同一个东西——一个下拉里既能挑本地 YuE2 / ACE-Step，
 * 又能挑已配置的云端服务（Suno / Mureka / Lyria …），外加一个「设置音乐 API…」入口。
 * 以前这段拼装写在 `AiPanel` 里，灵感页再抄一遍就会两处漂移：加一个服务要改两处、
 * 两处的分组标题还会不一致。这里收成一处。
 *
 * 约定：
 * - 云端选项的 id 带 `cloud:` 前缀，本地选项直接是模型 id —— 一个 `string | null` 就能
 *   同时表达两种来源，`<Md3Select>` 不用知道这回事；
 * - 「设置音乐 API…」是 `action` 项：点了只跳设置页，不改变当前选中；
 * - 选云端 = 把 `cloudMusic.selectedId` 指过去、并清掉「本地想用哪个」的记性？不——
 *   两者各自记住：切回本地时还是上次那一档。
 */
import { computed } from "vue";
import { useRouter } from "vue-router";
import type { SelectOption } from "@/components/common/Md3Select.vue";
import { useI18n } from "@/i18n";
import { useCloudMusicStore } from "@/stores/cloudMusic";
import { useModelChoice } from "@/composables/useModelChoice";
import { useModelsStore } from "@/stores/models";

/** 云端选项的 id 前缀（与本地模型 id 不会撞：模型 id 里没有冒号）。 */
export const CLOUD_PREFIX = "cloud:";

/** 「设置音乐 API…」这个动作项的 id。 */
const CONFIG_ACTION = "__configure_music__";

export function useMusicSource() {
  const router = useRouter();
  const i18n = useI18n();
  const models = useModelsStore();
  const cloudMusic = useCloudMusicStore();
  /** 本地权重：用户选过哪一档就记哪一档（未装也记，界面负责提示下载）。 */
  const local = useModelChoice("compose", () =>
    models.catalog.filter((m) => m.feature === "compose"),
  );

  /** 当前生效的云端服务；`null` = 走本地。 */
  const provider = computed(
    () => cloudMusic.enabledProviders.find((p) => p.id === cloudMusic.selectedId) ?? null,
  );

  const isCloud = computed(() => provider.value !== null);

  /** 本地当前这一档权重。 */
  const localModel = computed(() => local.current.value);

  /** 给下拉用的统一值。 */
  const value = computed<string | null>({
    get: () => (provider.value ? `${CLOUD_PREFIX}${provider.value.id}` : local.id.value),
    set: (next) => pick(next ?? ""),
  });

  /** 下拉列表：设置入口 → 本地模型组 → 云端服务组。 */
  const options = computed<SelectOption[]>(() => {
    const list: SelectOption[] = [
      {
        id: CONFIG_ACTION,
        label: i18n.t("ai.musicApiConfigure"),
        hint: i18n.t("ai.musicApiConfigureHint"),
        action: true,
      },
    ];
    for (const option of local.selectOptions.value) {
      list.push({ ...option, group: i18n.t("ai.musicGroupLocal") });
    }
    for (const item of cloudMusic.enabledProviders) {
      list.push({
        id: `${CLOUD_PREFIX}${item.id}`,
        label: item.name,
        hint: item.model || i18n.t("settings.music.defaultModel"),
        group: i18n.t("ai.musicGroupCloud"),
      });
    }
    return list;
  });

  /** 选中一项：云端切服务，本地切权重档。 */
  function pick(next: string): void {
    if (next.startsWith(CLOUD_PREFIX)) {
      cloudMusic.select(next.slice(CLOUD_PREFIX.length));
      return;
    }
    if (!next) return;
    cloudMusic.select("");
    local.id.value = next;
  }

  /** `action` 项：去设置的音乐分区。 */
  function onAction(id?: string): void {
    if (id !== CONFIG_ACTION) return;
    void router.push({ name: "settings", query: { section: "music" } });
  }

  /** 云端的「有没有服务可选」：一个都没有时界面要说清楚，而不是给个空下拉。 */
  const hasCloud = computed(() => cloudMusic.enabledProviders.length > 0);

  return {
    cloudMusic,
    local,
    localModel,
    localOptions: local.selectOptions,
    provider,
    isCloud,
    hasCloud,
    value,
    options,
    pick,
    onAction,
    /** 拉一次云端服务配置（幂等）。 */
    load: () => cloudMusic.load(),
    /** 保存到设置页回来：强制刷新。 */
    reload: () => cloudMusic.load(true),
  };
}
