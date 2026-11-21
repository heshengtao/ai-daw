/**
 * 扩展插件（Extensions）前端状态。
 *
 * 扩展是 **npm 分发的应用级插件**（和 VST3/CLAP 那种 DSP 插件是两回事）：Host 侧跑在
 * daemon 的 QuickJS 里，能给 MCP 加工具；Client 侧是一段 ESM，跑在 webview 里，往右栏
 * 注册面板。这里只管 Client 侧：拉清单、把模块当 blob 动态 `import()`、注册面板、热重载。
 *
 * 为什么是 blob 而不是 `import("http://…")`：Tauri 的 `tauri://` 与浏览器的 `http://`
 * 是两种源，跨源模块要 CORS；blob 与页面同源，两种传输下都成立，也不要额外的静态路由。
 * 代价是模块必须**自包含**（不能 `import "vue"`，Vue 从 `api.vue` 取）。
 */
import {
  computed,
  defineComponent,
  h,
  markRaw,
  onMounted,
  onUnmounted,
  ref,
  shallowRef,
  shallowReactive,
  watch,
  type Component,
} from "vue";
import { defineStore } from "pinia";

import { extensionsRpc } from "@/api/tauri";
import { listen } from "@/api/ipc";
import { useI18n } from "@/i18n";
import { registerPaneId, unregisterPaneId, useLayoutStore } from "@/stores/layout";

/** 扩展状态变化事件（与后端 `extensions::EVENT_EXTENSIONS` 一致）。 */
export const EVENT_EXTENSIONS = "extensions://changed";

/** 本地化文本：字符串或语言映射。 */
export type Localized = string | Record<string, string>;

/** 已装扩展。 */
export interface InstalledExtension {
  name: string;
  version: string;
  enabled: boolean;
  source: { type: string; registry?: string | null; index?: string | null; path?: string | null };
  capabilities: string[];
  permissions: string[];
  config: unknown;
  installedAt: number;
  state: "loaded" | "failed" | "disabled" | "idle";
  error: string | null;
  tools: string[];
  panes: string[];
}

/** 扩展声明的面板。 */
export interface ExtPaneMeta {
  id: string;
  plugin: string;
  title: Localized;
  icon: string;
  order: number;
}

/** 扩展注册的工具。 */
export interface ExtToolMeta {
  name: string;
  plugin: string;
  description: string;
  parameters: Record<string, unknown>;
  risk: "read" | "write" | "dangerous";
}

/** `list` 的快照。 */
export interface ExtSnapshot {
  appVersion: string;
  indexes: string[];
  registry: string;
  mirrors: string[];
  installed: InstalledExtension[];
  panes: ExtPaneMeta[];
  tools: ExtToolMeta[];
  contents: unknown[];
}

/** 市场里的一个版本。 */
export interface MarketVersion {
  version: string;
  publishTimestamp: number;
  install: string;
  compatible: boolean;
  reason: string | null;
}

/** 市场里的一个插件。 */
export interface MarketEntry {
  name: string;
  title: string;
  description: string | null;
  icon: string | null;
  author: string | null;
  homepage: string | null;
  indexUrl: string;
  category: string | null;
  labels: string[];
  keywords: string[];
  defaultVersion: string | null;
  versions: MarketVersion[];
  latestPublish: number;
  installed: { status: string; version: string | null };
}

/** 一个索引源的读取概况。 */
export interface MarketSource {
  url: string;
  title: string | null;
  fetchedAt: number | null;
  error: string | null;
  fromCache: boolean;
}

/** 一页搜索结果。 */
export interface MarketPage {
  entries: MarketEntry[];
  total: number;
  nextOffset: number | null;
  sources: MarketSource[];
}

/** Client 侧待加载的模块。 */
export interface ClientItem {
  name: string;
  version: string;
  title: string;
  panes: { id: string; title?: Localized; icon?: string; order?: number }[];
}

/** 面板组件注册时的形状。 */
export interface PaneRegistration {
  id: string;
  component: Component;
}

/** 插件拿到的宿主 API（挂到 `window.aidawExt`）。 */
export interface HostApi {
  extension: { name: string; version: string };
  registerPane(spec: { id: string; component: Component }): void;
  invoke<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  on(event: string, handler: (payload: unknown) => void): () => void;
  vue: Record<string, unknown>;
  i18n: { t: (key: string) => string; locale: string };
}

declare global {
  interface Window {
    aidawExt?: HostApi;
  }
}

/** 按当前语言取本地化文本。 */
export function localized(text: Localized | null | undefined, locale: string): string {
  if (!text) return "";
  if (typeof text === "string") return text;
  const exact = text[locale] ?? text[locale.split("-")[0]];
  if (exact) return exact;
  return text.en ?? Object.values(text)[0] ?? "";
}

export const useExtensionsStore = defineStore("extensions", () => {
  const i18n = useI18n();

  const snapshot = ref<ExtSnapshot | null>(null);
  const market = ref<MarketPage | null>(null);
  const marketBusy = ref(false);
  const marketError = ref<string | null>(null);
  const marketQuery = ref("");

  /** 面板 id → 组件（扩展注册进来的）。 */
  const components = shallowRef<Record<string, Component>>({});
  /** 扩展 Client 侧的加载错误（按包名）。 */
  const clientErrors = ref<Record<string, string>>({});
  /** 已加载的 Client 版本（按包名），用于判断是否需要重载。 */
  const clientVersions = new Map<string, string>();
  /** 正在加载的包（避免事件风暴里重复加载）。 */
  const loading = new Set<string>();

  let started = false;
  let unlisten: (() => void) | null = null;

  const installed = computed(() => snapshot.value?.installed ?? []);
  const tools = computed(() => snapshot.value?.tools ?? []);
  const indexes = computed(() => snapshot.value?.indexes ?? []);
  const registry = computed(() => snapshot.value?.registry ?? "");

  /** 右栏可打开的面板（内置 + 扩展，带展示文案）。 */
  const paneEntries = computed(() => {
    const items = (snapshot.value?.panes ?? []).map((pane) => ({
      id: pane.id,
      label: localized(pane.title, i18n.locale.value) || pane.id,
      hint: `${pane.plugin}`,
      icon: pane.icon,
      order: pane.order,
    }));
    return items.sort((a, b) => a.order - b.order);
  });

  function paneLabel(id: string): string | null {
    const panic = paneEntries.value.find((entry) => entry.id === id);
    return panic ? panic.label : null;
  }

  function componentFor(id: string): Component | null {
    return components.value[id] ?? null;
  }

  async function refresh(): Promise<void> {
    try {
      snapshot.value = await extensionsRpc<ExtSnapshot>("list");
    } catch (err) {
      clientErrors.value = { ...clientErrors.value, "": String(err) };
    }
  }

  /**
   * 装载所有扩展的 Client 模块。
   *
   * 版本没变的跳过（热重载事件会被重复触发）；版本变了先卸载旧的再装新的。
   */
  async function loadClients(): Promise<void> {
    let items: ClientItem[] = [];
    try {
      const page = await extensionsRpc<{ items: ClientItem[] }>("client_manifest");
      items = page.items;
    } catch (err) {
      clientErrors.value = { ...clientErrors.value, "": String(err) };
      return;
    }

    // 已经卸载的扩展：把它的面板收掉。
    const alive = new Set(items.map((item) => item.name));
    for (const [name, version] of [...clientVersions]) {
      if (!alive.has(name)) {
        unloadClient(name, version);
      }
    }

    for (const item of items) {
      if (clientVersions.get(item.name) === item.version || loading.has(item.name)) continue;
      loading.add(item.name);
      try {
        const payload = await extensionsRpc<{ name: string; version: string; source: string }>("client_source", {
          name: item.name,
        });
        // 同一个包换了版本：先把旧面板撤掉，再装新的。
        const previous = clientVersions.get(item.name);
        if (previous) unloadClient(item.name, previous);

        window.aidawExt = buildApi(item);
        const url = URL.createObjectURL(new Blob([payload.source], { type: "text/javascript" }));
        try {
          const module = (await import(/* @vite-ignore */ url)) as { default?: unknown };
          if (typeof module.default === "function") {
            await (module.default as (api: HostApi) => unknown)(window.aidawExt);
          }
        } finally {
          URL.revokeObjectURL(url);
        }
        clientVersions.set(item.name, item.version);
        delete clientErrors.value[item.name];
      } catch (err) {
        clientErrors.value = { ...clientErrors.value, [item.name]: String(err) };
      } finally {
        loading.delete(item.name);
      }
    }
  }

  /** 撤掉一个扩展贡献的面板（关掉已打开的标签，注销 id）。 */
  function unloadClient(name: string, version: string): void {
    void version;
    if (snapshot.value) {
      for (const pane of snapshot.value.panes) {
        if (pane.plugin !== name) continue;
        unregisterPaneId(pane.id);
        useLayoutStore().closeTab(pane.id);
        const next = { ...components.value };
        delete next[pane.id];
        components.value = next;
      }
    }
    clientVersions.delete(name);
  }

  function buildApi(item: ClientItem): HostApi {
    const layout = useLayoutStore();
    return {
      extension: { name: item.name, version: item.version },
      registerPane(spec) {
        if (!spec || typeof spec.id !== "string" || !spec.id) throw new Error("registerPane: 缺 id");
        if (!spec.component) throw new Error(`registerPane(${spec.id}): 缺 component`);
        if (components.value[spec.id]) {
          throw new Error(`面板 id「${spec.id}」已被占用（扩展之间不能重名）`);
        }
        components.value = { ...components.value, [spec.id]: markRaw(spec.component) };
        registerPaneId(spec.id);
        // 清单里没声明过的面板：补一条展示信息，保证它会出现在标签选择器里。
        if (snapshot.value && !snapshot.value.panes.some((pane) => pane.id === spec.id)) {
          snapshot.value = {
            ...snapshot.value,
            panes: [
              ...snapshot.value.panes,
              { id: spec.id, plugin: item.name, title: spec.id, icon: "puzzle", order: 900 },
            ],
          };
        }
        void layout;
      },
      // 前端扩展调宿主：走 `call_tool`（daemon 侧先查 /rpc 命令表，未命中落到 MCP 工具），
      // 所以 `invoke("get_project_overview")` 与 `invoke("set_tempo")` 都能用。
      invoke: (cmd, args) => extensionsRpc("call_tool", { name: cmd, args: args ?? {} }),
      on(event, handler) {
        let dispose = () => {};
        void listen(event, (payload) => handler(payload)).then((fn) => {
          dispose = fn;
        });
        return () => dispose();
      },
      vue: { h, defineComponent, ref, computed, shallowRef, shallowReactive, markRaw, onMounted, onUnmounted, watch },
      i18n: { t: i18n.t, locale: i18n.locale.value },
    };
  }

  /** 市场：拉一页（`refresh` 会跳过缓存）。 */
  async function loadMarket(options: { refresh?: boolean; reset?: boolean } = {}): Promise<void> {
    marketBusy.value = true;
    marketError.value = null;
    try {
      const page = await extensionsRpc<MarketPage>("market", {
        refresh: options.refresh ?? false,
        text: marketQuery.value,
        limit: 100,
      });
      market.value = options.reset === false && market.value ? { ...page, entries: [...market.value.entries, ...page.entries] } : page;
    } catch (err) {
      marketError.value = String(err);
    } finally {
      marketBusy.value = false;
    }
  }

  async function searchMarket(text: string): Promise<void> {
    marketQuery.value = text;
    await loadMarket({ reset: true });
  }

  /** 安装：市场条目 → 精确版本；不传版本时由 registry 选兼容的最新版。 */
  async function install(name: string, version?: string | null, index?: string | null): Promise<void> {
    await extensionsRpc("install", { name, version: version ?? null, index: index ?? null });
    await refresh();
    await loadClients();
  }

  async function installLocal(path: string): Promise<void> {
    await extensionsRpc("install_local", { path });
    await refresh();
    await loadClients();
  }

  async function uninstall(name: string): Promise<void> {
    await extensionsRpc("uninstall", { name });
    await refresh();
    await loadClients();
  }

  async function setEnabled(name: string, enabled: boolean): Promise<void> {
    await extensionsRpc("set_enabled", { name, enabled });
    await refresh();
    await loadClients();
  }

  async function reload(name: string): Promise<void> {
    await extensionsRpc("reload", { name });
    await refresh();
    // 版本号没变也要强制重装 Client 模块（用户改的就是文件，不是版本）。
    const version = clientVersions.get(name);
    if (version) unloadClient(name, version);
    await loadClients();
  }

  async function saveIndexes(urls: string[]): Promise<void> {
    await extensionsRpc("set_indexes", { indexes: urls });
    await refresh();
    market.value = null;
  }

  async function saveRegistry(registryUrl: string, mirrors: string[]): Promise<void> {
    await extensionsRpc("set_registry", { registry: registryUrl, mirrors });
    await refresh();
  }

  /** 启动一次（主窗口调用）；重复调用无副作用。 */
  async function init(): Promise<void> {
    if (started) return;
    started = true;
    await refresh();
    await loadClients();
    try {
      unlisten = await listen(EVENT_EXTENSIONS, () => {
        void (async () => {
          await refresh();
          await loadClients();
        })();
      });
    } catch {
      /* 浏览器预览模式没有事件流：忽略 */
    }
  }

  function dispose(): void {
    if (unlisten) unlisten();
    unlisten = null;
    started = false;
  }

  return {
    snapshot,
    market,
    marketBusy,
    marketError,
    marketQuery,
    installed,
    tools,
    indexes,
    registry,
    paneEntries,
    clientErrors,
    paneLabel,
    componentFor,
    refresh,
    loadClients,
    loadMarket,
    searchMarket,
    install,
    installLocal,
    uninstall,
    setEnabled,
    reload,
    saveIndexes,
    saveRegistry,
    init,
    dispose,
  };
});
