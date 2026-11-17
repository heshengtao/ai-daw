<script setup lang="ts">
/**
 * 插件面板里的「在线音色库」紧凑列表：自己的搜索框 + 一行一个库。
 *
 * 下载管理器那边用的是 `card` 卡片（与插件/模型同一套排版），所以这里只保留紧凑样式；
 * 两处的搜索与安装逻辑都来自 `useSoundfontStore`。
 */
import { onMounted, ref } from "vue";
import { useSoundfontStore } from "@/composables/useSoundfontStore";
import { useI18n } from "@/i18n";
import { humanSize } from "@/utils/models";

const i18n = useI18n();
const { results, loading, error, installing, search, install, openPage } = useSoundfontStore();
const query = ref("");

onMounted(() => void search(""));

function submit(): void {
  void search(query.value);
}
</script>

<template>
  <div class="sfp">
    <div class="sfp-search">
      <input
        v-model="query"
        type="search"
        :placeholder="i18n.t('plugins.soundfontStoreSearchPlaceholder')"
        :aria-label="i18n.t('plugins.soundfontStoreSearchPlaceholder')"
        @keyup.enter="submit"
      />
      <button :disabled="loading" @click="submit">{{ i18n.t("common.search") }}</button>
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
