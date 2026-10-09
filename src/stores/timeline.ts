/**
 * 共享时间轴：**编排视图与编辑视图用的是同一条时间轴**。
 *
 * 三件事必须共用，否则两个视图看起来就是两条不同的时间线：
 * 1. `zoom` —— 每拍像素；
 * 2. `scrollLeft` —— 横向滚动位置（从工程起点算起，不是从片段起点）；
 * 3. 网格 —— 小节线、拍线、吸附精度，全部由**拍号**推导。
 *
 * 网格推导（都以四分音符为「拍」的基准单位）：
 * - `meterBeat = 4 / 分母`：拍号里那一「拍」有多长。4/4 → 1，6/8 → 0.5；
 * - `barBeats = 分子 × meterBeat`：一小节多长。4/4 → 4，3/4 → 3，6/8 → 3，2/2 → 4；
 * - `snap = meterBeat / 4`：吸附网格。4/4 → 0.25（十六分音符），6/8 → 0.125。
 *
 * 可见格线只画到「拍」：一个小节 = 分子个格子，小节线再叠一层。
 * 吸附用的 1/16 细线**不画**——画出来会让一个小节看起来有很多格，跟「一个小节几拍」对不上。
 *
 * 所以**切换拍号会同时改变两个视图的网格格子与吸附精度**——这正是它该有的行为。
 */
import { defineStore } from "pinia";
import { computed, ref, watch } from "vue";
import { useProjectStore } from "@/stores/project";

const KEY = "daw-party-timeline";

/** 每拍像素的允许范围。 */
export const MIN_ZOOM = 10;
export const MAX_ZOOM = 320;
const DEFAULT_ZOOM = 56;

/** 每拍像素的默认值（新用户第一次打开时）。 */
export function defaultZoom(): number {
  return DEFAULT_ZOOM;
}

function clampZoom(z: number): number {
  if (!Number.isFinite(z)) return DEFAULT_ZOOM;
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z));
}

function load(): { zoom?: number } {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "{}") as { zoom?: number };
  } catch {
    return {};
  }
}

export const useTimelineStore = defineStore("timeline", () => {
  const project = useProjectStore();

  const zoom = ref(clampZoom(load().zoom ?? DEFAULT_ZOOM));
  /** 横向滚动位置（px，从工程起点算起）。两个视图共用，所以切换视图时视角不变。 */
  const scrollLeft = ref(0);

  watch(zoom, () => {
    localStorage.setItem(KEY, JSON.stringify({ zoom: zoom.value }));
  });

  const sigNum = computed(() => Math.max(1, project.project?.transport.time_sig_num ?? 4));
  const sigDen = computed(() => Math.max(1, project.project?.transport.time_sig_den ?? 4));

  /** 拍号里那一「拍」等于多少个四分音符。 */
  const meterBeat = computed(() => 4 / sigDen.value);
  /** 一小节的长度（四分音符数）。 */
  const barBeats = computed(() => sigNum.value * meterBeat.value);
  /** 吸附 / 细分网格（四分音符数）。 */
  const snap = computed(() => meterBeat.value / 4);

  /** 工程内容长度（四分音符）。 */
  const contentBeats = computed(() => {
    const clips = (project.project?.tracks ?? []).flatMap((t) => t.clips);
    return clips.reduce((m, c) => Math.max(m, c.start + c.length), 0);
  });

  /** 内容占的小节数（至少 4 小节，再加 2 小节余量）。 */
  const contentBars = computed(() => {
    const bar = barBeats.value;
    return Math.max(4, Math.ceil(contentBeats.value / bar) + 2);
  });

  /** 标尺与网格多画一段，保证滚动 / 拉伸时右侧仍有刻度。 */
  const totalBars = computed(() => contentBars.value + 24);
  /** 时间轴总长度（四分音符）。 */
  const totalBeats = computed(() => totalBars.value * barBeats.value);

  /** 内容宽度（px），供滚动容器与网格使用。 */
  const contentWidth = computed(() => Math.max(1, totalBeats.value * zoom.value));
  /** 宽度样式：至少铺满可视区，避免右侧出现空白。 */
  const widthStyle = computed(() => ({ width: `max(100%, ${contentWidth.value}px)` }));

  /** 每格像素：吸附细分（`snapPx`，不画线）/ 拍线 / 小节线。 */
  const snapPx = computed(() => `${zoom.value * snap.value}px`);
  const beatPx = computed(() => `${zoom.value * meterBeat.value}px`);
  const barPx = computed(() => `${zoom.value * barBeats.value}px`);

  /**
   * 网格背景（编排与卷帘共用同一串）。
   *
   * 只画两层：**拍线**（一个小节 = 分子个格子）+ **小节线**。
   * 不再叠固定的 1/16 细线——那会让一个小节看起来有很多格，和「一个小节几拍」对不上。
   * 吸附精度（`snap`）不受影响：拖动仍按 1/16 拍吸附，只是格线更干净。
   */
  const gridBackground = computed(() => {
    const layers: string[] = [];
    layers.push(
      `repeating-linear-gradient(to right, var(--grid-line) 0 1px, transparent 1px ${zoom.value * meterBeat.value}px)`,
    );
    layers.push(
      `repeating-linear-gradient(to right, var(--grid-bar) 0 1px, transparent 1px ${zoom.value * barBeats.value}px)`,
    );
    return layers.join(", ");
  });

  /** 吸附到网格。 */
  function snapBeats(beat: number): number {
    if (!Number.isFinite(beat)) return 0;
    const s = snap.value;
    return Math.max(0, Math.round(beat / s) * s);
  }

  /** 指针在内容里的 x（px）→ 拍。 */
  function beatsAt(contentPx: number): number {
    return contentPx / zoom.value;
  }

  /**
   * 以视口内某个 x 为锚点缩放（Ctrl/Cmd + 滚轮）：锚点下的那一拍保持不动。
   * 返回新的 `scrollLeft`，调用方写回滚动容器即可。
   *
   * `originPx` 是调用方在工程起点左侧预留的空白宽度（编排视图用它防止 0 拍贴边）。
   * 传 0 表示「视口左边缘就是工程起点」（编辑视图如此）。
   */
  function zoomAt(viewportX: number, factor: number, originPx = 0): number {
    const anchorBeat = Math.max(0, (viewportX - originPx + scrollLeft.value) / zoom.value);
    const next = clampZoom(zoom.value * factor);
    zoom.value = next;
    const nextScroll = Math.max(0, anchorBeat * next + originPx - viewportX);
    scrollLeft.value = nextScroll;
    return nextScroll;
  }

  /** 把某个拍位置滚到视口中间（播放头跟随等场景用）。 */
  function centerOn(beats: number, viewportWidth: number): void {
    scrollLeft.value = Math.max(0, beats * zoom.value - viewportWidth / 2);
  }

  return {
    zoom,
    scrollLeft,
    sigNum,
    sigDen,
    meterBeat,
    barBeats,
    snap,
    contentBeats,
    contentBars,
    totalBars,
    totalBeats,
    contentWidth,
    widthStyle,
    snapPx,
    beatPx,
    barPx,
    gridBackground,
    snapBeats,
    beatsAt,
    zoomAt,
    centerOn,
  };
});
