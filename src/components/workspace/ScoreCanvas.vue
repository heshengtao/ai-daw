<script setup lang="ts">
/**
 * 谱面画布：用 **alphaTab**（Guitar Pro 的引擎）渲染 MusicXML，并画播放指示。
 *
 * 为什么换掉 OSMD：OSMD 不支持六线谱，而且它的坐标是内部结构（`GraphicSheet`），
 * 播放光标只能自己按包围盒硬算，实测对不上。alphaTab 这些是它的正业：
 * - 五线谱 / 六线谱 / 鼓谱都能排，六线谱还带连梁、符头、调弦标注；
 * - 渲染完直接给 `boundsLookup`（每个 beat / masterBar 的像素框），播放指示不用再猜坐标；
 * - 还能顺手支持 .gp 文件导入（将来）。
 *
 * 数据仍然走我们自己的 MusicXML（`src/score`），六线谱靠 `<technical><string>/<fret>`。
 * alphaTab 的字体（Bravura）在 `public/alphatab/`，桌面端离线可用。
 */
import { computed, nextTick, onMounted, onUnmounted, ref, shallowRef, watch } from "vue";
import * as alphaTab from "@coderline/alphatab";
import { useI18n } from "@/i18n";
import { useTheme } from "@/composables/useTheme";
import { useProjectStore } from "@/stores/project";
import { useLayoutStore } from "@/stores/layout";
import { useTransportStore } from "@/stores/transport";
import { buildMusicXML, type ClefKind } from "@/score/musicxml";
import { defaultNotationFor, isTabNotation } from "@/utils/notation";

const props = withDefaults(
  defineProps<{
    /** 只出这些轨道；不传 = 全部有音符的轨道。 */
    tracks?: number[];
    /** 只出这一段（绝对拍）；不传 = 整首。 */
    range?: { start: number; end: number };
    /** 播放头出视口时自动滚动。 */
    follow?: boolean;
  }>(),
  { follow: true },
);

const i18n = useI18n();
const project = useProjectStore();
const layout = useLayoutStore();
const transport = useTransportStore();
const { isDark } = useTheme();

const emit = defineEmits<{
  /** 双击谱面：外壳据此打开「记谱设置」弹窗（谱号 / 调弦）。 */
  (e: "edit-notation"): void;
}>();

/** 记谱量化网格（拍）。与导出对话框的默认值一致。 */
const GRID = 0.25;
/** alphaTab 的 tick：四分音符 = 960。 */
const TICKS_PER_BEAT = 960;

const host = ref<HTMLDivElement | null>(null);
const scroller = ref<HTMLDivElement | null>(null);
const error = ref<string | null>(null);
const busy = ref(false);

/** 一个包围盒（像素，alphaTab 的画布坐标）。 */
interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** 渲染出来的一拍：位置 + 它属于第几个小节（`bar` = masterBar 下标）。 */
interface TickBox extends Box {
  tick: number;
  bar: number;
}

const beatBoxes = shallowRef<TickBox[]>([]);
/** 小节包围盒，**按 masterBar 下标存**（没有的留 `null`）。 */
const barBoxes = shallowRef<(Box | null)[]>([]);
const cursor = ref<{ x: number; y: number; h: number } | null>(null);
const barHighlight = ref<Box | null>(null);
/** alphaTab 的画布元素：包围盒以它的左上角为原点，点谱换算也必须以它为准。 */
const surface = shallowRef<HTMLElement | null>(null);
/**
 * 谱面第 1 拍的**绝对**拍位置。
 *
 * 下窗口只画当前片段时，谱面从片段所在的那一小节开始——alphaTab 的 tick 从 0 起算，
 * 播放头却是整首的绝对拍，中间差的就是这个偏移。少了它，光标永远追不上，点击也永远
 * 跳回片段开头（听感上就是「在第一行里无限循环」）。
 */
const scoreStartBeat = ref(0);

/** alphaTab 的运行时导出是 `alphaTab.*` 命名空间，这里做一次兼容取值。 */
const AlphaTabApiCtor =
  (alphaTab as unknown as { AlphaTabApi?: unknown }).AlphaTabApi ??
  (alphaTab as unknown as { alphaTab?: { AlphaTabApi?: unknown } }).alphaTab?.AlphaTabApi;

const api = shallowRef<{ destroy?: () => void } | null>(null);
let disposed = false;
let renderTimer: number | null = null;

const renderKey = computed(() => {
  const t = props.tracks ? props.tracks.join(",") : "*";
  const r = props.range ? `${props.range.start}:${props.range.end}` : "*";
  // 任何一条轨道的谱式变了都要重排
  return `${t}|${r}|${JSON.stringify(layout.trackNotations)}|${JSON.stringify(layout.trackClefs)}|${JSON.stringify(layout.trackTunings)}`;
});

function scheduleRender(delay = 450): void {
  if (renderTimer !== null) window.clearTimeout(renderTimer);
  renderTimer = window.setTimeout(() => {
    renderTimer = null;
    void render();
  }, delay);
}

/** 深色 / 浅色下的墨色（alphaTab 的渲染资源）。 */
function resources(): Record<string, string> {
  return isDark.value
    ? {
        mainGlyphColor: "#e4e4e7",
        secondaryGlyphColor: "#a1a1aa",
        staffLineColor: "#3f3f46",
        barSeparatorColor: "#52525b",
        // 小节号：alphaTab 默认是偏红的，这里压成中性灰，跟着主题走
        barNumberColor: "#a1a1aa",
        scoreInfoColor: "#d4d4d8",
      }
    : {
        mainGlyphColor: "#17171a",
        secondaryGlyphColor: "#52525b",
        staffLineColor: "#a1a1aa",
        barSeparatorColor: "#71717a",
        barNumberColor: "#52525b",
        scoreInfoColor: "#3f3f46",
      };
}

/** 读一个 alphaTab 的 `Bounds`（不同版本字段名略有差异，都兜一下）。 */
function readBox(value: unknown): Box | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  const x = v.x ?? v.X;
  const y = v.y ?? v.Y;
  const w = v.w ?? v.W ?? v.width;
  const h = v.h ?? v.H ?? v.height;
  if (![x, y, w, h].every((n) => typeof n === "number" && Number.isFinite(n))) return null;
  return { x: x as number, y: y as number, w: w as number, h: h as number };
}

/**
 * 收集 alphaTab 渲染出来的包围盒。
 *
 * 用**乐谱模型 + `findBeat`** 枚举，而不是去猜 `boundsLookup` 的内部层级：
 * 后者拿到的 `staffSystems` 在某些配置下只覆盖第一行。现在的做法对每一行、每个拍都问一次
 * 渲染器「你在哪」，跨行自然就对上了。
 *
 * 小节必须**按 masterBar 下标**存：以前是拿光标的 x 去反查小节，可多行谱里每行的小节的 x
 * 范围是重叠的，`find` 永远命中第一行那一个 —— 于是光标看着一直在第一行上跑，点下行
 * 也像没反应。
 */
function collectBounds(): void {
  const instance = api.value as unknown as {
    boundsLookup?: {
      findBeat?: (beat: unknown) => { visualBounds?: unknown; realBounds?: unknown } | null;
      findMasterBarByIndex?: (index: number) => { visualBounds?: unknown; realBounds?: unknown } | null;
    };
    canvasElement?: { element?: HTMLElement };
    score?: {
      masterBars?: unknown[];
      tracks?: {
        staves?: {
          bars?: { masterBar?: { index?: number }; voices?: { beats?: unknown[] }[] }[];
        }[];
      }[];
    };
  } | null;
  const lookup = instance?.boundsLookup;
  const score = instance?.score;
  if (!lookup || !score) return;

  surface.value = instance?.canvasElement?.element ?? null;

  const masterCount = score.masterBars?.length ?? 0;
  const bars: (Box | null)[] = [];
  for (let i = 0; i < masterCount; i += 1) {
    const found = lookup.findMasterBarByIndex?.(i);
    // `realBounds` 是整条系统的框（所有谱表叠起来），`visualBounds` 只有谱线一带
    bars.push(readBox(found?.realBounds) ?? readBox(found?.visualBounds) ?? null);
  }

  // 拍只走**第一条轨道的第一个谱表**：各轨道共用同一组小节网格，横向位置一致，
  // 纵向由「它属于哪个小节」给出的系统框负责。
  const beats: TickBox[] = [];
  for (const bar of score.tracks?.[0]?.staves?.[0]?.bars ?? []) {
    const barIndex = bar.masterBar?.index ?? 0;
    for (const voice of bar.voices ?? []) {
      for (const beat of voice.beats ?? []) {
        const found = lookup.findBeat?.(beat);
        const box = readBox(found?.visualBounds ?? found?.realBounds);
        const tick = (beat as { absolutePlaybackStart?: number }).absolutePlaybackStart;
        if (box && typeof tick === "number") beats.push({ tick, bar: barIndex, ...box });
      }
    }
  }

  beats.sort((a, b) => a.tick - b.tick);
  beatBoxes.value = beats;
  barBoxes.value = bars;
}

/** 谱面画布左上角为原点时，页面坐标 → 谱面坐标。 */
function toScorePoint(e: MouseEvent): { x: number; y: number } | null {
  const el = surface.value ?? host.value;
  if (!el) return null;
  const rect = el.getBoundingClientRect();
  return { x: e.clientX - rect.left, y: e.clientY - rect.top };
}

/**
 * `getBeatAtPos` 只在**谱线正上方**命中：页面留白、行间空隙、小节末尾之外的空白全返回
 * `null`。点这些地方时退回「最近的拍」——先保证落在同一行，再比横向距离。
 */
function nearestBoxTick(x: number, y: number): number | null {
  let best: TickBox | null = null;
  let bestScore = Number.POSITIVE_INFINITY;
  for (const box of beatBoxes.value) {
    const dy = y < box.y ? box.y - y : y > box.y + box.h ? y - box.y - box.h : 0;
    const dx = x < box.x ? box.x - x : x > box.x + box.w ? x - box.x - box.w : 0;
    // 纵向权重放大：先保证是同一行，再比横向
    const score = dy * 1000 + dx;
    if (score < bestScore) {
      bestScore = score;
      best = box;
    }
  }
  return best ? best.tick : null;
}

/** 在谱面上点击 → 把播放头挪到最近的节拍（GTP 的行为）。 */
function onSeek(e: MouseEvent): void {
  const lookup = (
    api.value as unknown as {
      boundsLookup?: { getBeatAtPos?: (x: number, y: number) => { absolutePlaybackStart?: number } | null };
    } | null
  )?.boundsLookup;
  if (!lookup) return;
  const point = toScorePoint(e);
  if (!point) return;
  const hit = lookup.getBeatAtPos?.(point.x, point.y) ?? null;
  const tick =
    typeof hit?.absolutePlaybackStart === "number" ? hit.absolutePlaybackStart : nearestBoxTick(point.x, point.y);
  if (tick === null) return;
  // 谱面 tick 是相对本谱起点的，播放头是绝对拍：加回偏移，否则永远跳回片段头
  void transport.seekBeats(tick / TICKS_PER_BEAT + scoreStartBeat.value);
}

async function render(): Promise<void> {
  const el = host.value;
  if (!el || disposed) return;
  const current = project.project;
  if (!current) return;

  const result = buildMusicXML(current, {
    tracks: props.tracks,
    range: props.range,
    grid: GRID,
    keyFifths: 0,
    // 谱式逐轨道决定：手动设置过的优先，否则按乐器猜（吉他→六线谱、鼓→打击乐谱表）。
    notationFor: (track) => layout.trackNotationOf(track.id) ?? defaultNotationFor(track),
    clefFor: (track) => (layout.trackClefs[String(track.id)] as ClefKind | undefined) ?? null,
    tuningFor: (track) => layout.trackTunings[String(track.id)] ?? null,
  });

  if (result.parts === 0) {
    error.value = i18n.t("score.empty");
    scoreStartBeat.value = 0;
    teardown();
    el.replaceChildren();
    return;
  }

  // 谱面从哪个绝对拍开始（整首 = 0，片段 = 所在小节的开头）
  scoreStartBeat.value = result.barStarts[0] ?? 0;

  if (!AlphaTabApiCtor) {
    error.value = i18n.t("score.failed", { error: "alphaTab 未加载" });
    return;
  }

  busy.value = true;
  try {
    teardown();
    el.replaceChildren();

    const settings = {
      // `enableLazyLoading` 关掉：它省的是「画」，省不了最贵的「排版」，但把每一页的绘制
      // 交给 IntersectionObserver——观察器在某些环境里不回调（截图的 headless 浏览器就是），
      // 结果是排版完成、包围盒齐全、页面却是空白。宁可一次画完。
      core: { fontDirectory: "/alphatab/", useWorkers: false, enableLazyLoading: false, logLevel: "warning" },
      player: { playerMode: 0, enableUserInteraction: false, enableCursor: false, scrollMode: 0 },
      display: {
        layoutMode: "Page",
        // 总谱里各轨谱式不同：吉他有调弦 → 五线谱+六线谱，鼓 → 打击乐谱表，其余 → 五线谱。
        // `ScoreTab` 正是「有调弦的排六线谱、没有的只排五线谱」。
        staveProfile: "ScoreTab",
        scale: 1,
        resources: resources(),
      },
    };

    const instance = new (AlphaTabApiCtor as new (el: HTMLElement, s: unknown) => Record<string, unknown>)(
      el,
      settings,
    );
    api.value = instance as { destroy?: () => void };

    const onError = (e: unknown): void => {
      error.value = i18n.t("score.failed", { error: String((e as { message?: string })?.message ?? e) });
    };
    (instance as { error?: { on: (cb: (e: unknown) => void) => void } }).error?.on(onError);

    (instance as { renderFinished?: { on: (cb: () => void) => void } }).renderFinished?.on(() => {
      busy.value = false;
      error.value = null;
      collectBounds();
      updateCursor();
    });

    /**
     * 先自己解析成 Score、改好每轨的可见性，再交给渲染器 —— 不要 `load()`。
     *
     * `load()` 会「解析 → 立刻排一次」；我们想在这之间打开所有谱表的 `showStandardNotation`
     * （否则某些 part 整条不画，总谱只剩一行），过去是等排完再改再重排一次，等于把最贵的
     * 排版干了两遍，总谱一长就卡死。
     */
    const bytes = new TextEncoder().encode(result.xml);
    const loader = (alphaTab as unknown as { importer?: { ScoreLoader?: { loadScoreFromBytes: (b: Uint8Array, s: unknown) => unknown } } })
      .importer?.ScoreLoader;
    const SettingsCtor = (alphaTab as unknown as { Settings?: new () => unknown }).Settings;
    const renderScore = (instance as { renderScore?: (score: unknown, tracks?: number[]) => void }).renderScore;

    if (loader && SettingsCtor && typeof renderScore === "function") {
      const score = loader.loadScoreFromBytes(bytes, new SettingsCtor()) as {
        tracks?: { staves?: { showStandardNotation?: boolean; showTablature?: boolean }[] }[];
      };
      // 逐轨定谱式：`<part>` 的顺序就是 `result.trackIds` 的顺序。
      // - 五线谱/鼓谱：只画标准谱表；
      // - 吉他谱/贝斯谱：标准谱表 + 六线谱（调弦信息在 MusicXML 的 `<staff-tuning>` 里）。
      // 全画 / 只画某一边都由这两个开关决定，不靠渲染器的「自动」猜。
      score.tracks?.forEach((track, index) => {
        const id = result.trackIds[index];
        const src = id === undefined ? null : (current.tracks.find((t) => t.id === id) ?? null);
        const notation = src ? (layout.trackNotationOf(src.id) ?? defaultNotationFor(src)) : "score";
        const tab = isTabNotation(notation);
        for (const staff of track.staves ?? []) {
          staff.showStandardNotation = true;
          staff.showTablature = tab;
        }
      });
      /**
       * 第二个参数是**轨道列表**：`renderScore(score)` 不传它时 alphaTab 只画
       * `tracks[0]`（源码里 `if (!trackIndexes) tracks.push(score.tracks[0])`），
       * 总谱因此永远只剩第一轨。`[-1]` 才是「全部轨道」。
       */
      renderScore.call(instance, score, [-1]);
    } else {
      // 兜底：拿不到解析器就直接 load（会有个别谱表不显示，但至少能看）
      (instance as { load: (data: unknown) => boolean }).load(bytes);
    }
  } catch (err) {
    error.value = i18n.t("score.failed", { error: String(err) });
  } finally {
    busy.value = false;
  }
}

function teardown(): void {
  try {
    (api.value as { destroy?: () => void })?.destroy?.();
  } catch {
    // 销毁失败不影响后续重建
  }
  api.value = null;
  surface.value = null;
  beatBoxes.value = [];
  barBoxes.value = [];
  cursor.value = null;
  barHighlight.value = null;
}

/** 按播放头更新指示：当前 beat 的框 + 小节框 + 竖线。 */
function updateCursor(): void {
  const boxes = beatBoxes.value;
  if (boxes.length === 0) {
    cursor.value = null;
    barHighlight.value = null;
    return;
  }

  // 谱面 tick 相对本谱起点，播放头是绝对拍
  const tick = (transport.positionBeats - scoreStartBeat.value) * TICKS_PER_BEAT;
  const last = boxes[boxes.length - 1]!;
  if (tick < boxes[0]!.tick - 1 || tick > last.tick + TICKS_PER_BEAT * 4) {
    cursor.value = null;
    barHighlight.value = null;
    return;
  }

  // 二分找「最后一个 tick <= 当前位置」的 beat（逐帧线性扫全谱在长曲子上会拖慢播放）
  let lo = 0;
  let hi = boxes.length - 1;
  let index = 0;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (boxes[mid]!.tick <= tick) {
      index = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  const box = boxes[index]!;
  const next = boxes[index + 1];
  const span = next ? Math.max(1, next.tick - box.tick) : TICKS_PER_BEAT;
  const fraction = Math.min(1, Math.max(0, (tick - box.tick) / span));
  const x = box.x + fraction * box.w;

  // 小节框直接用**这一拍所在的小节下标**取：竖线的纵向范围是整条系统（所有谱表），
  // 换行时 y 会跟着跳。以前按 x 反查小节，多行谱里 x 范围重叠 → 永远命中第一行。
  const bar = barBoxes.value[box.bar] ?? null;
  barHighlight.value = bar;

  const y = bar?.y ?? box.y;
  const h = bar?.h ?? box.h;
  cursor.value = { x, y, h };

  if (props.follow) reveal(y, h);
}

/**
 * 播放头出视口时把谱滚过去。
 *
 * 这里的关键是**每帧只能有一个方向的结论**。一条系统比视口还高时（总谱几条轨道叠起来很常见），
 * 「顶端出界」和「底端出界」会同时成立：先按顶端滚 → 下一帧底端又出界 → 再按底端滚 →
 * 顶端又出界……于是每帧来回弹，看起来就是整页在抖。系统比视口高时只跟系统顶端。
 */
function reveal(y: number, h: number): void {
  const el = scroller.value;
  if (!el) return;
  const margin = 24;
  const top = el.scrollTop;
  const viewH = el.clientHeight;

  // 整条系统装不下：只保证「这条系统的顶端」在视口里，剩下的让用户自己滚。
  // 顶端已经露在上三分之一以内就不动，避免在每个拍上反复微调。
  if (h + margin * 2 >= viewH) {
    const head = y - margin;
    if (head < top || head > top + viewH * 0.34) el.scrollTop = Math.max(0, head);
    return;
  }

  // 装得下：两个条件互斥（能一次塞进视口就不会同时两边都出界），不会来回弹
  if (y - margin < top) el.scrollTop = Math.max(0, y - margin);
  else if (y + h + margin > top + viewH) el.scrollTop = Math.max(0, y + h + margin - viewH);
}

let resizeTimer: number | null = null;

onMounted(() => {
  void nextTick(() => void render());
});

onUnmounted(() => {
  disposed = true;
  if (renderTimer !== null) window.clearTimeout(renderTimer);
  if (resizeTimer !== null) window.clearTimeout(resizeTimer);
  teardown();
});

watch(() => project.project, () => scheduleRender(), { deep: false });
watch(renderKey, () => scheduleRender(0));
watch(() => transport.positionBeats, updateCursor);
// 主题变了：颜色是渲染资源，得整块重排一次。
watch(isDark, () => scheduleRender(0));
</script>

<template>
  <div class="score-canvas-root">
    <div ref="scroller" class="score-body">
      <div class="score-canvas" @click="onSeek" @dblclick="emit('edit-notation')">
        <div ref="host" class="score-host" />
        <div
          v-if="barHighlight"
          class="bar-highlight"
          :style="{
            left: `${barHighlight.x}px`,
            top: `${barHighlight.y}px`,
            width: `${barHighlight.w}px`,
            height: `${barHighlight.h}px`,
          }"
        />
        <div
          v-if="cursor"
          class="score-cursor"
          :style="{ left: `${cursor.x}px`, top: `${cursor.y}px`, height: `${cursor.h}px` }"
        />
        <p v-if="error" class="score-empty">{{ error }}</p>
      </div>
    </div>
    <span v-if="busy" class="score-busy">{{ i18n.t("score.rendering") }}</span>
  </div>
</template>

<style scoped>
.score-canvas-root {
  position: relative;
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.score-body {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
  background-color: var(--app-bg);
}
.score-canvas {
  position: relative;
}
.score-host {
  position: relative;
}
/* 当前小节：淡底 */
.bar-highlight {
  position: absolute;
  border-radius: 3px;
  background-color: rgb(var(--md-primary) / 0.09);
  pointer-events: none;
}
/* 播放光标：一条竖线 */
.score-cursor {
  position: absolute;
  width: 2px;
  border-radius: 1px;
  background-color: rgb(var(--md-primary));
  opacity: 0.9;
  pointer-events: none;
}
.score-empty {
  margin: 24px 0;
  text-align: center;
  font-size: 12.5px;
  color: rgb(var(--md-on-surface-muted));
}
.score-busy {
  position: absolute;
  right: 14px;
  top: 8px;
  padding: 3px 8px;
  border-radius: 999px;
  font-size: 10.5px;
  background-color: rgb(var(--md-surface-container-high) / 0.92);
  color: rgb(var(--md-on-surface-muted));
  pointer-events: none;
}
</style>
