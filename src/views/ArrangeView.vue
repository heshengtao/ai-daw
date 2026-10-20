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
 * - **右键点击 → 删除**；**右键拖动 → 框选批量删除**；
 * - 双击片段 → 进入它的钢琴卷帘；
 * - Delete/Backspace → 删除所选片段；Ctrl/Cmd + 滚轮 → 缩放。
 */
import { computed, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import { useRouter } from "vue-router";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore, type ClipSel } from "@/stores/selection";
import { useClipboardStore } from "@/stores/clipboard";
import { useTransportStore } from "@/stores/transport";
import { useLayoutStore } from "@/stores/layout";
import { useTimelineStore } from "@/stores/timeline";
import TimelineRuler from "@/components/workspace/TimelineRuler.vue";
import ClipWaveform from "@/components/common/ClipWaveform.vue";
import { clipBounds } from "@/api/tauri";
import AutomationLane from "@/components/workspace/AutomationLane.vue";
import PanelResizer from "@/components/workspace/PanelResizer.vue";
import { trackHue } from "@/utils/trackColor";
import { laneScrollTop } from "@/composables/useLaneScroll";
import { capturePointer } from "@/utils/pointer";
import { shouldDeferToBrowser } from "@/utils/editable";
import { useI18n } from "@/i18n";
import type { Clip, ClipPlacement, Note, Track } from "@/api/types";

const store = useProjectStore();
const i18n = useI18n();
const selection = useSelectionStore();
const clipboard = useClipboardStore();
const transport = useTransportStore();
const layout = useLayoutStore();
const timeline = useTimelineStore();
const router = useRouter();

/** 与 TrackHeaderList 约定的行高（px）。 */
const LANE_H = 64;
/** 新片段的最短长度（拍）：低于它视为「原地点击」。 */
const MIN_CREATE_BEATS = 1 / 32;

const scroller = ref<HTMLElement | null>(null);
const lanesEl = ref<HTMLElement | null>(null);
let applyingScroll = false;
let resizeObserver: ResizeObserver | null = null;

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

type DragMode = "create" | "move" | "resize" | "marquee" | "marquee-delete";

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
  if (e.button !== 0) return;
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

/** 空白处按下：右键 → 框选删除；Shift/Alt 或「框选模式」→ 框选；否则左键拖拽创建片段。 */
function onLanePointerDown(e: PointerEvent, track: Track): void {
  if (e.button === 2) {
    // 片段自己处理右键（右键点击删除 / 整组删除），这里只管空白区域。
    if ((e.target as HTMLElement).closest(".clip")) return;
    capturePointer(e.currentTarget, e.pointerId);
    selection.selectTrack(track.id);
    mode.value = "marquee-delete";
    marquee.x1 = marquee.x2 = e.clientX;
    marquee.y1 = marquee.y2 = e.clientY;
    return;
  }
  if (e.button !== 0) return;
  // 分隔模式只对片段生效，空白处按下不做任何事。
  if (layout.tool === "split") return;
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

/** 抑制原生右键菜单（框选删除与右键删除都在自己的处理里）。 */
function onLaneContextMenu(e: MouseEvent): void {
  e.preventDefault();
}

function onClipContextMenu(e: MouseEvent, track: Track, clip: Clip): void {
  e.preventDefault();
  e.stopPropagation();
  // 右键点未选中的片段：只删它；点已选中的：删掉整组。
  const targets = selection.isClipSelected(track.id, clip.id)
    ? selection.clipRefs.map((r) => ({ track: r.track, clip: r.clip }))
    : [{ track: track.id, clip: clip.id }];
  void deleteClips(targets);
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
      const delta = snapDelta(beatAt(e.clientX) - d.originBeat);
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
    case "marquee":
    case "marquee-delete": {
      marquee.x2 = e.clientX;
      marquee.y2 = e.clientY;
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

  if (current === "marquee") {
    const refs: ClipSel[] = [];
    lanesEl.value?.querySelectorAll<HTMLElement>(".clip").forEach((el) => {
      if (!intersects(el)) return;
      const track = Number(el.dataset.track);
      const clip = Number(el.dataset.clip);
      if (Number.isFinite(track) && Number.isFinite(clip)) refs.push({ track, clip });
    });
    selection.setClipSelection(refs);
    return;
  }

  if (current === "marquee-delete") {
    const width = Math.abs(marquee.x2 - marquee.x1);
    const height = Math.abs(marquee.y2 - marquee.y1);
    // 几乎没动 = 一次右键点击：删除指针下的那一个片段（由 contextmenu 处理），这里不做事。
    if (width < 4 && height < 4) return;
    const items: { track: number; clip: number }[] = [];
    lanesEl.value?.querySelectorAll<HTMLElement>(".clip").forEach((el) => {
      if (!intersects(el)) return;
      const track = Number(el.dataset.track);
      const clip = Number(el.dataset.clip);
      if (Number.isFinite(track) && Number.isFinite(clip)) items.push({ track, clip });
    });
    await deleteClips(items);
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

/** 粘贴：副本落在该轨道最后一个片段之后。 */
async function pasteClip(): Promise<void> {
  const p = clipboard.payload;
  if (!p || p.kind !== "clip") return;
  const ref = selection.clipRefs;
  const trackId = ref.length > 0 ? ref[0]!.track : selection.trackId;
  if (trackId === null) return;
  const t = tracks.value.find((x) => x.id === trackId);
  const start = (t?.clips ?? []).reduce((m, c) => Math.max(m, c.start + c.length), 0);
  try {
    // 音频片段：内容在 `audio` 上、`notes` 是空的。必须按音频建块，否则粘出来是个等长的空块。
    if (p.audio) {
      await store.addAudioClip(trackId, start, p.length, p.audio.path, p.audio.offset_frames, p.audio.frames);
      const track = store.project?.tracks.find((x) => x.id === trackId);
      const created = track?.clips[track.clips.length - 1];
      if (created) selection.selectClip(trackId, created.id);
      return;
    }
    await store.addClip(trackId, start, p.length);
    const track = store.project?.tracks.find((x) => x.id === trackId);
    const created = track?.clips[track.clips.length - 1];
    if (!created) return;
    for (const n of p.notes) await store.addNote(trackId, created.id, { ...n });
    selection.selectClip(trackId, created.id);
  } catch (err) {
    layout.notify(i18n.t("error.addClip", { error: String(err) }));
  }
}

/** Delete / Backspace 删除所选片段；Ctrl/Cmd+C / V 复制粘贴。 */
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
  if (!el || Math.abs(el.scrollTop - v) < 1) return;
  el.scrollTop = v;
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

onMounted(() => {
  window.addEventListener("pointerup", onPointerUp);
  window.addEventListener("pointercancel", onPointerUp);
  window.addEventListener("keydown", onKey);
  // 可视区变宽 / 变窄（窗口缩放、面板拖动）都要重新补足刻度。
  resizeObserver = new ResizeObserver(reportViewportWidth);
  if (scroller.value) resizeObserver.observe(scroller.value);
  // 进视图时对齐共享时间轴的横向位置（从编辑切过来时视角不变）。
  requestAnimationFrame(() => {
    const el = scroller.value;
    if (el) el.scrollLeft = timeline.scrollLeft;
    reportViewportWidth();
  });
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

      <div ref="lanesEl" class="lanes" :style="timeline.widthStyle" @contextmenu="onLaneContextMenu">
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

      <!-- 框选矩形：客户端坐标的固定覆盖层 -->
      <div
        v-if="mode === 'marquee' || mode === 'marquee-delete'"
        class="marquee"
        :class="{ danger: mode === 'marquee-delete' }"
        :style="marqueeStyle"
      />
    </div>

    <!-- 自动化曲线编辑器：编排视图底部，与网格共用同一条时间轴（原点 = 同一份留白常量） -->
    <template v-if="layout.showAutomation">
      <PanelResizer side="bottom" @resize="layout.resizeLane($event)" />
      <AutomationLane :height="layout.laneHeight" :inset-px="TIMELINE_INSET_PX" />
    </template>
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
.marquee.danger {
  border-color: rgb(var(--md-error));
  background-color: rgb(var(--md-error) / 0.16);
}
</style>
