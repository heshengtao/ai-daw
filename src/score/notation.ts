/**
 * 记谱量化：把「拍」为单位的浮点音符，变成能画上五线谱的东西。
 *
 * 钢琴卷帘里的音符是连续的浮点数，而谱面只接受有限种时值符号（全音符 … 64 分音符、附点、
 * 三连音）。这里负责三件事：
 *
 * 1. **小节网格**：按速度 / 拍号关键点切出小节。关键点如果落在小节中间，当前小节记成「残小节」，
 *    而不是把小节线挪到关键点上——宁可谱面难看，也不能让时间轴和听到的东西错位。
 * 2. **可记谱化**：起点 / 终点吸附到网格（同时认二连与三连），时值拆成有限的符号串；跨小节
 *    的地方拆开，用连音线接起来。
 * 3. **声部**：同轨同时发声的音符分到 1..4 个声部，每个声部补满休止符——谱面要求每小节每个
 *    声部都是「填满」的，少一个休止符，后面的音符就会整体左移。
 *
 * 这个模块**不依赖渲染器**：输出是纯数据，`musicxml.ts` 只做序列化。换渲染器（OSMD / Verovio /
 * 交给 MuseScore）都不用改这里，也方便单独跑自检（见 `tools/score-selftest.ts`）。
 */
import type { Note, Track, Transport } from "@/api/types";

/** 每四分音符的 tick 数。960 覆盖三 / 五 / 六连音；七连音需要 3360。 */
export const DIVISIONS = 960;

/** 浮点比较容差（拍）。1/960 拍约 0.001，这里更严一点。 */
const EPS = 1e-6;

/** 一个声部表最多容纳的声部数。超出后剩余音符会并进最早的声部并截断。 */
const MAX_VOICES = 4;

export type NoteType = "whole" | "half" | "quarter" | "eighth" | "16th" | "32nd" | "64th";

/** 一个能画出来的时值符号。 */
export interface DurationSymbol {
  /** MusicXML 的 `<type>`。 */
  type: NoteType;
  /** 附点个数（0 = 无附点）。 */
  dots: number;
  /** 时值（四分音符 = 1）。 */
  beats: number;
  /** 三连音（需要 `<time-modification>3:2`）。 */
  triplet: boolean;
}

const BASE: { type: NoteType; beats: number }[] = [
  { type: "whole", beats: 4 },
  { type: "half", beats: 2 },
  { type: "quarter", beats: 1 },
  { type: "eighth", beats: 0.5 },
  { type: "16th", beats: 0.25 },
  { type: "32nd", beats: 0.125 },
  { type: "64th", beats: 0.0625 },
];

/**
 * 时值表：每个基础时值 × 三种附点 ×（二连 / 三连）。
 *
 * 排序规则是「长的优先、二连优先」：分解时先挑大的，遇到等长的先挑二连（三连音在谱面上
 * 更"贵"，不该抢二连的位置）。
 */
const SYMBOLS: DurationSymbol[] = (() => {
  const out: DurationSymbol[] = [];
  for (const base of BASE) {
    for (const dots of [0, 1, 2]) {
      const mul = dots === 0 ? 1 : dots === 1 ? 1.5 : 1.75;
      out.push({ type: base.type, dots, beats: base.beats * mul, triplet: false });
      out.push({ type: base.type, dots, beats: (base.beats * mul * 2) / 3, triplet: true });
    }
  }
  return out.sort((a, b) => b.beats - a.beats || Number(a.triplet) - Number(b.triplet));
})();

/** 最小的可记谱时值（64 分音符）。 */
export const MIN_DURATION = 1 / 16;

// ---------------------------------------------------------------------------
// 时值：精确匹配 / 贪婪分解
// ---------------------------------------------------------------------------

/** 时值与符号**完全相等**（容差随长度放大，免得 2/3 这种循环小数判不出来）。 */
export function exactSymbol(beats: number): DurationSymbol | null {
  const tol = Math.max(EPS, Math.abs(beats) * 1e-4);
  for (const s of SYMBOLS) {
    if (Math.abs(s.beats - beats) <= tol) return s;
  }
  return null;
}

/**
 * 取最接近的时值符号。
 *
 * 序列化时要靠它决定 `<type>` / 附点 / 三连音标记。正常路径上每个事件的时值都来自
 * `decompose`，一定能精确命中；这里的兜底是给「碎屑并进上一个符号」那种边角情况用的。
 */
export function symbolFor(beats: number): DurationSymbol {
  const exact = exactSymbol(beats);
  if (exact) return exact;
  let best = SYMBOLS[0]!;
  for (const s of SYMBOLS) {
    if (Math.abs(s.beats - beats) < Math.abs(best.beats - beats)) best = s;
  }
  return best;
}

/**
 * 一个时值属于哪个「节奏家族」：三连还是二连。
 *
 * 这一步是为了让 4/3 拍（两个三连四分）分解成 `三连二分` 一个符号，而不是
 * `四分 + 三连八分` 两个——后者谱面上看起来毫无三连感。
 */
function family(beats: number): "triplet" | "duple" {
  const tri = Math.abs(beats * 3 - Math.round(beats * 3));
  const dup = Math.abs(beats * 2 - Math.round(beats * 2));
  return tri < 1e-4 && dup > 1e-4 ? "triplet" : "duple";
}

/**
 * 把一个时值拆成若干能画出来的符号（长音在前）。
 *
 * 返回的符号时值之和**等于**入参（除非入参小于 64 分音符，那时会返回空数组，由调用方兜底）。
 */
export function decompose(beats: number): DurationSymbol[] {
  if (!(beats > EPS)) return [];
  const exact = exactSymbol(beats);
  if (exact) return [exact];

  const fam = family(beats);
  const preferred = SYMBOLS.filter((s) => (fam === "triplet" ? s.triplet : !s.triplet));
  const out: DurationSymbol[] = [];
  let left = beats;
  let guard = 0;
  while (left > EPS && guard < 64) {
    guard += 1;
    const rest = exactSymbol(left);
    if (rest) {
      out.push(rest);
      left = 0;
      break;
    }
    const pool = preferred.some((s) => s.beats <= left + EPS) ? preferred : SYMBOLS;
    const fit = pool.find((s) => s.beats <= left + EPS);
    if (!fit) break;
    out.push(fit);
    left -= fit.beats;
  }
  // 剩下的碎屑（小于 64 分音符）并进最后一个符号：宁可长一点，也不要凭空多一个音。
  if (left > EPS && out.length > 0) {
    const last = out[out.length - 1]!;
    out[out.length - 1] = { ...last, beats: last.beats + left };
  }
  return out;
}

/**
 * 把拍位置吸附到网格上。
 *
 * 同时认二连网格和三连网格（`grid * 2/3`），但三连要**明显更近**才采用：手弹的 1/16 不能
 * 被吸到 1/24 上去。`grid <= 0` 表示不量化，原样返回。
 */
export function quantizeBeat(value: number, grid: number): number {
  if (!(grid > 0)) return value;
  const duple = Math.round(value / grid) * grid;
  const triGrid = (grid * 2) / 3;
  const triple = Math.round(value / triGrid) * triGrid;
  const dd = Math.abs(value - duple);
  const dt = Math.abs(value - triple);
  return dt + EPS < dd * 0.6 ? triple : duple;
}

// ---------------------------------------------------------------------------
// 小节网格
// ---------------------------------------------------------------------------

/** 拍号 / 速度的连续段（与 `utils/tempo.ts` 同源，但这里要的是「段」而不是换算）。 */
export interface MeterSegment {
  start: number;
  end: number;
  num: number;
  den: number;
  tempo: number;
}

/** 按关键点切段（第 0 段来自基准字段，和 `utils/tempo.ts` 的规则一致）。 */
export function meterSegments(t: Transport): MeterSegment[] {
  const kfs = [...(t.keyframes ?? [])].sort((a, b) => a.beat - b.beat);
  const out: MeterSegment[] = [];
  let start = 0;
  let num = t.time_sig_num || 4;
  let den = t.time_sig_den || 4;
  let tempo = t.tempo || 120;
  for (const kf of kfs) {
    if (kf.beat <= start) {
      num = kf.time_sig_num || num;
      den = kf.time_sig_den || den;
      tempo = kf.tempo || tempo;
      continue;
    }
    out.push({ start, end: kf.beat, num, den, tempo });
    start = kf.beat;
    num = kf.time_sig_num || num;
    den = kf.time_sig_den || den;
    tempo = kf.tempo || tempo;
  }
  out.push({ start, end: Number.POSITIVE_INFINITY, num, den, tempo });
  return out;
}

/** 一个小节。 */
export interface Measure {
  /** 0 起的序号。 */
  index: number;
  /** 起始拍。 */
  start: number;
  /** 长度（拍）；换拍号落在小节中间时会短于整小节。 */
  beats: number;
  num: number;
  den: number;
  /** 不是完整小节：换拍号 / 换速点落在小节中间，或曲尾截断。 */
  partial: boolean;
}

/**
 * 切出覆盖到 `endBeat` 的小节表。
 *
 * 拍号变化点如果不在小节线上，**当前小节就地截断**（`partial = true`），下一小节按新拍号起。
 * 这是 MusicXML 允许的「短小节」，MuseScore / OSMD 都能正常显示。
 */
export function buildMeasures(t: Transport, endBeat: number): Measure[] {
  const out: Measure[] = [];
  const limitAll = Math.max(endBeat, 0);
  let beat = 0;

  for (const seg of meterSegments(t)) {
    const barBeats = seg.num * (4 / Math.max(1, seg.den));
    if (!(barBeats > EPS)) continue;
    if (beat >= limitAll - EPS) break;

    // 完整小节：只要小节**起点**还在 endBeat 之内就收下，哪怕它跨过 endBeat——
    // 小节必须是完整的，否则最后一小节会短一截，谱面直接错位。
    while (beat < limitAll - EPS && beat + barBeats <= seg.end + EPS) {
      out.push({ index: out.length, start: beat, beats: barBeats, num: seg.num, den: seg.den, partial: false });
      beat += barBeats;
      if (out.length > 4096) break;
    }

    // 拍号变化点不在小节线上：这一段剩下的部分记成残小节，下一段从新拍号起。
    if (beat < Math.min(seg.end, limitAll) - EPS) {
      const end = Math.min(seg.end, limitAll);
      out.push({ index: out.length, start: beat, beats: end - beat, num: seg.num, den: seg.den, partial: true });
      beat = end;
    }
    if (beat >= limitAll - EPS) break;
  }

  if (out.length === 0) {
    out.push({ index: 0, start: 0, beats: 4, num: t.time_sig_num || 4, den: t.time_sig_den || 4, partial: false });
  }
  return out;
}

/** 某个拍落在哪个小节（二分；越界时贴到首 / 尾）。 */
export function measureIndexAt(beat: number, measures: Measure[]): number {
  let lo = 0;
  let hi = measures.length - 1;
  let ans = 0;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const m = measures[mid]!;
    if (beat + EPS >= m.start) {
      ans = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return ans;
}

// ---------------------------------------------------------------------------
// 音符 → 事件
// ---------------------------------------------------------------------------

/** 谱面上的一个事件（音符或休止符）。 */
export interface NotatedEvent {
  /** 绝对拍。 */
  start: number;
  /** 时值（拍）。 */
  beats: number;
  /** 声部（1 起）。 */
  voice: number;
  /** 音高；休止符为 0。 */
  pitch: number;
  rest: boolean;
  /** 连音线状态：只有被小节线拆开的音才有。 */
  tie: "start" | "stop" | "both" | null;
  /** 源音符；休止符为 null。 */
  note: Note | null;
  /** 连音线链上的第几段（0 起）/ 共几段。1 段表示没被拆。 */
  chainIndex: number;
  chainLength: number;
}

/** 一条轨道的记谱结果。 */
export interface NotatedTrack {
  track: Track;
  /** 与 `measures` 等长，每项是该小节的事件（已按 声部 → 起点 → 音高 排序）。 */
  bars: NotatedEvent[][];
  /** 用来选谱号的音高集合。 */
  pitches: number[];
  warnings: string[];
}

export interface NotationOptions {
  /** 量化网格（拍）；0 = 不量化。 */
  grid: number;
}

interface Raw {
  start: number;
  end: number;
  pitch: number;
  note: Note;
  voice: number;
}

/**
 * 一条轨道 → 每小节的事件表。
 *
 * 输入是这个轨道所有片段的音符（片段起点会加进去），输出已经补好休止符、拆好连音线。
 */
export function notateTrack(
  track: Track,
  measures: Measure[],
  opts: NotationOptions,
): NotatedTrack {
  const warnings: string[] = [];
  const grid = opts.grid;
  const minUnit = grid > 0 ? grid : MIN_DURATION;

  // ---- 1. 绝对化 + 量化 ----
  // 只出片段时 `measures` 是裁过的：范围外的音符必须丢掉，否则 `measureIndexAt` 会把它们
  // 夹到第一小节上（谱面凭空多出一堆音）。
  const spanStart = measures[0]?.start ?? 0;
  const lastMeasure = measures[measures.length - 1];
  const spanEnd = lastMeasure ? lastMeasure.start + lastMeasure.beats : 0;

  const raw: Raw[] = [];
  for (const clip of track.clips) {
    for (const note of clip.notes) {
      const abs = clip.start + note.start;
      if (!Number.isFinite(abs) || !Number.isFinite(note.duration)) continue;
      if (abs < spanStart - EPS || abs >= spanEnd - EPS) continue;
      let start = grid > 0 ? quantizeBeat(Math.max(0, abs), grid) : Math.max(0, abs);
      let end = grid > 0 ? quantizeBeat(abs + note.duration, grid) : abs + note.duration;
      if (end <= start + EPS) end = start + minUnit;
      const pitch = Math.max(0, Math.min(127, Math.round(note.pitch)));
      raw.push({ start, end, pitch, note, voice: 1 });
    }
  }
  raw.sort((a, b) => a.start - b.start || b.pitch - a.pitch || a.end - b.end);

  // 空轨道不提前返回：后面的「补休止」会给每个小节填一个整小节休止符——总谱里空轨道也必须
  // 占着自己的谱表，否则上下对不齐。

  // ---- 2. 声部：同起点的音（和弦）一定同声部；重叠才另开一个 ----
  const voiceEnd: number[] = [];
  for (let i = 0; i < raw.length; ) {
    let j = i;
    while (j + 1 < raw.length && Math.abs(raw[j + 1]!.start - raw[i]!.start) < EPS) j += 1;
    const group = raw.slice(i, j + 1);
    const end = Math.max(...group.map((g) => g.end));
    let v = voiceEnd.findIndex((e) => e <= group[0]!.start + EPS);
    if (v < 0) {
      if (voiceEnd.length < MAX_VOICES) {
        v = voiceEnd.length;
      } else {
        // 四个声部都占着：挤进最早空出来的那个，稍后会被截断。
        v = 0;
        for (let k = 1; k < voiceEnd.length; k += 1) if (voiceEnd[k]! < voiceEnd[v]!) v = k;
        warnings.push("同时发声音符超过 4 个声部，部分音符被截短");
      }
    }
    voiceEnd[v] = end;
    for (const g of group) g.voice = v + 1;
    i = j + 1;
  }

  // ---- 3. 同一 onset 组之间如果还重叠，截短前一组（谱面画不出来） ----
  const voiceCount = Math.max(1, voiceEnd.length);
  for (let v = 1; v <= voiceCount; v += 1) {
    const list = raw.filter((r) => r.voice === v);
    let prevGroup: Raw[] | null = null;
    for (let i = 0; i < list.length; ) {
      let j = i;
      while (j + 1 < list.length && Math.abs(list[j + 1]!.start - list[i]!.start) < EPS) j += 1;
      const group = list.slice(i, j + 1);
      if (prevGroup) {
        const prevEnd = Math.max(...prevGroup.map((g) => g.end));
        const start = group[0]!.start;
        if (prevEnd > start + EPS) {
          for (const g of prevGroup) g.end = Math.min(g.end, start);
          warnings.push("同声部音符重叠，已按下一个音的起点截短");
        }
      }
      prevGroup = group;
      i = j + 1;
    }
  }

  // ---- 4. 按小节线切段，每段再拆成可记谱时值 ----
  const bars: NotatedEvent[][] = measures.map(() => []);
  const pitches: number[] = [];

  for (const r of raw) {
    if (!(r.end > r.start + EPS)) continue;
    const pieces: { start: number; beats: number }[] = [];
    let cursor = r.start;
    let guard = 0;
    while (cursor < r.end - EPS && guard < 512) {
      guard += 1;
      const m = measures[measureIndexAt(cursor, measures)]!;
      const barEnd = m.start + m.beats;
      const segEnd = Math.min(r.end, barEnd);
      let segCursor = cursor;
      for (const sym of decompose(segEnd - cursor)) {
        pieces.push({ start: segCursor, beats: sym.beats });
        segCursor += sym.beats;
      }
      // 分解不尽（理论上不会）：硬跳过，避免死循环。
      cursor = Math.max(segEnd, segCursor);
    }
    if (pieces.length === 0) continue;

    const chainLength = pieces.length;
    for (let k = 0; k < pieces.length; k += 1) {
      const piece = pieces[k]!;
      const tie: NotatedEvent["tie"] =
        chainLength === 1 ? null : k === 0 ? "start" : k === chainLength - 1 ? "stop" : "both";
      const idx = measureIndexAt(piece.start, measures);
      bars[idx]!.push({
        start: piece.start,
        beats: piece.beats,
        voice: r.voice,
        pitch: r.pitch,
        rest: false,
        tie,
        note: r.note,
        chainIndex: k,
        chainLength,
      });
      pitches.push(r.pitch);
    }
  }

  // ---- 5. 补休止符：每小节、每个用到的声部都要「填满」 ----
  measures.forEach((m, i) => {
    const inBar = bars[i]!;
    const voices = new Set(inBar.map((e) => e.voice));
    if (voices.size === 0) voices.add(1);
    const barEnd = m.start + m.beats;

    for (const v of voices) {
      const list = inBar.filter((e) => e.voice === v && !e.rest).sort((a, b) => a.start - b.start || b.pitch - a.pitch);
      const rests: NotatedEvent[] = [];
      let cursor = m.start;
      const pushRest = (from: number, to: number): void => {
        const syms = decompose(to - from);
        let at = from;
        for (const s of syms) {
          rests.push({
            start: at,
            beats: s.beats,
            voice: v,
            pitch: 0,
            rest: true,
            tie: null,
            note: null,
            chainIndex: 0,
            chainLength: 1,
          });
          at += s.beats;
        }
      };

      for (const e of list) {
        if (e.start > cursor + EPS) pushRest(cursor, Math.min(e.start, barEnd));
        cursor = Math.max(cursor, e.start + e.beats);
      }
      if (cursor < barEnd - EPS) pushRest(cursor, barEnd);
      inBar.push(...rests);
    }

    inBar.sort((a, b) => a.voice - b.voice || a.start - b.start || b.pitch - a.pitch);
  });

  return { track, bars, pitches, warnings };
}
