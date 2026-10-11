<script setup lang="ts">
/**
 * 插件管理器面板：管理插件的**文件本身**。
 *
 * 两个标签：
 * - **已安装**：本管理器装过的东西（注册表安装 / 本地导入），可删除；注册表里有更新版本时给「更新」。
 * - **可下载**：开放注册表里能装进本 DAW 的插件（VST3 / CLAP），可搜索、可按类型与兼容性筛选。
 *
 * 装完 / 删完由后端重扫插件库，结果经 `engine://plugins` 回到「插件」面板——本面板不直接改那份清单。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import {
  ArrowDownTrayIcon,
  ArrowPathIcon,
  ArrowTopRightOnSquareIcon,
  ArrowUpTrayIcon,
  ExclamationTriangleIcon,
  FolderOpenIcon,
  LinkIcon,
  TrashIcon,
} from "@heroicons/vue/24/outline";
import { managerResolveUrl, modelResolveUrl } from "@/api/tauri";
import { useManagerStore } from "@/stores/manager";
import { useModelsStore } from "@/stores/models";
import { usePluginsStore } from "@/stores/plugins";
import { useLayoutStore } from "@/stores/layout";
import { useI18n } from "@/i18n";
import { pickPluginToImport } from "@/api/dialogs";
import type { ManagerFile, ManagerInstalled, ManagerPlugin } from "@/api/types";

const manager = useManagerStore();
const models = useModelsStore();
const plugins = usePluginsStore();
const layout = useLayoutStore();
const i18n = useI18n();

type Tab = "installed" | "foreign" | "available";
const tab = ref<Tab>("installed");
const query = ref("");
/** 只看当前系统能装的（默认开：注册表很大，先给能用的）。 */
const compatibleOnly = ref(true);
const kindFilter = ref<"all" | "instrument" | "effect" | "model">("all");
/** 待确认删除的记录 key。 */
const confirming = ref<string | null>(null);
/** 待确认删除的模型 id。 */
const confirmingModel = ref<string | null>(null);
/** 进度订阅的取消函数（卸载时调用）。 */
let stopProgress: (() => void) | null = null;
/** 模型下载进度订阅的取消函数。 */
let stopModelProgress: (() => void) | null = null;

/** 这条卡片是不是正在被装（决定要不要画下边缘的进度条）。 */
function installing(slug: string): boolean {
  return manager.progress?.slug === slug;
}

/** 复制**实际会请求**的下载地址（经后端解析，与真去下载的 URL 一致）。 */
async function doCopyLink(url: string): Promise<void> {
  try {
    const resolved = await managerResolveUrl(url);
    await navigator.clipboard.writeText(resolved);
    layout.notify(i18n.t("manager.linkCopied"));
  } catch (err) {
    layout.notify(String(err));
  }
}

/** 复制模型的下载地址（经后端按当前镜像配置解析）。 */
async function doCopyModelLink(id: string): Promise<void> {
  try {
    const resolved = await modelResolveUrl(id);
    await navigator.clipboard.writeText(resolved);
    layout.notify(i18n.t("manager.linkCopied"));
  } catch (err) {
    layout.notify(String(err));
  }
}

/** 从别处（如 AI 面板点「开始分离」）请求聚焦到「可下载 → 模型」。 */
function applyFocus(): void {
  if (models.consumeFocus()) {
    tab.value = "available";
    kindFilter.value = "model";
  }
}

onMounted(() => {
  if (manager.registry.length === 0 && !manager.loading) void manager.load();
  stopProgress = manager.watchProgress();
  if (models.catalog.length === 0) void models.load();
  stopModelProgress = models.watchProgress();
  applyFocus();
});
// 面板已经开着时，聚焦请求通过 watcher 生效。
watch(
  () => models.focus,
  (v) => {
    if (v) applyFocus();
  },
);
onUnmounted(() => {
  stopProgress?.();
  stopModelProgress?.();
});

// ---- 展示辅助 ----

/** 人类可读体积。 */
function humanSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

function platformLabel(p: string): string {
  if (p === "mac") return "macOS";
  if (p === "win") return "Windows";
  if (p === "linux") return "Linux";
  return p;
}

/** 归档类型里那一段扩展名（`zip` / `tar.gz` / …）。 */
function archiveKind(url: string): string {
  const name = url.split(/[?#]/)[0].split("/").pop()?.toLowerCase() ?? "";
  if (name.endsWith(".tar.gz")) return "tar.gz";
  const dot = name.lastIndexOf(".");
  return dot >= 0 ? name.slice(dot + 1) : "";
}

/** 一行文件摘要：平台 · 架构 · 格式 · 类型。 */
function fileSummary(f: ManagerFile): string {
  const parts = [
    f.platforms.map(platformLabel).join("/") || i18n.t("manager.anyPlatform"),
    f.archs.join("/") || i18n.t("manager.anyArch"),
    f.contains.map((c) => c.toUpperCase()).join(" / "),
    f.kind === "installer" ? i18n.t("manager.kindInstaller") : archiveKind(f.url).toUpperCase(),
    humanSize(f.size),
  ].filter(Boolean);
  return parts.join(" · ");
}

/** 归档能否一键安装（安装器类型只能打开下载页）。 */
function isArchive(f: ManagerFile): boolean {
  return f.kind !== "installer";
}

// ---- 列表 ----

const available = computed<ManagerPlugin[]>(() => {
  // 「模型」筛选只列模型，插件一条都不出。
  if (kindFilter.value === "model") return [];
  const q = query.value.trim().toLowerCase();
  return manager.registry.filter((p) => {
    if (compatibleOnly.value && !p.files.some((f) => f.compatible)) return false;
    if (kindFilter.value !== "all" && p.kind !== kindFilter.value) return false;
    if (!q) return true;
    const hay = [p.name, p.author, p.description, p.slug, ...p.tags].join(" ").toLowerCase();
    return hay.includes(q);
  });
});

/** 「可下载」里会一并列出的模型（按同一份搜索框过滤）。
 *
 * 模型与插件共用「全部 / 乐器 / 效果 / 模型」这组筛选：选中「模型」只出模型，
 * 选中「全部」两者都出，选中「乐器 / 效果」则不出模型。兼容性筛选对模型不适用。 */
const visibleModels = computed(() => {
  if (kindFilter.value === "instrument" || kindFilter.value === "effect") return [];
  const q = query.value.trim().toLowerCase();
  return models.catalog.filter((m) => !q || m.name.toLowerCase().includes(q));
});

/** 「可下载」的计数（插件 + 模型）。 */
const availableShown = computed(() => available.value.length + visibleModels.value.length);
const availableTotal = computed(() => manager.registry.length + models.catalog.length);

/** 乐器 / 效果（未知为 `null`）。 */
type PluginKind = "instrument" | "effect";
function toKind(kind: string | undefined | null): PluginKind | null {
  return kind === "instrument" || kind === "effect" ? kind : null;
}

/** 一条安装记录的插件类型：优先注册表条目，其次按名字匹配已扫描的插件（本地导入也能标上）。 */
function installedKind(rec: ManagerInstalled, plugin: ManagerPlugin | undefined): PluginKind | null {
  const fromRegistry = toKind(plugin?.kind);
  if (fromRegistry) return fromRegistry;
  const name = rec.name.toLowerCase();
  return toKind(plugins.scanned.find((p) => p.name.toLowerCase() === name)?.kind);
}

/** 已安装记录 + 对应的注册表条目（用于判断可更新）+ 插件类型。 */
const installedRows = computed(() =>
  manager.installed.map((rec) => {
    const plugin = rec.slug ? manager.registry.find((p) => p.slug === rec.slug) : undefined;
    return { rec, plugin, kind: installedKind(rec, plugin) };
  }),
);

// ---- 「已安装」的搜索 + 快捷筛选（乐器 / 效果 / 模型）----
/** 可选的快捷筛选。 */
const INSTALLED_FILTERS = ["all", "instrument", "effect", "model"] as const;
type InstalledFilter = (typeof INSTALLED_FILTERS)[number];
const installedFilter = ref<InstalledFilter>("all");
const installedQuery = ref("");

function installedFilterLabel(k: InstalledFilter): string {
  if (k === "instrument") return i18n.t("manager.kindInstrument");
  if (k === "effect") return i18n.t("manager.kindEffect");
  if (k === "model") return i18n.t("manager.kindModel");
  return i18n.t("common.all");
}

/** 已就位的模型（按筛选与搜索过滤）。 */
const visibleInstalledModels = computed(() => {
  if (installedFilter.value === "instrument" || installedFilter.value === "effect") return [];
  const q = installedQuery.value.trim().toLowerCase();
  return models.catalog.filter((m) => m.installed && (!q || m.name.toLowerCase().includes(q)));
});

/** 已安装的插件（按筛选与搜索过滤）。
 *
 * 非注册表来源（本地导入）的条目没有 `kind`，因此只在「全部」里出现。 */
const visibleInstalledPlugins = computed(() => {
  if (installedFilter.value === "model") return [];
  const q = installedQuery.value.trim().toLowerCase();
  return installedRows.value.filter((row) => {
    if (installedFilter.value !== "all" && row.kind !== installedFilter.value) return false;
    if (q && !row.rec.name.toLowerCase().includes(q)) return false;
    return true;
  });
});

/** 「已安装」标签上的角标：插件 + 模型。 */
const installedTotal = computed(
  () => manager.installed.length + models.catalog.filter((m) => m.installed).length,
);

/** 兼容文件排在前面，安装器排后面。 */
function sortedFiles(p: ManagerPlugin): ManagerFile[] {
  return [...p.files].sort((a, b) => {
    if (a.compatible !== b.compatible) return a.compatible ? -1 : 1;
    return Number(isArchive(b)) - Number(isArchive(a));
  });
}

// ---- 动作 ----

async function doRefresh(force: boolean): Promise<void> {
  await manager.load(force);
  if (manager.error) layout.notify(i18n.t("manager.error", { error: manager.error }));
}

async function doInstall(p: ManagerPlugin, f: ManagerFile): Promise<void> {
  try {
    await manager.install(p, f);
    layout.notify(i18n.t("manager.installed", { name: p.name }));
  } catch (err) {
    layout.notify(i18n.t("manager.error", { error: String(err) }));
  }
}

async function doOpen(url: string): Promise<void> {
  try {
    await manager.open(url);
  } catch (err) {
    layout.notify(i18n.t("manager.error", { error: String(err) }));
  }
}

async function doImport(): Promise<void> {
  const path = await pickPluginToImport();
  if (!path) return;
  try {
    await manager.importFile(path);
    layout.notify(i18n.t("manager.imported"));
  } catch (err) {
    layout.notify(i18n.t("manager.error", { error: String(err) }));
  }
}

async function doRemove(key: string, name: string): Promise<void> {
  try {
    await manager.remove(key);
    confirming.value = null;
    layout.notify(i18n.t("manager.removed", { name }));
  } catch (err) {
    layout.notify(i18n.t("manager.error", { error: String(err) }));
  }
}

/** 删除一个已下载的模型（随包的模型不提供删除）。 */
async function doRemoveModel(id: string): Promise<void> {
  confirmingModel.value = null;
  await models.remove(id);
}
</script>

<template>
  <div class="pane">
    <!-- 标签 + 显眼的「导入插件」按钮 -->
    <div class="head">
      <div class="tabs" role="tablist">
        <button
          class="tab"
          :class="{ on: tab === 'installed' }"
          role="tab"
          :aria-selected="tab === 'installed'"
          @click="tab = 'installed'"
        >
          {{ i18n.t("manager.tabInstalled") }}
          <span v-if="installedTotal" class="tab-count">{{ installedTotal }}</span>
        </button>
        <button
          class="tab"
          :class="{ on: tab === 'foreign' }"
          role="tab"
          :aria-selected="tab === 'foreign'"
          @click="tab = 'foreign'"
        >
          {{ i18n.t("manager.tabForeign") }}
          <span v-if="manager.foreign.length" class="tab-count">{{ manager.foreign.length }}</span>
        </button>
        <button
          class="tab"
          :class="{ on: tab === 'available' }"
          role="tab"
          :aria-selected="tab === 'available'"
          @click="tab = 'available'"
        >
          {{ i18n.t("manager.tabAvailable") }}
        </button>
      </div>

      <button
        class="import-btn"
        :title="i18n.t('manager.importHint')"
        :disabled="manager.busy !== null"
        @click="doImport"
      >
        <ArrowUpTrayIcon class="w-[15px] h-[15px]" />
        {{ i18n.t("manager.import") }}
      </button>
    </div>

    <!-- 错误横幅 -->
    <div v-if="manager.error" class="banner">
      <ExclamationTriangleIcon class="banner-icon" />
      <span class="banner-text">{{ manager.error }}</span>
      <button class="banner-close" @click="manager.error = null">×</button>
    </div>

    <!-- ===== 已安装（插件 + 模型）===== -->
    <template v-if="tab === 'installed'">
      <!-- 搜索 + 快捷筛选 -->
      <div class="filters">
        <input
          v-model="installedQuery"
          class="search"
          type="search"
          :placeholder="i18n.t('manager.installedSearchPlaceholder')"
          :aria-label="i18n.t('manager.installedSearchPlaceholder')"
        />
        <div class="chips" role="group" :aria-label="i18n.t('manager.kindFilter')">
          <button
            v-for="k in INSTALLED_FILTERS"
            :key="k"
            class="chip"
            :class="{ on: installedFilter === k }"
            :aria-pressed="installedFilter === k"
            @click="installedFilter = k"
          >
            {{ installedFilterLabel(k) }}
          </button>
        </div>
      </div>

      <ul v-if="visibleInstalledModels.length + visibleInstalledPlugins.length > 0" class="list">
        <!-- 模型（随包或已下载）：与已安装插件同一套排版（信息在左、按钮在右） -->
        <li
          v-for="m in visibleInstalledModels"
          :key="'model-' + m.id"
          class="card"
          :class="{ busy: models.installing(m.id) }"
        >
          <div class="card-main">
            <div class="card-top">
              <span class="card-name">{{ m.name }}</span>
              <span class="badge">{{ i18n.t("manager.kindModel") }}</span>
              <span v-if="m.bundled" class="badge local">{{ i18n.t("manager.modelBundled") }}</span>
              <span v-else class="badge installed">{{ i18n.t("manager.modelInstalled") }}</span>
            </div>
            <p class="card-author">{{ humanSize(m.approx_bytes) }} · {{ m.license }}</p>
          </div>

          <!-- 在文件夹中显示 -->
          <button
            class="icon-btn"
            :title="i18n.t('manager.modelReveal')"
            :aria-label="i18n.t('manager.modelReveal')"
            :disabled="models.installing(m.id)"
            @click="models.reveal(m.id)"
          >
            <FolderOpenIcon class="w-[15px] h-[15px]" />
          </button>

          <!-- 删除（随包的不能删）：点一次进入确认 -->
          <template v-if="!m.bundled">
            <button
              v-if="confirmingModel !== m.id"
              class="icon-btn danger"
              :title="i18n.t('manager.modelRemove')"
              :aria-label="i18n.t('manager.modelRemove')"
              :disabled="models.busyId !== null || models.installing(m.id)"
              @click="confirmingModel = m.id"
            >
              <TrashIcon class="w-[15px] h-[15px]" />
            </button>
            <span v-else class="confirm">
              <button
                class="mini-btn danger"
                :disabled="models.busyId !== null"
                @click="doRemoveModel(m.id)"
              >
                {{ i18n.t("manager.confirmRemove") }}
              </button>
              <button class="mini-btn" @click="confirmingModel = null">
                {{ i18n.t("common.cancel") }}
              </button>
            </span>
          </template>

          <div v-if="models.installing(m.id)" class="progress">
            <div
              class="progress-fill"
              :class="{ indeterminate: models.progressPct === null }"
              :style="models.progressPct === null ? undefined : { width: `${models.progressPct}%` }"
            />
          </div>
        </li>

        <!-- 插件 -->
        <li
          v-for="row in visibleInstalledPlugins"
          :key="row.rec.key"
          class="card"
          :class="{ busy: installing(row.rec.key) }"
        >
          <div class="card-main">
            <div class="card-top">
              <span class="card-name">{{ row.rec.name }}</span>
              <span
                v-if="row.kind"
                class="badge"
                :class="{ local: row.kind === 'instrument' }"
              >
                {{ i18n.t(row.kind === "instrument" ? "manager.kindInstrument" : "manager.kindEffect") }}
              </span>
              <span v-if="row.rec.version" class="badge">v{{ row.rec.version }}</span>
              <span class="badge" :class="{ local: row.rec.source === 'local' }">
                {{ i18n.t(row.rec.source === "local" ? "manager.sourceLocal" : "manager.sourceRegistry") }}
              </span>
            </div>
            <p class="card-path" :title="row.rec.files.join('\n')">
              {{ row.rec.files.map((f) => f.split("/").pop()).join(", ") }}
            </p>
          </div>

          <!-- 有更新时给「更新到 x.y.z」 -->
          <button
            v-if="row.plugin && manager.updateAvailable(row.plugin)"
            class="mini-btn primary"
            :disabled="manager.busy !== null"
            @click="doInstall(row.plugin, sortedFiles(row.plugin).find((f) => f.compatible && isArchive(f)) ?? row.plugin.files[0])"
          >
            {{ i18n.t("manager.updateTo", { version: row.plugin.version }) }}
          </button>

          <!-- 删除：点一次进入确认 -->
          <button
            v-if="confirming !== row.rec.key"
            class="icon-btn danger"
            :title="i18n.t('manager.remove')"
            :aria-label="i18n.t('manager.remove')"
            :disabled="manager.busy !== null"
            @click="confirming = row.rec.key"
          >
            <TrashIcon class="w-[15px] h-[15px]" />
          </button>
          <span v-else class="confirm">
            <button
              class="mini-btn danger"
              :disabled="manager.busy !== null"
              @click="doRemove(row.rec.key, row.rec.name)"
            >
              {{ i18n.t("manager.confirmRemove") }}
            </button>
            <button class="mini-btn" @click="confirming = null">{{ i18n.t("common.cancel") }}</button>
          </span>

          <!-- 更新时的下载进度，同样贴在卡片下边缘 -->
          <div v-if="installing(row.rec.key)" class="progress">
            <div
              class="progress-fill"
              :class="{ indeterminate: manager.progressPct === null }"
              :style="manager.progressPct === null ? undefined : { width: `${manager.progressPct}%` }"
            />
          </div>
        </li>
      </ul>

      <div v-else class="empty">
        <ArrowDownTrayIcon class="empty-icon" />
        <p class="empty-text">
          {{ i18n.t(installedTotal > 0 ? "manager.emptyInstalledFiltered" : "manager.emptyInstalled") }}
        </p>
        <template v-if="installedTotal === 0">
          <p class="empty-hint">{{ i18n.t("manager.emptyInstalledHint") }}</p>
          <button class="import-btn" :disabled="manager.busy !== null" @click="doImport">
            <ArrowUpTrayIcon class="w-[15px] h-[15px]" />
            {{ i18n.t("manager.import") }}
          </button>
        </template>
      </div>
    </template>

    <!-- ===== 外部（你自己装的，本管理器不碰）===== -->
    <template v-else-if="tab === 'foreign'">
      <p class="hint">{{ i18n.t("manager.foreignHint") }}</p>

      <ul v-if="manager.foreign.length > 0" class="list">
        <li v-for="p in manager.foreign" :key="p.id" class="card">
          <div class="card-main">
            <div class="card-top">
              <span class="card-name">{{ p.name }}</span>
              <span class="badge" :class="{ local: p.kind === 'instrument' }">
                {{ i18n.t(p.kind === "instrument" ? "manager.kindInstrument" : "manager.kindEffect") }}
              </span>
              <span class="badge">{{ p.format.toUpperCase() }}</span>
            </div>
            <p class="card-author">{{ p.vendor }}</p>
            <p class="card-path" :title="p.path">{{ p.path }}</p>
          </div>
        </li>
      </ul>

      <div v-else class="empty">
        <ArrowDownTrayIcon class="empty-icon" />
        <p class="empty-text">{{ i18n.t("manager.emptyForeign") }}</p>
        <p class="empty-hint">{{ i18n.t("manager.emptyForeignHint") }}</p>
      </div>
    </template>

    <!-- ===== 可下载：模型 + 插件 ===== -->
    <template v-else>
      <div class="filters">
        <input
          v-model="query"
          class="search"
          type="search"
          :placeholder="i18n.t('manager.searchPlaceholder')"
          :aria-label="i18n.t('manager.searchPlaceholder')"
        />
        <div class="chips" role="group" :aria-label="i18n.t('manager.kindFilter')">
          <button
            v-for="k in (['all', 'instrument', 'effect', 'model'] as const)"
            :key="k"
            class="chip"
            :class="{ on: kindFilter === k }"
            :aria-pressed="kindFilter === k"
            @click="kindFilter = k"
          >
            {{
              i18n.t(
                k === "all"
                  ? "common.all"
                  : k === "instrument"
                    ? "manager.kindInstrument"
                    : k === "effect"
                      ? "manager.kindEffect"
                      : "manager.kindModel",
              )
            }}
          </button>
          <button
            class="chip"
            :class="{ on: compatibleOnly }"
            :aria-pressed="compatibleOnly"
            @click="compatibleOnly = !compatibleOnly"
          >
            {{ i18n.t("manager.compatibleOnly") }}
          </button>
        </div>
        <div class="meta">
          <span class="meta-text">
            {{ i18n.t("manager.count", { shown: availableShown, total: availableTotal }) }}
          </span>
          <button
            class="icon-btn"
            :title="i18n.t('manager.refresh')"
            :aria-label="i18n.t('manager.refresh')"
            :disabled="manager.loading || models.loading"
            @click="doRefresh(true); models.load()"
          >
            <ArrowPathIcon
              class="w-[15px] h-[15px]"
              :class="{ spinning: manager.loading || models.loading }"
            />
          </button>
        </div>
      </div>

      <ul v-if="availableShown > 0" class="list">
        <!-- 模型（当筛选允许时；与插件共用同一份搜索与筛选、同一套卡片排版） -->
        <li
          v-for="m in visibleModels"
          :key="m.id"
          class="card col"
          :class="{ busy: models.installing(m.id) }"
        >
          <div class="card-top">
            <span class="card-name">{{ m.name }}</span>
            <span class="badge">{{ i18n.t("manager.kindModel") }}</span>
            <span v-if="m.bundled" class="badge local">{{ i18n.t("manager.modelBundled") }}</span>
            <span v-else-if="m.installed" class="badge installed">
              {{ i18n.t("manager.modelInstalled") }}
            </span>
          </div>
          <p class="card-author">
            {{ humanSize(m.approx_bytes) }}<span v-if="m.license"> · {{ m.license }}</span>
          </p>

          <div class="files">
            <div class="file-row">
              <span class="file-summary" :title="m.filename">{{ m.filename }}</span>

              <!-- 有可下载的源才给「复制下载链接」 -->
              <button
                v-if="m.downloadable"
                class="icon-btn"
                :title="i18n.t('manager.copyLink')"
                :aria-label="i18n.t('manager.copyLink')"
                @click="doCopyModelLink(m.id)"
              >
                <LinkIcon class="w-[14px] h-[14px]" />
              </button>

              <!-- 未安装：下载 -->
              <button
                v-if="m.downloadable && !m.installed && !models.installing(m.id)"
                class="mini-btn primary"
                :disabled="models.busyId !== null"
                @click="models.download(m.id)"
              >
                {{ i18n.t("manager.modelDownload", { size: humanSize(m.approx_bytes) }) }}
              </button>
              <!-- 下载中：取消 -->
              <button
                v-else-if="models.installing(m.id)"
                class="mini-btn danger"
                @click="models.cancel()"
              >
                {{ i18n.t("manager.modelCancel") }}
              </button>
              <!-- 已安装且可下载：重新安装（随包的不用重装；删除请到「已安装」） -->
              <button
                v-else-if="m.installed && m.downloadable && !m.bundled"
                class="mini-btn"
                :disabled="models.busyId !== null"
                @click="models.download(m.id)"
              >
                {{ i18n.t("manager.reinstall") }}
              </button>
            </div>
          </div>

          <div v-if="models.installing(m.id)" class="progress">
            <div
              class="progress-fill"
              :class="{ indeterminate: models.progressPct === null }"
              :style="models.progressPct === null ? undefined : { width: `${models.progressPct}%` }"
            />
          </div>
        </li>

        <!-- 插件 -->
        <li
          v-for="p in available"
          :key="p.slug"
          class="card col"
          :class="{ busy: installing(p.slug) }"
        >
          <div class="card-top">
            <span class="card-name">{{ p.name }}</span>
            <span class="badge" :class="{ local: p.kind === 'instrument' }">
              {{ i18n.t(p.kind === "instrument" ? "manager.kindInstrument" : p.kind === "effect" ? "manager.kindEffect" : "manager.kindOther") }}
            </span>
            <span v-if="p.verified" class="badge verified">{{ i18n.t("manager.verified") }}</span>
            <span v-if="manager.installedFor(p.slug)" class="badge installed">
              {{ i18n.t("manager.installedBadge", { version: manager.installedFor(p.slug)?.version || "?" }) }}
            </span>
          </div>
          <p class="card-author">{{ p.author }}<span v-if="p.license"> · {{ p.license }}</span></p>
          <p class="card-desc">{{ p.description }}</p>

          <div class="files">
            <div v-for="f in sortedFiles(p)" :key="f.url" class="file-row" :class="{ unavailable: !f.compatible }">
              <span class="file-summary" :title="f.url">{{ fileSummary(f) }}</span>
              <button
                class="icon-btn"
                :title="i18n.t('manager.copyLink')"
                :aria-label="i18n.t('manager.copyLink')"
                @click="doCopyLink(f.url)"
              >
                <LinkIcon class="w-[14px] h-[14px]" />
              </button>
              <button
                v-if="!f.compatible"
                class="mini-btn"
                disabled
              >
                {{ i18n.t("manager.notForSystem") }}
              </button>
              <button
                v-else-if="isArchive(f)"
                class="mini-btn primary"
                :disabled="manager.busy !== null"
                @click="doInstall(p, f)"
              >
                {{
                  manager.updateAvailable(p)
                    ? i18n.t("manager.update")
                    : manager.installedFor(p.slug)
                      ? i18n.t("manager.reinstall")
                      : i18n.t("manager.install")
                }}
              </button>
              <button v-else class="mini-btn" :disabled="manager.busy !== null" @click="doOpen(f.url)">
                {{ i18n.t("manager.openDownload") }}
                <ArrowTopRightOnSquareIcon class="w-[13px] h-[13px]" />
              </button>
            </div>
          </div>

          <!-- 下载 / 解压 / 安装的进度：贴在卡片下边缘 -->
          <div v-if="installing(p.slug)" class="progress">
            <div
              class="progress-fill"
              :class="{ indeterminate: manager.progressPct === null }"
              :style="manager.progressPct === null ? undefined : { width: `${manager.progressPct}%` }"
            />
          </div>
        </li>
      </ul>

      <div v-else class="empty">
        <ArrowPathIcon class="empty-icon" :class="{ spinning: manager.loading }" />
        <p class="empty-text">
          {{ i18n.t(manager.loading ? "manager.loadingRegistry" : "manager.emptyAvailable") }}
        </p>
        <p class="empty-hint">
          {{ i18n.t(manager.loading ? "manager.loadingRegistryHint" : "manager.emptyAvailableHint") }}
        </p>
      </div>
    </template>
  </div>
</template>

<style scoped>
.pane {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding-bottom: 12px;
}
/* ---- 标签栏 + 导入按钮 ---- */
.head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 12px 12px 0;
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
/* 显眼的「导入插件」：图标 + 文字（窄面板里会换到下一行）。 */
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
  transition: background-color 0.14s ease;
}
.import-btn:hover:not(:disabled) {
  background-color: rgb(var(--md-primary) / 0.88);
}
.import-btn:disabled {
  opacity: 0.45;
  cursor: default;
}
.import-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 2px;
}
/* 说明条（外部插件那一栏用）。 */
.hint {
  margin: 10px 12px 0;
  font-size: 11px;
  line-height: 1.5;
  color: rgb(var(--md-on-surface-muted));
}
/* ---- 图标按钮 ---- */
.icon-btn {
  width: 28px;
  height: 28px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease;
}
.icon-btn:hover:not(:disabled) {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.icon-btn:disabled {
  opacity: 0.4;
  cursor: default;
}
.icon-btn.danger:hover:not(:disabled) {
  background-color: rgb(var(--md-error) / 0.14);
  color: rgb(var(--md-error));
}
.icon-btn:focus-visible,
.tab:focus-visible,
.chip:focus-visible,
.mini-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.spinning {
  animation: spin 0.9s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
/* ---- 错误横幅 ---- */
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
.banner-close {
  border: 0;
  background: transparent;
  color: inherit;
  font-size: 15px;
  cursor: pointer;
  line-height: 1;
}
/* ---- 筛选 ---- */
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
  transition: background-color 0.14s ease, color 0.14s ease, border-color 0.14s ease;
}
.chip:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.chip.on {
  border-color: transparent;
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.meta {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.meta-text {
  font-size: 11px;
  color: rgb(var(--md-on-surface-muted));
  font-variant-numeric: tabular-nums;
}
/* ---- 列表 ---- */
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
  overflow: hidden;
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
/* 安装 / 下载进度：贴在卡片**下边缘**（与 AI 面板同一套观感）。 */
.progress {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 0;
  height: 3px;
  background-color: rgb(var(--md-primary) / 0.15);
}
.progress-fill {
  height: 100%;
  width: 0;
  background-color: rgb(var(--md-primary));
  transition: width 120ms linear;
}
/* 总大小未知（或正在解压 / 安装）时长条来回跑，表示「还在动，但算不出百分比」。 */
.progress-fill.indeterminate {
  width: 35%;
  animation: manager-indeterminate 1.1s ease-in-out infinite;
}
@keyframes manager-indeterminate {
  0% {
    margin-left: -35%;
  }
  100% {
    margin-left: 100%;
  }
}
@media (prefers-reduced-motion: reduce) {
  .progress-fill.indeterminate {
    width: 100%;
    animation: none;
  }
}
.card-main {
  flex: 1 1 auto;
  min-width: 0;
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
.badge.local {
  background-color: rgb(var(--md-surface-container-highest));
}
.badge.verified {
  background-color: rgb(var(--md-primary) / 0.16);
  color: rgb(var(--md-primary));
}
.badge.installed {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.card-author {
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
.card-path {
  margin: 2px 0 0;
  font-size: 11px;
  color: rgb(var(--md-on-surface-muted));
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
/* ---- 文件行 ---- */
.files {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 6px;
}
.file-row {
  display: flex;
  align-items: center;
  gap: 8px;
}
.file-row.unavailable {
  opacity: 0.5;
}
.file-summary {
  flex: 1 1 auto;
  min-width: 0;
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
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
.mini-btn.primary {
  border-color: transparent;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}
.mini-btn.primary:hover:not(:disabled) {
  background-color: rgb(var(--md-primary) / 0.88);
}
.mini-btn.danger {
  border-color: transparent;
  background-color: rgb(var(--md-error));
  color: rgb(var(--md-on-error));
}
.mini-btn:disabled {
  opacity: 0.45;
  cursor: default;
}
.confirm {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex: 0 0 auto;
}
/* ---- 空态 ---- */
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
}
.empty-hint {
  margin: 0;
  font-size: 11.5px;
  text-align: center;
}
</style>
