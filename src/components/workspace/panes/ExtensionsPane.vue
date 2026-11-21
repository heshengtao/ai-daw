<script setup lang="ts">
/**
 * 应用扩展面板：**已安装**与**发现**两件事分开做。
 *
 * 视觉与交互沿用「下载管理器」（`ManagerPane`）那一套：同一套卡片 / 标签 / 胶囊类名、
 * 同一套 MD3 token。索引源 / registry 这类**设置项不放在这里**——面板只留一个跳转胶囊，
 * 真正的编辑在「设置 → 应用扩展」。
 *
 * 这里的「应用扩展」是 npm 分发的应用级插件（MCP 工具 / 面板 / 内容包），不是 VST3/CLAP
 * 音频插件；音频插件在「插件」与「下载管理器」面板里。
 */
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import {
  AdjustmentsHorizontalIcon,
  ArrowPathIcon,
  ArrowTopRightOnSquareIcon,
  FolderOpenIcon,
  PuzzlePieceIcon,
  ExclamationTriangleIcon,
} from "@heroicons/vue/24/outline";
import { useExtensionsStore, type InstalledExtension, type MarketEntry } from "@/stores/extensions";
import { useLayoutStore } from "@/stores/layout";
import { pickDirectory } from "@/api/dialogs";
import { useI18n } from "@/i18n";

const ext = useExtensionsStore();
const layout = useLayoutStore();
const router = useRouter();
const i18n = useI18n();

const tab = ref<"installed" | "discover">("installed");
const query = ref("");
const busy = ref<string | null>(null);
const notice = ref<string | null>(null);

/** 已装扩展里可更新的：包名 → 目标版本。 */
const updateFor = computed(() => {
  const map = new Map<string, string>();
  for (const entry of ext.market?.entries ?? []) {
    if (entry.installed.status === "update-available" && entry.defaultVersion) {
      map.set(entry.name, entry.defaultVersion);
    }
  }
  return map;
});

const marketCount = computed(() => ext.market?.total ?? 0);

/** 索引状态：没有源 / 在线 / 缓存 / 有源不可达。 */
const indexStatus = computed<"none" | "online" | "cached" | "error">(() => {
  const sources = ext.market?.sources ?? [];
  if (sources.length === 0 && ext.indexes.length === 0) return "none";
  if (sources.some((source) => source.error)) return "error";
  if (sources.length > 0 && sources.every((source) => source.fromCache)) return "cached";
  return sources.length > 0 ? "online" : "none";
});

const indexLabel = computed(() => {
  switch (indexStatus.value) {
    case "online":
      return i18n.t("extensions.indexOnline");
    case "cached":
      return i18n.t("extensions.indexCached");
    case "error":
      return i18n.t("extensions.indexError");
    default:
      return i18n.t("extensions.indexNone");
  }
});

function stateKey(item: InstalledExtension): string {
  return item.state;
}

function sourceLine(item: InstalledExtension): string {
  if (item.source.type === "npm") return item.source.registry ?? item.source.index ?? "npm";
  return item.source.path ?? item.source.type;
}

function openSettings(): void {
  void router.push({ path: "/settings", query: { section: "extensions" } });
}

async function run(key: string, action: () => Promise<void>): Promise<void> {
  busy.value = key;
  notice.value = null;
  try {
    await action();
  } catch (error) {
    notice.value = error instanceof Error ? error.message : String(error);
  } finally {
    busy.value = null;
  }
}

function refreshAll(): void {
  void run("reload", async () => {
    await ext.refresh();
    await ext.loadMarket({ refresh: true, reset: true });
  });
}

function importFolder(): void {
  void run("import", async () => {
    const path = await pickDirectory();
    if (!path) return;
    await ext.installLocal(path);
    layout.notify(i18n.t("extensions.imported"));
  });
}

function install(name: string, version: string | null, index: string | null): void {
  if (!window.confirm(i18n.t("extensions.confirmInstall", { name, version: version ?? "", index: index ?? "" }))) {
    return;
  }
  void run(`install:${name}`, () => ext.install(name, version, index));
}

function installEntry(entry: MarketEntry): void {
  install(entry.name, entry.defaultVersion, entry.indexUrl);
}

function toggle(item: InstalledExtension): void {
  void run(`toggle:${item.name}`, () => ext.setEnabled(item.name, !item.enabled));
}

function reload(name: string): void {
  void run(`reload:${name}`, () => ext.reload(name));
}

function remove(name: string): void {
  if (!window.confirm(i18n.t("extensions.confirmUninstall", { name }))) return;
  void run(`remove:${name}`, () => ext.uninstall(name));
}

function search(): void {
  void run("search", () => ext.searchMarket(query.value));
}

function openHomepage(url: string): void {
  window.open(url, "_blank", "noreferrer");
}

onMounted(() => {
  void ext.refresh();
  if (!ext.market) void ext.loadMarket();
});
</script>

<template>
  <div class="pane">
    <header class="head">
      <div class="head-main">
        <h2 class="title">{{ i18n.t("extensions.title") }}</h2>
        <p class="sub">
          {{
            tab === "installed"
              ? i18n.t("extensions.subInstalled", { count: ext.installed.length })
              : i18n.t("extensions.subDiscover", { count: marketCount })
          }}
        </p>
      </div>
      <button
        class="icon-btn"
        :title="i18n.t('extensions.refresh')"
        :aria-label="i18n.t('extensions.refresh')"
        :disabled="busy !== null || ext.marketBusy"
        @click="refreshAll"
      >
        <ArrowPathIcon class="w-[16px] h-[16px]" :class="{ spinning: busy === 'reload' || ext.marketBusy }" />
      </button>
    </header>

    <div class="manager-bar">
      <div class="tabs" role="tablist">
        <button
          class="tab"
          :class="{ on: tab === 'installed' }"
          role="tab"
          :aria-selected="tab === 'installed'"
          @click="tab = 'installed'"
        >
          {{ i18n.t("extensions.tabInstalled") }}
          <span v-if="ext.installed.length" class="tab-count">{{ ext.installed.length }}</span>
        </button>
        <button
          class="tab"
          :class="{ on: tab === 'discover' }"
          role="tab"
          :aria-selected="tab === 'discover'"
          @click="tab = 'discover'"
        >
          {{ i18n.t("extensions.tabDiscover") }}
          <span v-if="marketCount" class="tab-count">{{ marketCount }}</span>
        </button>
      </div>
      <button v-if="tab === 'installed'" class="import-btn" type="button" :disabled="busy !== null" @click="importFolder">
        <FolderOpenIcon class="w-[14px] h-[14px]" />
        {{ i18n.t("extensions.installFolder") }}
      </button>
    </div>

    <p v-if="notice" class="banner">
      <ExclamationTriangleIcon class="banner-icon" />
      <span class="banner-text">{{ notice }}</span>
    </p>

    <!-- 已安装 -->
    <template v-if="tab === 'installed'">
      <div v-if="ext.installed.length === 0" class="empty">
        <PuzzlePieceIcon class="empty-icon" />
        <p class="empty-text">{{ i18n.t("extensions.empty") }}</p>
      </div>
      <ul v-else class="list">
        <li v-for="item in ext.installed" :key="item.name" class="card col">
          <div class="card-top">
            <span class="card-name">{{ item.name }}</span>
            <span class="badge">v{{ item.version }}</span>
            <span class="badge" :class="item.state === 'loaded' ? 'installed' : item.state === 'failed' ? 'danger' : 'local'">
              {{ i18n.t(`extensions.state.${stateKey(item)}`) }}
            </span>
            <span class="badge local">
              {{ i18n.t(item.source.type === "npm" ? "extensions.kindManaged" : "extensions.kindLocal") }}
            </span>
          </div>
          <p class="card-path" :title="sourceLine(item)">{{ sourceLine(item) }}</p>
          <p v-if="item.error" class="card-desc error-text">{{ item.error }}</p>
          <p v-if="item.tools.length || item.panes.length" class="card-meta">
            <span v-if="item.tools.length">{{ i18n.t("extensions.toolsCount", { n: item.tools.length }) }}</span>
            <span v-if="item.panes.length">{{ i18n.t("extensions.panesCount", { n: item.panes.length }) }}</span>
          </p>
          <div class="card-actions">
            <button
              v-if="updateFor.get(item.name)"
              class="mini-btn primary"
              type="button"
              :disabled="busy !== null"
              @click="install(item.name, updateFor.get(item.name) ?? null, null)"
            >
              {{ i18n.t("extensions.updateTo", { version: updateFor.get(item.name) ?? "" }) }}
            </button>
            <button class="mini-btn" type="button" :disabled="busy !== null || !item.enabled" @click="reload(item.name)">
              {{ i18n.t("extensions.reload") }}
            </button>
            <button class="mini-btn" type="button" :disabled="busy !== null" @click="toggle(item)">
              {{ item.enabled ? i18n.t("extensions.disable") : i18n.t("extensions.enable") }}
            </button>
            <button class="mini-btn danger" type="button" :disabled="busy !== null" @click="remove(item.name)">
              {{ i18n.t("extensions.remove") }}
            </button>
          </div>
        </li>
      </ul>
    </template>

    <!-- 发现 -->
    <template v-else>
      <div class="filters">
        <input
          v-model="query"
          class="search"
          type="search"
          :placeholder="i18n.t('extensions.search')"
          :aria-label="i18n.t('extensions.search')"
          @keyup.enter="search"
        />
        <div class="chips" role="group" :aria-label="i18n.t('extensions.indexSource')">
          <span class="chip status-chip" :class="indexStatus">{{ indexLabel }}</span>
          <button class="chip source-chip" type="button" @click="openSettings">
            <AdjustmentsHorizontalIcon class="w-[13px] h-[13px]" />
            {{ i18n.t("extensions.sourceSettings") }}
          </button>
        </div>
      </div>

      <p v-if="ext.marketError" class="banner">
        <ExclamationTriangleIcon class="banner-icon" />
        <span class="banner-text">{{ ext.marketError }}</span>
      </p>
      <p v-else-if="ext.marketBusy" class="empty">
        <span class="empty-text">{{ i18n.t("extensions.loading") }}</span>
      </p>
      <div v-else-if="!ext.market || ext.market.entries.length === 0" class="empty">
        <PuzzlePieceIcon class="empty-icon" />
        <p class="empty-text">{{ i18n.t("extensions.marketEmpty") }}</p>
      </div>

      <ul v-else class="list">
        <li v-for="entry in ext.market.entries" :key="`${entry.indexUrl}:${entry.name}`" class="card col">
          <div class="card-top">
            <span class="card-name">{{ entry.title || entry.name }}</span>
            <span v-if="entry.defaultVersion" class="badge">v{{ entry.defaultVersion }}</span>
            <span v-if="entry.installed.status === 'installed'" class="badge installed">
              {{ i18n.t("extensions.marketInstalled") }}
            </span>
            <span v-if="entry.installed.status === 'update-available'" class="badge verified">
              {{ i18n.t("extensions.marketUpdate") }}
            </span>
            <span v-if="!entry.defaultVersion" class="badge danger">{{ i18n.t("extensions.incompatible") }}</span>
          </div>
          <p class="card-meta">
            <span>{{ entry.name }}</span>
            <span v-if="entry.author">{{ entry.author }}</span>
          </p>
          <p v-if="entry.description" class="card-desc">{{ entry.description }}</p>
          <div class="card-actions">
            <button class="mini-btn" type="button" :disabled="busy !== null || !entry.defaultVersion || entry.installed.status === 'installed'" @click="installEntry(entry)">
              {{
                entry.installed.status === "update-available"
                  ? i18n.t("extensions.updateTo", { version: entry.defaultVersion ?? "" })
                  : i18n.t("extensions.install")
              }}
            </button>
            <button v-if="entry.homepage" class="mini-btn" type="button" @click="openHomepage(entry.homepage)">
              <ArrowTopRightOnSquareIcon class="w-[12px] h-[12px]" />
              {{ i18n.t("extensions.homepage") }}
            </button>
          </div>
        </li>
      </ul>
    </template>
  </div>
</template>

<style scoped>
/* 视觉基线来自「下载管理器」（ManagerPane）：同一套类名与 MD3 token。 */
.pane {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding-bottom: 12px;
}
.head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 8px;
  padding: 12px 12px 0;
}
.head-main {
  min-width: 0;
}
.title {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
  color: rgb(var(--md-on-surface));
}
.sub {
  margin: 3px 0 0;
  font-size: 11px;
  color: rgb(var(--md-on-surface-muted));
}
.icon-btn {
  flex: 0 0 auto;
  width: 30px;
  height: 30px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
}
.icon-btn:hover:not(:disabled) {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.icon-btn:disabled {
  opacity: 0.4;
  cursor: default;
}
.manager-bar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 10px 12px 0;
}
.tabs {
  display: flex;
  align-items: center;
  gap: 4px;
  flex-wrap: wrap;
  min-width: 0;
}
.tab {
  height: 28px;
  padding: 0 12px;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 12px;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  transition: background-color 0.14s ease, color 0.14s ease;
}
.tab:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.tab.on {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.tab-count {
  font-size: 10px;
  padding: 1px 6px;
  border-radius: 999px;
  background-color: rgb(var(--md-primary) / 0.16);
  color: rgb(var(--md-primary));
}
.import-btn {
  flex: 0 0 auto;
  height: 30px;
  padding: 0 14px;
  border: 0;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  white-space: nowrap;
}
.import-btn:disabled {
  opacity: 0.5;
  cursor: default;
}
.spinning {
  animation: spin 0.9s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
.banner {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 10px 12px 0;
  padding: 8px 10px;
  border-radius: 10px;
  background-color: rgb(var(--md-error) / 0.12);
  color: rgb(var(--md-error));
}
.banner-icon {
  width: 15px;
  height: 15px;
  flex: 0 0 auto;
}
.banner-text {
  flex: 1 1 auto;
  font-size: 11.5px;
  min-width: 0;
  overflow-wrap: anywhere;
}
.filters {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px 12px 4px;
}
.search {
  height: 32px;
  padding: 0 12px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: var(--app-inset);
  color: rgb(var(--md-on-surface));
  font-size: 12px;
}
.search:focus {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 0;
}
.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}
.chip {
  height: 24px;
  padding: 0 10px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 11px;
  white-space: nowrap;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  transition: background-color 0.14s ease, color 0.14s ease, border-color 0.14s ease;
}
.chip:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.status-chip.online {
  border-color: transparent;
  background-color: rgb(var(--md-primary) / 0.12);
  color: rgb(var(--md-primary));
}
.status-chip.cached {
  border-color: transparent;
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.status-chip.error {
  border-color: transparent;
  background-color: rgb(var(--md-error) / 0.12);
  color: rgb(var(--md-error));
}
.list {
  list-style: none;
  margin: 0;
  padding: 10px 12px 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.card {
  position: relative;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 12px;
  border-radius: 14px;
  background-color: var(--app-inset);
}
.card.col {
  flex-direction: column;
  align-items: stretch;
  gap: 4px;
}
.card-top {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
}
.card-name {
  font-size: 13px;
  font-weight: 600;
  color: rgb(var(--md-on-surface));
}
.badge {
  font-size: 10px;
  padding: 2px 8px;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
}
.badge.installed {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.badge.verified {
  background-color: rgb(var(--md-primary) / 0.16);
  color: rgb(var(--md-primary));
}
.badge.danger {
  background-color: rgb(var(--md-error) / 0.12);
  color: rgb(var(--md-error));
}
.badge.local {
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface-muted));
}
.card-path {
  margin: 2px 0 0;
  font-size: 10.5px;
  color: rgb(var(--md-on-surface-muted));
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.card-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  margin: 2px 0 0;
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
}
.card-desc {
  margin: 2px 0 0;
  font-size: 11.5px;
  line-height: 1.45;
  color: rgb(var(--md-on-surface-variant));
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.error-text {
  color: rgb(var(--md-error));
}
.card-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 6px;
}
.mini-btn {
  flex: 0 0 auto;
  height: 26px;
  padding: 0 12px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 11px;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  white-space: nowrap;
  transition: background-color 0.14s ease, color 0.14s ease;
}
.mini-btn:hover:not(:disabled) {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.mini-btn:disabled {
  opacity: 0.45;
  cursor: default;
}
.mini-btn.primary {
  border-color: transparent;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}
.mini-btn.danger {
  border-color: rgb(var(--md-error) / 0.5);
  color: rgb(var(--md-error));
}
.empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 44px 16px;
  color: rgb(var(--md-on-surface-muted));
}
.empty-icon {
  width: 26px;
  height: 26px;
}
.empty-text {
  margin: 0;
  font-size: 13px;
  color: rgb(var(--md-on-surface-variant));
  text-align: center;
}
</style>
