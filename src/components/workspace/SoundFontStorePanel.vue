<script setup lang="ts">
/**
 * 在线 SoundFont 音色库列表：搜索 → 一键安装 → 已装标记 / 打不开时给出口。
 *
 * 自包含（搜索词、结果、安装中状态都在组件里），插件面板和下载管理器共用同一份实现。
 * 数据来自 `ai-daw-soundfont-registry` 的静态索引：后端带 6 小时缓存，拉不到时退回随包快照。
 */
import { onMounted, ref, watch } from "vue";
import { installSoundfontFromStore, searchSoundfontStore } from "@/api/tauri";
import { usePluginsStore } from "@/stores/plugins";
import { useLayoutStore } from "@/stores/layout";
import { useI18n } from "@/i18n";
import { humanSize } from "@/utils/models";
import { openExternal } from "@/utils/external";
import type { SoundfontStoreMatch } from "@/api/types";

const props = withDefaults(
  defineProps<{
    /** 外部的搜索词（管理器的搜索框）；`showSearch=false` 时用它驱动搜索。 */
    query?: string;
    /** 是否渲染自己的搜索框。 */
    showSearch?: boolean;
    /** 挂载后是否立刻拉一次索引。 */
    autoLoad?: boolean;
  }>(),
  { query: "", showSearch: true, autoLoad: true },
);

const emit = defineEmits<{ (e: "installed", slug: string): void }>();

const plugins = usePluginsStore();
const layout = useLayoutStore();
const i18n = useI18n();

const ownQuery = ref(props.query);
const results = ref<SoundfontStoreMatch[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);
const installing = ref<string | null>(null);

const effectiveQuery = () => (props.showSearch ? ownQuery.value : props.query).trim();

async function search(): Promise<void> {
  loading.value = true;
  error.value = null;
  try {
    results.value = await searchSoundfontStore(effectiveQuery());
  } catch (err) {
    error.value = String(err);
    results.value = [];
  } finally {
    loading.value = false;
  }
}

// 管理器里共用顶部搜索框：输入停下来再查，别每个键都打一次索引。
let debounce: number | undefined;
watch(
  () => props.query,
  () => {
    if (props.showSearch) return;
    window.clearTimeout(debounce);
    debounce = window.setTimeout(() => void search(), 300);
  },
);

onMounted(() => {
  if (props.autoLoad) void search();
});

async function install(bank: SoundfontStoreMatch): Promise<void> {
  if (!bank.installable || installing.value) return;
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
    emit("installed", bank.slug);
  } catch (err) {
    layout.notify(i18n.t("error.load", { error: String(err) }));
  } finally {
    installing.value = null;
  }
}

function openPage(bank: SoundfontStoreMatch): void {
  if (bank.homepage) void openExternal(bank.homepage);
}
</script>

<template>
  <div class="sfp">
    <div v-if="showSearch" class="sfp-search">
      <input
        v-model="ownQuery"
        type="search"
        :placeholder="i18n.t('plugins.soundfontStoreSearchPlaceholder')"
        :aria-label="i18n.t('plugins.soundfontStoreSearchPlaceholder')"
        @keyup.enter="search"
      />
      <button :disabled="loading" @click="search">{{ i18n.t("common.search") }}</button>
    </div>

    <p v-if="loading" class="sfp-hint">{{ i18n.t("common.loading") }}</p>
    <p v-else-if="error" class="sfp-hint sfp-error">{{ error }}</p>
    <p v-else-if="results.length === 0" class="sfp-hint">
      {{ i18n.t("plugins.soundfontStoreEmpty") }}
    </p>

    <ul v-else class="sfp-list">
      <li v-for="bank in results" :key="bank.slug" class="sfp-item">
        <div class="sfp-main">
          <span class="sfp-name" :title="bank.description">{{ bank.name }}</span>
          <span class="sfp-note">
            {{ bank.license }}
            <template v-if="bank.size"> · {{ humanSize(bank.size) }}</template>
            <template v-if="bank.format"> · {{ bank.format.toUpperCase() }}</template>
            <template v-if="bank.presets"> · {{ i18n.t("plugins.soundfontPresets", { count: bank.presets }) }}</template>
          </span>
        </div>
        <span v-if="bank.installed" class="sfp-badge">
          {{ i18n.t("plugins.soundfontStoreInstalledBadge") }}
        </span>
        <!-- 不能一键装的（只给下载页）：把用户送到作者页面，而不是发一个灰按钮 -->
        <button
          v-else-if="!bank.installable && bank.homepage"
          class="sfp-btn"
          :title="bank.blocked ?? ''"
          @click="openPage(bank)"
        >
          {{ i18n.t("plugins.soundfontStoreOpen") }}
        </button>
        <button
          v-else
          class="sfp-btn"
          :disabled="!bank.installable || installing !== null"
          :title="bank.blocked ?? ''"
          @click="install(bank)"
        >
          {{
            installing === bank.slug
              ? i18n.t("plugins.soundfontStoreInstalling")
              : i18n.t("plugins.soundfontStoreInstall")
          }}
        </button>
      </li>
    </ul>
  </div>
</template>

<style scoped>
.sfp {
  display: flex;
  flex-direction: column;
  min-height: 0;
}
.sfp-search {
  display: flex;
  gap: 6px;
  margin: 8px 0;
}
.sfp-search input {
  flex: 1 1 auto;
  min-width: 0;
  height: 28px;
  padding: 0 8px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.8);
  border-radius: 8px;
  background: transparent;
  color: rgb(var(--md-on-surface));
  font-size: 12px;
}
.sfp-search input:focus {
  outline: 2px solid rgb(var(--md-primary) / 0.4);
  outline-offset: 0;
}
.sfp-search button {
  flex: 0 0 auto;
  height: 28px;
  padding: 0 10px;
  border: none;
  border-radius: 8px;
  background-color: rgb(var(--md-primary) / 0.12);
  color: rgb(var(--md-primary));
  font-size: 11.5px;
  cursor: pointer;
}
.sfp-search button:disabled {
  opacity: 0.5;
  cursor: default;
}
.sfp-hint {
  margin: 8px 0;
  font-size: 11.5px;
  line-height: 1.5;
  color: rgb(var(--md-on-surface-muted));
}
.sfp-error {
  color: rgb(var(--md-error));
}
.sfp-list {
  list-style: none;
  margin: 4px 0 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.sfp-item {
  display: flex;
  align-items: center;
  gap: 8px;
}
.sfp-main {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
  flex: 1 1 auto;
}
.sfp-name {
  font-size: 12.5px;
  color: rgb(var(--md-on-surface));
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sfp-note {
  font-size: 10.5px;
  color: rgb(var(--md-on-surface-muted));
}
.sfp-badge {
  flex: 0 0 auto;
  font-size: 10.5px;
  color: rgb(var(--md-on-surface-muted));
}
.sfp-btn {
  flex: 0 0 auto;
  height: 26px;
  padding: 0 10px;
  border: 1px solid rgb(var(--md-primary) / 0.5);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-primary));
  font-size: 11.5px;
  cursor: pointer;
}
.sfp-btn:hover:not(:disabled) {
  background-color: rgb(var(--md-primary) / 0.1);
}
.sfp-btn:disabled {
  opacity: 0.45;
  cursor: default;
}
</style>
