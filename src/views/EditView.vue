<script setup lang="ts">
/**
 * 编辑视图：钢琴卷帘 + 下方控制器泳道。
 *
 * 编辑对象是「当前选中的片段」——音轨栏 / 编排视图选中什么，这里就编辑什么。
 *
 * **时间轴与编排视图完全共用**（`stores/timeline`）：同一缩放、同一横向滚动位置、
 * 同一套由拍号推导的网格。所以卷帘里的 x 坐标是**绝对时间**（片段起点偏移在内），
 * 两边的网格线逐像素对齐。顶部还有一条可点击的标尺，点击 / 拖动即定位播放头。
 *
 * 交互（一次拖拽只提交一次，因此只产生一条撤销记录）：
 * - 空白处左键拖动 → 画出等长的新音符（纵向锁定在按下时的音高行）；
 * - 拖动音符本体 → 同时改音高与起点；多选时整组一起移动；
 * - 拖动音符右边缘 → 改时长（音符上**没有**删除按钮，删除走右键或 Delete）；
 * - 左键点击音符 → 选中；Ctrl/Cmd+点击 → 加入 / 移出选择；
 * - Shift/Alt + 左键拖动 → 框选（顶栏的鼠标模式也能切换默认左键行为）；
 * - 右键点击音符 → 删除；右键拖动 → 框选批量删除；
 * - Ctrl/Cmd + 滚轮 → 缩放（锚点在指针下，两个视图同步）。
 */
import { computed, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import { useRouter } from "vue-router";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { useClipboardStore } from "@/stores/clipboard";
import { useLayoutStore } from "@/stores/layout";
import { useTimelineStore } from "@/stores/timeline";
import { useTransportStore } from "@/stores/transport";
import ControllerLane, { type ControllerProperty } from "@/components/workspace/ControllerLane.vue";
import PanelResizer from "@/components/workspace/PanelResizer.vue";
import TimelineRuler from "@/components/workspace/TimelineRuler.vue";
import { capturePointer } from "@/utils/pointer";
import { auditionNote } from "@/api/tauri";
import { useI18n } from "@/i18n";
import type { Note, NoteEdit } from "@/api/types";

const store = useProjectStore();
const i18n = useI18n();
const selection = useSelectionStore();
const clipboard = useClipboardStore();
const layout = useLayoutStore();
const timeline = useTimelineStore();
const transport = useTransportStore();
const router = useRouter();

const LOW = 36; // C2
const HIGH = 84; // C6
const ROW_H = 14;
/** 新音符的最短时长（拍）：低于它视为「原地点击」，只清空选择、不建音符。 */
const MIN_CREATE_BEATS = 1 / 32;
/** 原地点击新建音符时的默认时长（拍）。 */
const CLICK_NOTE_BEATS = 0.5;

const rows = computed(() => {
  const out: number[] = [];
  for (let p = HIGH; p >= LOW; p--) out.push(p);
  return out;
});

function isBlack(p: number): boolean {
  return [1, 3, 6, 8, 10].includes(p % 12);
}

const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
function noteName(p: number): string {
  return `${NOTE_NAMES[p % 12]}${Math.floor(p / 12) - 1}`;
}

const clip = computed(() => selection.clip);
const trackId = computed(() => selection.trackId);
const clipStart = computed(() => clip.value?.start ?? 0);
/** 每小节拍数：按片段起点所在的拍号分段取；新建片段时用它作为默认长度。 */
const beatsPerBar = computed(() => timeline.barBeatsAt(clipStart.value));

const rollHeight = computed(() => `${rows.value.length * ROW_H}px`);
/** 卷帘内容宽度：与编排视图同一套时间轴。 */
const rollWidth = computed(() => timeline.widthStyle.width);
/** 片段在时间轴上占据的范围（在卷帘里高亮出来，避免把音符放到片段之外）。 */
const clipBand = computed(() => {
  const c = clip.value;
  if (!c) return null;
  return {
    left: `${c.start * timeline.zoom}px`,
    width: `${Math.max(2, c.length * timeline.zoom)}px`,
  };
});
/** 网格细分标签（例如 1/16）：按片段起点所在段的吸附步长计算。 */
const gridLabel = computed(() => {
  const snap = timeline.snapAt(clipStart.value);
  return `1/${Math.round(4 / snap)}`;
});

// ---------------------------------------------------------------------------
// 拖拽状态机
// ---------------------------------------------------------------------------

type DragMode = "create" | "move" | "resize" | "marquee" | "marquee-delete";

interface MoveDrag {
  originBeat: number;
  originPitch: number;
  /** 拖动前的快照：片段内序号 + 原音符。 */
  base: { index: number; note: Note }[];
  deltaBeat: number;
  deltaPitch: number;
  moved: boolean;
}

const mode = ref<DragMode | null>(null);
/** 拖拽创建中的音符（**相对片段**的拍，行锁定在 pitch）。 */
const create = reactive({ pitch: 60, from: 0, to: 0 });
const moveDrag = ref<MoveDrag | null>(null);
const resizeDrag = ref<{ index: number; originBeat: number; base: number; next: number; moved: boolean } | null>(null);
const marquee = reactive({ x1: 0, y1: 0, x2: 0, y2: 0 });

const scroller = ref<HTMLElement | null>(null);
const rollEl = ref<HTMLElement | null>(null);
/** 卷帘的纵向滚动（琴键列靠它同步）。 */
const scrollTop = ref(0);
let applyingScroll = false;

/** 控制器泳道当前显示的属性。 */
const controllerProperty = ref<ControllerProperty>("volume");

/** 指针位置 → (绝对拍, 音高)；`rel` 是相对片段起点的拍。 */
function pointAt(clientX: number, clientY: number): { beat: number; pitch: number; rel: number } {
  const el = rollEl.value;
  if (!el) return { beat: 0, pitch: HIGH, rel: 0 };
  const rect = el.getBoundingClientRect();
  const beat = Math.max(0, timeline.snapBeats((clientX - rect.left) / timeline.zoom));
  const row = Math.floor((clientY - rect.top) / ROW_H);
  const pitch = Math.min(HIGH, Math.max(LOW, HIGH - row));
  return { beat, pitch, rel: Math.max(0, beat - clipStart.value) };
}

const marqueeStyle = computed(() => ({
  left: `${Math.min(marquee.x1, marquee.x2)}px`,
  top: `${Math.min(marquee.y1, marquee.y2)}px`,
  width: `${Math.abs(marquee.x2 - marquee.x1)}px`,
  height: `${Math.abs(marquee.y2 - marquee.y1)}px`,
}));

function intersects(el: Element): boolean {
  const r = el.getBoundingClientRect();
  const left = Math.min(marquee.x1, marquee.x2);
  const right = Math.max(marquee.x1, marquee.x2);
  const top = Math.min(marquee.y1, marquee.y2);
  const bottom = Math.max(marquee.y1, marquee.y2);
  return !(r.right < left || r.left > right || r.bottom < top || r.top > bottom);
}

// ---------------------------------------------------------------------------
// 渲染
// ---------------------------------------------------------------------------

/** 渲染用：拖动中的音符使用预览值。 */
function noteOf(index: number, original: Note): Note {
  const d = moveDrag.value;
  if (d) {
    const item = d.base.find((b) => b.index === index);
    if (item) {
      return {
        ...item.note,
        start: Math.max(0, item.note.start + d.deltaBeat),
        pitch: Math.min(127, Math.max(0, item.note.pitch + d.deltaPitch)),
      };
    }
  }
  const r = resizeDrag.value;
  if (r && r.index === index) return { ...original, duration: r.next };
  return original;
}

function noteStyle(note: Note): Record<string, string> {
  return {
    top: `${(HIGH - note.pitch) * ROW_H + 1}px`,
    left: `${(clipStart.value + note.start) * timeline.zoom}px`,
    width: `${Math.max(8, note.duration * timeline.zoom)}px`,
  };
}

// ---------------------------------------------------------------------------
// 指针按下
// ---------------------------------------------------------------------------

function onNotePointerDown(e: PointerEvent, index: number): void {
  if (e.button !== 0) return;
  e.stopPropagation();

  if (e.metaKey || e.ctrlKey) {
    selection.toggleNote(index);
    return;
  }
  if (!selection.isNoteSelected(index)) {
    selection.selectNote(index);
  }

  capturePointer(e.currentTarget, e.pointerId);
  const p = pointAt(e.clientX, e.clientY);
  const indices = selection.noteIndices.length > 0 ? selection.noteIndices : [index];
  const c = clip.value;
  if (!c) return;
  const base = indices
    .map((i) => (i >= 0 && i < c.notes.length ? { index: i, note: { ...c.notes[i] } } : null))
    .filter((x): x is { index: number; note: Note } => x !== null);

  moveDrag.value = { originBeat: p.beat, originPitch: p.pitch, base, deltaBeat: 0, deltaPitch: 0, moved: false };
  mode.value = "move";
}

function onResizePointerDown(e: PointerEvent, index: number, note: Note): void {
  if (e.button !== 0) return;
  e.stopPropagation();
  selection.selectNote(index);
  capturePointer(e.currentTarget, e.pointerId);
  const p = pointAt(e.clientX, e.clientY);
  resizeDrag.value = { index, originBeat: p.beat, base: note.duration, next: note.duration, moved: false };
  mode.value = "resize";
}

/** 卷帘空白处按下：右键 → 框选删除；Shift/Alt 或「框选模式」→ 框选；否则左键拖拽创建音符。 */
function onRollPointerDown(e: PointerEvent): void {
  const c = clip.value;
  const t = trackId.value;
  if (!c || t === null) return;

  if (e.button === 2) {
    if ((e.target as HTMLElement).closest(".roll-note")) return;
    capturePointer(e.currentTarget, e.pointerId);
    mode.value = "marquee-delete";
    marquee.x1 = marquee.x2 = e.clientX;
    marquee.y1 = marquee.y2 = e.clientY;
    return;
  }
  if (e.button !== 0) return;

  // 顶栏的鼠标模式决定左键的默认含义；Shift/Alt 随时可以临时切到框选。
  const wantsMarquee = e.shiftKey || e.altKey || layout.tool === "select";
  if (wantsMarquee) {
    capturePointer(e.currentTarget, e.pointerId);
    mode.value = "marquee";
    marquee.x1 = marquee.x2 = e.clientX;
    marquee.y1 = marquee.y2 = e.clientY;
    return;
  }

  capturePointer(e.currentTarget, e.pointerId);
  const p = pointAt(e.clientX, e.clientY);
  selection.clearNotes();
  create.pitch = p.pitch;
  create.from = p.rel;
  create.to = p.rel;
  mode.value = "create";
  // 落笔就响：画音符的人要立刻听到这个音高。
  audition(p.pitch);
}

function onRollContextMenu(e: MouseEvent): void {
  e.preventDefault();
}

function onNoteContextMenu(e: MouseEvent, index: number): void {
  e.preventDefault();
  e.stopPropagation();
  const indices = selection.isNoteSelected(index) ? [...selection.noteIndices] : [index];
  void deleteNotes(indices);
}

// ---------------------------------------------------------------------------
// 试听（画音符 / 拖音符 / 点左侧琴键都会出声）
// ---------------------------------------------------------------------------

/**
 * 同一时刻只响一个音。
 *
 * 画音符时鼠标一路扫过很多行，如果每个音都留着会叠成噪音；所以记住当前响的那个，
 * 换音高时先把上一个松开。走的是插件的试听通道：不写工程、不进撤销历史、不等传输。
 */
const auditioning = ref<number | null>(null);

function audition(pitch: number): void {
  const t = trackId.value;
  if (t === null || pitch === auditioning.value) return;
  if (auditioning.value !== null) void auditionNote(t, auditioning.value, 0, false);
  auditioning.value = pitch;
  void auditionNote(t, pitch, 100, true);
}

function stopAudition(): void {
  const t = trackId.value;
  if (auditioning.value === null) return;
  if (t !== null) void auditionNote(t, auditioning.value, 0, false);
  auditioning.value = null;
}

/** 左侧琴键：按住即响，划过去换音高。 */
const keyDragging = ref(false);

function onKeyPointerDown(pitch: number, e: PointerEvent): void {
  keyDragging.value = true;
  audition(pitch);
  (e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
}

function onKeyPointerEnter(pitch: number): void {
  if (keyDragging.value) audition(pitch);
}

/** MIDI 音高夹到合法范围。 */
function clampPitch(pitch: number): number {
  return Math.min(127, Math.max(0, pitch));
}

// ---------------------------------------------------------------------------
// 指针移动
// ---------------------------------------------------------------------------

function onPointerMove(e: PointerEvent): void {
  switch (mode.value) {
    case "create": {
      const p = pointAt(e.clientX, e.clientY);
      create.to = p.rel;
      // 划到哪一行就试听哪一行——「画出音符的瞬间也该听到音高」。
      if (p.pitch !== create.pitch) audition(p.pitch);
      break;
    }
    case "move": {
      const d = moveDrag.value;
      if (!d) break;
      const p = pointAt(e.clientX, e.clientY);
      // 相对拖动：起点变化量 = 绝对拍变化量（片段偏移是常量，直接抵消）；按指针所在分段的步长吸附。
      const snap = timeline.snapAt(p.beat);
      const db = Math.round((p.beat - d.originBeat) / snap) * snap;
      const dp = p.pitch - d.originPitch;
      if (db !== d.deltaBeat || dp !== d.deltaPitch) {
        d.deltaBeat = db;
        d.deltaPitch = dp;
        d.moved = true;
        // 拖音符时听到「到达的那个音高」。
        audition(clampPitch(d.originPitch + dp));
      }
      break;
    }
    case "resize": {
      const r = resizeDrag.value;
      if (!r) break;
      const p = pointAt(e.clientX, e.clientY);
      const next = Math.max(MIN_CREATE_BEATS, timeline.snapBeats(r.base + (p.beat - r.originBeat)));
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
  // 试听收尾放在最前面：无论这次手势是画、是拖、还是点琴键，都要松开那个音。
  keyDragging.value = false;
  stopAudition();

  const current = mode.value;
  mode.value = null;
  const c = clip.value;
  const t = trackId.value;
  if (!c || t === null) return;

  if (current === "create") {
    const from = Math.min(create.from, create.to);
    const span = Math.abs(create.to - create.from);
    // 拖动距离即音符时长；原地点击给一个默认时长（不是「什么都不做」）。
    const duration = span >= MIN_CREATE_BEATS ? span : CLICK_NOTE_BEATS;
    const index = c.notes.length;
    try {
      await store.addNote(t, c.id, {
        pitch: create.pitch,
        start: from,
        duration,
        velocity: 100,
        volume: 127,
      });
      selection.selectNote(index);
    } catch (err) {
      layout.notify(i18n.t("error.addNote", { error: String(err) }));
    }
    return;
  }

  if (current === "move") {
    const d = moveDrag.value;
    moveDrag.value = null;
    if (!d || !d.moved) return;
    const entries: NoteEdit[] = d.base.map((b) => ({
      index: b.index,
      note: {
        ...b.note,
        start: Math.max(0, b.note.start + d.deltaBeat),
        pitch: Math.min(127, Math.max(0, b.note.pitch + d.deltaPitch)),
      },
    }));
    try {
      await store.setNotes(t, c.id, entries);
    } catch (err) {
      layout.notify(i18n.t("error.moveNote", { error: String(err) }));
    }
    return;
  }

  if (current === "resize") {
    const r = resizeDrag.value;
    resizeDrag.value = null;
    if (!r || !r.moved) return;
    const note = c.notes[r.index];
    if (!note) return;
    try {
      await store.setNote(t, c.id, r.index, { ...note, duration: r.next });
    } catch (err) {
      layout.notify(i18n.t("error.resizeNote", { error: String(err) }));
    }
    return;
  }

  if (current === "marquee") {
    const indices: number[] = [];
    rollEl.value?.querySelectorAll<HTMLElement>(".roll-note").forEach((el) => {
      if (!intersects(el)) return;
      const index = Number(el.dataset.index);
      if (Number.isFinite(index)) indices.push(index);
    });
    selection.setNoteSelection(indices);
    return;
  }

  if (current === "marquee-delete") {
    const width = Math.abs(marquee.x2 - marquee.x1);
    const height = Math.abs(marquee.y2 - marquee.y1);
    // 几乎没动 = 一次右键点击：由 contextmenu 处理，这里不做事。
    if (width < 4 && height < 4) return;
    const indices: number[] = [];
    rollEl.value?.querySelectorAll<HTMLElement>(".roll-note").forEach((el) => {
      if (!intersects(el)) return;
      const index = Number(el.dataset.index);
      if (Number.isFinite(index)) indices.push(index);
    });
    await deleteNotes(indices);
  }
}

async function deleteNotes(indices: number[]): Promise<void> {
  const c = clip.value;
  const t = trackId.value;
  if (!c || t === null || indices.length === 0) return;
  try {
    await store.removeNotes(t, c.id, indices);
    // 序号会因删除而位移，选择必须清空（不能指向错位的音符）。
    selection.clearNotes();
    layout.notify(
      indices.length === 1
        ? i18n.t("edit.noteRemoved")
        : i18n.t("edit.notesRemoved", { count: indices.length }),
    );
  } catch (err) {
    layout.notify(i18n.t("error.removeNote", { error: String(err) }));
  }
}

// ---------------------------------------------------------------------------
// 控制器泳道
// ---------------------------------------------------------------------------

/** 把属性改动应用到全部所选音符（一次提交 = 一条撤销记录）。 */
async function applyToSelection(patch: Partial<Note>): Promise<void> {
  const c = clip.value;
  const t = trackId.value;
  if (!c || t === null) return;
  const entries: NoteEdit[] = selection.noteIndices
    .map((i) => (i >= 0 && i < c.notes.length ? { index: i, note: { ...c.notes[i], ...patch } } : null))
    .filter((x): x is NoteEdit => x !== null);
  if (entries.length === 0) return;
  try {
    await store.setNotes(t, c.id, entries);
  } catch (err) {
    layout.notify(i18n.t("error.editNote", { error: String(err) }));
  }
}

// ---------------------------------------------------------------------------
// 轨道切换：跳到「同一时刻」的片段，没有就回编排
// ---------------------------------------------------------------------------

/** 某轨道里覆盖指定拍位置的片段。 */
function clipCovering(track: number, beats: number) {
  const t = store.project?.tracks.find((x) => x.id === track);
  if (!t) return null;
  return t.clips.find((c) => beats >= c.start && beats < c.start + c.length) ?? null;
}

/**
 * 在钢琴卷帘界面点左侧音轨栏换轨道时：
 * - 该轨道在**当前播放头位置**有片段 → 直接切到那个片段（"同一时刻的音频块"）；
 * - 没有 → 回编排视图（用户显然想先造一个片段）。
 *
 * 从编排双击片段进来时 `trackId` 不变，所以不会误触发。
 */
watch(
  () => selection.trackId,
  (id, prev) => {
    if (id === null || id === prev) return;
    const target = clipCovering(id, transport.positionBeats);
    if (target) {
      selection.selectClip(id, target.id);
      return;
    }
    void router.push("/arrange");
  },
);

// ---------------------------------------------------------------------------
// 空状态：轨道上没有片段时给一个直接可用的入口
// ---------------------------------------------------------------------------

/** 当前轨道还没有片段时，一键建一个 1 小节片段并选中它。 */
async function createClipHere(): Promise<void> {
  const t = trackId.value;
  if (t === null) return;
  try {
    await store.addClip(t, 0, beatsPerBar.value);
    const track = store.project?.tracks.find((x) => x.id === t);
    const created = track?.clips[track.clips.length - 1];
    if (created) selection.selectClip(t, created.id);
  } catch (err) {
    layout.notify(i18n.t("error.addClip", { error: String(err) }));
  }
}

// ---------------------------------------------------------------------------
// 滚动 / 缩放
// ---------------------------------------------------------------------------

/** 自己滚动：横向写回共享时间轴，纵向留给琴键列同步。 */
function onScroll(): void {
  const el = scroller.value;
  if (!el) return;
  scrollTop.value = el.scrollTop;
  if (!applyingScroll && Math.abs(timeline.scrollLeft - el.scrollLeft) >= 1) {
    timeline.scrollLeft = el.scrollLeft;
  }
}

// 共享时间轴的滚动位置变了（另一个视图滚的、或者播放头跟随）→ 跟随。
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

/** Ctrl/Cmd + 滚轮缩放：锚点在指针下，两个视图同步。 */
function onWheel(e: WheelEvent): void {
  if (!(e.ctrlKey || e.metaKey)) return;
  e.preventDefault();
  const el = scroller.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  el.scrollLeft = timeline.zoomAt(e.clientX - rect.left, e.deltaY < 0 ? 1.15 : 1 / 1.15);
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

/**
 * 纵向滚到「这个片段的音符所在的地方」。
 *
 * 卷帘的可视音域是 C2..C6（49 行 ≈ 686px），而低音片段可能在第 500px 处——
 * 不做这件事，进低音片段的卷帘会是一片空白，用户以为音符丢了。
 */
function revealNotes(): void {
  const el = scroller.value;
  if (!el) return;
  const pitches = clip.value?.notes.map((n) => n.pitch) ?? [];
  const center =
    pitches.length > 0 ? pitches.reduce((a, b) => a + b, 0) / pitches.length : (HIGH + LOW) / 2;
  const y = (HIGH - center) * ROW_H;
  el.scrollTop = Math.max(0, y - el.clientHeight / 2);
  scrollTop.value = el.scrollTop;
}

// 换片段 / 首次进入时都对准一次。
watch(() => `${selection.trackId}:${selection.clipId}`, () => {
  requestAnimationFrame(revealNotes);
});

// ---------------------------------------------------------------------------
// 键盘
// ---------------------------------------------------------------------------

function onKey(e: KeyboardEvent): void {
  if (e.target instanceof HTMLInputElement) return;
  const mod = e.metaKey || e.ctrlKey;
  if (mod && e.key.toLowerCase() === "c") {
    const c = clip.value;
    const idx = selection.noteIndices;
    if (!c || idx.length !== 1) return;
    const note = c.notes[idx[0]!];
    if (note) {
      e.preventDefault();
      clipboard.copyNote(note);
    }
    return;
  }
  if (mod && e.key.toLowerCase() === "v") {
    e.preventDefault();
    void pasteNote();
    return;
  }
  if (e.key !== "Delete" && e.key !== "Backspace") return;
  if (selection.noteIndices.length === 0) return;
  e.preventDefault();
  void deleteNotes([...selection.noteIndices]);
}

/** 粘贴：副本落在该片段最后一个音符之后。 */
async function pasteNote(): Promise<void> {
  const p = clipboard.payload;
  const c = clip.value;
  const t = trackId.value;
  if (!p || p.kind !== "note" || !c || t === null) return;
  const start = c.notes.reduce((m, n) => Math.max(m, n.start + n.duration), 0);
  try {
    const index = c.notes.length;
    await store.addNote(t, c.id, { ...p.note, start });
    selection.selectNote(index);
  } catch (err) {
    layout.notify(i18n.t("error.addNote", { error: String(err) }));
  }
}

/** 切换片段后清空音符选择（序号属于旧片段）。 */
watch(
  () => `${selection.trackId}:${selection.clipId}`,
  () => selection.clearNotes(),
);

onMounted(() => {
  window.addEventListener("keydown", onKey);
  window.addEventListener("pointerup", onPointerUp);
  window.addEventListener("pointercancel", onPointerUp);
  // 进视图时对齐共享时间轴的横向位置（从编排切过来时视角不变），纵向对准音符。
  requestAnimationFrame(() => {
    const el = scroller.value;
    if (el) el.scrollLeft = timeline.scrollLeft;
    revealNotes();
  });
});
onUnmounted(() => {
  window.removeEventListener("keydown", onKey);
  window.removeEventListener("pointerup", onPointerUp);
  window.removeEventListener("pointercancel", onPointerUp);
});
</script>

<template>
  <div class="editor">
    <!-- 顶部标尺：左侧 60px 与琴键列对齐；其余与卷帘内容同一横坐标原点 -->
    <div class="ruler-row">
      <div class="ruler-gutter">
        <span class="grid-label" :title="i18n.t('edit.snapGrid', { grid: gridLabel })">{{ gridLabel }}</span>
      </div>
      <div class="ruler-view">
        <div class="ruler-inner" :style="{ width: rollWidth, transform: `translateX(${-timeline.scrollLeft}px)` }">
          <TimelineRuler />
        </div>
      </div>
    </div>

    <!-- 主体：琴键列（不随横向滚动）+ 卷帘 -->
    <div class="body-row">
      <div class="keys" role="group" :aria-label="i18n.t('edit.auditionKeys')">
        <div class="keys-inner" :style="{ transform: `translateY(${-scrollTop}px)` }">
          <div
            v-for="p in rows"
            :key="p"
            class="key"
            :class="[isBlack(p) ? 'black' : 'white', { on: auditioning === p }]"
            :title="i18n.t('edit.auditionKey', { name: noteName(p) })"
            @pointerdown="onKeyPointerDown(p, $event)"
            @pointerenter="onKeyPointerEnter(p)"
          >
            <span v-if="isBlack(p)" class="key-cap" />
            <span v-else-if="p % 12 === 0" class="key-label">{{ noteName(p) }}</span>
          </div>
        </div>
      </div>

      <div ref="scroller" class="piano" @scroll="onScroll" @wheel="onWheel">
        <div
          ref="rollEl"
          class="roll"
          :style="{ width: rollWidth, height: rollHeight, ...timeline.gridStyle }"
          @pointerdown="onRollPointerDown"
          @pointermove="onPointerMove"
          @contextmenu="onRollContextMenu"
        >
          <!-- 黑键行的暗色带：让八度结构在网格里也看得见 -->
          <div v-for="p in rows" :key="p" class="roll-row" :class="{ black: isBlack(p) }" />

          <!-- 片段范围：音符该待的地方 -->
          <div v-if="clipBand" class="clip-band" :style="clipBand" />

          <!-- 拖拽创建的预览 -->
          <div
            v-if="mode === 'create'"
            class="note-ghost"
            :style="{
              top: `${(HIGH - create.pitch) * ROW_H + 1}px`,
              left: `${(clipStart + Math.min(create.from, create.to)) * timeline.zoom}px`,
              width: `${Math.max(2, Math.abs(create.to - create.from) * timeline.zoom)}px`,
            }"
          />

          <div
            v-for="(n, i) in clip?.notes ?? []"
            :key="i"
            class="roll-note"
            :class="{
              selected: selection.isNoteSelected(i),
              dragging: moveDrag?.base.some((b) => b.index === i),
            }"
            :style="noteStyle(noteOf(i, n))"
            :data-index="i"
            :title="
              i18n.t('edit.noteTitle', {
                name: noteName(noteOf(i, n).pitch),
                start: noteOf(i, n).start.toFixed(2),
                velocity: noteOf(i, n).velocity,
                volume: noteOf(i, n).volume,
              })
            "
            @pointerdown="onNotePointerDown($event, i)"
            @contextmenu="onNoteContextMenu($event, i)"
          >
            <span
              class="note-handle"
              aria-hidden="true"
              @pointerdown="onResizePointerDown($event, i, n)"
            />
          </div>

          <div class="playhead" :style="{ left: `${playheadPx}px` }" />
        </div>

        <!-- 没有片段时的入口：直接建一个片段，否则这里是个死胡同 -->
        <div v-if="!clip" class="roll-blank">
          <p class="text-body-md">{{ i18n.t("edit.noClip") }}</p>
          <button v-if="trackId !== null" class="btn-filled" @click="createClipHere">
            {{ i18n.t("edit.createBarClip") }}
          </button>
        </div>
      </div>
    </div>

    <!-- 卷帘与控制器泳道之间可拖拽 -->
    <PanelResizer side="bottom" @resize="layout.resizeLane($event)" />

    <!-- 控制器泳道：经典 DAW 的一排直角条 -->
    <ControllerLane
      :notes="clip?.notes ?? []"
      :selected-indices="selection.noteIndices"
      :property="controllerProperty"
      :offset-beats="clipStart"
      :content-height="layout.laneHeight"
      @update:property="(p: ControllerProperty) => (controllerProperty = p)"
      @select="selection.setNoteSelection($event)"
      @apply="applyToSelection"
      @delete="deleteNotes"
    />

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
.editor {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background-color: transparent;
  user-select: none;
  -webkit-user-select: none;
}
.editor ::selection {
  background: transparent;
}

/* ---- 顶部标尺 ---- */
.ruler-row {
  flex: 0 0 auto;
  display: flex;
  min-width: 0;
}
/* 与琴键列同宽：让标尺的横坐标原点与卷帘一致 */
.ruler-gutter {
  flex: 0 0 60px;
  height: 40px;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  padding-right: 7px;
  background-color: var(--app-inset);
  border-bottom: 1px solid var(--grid-line-strong);
  border-right: 1px solid var(--grid-line-row);
}
.grid-label {
  font-size: 9px;
  letter-spacing: 0.4px;
  color: rgb(var(--md-on-surface-muted));
  font-variant-numeric: tabular-nums;
}
.ruler-view {
  position: relative;
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
}
.ruler-inner {
  height: 40px;
}

/* ---- 主体 ---- */
.body-row {
  flex: 1;
  min-height: 0;
  display: flex;
  min-width: 0;
}
/* 琴键列：不参与横向滚动，纵向由 scrollTop 同步 */
.keys {
  flex: 0 0 60px;
  overflow: hidden;
  background-color: var(--piano-white);
  border-right: 1px solid rgb(var(--md-outline-variant) / 0.65);
  box-shadow: 1px 0 0 rgb(var(--md-on-surface) / 0.06);
}
.keys-inner {
  will-change: transform;
}
.key {
  position: relative;
  height: 14px;
  display: flex;
  align-items: center;
  cursor: pointer;
  touch-action: none;
}
/* 白键：整行浅色，底部一条缝 */
.key.white {
  background-color: var(--piano-white);
  border-bottom: 1px solid var(--piano-edge);
}
/* 黑键：左侧一段深色短键，右侧露出下面的白键 */
.key.black {
  background-color: var(--piano-white);
}
/* 正在试听的那一行：整行点亮，一眼看得出在响哪个音。
   放在白/黑键规则**之后**，否则同特异度下会被它们盖掉。 */
.key.on {
  background-color: rgb(var(--md-primary) / 0.45);
}
.key.on .key-cap {
  background-color: rgb(var(--md-primary) / 0.75);
}
.key.on .key-label {
  color: rgb(var(--md-on-primary));
}
.key-cap {
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 62%;
  background-color: var(--piano-black);
  border-bottom: 1px solid var(--piano-edge);
  border-top-right-radius: 3px;
  border-bottom-right-radius: 3px;
}
.key-label {
  position: relative;
  padding-left: 6px;
  font-size: 8.5px;
  color: rgb(var(--md-on-surface-muted));
}

/* 卷帘：横向滚动容器，内容原点 = 工程起点 */
.piano {
  position: relative;
  flex: 1 1 auto;
  min-width: 0;
  overflow: auto;
  background-color: var(--app-card);
  scrollbar-width: thin;
}
.roll {
  position: relative;
  min-width: 100%;
  cursor: crosshair;
}
.roll-row {
  height: 14px;
  border-bottom: 1px solid var(--grid-line-row);
}
/* 黑键行整行压暗：八度结构一眼可辨（这是钢琴卷帘的通用语言） */
.roll-row.black {
  background-color: var(--roll-black-tint);
}
/* 片段范围高亮 */
.clip-band {
  position: absolute;
  top: 0;
  bottom: 0;
  background-color: rgb(var(--md-primary) / 0.06);
  border-left: 1px solid rgb(var(--md-primary) / 0.45);
  border-right: 1px solid rgb(var(--md-primary) / 0.45);
  pointer-events: none;
}
/* 拖拽创建预览：虚线框 + 主色淡填充 */
.note-ghost {
  position: absolute;
  height: 12px;
  border-radius: 4px;
  border: 1px dashed rgb(var(--md-primary));
  background-color: rgb(var(--md-primary) / 0.25);
  pointer-events: none;
}
.roll-note {
  position: absolute;
  height: 12px;
  border-radius: 4px;
  background-color: rgb(var(--md-primary));
  cursor: grab;
  touch-action: none;
}
.roll-note.dragging {
  cursor: grabbing;
  opacity: 0.9;
}
/* 选中：只加一圈同色系描边，不做任何「蓝色高亮」 */
.roll-note.selected {
  box-shadow: 0 0 0 2px rgb(var(--md-surface)), 0 0 0 3px rgb(var(--md-primary));
}
/* 右边缘的时长热区 */
.note-handle {
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  width: 7px;
  cursor: ew-resize;
}
.roll-note:hover .note-handle {
  background-color: rgb(var(--md-on-primary) / 0.35);
}
.playhead {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 2px;
  margin-left: -1px;
  background-color: rgb(var(--md-primary) / 0.9);
  pointer-events: none;
  z-index: 2;
}
/* 没有片段时的空状态：居中在可见区域，并给出一个可点的入口 */
.roll-blank {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  color: rgb(var(--md-on-surface-muted));
  background-color: rgb(var(--md-surface) / 0.55);
  pointer-events: none;
}
.roll-blank button {
  pointer-events: auto;
}
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
