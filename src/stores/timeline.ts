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
 * - `snap = meterBeat / 网格等分数`：吸附网格。等分数来自 `layout.noteGrid`（默认 1/16 → 4 等分），
 *   4/4 下即 0.25（十六分音符），6/8 下 0.125。换拍号时步长跟着变，换网格时整条时间轴一起变——
 *   钢琴卷帘与编排视图共用这一份网格，所以两边的吸附手感永远一致。
 *
 * 可见格线只画到「拍」：一个小节 = 分子个格子，小节线再叠一层。
 * 吸附用的细分（`snap` = 拍/4，4/4 是十六分音符、6/8 是 32 分音符）细线**不画**——
 * 画出来会让一个小节看起来有很多格，跟「一个小节几拍」对不上。
 *
 * 所以**切换拍号会同时改变两个视图的网格格子与吸附精度**——这正是它该有的行为。
 */
import { defineStore } from "pinia";
import { computed, ref, watch } from "vue";
import { useProjectStore } from "@/stores/project";
import { gridStepFor, useLayoutStore } from "@/stores/layout";
import {
  barBeatsAt as tempoBarBeatsAt,
  meterBeatAt as tempoMeterBeatAt,
  segments as tempoSegments,
} from "@/utils/tempo";

const KEY = "ai-daw-timeline";

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
  const layout = useLayoutStore();

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
  /**
   * 吸附 / 细分网格（四分音符数）：由「一拍」按当前网格等分。
   *
   * 自由模式没有「一格」可言，但键盘步进（左右方向键）仍需要一个有意义的步长，
   * 所以这里回落到 1/16（此前的固定值）；真正的吸附判断见 `gridStepAt`。
   */
  const snap = computed(() => gridStepFor(layout.noteGrid, meterBeat.value) ?? meterBeat.value / 4);

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

  /** 每小节像素：标尺与网格的小节宽度都用它。 */
  const barPx = computed(() => `${zoom.value * barBeats.value}px`);

  // ---- 分段（速度/拍号关键点） ----

  /** 拍号分段（用于网格与刻度）。第 0 段来自基准字段。 */
  const meterSegments = computed(() =>
    project.project
      ? tempoSegments(project.project.transport)
      : [{ startBeat: 0, endBeat: Infinity, tempo: 120, num: 4, den: 4 }],
  );

  /** 某拍处的拍号长度（一小节多少四分音符）。 */
  function barBeatsAt(beat: number): number {
    return project.project ? tempoBarBeatsAt(beat, project.project.transport) : 4;
  }

  /** 某拍处「一拍」的长度（四分音符数）：拍号关键点切换时跟着变。 */
  function beatLengthAt(beat: number): number {
    return project.project ? tempoMeterBeatAt(beat, project.project.transport) : 1;
  }

  /**
   * 某拍处的一格（四分音符数）；自由模式 = `null`（不吸附）。
   *
   * 「一拍」的长度按该处的拍号算（关键点换拍号时步长跟着变），等分数取自 `layout.noteGrid`。
   */
  function gridStepAt(beat: number): number | null {
    return gridStepFor(layout.noteGrid, beatLengthAt(beat));
  }

  /** 某拍处的吸附步长（四分音符数）；自由模式回落到 1/16（只给键盘步进用）。 */
  function snapAt(beat: number): number {
    const meter = beatLengthAt(beat);
    return gridStepFor(layout.noteGrid, meter) ?? meter / 4;
  }

  /** 小节刻度（跨分段累计小节号）。 */
  const barMarks = computed(() => {
    const z = zoom.value;
    const out: { bar: number; beat: number; width: number }[] = [];
    let bar = 1;
    for (const s of meterSegments.value) {
      const barBeats = s.num * (4 / Math.max(1, s.den));
      const end = Number.isFinite(s.endBeat) ? s.endBeat : totalBeats.value;
      let b = s.startBeat;
      while (b < end - 1e-9) {
        const w = Math.min(barBeats, end - b);
        out.push({ bar, beat: b, width: w * z });
        bar += 1;
        b += barBeats;
      }
    }
    return out;
  });

  /**
   * 分段叠层的网格样式。
   *
   * 每段一对层：拍线格（每拍一格）+ 小节线；用 `background-size` 把 repeating 渐变限制在该段内，
   * 于是不同拍号的段落各自成立。无关键点时只有一段，等价于「整首一套网格」。
   *
   * **顺序有讲究**：CSS 把 `background-image` 里**靠前**的那层画在最上面，同一段里小节线
   * 必须排在拍线前面（小节线压在拍线上），否则会被更淡的拍线盖掉。
   */
  const gridStyle = computed(() => {
    const z = zoom.value;
    const images: string[] = [];
    const sizes: string[] = [];
    const positions: string[] = [];
    const repeats: string[] = [];
    for (const s of meterSegments.value) {
      const startPx = s.startBeat * z;
      const widthPx = Number.isFinite(s.endBeat) ? (s.endBeat - s.startBeat) * z : contentWidth.value;
      const cell = Math.max(2, z * (4 / Math.max(1, s.den)));
      const bar = Math.max(2, z * (s.num * (4 / Math.max(1, s.den))));
      images.push(`repeating-linear-gradient(to right, var(--grid-bar) 0 1px, transparent 1px ${bar}px)`);
      positions.push(`${startPx}px 0`);
      sizes.push(`${Math.max(1, widthPx)}px 100%`);
      repeats.push("no-repeat");
      images.push(`repeating-linear-gradient(to right, var(--grid-line) 0 1px, transparent 1px ${cell}px)`);
      positions.push(`${startPx}px 0`);
      sizes.push(`${Math.max(1, widthPx)}px 100%`);
      repeats.push("no-repeat");
    }
    return {
      backgroundImage: images.join(", "),
      backgroundPosition: positions.join(", "),
      backgroundSize: sizes.join(", "),
      backgroundRepeat: repeats.join(", "),
    };
  });

  /** 吸附到网格（按所在分段的拍长与当前网格）；自由模式原样返回（只夹到 ≥ 0）。 */
  function snapBeats(beat: number): number {
    if (!Number.isFinite(beat)) return 0;
    const s = gridStepAt(beat);
    if (s === null) return Math.max(0, beat);
    return Math.max(0, Math.round(beat / s) * s);
  }

  /** 有符号吸附：拖动增量可以是负数，所以不夹 0；自由模式原样返回。 */
  function snapDelta(value: number): number {
    if (!Number.isFinite(value)) return 0;
    const s = gridStepAt(value);
    if (s === null) return value;
    return Math.round(value / s) * s;
  }

  /**
   * 小节容差（拍）。
   *
   * 播放范围以**帧**为单位存在引擎里，界面算小节号前要先把帧换算回拍，一来一回会引入最多
   * **半个采样帧**的量化误差（48 kHz / 300 BPM 下约 5e-5 拍）。旧代码用固定 1e-6 吸不住
   * 这个误差——「从第 2 小节开头」划的范围，起点会被算回第 1 小节（显示「第 1–3 小节」）。
   * 1e-3 拍在 120 BPM 下只有 0.5ms，远小于任何鼠标拖动的精度，不会把真实偏移误判成小节线。
   */
  const BAR_EPS = 1e-3;

  /**
   * 拍 → 小节号（1 起）。**本身不带容差**：`mark.beat <= beat` 即算这一小节。
   *
   * 容差由调用方按方向施加（见 `barRange`）——`barAt` 自己带容差会和 `barRange` 给 end
   * 减的容差相互抵消（`x - eps + eps` 在浮点上可能回到 `x`），正好落在小节线上的 end
   * 又会被算进下一小节（「第 2–3 小节」显示成「第 2–4」）。
   *
   * 直接扫 `barMarks`（标尺已经按速度 / 拍号分段累计好的小节刻度），所以换拍号、跨关键点
   * 都准；比「按当前位置的小节长取整」可靠。
   */
  function barAt(beat: number): number {
    let bar = 1;
    for (const mark of barMarks.value) {
      if (mark.beat <= beat) bar = mark.bar;
      else break;
    }
    return bar;
  }

  /**
   * 一段拍区间覆盖的小节号范围。
   *
   * - `first`：起点**加**一个容差再判——帧量化可能把「第 2 小节开头」压到第 1 小节里去；
   * - `last`：终点按**开区间**算（正好落在小节线上时归上一小节，否则「第 2–3 小节」会显示成
   *   「第 2–4」），减容差的方向与起点相反，两边各自吸收量化误差。
   */
  function barRange(startBeat: number, endBeat: number): { first: number; last: number } {
    const first = barAt(startBeat + BAR_EPS);
    return { first, last: barAt(Math.max(startBeat + BAR_EPS, endBeat - BAR_EPS)) };
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
    barPx,
    meterSegments,
    barMarks,
    barBeatsAt,
    beatLengthAt,
    gridStepAt,
    snapAt,
    gridStyle,
    snapBeats,
    snapDelta,
    barAt,
    barRange,
    beatsAt,
    zoomAt,
    centerOn,
  };
});
