<script setup lang="ts">
/**
 * 设置页：自带一条侧栏（进入设置时应用侧栏隐藏，避免出现两条侧栏）。
 * 内容为居中列 + 分组卡片；全部控件自绘，不使用原生表单控件。
 */
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { ArrowLeftIcon } from "@heroicons/vue/24/outline";
import {
  ACCENTS,
  THEMES,
  useTheme,
  type AccentColor,
  type Theme,
} from "@/composables/useTheme";
import { useLayoutStore } from "@/stores/layout";
import { useProjectStore } from "@/stores/project";
import Md3Slider from "@/components/common/Md3Slider.vue";
import { usePluginsStore } from "@/stores/plugins";
import { useI18n } from "@/i18n";
import { lastWorkspace } from "@/router";
import { mcpStatus, setMcpConfig } from "@/api/tauri";
import type { McpStatus } from "@/api/types";

type Section = "general" | "appearance" | "layout" | "plugins" | "mcp" | "about";

const SECTIONS: { id: Section; labelKey: string; captionKey: string }[] = [
  { id: "general", labelKey: "settings.section.general", captionKey: "settings.section.general.caption" },
  { id: "appearance", labelKey: "settings.section.appearance", captionKey: "settings.section.appearance.caption" },
  { id: "layout", labelKey: "settings.section.layout", captionKey: "settings.section.layout.caption" },
  { id: "plugins", labelKey: "settings.section.plugins", captionKey: "settings.section.plugins.caption" },
  { id: "mcp", labelKey: "settings.section.mcp", captionKey: "settings.section.mcp.caption" },
  { id: "about", labelKey: "settings.section.about", captionKey: "settings.section.about.caption" },
];

const APP_VERSION = "0.1.0";

const router = useRouter();
const active = ref<Section>("general");
const { theme, accent, setTheme, setAccent } = useTheme();
const layout = useLayoutStore();
const project = useProjectStore();
const plugins = usePluginsStore();
const i18n = useI18n();

const activeMeta = computed(() => SECTIONS.find((s) => s.id === active.value) ?? SECTIONS[0]);

const projectName = computed(() => {
  const name = project.project?.name;
  return !name || name === "untitled" ? i18n.t("settings.untitled") : name;
});

function back(): void {
  void router.push(lastWorkspace.value);
}
function onTheme(t: Theme): void {
  setTheme(t);
}
function onAccent(a: AccentColor): void {
  setAccent(a);
}

// ---- MCP 服务器 ----
/** 配置与运行状态；加载失败（预览 / 后端不可用）时保持 null。 */
const mcp = ref<McpStatus | null>(null);
/** 端口输入框的草稿值（数字输入在编辑过程中不应该被后端回写打断）。 */
const mcpPort = ref("0");
const mcpSaving = ref(false);
/** 上一次应用的失败原因，供界面提示。 */
const mcpError = ref("");

async function loadMcp(): Promise<void> {
  try {
    mcp.value = await mcpStatus();
    mcpPort.value = String(mcp.value.port);
  } catch {
    mcp.value = null;
  }
}

async function applyMcp(enabled: boolean, port: number): Promise<void> {
  mcpSaving.value = true;
  mcpError.value = "";
  try {
    mcp.value = await setMcpConfig(enabled, port);
    mcpPort.value = String(mcp.value.port);
  } catch (err) {
    mcpError.value = String(err);
  } finally {
    mcpSaving.value = false;
  }
}

function toggleMcp(): void {
  void applyMcp(!(mcp.value?.enabled ?? false), clampPort(mcpPort.value));
}

function onMcpPort(e: Event): void {
  const raw = (e.target as HTMLInputElement).value;
  const port = clampPort(raw);
  mcpPort.value = String(port);
  void applyMcp(mcp.value?.enabled ?? false, port);
}

/** 端口夹紧到 0..=65535（0 = 自动）。 */
function clampPort(raw: string): number {
  const n = Number.parseInt(raw, 10);
  if (!Number.isFinite(n)) return 0;
  return Math.min(Math.max(n, 0), 65535);
}

async function copyEndpoint(): Promise<void> {
  const url = mcp.value?.url;
  if (!url) return;
  try {
    await navigator.clipboard.writeText(url);
  } catch {
    // 剪贴板不可用（无权限 / 非安全上下文）时静默：地址就在旁边，用户可以手抄。
  }
}

onMounted(loadMcp);
</script>

<template>
  <div class="settings">
    <!-- 设置自带的侧栏 -->
    <aside class="settings-nav">
      <button class="back" @click="back">
        <ArrowLeftIcon class="back-icon" />
        <span class="back-label">{{ i18n.t("settings.title") }}</span>
      </button>

      <nav class="nav-list" :aria-label="i18n.t('settings.navAria')">
        <button
          v-for="s in SECTIONS"
          :key="s.id"
          class="nav-item"
          :class="{ on: active === s.id }"
          :aria-current="active === s.id ? 'page' : undefined"
          @click="active = s.id"
        >
          <span class="nav-ind" />
          <span class="nav-text">
            <span class="nav-label">{{ i18n.t(s.labelKey) }}</span>
            <span class="nav-caption">{{ i18n.t(s.captionKey) }}</span>
          </span>
        </button>
      </nav>
    </aside>

    <!-- 内容：居中列 -->
    <section class="settings-body">
      <div class="settings-content">
        <h1 class="page-title">{{ i18n.t(activeMeta.labelKey) }}</h1>
        <p class="page-sub">{{ i18n.t(activeMeta.captionKey) }}</p>

        <!-- 常规 -->
        <template v-if="active === 'general'">
          <div class="group">
            <div class="group-title">{{ i18n.t("settings.project") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.project.current") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.project.currentHint") }}</span>
                </div>
                <span class="row-value">{{ projectName }}</span>
              </div>
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.project.tempo") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.project.tempoHint") }}</span>
                </div>
                <span class="row-value">120</span>
              </div>
            </div>
          </div>

          <div class="group">
            <div class="group-title">{{ i18n.t("settings.language") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.language.label") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.language.hint") }}</span>
                </div>
                <div class="m3-seg" role="group" :aria-label="i18n.t('settings.language.label')">
                  <button
                    v-for="l in i18n.locales"
                    :key="l.id"
                    class="m3-seg-btn"
                    :class="{ on: i18n.locale.value === l.id }"
                    :aria-pressed="i18n.locale.value === l.id"
                    @click="i18n.setLocale(l.id)"
                  >
                    {{ l.label }}
                  </button>
                </div>
              </div>
            </div>
          </div>
        </template>

        <!-- 外观 -->
        <template v-else-if="active === 'appearance'">
          <div class="group">
            <div class="group-title">{{ i18n.t("settings.theme") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.theme.label") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.theme.hint") }}</span>
                </div>
                <div class="m3-seg" role="group" :aria-label="i18n.t('settings.theme.label')">
                  <button
                    v-for="t in THEMES"
                    :key="t.id"
                    class="m3-seg-btn"
                    :class="{ on: theme === t.id }"
                    :aria-pressed="theme === t.id"
                    @click="onTheme(t.id)"
                  >
                    {{ i18n.t(t.labelKey) }}
                  </button>
                </div>
              </div>
            </div>
          </div>

          <div class="group">
            <div class="group-title">{{ i18n.t("settings.accent") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.accent.label") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.accent.hint") }}</span>
                </div>
                <div class="swatches" role="group" :aria-label="i18n.t('settings.accent.label')">
                  <button
                    v-for="a in ACCENTS"
                    :key="a.id"
                    class="swatch"
                    :class="{ on: accent === a.id }"
                    :style="{ backgroundColor: a.swatch }"
                    :aria-label="i18n.t(a.labelKey)"
                    :aria-pressed="accent === a.id"
                    @click="onAccent(a.id)"
                  />
                </div>
              </div>
            </div>
          </div>
        </template>

        <!-- 布局 -->
        <template v-else-if="active === 'layout'">
          <div class="group">
            <div class="group-title">{{ i18n.t("settings.panels") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.panels.tracks") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.panels.tracksHint") }}</span>
                </div>
                <button
                  class="m3-switch"
                  :class="{ on: layout.showTrackHeaders }"
                  role="switch"
                  :aria-checked="layout.showTrackHeaders"
                  :aria-label="i18n.t('settings.panels.tracks')"
                  @click="layout.showTrackHeaders = !layout.showTrackHeaders"
                />
              </div>
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.panels.inspector") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.panels.inspectorHint") }}</span>
                </div>
                <button
                  class="m3-switch"
                  :class="{ on: layout.showInspector }"
                  role="switch"
                  :aria-checked="layout.showInspector"
                  :aria-label="i18n.t('settings.panels.inspector')"
                  @click="layout.showInspector = !layout.showInspector"
                />
              </div>
            </div>
          </div>

          <div class="group">
            <div class="group-title">{{ i18n.t("settings.width") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.width.tracks") }}</span>
                </div>
                <Md3Slider
                  v-model="layout.trackWidth"
                  class="row-slider"
                  :min="160"
                  :max="360"
                  :step="4"
                  :label="i18n.t('settings.width.tracks')"
                />
              </div>
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.width.inspector") }}</span>
                  <span class="row-sub">{{ Math.round(layout.inspectorWidth) }} px</span>
                </div>
                <Md3Slider
                  v-model="layout.inspectorWidth"
                  class="row-slider"
                  :min="200"
                  :max="440"
                  :step="4"
                  :label="i18n.t('settings.width.inspector')"
                />
              </div>
            </div>
          </div>
          <div class="group">
            <div class="group-title">{{ i18n.t("settings.dev") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.dev.log") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.dev.logHint") }}</span>
                </div>
                <button
                  class="m3-switch"
                  :class="{ on: layout.devDrawer }"
                  role="switch"
                  :aria-checked="layout.devDrawer"
                  :aria-label="i18n.t('settings.dev.log')"
                  @click="layout.toggleDevDrawer()"
                />
              </div>
            </div>
          </div>
        </template>

        <!-- 插件管理 -->
        <template v-else-if="active === 'plugins'">
          <div class="group">
            <div class="group-title">{{ i18n.t("settings.plugins.installed") }}</div>
            <div class="card">
              <div v-for="p in plugins.all" :key="p.id" class="row">
                <div class="row-text">
                  <span class="row-title">{{ p.name }}</span>
                  <span class="row-sub">{{ p.note }}</span>
                </div>
                <span class="row-value">{{ p.builtin ? p.category : i18n.t("settings.plugins.externalTag") }}</span>
              </div>
            </div>
          </div>

          <div class="group">
            <div class="group-title">{{ i18n.t("settings.plugins.locations") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.plugins.builtin") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.plugins.builtinHint") }}</span>
                </div>
                <span class="row-value">{{ plugins.all.filter((p) => p.builtin).length }}</span>
              </div>
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.plugins.external") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.plugins.externalHint") }}</span>
                </div>
                <span class="row-value">{{ plugins.scanned.length }}</span>
              </div>
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.plugins.status") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.plugins.statusHint") }}</span>
                </div>
                <button class="scan-btn" :disabled="plugins.loading" @click="plugins.rescan()">
                  {{ plugins.loading ? i18n.t("settings.plugins.scanning") : i18n.t("settings.plugins.rescan") }}
                </button>
              </div>
            </div>
          </div>
        </template>

        <!-- AI / MCP -->
        <template v-else-if="active === 'mcp'">
          <div class="group">
            <div class="group-title">{{ i18n.t("settings.mcp.server") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.mcp.enable") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.mcp.enableHint") }}</span>
                </div>
                <button
                  class="m3-switch"
                  :class="{ on: mcp?.enabled ?? false }"
                  role="switch"
                  :aria-checked="mcp?.enabled ?? false"
                  :aria-label="i18n.t('settings.mcp.enable')"
                  :disabled="mcpSaving"
                  @click="toggleMcp"
                />
              </div>
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.mcp.port") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.mcp.portHint") }}</span>
                </div>
                <input
                  class="port-input md3-field"
                  type="number"
                  min="0"
                  max="65535"
                  inputmode="numeric"
                  :value="mcpPort"
                  :disabled="mcpSaving || !mcp?.enabled"
                  :aria-label="i18n.t('settings.mcp.port')"
                  @change="onMcpPort"
                />
              </div>
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.mcp.status") }}</span>
                  <span class="row-sub">{{ mcpError || mcp?.url || i18n.t("settings.mcp.notListening") }}</span>
                </div>
                <span class="row-value">
                  {{ mcp?.running ? i18n.t("settings.mcp.running") : i18n.t("settings.mcp.stopped") }}
                </span>
              </div>
            </div>
          </div>

          <div class="group">
            <div class="group-title">{{ i18n.t("settings.mcp.usage") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.mcp.endpoint") }}</span>
                  <span class="row-sub">{{ mcp?.url ?? i18n.t("settings.mcp.notListening") }}</span>
                </div>
                <button class="scan-btn" :disabled="!mcp?.url" @click="copyEndpoint">
                  {{ i18n.t("settings.mcp.copy") }}
                </button>
              </div>
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.mcp.security") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.mcp.securityHint") }}</span>
                </div>
              </div>
            </div>
          </div>
        </template>

        <!-- 关于 -->
        <template v-else>
          <div class="group">
            <div class="group-title">{{ i18n.t("settings.about.app") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.about.version") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.about.versionHint") }}</span>
                </div>
                <span class="row-value">v{{ APP_VERSION }}</span>
              </div>
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.about.engine") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.about.engineHint") }}</span>
                </div>
                <span class="row-value">daw-party-engine</span>
              </div>
            </div>
          </div>

          <div class="group">
            <div class="group-title">{{ i18n.t("settings.about.license") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.about.licenseLabel") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.about.licenseHint") }}</span>
                </div>
                <span class="row-value">Apache-2.0</span>
              </div>
            </div>
          </div>
        </template>
      </div>
    </section>
  </div>
</template>

<style scoped>
.settings {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  background-color: transparent;
}

/* ---- 设置侧栏 ---- */
.settings-nav {
  width: 220px;
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 10px 14px;
  border-right: 1px solid rgb(var(--md-outline-variant) / 0.3);
  background-color: transparent;
  overflow-y: auto;
}
.back {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 40px;
  padding: 0 8px;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
  transition: background-color var(--md-ease-emphasized), color var(--md-ease-emphasized);
}
.back:hover {
  background-color: rgb(var(--md-on-surface-variant) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.back:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.back-icon {
  width: 18px;
  height: 18px;
  flex: 0 0 auto;
}
.nav-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin-top: 4px;
}
.nav-item {
  position: relative;
  display: flex;
  align-items: center;
  min-height: 52px;
  padding: 6px 14px;
  border: 0;
  border-radius: 14px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  text-align: left;
  cursor: pointer;
  transition: background-color var(--md-ease-emphasized), color var(--md-ease-emphasized);
}
.nav-item:hover {
  background-color: rgb(var(--md-on-surface-variant) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.nav-item:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.nav-item.on {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.nav-ind {
  position: absolute;
  left: 4px;
  top: 50%;
  width: 3px;
  height: 0;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  transform: translateY(-50%);
  opacity: 0;
  transition: height var(--md-ease-emphasized), opacity var(--md-ease-emphasized);
}
.nav-item.on .nav-ind {
  height: 20px;
  opacity: 1;
}
.nav-text {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}
.nav-label {
  font-size: 14px;
  font-weight: 500;
}
.nav-caption {
  font-size: 11px;
  opacity: 0.7;
}

/* ---- 内容 ---- */
.settings-body {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  display: flex;
  justify-content: center;
}
.settings-content {
  width: 100%;
  max-width: 720px;
  padding: 32px 32px 56px;
}
.page-title {
  margin: 0;
  font-size: 26px;
  line-height: 1.25;
  font-weight: 400;
  letter-spacing: 0;
  color: rgb(var(--md-on-surface));
}
.page-sub {
  margin: 4px 0 24px;
  font-size: 13px;
  color: rgb(var(--md-on-surface-variant));
}
.group + .group {
  margin-top: 22px;
}
.group-title {
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.4px;
  color: rgb(var(--md-primary));
  padding: 0 4px 8px;
}
.card {
  background-color: var(--app-inset);
  border-radius: 20px;
  padding: 4px 16px;
}
.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  min-height: 62px;
  padding: 10px 0;
}
.row + .row {
  border-top: 1px solid rgb(var(--md-outline-variant) / 0.3);
}
.row-text {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}
.row-title {
  font-size: 14px;
  font-weight: 500;
  color: rgb(var(--md-on-surface));
}
.row-sub {
  font-size: 12px;
  color: rgb(var(--md-on-surface-variant));
}
.row-value {
  font-size: 13px;
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
  flex: 0 0 auto;
}
.row-slider {
  width: 200px;
  flex: 0 0 auto;
}
.swatches {
  display: flex;
  align-items: center;
  gap: 10px;
  flex: 0 0 auto;
}
/* 色板：自绘圆钮 */
.swatch {
  position: relative;
  width: 36px;
  height: 36px;
  border: 0;
  border-radius: 999px;
  padding: 0;
  cursor: pointer;
  box-shadow: var(--md-elevation-1);
  transition: transform var(--md-ease-emphasized);
}
.swatch:hover {
  transform: scale(1.08);
}
.swatch:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 2px;
}
.swatch.on::after {
  content: "";
  position: absolute;
  inset: -4px;
  border-radius: 999px;
  border: 2px solid rgb(var(--md-primary));
}

@media (max-width: 820px) {
  .settings-nav {
    width: 76px;
    padding: 10px 8px;
  }
  .back-label,
  .nav-text {
    display: none;
  }
  .back,
  .nav-item {
    justify-content: center;
    padding: 0;
  }
  .settings-content {
    padding: 24px 18px 44px;
  }
  .row {
    flex-direction: column;
    align-items: flex-start;
    gap: 10px;
  }
  .row-slider {
    width: 100%;
  }
}
@media (prefers-reduced-motion: reduce) {
  .back,
  .nav-item,
  .nav-ind,
  .swatch {
    transition: none;
  }
}
/* 重新扫描按钮 */
.scan-btn {
  flex: 0 0 auto;
  height: 26px;
  padding: 0 12px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-primary));
  font-size: 11.5px;
  cursor: pointer;
  transition: background-color 0.14s ease;
}
.scan-btn:hover:not(:disabled) {
  background-color: rgb(var(--md-primary) / 0.08);
}
.scan-btn:disabled {
  color: rgb(var(--md-on-surface-muted));
  cursor: default;
}
/* MCP 端口输入：与其它 md3-field 同形状，只是更窄并右对齐。 */
.port-input {
  width: 104px;
  flex: 0 0 auto;
  text-align: right;
}
.port-input:disabled {
  opacity: 0.5;
}
</style>
