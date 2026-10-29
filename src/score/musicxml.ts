/**
 * Project → MusicXML（partwise 4.0）。
 *
 * 为什么先做 MusicXML 而不是直接接某个渲染器：OSMD / Verovio / alphaTab / MuseScore **全都吃**
 * MusicXML。先把这一层写对，导出的文件随手丢进 MuseScore 就能看出谱面质量，再决定用哪个渲染器
 * 都不会返工。反过来先接渲染器，等于把「量化 / 声部 / 连音线」和「渲染器的排版假设」绑在一起调，
 * 出问题分不清是谁的锅。
 *
 * 这一层只做序列化：音符怎么切、怎么分声部、怎么补休止，全在 `notation.ts` 里。
 */
import type { Project, Track, Transport } from "@/api/types";
import { defaultNotationFor, tabSpecFor, type Notation } from "@/utils/notation";
import {
  DIVISIONS,
  buildMeasures,
  measureIndexAt,
  meterSegments,
  notateTrack,
  quantizeBeat,
  symbolFor,
  type Measure,
  type NotatedEvent,
  type NotatedTrack,
} from "./notation";

/**
 * 导出选项。
 *
 * `tracks` 是「总谱 / 分谱」的开关：不给 = 所有有音符的轨道（总谱），给了 = 只出这几条（分谱）。
 */
export interface ScoreOptions {
  tracks?: number[];
  /**
   * 只出这一段（绝对拍）。
   *
   * 下窗口的乐谱要「跟上面的片段对应」，所以只渲染当前片段所在的那几小节；小节网格从
   * **包含** `range.start` 的那一小节开始（不要求 `start` 正好压在小节线上），到覆盖
   * `range.end` 的小节为止。
   */
  range?: { start: number; end: number };
  /** 调号（升号为正，-7..7）。工程模型里没有调号，先由调用方指定；0 = C 大调。 */
  keyFifths: number;
  /** 量化网格（拍，四分音符 = 1）；0 = 不量化。默认 0.25（1/16）。 */
  grid: number;
  /** 是否写歌词（`note.lyric`）。 */
  lyrics: boolean;
  /** 是否写和弦标记（`project.harmony`）。 */
  harmony: boolean;
  /** 是否写段落排练号（`project.sections`）。 */
  sections: boolean;
  /**
   * 每条轨道用哪种谱式。总谱里**逐 part** 决定：吉他轨带六线谱、鼓轨用打击乐谱表，
   * 互不影响。不传则按乐器猜（见 `defaultNotationFor`）。
   */
  notationFor?: (track: Track) => Notation;
  /** 谱号覆盖：`G` 高音 / `F` 低音 / `C` 中音 / `percussion` 打击乐；`null` = 按音域自动选。 */
  clefFor?: (track: Track) => ClefKind | null;
  /** 调弦覆盖（低 → 高）；只在六线谱下有意义，`null` = 用标准调弦。 */
  tuningFor?: (track: Track) => number[] | null;
  /** 曲名；缺省用工程名。 */
  title?: string;
}

export interface ScoreResult {
  xml: string;
  /** 参与记谱的轨道数。0 表示没有可记谱的内容。 */
  parts: number;
  /**
   * 参与记谱的轨道 id，**顺序与 `<part>` 一致**。
   *
   * 渲染器（alphaTab）把 `<part>` 按顺序变成自己的 track，要逐轨改谱式 / 谱号 / 调弦
   * 就得靠这张表把两边对上，不能靠名字猜。
   */
  trackIds: number[];
  measures: number;
  /** 记进谱面的源音符数。 */
  notes: number;
  warnings: string[];
  /**
   * 每个小节的起始拍与长度。
   *
   * 谱面视图要靠它把播放头（拍）换算成谱面坐标——渲染器给的坐标是按小节走的，
   * 而播放头是连续的拍，中间必须有这张表。
   */
  barStarts: number[];
  barLengths: number[];
}

const DEFAULT_GRID = 0.25;

/** 谱号种类（与 MusicXML 的 `<sign>` 对应）。 */
export type ClefKind = "G" | "F" | "C" | "percussion";

/** 把可选项补成完整选项。 */
function resolve(project: Project, options: Partial<ScoreOptions>): ScoreOptions {
  return {
    tracks: options.tracks,
    range: options.range,
    keyFifths: Math.max(-7, Math.min(7, Math.round(options.keyFifths ?? 0))),
    grid: options.grid ?? DEFAULT_GRID,
    lyrics: options.lyrics ?? true,
    harmony: options.harmony ?? true,
    sections: options.sections ?? true,
    notationFor: options.notationFor,
    clefFor: options.clefFor,
    tuningFor: options.tuningFor,
    title: options.title ?? project.name,
  };
}

// ---------------------------------------------------------------------------
// 转义 / 拼装
// ---------------------------------------------------------------------------

/** XML 文本转义。 */
function esc(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;");
}

/** 音名 → `<step>` / `<alter>`。 */
const SHARP_STEPS: [string, number][] = [
  ["C", 0],
  ["C", 1],
  ["D", 0],
  ["D", 1],
  ["E", 0],
  ["F", 0],
  ["F", 1],
  ["G", 0],
  ["G", 1],
  ["A", 0],
  ["A", 1],
  ["B", 0],
];
const FLAT_STEPS: [string, number][] = [
  ["C", 0],
  ["D", -1],
  ["D", 0],
  ["E", -1],
  ["E", 0],
  ["F", 0],
  ["G", -1],
  ["G", 0],
  ["A", -1],
  ["A", 0],
  ["B", -1],
  ["B", 0],
];

/**
 * MIDI 音高 → 五线谱音名。
 *
 * 没有调号时按 C 大调处理（全用升号）；有降号调时改用降号拼写，否则谱面会出现
 * 「B♭ 写成 A♯」这种看着别扭、也没法跟和弦标记对上的写法。
 */
function spell(pitch: number, fifths: number): { step: string; alter: number; octave: number } {
  const table = fifths < 0 ? FLAT_STEPS : SHARP_STEPS;
  const [step, alter] = table[((pitch % 12) + 12) % 12]!;
  return { step, alter, octave: Math.floor(pitch / 12) - 1 };
}

function pitchXml(pitch: number, fifths: number): string {
  const { step, alter, octave } = spell(pitch, fifths);
  const alt = alter === 0 ? "" : `<alter>${alter}</alter>`;
  return `<pitch><step>${step}</step>${alt}<octave>${octave}</octave></pitch>`;
}

/**
 * 鼓组音高 → 五线谱位置（GM 打击乐映射）。
 *
 * 只覆盖常用的一套；没收录的音按「五个位置循环」硬排——不准确，但至少不会全挤在一条线上。
 * 正经做法是给轨道配一张可编辑的鼓映射表（见 ROADMAP），这一版先够看。
 */
const DRUM_MAP: Record<number, [string, number]> = {
  35: ["F", 4],
  36: ["F", 4],
  37: ["C", 5],
  38: ["C", 5],
  39: ["C", 5],
  40: ["C", 5],
  41: ["A", 4],
  42: ["G", 5],
  43: ["B", 4],
  44: ["G", 5],
  45: ["D", 5],
  46: ["G", 5],
  47: ["E", 5],
  48: ["E", 5],
  49: ["A", 5],
  50: ["E", 5],
  51: ["F", 5],
  52: ["A", 5],
  53: ["F", 5],
  54: ["C", 6],
  55: ["A", 5],
  56: ["D", 5],
  57: ["A", 5],
  58: ["E", 6],
  59: ["F", 5],
};

const DRUM_CYCLE: [string, number][] = [
  ["F", 4],
  ["A", 4],
  ["C", 5],
  ["E", 5],
  ["G", 5],
];

/** 轨道是不是鼓组（目前只看 `meta.role`）。 */
function unpitchedXml(pitch: number): string {
  const hit = DRUM_MAP[pitch] ?? DRUM_CYCLE[((pitch % DRUM_CYCLE.length) + DRUM_CYCLE.length) % DRUM_CYCLE.length]!;
  return `<unpitched><display-step>${hit[0]}</display-step><display-octave>${hit[1]}</display-octave></unpitched>`;
}

/**
 * 选谱号：按音域中位（优先 `meta.pitch_range`，否则用实际音符的中位）决定高音 / 低音谱号。
 *
 * 分界取 A3（MIDI 57）：低于它基本是大提琴 / 贝斯 / 低音区钢琴，用低音谱号更少加线。
 */
function clefOf(track: Track, pitches: number[]): "G" | "F" {
  const range = track.meta?.pitch_range;
  let mid: number;
  if (range) {
    mid = (range.min + range.max) / 2;
  } else if (pitches.length > 0) {
    const sorted = [...pitches].sort((a, b) => a - b);
    mid = sorted[Math.floor(sorted.length / 2)]!;
  } else {
    mid = 60;
  }
  return mid < 57 ? "F" : "G";
}

function clefXml(track: Track, pitches: number[], drum: boolean, tab: boolean, override: ClefKind | null): string {
  if (tab) return "<clef><sign>TAB</sign></clef>";
  if (override === "percussion" || (!override && drum)) return "<clef><sign>percussion</sign></clef>";
  const clef: ClefKind = override ?? clefOf(track, pitches);
  if (clef === "F") return "<clef><sign>F</sign><line>4</line></clef>";
  if (clef === "C") return "<clef><sign>C</sign><line>3</line></clef>";
  return "<clef><sign>G</sign><line>2</line></clef>";
}

/**
 * 六线谱的弦数与调弦。
 *
 * `staff-tuning` 的行号从**最下面那根弦**（最低音）开始编号，所以 `tuning` 按「低 → 高」给。
 */
function staffDetailsXml(tab: { strings: number; tuning: number[] }): string {
  const lines = tab.tuning
    .map((pitch, i) => {
      const { step, alter, octave } = spell(pitch, 0);
      const alt = alter === 0 ? "" : `<tuning-alter>${alter}</tuning-alter>`;
      return `<staff-tuning line="${i + 1}"><tuning-step>${step}</tuning-step>${alt}<tuning-octave>${octave}</tuning-octave></staff-tuning>`;
    })
    .join("");
  return `<staff-details><staff-lines>${tab.strings}</staff-lines>${lines}</staff-details>`;
}

/** 音高 → 弦 / 品：品数最小优先（同品数时选更粗的弦）。弦号 1 = 最细的那根。 */
function tabPosition(pitch: number, tuning: number[]): { string: number; fret: number } {
  let best: { string: number; fret: number } | null = null;
  for (let i = 0; i < tuning.length; i += 1) {
    const fret = pitch - tuning[i]!;
    if (fret < 0 || fret > 24) continue;
    if (!best || fret < best.fret) best = { string: tuning.length - i, fret };
  }
  return best ?? { string: tuning.length, fret: 0 };
}

/**
 * 一个谱表里所有音的弦/品分配（按「同一起点 = 和弦」分组）。
 *
 * **必须成组算**：每个音各自挑「品数最小的弦」会把和弦里的几个音全塞到同一根弦上，
 * 六线谱上就是几个数字叠在同一个位置。这里按音高从高到低、每根弦只用一次。
 */
function assignTabPositions(
  events: NotatedEvent[],
  tab: { strings: number; tuning: number[] },
): Map<NotatedEvent, { string: number; fret: number }> {
  const map = new Map<NotatedEvent, { string: number; fret: number }>();
  const groups = new Map<string, NotatedEvent[]>();
  for (const e of events) {
    if (e.rest) continue;
    const key = `${e.voice}:${e.start.toFixed(4)}`;
    const list = groups.get(key) ?? [];
    list.push(e);
    groups.set(key, list);
  }

  for (const list of groups.values()) {
    const used = new Set<number>();
    // 高音在前：高音优先占高音弦，低音落在粗弦上，才像正常的指法
    for (const e of [...list].sort((a, b) => b.pitch - a.pitch)) {
      let best: { string: number; fret: number } | null = null;
      for (let i = 0; i < tab.tuning.length; i += 1) {
        const stringNo = tab.tuning.length - i; // 1 = 最高音弦
        if (used.has(stringNo)) continue;
        const fret = e.pitch - tab.tuning[i]!;
        if (fret < 0 || fret > 24) continue;
        if (!best || fret < best.fret) best = { string: stringNo, fret };
      }
      // 所有弦都占满了（超过弦数的和弦）：退回单音分配，宁可挤也不丢音
      const pos = best ?? tabPosition(e.pitch, tab.tuning);
      used.add(pos.string);
      map.set(e, pos);
    }
  }
  return map;
}

function timeXml(num: number, den: number): string {
  return `<time><beats>${num}</beats><beat-type>${den}</beat-type></time>`;
}

// ---------------------------------------------------------------------------
// 和弦标记
// ---------------------------------------------------------------------------

/** 和弦符号解析结果。 */
interface ParsedChord {
  step: string;
  alter: number;
  kind: string;
  bassStep?: string;
  bassAlter?: number;
}

/** 后缀 → MusicXML `<kind>`。键都做过归一化（小写、去空格、符号替换）。 */
const CHORD_KINDS: Record<string, string> = {
  "": "major",
  maj: "major",
  maj7: "major-seventh",
  maj9: "major-ninth",
  maj11: "major-11th",
  maj13: "major-13th",
  "6": "major-sixth",
  m: "minor",
  min: "minor",
  m7: "minor-seventh",
  m9: "minor-ninth",
  m11: "minor-11th",
  m13: "minor-13th",
  m6: "minor-sixth",
  mmaj7: "minor-major-seventh",
  "7": "dominant",
  "9": "dominant-ninth",
  "11": "dominant-11th",
  "13": "dominant-13th",
  dim: "diminished",
  dim7: "diminished-seventh",
  aug: "augmented",
  m7b5: "half-diminished",
  sus2: "suspended-second",
  sus4: "suspended-fourth",
  sus: "suspended-fourth",
};

/** 后缀的归一化：符号别名统一成 ASCII 缩写。 */
function normalizeSuffix(raw: string): string {
  let s = raw.trim().toLowerCase().replace(/\s+/g, "");
  s = s.replace(/♯/g, "#").replace(/♭/g, "b").replace(/[△Δ]/g, "maj");
  s = s.replace(/[°º]/g, "dim").replace(/ø/g, "m7b5").replace(/[＋+]/g, "aug");
  s = s.replace(/^−/, "-");
  if (s.startsWith("major")) s = `maj${s.slice(5)}`;
  if (s.startsWith("minor")) s = `m${s.slice(5)}`;
  if (s.startsWith("-")) s = `m${s.slice(1)}`;
  return s;
}

/**
 * 解析一个和弦符号（`Cmaj7`、`F#m7b5/A`…）。
 *
 * 解析不了就返回 null，由调用方退化成「文字方向」——宁可显示成普通文本，也不要丢掉用户的标记。
 */
function parseChord(symbol: string): ParsedChord | null {
  const text = symbol.trim();
  const m = /^([A-Ga-g])([#b♯♭]?)([^/]*)(?:\/([A-Ga-g])([#b♯♭]?))?$/u.exec(text);
  if (!m) return null;
  const rootStep = m[1]!.toUpperCase();
  const rootAlter = m[2] === "#" || m[2] === "♯" ? 1 : m[2] === "b" || m[2] === "♭" ? -1 : 0;
  const kind = CHORD_KINDS[normalizeSuffix(m[3] ?? "")];
  if (!kind) return null;
  const bassStep = m[4] ? m[4].toUpperCase() : undefined;
  const bassAlter = m[5] ? (m[5] === "#" || m[5] === "♯" ? 1 : -1) : undefined;
  return { step: rootStep, alter: rootAlter, kind, bassStep, bassAlter };
}

function harmonyXml(symbol: string, warnings: string[]): string {
  const parsed = parseChord(symbol);
  if (!parsed) {
    warnings.push(`和弦标记「${symbol}」无法解析，已按文字标记输出`);
    return `<direction placement="above"><direction-type><words>${esc(symbol)}</words></direction-type></direction>`;
  }
  const alter = (n: number): string => (n === 0 ? "" : `<root-alter>${n}</root-alter>`);
  const root = `<root><root-step>${parsed.step}</root-step>${alter(parsed.alter)}</root>`;
  const bass =
    parsed.bassStep === undefined
      ? ""
      : `<bass><bass-step>${parsed.bassStep}</bass-step>${
          parsed.bassAlter ? `<bass-alter>${parsed.bassAlter}</bass-alter>` : ""
        }</bass>`;
  return `<harmony>${root}<kind>${parsed.kind}</kind>${bass}</harmony>`;
}

// ---------------------------------------------------------------------------
// 序列化
// ---------------------------------------------------------------------------

/** 歌词信息（每段连音线只在第一段上写一次）。 */
interface LyricInfo {
  text: string;
  syllabic: "single" | "begin" | "middle" | "end";
  extend: boolean;
}

interface PartContext {
  opts: ScoreOptions;
  measures: Measure[];
  /** 第一个 part 才写速度 / 段落 / 和弦（否则每行谱表上方都重复一遍）。 */
  lead: boolean;
  drum: boolean;
  /** 这条轨道的六线谱参数；`null` = 五线谱。 */
  tab: { strings: number; tuning: number[] } | null;
  /** 谱号覆盖（`null` = 按音域自动）。 */
  clef: ClefKind | null;
  /** 小节序号 → 该小节起始处的和弦。 */
  harmony: Map<number, string[]>;
  /** 小节序号 → 该小节的排练号。 */
  sections: Map<number, string[]>;
  /** 小节序号 → 该小节开始时的速度。 */
  tempo: Map<number, number>;
  warnings: string[];
}

/** 去掉歌词里的声调数字（`小4` → `小`），声调是给歌声合成用的，不该进谱面。 */
function lyricText(raw: string): string {
  return raw.replace(/[1-5]$/u, "").trim();
}

/** 一条轨道的歌词表：事件对象 → 歌词信息。 */
function lyricMap(nt: NotatedTrack, opts: ScoreOptions): Map<NotatedEvent, LyricInfo> {
  const out = new Map<NotatedEvent, LyricInfo>();
  if (!opts.lyrics) return out;

  // 按声部摊平（跨小节），才能判断「下一个唱词是不是连音记号 -」。
  const byVoice = new Map<number, NotatedEvent[]>();
  for (const bar of nt.bars) {
    for (const e of bar) {
      if (e.rest || !e.note) continue;
      const list = byVoice.get(e.voice) ?? [];
      list.push(e);
      byVoice.set(e.voice, list);
    }
  }

  for (const list of byVoice.values()) {
    list.sort((a, b) => a.start - b.start || b.pitch - a.pitch);
    for (let i = 0; i < list.length; i += 1) {
      const e = list[i]!;
      const raw = e.note?.lyric;
      if (!raw || raw === "-" || raw === "—") continue;
      // 连音线拆出来的后续片段不重复写歌词。
      if (e.chainIndex > 0) continue;
      const next = list.slice(i + 1).find((x) => x.note?.lyric && x.note.lyric !== "");
      const nextRaw = next?.note?.lyric ?? "";
      const melisma = nextRaw === "-" || nextRaw === "—";
      out.set(e, {
        text: lyricText(raw),
        syllabic: melisma ? "begin" : "single",
        extend: melisma || e.chainLength > 1,
      });
    }
  }
  return out;
}

/** 单个音符 / 休止符。 */
function noteXml(
  e: NotatedEvent,
  ctx: PartContext,
  chord: boolean,
  lyric?: LyricInfo,
  tabPos?: { string: number; fret: number },
): string {
  const sym = symbolFor(e.beats);
  const dur = Math.max(1, Math.round(e.beats * DIVISIONS));
  const body: string[] = [];
  if (chord) body.push("  <chord/>");
  if (e.rest) body.push("  <rest/>");
  else if (ctx.drum) body.push(`  ${unpitchedXml(e.pitch)}`);
  else body.push(`  ${pitchXml(e.pitch, ctx.opts.keyFifths)}`);
  body.push(`  <duration>${dur}</duration>`);
  if (e.tie === "both") {
    // 中间那一段要同时「接上一段」和「起下一段」：DTD 里 `<tie>` 只有 start / stop 两种，
    // 所以是两个元素，不是一个 `type="both"`。
    body.push('  <tie type="start"/>');
    body.push('  <tie type="stop"/>');
  } else if (e.tie) {
    body.push(`  <tie type="${e.tie}"/>`);
  }
  body.push(`  <voice>${e.voice}</voice>`);
  body.push(`  <type>${sym.type}</type>`);
  for (let d = 0; d < sym.dots; d += 1) body.push("  <dot/>");
  if (sym.triplet) {
    body.push("  <time-modification><actual-notes>3</actual-notes><normal-notes>2</normal-notes></time-modification>");
  }
  if (!e.rest) {
    const notations: string[] = [];
    if (e.tie === "start" || e.tie === "both") notations.push('<tied type="start"/>');
    if (e.tie === "stop" || e.tie === "both") notations.push('<tied type="stop"/>');
    // 六线谱：这个音落在哪根弦、第几品（由整小节成组分配给出，和弦不会挤到同一根弦）
    if (ctx.tab && tabPos) {
      notations.push(`<technical><string>${tabPos.string}</string><fret>${tabPos.fret}</fret></technical>`);
    }
    if (notations.length > 0) body.push(`  <notations>${notations.join("")}</notations>`);
  }
  if (lyric) {
    const ext = lyric.extend ? "<extend/>" : "";
    body.push(
      `  <lyric number="1"><syllabic>${lyric.syllabic}</syllabic><text>${esc(lyric.text)}</text>${ext}</lyric>`,
    );
  }
  return `    <note>\n${body.join("\n")}\n    </note>`;
}

/** 一个小节。 */
function measureXml(
  m: Measure,
  index: number,
  nt: NotatedTrack,
  ctx: PartContext,
  lyrics: Map<NotatedEvent, LyricInfo>,
): string {
  const lines: string[] = [`    <measure number="${index + 1}">`];

  // ---- attributes ----
  const attrs: string[] = [];
  const prev = index > 0 ? ctx.measures[index - 1] : null;
  if (index === 0) {
    attrs.push(`<divisions>${DIVISIONS}</divisions>`);
    attrs.push(`<key><fifths>${ctx.opts.keyFifths}</fifths></key>`);
    attrs.push(timeXml(m.num, m.den));
    attrs.push(clefXml(nt.track, nt.pitches, ctx.drum, Boolean(ctx.tab), ctx.clef));
    if (ctx.tab) attrs.push(staffDetailsXml(ctx.tab));
  } else if (prev && (prev.num !== m.num || prev.den !== m.den)) {
    attrs.push(timeXml(m.num, m.den));
  }
  if (attrs.length > 0) lines.push(`      <attributes>${attrs.join("")}</attributes>`);

  // ---- 只有第一行谱表写速度 / 段落 / 和弦 ----
  if (ctx.lead) {
    const tempo = ctx.tempo.get(index);
    if (tempo !== undefined) {
      lines.push(
        `      <direction placement="above"><direction-type><metronome><beat-unit>quarter</beat-unit><per-minute>${Math.round(
          tempo,
        )}</per-minute></metronome></direction-type><sound tempo="${tempo}"/></direction>`,
      );
    }
    for (const name of ctx.sections.get(index) ?? []) {
      lines.push(
        `      <direction placement="above"><direction-type><rehearsal>${esc(name)}</rehearsal></direction-type></direction>`,
      );
    }
    for (const symbol of ctx.harmony.get(index) ?? []) {
      lines.push(`      ${harmonyXml(symbol, ctx.warnings)}`);
    }
  }

  // ---- 各声部 ----
  const events = nt.bars[index] ?? [];
  const voices = [...new Set(events.map((e) => e.voice))].sort((a, b) => a - b);
  const backup = Math.round(m.beats * DIVISIONS);

  // 六线谱的弦/品必须**整小节成组**算：和弦里的音各自挑最小品，会全挤到同一根弦上。
  const tabPositions = ctx.tab ? assignTabPositions(events, ctx.tab) : null;

  voices.forEach((voice, order) => {
    if (order > 0) lines.push(`      <backup><duration>${backup}</duration></backup>`);
    const list = events.filter((e) => e.voice === voice);
    let prevStart = Number.NaN;
    for (const e of list) {
      // 同起点、同声部 → 和弦音（第二个起要 <chord/>）。
      const chord = !e.rest && Math.abs(e.start - prevStart) < 1e-6;
      lines.push(noteXml(e, ctx, chord, lyrics.get(e), tabPositions?.get(e)));
      prevStart = e.start;
    }
  });

  lines.push("    </measure>");
  return lines.join("\n");
}

/** 一条轨道 → 一个 `<part>`。 */
function partXml(id: string, nt: NotatedTrack, ctx: PartContext): string {
  const lyrics = lyricMap(nt, ctx.opts);
  const measures = ctx.measures.map((m, i) => measureXml(m, i, nt, ctx, lyrics)).join("\n");
  return `  <part id="${id}">\n${measures}\n  </part>`;
}

/** 每小节起始拍的速度表。 */
function tempoMap(transport: Transport, measures: Measure[]): Map<number, number> {
  const segs = meterSegments(transport);
  const out = new Map<number, number>();
  for (const m of measures) {
    let tempo = segs[0]?.tempo ?? 120;
    for (const s of segs) {
      if (s.start <= m.start + 1e-6) tempo = s.tempo;
      else break;
    }
    out.set(m.index, tempo);
  }
  // 速度没变的小节不写（否则每小节都挂一个速度记号）。
  const result = new Map<number, number>();
  let last: number | null = null;
  for (const m of measures) {
    const tempo = out.get(m.index) ?? 120;
    if (last === null || Math.abs(tempo - last) > 1e-6) result.set(m.index, tempo);
    last = tempo;
  }
  return result;
}

/** 和弦 / 段落 → 小节序号。 */
function barAnchors(
  project: Project,
  measures: Measure[],
): { harmony: Map<number, string[]>; sections: Map<number, string[]> } {
  // 只出片段时，片段之外的标记不能全挤到第 1 小节上。
  const spanStart = measures[0]?.start ?? 0;
  const last = measures[measures.length - 1];
  const spanEnd = last ? last.start + last.beats : 0;
  const inside = (beat: number): boolean => beat >= spanStart - 1e-6 && beat < spanEnd - 1e-6;

  const harmony = new Map<number, string[]>();
  for (const bar of project.harmony ?? []) {
    if (!bar.symbol || !inside(bar.start)) continue;
    const idx = measureIndexAt(bar.start, measures);
    const list = harmony.get(idx) ?? [];
    list.push(bar.symbol);
    harmony.set(idx, list);
  }
  const sections = new Map<number, string[]>();
  for (const sec of project.sections ?? []) {
    if (!sec.name || !inside(sec.start)) continue;
    const idx = measureIndexAt(sec.start, measures);
    const list = sections.get(idx) ?? [];
    list.push(sec.name);
    sections.set(idx, list);
  }
  return { harmony, sections };
}

/**
 * 只保留和 `range` 相交的小节，并重排 `index`（`<measure number>` 从 1 起）。
 *
 * 起点是「**包含** `range.start` 的那一小节」：片段起点不一定压在小节线上（1/16 网格切出来
 * 的片段很常见），但谱面必须从小节线开始，否则第一小节会缺拍、后面的小节全跟着错位。
 */
function sliceMeasures(all: Measure[], range?: { start: number; end: number }): Measure[] {
  if (!range) return all;
  const found = all.findIndex((m) => m.start + m.beats > range.start + 1e-6);
  const from = found < 0 ? Math.max(0, all.length - 1) : found;
  const kept = all.filter((m, i) => i >= from && m.start < range.end - 1e-6);
  const list = kept.length > 0 ? kept : all.slice(from, from + 1);
  return list.map((m, i) => ({ ...m, index: i }));
}

/**
 * 把工程序列化成 MusicXML 文本。
 *
 * 没有任何可记谱的音符时返回 `parts: 0`（调用方据此拦住导出，而不是写一个空文件）。
 */
export function buildMusicXML(project: Project, options: Partial<ScoreOptions> = {}): ScoreResult {
  const opts = resolve(project, options);
  const warnings: string[] = [];

  const scored = project.tracks.filter((t) => t.clips.some((c) => c.notes.length > 0));
  const picked = opts.tracks ? scored.filter((t) => opts.tracks!.includes(t.id)) : scored;

  if (picked.length === 0) {
    return {
      xml: "",
      parts: 0,
      trackIds: [],
      measures: 0,
      notes: 0,
      warnings: ["没有可记谱的音符"],
      barStarts: [],
      barLengths: [],
    };
  }

  let endBeat = 0;
  let noteCount = 0;
  for (const t of picked) {
    for (const c of t.clips) {
      for (const n of c.notes) {
        // 用**量化后**的终点算曲长：否则「3.99 拍 + 0.01」这种音量化后会落到 4.0，
        // 落在最后一个小节之外，补休止时就对不上。
        const end = c.start + n.start + n.duration;
        endBeat = Math.max(endBeat, opts.grid > 0 ? quantizeBeat(end, opts.grid) : end);
        noteCount += 1;
      }
    }
  }

  // 曲长要覆盖到 range 的终点：只出片段时，片段末尾的小节也得排出来（哪怕音符早结束了）。
  if (opts.range) endBeat = Math.max(endBeat, opts.range.end);

  const measures = sliceMeasures(buildMeasures(project.transport, endBeat), opts.range);
  const notated = picked.map((t) => notateTrack(t, measures, { grid: opts.grid }));
  for (const nt of notated) {
    for (const w of nt.warnings) warnings.push(`${nt.track.name}：${w}`);
  }

  const anchors = barAnchors(project, measures);
  const tempo = tempoMap(project.transport, measures);

  const parts = notated.map((nt, i) => {
    const id = `P${i + 1}`;
    // 谱式逐 part 决定：总谱里每条轨道可以有自己的排法（吉他带六线谱、鼓用打击乐谱表）。
    const notation = opts.notationFor ? opts.notationFor(nt.track) : defaultNotationFor(nt.track);
    const baseTab = tabSpecFor(notation);
    const tuning = opts.tuningFor?.(nt.track) ?? null;
    const ctx: PartContext = {
      opts,
      measures,
      lead: i === 0,
      drum: notation === "drums",
      tab: baseTab ? { strings: tuning ? tuning.length : baseTab.strings, tuning: tuning ?? baseTab.tuning } : null,
      clef: opts.clefFor?.(nt.track) ?? null,
      harmony: anchors.harmony,
      sections: anchors.sections,
      tempo,
      warnings,
    };
    return { id, nt, ctx };
  });

  const partList = parts
    .map(({ id, nt, ctx }, i) => {
      const channel = ctxChannel(ctx.drum, i);
      return [
        `    <score-part id="${id}">`,
        `      <part-name>${esc(nt.track.name || `Track ${id.slice(1)}`)}</part-name>`,
        `      <score-instrument id="${id}-I1"><instrument-name>${esc(nt.track.name || "Instrument")}</instrument-name></score-instrument>`,
        `      <midi-instrument id="${id}-I1"><midi-channel>${channel}</midi-channel><midi-program>1</midi-program></midi-instrument>`,
        "    </score-part>",
      ].join("\n");
    })
    .join("\n");

  const body = parts.map(({ id, nt, ctx }) => partXml(id, nt, ctx)).join("\n");

  const xml = [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<!DOCTYPE score-partwise PUBLIC "-//Recordare//DTD MusicXML 4.0 Partwise//EN" "http://www.musicxml.org/dtds/partwise.dtd">',
    '<score-partwise version="4.0">',
    `  <work><work-title>${esc(opts.title ?? "Untitled")}</work-title></work>`,
    '  <identification><encoding><software>AI DAW</software></encoding></identification>',
    "  <part-list>",
    partList,
    "  </part-list>",
    body,
    "</score-partwise>",
    "",
  ].join("\n");

  return {
    xml,
    parts: parts.length,
    trackIds: picked.map((t) => t.id),
    measures: measures.length,
    notes: noteCount,
    warnings,
    barStarts: measures.map((m) => m.start),
    barLengths: measures.map((m) => m.beats),
  };
}

/** MIDI 通道：鼓固定第 10 通道，其余按 part 顺序往后排（最多 16）。 */
function ctxChannel(drum: boolean, index: number): number {
  if (drum) return 10;
  const used = index + 1;
  return used === 10 ? 11 : Math.min(16, used);
}
