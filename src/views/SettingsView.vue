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
import { useSettingsStore, MAX_DEFAULT_TEMPO, MIN_DEFAULT_TEMPO } from "@/stores/settings";
import Md3Slider from "@/components/common/Md3Slider.vue";
import NumberField from "@/components/common/NumberField.vue";
import ComboField from "@/components/common/ComboField.vue";
import { usePluginsStore } from "@/stores/plugins";
import { useUpdaterStore } from "@/stores/updater";
import { useI18n } from "@/i18n";
import { copyText } from "@/utils/clipboard";
import { lastWorkspace } from "@/router";
import {
  agentConfig,
  agentModels,
  agentSetConfig,
  agentTest,
  aiBackendInfo,
  aiSetBackend,
  cliStatus,
  installCli,
  managerNetwork,
  managerProbeProxy,
  managerProxyStatus,
  managerSetNetwork,
  mcpStatus,
  setMcpConfig,
  uninstallCli,
} from "@/api/tauri";
import type { BackendInfo, BackendPreference, CliStatus, McpStatus, ModelSourcePreference, ProxyMode } from "@/api/types";

type Section = "general" | "appearance" | "layout" | "plugins" | "network" | "ai" | "cli" | "about";

const SECTIONS: { id: Section; labelKey: string; captionKey: string }[] = [
  { id: "general", labelKey: "settings.section.general", captionKey: "settings.section.general.caption" },
  { id: "appearance", labelKey: "settings.section.appearance", captionKey: "settings.section.appearance.caption" },
  { id: "layout", labelKey: "settings.section.layout", captionKey: "settings.section.layout.caption" },
  { id: "plugins", labelKey: "settings.section.plugins", captionKey: "settings.section.plugins.caption" },
  { id: "network", labelKey: "settings.section.network", captionKey: "settings.section.network.caption" },
  { id: "ai", labelKey: "settings.section.ai", captionKey: "settings.section.ai.caption" },
  { id: "cli", labelKey: "settings.section.cli", captionKey: "settings.section.cli.caption" },
  { id: "about", labelKey: "settings.section.about", captionKey: "settings.section.about.caption" },
];

/** 代理三种取用方式（顺序即界面顺序）。 */
const PROXY_MODES: { id: ProxyMode; labelKey: string }[] = [
  { id: "direct", labelKey: "settings.proxy.modeDirect" },
  { id: "system", labelKey: "settings.proxy.modeSystem" },
  { id: "manual", labelKey: "settings.proxy.modeManual" },
];

/** 模型下载源的区域偏好（顺序即界面顺序）。 */
const SOURCE_PREFS: { id: ModelSourcePreference; labelKey: string }[] = [
  { id: "auto", labelKey: "settings.store.sourceAuto" },
  { id: "international", labelKey: "settings.store.sourceIntl" },
  { id: "domestic", labelKey: "settings.store.sourceCn" },
];

const APP_VERSION = "1.3.0";

const router = useRouter();
const active = ref<Section>("general");
const { theme, accent, setTheme, setAccent } = useTheme();
const layout = useLayoutStore();
const project = useProjectStore();
const settings = useSettingsStore();
const plugins = usePluginsStore();
const updater = useUpdaterStore();
const i18n = useI18n();

const activeMeta = computed(() => SECTIONS.find((s) => s.id === active.value) ?? SECTIONS[0]);

const projectName = computed(() => {
  const name = project.project?.name;
  return !name || name === "untitled" ? i18n.t("settings.untitled") : name;
});

function back(): void {
  void router.push(lastWorkspace.value);
}
/** 打开右侧栏的「插件管理器」面板并回到工作区（设置页里只是入口）。 */
function openManager(): void {
  layout.openTab("manager");
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
const mcpPort = ref("50562");
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

// ---- aidaw 命令行工具 ----
/** 安装状态；加载失败（预览 / 后端不可用）时保持 null。 */
const cli = ref<CliStatus | null>(null);
const cliBusy = ref(false);
/** 上一次操作的失败原因，供界面提示。 */
const cliError = ref("");

async function loadCli(): Promise<void> {
  try {
    cli.value = await cliStatus();
  } catch {
    cli.value = null;
  }
}

/** 挂载 / 卸载共用：忙标志 + 错误 + 成功 toast。 */
async function runCli(action: () => Promise<CliStatus>, toastKey: string): Promise<void> {
  cliBusy.value = true;
  cliError.value = "";
  try {
    cli.value = await action();
    layout.notify(i18n.t(toastKey));
  } catch (err) {
    cliError.value = String(err);
  } finally {
    cliBusy.value = false;
  }
}

/** 状态文案（`missing` → `settings.cli.state.missing`）。 */
const cliStateLabel = computed(() =>
  cli.value ? i18n.t(`settings.cli.state.${cli.value.state}`) : "—",
);

/** 状态说明：错误 > 环境原因 > 重名冲突 > 一般提示。 */
const cliMessage = computed(() => {
  if (cliError.value) return cliError.value;
  const status = cli.value;
  if (!status) return i18n.t("settings.cli.statusHint");
  if (status.hint) return i18n.t(`settings.cli.hint.${status.hint}`);
  if (status.state === "conflict") return i18n.t("settings.cli.hint.conflict");
  return i18n.t("settings.cli.statusHint");
});

/** 命令现在指向哪里（或包内位置）。 */
const cliLocation = computed(() => {
  const status = cli.value;
  if (!status) return "—";
  if (status.state === "conflict") return status.conflict ?? "—";
  return status.target ?? status.bundled ?? i18n.t("settings.cli.notBundled");
});

/** 开关状态：已挂载（含 Linux 系统包）为「开」。 */
const cliOn = computed(() => {
  const state = cli.value?.state;
  return state === "installed" || state === "system";
});

/** 能操作才给开关；「装不了」的两种状态是灰的。 */
const cliToggleable = computed(() => {
  const state = cli.value?.state;
  return state === "missing" || state === "conflict" || state === "installed";
});

/** 开关是**唯一**入口——不做任何自动挂载。 */
function onToggleCli(): void {
  const turningOn = !cliOn.value;
  void runCli(
    turningOn ? installCli : uninstallCli,
    turningOn ? "settings.cli.mountedToast" : "settings.cli.unmountedToast",
  );
}

/** 复制一段文本；两条路都不通（无权限 / 非安全上下文 / 没聚焦）时才提示手抄。 */
async function copy(text: string | null | undefined): Promise<void> {
  if (!text) return;
  if (await copyText(text)) layout.notify(i18n.t("settings.copy.ok"));
  else layout.notify(i18n.t("copy.failed"));
}

// ---- 网络：代理 / 镜像 / 地址重写 ----
/** 注册表镜像：多行文本，一行一个地址（按顺序在官方源之前尝试）。 */
const storeMirrors = ref("");
/** 下载地址重写前缀（只对 GitHub 地址生效）；可用 `{url}` 占位。 */
const storePrefix = ref("");
/** 模型权重镜像 / 前置代理：多行文本，一行一个前缀。 */
const storeModels = ref("");
/** 模型下载源的区域偏好：自动探测 / 优先国外 / 优先国内。 */
const modelSourcePref = ref<ModelSourcePreference>("auto");
/** 代理取用方式；默认跟随系统。 */
const proxyMode = ref<ProxyMode>("system");
/** 手动模式的代理地址。 */
const proxyUrl = ref("");
/** 当前解析出的代理来源（打开页面时由后端解析，**不联网**）。 */
const proxySource = ref("");
/** 「测试」按钮的结果（会真的发一次请求）。 */
const proxyProbe = ref("");
const proxyTesting = ref(false);
const storeSaving = ref(false);

async function refreshProxyStatus(): Promise<void> {
  try {
    const status = await managerProxyStatus();
    proxySource.value = status.source;
  } catch {
    proxySource.value = "";
  }
}

async function loadStoreNetwork(): Promise<void> {
  try {
    const net = await managerNetwork();
    storeMirrors.value = net.registry_mirrors.join("\n");
    storePrefix.value = net.download_prefix;
    storeModels.value = net.model_mirrors.join("\n");
    modelSourcePref.value = net.model_source_preference;
    proxyMode.value = net.proxy_mode;
    proxyUrl.value = net.proxy_url;
  } catch {
    // 预览 / 后端不可用：保持空白（等于直连）。
  }
  await refreshProxyStatus();
}

/** 执行后端（CPU / CoreML / DirectML）：只列出本机平台支持的那些。 */
const backendInfo = ref<BackendInfo | null>(null);
const backendSaving = ref(false);
const availableBackends = computed(() => backendInfo.value?.options.filter((o) => o.available) ?? []);

/**
 * 执行后端的文案。
 *
 * 后端目录（`local-ai::backend::catalog`）里的 `label` / `note` 只有中文；这里按 `kind` 取
 * i18n 文案，英文界面才不会是中文。词典里没有该 kind 时回退到后端下发的那份文案。
 */
function backendText(kind: string, field: "label" | "note", fallback: string): string {
  const key = field === "note" ? `backend.${kind}.note` : `backend.${kind}`;
  const text = i18n.t(key);
  return text === key ? fallback : text;
}

const backendNote = computed(() => {
  const current = backendInfo.value?.options.find((o) => o.kind === backendInfo.value?.preference);
  return current ? backendText(current.kind, "note", current.note) : "";
});

async function loadBackend(): Promise<void> {
  try {
    backendInfo.value = await aiBackendInfo();
  } catch {
    // 预览 / 后端不可用：保持空（界面只显示 CPU 之外的选项为空）。
  }
}

async function setBackend(preference: BackendPreference): Promise<void> {
  if (backendInfo.value?.preference === preference) return;
  backendSaving.value = true;
  try {
    backendInfo.value = await aiSetBackend(preference);
  } catch (err) {
    layout.notify(String(err));
  } finally {
    backendSaving.value = false;
  }
}

async function saveStoreNetwork(): Promise<void> {
  storeSaving.value = true;
  try {
    const mirrors = storeMirrors.value
      .split("\n")
      .map((s) => s.trim())
      .filter(Boolean);
    const modelMirrors = storeModels.value
      .split("\n")
      .map((s) => s.trim())
      .filter(Boolean);
    const net = await managerSetNetwork(
      mirrors,
      storePrefix.value.trim(),
      proxyMode.value,
      proxyUrl.value.trim(),
      modelMirrors,
      modelSourcePref.value,
    );
    storeMirrors.value = net.registry_mirrors.join("\n");
    storePrefix.value = net.download_prefix;
    storeModels.value = net.model_mirrors.join("\n");
    modelSourcePref.value = net.model_source_preference;
    proxyMode.value = net.proxy_mode;
    proxyUrl.value = net.proxy_url;
    proxyProbe.value = "";
    await refreshProxyStatus();
    layout.notify(i18n.t("settings.store.saved"));
  } catch (err) {
    layout.notify(i18n.t("settings.store.saveFailed", { error: String(err) }));
  } finally {
    storeSaving.value = false;
  }
}

/** 测的是**输入框里**的设置（非保存后的），且不会写盘。 */
async function testProxy(): Promise<void> {
  proxyTesting.value = true;
  proxyProbe.value = "";
  try {
    const probe = await managerProbeProxy(proxyMode.value, proxyUrl.value.trim());
    const where = probe.proxy ?? i18n.t("settings.proxy.direct");
    proxyProbe.value = probe.ok
      ? i18n.t("settings.proxy.probeOk", { proxy: where, detail: probe.detail })
      : i18n.t("settings.proxy.probeFail", { proxy: where, detail: probe.detail });
  } catch (err) {
    proxyProbe.value = String(err);
  } finally {
    proxyTesting.value = false;
  }
}

// ---- 写歌智能体（OpenAI 兼容端点）----

const agentBaseUrl = ref("");
const agentApiKey = ref("");
const agentModel = ref("");
const agentModelList = ref<string[]>([]);
const agentSaving = ref(false);
const agentFetchingModels = ref(false);
const agentTesting = ref(false);
const agentProbe = ref("");

async function loadAgent(): Promise<void> {
  try {
    const cfg = await agentConfig();
    agentBaseUrl.value = cfg.base_url;
    agentApiKey.value = cfg.api_key;
    agentModel.value = cfg.model;
  } catch {
    // 读取失败就保持空白，用户照常可以填。
  }
}

/** 用输入框里的值拉模型列表（不写盘）。 */
async function fetchAgentModels(): Promise<void> {
  agentFetchingModels.value = true;
  agentProbe.value = "";
  try {
    agentModelList.value = await agentModels(agentBaseUrl.value.trim(), agentApiKey.value.trim(), agentModel.value.trim());
    if (!agentModel.value.trim() && agentModelList.value.length) agentModel.value = agentModelList.value[0];
  } catch (err) {
    agentProbe.value = String(err);
  } finally {
    agentFetchingModels.value = false;
  }
}

/** 用输入框里的值测连通性（不写盘）。 */
async function testAgent(): Promise<void> {
  agentTesting.value = true;
  agentProbe.value = "";
  try {
    const probe = await agentTest(agentBaseUrl.value.trim(), agentApiKey.value.trim(), agentModel.value.trim());
    agentProbe.value = probe.ok ? `✅ ${probe.detail}（${probe.ms} ms）` : `❌ ${probe.detail}`;
  } catch (err) {
    agentProbe.value = String(err);
  } finally {
    agentTesting.value = false;
  }
}

async function saveAgent(): Promise<void> {
  agentSaving.value = true;
  try {
    await agentSetConfig(agentBaseUrl.value.trim(), agentApiKey.value.trim(), agentModel.value.trim());
    layout.notify(i18n.t("settings.ai.openai.saved"));
  } catch (err) {
    layout.notify(String(err));
  } finally {
    agentSaving.value = false;
  }
}

// ---- 自动更新 ----
/** 更新行标题：随阶段变化（检查中 / 发现新版本 / 下载中 / 就绪 / 失败）。 */
const updateTitle = computed(() => {
  switch (updater.phase) {
    case "checking":
      return i18n.t("settings.update.status.checking");
    case "available":
      return i18n.t("settings.update.status.available", { version: updater.info?.version ?? "" });
    case "downloading":
      return i18n.t("settings.update.status.downloading", { version: updater.info?.version ?? "" });
    case "ready":
      return i18n.t("settings.update.status.ready");
    case "error":
      return i18n.t("settings.update.status.error", { error: updater.error });
    default:
      return i18n.t("settings.update.title");
  }
});

/** 更新行说明：空闲给「已是最新」，下载中给百分比，就绪提示重启，其余给来源提示。 */
const updateMessage = computed(() => {
  if (updater.phase === "idle") return i18n.t("settings.update.status.idle");
  if (updater.phase === "ready") return i18n.t("settings.update.status.restartHint");
  if (updater.phase === "downloading") {
    return i18n.t("settings.update.downloading", { percent: Math.round(updater.progress * 100) });
  }
  return i18n.t("settings.update.hint");
});

onMounted(() => {
  void loadMcp();
  void loadCli();
  void loadStoreNetwork();
  void loadBackend();
  void loadAgent();
  void updater.loadVersion();
  // 深链：`/settings?section=ai` 直接落到 AI / MCP 子页（写歌智能体的「去配置」会这么跳）。
  const section = router.currentRoute.value.query.section;
  if (typeof section === "string" && SECTIONS.some((s) => s.id === section)) {
    active.value = section as Section;
  }
});
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
                <NumberField
                  v-model="settings.defaultTempo"
                  :min="MIN_DEFAULT_TEMPO"
                  :max="MAX_DEFAULT_TEMPO"
                  label="BPM"
                  input-width="48px"
                />
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

          <div class="group">
            <div class="group-title">{{ i18n.t("settings.plugins.managerGroup") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.plugins.manager") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.plugins.managerHint") }}</span>
                </div>
                <button class="scan-btn" @click="openManager">
                  {{ i18n.t("settings.plugins.managerOpen") }}
                </button>
              </div>
            </div>
          </div>

          <!-- 网络设置已独立成一节，这里给个指路牌（免得按老位置找不到）。 -->
          <div class="group">
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.plugins.networkMoved") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.plugins.networkMovedHint") }}</span>
                </div>
                <button class="scan-btn" @click="active = 'network'">
                  {{ i18n.t("settings.section.network") }}
                </button>
              </div>
            </div>
          </div>
        </template>

        <!-- 网络：代理 / 镜像 / 地址重写 -->
        <template v-else-if="active === 'network'">
          <!-- 代理：插件与模型下载都走它 -->
          <div class="group">
            <div class="group-title">{{ i18n.t("settings.proxy.group") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.proxy.mode") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.proxy.modeHint") }}</span>
                </div>
                <div class="m3-seg" role="group" :aria-label="i18n.t('settings.proxy.mode')">
                  <button
                    v-for="m in PROXY_MODES"
                    :key="m.id"
                    class="m3-seg-btn"
                    :class="{ on: proxyMode === m.id }"
                    :aria-pressed="proxyMode === m.id"
                    @click="proxyMode = m.id"
                  >
                    {{ i18n.t(m.labelKey) }}
                  </button>
                </div>
              </div>

              <div v-if="proxyMode === 'manual'" class="col-row">
                <span class="row-title">{{ i18n.t("settings.proxy.url") }}</span>
                <span class="row-sub">{{ i18n.t("settings.proxy.urlHint") }}</span>
                <input
                  v-model="proxyUrl"
                  class="net-input"
                  type="text"
                  spellcheck="false"
                  :placeholder="i18n.t('settings.proxy.urlPlaceholder')"
                />
              </div>

              <!-- 当前会走哪个代理：打开页面就显示（后端只解析、不联网） -->
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.proxy.current") }}</span>
                  <span class="row-sub">{{ proxySource || i18n.t("settings.proxy.currentUnknown") }}</span>
                </div>
                <button class="scan-btn" :disabled="proxyTesting || storeSaving" @click="testProxy">
                  {{ proxyTesting ? i18n.t("settings.proxy.testing") : i18n.t("settings.proxy.test") }}
                </button>
              </div>

              <div v-if="proxyProbe" class="col-row">
                <span class="row-sub">{{ proxyProbe }}</span>
              </div>
            </div>
          </div>

          <!-- 镜像 / 地址重写：解决国内拉不动注册表与插件二进制的问题。 -->
          <div class="group">
            <div class="group-title">{{ i18n.t("settings.store.group") }}</div>
            <div class="card">
              <div class="col-row">
                <span class="row-title">{{ i18n.t("settings.store.mirrors") }}</span>
                <span class="row-sub">{{ i18n.t("settings.store.mirrorsHint") }}</span>
                <textarea
                  v-model="storeMirrors"
                  class="net-input"
                  rows="2"
                  spellcheck="false"
                  :placeholder="i18n.t('settings.store.mirrorsPlaceholder')"
                />
              </div>
              <div class="col-row">
                <span class="row-title">{{ i18n.t("settings.store.prefix") }}</span>
                <span class="row-sub">{{ i18n.t("settings.store.prefixHint") }}</span>
                <input
                  v-model="storePrefix"
                  class="net-input"
                  type="text"
                  spellcheck="false"
                  :placeholder="i18n.t('settings.store.prefixPlaceholder')"
                />
              </div>
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.store.source") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.store.sourceHint") }}</span>
                </div>
                <div class="m3-seg" role="group" :aria-label="i18n.t('settings.store.source')">
                  <button
                    v-for="p in SOURCE_PREFS"
                    :key="p.id"
                    class="m3-seg-btn"
                    :class="{ on: modelSourcePref === p.id }"
                    :aria-pressed="modelSourcePref === p.id"
                    @click="modelSourcePref = p.id"
                  >
                    {{ i18n.t(p.labelKey) }}
                  </button>
                </div>
              </div>
              <div class="col-row">
                <span class="row-title">{{ i18n.t("settings.store.models") }}</span>
                <span class="row-sub">{{ i18n.t("settings.store.modelsHint") }}</span>
                <textarea
                  v-model="storeModels"
                  class="net-input"
                  rows="2"
                  spellcheck="false"
                  :placeholder="i18n.t('settings.store.modelsPlaceholder')"
                />
              </div>
              <div class="row">
                <div class="row-text">
                  <span class="row-sub">{{ i18n.t("settings.store.saveHint") }}</span>
                </div>
                <button class="scan-btn" :disabled="storeSaving" @click="saveStoreNetwork">
                  {{ storeSaving ? i18n.t("settings.store.saving") : i18n.t("settings.store.save") }}
                </button>
              </div>
            </div>
          </div>
        </template>

        <!-- AI / MCP：OpenAI 兼容端点（写歌智能体）+ 本地模型执行后端 + MCP 代理接入 -->
        <template v-else-if="active === 'ai'">
          <!-- OpenAI 兼容端点：右侧「写歌智能体」面板直连它 -->
          <div class="group">
            <div class="group-title">{{ i18n.t("settings.ai.openai.title") }}</div>
            <div class="card">
              <div class="col-row">
                <span class="row-title">{{ i18n.t("settings.ai.openai.baseUrl") }}</span>
                <span class="row-sub">{{ i18n.t("settings.ai.openai.baseUrlHint") }}</span>
                <input
                  v-model="agentBaseUrl"
                  class="net-input"
                  type="text"
                  spellcheck="false"
                  placeholder="https://api.openai.com/v1"
                />
              </div>
              <div class="col-row">
                <span class="row-title">{{ i18n.t("settings.ai.openai.apiKey") }}</span>
                <span class="row-sub">{{ i18n.t("settings.ai.openai.apiKeyHint") }}</span>
                <input
                  v-model="agentApiKey"
                  class="net-input"
                  type="password"
                  spellcheck="false"
                  placeholder="sk-…"
                />
              </div>
              <div class="col-row">
                <span class="row-title">{{ i18n.t("settings.ai.openai.model") }}</span>
                <span class="row-sub">{{ i18n.t("settings.ai.openai.modelHint") }}</span>
                <ComboField
                  :model-value="agentModel"
                  :options="agentModelList"
                  :label="i18n.t('settings.ai.openai.model')"
                  :placeholder="i18n.t('settings.ai.openai.modelPlaceholder')"
                  :empty="i18n.t('settings.ai.openai.modelEmpty')"
                  @update:model-value="agentModel = $event"
                />
              </div>
              <div class="row">
                <div class="row-text">
                  <span class="row-sub">{{ agentProbe || i18n.t("settings.ai.openai.probeHint") }}</span>
                </div>
                <div class="row-actions">
                  <button class="scan-btn" :disabled="agentFetchingModels" @click="fetchAgentModels">
                    {{ i18n.t("settings.ai.openai.fetchModels") }}
                  </button>
                  <button class="scan-btn" :disabled="agentTesting" @click="testAgent">
                    {{ i18n.t("settings.ai.openai.test") }}
                  </button>
                  <button class="scan-btn" :disabled="agentSaving" @click="saveAgent">
                    {{ i18n.t("settings.ai.openai.save") }}
                  </button>
                </div>
              </div>
            </div>
          </div>

          <div class="group">
            <div class="group-title">{{ i18n.t("settings.ai.title") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.ai.backend") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.ai.backendHint") }}</span>
                </div>
                <div class="m3-seg" role="group" :aria-label="i18n.t('settings.ai.backend')">
                  <button
                    v-for="o in availableBackends"
                    :key="o.kind"
                    class="m3-seg-btn"
                    :class="{ on: backendInfo?.preference === o.kind }"
                    :aria-pressed="backendInfo?.preference === o.kind"
                    :disabled="backendSaving"
                    @click="setBackend(o.kind)"
                  >
                    {{ backendText(o.kind, "label", o.label) }}
                  </button>
                </div>
              </div>
              <div class="col-row">
                <span class="row-sub" v-if="backendNote">{{ backendNote }}</span>
                <span class="row-sub">
                  {{ i18n.t("settings.ai.platform", { platform: backendInfo?.platform ?? "…" }) }}
                </span>
              </div>
            </div>
          </div>
        <!-- MCP：并入本节（服务器 + 接入） -->
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
                <button class="scan-btn" :disabled="!mcp?.url" @click="copy(mcp?.url)">
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

        <!-- 命令行工具（aidaw） -->
        <template v-else-if="active === 'cli'">
          <div class="group">
            <div class="group-title">{{ i18n.t("settings.cli.group") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.cli.mount") }}</span>
                  <span class="row-sub">{{ cliMessage }}</span>
                </div>
                <button
                  class="m3-switch"
                  :class="{ on: cliOn }"
                  role="switch"
                  :aria-checked="cliOn"
                  :aria-label="i18n.t('settings.cli.mount')"
                  :disabled="cliBusy || !cliToggleable"
                  @click="onToggleCli"
                />
              </div>
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.cli.status") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.cli.mountHint") }}</span>
                </div>
                <span class="row-value">{{ cliBusy ? i18n.t("settings.cli.busy") : cliStateLabel }}</span>
              </div>
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.cli.command") }}</span>
                  <span class="row-sub">{{ cliLocation }}</span>
                </div>
              </div>
              <div v-if="cli?.full_path && !cliOn" class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.cli.fullPath") }}</span>
                  <span class="row-sub">{{ cli.full_path }}</span>
                </div>
                <button class="scan-btn" @click="copy(cli?.full_path)">
                  {{ i18n.t("settings.cli.copy") }}
                </button>
              </div>
              <div v-if="cli?.manual_command" class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.cli.manual") }}</span>
                  <span class="row-sub">{{ cli.manual_command }}</span>
                </div>
                <button class="scan-btn" @click="copy(cli?.manual_command)">
                  {{ i18n.t("settings.cli.copy") }}
                </button>
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
                <span class="row-value">v{{ updater.version || APP_VERSION }}</span>
              </div>
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.about.engine") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.about.engineHint") }}</span>
                </div>
                <span class="row-value">ai-daw-engine</span>
              </div>
            </div>
          </div>

          <div class="group">
            <div class="group-title">{{ i18n.t("settings.update.group") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ updateTitle }}</span>
                  <span class="row-sub">{{ updateMessage }}</span>
                </div>
                <div class="row-actions">
                  <button
                    v-if="updater.phase === 'ready'"
                    class="scan-btn"
                    @click="updater.restart()"
                  >
                    {{ i18n.t("settings.update.restart") }}
                  </button>
                  <button
                    v-else-if="updater.phase === 'available'"
                    class="scan-btn"
                    @click="updater.install()"
                  >
                    {{ i18n.t("settings.update.download") }}
                  </button>
                  <button
                    v-else
                    class="scan-btn"
                    :disabled="updater.phase === 'checking' || updater.phase === 'downloading'"
                    @click="updater.check()"
                  >
                    {{ i18n.t("settings.update.check") }}
                  </button>
                </div>
              </div>
              <div v-if="updater.phase === 'downloading'" class="col-row">
                <div
                  class="update-progress"
                  role="progressbar"
                  :aria-valuenow="Math.round(updater.progress * 100)"
                  aria-valuemin="0"
                  aria-valuemax="100"
                >
                  <div
                    class="update-progress-fill"
                    :style="{ width: `${Math.round(updater.progress * 100)}%` }"
                  />
                </div>
              </div>
              <div v-if="updater.info?.notes" class="col-row">
                <span class="row-title">{{ i18n.t("settings.update.notes") }}</span>
                <p class="row-sub update-notes">{{ updater.info.notes }}</p>
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
/* 竖向排布的设置行（网络输入那种：标题在上、控件占满一整行）。 */
.col-row {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 12px 0;
}
.col-row + .col-row,
.col-row + .row {
  border-top: 1px solid rgb(var(--md-outline-variant) / 0.3);
}
.net-input {
  width: 100%;
  box-sizing: border-box;
  padding: 8px 10px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 10px;
  background-color: var(--app-card);
  color: rgb(var(--md-on-surface));
  font-family: inherit;
  font-size: 12px;
  line-height: 1.5;
  resize: vertical;
}
.net-input:focus {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 0;
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

/* ---- 写歌智能体（OpenAI 兼容端点） ---- */
.row-actions {
  display: flex;
  gap: 6px;
  flex-wrap: wrap;
  justify-content: flex-end;
}

/* ---- 自动更新 ---- */
.update-progress {
  height: 4px;
  border-radius: 999px;
  background-color: rgb(var(--md-outline-variant) / 0.4);
  overflow: hidden;
}
.update-progress-fill {
  height: 100%;
  background-color: rgb(var(--md-primary));
  transition: width 0.15s linear;
}
.update-notes {
  margin: 0;
  max-height: 160px;
  overflow-y: auto;
  white-space: pre-wrap;
}
</style>
