<script setup lang="ts">
/**
 * 设置页：自带一条侧栏（进入设置时应用侧栏隐藏，避免出现两条侧栏）。
 * 内容为居中列 + 分组卡片；全部控件自绘，不使用原生表单控件。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
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
import ConfirmDialog from "@/components/common/ConfirmDialog.vue";
import { usePluginsStore } from "@/stores/plugins";
import { useRuntimesStore } from "@/stores/runtimes";
import EngineBackendPicker from "@/components/common/EngineBackendPicker.vue";
import Md3Segmented from "@/components/common/Md3Segmented.vue";
import type { SegmentedOption } from "@/components/common/segmented";
import { useUpdaterStore } from "@/stores/updater";
import { useI18n, type Locale } from "@/i18n";
import { copyText } from "@/utils/clipboard";
import { restartApp as restartApplication } from "@/utils/restart";
import { lastWorkspace } from "@/router";
import {
  agentConfig,
  agentModels,
  agentSetConfig,
  agentTest,
  aiBackendInfo,
  aiSetBackend,
  cliStatus,
  dataDirCleanupPrevious,
  dataDirInfo,
  dataDirMigrate,
  installCli,
  managerNetwork,
  managerProbeProxy,
  managerProxyStatus,
  managerSetNetwork,
  mcpStatus,
  onDataDirProgress,
  setMcpConfig,
  uninstallCli,
} from "@/api/tauri";
import { pickDirectory } from "@/api/dialogs";
import { humanSize } from "@/utils/models";
import type {
  BackendInfo,
  BackendPreference,
  CliStatus,
  DataDirInfo,
  DataDirProgress,
  DataDirSource,
  McpStatus,
  ModelSourcePreference,
  ProxyMode,
} from "@/api/types";

type Section =
  | "general"
  | "appearance"
  | "layout"
  | "storage"
  | "plugins"
  | "network"
  | "ai"
  | "cli"
  | "about";

const SECTIONS: { id: Section; labelKey: string; captionKey: string }[] = [
  { id: "general", labelKey: "settings.section.general", captionKey: "settings.section.general.caption" },
  { id: "appearance", labelKey: "settings.section.appearance", captionKey: "settings.section.appearance.caption" },
  { id: "layout", labelKey: "settings.section.layout", captionKey: "settings.section.layout.caption" },
  { id: "storage", labelKey: "settings.section.storage", captionKey: "settings.section.storage.caption" },
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

const APP_VERSION = "1.7.0";

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

/** 内容滚动容器：切换子页时回到顶部，避免沿用上一页（可能很长）的滚动位置。 */
const contentScroll = ref<HTMLElement | null>(null);
watch(active, () => contentScroll.value?.scrollTo({ top: 0 }));

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

/*
 * 设置页里的所有「几选一」都走同一个 `Md3Segmented`（滑块胶囊）：
 * 语言 / 主题 / 代理方式 / 下载源，以及下面的两个 AI 后端。之前这里各自手写
 * `.m3-seg`，而 AI 后端又用了另一套控件，宽度一紧就直接换行，看起来像两个组件。
 */
const localeSegOptions = computed<SegmentedOption[]>(() =>
  i18n.locales.map((l) => ({ value: l.id, label: l.label })),
);
const themeSegOptions = computed<SegmentedOption[]>(() =>
  THEMES.map((t) => ({ value: t.id, label: i18n.t(t.labelKey) })),
);
const proxyModeSegOptions = computed<SegmentedOption[]>(() =>
  PROXY_MODES.map((p) => ({ value: p.id, label: i18n.t(p.labelKey) })),
);
const sourceSegOptions = computed<SegmentedOption[]>(() =>
  SOURCE_PREFS.map((p) => ({ value: p.id, label: i18n.t(p.labelKey) })),
);
function onLocalePick(value: string): void {
  i18n.setLocale(value as Locale);
}
function onThemePick(value: string): void {
  onTheme(value as Theme);
}
function onProxyModePick(value: string): void {
  proxyMode.value = value as ProxyMode;
}
function onSourcePick(value: string): void {
  modelSourcePref.value = value as ModelSourcePreference;
}

// ---- MCP 服务器 ----
/** 配置与运行状态；加载失败（预览 / 后端不可用）时保持 null。 */
const mcp = ref<McpStatus | null>(null);
/** 端口输入框的草稿值（数字输入在编辑过程中不应该被后端回写打断）。 */
const mcpPort = ref("50562");
const mcpSaving = ref(false);
/** 上一次应用的失败原因，供界面提示。 */
const mcpError = ref("");

/** 配置端口没绑上、退回了临时端口时的一句话说明；正常监听时为 null。 */
const mcpFallbackNote = computed(() => {
  const status = mcp.value;
  if (!status?.fallback) return null;
  const key =
    status.portConflict === "denied"
      ? "settings.mcp.portConflictDenied"
      : "settings.mcp.portConflictBusy";
  return i18n.t(key, { port: status.port, bound: status.boundPort ?? status.port });
});

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
/** 运行时清单（切后端时要看那份运行时下没下、有多大）。 */
const runtimes = useRuntimesStore();
const availableBackends = computed(() => backendInfo.value?.options.filter((o) => o.available) ?? []);
/** 分段控件用的选项（与 `availableBackends` 一一对应；文案走 i18n）。 */
const backendSegOptions = computed<SegmentedOption[]>(() =>
  availableBackends.value.map((o) => ({
    value: o.kind,
    label: backendText(o.kind, "label", o.label),
    disabled: backendSaving.value,
    title: o.runtime_installed ? undefined : i18n.t("settings.ai.runtimeMissing"),
  })),
);
function onBackendPick(value: string): void {
  void setBackend(value as BackendPreference);
}
/** audio.cpp 引擎的后端包（与上面 ONNX Runtime 的后端是两套独立的东西）。 */

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

/** CUDA provider 硬依赖的四个组件（后端没下发 `missing` 时的兜底清单）。 */
function cudaParts(runtime: string): string[] {
  const suffix = runtime === "cuda13" ? "13" : "";
  return [`cudart${suffix}`, `cublas${suffix}`, `cufft${suffix}`, `cudnn${suffix}`];
}

/** 组件 id → 界面上显示的名字（清单还没加载时退回 id 本身）。 */
function partName(id: string): string {
  return runtimes.items.find((r) => r.id === id)?.label ?? id;
}

/**
 * 「看起来能用、实际在跑 CPU」的提示。
 *
 * 两种情况都必须说出来，否则用户只会看到「CUDA 已选」，然后发现显卡闲着：
 * 1. 组件缺（provider 载不进来）；
 * 2. provider 没注册成功（本次会话建的会话只能是 CPU）。
 */
const backendWarning = computed(() => {
  const info = backendInfo.value;
  if (!info || !info.preference.startsWith("cuda")) return "";
  const cuda = info.options.find((o) => o.kind === info.preference);
  if (!cuda || !cuda.available) return "";
  // 本体都没装：先说本体，别把「组件齐了」当成能跑。
  if (!cuda.runtime_installed) {
    return i18n.t("settings.ai.cudaCoreMissing", {
      version: info.preference === "cuda13" ? "13" : "12",
    });
  }
  const missing = cuda.missing ?? [];
  if (missing.length > 0) {
    return i18n.t("settings.ai.cudaPartsMissing", {
      parts: missing.map(partName).join("、"),
    });
  }
  // 运行时还没被加载过（启动后尚未建会话）时 `ep_ready` 必然是 false：
  // 这不是失败，只有加载过、注册仍失败才提示。
  if (!info.ep_ready && info.loaded_runtime) {
    return i18n.t("settings.ai.cudaNotActive");
  }
  return "";
});

async function setBackend(preference: BackendPreference): Promise<void> {
  if (backendInfo.value?.preference === preference) return;
  const option = backendInfo.value?.options.find((o) => o.kind === preference);
  if (!option) return;
  // 计算缺口：本体 +（CUDA 时）**全套组件**。**本体已装、组件缺**也要先补，
  // 不能只看 `runtime_installed`（它只代表 ONNX Runtime 本体）。
  // CUDA 的 provider dll 硬依赖这四个：cudart / cuBLAS / cuFFT / cuDNN，缺一个就载不进来，
  // ORT 会静默回落 CPU（以前漏了 cuFFT，正好踩在这一条上）。
  const pending: string[] = [];
  const core = runtimes.items.find((r) => r.id === option.runtime);
  if (!core || !core.installed) pending.push(option.runtime);
  // 组件缺口以后端下发的 `missing` 为准（它按磁盘实时算）；再并上已知的四件套兜底，
  // 这样即使清单还没加载完也不会漏装。CUDA 12 / 13 的组件 id 不同（13 带后缀）。
  if (option.runtime.startsWith("cuda")) {
    for (const id of [...(option.missing ?? []), ...cudaParts(option.runtime)]) {
      const item = runtimes.items.find((r) => r.id === id);
      if (item && !item.installed && !pending.includes(id)) pending.push(id);
    }
  }
  if (pending.length > 0) {
    const total = pending.reduce(
      (sum, id) => sum + (runtimes.items.find((r) => r.id === id)?.bytes ?? 0),
      0,
    );
    runtimePrompt.value = {
      kind: "missing",
      label: backendText(option.kind, "label", option.label),
      id: option.runtime,
      bytes: total,
      preference,
      pending,
    };
    return;
  }
  backendSaving.value = true;
  try {
    backendInfo.value = await aiSetBackend(preference);
    // 换运行时 = 换进程里那一份 dylib，下次启动才生效；
    // 进程里还没加载过运行时 / 已经加载的就是这一份时，不需要重启，也要明确告诉用户。
    if (backendInfo.value?.restart_required) {
      runtimePrompt.value = { kind: "restart" };
    } else {
      const label = backendText(option.kind, "label", option.label);
      const loaded = backendInfo.value?.loaded_runtime === option.runtime;
      layout.notify(
        i18n.t(
          loaded ? "settings.ai.backendAppliedCurrent" : "settings.ai.backendAppliedNext",
          { backend: label },
        ),
      );
    }
  } catch (err) {
    layout.notify(String(err));
  } finally {
    backendSaving.value = false;
  }
}

/** 重启提示正文：开发模式下补一句「不会自动重启」，别让人点完才发现。 */
const restartPromptBody = computed(() => {
  const body = i18n.t("settings.ai.restartBody");
  return import.meta.env.DEV ? `${body} ${i18n.t("settings.ai.restartDevHint")}` : body;
});

/** 切后端时的那两个提示：运行时缺失（先去下载）/ 已切换（重启生效）。 */
const runtimePrompt = ref<
  | {
      kind: "missing";
      label: string;
      id: string;
      bytes: number;
      preference: BackendPreference;
      pending: string[];
    }
  | { kind: "restart" }
  | null
>(null);

/** 缺失提示的正文（知道体积就报体积）。 */
const missingPromptBody = computed(() => {
  const prompt = runtimePrompt.value;
  if (prompt?.kind !== "missing") return "";
  return prompt.bytes > 0
    ? i18n.t("settings.ai.runtimeMissingBody", {
        backend: prompt.label,
        size: `${Math.round(prompt.bytes / 1048576)} MB`,
      })
    : i18n.t("settings.ai.runtimeMissingBodyUnknown", { backend: prompt.label });
});

/** 「去下载」：跳到下载管理器 → 聚焦「运行时」→ 下载 → 切回设置页 → 提示重启。 */
async function downloadMissingRuntime(): Promise<void> {
  const prompt = runtimePrompt.value;
  runtimePrompt.value = null;
  if (prompt?.kind !== "missing") return;
  // 带上第一个要装的 id：管理器会滚到那一行，而不是让用户在清单里找。
  runtimes.requestFocus(prompt.pending[0] ?? "*");
  openManager();
  for (const id of prompt.pending) {
    // 下 CUDA 本体时会顺带补齐同大版本的全部组件；轮到后面的组件 id 时它已经装好了，
    // 不要再把几 GB 的包重下一遍。
    if (runtimes.items.find((r) => r.id === id)?.installed) continue;
    await runtimes.install(id);
    // 后端下载是独立线程：等 done 事件把 items 刷新成 installed。
    if (!(await waitForInstalled(id))) return;
  }
  try {
    const info = await aiSetBackend(prompt.preference);
    backendInfo.value = info;
    // 只有「已经加载过旧运行时」才需要重启；首次使用会直接加载新运行时，
    // 这时弹重启只会打断用户（组件刚下完就弹窗的根因之一）。
    if (info.restart_required) runtimes.requestRestartPrompt();
  } catch (err) {
    layout.notify(String(err));
    return;
  }
  void router.push({ path: "/settings", query: { section: "ai" } });
}

/** 等运行时下载完成（done 事件会刷新 items）。 */
async function waitForInstalled(id: string): Promise<boolean> {
  for (let i = 0; i < 1800; i += 1) {
    const item = runtimes.items.find((r) => r.id === id);
    if (item?.installed) return true;
    if (runtimes.error) return false;
    await new Promise((resolve) => setTimeout(resolve, 1000));
  }
  return false;
}

/** 对话框关掉（取消 / 点遮罩）时清掉状态。 */
function dismissRuntimePrompt(open: boolean): void {
  if (!open) runtimePrompt.value = null;
}

/**
 * 重启应用；开发模式下自动重启会丢掉前端 dev server，改成提示手动重启。
 *
 * 守卫本身挪到了 [`restartApp`](@/utils/restart)：AI 面板 / 下载管理器 / 设置页
 * 都要走同一份逻辑，否则同一个「立即重启」按钮在三个地方行为不一致。
 */
function restartApp(): void {
  void restartApplication();
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

/** 打开管理器并落到「运行时」标签。 */
function openRuntimeManager(): void {
  const current = backendInfo.value?.preference;
  const target = current === "cuda13" ? "cuda13" : current === "cuda12" ? "cuda" : "*";
  runtimes.requestFocus(target);
  openManager();
}

// ---- 数据目录：查看 / 一键迁移 / 清理旧目录 ----

const dataDir = ref<DataDirInfo | null>(null);
const dataDirLoading = ref(false);
const dataDirBusy = ref(false);
const dataDirError = ref("");
const dataDirProgress = ref<DataDirProgress | null>(null);
/** 用户选好、等待确认的目标目录。 */
const dataDirPending = ref<string | null>(null);
const cleanupPrompt = ref(false);
const cleanupBusy = ref(false);
let offDataDir: (() => void) | null = null;

const dataDirSize = computed(() => humanSize(dataDir.value?.size_bytes ?? 0));
/** 已迁移、等重启时的新目录；没有则为空。 */
const dataDirNext = computed(() =>
  dataDir.value?.restart_required ? dataDir.value.next : null,
);

const dataDirSource = computed(() => {
  const source: DataDirSource = dataDir.value?.source ?? "default";
  return i18n.t(`settings.storage.source.${source}`);
});

/** 迁移进度文案：扫描阶段不知道总量，复制阶段按字节报百分比。 */
const dataDirProgressText = computed(() => {
  const p = dataDirProgress.value;
  if (!p || p.phase === "scan") return i18n.t("settings.storage.scanning");
  if (p.total <= 0) return i18n.t("settings.storage.copying", { done: humanSize(p.done) });
  const percent = Math.min(100, Math.round((p.done / p.total) * 100));
  return `${i18n.t("settings.storage.copying", { done: `${humanSize(p.done)} / ${humanSize(p.total)}` })} · ${percent}%`;
});

const dataDirPercent = computed(() => {
  const p = dataDirProgress.value;
  if (!p || p.total <= 0) return 0;
  return Math.min(100, Math.round((p.done / p.total) * 100));
});

async function loadDataDir(): Promise<void> {
  dataDirLoading.value = true;
  dataDirError.value = "";
  try {
    dataDir.value = await dataDirInfo();
  } catch (err) {
    dataDirError.value = String(err);
  } finally {
    dataDirLoading.value = false;
  }
}

async function chooseDataDir(): Promise<void> {
  const dir = await pickDirectory();
  if (dir) dataDirPending.value = dir;
}

function onDataDirConfirmOpen(open: boolean): void {
  if (!open) dataDirPending.value = null;
}

function onCleanupConfirmOpen(open: boolean): void {
  if (!open) cleanupPrompt.value = false;
}

/** 执行迁移：复制 → 写指针 → 提示重启。旧目录留到重启后再清理。 */
async function migrateDataDir(): Promise<void> {
  const target = dataDirPending.value;
  if (!target || dataDirBusy.value) return;
  dataDirPending.value = null;
  dataDirBusy.value = true;
  dataDirError.value = "";
  dataDirProgress.value = { phase: "scan", done: 0, total: 0, files: 0, message: null };
  // 把当前工作状态先落盘（10 秒一次的自动缓存可能还没轮到），否则迁移复制的是旧快照。
  await project.cacheSession().catch(() => {});
  try {
    offDataDir = await onDataDirProgress((p) => {
      dataDirProgress.value = p;
    });
    const next = await dataDirMigrate(target);
    dataDir.value = next;
    await loadDataDir();
    layout.notify(
      i18n.t("settings.storage.done", { path: next.next ?? target }),
    );
  } catch (err) {
    dataDirError.value = String(err);
    layout.notify(i18n.t("settings.storage.failed", { error: String(err) }));
  } finally {
    offDataDir?.();
    offDataDir = null;
    dataDirBusy.value = false;
  }
}

/** 清理上一次迁移留下的旧目录（重启后才会真的释放）。 */
async function cleanupPreviousDataDir(): Promise<void> {
  if (cleanupBusy.value) return;
  cleanupBusy.value = true;
  cleanupPrompt.value = false;
  try {
    const removed = await dataDirCleanupPrevious();
    if (dataDir.value) dataDir.value = { ...dataDir.value, previous: null };
    layout.notify(
      removed
        ? i18n.t("settings.storage.cleaned", { path: removed })
        : i18n.t("settings.storage.nothingToClean"),
    );
  } catch (err) {
    layout.notify(i18n.t("settings.storage.cleanupFailed", { error: String(err) }));
  } finally {
    cleanupBusy.value = false;
  }
}

onMounted(() => {
  void loadMcp();
  void loadCli();
  void loadStoreNetwork();
  void loadBackend();
  void loadDataDir();
  // 子组件（EngineBackendPicker）的 onMounted 比这里先跑；别重复拉同一张清单。
  if (runtimes.items.length === 0 && !runtimes.loading) void runtimes.load();
  void loadAgent();
  void updater.loadVersion();
  // 深链：`/settings?section=ai` 直接落到 AI / MCP 子页（写歌智能体的「去配置」会这么跳）。
  const section = router.currentRoute.value.query.section;
  if (typeof section === "string" && SECTIONS.some((s) => s.id === section)) {
    active.value = section as Section;
  }
  // 运行时下载完自动切回设置页：把「重启生效」弹框补上。
  if (runtimes.consumeRestartPrompt()) runtimePrompt.value = { kind: "restart" };
});

onUnmounted(() => {
  offDataDir?.();
  offDataDir = null;
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
    <section ref="contentScroll" class="settings-body">
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
                <div class="row-control">
                  <Md3Segmented
                    :model-value="i18n.locale.value"
                    :options="localeSegOptions"
                    :aria-label="i18n.t('settings.language.label')"
                    @update:model-value="onLocalePick"
                  />
                </div>
              </div>
            </div>
          </div>

          <div class="group">
            <div class="group-title">{{ i18n.t("settings.startup") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.welcome.label") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.welcome.hint") }}</span>
                </div>
                <button
                  class="m3-switch"
                  :class="{ on: settings.welcomeOnStart }"
                  role="switch"
                  :aria-checked="settings.welcomeOnStart"
                  :aria-label="i18n.t('settings.welcome.label')"
                  @click="settings.welcomeOnStart = !settings.welcomeOnStart"
                />
              </div>
            </div>
          </div>
        </template>

        <!-- 存储：数据目录（模型 / 运行时 / 插件 / AI 产物） -->
        <template v-else-if="active === 'storage'">
          <div class="group">
            <div class="group-title">{{ i18n.t("settings.storage.location") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.storage.current") }}</span>
                  <span class="row-sub path">{{ dataDir?.current ?? i18n.t("common.loading") }}</span>
                </div>
                <span class="row-value">{{ dataDirSize }}</span>
              </div>
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.storage.source.label") }}</span>
                  <span class="row-sub">{{ dataDirSource }}</span>
                </div>
              </div>
              <div v-if="dataDir && dataDir.default !== dataDir.current" class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.storage.default") }}</span>
                  <span class="row-sub path">{{ dataDir.default }}</span>
                </div>
              </div>
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.storage.move") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.storage.moveHint") }}</span>
                </div>
                <button
                  class="scan-btn"
                  :disabled="dataDirBusy || dataDirLoading || dataDir?.portable === true"
                  @click="chooseDataDir"
                >
                  {{ i18n.t("settings.storage.choose") }}
                </button>
              </div>
              <div v-if="dataDir?.portable" class="col-row">
                <span class="row-sub warn">{{ i18n.t("settings.storage.portable") }}</span>
              </div>
              <div v-if="dataDirBusy" class="col-row">
                <div
                  class="update-progress"
                  role="progressbar"
                  :aria-valuenow="dataDirPercent"
                  aria-valuemin="0"
                  aria-valuemax="100"
                >
                  <div class="update-progress-fill" :style="{ width: `${dataDirPercent}%` }" />
                </div>
                <span class="row-sub">{{ dataDirProgressText }}</span>
              </div>
              <div v-if="dataDirError" class="col-row">
                <span class="row-sub warn">{{ dataDirError }}</span>
              </div>
            </div>
          </div>

          <!-- 迁移完成：重启前当前进程仍写旧目录，必须显眼地提醒。 -->
          <div v-if="dataDirNext" class="group">
            <div class="group-title">{{ i18n.t("settings.storage.migratedTitle") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.storage.migrated", { path: dataDirNext }) }}</span>
                  <span class="row-sub">{{ i18n.t("settings.storage.migratedHint") }}</span>
                </div>
                <button class="scan-btn" @click="restartApp">
                  {{ i18n.t("settings.storage.restartNow") }}
                </button>
              </div>
            </div>
          </div>

          <!-- 旧目录清理：重启后执行；指针文件留在默认目录，不受影响。 -->
          <div v-if="dataDir?.previous && !dataDir?.restart_required" class="group">
            <div class="group-title">{{ i18n.t("settings.storage.previousTitle") }}</div>
            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.storage.previous") }}</span>
                  <span class="row-sub path">{{ dataDir.previous }}</span>
                  <span class="row-sub">{{ i18n.t("settings.storage.previousHint") }}</span>
                </div>
                <button class="scan-btn" :disabled="cleanupBusy" @click="cleanupPrompt = true">
                  {{ i18n.t("settings.storage.cleanup") }}
                </button>
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
                <div class="row-control">
                  <Md3Segmented
                    :model-value="theme"
                    :options="themeSegOptions"
                    :aria-label="i18n.t('settings.theme.label')"
                    @update:model-value="onThemePick"
                  />
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
                <div class="row-control">
                  <Md3Segmented
                    :model-value="proxyMode"
                    :options="proxyModeSegOptions"
                    :aria-label="i18n.t('settings.proxy.mode')"
                    @update:model-value="onProxyModePick"
                  />
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
                <div class="row-control">
                  <Md3Segmented
                    :model-value="modelSourcePref"
                    :options="sourceSegOptions"
                    :aria-label="i18n.t('settings.store.source')"
                    @update:model-value="onSourcePick"
                  />
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
                <div class="row-control">
                  <Md3Segmented
                    v-if="backendSegOptions.length"
                    :model-value="backendInfo?.preference ?? ''"
                    :options="backendSegOptions"
                    :aria-label="i18n.t('settings.ai.backend')"
                    @update:model-value="onBackendPick"
                  />
                </div>
              </div>
              <div class="col-row">
                <span class="row-sub" v-if="backendNote">{{ backendNote }}</span>
                <span class="row-sub">
                  {{ i18n.t("settings.ai.platform", { platform: backendInfo?.platform ?? "…" }) }}
                </span>
              </div>
              <div class="col-row" v-if="backendWarning">
                <span class="row-sub warn">{{ backendWarning }}</span>
              </div>
            </div>

            <div class="card">
              <EngineBackendPicker row />
            </div>

            <div class="card">
              <div class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.ai.runtimes") }}</span>
                  <span class="row-sub">{{ i18n.t("settings.ai.runtimesHint") }}</span>
                </div>
                <div class="row-actions">
                  <span class="row-sub restart-hint" v-if="backendInfo?.restart_required">
                    {{ i18n.t("settings.ai.restartRequired") }}
                  </span>
                  <button class="scan-btn" @click="openRuntimeManager">
                    {{ i18n.t("settings.ai.runtimeOpen") }}
                  </button>
                </div>
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
              <div v-if="mcpFallbackNote" class="row">
                <div class="row-text">
                  <span class="row-title">{{ i18n.t("settings.mcp.portConflict") }}</span>
                  <span class="row-sub warn">{{ mcpFallbackNote }}</span>
                </div>
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

  <!-- 切后端后的两种提示：运行时缺失 → 去下载；已切换 → 重启生效 -->
  <ConfirmDialog
    :open="runtimePrompt?.kind === 'missing'"
    :title="i18n.t('settings.ai.runtimeMissingTitle')"
    :message="missingPromptBody"
    :confirm-label="i18n.t('settings.ai.runtimeMissingGo')"
    @confirm="downloadMissingRuntime"
    @update:open="dismissRuntimePrompt"
  />
  <ConfirmDialog
    :open="runtimePrompt?.kind === 'restart'"
    :title="i18n.t('settings.ai.restartTitle')"
    :message="restartPromptBody"
    :confirm-label="i18n.t('settings.ai.restartNow')"
    @confirm="restartApp"
    @update:open="dismissRuntimePrompt"
  />

  <!-- 数据目录迁移确认：说清楚「复制而不是移动」以及要重启。 -->
  <ConfirmDialog
    :open="dataDirPending !== null"
    :title="i18n.t('settings.storage.confirmTitle')"
    :message="i18n.t('settings.storage.confirmBody', { path: dataDirPending ?? '' })"
    :confirm-label="i18n.t('settings.storage.confirmGo')"
    @confirm="migrateDataDir"
    @update:open="onDataDirConfirmOpen"
  />

  <!-- 清理旧目录确认：不可撤销。 -->
  <ConfirmDialog
    :open="cleanupPrompt"
    :title="i18n.t('settings.storage.cleanupTitle')"
    :message="i18n.t('settings.storage.cleanupBody', { path: dataDir?.previous ?? '' })"
    :confirm-label="i18n.t('settings.storage.cleanupGo')"
    @confirm="cleanupPreviousDataDir"
    @update:open="onCleanupConfirmOpen"
  />
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
  scrollbar-gutter: stable;
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
  /* Windows 的滚动条占布局宽度：预留给它，切换长短不同的子页时内容不会横向跳动 */
  scrollbar-gutter: stable;
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
  /* 标题 / 说明吃掉剩余宽度，右侧控件按自身内容定宽，不会被长说明挤到换行。 */
  flex: 1 1 0;
  min-width: 0;
}
/* 行右侧的控件：分段滑块、开关、按钮等都不参与收缩。 */
.row-control {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  max-width: 100%;
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
/* 数据目录路径：长、可选中复制；用等宽字体避免 `\` 与 `l` 看混。 */
.row-sub.path {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 11px;
  overflow-wrap: anywhere;
  user-select: text;
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
/* 警告：后端看起来可用、但其实会回落 CPU（组件缺失 / provider 没注册成功）。 */
.row-sub.warn {
  color: rgb(var(--md-error));
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
  .row-control {
    width: 100%;
    justify-content: flex-start;
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
  flex: 0 0 auto;
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

/* ---- ONNX Runtime 运行时（下载 / 删除） ---- */
.runtime-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 10px 0;
  border-top: 1px solid rgb(var(--md-outline-variant) / 0.4);
}
.runtime-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-shrink: 0;
}
.runtime-tag {
  margin-left: 6px;
  font-style: normal;
  font-size: 0.85em;
  color: rgb(var(--md-primary));
}
.restart-hint {
  color: rgb(var(--md-primary));
}
.runtime-progress {
  margin: 4px 0 10px;
}
.runtime-error {
  color: #d93025;
}
</style>
