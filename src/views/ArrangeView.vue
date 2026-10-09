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
import { trackHue } from "@/utils/trackColor";
import { laneScrollTop } from "@/composables/useLaneScroll";
import { capturePointer } from "@/utils/pointer";
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

const tracks = computed(() => store.project?.tracks ?? []);

/**
 * 时间轴在 T=0 **左侧**预留的空白宽度（px）。
 *
 * 不留的话工程起点就贴在滚动区最左边：想点「第 0 拍」只能贴着边线点，很容易点空；留一小段
 * 之后 0 拍永远点得到，视觉上也能看出「这里是工程的开始」。网格、片段、播放头的坐标都以
 * 内容元素的左边缘为原点，所以只有「指针 → 拍」和缩放锚点两处换算需要减掉它。
 */
const TIMELINE_INSET_PX = 26;

/** 指针 x → 拍（已含横向滚动；落在预留空白里算第 0 拍）。 */
function beatAt(clientX: number): number {
  const el = scroller.value;
  if (!el) return 0;
  const rect = el.getBoundingClientRect();
  return Math.max(0, (clientX - rect.left - TIMELINE_INSET_PX + el.scrollLeft) / timeline.zoom);
}

/** 有符号吸附（拖动增量必须允许负值）。 */
function snapDelta(beat: number): number {
  const s = timeline.snap;
  return Math.round(beat / s) * s;
}

/** 指针 y → 所在轨道的 id（跨轨道拖动用）；越界返回 null。 */
function trackAt(clientY: number): number | null {
  const el = lanesEl.value;
  if (!el) return null;
  const rect = el.getBoundingClientRect();
  const index = Math.floor((clientY - rect.top) / LANE_H);
  return tracks.value[index]?.id ?? null;
}

// ---------------------------------------------------------------------------
// 拖拽状态机
// ---------------------------------------------------------------------------

type DragMode = "create" | "move" | "resize" | "marquee" | "marquee-delete";

interface MoveDrag {
  originBeat: number;
  /** 拖动前的 (track, clip, start) 快照。 */
  base: (ClipSel & { start: number; length: number })[];
  delta: number;
  /** 目标轨道（单个片段可跨轨道拖动；多选时恒等于原轨道）。 */
  targetTrack: number | null;
  moved: boolean;
}

const mode = ref<DragMode | null>(null);
/** 拖拽创建中的片段（拍）。 */
const create = reactive({ trackId: 0, from: 0, to: 0 });
/** 移动中的片段预览。 */
const moveDrag = ref<MoveDrag | null>(null);
/** 缩放中的片段预览。 */
const resizeDrag = ref<{ track: number; clip: number; originBeat: number; base: number; next: number; moved: boolean } | null>(null);
/** 框选矩形（客户端坐标，用于 fixed 覆盖层与相交测试）。 */
const marquee = reactive({ x1: 0, y1: 0, x2: 0, y2: 0 });

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
  const d = moveDrag.value;
  if (d) {
    const item = d.base.find((b) => b.clip === clip.id);
    if (item) return Math.max(0, item.start + d.delta);
  }
  return clip.start;
}

function effLength(clip: Clip): number {
  const r = resizeDrag.value;
  if (r && r.clip === clip.id) return r.next;
  return clip.length;
}

/**
 * 该片段是否正被拖到别的轨道上。
 * 拖到别处时要把它从原轨道隐藏，改在目标轨道画一个预览块——否则会看到它「粘」在原处。
 */
function isDraggingAway(clip: Clip, trackId: number): boolean {
  const d = moveDrag.value;
  if (!d || d.targetTrack === null || d.targetTrack === trackId) return false;
  return d.base.some((b) => b.clip === clip.id);
}

/** 跨轨道拖动时，目标轨道上显示的预览块（只有单片段拖动才会出现）。 */
const dragGhost = computed(() => {
  const d = moveDrag.value;
  if (!d || d.targetTrack === null || d.base.length !== 1) return null;
  const item = d.base[0];
  if (item.track === d.targetTrack) return null;
  return { track: d.targetTrack, start: Math.max(0, item.start + d.delta), length: item.length };
});

function ghostStyle(trackId: number, index: number): Record<string, string> {
  const g = dragGhost.value;
  if (!g || g.track !== trackId) return {};
  return {
    left: `${g.start * timeline.zoom}px`,
    width: `${Math.max(6, g.length * timeline.zoom)}px`,
    "--track-hue": String(trackHue(index)),
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
  const pitches = clip.notes.map((n) => n.pitch);
  if (pitches.length === 0) return { low: 48, high: 60 };
  const lo = Math.min(...pitches) - 2;
  const hi = Math.max(...pitches) + 2;
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
 * 片段内的音符预览：**迷你钢琴卷帘**。
 * 横轴 = 时间，纵轴 = 音高。以前这里是一根居中细条，看起来像滚动条——现在不会了。
 */
function blipStyle(n: Note, clip: Clip): Record<string, string> {
  const range = previewRanges.value.get(clip.id) ?? { low: 48, high: 60 };
  const span = Math.max(1, range.high - range.low + 1);
  const pitch = Math.min(range.high, Math.max(range.low, n.pitch));
  return {
    left: `${(n.start / clip.length) * 100}%`,
    width: `${Math.max(1.5, (n.duration / clip.length) * 100)}%`,
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

function onClipPointerDown(e: PointerEvent, track: Track, clip: Clip): void {
  if (e.button !== 0) return;
  e.stopPropagation();

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
  const base = (selection.clipRefs.length > 0 ? selection.clipRefs : [{ track: track.id, clip: clip.id }])
    .map((ref) => {
      const t = tracks.value.find((x) => x.id === ref.track);
      const c = t?.clips.find((x) => x.id === ref.clip);
      return c ? { ...ref, start: c.start, length: c.length } : null;
    })
    .filter((x): x is ClipSel & { start: number; length: number } => x !== null);

  // 跨轨道拖动只对单个片段开放：多选时各自的原轨道不同，「一起挪到哪一条」没有明确答案。
  moveDrag.value = {
    originBeat: beatAt(e.clientX),
    base,
    delta: 0,
    targetTrack: base.length === 1 ? track.id : null,
    moved: false,
  };
  mode.value = "move";
}

function onResizePointerDown(e: PointerEvent, track: Track, clip: Clip): void {
  if (e.button !== 0) return;
  e.stopPropagation();
  selection.selectClip(track.id, clip.id);
  capturePointer(e.currentTarget, e.pointerId);
  resizeDrag.value = {
    track: track.id,
    clip: clip.id,
    originBeat: beatAt(e.clientX),
    base: clip.length,
    next: clip.length,
    moved: false,
  };
  mode.value = "resize";
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
      // 纵向：单个片段可以换轨道（多选时锁死在原轨道）。
      if (d.base.length === 1) {
        const target = trackAt(e.clientY);
        if (target !== null && target !== d.targetTrack) {
          d.targetTrack = target;
          d.moved = true;
        }
      }
      break;
    }
    case "resize": {
      const r = resizeDrag.value;
      if (!r) break;
      const next = Math.max(MIN_CREATE_BEATS, snapDelta(r.base + (beatAt(e.clientX) - r.originBeat)));
      if (next !== r.next) {
        r.next = next;
        r.moved = true;
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
    moveDrag.value = null;
    if (!d) return;
    // 没动过 = 一次单击：顺手打开检查器（方便换音色）。
    if (!d.moved) {
      scheduleOpenInspector();
      return;
    }
    const items: ClipPlacement[] = d.base.map((b) => ({
      track: b.track,
      clip: b.clip,
      to_track: d.targetTrack ?? b.track,
      start: Math.max(0, b.start + d.delta),
    }));
    try {
      if (items.length === 1) {
        const only = items[0];
        // 同轨道只改起点时走更轻的命令，跨轨道才需要搬移。
        if (only.to_track === only.track) await store.moveClip(only.track, only.clip, only.start);
        else await store.moveClips(items);
      } else {
        await store.moveClips(items);
      }
    } catch (err) {
      layout.notify(i18n.t("error.moveClip", { error: String(err) }));
    }
    return;
  }

  if (current === "resize") {
    const r = resizeDrag.value;
    resizeDrag.value = null;
    if (!r || !r.moved) return;
    try {
      await store.resizeClip(r.track, r.clip, r.next);
    } catch (err) {
      layout.notify(i18n.t("error.resizeClip", { error: String(err) }));
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
      await store.addAudioClip(trackId, start, p.length, p.audio.path, p.audio.offset_frames);
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
  if (e.target instanceof HTMLInputElement) return;
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
  // 进视图时对齐共享时间轴的横向位置（从编辑切过来时视角不变）。
  requestAnimationFrame(() => {
    const el = scroller.value;
    if (el) el.scrollLeft = timeline.scrollLeft;
  });
});
onUnmounted(() => {
  window.removeEventListener("pointerup", onPointerUp);
  window.removeEventListener("pointercancel", onPointerUp);
  window.removeEventListener("keydown", onKey);
  cancelOpenInspector();
});
</script>

<template>
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

        <!-- 跨轨道拖动的落点预览 -->
        <div
          v-if="dragGhost && dragGhost.track === track.id"
          class="clip-ghost"
          :style="ghostStyle(track.id, ti)"
        />

        <template v-for="clip in track.clips" :key="clip.id">
          <div
            v-if="!isDraggingAway(clip, track.id)"
            class="clip"
            :class="{
              dragging: moveDrag?.base.some((b) => b.clip === clip.id),
              selected: isSelected(clip, track.id),
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
            <ClipWaveform v-if="clip.audio" class="clip-wave" :path="clip.audio.path" />
            <template v-else>
              <span
                v-for="(n, i) in clip.notes"
                :key="i"
                class="note-blip"
                :style="blipStyle(n, clip)"
              />
            </template>
            <span
              class="clip-handle"
              aria-hidden="true"
              @pointerdown="onResizePointerDown($event, track, clip)"
            />
          </div>
        </template>
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
</template>

<style scoped>
.arrange {
  flex: 1;
  min-width: 0;
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
  border: 2px dashed hsl(var(--track-hue) 60% 62%);
  background-color: hsl(var(--track-hue) 45% 45% / 0.22);
  pointer-events: none;
}
.clip {
  position: absolute;
  top: 8px;
  bottom: 8px;
  border-radius: 12px;
  background-color: hsl(var(--track-hue) 42% 32%);
  border: 1px solid hsl(var(--track-hue) 60% 62% / 0.9);
  box-shadow: var(--md-elevation-1);
  overflow: hidden;
  cursor: grab;
  touch-action: none;
}
.clip.dragging {
  cursor: grabbing;
  opacity: 0.9;
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
  background-color: hsl(var(--track-hue) 72% 88%);
  box-shadow: 0 0 0 1px hsl(var(--track-hue) 40% 22% / 0.5);
  pointer-events: none;
}
/* 音频片段的波形层：铺满片段盒子，不抢指针事件（拖动/缩放仍归 .clip）。 */
.clip-wave {
  position: absolute;
  inset: 0;
  --wave-color: hsl(var(--track-hue) 78% 86%);
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
  border-right-color: hsl(var(--track-hue) 70% 88% / 0.8);
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
