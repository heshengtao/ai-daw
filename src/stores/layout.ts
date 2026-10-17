/**
 * 布局偏好与轻量通知（snackbar）：面板可见性与宽度、右侧面板的标签页，持久化到 localStorage。
 */
import { defineStore } from "pinia";
import { ref, watch } from "vue";

const KEY = "ai-daw-layout";

/** 右侧面板可打开的窗口类型；后续新增面板只需在此登记。 */
export type PaneId =
  | "inspector"
  | "mixer"
  | "plugins"
  | "manager"
  | "editor"
  | "agent"
  | "ai";

/**
 * 鼠标模式（顶栏三选一）：
 * - `draw`：在空白处左键拖动 = 画出片段 / 音符（创建 / 拖动）；
 * - `select`：左键拖动 = 框选；
 * - `split`：左键点击片段 = 在该处切开（razor）。
 *
 * 三者共用同一个手势，只能靠模式区分，因此必须由用户显式选择。
 */
export type Tool = "draw" | "select" | "split";

/** 全部鼠标模式（顺序即顶栏里的展示顺序）。 */
export const TOOLS: Tool[] = ["draw", "select", "split"];

/** 全部可用面板的元数据（供"新建标签"选择器展示）。文案是 i18n key，渲染时翻译。 */
export const PANES: { id: PaneId; labelKey: string; hintKey: string }[] = [
  { id: "inspector", labelKey: "panes.inspector", hintKey: "panes.inspectorHint" },
  { id: "mixer", labelKey: "panes.mixer", hintKey: "panes.mixerHint" },
  { id: "editor", labelKey: "panes.editor", hintKey: "panes.editorHint" },
  { id: "plugins", labelKey: "panes.plugins", hintKey: "panes.pluginsHint" },
  { id: "manager", labelKey: "panes.manager", hintKey: "panes.managerHint" },
  { id: "agent", labelKey: "panes.agent", hintKey: "panes.agentHint" },
  { id: "ai", labelKey: "panes.ai", hintKey: "panes.aiHint" },
];

/**
 * 可以**手动**打开的面板（「+」选择器与「尚未打开任何面板」空态共用）。
 *
 * 不含「插件界面」：那个面板是**跟着插件走**的，入口在插件自己的按钮上（插件清单页 /
 * 检查器里那条轨道的「插件界面」按钮）。凭空打开一个没有目标的插件界面只会让人困惑。
 */
export const PICKABLE_PANES = PANES.filter((p) => p.id !== "editor");

/** 左侧音轨栏宽度上限：窗口宽度的 1/4。 */
export const MAX_TRACK_WIDTH_RATIO = 0.25;
/** 右侧面板宽度上限：扣除左侧后，剩余区域的 2/3。 */
export const MAX_INSPECTOR_RATIO = 2 / 3;

/** 左侧音轨栏宽度下限（px）：再窄内部元素（旋钮、按钮行）就会被压变形。 */
const MIN_TRACK_WIDTH = 200;
/** 右侧面板宽度下限（px）：插件行、参数字段需要的最小呼吸空间。 */
const MIN_INSPECTOR_WIDTH = 300;

interface Persisted {
  showTrackHeaders: boolean;
  showInspector: boolean;
  trackWidth: number;
  inspectorWidth: number;
  tabs: PaneId[];
  activeTab: PaneId;
  tool: Tool;
  /** 钢琴卷帘下方控制器泳道的高度（px）。 */
  laneHeight: number;
  /** 编排视图底部的自动化曲线编辑器是否打开。 */
  showAutomation: boolean;
  /** 自动化曲线编辑器的高度（px）。 */
  automationHeight: number;
  /** 钢琴卷帘下方的控制器（力度 / 音量）泳道是否打开。 */
  showControllerLane: boolean;
}

function load(): Partial<Persisted> {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "{}") as Partial<Persisted>;
  } catch {
    return {};
  }
}

const VALID_PANES: PaneId[] = PANES.map((p) => p.id);

function sanitizeTabs(value: unknown): PaneId[] {
  if (!Array.isArray(value)) return ["inspector"];
  const tabs = value.filter((v): v is PaneId => VALID_PANES.includes(v as PaneId));
  return tabs.length > 0 ? tabs : ["inspector"];
}

/** 控制器泳道的高度范围（px）。 */
const MIN_LANE_HEIGHT = 56;
const MAX_LANE_HEIGHT = 320;

function clampLaneHeight(value: number): number {
  if (!Number.isFinite(value)) return 92;
  return Math.min(MAX_LANE_HEIGHT, Math.max(MIN_LANE_HEIGHT, Math.round(value)));
}

/** 自动化曲线编辑器的高度范围（px）。 */
const MIN_AUTOMATION_HEIGHT = 80;
const MAX_AUTOMATION_HEIGHT = 360;

function clampAutomationHeight(value: number): number {
  if (!Number.isFinite(value)) return 150;
  return Math.min(MAX_AUTOMATION_HEIGHT, Math.max(MIN_AUTOMATION_HEIGHT, Math.round(value)));
}

export const useLayoutStore = defineStore("layout", () => {
  const saved = load();
  const showTrackHeaders = ref(saved.showTrackHeaders ?? true);
  const showInspector = ref(saved.showInspector ?? true);
  const trackWidth = ref(saved.trackWidth ?? 220);
  // 默认给宽一点：复杂插件（大量参数 / 将来的原生界面）需要地方。
  const inspectorWidth = ref(saved.inspectorWidth ?? 380);
  const tabs = ref<PaneId[]>(sanitizeTabs(saved.tabs));
  const activeTab = ref<PaneId>(
    saved.activeTab && tabs.value.includes(saved.activeTab) ? saved.activeTab : tabs.value[0],
  );
  // 默认「绘制」：创建是最常用的动作，框选 / 切分随时可以切（顶栏按钮 / 快捷键 1 / 2 / 3）。
  const tool = ref<Tool>(TOOLS.includes(saved.tool as Tool) ? (saved.tool as Tool) : "draw");
  const laneHeight = ref(clampLaneHeight(saved.laneHeight ?? 92));
  // 两个底部窗格默认都打开：它们是各自视图的常规工作区，不是可选的调试面板。
  const showAutomation = ref(saved.showAutomation ?? true);
  const automationHeight = ref(clampAutomationHeight(saved.automationHeight ?? 150));
  const showControllerLane = ref(saved.showControllerLane ?? true);
  const devDrawer = ref(false);

  /** 当前提示文案（自动消失）。 */
  const toast = ref<string | null>(null);
  let toastTimer: number | null = null;

  watch(
    [
      showTrackHeaders,
      showInspector,
      trackWidth,
      inspectorWidth,
      tabs,
      activeTab,
      tool,
      laneHeight,
      showAutomation,
      automationHeight,
      showControllerLane,
    ],
    () => {
      localStorage.setItem(
        KEY,
        JSON.stringify({
          showTrackHeaders: showTrackHeaders.value,
          showInspector: showInspector.value,
          trackWidth: trackWidth.value,
          inspectorWidth: inspectorWidth.value,
          tabs: tabs.value,
          activeTab: activeTab.value,
          tool: tool.value,
          laneHeight: laneHeight.value,
          showAutomation: showAutomation.value,
          automationHeight: automationHeight.value,
          showControllerLane: showControllerLane.value,
        } satisfies Persisted),
      );
    },
    { deep: true },
  );

  /** 切换鼠标模式。 */
  function setTool(next: Tool): void {
    tool.value = next;
  }

  /** 调整控制器泳道高度（拖拽分隔线时调用）。 */
  function resizeLane(delta: number): void {
    laneHeight.value = clampLaneHeight(laneHeight.value + delta);
  }

  /** 调整自动化编辑器高度（拖拽分隔线时调用）。 */
  function resizeAutomation(delta: number): void {
    automationHeight.value = clampAutomationHeight(automationHeight.value + delta);
  }

  // ---- 面板宽度：左右各有自己的上限 ----

  /** 当前视口宽度。 */
  function viewportWidth(): number {
    return typeof window === "undefined" ? 1280 : window.innerWidth;
  }

  /** 左侧音轨栏的宽度上限：窗口的 1/4。 */
  function maxTrackWidth(): number {
    return Math.max(MIN_TRACK_WIDTH, Math.round(viewportWidth() * MAX_TRACK_WIDTH_RATIO));
  }

  /** 右侧面板的宽度上限：扣掉左侧后，剩余区域的 2/3。 */
  function maxInspectorWidth(): number {
    const used = showTrackHeaders.value ? trackWidth.value : 0;
    return Math.max(MIN_INSPECTOR_WIDTH, Math.round((viewportWidth() - used) * MAX_INSPECTOR_RATIO));
  }

  function clampTrackWidth(value: number): number {
    if (!Number.isFinite(value)) return 220;
    return Math.min(maxTrackWidth(), Math.max(MIN_TRACK_WIDTH, Math.round(value)));
  }

  function clampInspectorWidth(value: number): number {
    if (!Number.isFinite(value)) return 380;
    return Math.min(maxInspectorWidth(), Math.max(MIN_INSPECTOR_WIDTH, Math.round(value)));
  }

  /** 拖拽左侧分隔线：`delta` 为正表示变宽。 */
  function resizeTrack(delta: number): void {
    trackWidth.value = clampTrackWidth(trackWidth.value + delta);
    // 左侧变宽会挤压右侧可用空间，顺手把右侧也夹一下。
    inspectorWidth.value = clampInspectorWidth(inspectorWidth.value);
  }

  /** 拖拽右侧分隔线：`delta` 为正表示变宽。 */
  function resizeInspector(delta: number): void {
    inspectorWidth.value = clampInspectorWidth(inspectorWidth.value + delta);
  }

  /** 视口变化后重新夹紧两侧宽度：窗口变小不该留下超宽面板。 */
  function clampPanes(): void {
    trackWidth.value = clampTrackWidth(trackWidth.value);
    inspectorWidth.value = clampInspectorWidth(inspectorWidth.value);
  }

  /** 打开（或聚焦）一个面板标签。 */
  function openTab(id: PaneId): void {
    if (!tabs.value.includes(id)) tabs.value.push(id);
    activeTab.value = id;
    showInspector.value = true;
  }

  /** 只把标签加进来（不切换当前标签）——「+」选择器用它。 */
  function addTab(id: PaneId): void {
    if (!tabs.value.includes(id)) tabs.value.push(id);
    showInspector.value = true;
  }

  /** 关闭一个标签；关掉当前标签时激活相邻的一个。全部关掉后面板显示"可打开面板"列表。 */
  function closeTab(id: PaneId): void {
    const idx = tabs.value.indexOf(id);
    if (idx < 0) return;
    tabs.value.splice(idx, 1);
    if (tabs.value.length > 0 && activeTab.value === id) {
      activeTab.value = tabs.value[Math.min(idx, tabs.value.length - 1)];
    }
  }

  /** 开关开发者抽屉。 */
  function toggleDevDrawer(): void {
    devDrawer.value = !devDrawer.value;
  }

  /** 弹出一条提示（3.5 秒后自动消失）。 */
  function notify(message: string): void {
    toast.value = message;
    if (toastTimer !== null) window.clearTimeout(toastTimer);
    toastTimer = window.setTimeout(() => {
      toast.value = null;
      toastTimer = null;
    }, 3500);
  }

  return {
    showTrackHeaders,
    showInspector,
    trackWidth,
    inspectorWidth,
    tabs,
    activeTab,
    tool,
    laneHeight,
    showAutomation,
    automationHeight,
    showControllerLane,
    setTool,
    resizeLane,
    resizeAutomation,
    resizeTrack,
    resizeInspector,
    clampPanes,
    maxTrackWidth,
    maxInspectorWidth,
    devDrawer,
    toast,
    openTab,
    addTab,
    closeTab,
    toggleDevDrawer,
    notify,
  };
});
