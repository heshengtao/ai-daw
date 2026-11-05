<script setup lang="ts">
/**
 * 编排视图：网格铺满整宽，片段按轨道配色。
 *
 * 行高与表头高度和左侧音轨栏严格一致，并通过 laneScrollTop 与音轨栏同步纵向滚动。
 * 横向则与编辑视图**共用同一条时间轴**（`stores/timeline`）：同一缩放、同一滚动位置、
 * 同一套由拍号推导的网格，标尺也是同一个组件。
 *
 * 交互（一次拖拽只提交一次，因此只产生一条撤销记录）：
 * - **在空白处左键拖动 → 画出等长的新片段**（松手前是虚线预览；原地点击则只是选中轨道）；
 * - 拖动片段本体 → 移动；多选时整组一起移动；单个片段可以上下换轨道；
 * - 拖动片段右边缘 → 改长度；
 * - 左键点击 → 选中；**单击片段会顺手打开右侧检查器**（方便立刻换音色）；Ctrl/Cmd+点击 → 加选；
 * - **Shift/Alt + 左键拖动 → 框选**（顶栏的鼠标模式也能切换默认左键行为）；
 * - **分隔模式（顶栏 / 快捷键 3）**：左键点击片段 → 在该拍切成两段（razor）；
 * - **右键点击片段 / 空白 → AI 优先的分类菜单**：音频块走翻唱 / 音频处理，MIDI 块走歌声合成 /
 *   旋律翻唱，空白处走「凭空写歌」；编辑与调音台收在二级菜单里；
 * - 双击片段 → 进入它的钢琴卷帘；
 * - Delete/Backspace → 删除所选片段；Ctrl/Cmd + 滚轮 → 缩放。
 */
import { computed, defineAsyncComponent, nextTick, onMounted, onUnmounted, reactive, ref, watch, type Component } from "vue";
import { useRouter } from "vue-router";
import {
  AdjustmentsHorizontalIcon,
  AdjustmentsVerticalIcon,
  ArrowPathIcon,
  ArrowsPointingInIcon,
  ArrowsRightLeftIcon,
  ArrowUpTrayIcon,
  BoltIcon,
  ChartBarIcon,
  ChatBubbleLeftRightIcon,
  ClipboardDocumentIcon,
  ClockIcon,
  DocumentDuplicateIcon,
  DocumentTextIcon,
  EllipsisHorizontalIcon,
  MicrophoneIcon,
  MusicalNoteIcon,
  PlusIcon,
  PuzzlePieceIcon,
  QueueListIcon,
  ScissorsIcon,
  SparklesIcon,
  SpeakerWaveIcon,
  SpeakerXMarkIcon,
  TrashIcon,
  UserIcon,
} from "@heroicons/vue/24/outline";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore, type ClipSel } from "@/stores/selection";
import { useClipboardStore } from "@/stores/clipboard";
import { useTransportStore } from "@/stores/transport";
import { useLayoutStore, type LaneView } from "@/stores/layout";
import { useTimelineStore } from "@/stores/timeline";
import { usePluginsStore } from "@/stores/plugins";
import { useAiIntentStore } from "@/stores/aiIntent";
import TimelineRuler from "@/components/workspace/TimelineRuler.vue";
import ClipWaveform from "@/components/common/ClipWaveform.vue";
import { clipBounds } from "@/api/tauri";
import AutomationLane from "@/components/workspace/AutomationLane.vue";
import MidiKeyboardPane from "@/components/workspace/MidiKeyboardPane.vue";
import LaneLoading from "@/components/workspace/LaneLoading.vue";
import PanelResizer from "@/components/workspace/PanelResizer.vue";
import ContextMenu from "@/components/common/ContextMenu.vue";
import type { ContextMenuItem } from "@/components/common/contextMenu";
import { trackHue } from "@/utils/trackColor";
import { laneScrollTop } from "@/composables/useLaneScroll";
import { capturePointer } from "@/utils/pointer";
import { shouldDeferToBrowser } from "@/utils/editable";
import { isMacOS } from "@/utils/platform";
import { useAiQuickActions } from "@/composables/useAiQuickActions";
import { pickAudioToOpen } from "@/api/dialogs";
import { useI18n } from "@/i18n";
import type { Clip, ClipPlacement, Note, Track } from "@/api/types";

/**
 * 乐谱窗格按需加载：alphaTab 的包一兆多，编排视图首屏不该为它买单——只有用户真的
 * 把下窗口切到「乐谱」时才下载。加载期间用等高占位，避免编排网格高度跳一下。
 */
const ScorePane = defineAsyncComponent({
  loader: () => import("@/components/workspace/ScorePane.vue"),
  loadingComponent: LaneLoading,
  delay: 0,
});

const store = useProjectStore();
const i18n = useI18n();
const selection = useSelectionStore();
const clipboard = useClipboardStore();
const transport = useTransportStore();
const layout = useLayoutStore();
const timeline = useTimelineStore();
const plugins = usePluginsStore();
const aiIntent = useAiIntentStore();
const quickAi = useAiQuickActions();
const router = useRouter();

/** 下窗口视图切换：只接受编排视图能用的三格（视图组件的 `views` 已经限制了，这里再收窄类型）。 */
function setArrangeView(view: LaneView): void {
  if (view === "automation" || view === "score" || view === "keyboard") {
    layout.arrangeLaneView = view;
  }
}

/** 与 TrackHeaderList 约定的行高（px）。 */
const LANE_H = 64;
/** 新片段的最短长度（拍）：低于它视为「原地点击」。 */
const MIN_CREATE_BEATS = 1 / 32;

const scroller = ref<HTMLElement | null>(null);
const lanesEl = ref<HTMLElement | null>(null);
/** 底部自动化块的根元素：实测高度写给左侧音轨栏（见 `layout.arrangeBottomInset`）。 */
const autoBlockEl = ref<HTMLElement | null>(null);
let applyingScroll = false;
let resizeObserver: ResizeObserver | null = null;
let autoObserver: ResizeObserver | null = null;

const tracks = computed(() => store.project?.tracks ?? []);

/**
 * 时间轴在 T=0 **左侧**预留的空白宽度（px）。
 *
 * 不留的话工程起点就贴在滚动区最左边：想点「第 0 拍」只能贴着边线点，很容易点空；留一小段
 * 之后 0 拍永远点得到，视觉上也能看出「这里是工程的开始」。网格、片段、播放头的坐标都以
 * 内容元素的左边缘为原点，所以只有「指针 → 拍」和缩放锚点两处换算需要减掉它。
 */
const TIMELINE_INSET_PX = 26;

/** 把滚动容器的可用宽度回报给共享时间轴：缩得很小时刻度要按它补足，右侧才不会留白。 */
function reportViewportWidth(): void {
  const el = scroller.value;
  if (el) timeline.setViewportWidth(el.clientWidth - TIMELINE_INSET_PX);
  syncBottomInset();
}

/**
 * 量出「左侧音轨栏底部要让出多少」：自动化块 + 编排容器的横向滚动条。
 *
 * 音轨栏的轨道列表必须和编排的**可滚动区域**等高，否则两边滚到底部时行会错位
 * （音轨栏停得更早）。两项都不能写死：
 * - 自动化块 = 分隔线 + 表头 + 曲线画布；曲线没渲染 / 表头换行都会变；
 * - 横向滚动条：`scrollbar-width: thin` 下 10px，覆盖式滚动条（macOS）为 0，
 *   而且只有内容横向溢出时才出现。
 */
function syncBottomInset(): void {
  const el = scroller.value;
  const auto = autoBlockEl.value?.offsetHeight ?? 0;
  const hbar = el ? Math.max(0, el.offsetHeight - el.clientHeight) : 0;
  layout.arrangeBottomInset = auto + hbar;
}

/** 指针 x → 拍（已含横向滚动；落在预留空白里算第 0 拍）。 */
function beatAt(clientX: number): number {
  const el = scroller.value;
  if (!el) return 0;
  const rect = el.getBoundingClientRect();
  return Math.max(0, (clientX - rect.left - TIMELINE_INSET_PX + el.scrollLeft) / timeline.zoom);
}

/**
 * 有符号吸附（拖动增量必须允许负值）。
 *
 * 直接交给共享时间轴：编排视图的吸附网格与钢琴卷帘**是同一份设置**（`layout.noteGrid`），
 * 所以两边的手感永远一致；自由模式下原样返回（不吸附）。
 */
function snapDelta(beat: number): number {
  return timeline.snapDelta(beat);
}

/**
 * 指针 y → lane 下标（跨轨道拖动用）。
 *
 * **不夹取**：可能为负（拖到最上方轨道之外），也可能 >= 轨道数（拖到最下方之外——那里要
 * 新建一条轨道接住）。调用方负责夹到合法范围。
 */
function laneIndexAt(clientY: number): number {
  const el = lanesEl.value;
  if (!el) return 0;
  const rect = el.getBoundingClientRect();
  return Math.floor((clientY - rect.top) / LANE_H);
}

// ---------------------------------------------------------------------------
// 拖拽状态机
// ---------------------------------------------------------------------------

type DragMode = "create" | "move" | "resize" | "marquee" | "erase";

/** 右键菜单的快捷键键帽前缀（与 App 的全局快捷键一致；键帽会自己加 `+`）。 */
const modKey = isMacOS() ? "⌘" : "Ctrl";

/** 内置效果器的行图标（按插件 id 取；外部插件退回拼图图标）。 */
const FX_ICONS: Record<string, Component> = {
  "builtin.delay": ClockIcon,
  "builtin.reverb": SpeakerWaveIcon,
  "builtin.chorus": ArrowsRightLeftIcon,
  "builtin.drive": BoltIcon,
  "builtin.eq": AdjustmentsHorizontalIcon,
  "builtin.compressor": ChartBarIcon,
  "builtin.limiter": ArrowsPointingInIcon,
};

/** 当前右键菜单：位置 + 菜单项 + 选中后的动作。 */
const ctxMenu = ref<{
  x: number;
  y: number;
  items: ContextMenuItem[];
  run: (id: string) => void;
} | null>(null);

function closeMenu(): void {
  ctxMenu.value = null;
}

/** 菜单项被选中：执行当前菜单绑定的动作。 */
function runMenu(id: string): void {
  ctxMenu.value?.run(id);
}

interface MoveDrag {
  originBeat: number;
  /** 拖动前的快照：轨道下标 + 片段 + 起点 / 长度。 */
  base: (ClipSel & { start: number; length: number; trackIndex: number })[];
  /** 横向位移（拍，已吸附）。 */
  delta: number;
  /** 纵向位移（按轨道数）：**整组一起**上下移动；0 = 只横向。 */
  deltaTrack: number;
  /** 指针按下时所在轨道的下标（纵向位移的锚点）。 */
  anchorIndex: number;
  moved: boolean;
}

const mode = ref<DragMode | null>(null);
/** 拖拽创建中的片段（拍）。 */
const create = reactive({ trackId: 0, from: 0, to: 0 });
/** 移动中的片段预览。 */
const moveDrag = ref<MoveDrag | null>(null);
/** 缩放 / 裁剪中的片段预览（左右边缘都可拖）。 */
const resizeDrag = ref<{
  track: number;
  clip: number;
  /** 拖哪条边：右 = 改长度；左 = 改起点（长度随之）。 */
  edge: "left" | "right";
  originBeat: number;
  baseStart: number;
  base: number;
  /** 正在拖的值：右边缘 = 新长度；左边缘 = 新起点。 */
  next: number;
  moved: boolean;
} | null>(null);
/** 框选矩形（客户端坐标，用于 fixed 覆盖层与相交测试）。 */
const marquee = reactive({ x1: 0, y1: 0, x2: 0, y2: 0 });

/**
 * 橡皮拖动：按下时的位置 / 是否真的拖动过 / 命中项。
 *
 * 手势与框选一致：**拖动 = 框选批量删除**；原地点击 = 删除指针下的那一个。
 * 不直接「按下即删」是为了让用户能从一个片段上开始拖出一块区域，而不是先删掉它。
 */
const eraseDrag = ref<{
  x: number;
  y: number;
  moved: boolean;
  target: { track: number; clip: number } | null;
} | null>(null);

const ERASE_DRAG_PX = 4;

/** 开始一次橡皮框选（`target` = 按下的那个片段；空白处为 null）。 */
function startErase(e: PointerEvent, target: { track: number; clip: number } | null): void {
  e.preventDefault();
  if (scroller.value) capturePointer(scroller.value, e.pointerId);
  eraseDrag.value = { x: e.clientX, y: e.clientY, moved: false, target };
  marquee.x1 = marquee.x2 = e.clientX;
  marquee.y1 = marquee.y2 = e.clientY;
  mode.value = "erase";
}

/** 边缘拖拽的夹取范围（拍）——展开不能超出素材本身。`null` = 还没取回来（不夹）。 */
const resizeBounds = ref<{ min: number; max: number } | null>(null);

const marqueeStyle = computed(() => ({
  left: `${Math.min(marquee.x1, marquee.x2)}px`,
  top: `${Math.min(marquee.y1, marquee.y2)}px`,
  width: `${Math.abs(marquee.x2 - marquee.x1)}px`,
  height: `${Math.abs(marquee.y2 - marquee.y1)}px`,
}));

/** 框选矩形与某个元素的客户端矩形是否相交。 */
function intersects(el: Element): boolean {
  const r = el.getBoundingClientRect();
  const left = Math.min(marquee.x1, marquee.x2);
  const right = Math.max(marquee.x1, marquee.x2);
  const top = Math.min(marquee.y1, marquee.y2);
  const bottom = Math.max(marquee.y1, marquee.y2);
  return !(r.right < left || r.left > right || r.bottom < top || r.top > bottom);
}

// ---------------------------------------------------------------------------
// 片段渲染
// ---------------------------------------------------------------------------

/** 渲染用：拖动中的片段使用预览值。 */
function effStart(clip: Clip): number {
  const r = resizeDrag.value;
  if (r && r.edge === "left" && r.clip === clip.id) return r.next;
  const d = moveDrag.value;
  if (d) {
    const item = d.base.find((b) => b.clip === clip.id);
    if (item) return Math.max(0, item.start + d.delta);
  }
  return clip.start;
}

function effLength(clip: Clip): number {
  const r = resizeDrag.value;
  if (r && r.clip === clip.id) {
    // 左拖：起点右移 → 长度缩短（右端钉住）。
    if (r.edge === "left") return Math.max(MIN_CREATE_BEATS, r.base - (r.next - r.baseStart));
    return r.next;
  }
  return clip.length;
}

/**
 * 波形的**内容锚点**（拍）：音频内容在工程时间轴上的起点。
 *
 * - 整体拖动：锚点跟着方框走（音频就是跟着动的，不跟手才对）；
 * - 裁切（拖左右边缘）：锚点**钉在原地**——拖动只改方框，不搬动音频。
 */
function waveAnchor(clip: Clip): number {
  const d = moveDrag.value;
  if (d && d.base.some((b) => b.clip === clip.id)) return effStart(clip);
  return clip.start;
}

/**
 * 该片段是否正被拖到别的轨道上。
 * 拖到别处时要把它从原轨道隐藏，改在目标轨道画一个预览块——否则会看到它「粘」在原处。
 */
function isDraggingAway(clip: Clip, trackIndex: number): boolean {
  const d = moveDrag.value;
  if (!d || d.deltaTrack === 0) return false;
  return d.base.some((b) => b.clip === clip.id && b.trackIndex === trackIndex);
}

/**
 * 跨轨道拖动时各目标行上的预览块（**整组**一起移动：每个片段在自己的目标行画一块）。
 * `index` 是**目标**轨道下标；等于轨道数时表示「下界之外的新轨道」。
 */
const dragGhosts = computed(() => {
  const d = moveDrag.value;
  if (!d || d.deltaTrack === 0) return [];
  return d.base.map((b) => ({
    clip: b.clip,
    index: b.trackIndex + d.deltaTrack,
    start: Math.max(0, b.start + d.delta),
    length: b.length,
  }));
});

/** 落到某一行上的预览块。 */
function ghostsAt(index: number) {
  return dragGhosts.value.filter((g) => g.index === index);
}

/** 落到**下界之外**（需要新建轨道）的预览块。 */
const newTrackGhosts = computed(() => dragGhosts.value.filter((g) => g.index >= tracks.value.length));

function ghostStyleAt(g: { index: number; start: number; length: number }): Record<string, string> {
  return {
    left: `${g.start * timeline.zoom}px`,
    width: `${Math.max(6, g.length * timeline.zoom)}px`,
    "--track-hue": String(trackHue(g.index)),
  };
}

function clipStyle(clip: Clip, index: number): Record<string, string> {
  return {
    left: `${effStart(clip) * timeline.zoom}px`,
    width: `${Math.max(6, effLength(clip) * timeline.zoom)}px`,
    "--track-hue": String(trackHue(index)),
  };
}

/**
 * 片段内音符预览的纵向映射：按**该片段自身的音域**自适应（至少 12 个半音）。
 * 若固定用 36..84，低音线会全挤在底部 15% 里，形状完全看不出来。
 */
function pitchRange(clip: Clip): { low: number; high: number } {
  // 必须用循环求极值，**不要**写回 `Math.min(...pitches)`：音符到十万级时展开实参会抛
  // `RangeError: Maximum call stack size exceeded`。这个函数在 render 期被读（`blipStyle`），
  // 一抛就是 Vue 把整个编排网格换成空节点——表现为「中间所有轨道一起消失」。
  let lowest = 127;
  let highest = 0;
  for (const n of clip.notes) {
    if (n.pitch < lowest) lowest = n.pitch;
    if (n.pitch > highest) highest = n.pitch;
  }
  if (highest < lowest) return { low: 48, high: 60 };
  const lo = lowest - 2;
  const hi = highest + 2;
  const span = Math.max(12, hi - lo);
  const center = (lo + hi) / 2;
  return { low: Math.round(center - span / 2), high: Math.round(center + span / 2) };
}

/** 每个片段的预览音域（按片段 id 缓存，避免每个音符都重算一遍）。 */
const previewRanges = computed(() => {
  const map = new Map<number, { low: number; high: number }>();
  for (const t of tracks.value) for (const c of t.clips) map.set(c.id, pitchRange(c));
  return map;
});

/**
 * 音符方块的**拖动补偿**（拍）。
 *
 * 拖**左边缘**时方框左端在动，但音符本身不该动（那是「收起 / 展开」，不是搬移）——所以给
 * 每个音符加上这个补偿量，让它在屏幕上**钉在原处**。整体拖动 / 右边缘不需要补偿。
 */
function blipShift(clip: Clip): number {
  const r = resizeDrag.value;
  if (r && r.clip === clip.id && r.edge === "left") return clip.start - effStart(clip);
  return 0;
}

/**
 * 片段内的音符预览：**迷你钢琴卷帘**。
 * 横轴 = 时间，纵轴 = 音高。以前这里是一根居中细条，看起来像滚动条——现在不会了。
 */
function blipStyle(n: Note, clip: Clip): Record<string, string> {
  const range = previewRanges.value.get(clip.id) ?? { low: 48, high: 60 };
  const span = Math.max(1, range.high - range.low + 1);
  const pitch = Math.min(range.high, Math.max(range.low, n.pitch));
  // 横轴用**绝对像素**（拍 × 缩放），不是百分比：拖动方框伸缩时音符**原地不动**——
  // 改变只是「这个 MIDI 块的哪一部分会发声」（越出方框的音符由引擎裁掉、不删）。
  const z = timeline.zoom;
  const shift = blipShift(clip);
  return {
    left: `${(n.start + shift) * z}px`,
    width: `${Math.max(1.5, n.duration * z)}px`,
    top: `${((range.high - pitch) / span) * 100}%`,
    height: `${(100 / span) * 4}%`,
  };
}

function isSelected(clip: Clip, trackId: number): boolean {
  return selection.isClipSelected(trackId, clip.id);
}

// ---------------------------------------------------------------------------
// 单击打开检查器（延迟一点点，好让双击有机会取消它）
// ---------------------------------------------------------------------------

let inspectorTimer: number | null = null;

/**
 * 单击片段 → 打开右侧检查器，方便立刻换音色。
 * 故意延后 220ms：双击片段是要进钢琴卷帘的，而打开检查器会让中栏变窄、
 * 第二次点击可能落到别的片段上——所以双击时必须能取消这次打开。
 */
function scheduleOpenInspector(): void {
  if (inspectorTimer !== null) window.clearTimeout(inspectorTimer);
  inspectorTimer = window.setTimeout(() => {
    inspectorTimer = null;
    layout.openTab("inspector");
  }, 220);
}

function cancelOpenInspector(): void {
  if (inspectorTimer !== null) {
    window.clearTimeout(inspectorTimer);
    inspectorTimer = null;
  }
}

// ---------------------------------------------------------------------------
// 指针按下
// ---------------------------------------------------------------------------

/**
 * 在指针所在处切分片段（分隔模式）。
 *
 * 切成两半之后左半段沿用原 id；切在片段之外（含两端）时直接忽略。
 */
async function splitAt(clientX: number, track: Track, clip: Clip): Promise<void> {
  const beat = timeline.snapBeats(beatAt(clientX));
  if (!(beat > clip.start + 1e-6 && beat < clip.start + clip.length - 1e-6)) return;
  try {
    await store.splitClip(track.id, clip.id, beat);
    layout.notify(i18n.t("arrange.clipSplit"));
  } catch (err) {
    layout.notify(i18n.t("error.splitClip", { error: String(err) }));
  }
}

function onClipPointerDown(e: PointerEvent, track: Track, clip: Clip): void {
  if (e.button !== 0) return;
  e.stopPropagation();

  // 橡皮模式：按下先不删，拖动可框选批量删；原地点击松手才删这一个。
  if (layout.tool === "eraser") {
    startErase(e, { track: track.id, clip: clip.id });
    return;
  }

  // 分隔模式：点片段即在该拍切开，不启动拖动。
  if (layout.tool === "split") {
    void splitAt(e.clientX, track, clip);
    return;
  }

  // Ctrl/Cmd 点击：加入/移出选择，不启动拖动。
  if (e.metaKey || e.ctrlKey) {
    selection.toggleClip(track.id, clip.id);
    return;
  }
  // 点未选中的片段 → 先把它设为唯一选择；点已选中的 → 保留整组（便于整组拖动）。
  if (!selection.isClipSelected(track.id, clip.id)) {
    selection.selectClip(track.id, clip.id);
  } else {
    selection.selectTrack(track.id);
    selection.clipId = clip.id;
  }

  capturePointer(e.currentTarget, e.pointerId);
  const all = tracks.value;
  // 每块记下自己的**轨道下标**：多选整组纵向移动时，各自按同一个位移挪（一起上下）。
  const base = (selection.clipRefs.length > 0 ? selection.clipRefs : [{ track: track.id, clip: clip.id }])
    .map((ref) => {
      const trackIndex = all.findIndex((x) => x.id === ref.track);
      const c = all[trackIndex]?.clips.find((x) => x.id === ref.clip);
      return c ? { ...ref, start: c.start, length: c.length, trackIndex } : null;
    })
    .filter(
      (x): x is ClipSel & { start: number; length: number; trackIndex: number } =>
        x !== null && x.trackIndex >= 0,
    );

  moveDrag.value = {
    originBeat: beatAt(e.clientX),
    base,
    delta: 0,
    deltaTrack: 0,
    anchorIndex: Math.max(0, all.findIndex((x) => x.id === track.id)),
    moved: false,
  };
  mode.value = "move";
}

function onResizePointerDown(
  e: PointerEvent,
  track: Track,
  clip: Clip,
  edge: "left" | "right",
): void {
  if (e.button !== 0 || layout.tool === "eraser") return;
  e.stopPropagation();
  selection.selectClip(track.id, clip.id);
  capturePointer(e.currentTarget, e.pointerId);
  resizeDrag.value = {
    track: track.id,
    clip: clip.id,
    edge,
    originBeat: beatAt(e.clientX),
    baseStart: clip.start,
    base: clip.length,
    next: edge === "left" ? clip.start : clip.length,
    moved: false,
  };
  mode.value = "resize";
  // 夹取范围：**展开不能超出素材本身**（与剪映一致）。取回来之前先不夹（几十毫秒）。
  resizeBounds.value = null;
  void clipBounds(track.id, clip.id)
    .then((b) => {
      resizeBounds.value = { min: b.min_start, max: b.max_end };
    })
    .catch(() => {});
}

/** 空白处按下：Shift/Alt 或「框选模式」→ 框选；否则左键拖拽创建片段。 */
function onLanePointerDown(e: PointerEvent, track: Track): void {
  if (e.button !== 0) return;
  // 分隔模式只对片段生效；橡皮模式可以在空白处起手，拖出一块删除区域。
  if (layout.tool === "split") return;
  if (layout.tool === "eraser") {
    if ((e.target as HTMLElement).closest(".clip")) return;
    startErase(e, null);
    return;
  }
  const beat = Math.max(0, snapDelta(beatAt(e.clientX)));

  // 顶栏的鼠标模式决定左键的默认含义；Shift/Alt 随时可以临时切到框选。
  if (e.shiftKey || e.altKey || layout.tool === "select") {
    capturePointer(e.currentTarget, e.pointerId);
    mode.value = "marquee";
    marquee.x1 = marquee.x2 = e.clientX;
    marquee.y1 = marquee.y2 = e.clientY;
    return;
  }

  capturePointer(e.currentTarget, e.pointerId);
  selection.selectTrack(track.id);
  create.trackId = track.id;
  create.from = beat;
  create.to = beat;
  mode.value = "create";
}

// ---------------------------------------------------------------------------
// 右键菜单：AI 优先，按「点到了什么」分菜单
// ---------------------------------------------------------------------------

/**
 * 菜单构造器：每个可点项登记自己的动作，点选时按 id 找回来。
 *
 * 一级菜单只描述结构（`divided` / `children`），二级菜单只在需要时展开；
 * 这样「AI / 编辑 / 调音台」几组不会把一级列表撑成一堵墙。
 */
function menuBuilder() {
  const actions: Record<string, () => void> = {};
  const item = (
    id: string,
    label: string,
    run: () => void,
    extra: Partial<ContextMenuItem> = {},
  ): ContextMenuItem => {
    actions[id] = run;
    return { id, label, ...extra };
  };
  return { actions, item };
}

type MenuItemFactory = (
  id: string,
  label: string,
  run: () => void,
  extra?: Partial<ContextMenuItem>,
) => ContextMenuItem;

/** 把「菜单项 + id → 动作」收成 `ContextMenu` 要的形状。 */
function menuResult(
  items: ContextMenuItem[],
  actions: Record<string, () => void>,
): { items: ContextMenuItem[]; run: (id: string) => void } {
  return { items, run: (id: string) => actions[id]?.() };
}

/** 一条轨道上的第一段有音符的 MIDI 片段。 */
function firstNoteClip(track: Track): Clip | undefined {
  return track.clips.find((c) => !c.audio && c.notes.length > 0);
}

/** 优先用指针所在的轨道；它不合适时退回工程里第一条合适的轨道。 */
function midiTarget(preferred: Track | null): { track: Track; clip: Clip } | null {
  const ordered = preferred
    ? [preferred, ...tracks.value.filter((t) => t.id !== preferred.id)]
    : tracks.value;
  for (const track of ordered) {
    const clip = firstNoteClip(track);
    if (clip) return { track, clip };
  }
  return null;
}

/** 优先用指针所在的轨道；没有音频就退回工程里第一条有音频的轨道。 */
function audioTarget(preferred: Track | null): { track: Track; clip: Clip } | null {
  const ordered = preferred
    ? [preferred, ...tracks.value.filter((t) => t.id !== preferred.id)]
    : tracks.value;
  for (const track of ordered) {
    const clip = track.clips.find((c) => c.audio);
    if (clip) return { track, clip };
  }
  return null;
}

/** 翻唱 / 写歌：把「从哪来」交给 AI 意图总线，子页挂载时自己取。 */
function openYue(mode: "create" | "cover", track?: Track | null, clip?: Clip | null): void {
  if (track) selection.selectTrack(track.id);
  if (track && clip) selection.selectClip(track.id, clip.id);
  aiIntent.open({
    page: "yue",
    mode,
    source: mode === "cover" ? (clip && !clip.audio ? "midi" : "track") : undefined,
    trackId: track?.id ?? null,
    clipId: clip?.id ?? null,
  });
}

function openSinging(track: Track, clip?: Clip | null): void {
  selection.selectTrack(track.id);
  if (clip) selection.selectClip(track.id, clip.id);
  aiIntent.open({ page: "singing", trackId: track.id, clipId: clip?.id ?? null });
}

function openSvc(track: Track, clip?: Clip | null): void {
  selection.selectTrack(track.id);
  if (clip) selection.selectClip(track.id, clip.id);
  aiIntent.open({ page: "svc", trackId: track.id, clipId: clip?.id ?? null });
}

function openInspector(track: Track | null, clip?: Clip | null): void {
  if (track && clip) selection.selectClip(track.id, clip.id);
  else if (track) selection.selectTrack(track.id);
  layout.openTab("inspector");
}

function openMixer(track: Track | null): void {
  if (track) selection.selectTrack(track.id);
  layout.openTab("mixer");
}

// ---- 轨道 / 混音快捷操作 ----

async function toggleTrackMute(track: Track | null): Promise<void> {
  if (!track) return;
  try {
    await store.setMute(track.id, !track.mixer.mute);
  } catch (err) {
    layout.notify(i18n.t("error.toggleMute", { error: String(err) }));
  }
}

async function toggleTrackSolo(track: Track | null): Promise<void> {
  if (!track) return;
  try {
    await store.setSolo(track.id, !track.mixer.solo);
  } catch (err) {
    layout.notify(i18n.t("error.toggleSolo", { error: String(err) }));
  }
}

async function centerTrackPan(track: Track | null): Promise<void> {
  if (!track) return;
  try {
    await store.setPan(track.id, 0);
  } catch (err) {
    layout.notify(i18n.t("error.setPan", { error: String(err) }));
  }
}

async function addTrackEffect(track: Track | null, pluginId: string): Promise<void> {
  if (!track) return;
  try {
    await store.addEffect(track.id, pluginId);
    layout.notify(i18n.t("menu.fxAdded", { name: plugins.name(pluginId) }));
  } catch (err) {
    layout.notify(i18n.t("error.addEffect", { error: String(err) }));
  }
}

/** 「调音台」二级菜单：静音 / 独奏 / 声像 / 打开面板。 */
function buildMixItem(
  item: MenuItemFactory,
  track: Track | null,
  extra: Partial<ContextMenuItem> = {},
): ContextMenuItem {
  return {
    id: "mix",
    label: i18n.t("menu.mixer"),
    icon: AdjustmentsVerticalIcon,
    ...extra,
    children: [
      item(
        "mix.mute",
        track?.mixer.mute ? i18n.t("menu.unmuteTrack") : i18n.t("menu.muteTrack"),
        () => void toggleTrackMute(track),
        {
          icon: track?.mixer.mute ? SpeakerWaveIcon : SpeakerXMarkIcon,
          checked: track?.mixer.mute ?? false,
          disabled: !track,
        },
      ),
      item(
        "mix.solo",
        track?.mixer.solo ? i18n.t("menu.unsoloTrack") : i18n.t("menu.soloTrack"),
        () => void toggleTrackSolo(track),
        { icon: UserIcon, checked: track?.mixer.solo ?? false, disabled: !track },
      ),
      item("mix.center", i18n.t("menu.panCenter"), () => void centerTrackPan(track), {
        icon: ArrowsRightLeftIcon,
        disabled: !track || track.mixer.pan === 0,
      }),
      item("mix.open", i18n.t("menu.openMixer"), () => openMixer(track), {
        icon: AdjustmentsVerticalIcon,
        divided: true,
      }),
    ],
  };
}

/**
 * 「添加效果器」二级菜单：只列内置效果（外部插件数量不可控，去检查器里挑）。
 *
 * 与「调音台」平级而不是套在它里面：三级菜单在窗口右半边会翻到左边、盖住一级菜单，
 * 平级展开更稳，也少一次悬停。
 */
function buildFxItem(
  item: MenuItemFactory,
  track: Track | null,
  extra: Partial<ContextMenuItem> = {},
): ContextMenuItem {
  const builtinFx = plugins.effects.filter((fx) => fx.builtin);
  return {
    id: "fx",
    label: i18n.t("menu.addEffect"),
    icon: AdjustmentsHorizontalIcon,
    ...extra,
    disabled: !track || builtinFx.length === 0,
    children: builtinFx.map((fx) =>
      item(`fx:${fx.id}`, fx.name, () => void addTrackEffect(track, fx.id), {
        icon: FX_ICONS[fx.id] ?? PuzzlePieceIcon,
        disabled: !track,
      }),
    ),
  };
}

// ---- 片段 / 空白菜单 ----

/** 播放头是否落在片段内部（切分只用这一种情形）。 */
function playheadInside(clip: Clip): boolean {
  const beat = transport.positionBeats;
  return beat > clip.start + MIN_CREATE_BEATS && beat < clip.start + clip.length - MIN_CREATE_BEATS;
}

async function splitClipAtPlayhead(track: Track | null, clip: Clip | null): Promise<void> {
  if (!track || !clip || !playheadInside(clip)) return;
  try {
    await store.splitClip(track.id, clip.id, transport.positionBeats);
    layout.notify(i18n.t("arrange.clipSplit"));
  } catch (err) {
    layout.notify(i18n.t("error.splitClip", { error: String(err) }));
  }
}

async function createMidiClipAt(track: Track | null, beat: number): Promise<void> {
  if (!track) return;
  try {
    const length = Math.max(MIN_CREATE_BEATS, selection.lastClipLength);
    await store.addClip(track.id, beat, length);
    const target = store.project?.tracks.find((t) => t.id === track.id);
    const created = target?.clips[target.clips.length - 1];
    if (created) selection.selectClip(track.id, created.id);
    layout.notify(i18n.t("arrange.clipAdded"));
  } catch (err) {
    layout.notify(i18n.t("error.addClip", { error: String(err) }));
  }
}

async function importAudioAt(track: Track | null, beat: number): Promise<void> {
  if (!track) return;
  try {
    const path = await pickAudioToOpen();
    if (!path) return;
    await store.importAudio(path, track.id, beat);
    layout.notify(i18n.t("menu.audioImported"));
  } catch (err) {
    layout.notify(i18n.t("error.importAudio", { error: String(err) }));
  }
}

async function addTrackAtEnd(): Promise<void> {
  const next = (store.project?.tracks.length ?? 0) + 1;
  try {
    await store.addTrack(i18n.t("tracks.newName", { n: next }));
  } catch (err) {
    layout.notify(String(err));
  }
}

/** 多选时把数量带进文案（例如「删除（3）」）。 */
function withCount(label: string, count: number): string {
  return count > 1 ? i18n.t("menu.withCount", { label, count }) : label;
}

/**
 * 「更多编辑」二级菜单：切分与视图跳转。
 *
 * 复制 / 剪切 / 粘贴 / 删除是高频操作，全部留在**一级菜单**，不塞进这里。
 */
function buildMoreEditItem(
  item: MenuItemFactory,
  track: Track,
  clip: Clip,
  midi: boolean,
): ContextMenuItem {
  return {
    id: "more-edit",
    label: i18n.t("menu.moreEdit"),
    icon: EllipsisHorizontalIcon,
    divided: true,
    children: [
      item("split", i18n.t("menu.splitAtPlayhead"), () => void splitClipAtPlayhead(track, clip), {
        icon: ScissorsIcon,
        disabled: !playheadInside(clip),
      }),
      ...(midi
        ? [
            item(
              "piano",
              i18n.t("menu.openPianoRoll"),
              () => {
                selection.selectClip(track.id, clip.id);
                void router.push("/edit");
              },
              { icon: MusicalNoteIcon },
            ),
            item(
              "lyrics",
              i18n.t("menu.editLyrics"),
              () => {
                selection.selectClip(track.id, clip.id);
                layout.setTool("lyric");
                void router.push("/edit");
              },
              { icon: DocumentTextIcon },
            ),
          ]
        : []),
      item("inspector", i18n.t("menu.openInspector"), () => openInspector(track, clip), {
        icon: AdjustmentsHorizontalIcon,
      }),
    ],
  };
}

/**
 * 一级「AI」目录：所有 AI 功能（含打开 AI 面板）都收进这一个子菜单。
 *
 * 根菜单只占一行，避免一堆 AI 动作把右键菜单撑开；目录本身可点开，不是不可点的分组标题。
 */
function buildAiItem(
  children: ContextMenuItem[],
  extra: Partial<ContextMenuItem> = {},
): ContextMenuItem {
  return {
    id: "ai",
    label: i18n.t("menu.ai"),
    icon: SparklesIcon,
    ai: true,
    ...extra,
    children,
  };
}

function buildLaneMenu(track: Track | null, beat: number) {
  const { actions, item } = menuBuilder();
  const midi = midiTarget(track);
  const audio = audioTarget(track);
  const hasTracks = tracks.value.length > 0;
  const hasClipboard = clipboard.payload?.kind === "clip";
  const items: ContextMenuItem[] = [
    buildAiItem([
      item("ai.write", i18n.t("menu.aiWrite"), () => openYue("create", track), {
        icon: SparklesIcon,
        ai: true,
      }),
      item("ai.agent", i18n.t("menu.aiAgent"), () => layout.openTab("agent"), {
        icon: ChatBubbleLeftRightIcon,
        ai: true,
      }),
      item(
        "ai.singing",
        i18n.t("menu.aiSinging"),
        () => midi && openSinging(midi.track, midi.clip),
        { icon: MicrophoneIcon, ai: true, disabled: !midi },
      ),
      item(
        "ai.swap",
        i18n.t("menu.aiVoiceSwap"),
        () => audio && openSvc(audio.track, audio.clip),
        { icon: ArrowsRightLeftIcon, ai: true, disabled: !audio },
      ),
      item("ai.master", i18n.t("menu.aiMaster"), () => void quickAi.run("master", 0), {
        icon: ChartBarIcon,
        ai: true,
        divided: true,
        disabled: !hasTracks,
      }),
      item("open.ai", i18n.t("menu.openAiPanel"), () => layout.openTab("ai"), {
        icon: SparklesIcon,
        ai: true,
        divided: true,
      }),
    ]),
    item("paste", i18n.t("common.paste"), () => void pasteClipAt(track?.id ?? null, beat), {
      icon: ClipboardDocumentIcon,
      keys: [modKey, "V"],
      divided: true,
      disabled: !hasClipboard,
    }),
    item("new.midi", i18n.t("menu.newClip"), () => void createMidiClipAt(track, beat), {
      icon: PlusIcon,
      disabled: !track,
    }),
    item("import.audio", i18n.t("menu.importAudio"), () => void importAudioAt(track, beat), {
      icon: ArrowUpTrayIcon,
      disabled: !track,
    }),
    item("new.track", i18n.t("menu.newTrack"), () => void addTrackAtEnd(), {
      icon: QueueListIcon,
    }),
    buildMixItem(item, track, { divided: true }),
    buildFxItem(item, track),
  ];
  return menuResult(items, actions);
}

function buildAudioClipMenu(
  track: Track,
  clip: Clip,
  targets: { track: number; clip: number }[],
  beat: number,
) {
  const { actions, item } = menuBuilder();
  const single = targets.length === 1;
  const hasClipboard = clipboard.payload?.kind === "clip";
  const items: ContextMenuItem[] = [
    buildAiItem([
      item("ai.cover", i18n.t("menu.aiCover"), () => openYue("cover", track, clip), {
        icon: ArrowPathIcon,
        ai: true,
      }),
      item("ai.swap", i18n.t("menu.aiVoiceSwap"), () => openSvc(track, clip), {
        icon: ArrowsRightLeftIcon,
        ai: true,
      }),
      item(
        "ai.toMidi",
        i18n.t("menu.aiToMidi"),
        () => void quickAi.run("to_midi", track.id, clip.id),
        { icon: DocumentTextIcon, ai: true, divided: true },
      ),
      item(
        "ai.enhance",
        i18n.t("menu.aiEnhance"),
        () => void quickAi.run("enhance", track.id, clip.id),
        { icon: SpeakerWaveIcon, ai: true },
      ),
      item(
        "ai.separate",
        i18n.t("menu.aiSeparate"),
        () => void quickAi.run("separate", track.id, clip.id),
        { icon: QueueListIcon, ai: true },
      ),
      item(
        "ai.tempo",
        i18n.t("menu.aiTempo"),
        () => void quickAi.run("detect_tempo", track.id, clip.id),
        { icon: ClockIcon, ai: true },
      ),
      item(
        "ai.enhanceDsp",
        i18n.t("menu.aiEnhanceDsp"),
        () => void quickAi.run("enhance_dsp", track.id, clip.id),
        { icon: SpeakerWaveIcon, ai: true },
      ),
      item("open.ai", i18n.t("menu.openAiPanel"), () => layout.openTab("ai"), {
        icon: SparklesIcon,
        ai: true,
        divided: true,
      }),
    ]),
    item("copy", withCount(i18n.t("common.copy"), targets.length), () => clipboard.copyClip(clip), {
      icon: DocumentDuplicateIcon,
      keys: [modKey, "C"],
      divided: true,
      disabled: !single,
    }),
    item(
      "cut",
      withCount(i18n.t("common.cut"), targets.length),
      () => {
        clipboard.copyClip(clip);
        void deleteClips(targets);
      },
      { icon: ScissorsIcon, keys: [modKey, "X"], disabled: !single },
    ),
    item("paste", i18n.t("common.paste"), () => void pasteClipAt(track.id, beat), {
      icon: ClipboardDocumentIcon,
      keys: [modKey, "V"],
      disabled: !hasClipboard,
    }),
    item(
      "delete",
      withCount(i18n.t("common.delete"), targets.length),
      () => void deleteClips(targets),
      { icon: TrashIcon, keys: ["Del"], danger: true },
    ),
    buildMoreEditItem(item, track, clip, false),
    buildMixItem(item, track, { divided: true }),
    buildFxItem(item, track),
  ];
  return menuResult(items, actions);
}

function buildMidiClipMenu(
  track: Track,
  clip: Clip,
  targets: { track: number; clip: number }[],
  beat: number,
) {
  const { actions, item } = menuBuilder();
  // 歌声合成 / 旋律翻唱都要有音符可唱：空 MIDI 块只留编辑与删除。
  const hasNotes = clip.notes.length > 0;
  const single = targets.length === 1;
  const hasClipboard = clipboard.payload?.kind === "clip";
  const items: ContextMenuItem[] = [
    buildAiItem([
      item("ai.singing", i18n.t("menu.aiSinging"), () => openSinging(track, clip), {
        icon: MicrophoneIcon,
        ai: true,
        disabled: !hasNotes,
      }),
      item("ai.coverMidi", i18n.t("menu.aiCoverMidi"), () => openYue("cover", track, clip), {
        icon: ArrowPathIcon,
        ai: true,
        disabled: !hasNotes,
      }),
      item("open.ai", i18n.t("menu.openAiPanel"), () => layout.openTab("ai"), {
        icon: SparklesIcon,
        ai: true,
        divided: true,
      }),
    ]),
    item("copy", withCount(i18n.t("common.copy"), targets.length), () => clipboard.copyClip(clip), {
      icon: DocumentDuplicateIcon,
      keys: [modKey, "C"],
      divided: true,
      disabled: !single,
    }),
    item(
      "cut",
      withCount(i18n.t("common.cut"), targets.length),
      () => {
        clipboard.copyClip(clip);
        void deleteClips(targets);
      },
      { icon: ScissorsIcon, keys: [modKey, "X"], disabled: !single },
    ),
    item("paste", i18n.t("common.paste"), () => void pasteClipAt(track.id, beat), {
      icon: ClipboardDocumentIcon,
      keys: [modKey, "V"],
      disabled: !hasClipboard,
    }),
    item(
      "delete",
      withCount(i18n.t("common.delete"), targets.length),
      () => void deleteClips(targets),
      { icon: TrashIcon, keys: ["Del"], danger: true },
    ),
    buildMoreEditItem(item, track, clip, true),
    buildMixItem(item, track, { divided: true }),
    buildFxItem(item, track),
  ];
  return menuResult(items, actions);
}

/** 右键空白处：AI 生成优先，其次才是落片段 / 粘贴 / 调音台。 */
function onLaneContextMenu(e: MouseEvent): void {
  e.preventDefault();
  if ((e.target as HTMLElement).closest(".clip")) return;
  const track = tracks.value[laneIndexAt(e.clientY)] ?? selection.track ?? null;
  const beat = Math.max(0, timeline.snapBeats(beatAt(e.clientX)));
  ctxMenu.value = { x: e.clientX, y: e.clientY, ...buildLaneMenu(track, beat) };
}

/** 右键片段：音频块走翻唱 / 音频处理，MIDI 块走歌声合成 / 旋律翻唱。 */
function onClipContextMenu(e: MouseEvent, track: Track, clip: Clip): void {
  e.preventDefault();
  e.stopPropagation();
  const wasSelected = selection.isClipSelected(track.id, clip.id);
  if (!wasSelected) selection.selectClip(track.id, clip.id);
  const targets = wasSelected
    ? selection.clipRefs.map((r) => ({ track: r.track, clip: r.clip }))
    : [{ track: track.id, clip: clip.id }];
  const beat = Math.max(0, timeline.snapBeats(beatAt(e.clientX)));
  const menu = clip.audio
    ? buildAudioClipMenu(track, clip, targets, beat)
    : buildMidiClipMenu(track, clip, targets, beat);
  ctxMenu.value = { x: e.clientX, y: e.clientY, ...menu };
}

// ---------------------------------------------------------------------------
// 指针移动
// ---------------------------------------------------------------------------

function onPointerMove(e: PointerEvent): void {
  switch (mode.value) {
    case "create": {
      create.to = Math.max(0, snapDelta(beatAt(e.clientX)));
      break;
    }
    case "move": {
      const d = moveDrag.value;
      if (!d) break;
      // 整组一起夹紧：位移的下限由「这一组里最靠左的片段」决定。
      // 否则撞到 0 的那块先停、其余继续走，多选拖完片段之间的相对位置就乱了。
      const minStart = d.base.length > 0 ? Math.min(...d.base.map((b) => b.start)) : 0;
      const delta = Math.max(snapDelta(beatAt(e.clientX) - d.originBeat), -minStart);
      if (delta !== d.delta) {
        d.delta = delta;
        d.moved = true;
      }
      // 纵向：**整组一起**上下移动（多选也能跨轨道）。上不越界（不早于第 0 轨），下最多到
      // 下界之外的那一格——松手时在那里新建一条轨道接住。
      if (d.base.length > 0) {
        const minOrig = Math.min(...d.base.map((b) => b.trackIndex));
        const maxOrig = Math.max(...d.base.map((b) => b.trackIndex));
        const raw = laneIndexAt(e.clientY) - d.anchorIndex;
        const next = Math.min(Math.max(raw, -minOrig), tracks.value.length - maxOrig);
        if (next !== d.deltaTrack) {
          d.deltaTrack = next;
          d.moved = true;
        }
      }
      break;
    }
    case "resize": {
      const r = resizeDrag.value;
      if (!r) break;
      const bounds = resizeBounds.value;
      if (r.edge === "left") {
        // 左边缘：拖的是新起点；吸附后不能越过右端，也不能越过**素材开头**（展开的极限）。
        const raw = snapDelta(r.baseStart + (beatAt(e.clientX) - r.originBeat));
        const minStart = Math.max(0, bounds?.min ?? 0);
        const maxStart = r.baseStart + r.base - MIN_CREATE_BEATS;
        const next = Math.min(Math.max(minStart, raw), maxStart);
        if (next !== r.next) {
          r.next = next;
          r.moved = true;
        }
      } else {
        // 右边缘：拖的是新长度；不能越过**素材末尾**（展开的极限）。
        const raw = Math.max(
          MIN_CREATE_BEATS,
          snapDelta(r.base + (beatAt(e.clientX) - r.originBeat)),
        );
        const maxLen = bounds ? Math.max(MIN_CREATE_BEATS, bounds.max - r.baseStart) : Infinity;
        const next = Math.min(raw, maxLen);
        if (next !== r.next) {
          r.next = next;
          r.moved = true;
        }
      }
      break;
    }
    case "marquee": {
      marquee.x2 = e.clientX;
      marquee.y2 = e.clientY;
      break;
    }
    case "erase": {
      marquee.x2 = e.clientX;
      marquee.y2 = e.clientY;
      const drag = eraseDrag.value;
      if (
        drag &&
        !drag.moved &&
        (Math.abs(e.clientX - drag.x) > ERASE_DRAG_PX ||
          Math.abs(e.clientY - drag.y) > ERASE_DRAG_PX)
      ) {
        drag.moved = true;
      }
      break;
    }
    default:
      break;
  }
}

// ---------------------------------------------------------------------------
// 指针松开
// ---------------------------------------------------------------------------

async function onPointerUp(): Promise<void> {
  const current = mode.value;
  mode.value = null;

  if (current === "create") {
    // 原地点击 = 用「上一次选中的长度」快速建一个；拖动画出的则按拖动长度。
    const clicked = Math.abs(create.to - create.from) < MIN_CREATE_BEATS;
    const start = clicked ? create.from : Math.min(create.from, create.to);
    const length = clicked ? selection.lastClipLength : Math.abs(create.to - create.from);
    try {
      await store.addClip(create.trackId, start, length);
      const t = store.project?.tracks.find((x) => x.id === create.trackId);
      const created = t?.clips[t.clips.length - 1];
      if (created) {
        selection.selectClip(create.trackId, created.id);
        selection.rememberClipLength(created.length);
      }
      layout.notify(i18n.t("arrange.clipAdded"));
    } catch (err) {
      layout.notify(i18n.t("error.addClip", { error: String(err) }));
    }
    return;
  }

  if (current === "move") {
    const d = moveDrag.value;
    if (!d) return;
    // 没动过 = 一次单击：顺手打开检查器（方便换音色）。
    if (!d.moved) {
      moveDrag.value = null;
      scheduleOpenInspector();
      return;
    }
    const all = tracks.value;
    try {
      // 只横向、单个片段 → 走更轻的命令（不改轨道归属）。
      if (d.deltaTrack === 0 && d.base.length === 1) {
        const only = d.base[0]!;
        await store.moveClip(only.track, only.clip, Math.max(0, only.start + d.delta));
        return;
      }
      // 目标落到下界之外 → 先新建一条轨道接住（整组最多只多出一条）。
      const needsNewTrack = d.base.some((b) => b.trackIndex + d.deltaTrack >= all.length);
      if (needsNewTrack) await store.addTrack(i18n.t("tracks.newName", { n: all.length + 1 }));
      const after = store.project?.tracks ?? all;
      const items: ClipPlacement[] = d.base.map((b) => ({
        track: b.track,
        clip: b.clip,
        // 新建的轨道追加在末尾，所以原有下标不变；越界的那块落到最后一条（就是新轨道）。
        to_track: after[Math.min(b.trackIndex + d.deltaTrack, after.length - 1)]?.id ?? b.track,
        start: Math.max(0, b.start + d.delta),
      }));
      await store.moveClips(items);
    } catch (err) {
      layout.notify(i18n.t("error.moveClip", { error: String(err) }));
    } finally {
      // 同 resize：预览留到命令回来，方框 / 波形才不会先弹回原位再跳过去。
      moveDrag.value = null;
    }
    return;
  }

  if (current === "resize") {
    const r = resizeDrag.value;
    if (!r || !r.moved) {
      resizeDrag.value = null;
      return;
    }
    const start = r.edge === "left" ? r.next : r.baseStart;
    const length =
      r.edge === "left"
        ? Math.max(MIN_CREATE_BEATS, r.base - (r.next - r.baseStart))
        : r.next;
    try {
      // 边缘拖动 = **裁剪**（音频改读写窗口，不改速度 / 音高）。
      // 预览要等命令回来再清：否则方框会先弹回旧值、再跳到新值（看着像「反复变化 / 回弹」）。
      await store.trimClip(r.track, r.clip, start, length);
    } catch (err) {
      layout.notify(i18n.t("error.resizeClip", { error: String(err) }));
    } finally {
      resizeDrag.value = null;
    }
    return;
  }

  if (current === "erase") {
    const drag = eraseDrag.value;
    eraseDrag.value = null;
    if (!drag) return;
    // 原地点击：只删指针下的那一个；拖动过：删掉框内所有片段。
    if (!drag.moved) {
      if (drag.target) await deleteClips([drag.target]);
      return;
    }
    const items: { track: number; clip: number }[] = [];
    lanesEl.value?.querySelectorAll<HTMLElement>(".clip").forEach((el) => {
      if (!intersects(el)) return;
      const track = Number(el.dataset.track);
      const clip = Number(el.dataset.clip);
      if (Number.isFinite(track) && Number.isFinite(clip)) items.push({ track, clip });
    });
    await deleteClips(items);
    return;
  }

  if (current === "marquee") {
    const refs: ClipSel[] = [];
    lanesEl.value?.querySelectorAll<HTMLElement>(".clip").forEach((el) => {
      if (!intersects(el)) return;
      const track = Number(el.dataset.track);
      const clip = Number(el.dataset.clip);
      if (Number.isFinite(track) && Number.isFinite(clip)) refs.push({ track, clip });
    });
    selection.setClipSelection(refs);
  }
}

async function deleteClips(items: { track: number; clip: number }[]): Promise<void> {
  if (items.length === 0) return;
  try {
    await store.removeClips(items);
    layout.notify(
      items.length === 1
        ? i18n.t("arrange.clipRemoved")
        : i18n.t("arrange.clipsRemoved", { count: items.length }),
    );
  } catch (err) {
    layout.notify(i18n.t("error.removeClip", { error: String(err) }));
  }
}

// ---------------------------------------------------------------------------
// 其它
// ---------------------------------------------------------------------------

// 记住当前选中片段的长度：单击空白新建片段时复用它。
watch(
  () => selection.clip,
  (c) => {
    if (c) selection.rememberClipLength(c.length);
  },
  { immediate: true },
);

/** 双击片段：进入它的钢琴卷帘（并取消那次「单击开检查器」）。 */
function onClipDoubleClick(track: Track, clip: Clip): void {
  cancelOpenInspector();
  selection.selectClip(track.id, clip.id);
  // 钢琴卷帘只认 MIDI：音频片段双击不进编辑视图（音频编辑器留后续）。
  if (clip.audio) return;
  void router.push("/edit");
}

/** 粘贴：`beat` 省略时副本落在该轨道最后一个片段之后（右键菜单则落在指针处）。 */
async function pasteClip(): Promise<void> {
  const ref = selection.clipRefs;
  await pasteClipAt(ref.length > 0 ? ref[0]!.track : selection.trackId, null);
}

async function pasteClipAt(trackId: number | null, beat: number | null): Promise<void> {
  const p = clipboard.payload;
  if (!p || p.kind !== "clip") return;
  const target = trackId ?? selection.clipRefs[0]?.track ?? selection.trackId;
  if (target === null) return;
  const t = tracks.value.find((x) => x.id === target);
  const start =
    beat === null
      ? (t?.clips ?? []).reduce((m, c) => Math.max(m, c.start + c.length), 0)
      : Math.max(0, beat);
  try {
    // 音频片段：内容在 `audio` 上、`notes` 是空的。必须按音频建块，否则粘出来是个等长的空块。
    if (p.audio) {
      await store.addAudioClip(target, start, p.length, p.audio.path, p.audio.offset_frames, p.audio.frames);
      const track = store.project?.tracks.find((x) => x.id === target);
      const created = track?.clips[track.clips.length - 1];
      if (created) selection.selectClip(target, created.id);
      return;
    }
    await store.addClip(target, start, p.length);
    const track = store.project?.tracks.find((x) => x.id === target);
    const created = track?.clips[track.clips.length - 1];
    if (!created) return;
    for (const n of p.notes) await store.addNote(target, created.id, { ...n });
    selection.selectClip(target, created.id);
  } catch (err) {
    layout.notify(i18n.t("error.addClip", { error: String(err) }));
  }
}

/** Delete / Backspace 删除所选片段；Ctrl/Cmd+C / X / V 复制 / 剪切 / 粘贴。 */
function onKey(e: KeyboardEvent): void {
  // 焦点在输入框/文本域里，或用户选了一段文字：这些按键归浏览器。
  // 少了这一关，只要有一条片段被选中，用户选中的文本一按 Cmd+C 就被当成「复制片段」吞掉。
  if (shouldDeferToBrowser(e)) return;
  const mod = e.metaKey || e.ctrlKey;
  if (mod && e.key.toLowerCase() === "c") {
    const ref = selection.clipRefs;
    if (ref.length !== 1) return;
    const only = ref[0]!;
    const t = tracks.value.find((x) => x.id === only.track);
    const c = t?.clips.find((x) => x.id === only.clip);
    if (c) {
      e.preventDefault();
      clipboard.copyClip(c);
    }
    return;
  }
  if (mod && e.key.toLowerCase() === "x") {
    const ref = selection.clipRefs;
    if (ref.length !== 1) return;
    const only = ref[0]!;
    const t = tracks.value.find((x) => x.id === only.track);
    const c = t?.clips.find((x) => x.id === only.clip);
    if (!c) return;
    e.preventDefault();
    clipboard.copyClip(c);
    void deleteClips([{ track: only.track, clip: only.clip }]);
    return;
  }
  if (mod && e.key.toLowerCase() === "v") {
    // 没有可粘贴的片段就别吞按键——否则在别处（如设置页文本域）粘贴会失效。
    if (clipboard.payload?.kind !== "clip") return;
    e.preventDefault();
    void pasteClip();
    return;
  }
  if (e.key !== "Delete" && e.key !== "Backspace") return;
  const items = selection.clipRefs;
  if (items.length === 0) return;
  e.preventDefault();
  void deleteClips(items.map((r) => ({ track: r.track, clip: r.clip })));
}

const playheadPx = computed(() => transport.positionBeats * timeline.zoom);

/** 播放时让播放头保持在可视范围内。 */
watch(
  () => transport.positionBeats,
  (beats) => {
    if (!transport.playing) return;
    const el = scroller.value;
    if (!el) return;
    const x = beats * timeline.zoom;
    if (x < el.scrollLeft + 40 || x > el.scrollLeft + el.clientWidth - 80) {
      timeline.scrollLeft = Math.max(0, x - el.clientWidth * 0.25);
    }
  },
);

/** 自己滚动：纵向广播给音轨栏，横向写回共享时间轴。 */
function onScroll(): void {
  const el = scroller.value;
  if (!el) return;
  if (!applyingScroll && Math.abs(timeline.scrollLeft - el.scrollLeft) >= 1) {
    timeline.scrollLeft = el.scrollLeft;
  }
  laneScrollTop.value = el.scrollTop;
}

// 共享时间轴的横向位置变了（另一个视图滚的、或播放头跟随）→ 跟随。
watch(
  () => timeline.scrollLeft,
  (v) => {
    const el = scroller.value;
    if (!el || Math.abs(el.scrollLeft - v) < 1) return;
    applyingScroll = true;
    el.scrollLeft = v;
    requestAnimationFrame(() => {
      applyingScroll = false;
    });
  },
);

watch(laneScrollTop, (v) => {
  const el = scroller.value;
  if (!el) return;
  // 编排（右侧）是纵向滚动的**权威**：音轨栏的底部让位高度是异步实测的（挂载、
  // 拖动分隔线、滚动条出现时都会晚一拍），请求越界时把位置夹回并回写，避免两边短暂错位。
  const max = Math.max(0, el.scrollHeight - el.clientHeight);
  const target = Math.min(Math.max(0, v), max);
  if (Math.abs(el.scrollTop - target) >= 1) el.scrollTop = target;
  if (Math.abs(target - v) >= 1) laneScrollTop.value = target;
});

/** Ctrl/Cmd + 滚轮缩放：锚点在指针下，两个视图同步。 */
function onWheel(e: WheelEvent): void {
  if (!(e.ctrlKey || e.metaKey)) return;
  e.preventDefault();
  const el = scroller.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  el.scrollLeft = timeline.zoomAt(
    e.clientX - rect.left,
    e.deltaY < 0 ? 1.15 : 1 / 1.15,
    TIMELINE_INSET_PX,
  );
}

/**
 * 实测下窗口的高度（分隔线 + 表头 + 窗格本体），写给左侧音轨栏。
 *
 * 音轨栏的列表必须和编排的**可滚动区域**一样高，两边滚到底部时行才不会错位。
 * 下窗口的高度不是常数：表头固定 30px，但本体随 `laneHeight` 变，没有可选目标 / 没选
 * 轨道时还会只剩占位文字，所以用 `ResizeObserver` 量，而不是拿常量估。
 */
function watchAutoBlock(): void {
  autoObserver?.disconnect();
  autoObserver = null;
  if (!layout.showAutomation) {
    // 自动化块被收起：下一位让位只剩横向滚动条，等 DOM 更新后重量一次。
    void nextTick(syncBottomInset);
    return;
  }
  void nextTick(() => {
    const el = autoBlockEl.value;
    if (!el) {
      syncBottomInset();
      return;
    }
    autoObserver = new ResizeObserver(syncBottomInset);
    autoObserver.observe(el);
    syncBottomInset();
  });
}

watch(() => layout.showAutomation, watchAutoBlock);

onMounted(() => {
  window.addEventListener("pointerup", onPointerUp);
  window.addEventListener("pointercancel", onPointerUp);
  window.addEventListener("keydown", onKey);
  // 可视区变宽 / 变窄（窗口缩放、面板拖动）都要重新补足刻度。
  resizeObserver = new ResizeObserver(reportViewportWidth);
  if (scroller.value) resizeObserver.observe(scroller.value);
  // 横向滚动条出现 / 消失只会改变 `clientHeight`，外层尺寸不变；顺带看住内容宽度变化。
  if (lanesEl.value) resizeObserver.observe(lanesEl.value);
  watchAutoBlock();
  // 进视图时对齐共享时间轴：横向沿用上一个视图的视角，纵向沿用音轨栏当前的位置
  // （从编辑视图切回来时，音轨栏可能被单独滚过；两边必须从同一位置开始）。
  requestAnimationFrame(() => {
    const el = scroller.value;
    if (el) {
      el.scrollLeft = timeline.scrollLeft;
      const max = Math.max(0, el.scrollHeight - el.clientHeight);
      const top = Math.min(Math.max(0, laneScrollTop.value), max);
      if (Math.abs(el.scrollTop - top) >= 1) el.scrollTop = top;
      if (Math.abs(laneScrollTop.value - top) >= 1) laneScrollTop.value = top;
    }
    reportViewportWidth();
  });
});
onUnmounted(() => {
  window.removeEventListener("pointerup", onPointerUp);
  window.removeEventListener("pointercancel", onPointerUp);
  window.removeEventListener("keydown", onKey);
  resizeObserver?.disconnect();
  resizeObserver = null;
  autoObserver?.disconnect();
  autoObserver = null;
  // 离开编排视图后音轨栏不再需要给底部让位（编辑视图没有自动化车道 / 横向滚动条）。
  layout.arrangeBottomInset = 0;
  cancelOpenInspector();
});
onUnmounted(() => {
  window.removeEventListener("pointerup", onPointerUp);
  window.removeEventListener("pointercancel", onPointerUp);
  window.removeEventListener("keydown", onKey);
  resizeObserver?.disconnect();
  resizeObserver = null;
  cancelOpenInspector();
});
</script>

<template>
  <div class="arrange-root">
    <div
      ref="scroller"
      class="arrange"
      :style="{ paddingLeft: `${TIMELINE_INSET_PX}px`, '--timeline-inset': `${TIMELINE_INSET_PX}px` }"
      @scroll="onScroll"
      @pointermove="onPointerMove"
      @wheel="onWheel"
    >
      <!-- 标尺：与编辑视图同一个组件、同一条时间轴 -->
      <div class="ruler-wrap" :style="timeline.widthStyle">
        <TimelineRuler />
      </div>

      <div
        ref="lanesEl"
        class="lanes"
        :class="{ 'eraser-mode': layout.tool === 'eraser' }"
        :style="timeline.widthStyle"
        @contextmenu="onLaneContextMenu"
      >
        <div
          v-for="(track, ti) in tracks"
          :key="track.id"
          class="lane"
          :class="{ selected: selection.trackId === track.id }"
          :style="{ height: `${LANE_H}px`, ...timeline.gridStyle }"
          @pointerdown="onLanePointerDown($event, track)"
        >
          <!-- 左侧留白里的轨道色竖线：点它即选中这条音轨 -->
          <!-- @pointerdown.stop：拦住冒泡，避免留白上的按下触发 lane 的拖拽建片段/框选删除 -->
          <button
            class="lane-accent"
            type="button"
            :style="{ '--track-hue': String(trackHue(ti)) }"
            :aria-label="`${i18n.t('arrange.selectTrack')} · ${track.name}`"
            :aria-pressed="selection.trackId === track.id"
            @pointerdown.stop
            @click.stop="selection.selectTrack(track.id)"
          />

          <!-- 拖拽创建的预览 -->
          <div
            v-if="mode === 'create' && create.trackId === track.id"
            class="clip-ghost"
            :style="{
              left: `${Math.min(create.from, create.to) * timeline.zoom}px`,
              width: `${Math.max(2, Math.abs(create.to - create.from) * timeline.zoom)}px`,
              '--track-hue': String(trackHue(ti)),
            }"
          />

          <!-- 跨轨道拖动的落点预览（整组：落到本行的每一块都画） -->
          <div
            v-for="g in ghostsAt(ti)"
            :key="g.clip"
            class="clip-ghost"
            :style="ghostStyleAt(g)"
          />

          <template v-for="clip in track.clips" :key="clip.id">
            <div
              v-if="!isDraggingAway(clip, ti)"
              class="clip"
              :class="{
                dragging: moveDrag?.base.some((b) => b.clip === clip.id),
                selected: isSelected(clip, track.id),
                'split-cursor': layout.tool === 'split',
                'erase-cursor': layout.tool === 'eraser',
              }"
              :style="clipStyle(clip, ti)"
              :data-track="track.id"
              :data-clip="clip.id"
              :title="
                i18n.t('arrange.clipTitle', {
                  start: effStart(clip).toFixed(2),
                  length: effLength(clip).toFixed(2),
                })
              "
              @pointerdown="onClipPointerDown($event, track, clip)"
              @dblclick.stop="onClipDoubleClick(track, clip)"
              @contextmenu="onClipContextMenu($event, track, clip)"
            >
              <!-- 片段内容：音频片段画波形，MIDI 片段画迷你钢琴卷帘（横轴时间、纵轴音高） -->
              <ClipWaveform
                v-if="clip.audio"
                class="clip-wave"
                :path="clip.audio.path"
                :offset-frames="clip.audio.offset_frames"
                :rate="clip.audio.rate ?? 1"
                :source-start="waveAnchor(clip)"
                :view-start="effStart(clip)"
                :view-length="effLength(clip)"
              />
              <template v-else>
                <span
                  v-for="(n, i) in clip.notes"
                  :key="i"
                  class="note-blip"
                  :style="blipStyle(n, clip)"
                />
              </template>
              <span
                class="clip-handle left"
                aria-hidden="true"
                @pointerdown="onResizePointerDown($event, track, clip, 'left')"
              />
              <span
                class="clip-handle"
                aria-hidden="true"
                @pointerdown="onResizePointerDown($event, track, clip, 'right')"
              />
            </div>
          </template>
        </div>

        <!-- 拖到下界之外：预览「即将新建的轨道」（松手后就在这一行落下） -->
        <div
          v-if="newTrackGhosts.length > 0"
          class="lane phantom"
          :style="{ height: `${LANE_H}px` }"
        >
          <span class="phantom-label">{{ i18n.t("arrange.newTrack") }}</span>
          <div v-for="g in newTrackGhosts" :key="g.clip" class="clip-ghost" :style="ghostStyleAt(g)" />
        </div>

        <div v-if="tracks.length === 0" class="lane-empty text-body-md">
          还没有轨道——在左侧点「新增轨道」，然后在网格上拖动即可画出片段
        </div>

        <div class="playhead" :style="{ left: `${playheadPx}px` }" />
      </div>

      <!-- 框选 / 橡皮框选：客户端坐标的固定覆盖层 -->
      <div
        v-if="mode === 'marquee' || mode === 'erase'"
        class="marquee"
        :class="{ danger: mode === 'erase' }"
        :style="marqueeStyle"
      />
    </div>

    <!-- 下窗口：自动化 / 选中轨道的乐谱 / 虚拟 MIDI 键盘，三选一（共用同一份高度与分隔线） -->
    <div v-if="layout.showAutomation" ref="autoBlockEl" class="auto-block">
      <PanelResizer side="bottom" @resize="layout.resizeLane($event)" />
      <ScorePane
        v-if="layout.arrangeLaneView === 'score'"
        context="arrange"
        @update:view="setArrangeView"
      />
      <MidiKeyboardPane
        v-else-if="layout.arrangeLaneView === 'keyboard'"
        context="arrange"
        @update:view="setArrangeView"
      />
      <AutomationLane
        v-else
        :height="layout.laneHeight"
        :inset-px="TIMELINE_INSET_PX"
        @update:view="setArrangeView"
      />
    </div>

    <!-- 右键菜单：AI 优先，按音频块 / MIDI 块 / 空白处分组 -->
    <ContextMenu
      :open="ctxMenu !== null"
      :x="ctxMenu?.x ?? 0"
      :y="ctxMenu?.y ?? 0"
      :items="ctxMenu?.items ?? []"
      @select="runMenu($event)"
      @close="closeMenu"
    />
  </div>
</template>

<style scoped>
/* 编排视图的纵向容器：上面是可滚动的网格，下面是（可选的）自动化曲线编辑器。 */
.arrange-root {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background-color: transparent;
}
.arrange {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: auto;
  background-color: transparent;
  scrollbar-width: thin;
  /* 拖拽/框选期间不允许浏览器把空白处当成文本选中（否则会出现一条蓝色高亮） */
  user-select: none;
  -webkit-user-select: none;
}
/* 底部自动化块：分隔线 + 自动化车道，整体高度由 ResizeObserver 量给左侧音轨栏 */
.auto-block {
  flex: 0 0 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
/* 标尺外层：吸附在顶部，宽度与网格一致 */
.ruler-wrap {
  position: sticky;
  top: 0;
  z-index: 3;
  flex: 0 0 auto;
}
.lanes {
  position: relative;
  flex: 1 0 auto;
  display: flex;
  flex-direction: column;
}
.lane {
  position: relative;
  flex: 0 0 auto;
  border-bottom: 1px solid var(--grid-line-row);
  cursor: crosshair;
}
/* 橡皮模式：空白处也能拖出删除框 */
.lanes.eraser-mode .lane {
  cursor: cell;
}
.lane.selected {
  background-color: rgb(var(--md-primary) / 0.05);
}
/* 「即将新建的轨道」行：拖到下界之外时出现，虚线示意它还没真的存在 */
.lane.phantom {
  cursor: default;
  border-bottom: 1px dashed rgb(var(--md-primary) / 0.5);
  background-color: rgb(var(--md-primary) / 0.04);
}
.phantom-label {
  position: absolute;
  left: 12px;
  top: 50%;
  transform: translateY(-50%);
  font-size: 11.5px;
  color: rgb(var(--md-primary));
  pointer-events: none;
}
/* 左侧留白里的轨道色竖线：与左栏 .grip 同色，留白不再突兀 */
.lane-accent {
  position: absolute;
  top: 0;
  bottom: 0;
  left: calc(-1 * var(--timeline-inset, 26px));
  width: var(--timeline-inset, 26px);
  padding: 0;
  border: 0;
  cursor: pointer;
  background-color: hsl(var(--track-hue) 55% 55% / 0.1);
}
.lane-accent:hover {
  background-color: hsl(var(--track-hue) 55% 55% / 0.18);
}
.lane-accent:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: -2px;
}
.lane-accent::before {
  content: "";
  position: absolute;
  top: 0;
  bottom: 0;
  left: 0;
  width: 3px;
  border-radius: 0 3px 3px 0;
  background-color: hsl(var(--track-hue) 55% 55%);
}
/* 拖拽创建预览：虚线框 + 轨道色淡填充 */
.clip-ghost {
  position: absolute;
  top: 8px;
  bottom: 8px;
  border-radius: 12px;
  border: 2px dashed hsl(var(--track-hue) var(--track-clip-edge-s) var(--track-clip-edge-l));
  background-color: hsl(var(--track-hue) var(--track-ghost-s) var(--track-ghost-l) / 0.22);
  pointer-events: none;
}
.clip {
  position: absolute;
  top: 8px;
  bottom: 8px;
  border-radius: 12px;
  background-color: hsl(var(--track-hue) var(--track-clip-bg-s) var(--track-clip-bg-l));
  border: 1px solid hsl(var(--track-hue) var(--track-clip-edge-s) var(--track-clip-edge-l) / 0.9);
  box-shadow: var(--md-elevation-1);
  overflow: hidden;
  cursor: grab;
  touch-action: none;
}
.clip.dragging {
  cursor: grabbing;
  opacity: 0.9;
}
/* 分隔模式：光标提示「这一下是切开」 */
.clip.split-cursor {
  cursor: col-resize;
}
/* 橡皮模式：光标提示「这一下是删除」 */
.clip.erase-cursor {
  cursor: cell;
}
.clip.selected {
  border-color: rgb(var(--md-primary));
  box-shadow: 0 0 0 2px rgb(var(--md-primary) / 0.75);
}
/* 片段里的音符：小方块，位置对应音高、长度对应时长（迷你钢琴卷帘） */
.note-blip {
  position: absolute;
  min-height: 3px;
  border-radius: 2px;
  background-color: hsl(var(--track-hue) var(--track-blip-s) var(--track-blip-l));
  box-shadow: 0 0 0 1px hsl(var(--track-hue) var(--track-blip-line-s) var(--track-blip-line-l) / 0.5);
  pointer-events: none;
}
/* 音频片段的波形层：铺满片段盒子，不抢指针事件（拖动/缩放仍归 .clip）。 */
.clip-wave {
  position: absolute;
  inset: 0;
  --wave-color: hsl(var(--track-hue) var(--track-wave-s) var(--track-wave-l));
  pointer-events: none;
}
/* 右边缘的缩放热区：命中区 10px，视觉上只有一条细亮边 */
.clip-handle {
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  width: 10px;
  cursor: ew-resize;
  border-right: 2px solid transparent;
}
.clip:hover .clip-handle {
  border-right-color: hsl(var(--track-hue) var(--track-handle-s) var(--track-handle-l) / 0.8);
}
/* 左边缘的裁剪热区（左右对称）。 */
.clip-handle.left {
  left: 0;
  right: auto;
  border-right: 0;
  border-left: 2px solid transparent;
}
.clip:hover .clip-handle.left {
  border-left-color: hsl(var(--track-hue) var(--track-handle-s) var(--track-handle-l) / 0.8);
}
.lane-empty {
  padding: 24px;
  color: rgb(var(--md-on-surface-muted));
}
.playhead {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 2px;
  background-color: rgb(var(--md-primary) / 0.9);
  pointer-events: none;
  z-index: 2;
}
/* 框选矩形 */
.marquee {
  position: fixed;
  z-index: 50;
  border: 1px solid rgb(var(--md-primary));
  background-color: rgb(var(--md-primary) / 0.16);
  pointer-events: none;
}
/* 橡皮框选：红色，一眼看出松手会删东西 */
.marquee.danger {
  border-color: rgb(var(--md-error));
  background-color: rgb(var(--md-error) / 0.16);
}
</style>
