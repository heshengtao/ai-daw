/**
 * 标准 MIDI 文件（SMF）解析 → 工程模型。
 *
 * 只做「导入」需要的部分：音符（note on/off）、速度（0x51）、拍号（0x58）、轨道名（0x03）、
 * 音色（program change）、歌词（0x05 / 0x01 文本事件）。CC / 弯音 / 触后这些演奏细节暂时丢掉
 * ——工程的音符模型里没有逐音 CC，硬塞进来也没地方放（`midi_events` 是逐轨的一次性事件，
 * 不适合搬 CC 曲线）。
 *
 * 时间统一换成**拍**（四分音符 = 1）：`beat = ticks / division`，与工程模型一致。
 * SMPTE 时间码的 MIDI（division 最高位为 1）直接拒绝——那种文件的时间基准是秒，不是拍。
 */
import type { Keyframe, Note, Project, Track } from "@/api/types";

/** 解析出来的一个音。 */
export interface MidiNoteData {
  pitch: number;
  /** 起点（拍）。 */
  start: number;
  /** 时值（拍）。 */
  duration: number;
  velocity: number;
  /**
   * 歌词（可选）。
   *
   * SMF 里歌词不是音符的属性，而是**时间线上的文本事件**：meta `0x05`(Lyric) / `0x01`(Text)。
   * 解析时按 tick 配到同一拍的那个音上（和弦取最高音，见 `parseTrack` 的注释）。
   */
  lyric?: string;
}

/** 解析出来的一条 MIDI 轨（可能多于源文件的 MTrk：format 0 会按通道拆开）。 */
export interface MidiTrackData {
  name: string;
  /** 0..15；鼓组是 9。 */
  channel: number;
  program: number;
  notes: MidiNoteData[];
}

export interface MidiTempo {
  beat: number;
  bpm: number;
}

export interface MidiMeter {
  beat: number;
  num: number;
  den: number;
}

export interface ParsedMidi {
  format: number;
  /** 每四分音符的 tick 数。 */
  division: number;
  tracks: MidiTrackData[];
  tempos: MidiTempo[];
  meters: MidiMeter[];
  /** 跳过 / 降级的内容，给界面提示用。 */
  warnings: string[];
}

// ---------------------------------------------------------------------------
// 读取
// ---------------------------------------------------------------------------

interface Reader {
  pos: number;
  u8(): number;
  u16(): number;
  u32(): number;
  ascii(n: number): string;
  varint(): number;
  skip(n: number): void;
}

function makeReader(bytes: Uint8Array, pos = 0): Reader {
  return {
    pos,
    u8(): number {
      if (this.pos >= bytes.length) throw new Error("MIDI 文件意外结束");
      return bytes[this.pos++]!;
    },
    u16(): number {
      return (this.u8() << 8) | this.u8();
    },
    u32(): number {
      return ((this.u8() << 24) | (this.u8() << 16) | (this.u8() << 8) | this.u8()) >>> 0;
    },
    ascii(n: number): string {
      let out = "";
      for (let i = 0; i < n; i += 1) out += String.fromCharCode(this.u8());
      return out;
    },
    /** 变长整数：每字节低 7 位，最高位是「还有后续」。最多 4 字节。 */
    varint(): number {
      let value = 0;
      for (let i = 0; i < 4; i += 1) {
        const b = this.u8();
        value = (value << 7) | (b & 0x7f);
        if ((b & 0x80) === 0) return value;
      }
      return value;
    },
    skip(n: number): void {
      this.pos = Math.min(bytes.length, this.pos + Math.max(0, n));
    },
  };
}

/** MIDI 文本按 UTF-8 解，失败就退回 Latin-1（老文件里很常见）。 */
function decodeText(raw: Uint8Array): string {
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(raw).trim();
  } catch {
    let out = "";
    for (const b of raw) out += String.fromCharCode(b);
    return out.trim();
  }
}

// ---------------------------------------------------------------------------
// 解析
// ---------------------------------------------------------------------------

/** 一个还没等到 note-off 的音。 */
interface OpenNote {
  startTick: number;
  velocity: number;
}

/**
 * 解析 SMF。
 *
 * 抛错只在「根本不是 MIDI」或「用了 SMPTE 时间码」这两种情况——其它问题一律降级进 warnings。
 */
export function parseMidi(bytes: Uint8Array): ParsedMidi {
  const warnings: string[] = [];
  const r = makeReader(bytes);

  const magic = r.ascii(4);
  if (magic !== "MThd") throw new Error("不是 MIDI 文件（没有 MThd 头）");
  const headerLen = r.u32();
  const format = r.u16();
  r.u16(); // 声明的轨数：以实际扫到的 MTrk 为准，不信这个字段
  const division = r.u16();
  r.skip(headerLen - 6);
  if ((division & 0x8000) !== 0) throw new Error("暂不支持 SMPTE 时间码的 MIDI 文件");
  if (division === 0) throw new Error("MIDI 头的 division 为 0");

  const tempos: MidiTempo[] = [];
  const meters: MidiMeter[] = [];
  const tracks: MidiTrackData[] = [];

  while (r.pos + 8 <= bytes.length) {
    const id = r.ascii(4);
    const len = r.u32();
    const start = r.pos;
    const end = Math.min(bytes.length, start + len);
    if (id === "MTrk") parseTrack(bytes, start, end, division, tempos, meters, tracks, warnings);
    r.pos = end;
  }

  if (tracks.length === 0) throw new Error("MIDI 文件里没有音符");

  tempos.sort((a, b) => a.beat - b.beat);
  meters.sort((a, b) => a.beat - b.beat);
  return { format, division, tracks, tempos, meters, warnings };
}

function parseTrack(
  bytes: Uint8Array,
  start: number,
  end: number,
  division: number,
  tempos: MidiTempo[],
  meters: MidiMeter[],
  out: MidiTrackData[],
  warnings: string[],
): void {
  const r = makeReader(bytes, start);
  let tick = 0;
  let running = 0;
  let name = "";
  let program = 0;
  /** 键 = 通道 * 128 + 音高；同音高重叠时按栈配对（后进先出）。 */
  const open = new Map<number, OpenNote[]>();
  const byChannel = new Map<number, MidiNoteData[]>();
  /** 歌词 / 文本事件（时间线上的，不是音符的属性）。 */
  const lyrics: { tick: number; text: string; kind: number }[] = [];

  const closeNote = (key: number, endTick: number): void => {
    const stack = open.get(key);
    const note = stack?.pop();
    if (!note) return;
    const channel = Math.floor(key / 128);
    const pitch = key % 128;
    const begin = note.startTick / division;
    const len = Math.max(1 / 960, (endTick - note.startTick) / division);
    const list = byChannel.get(channel) ?? [];
    list.push({ pitch, start: begin, duration: len, velocity: note.velocity });
    byChannel.set(channel, list);
  };

  while (r.pos < end) {
    tick += r.varint();
    if (r.pos >= end) break;
    let status = r.u8();
    if (status < 0x80) {
      // running status：这个字节本身就是数据
      if (running === 0) {
        warnings.push("遇到无法识别的 MIDI 事件，已跳过");
        break;
      }
      r.pos -= 1;
      status = running;
    }
    const type = status & 0xf0;
    const channel = status & 0x0f;

    if (status === 0xff) {
      const meta = r.u8();
      const len = r.varint();
      const dataStart = r.pos;
      if (meta === 0x51 && len >= 3) {
        const us = (bytes[dataStart]! << 16) | (bytes[dataStart + 1]! << 8) | bytes[dataStart + 2]!;
        if (us > 0) tempos.push({ beat: tick / division, bpm: 60_000_000 / us });
      } else if (meta === 0x58 && len >= 2) {
        const num = bytes[dataStart]!;
        const den = 2 ** bytes[dataStart + 1]!;
        if (num > 0 && den > 0) meters.push({ beat: tick / division, num, den });
      } else if (meta === 0x03) {
        name = decodeText(bytes.slice(dataStart, dataStart + len));
      } else if (meta === 0x05 || meta === 0x01) {
        // 0x05 = Lyric（卡拉 OK 用的那个），0x01 = Text（有些工具只写这个）
        const text = decodeText(bytes.slice(dataStart, dataStart + len));
        if (text) lyrics.push({ tick, text, kind: meta });
      } else if (meta === 0x2f) {
        r.pos = dataStart + len;
        break;
      }
      r.pos = dataStart + len;
      continue;
    }

    if (status === 0xf0 || status === 0xf7) {
      const len = r.varint();
      r.skip(len);
      continue;
    }

    running = status;

    if (type === 0x90) {
      const pitch = r.u8() & 0x7f;
      const velocity = r.u8() & 0x7f;
      const key = channel * 128 + pitch;
      if (velocity === 0) closeNote(key, tick);
      else {
        const stack = open.get(key) ?? [];
        stack.push({ startTick: tick, velocity });
        open.set(key, stack);
      }
    } else if (type === 0x80) {
      const pitch = r.u8() & 0x7f;
      r.u8();
      closeNote(channel * 128 + pitch, tick);
    } else if (type === 0xa0 || type === 0xb0 || type === 0xe0) {
      r.skip(2);
    } else if (type === 0xc0) {
      program = r.u8();
    } else if (type === 0xd0) {
      r.skip(1);
    } else {
      warnings.push(`未知的 MIDI 状态字节 0x${status.toString(16)}，已跳过`);
      break;
    }
  }

  // 文件结束时还没收到 note-off 的音：按当前时刻收尾（总比丢掉强）。
  for (const key of open.keys()) closeNote(key, tick);

  /**
   * 歌词 → 音符。
   *
   * SMF 的歌词是**时间线上的文本事件**（meta 0x05 / 0x01），不是音符属性，所以只能按 tick 配。
   * 同一 tick 上可能是一整个和弦：挂到**最高音**（旋律声部）上，和 Guitar Pro 的习惯一致。
   *
   * 优先只认 0x05(Lyric)；整条轨都没有 0x05 时才退回 0x01(Text)——Text 也被拿来写
   * 注释、作者、版权，所以只在没有 Lyric 时用，并滤掉一眼就是元信息的那几类。
   */
  if (lyrics.length > 0 && byChannel.size > 0) {
    const kind = lyrics.some((l) => l.kind === 0x05) ? 0x05 : 0x01;
    const byTick = new Map<number, string>();
    for (const l of lyrics) {
      if (l.kind !== kind) continue;
      if (kind === 0x01 && /^(created|generated|sequenced|http|www|©|\(c\))/i.test(l.text)) continue;
      byTick.set(l.tick, l.text);
    }
    if (byTick.size > 0) {
      const notesAt = new Map<number, MidiNoteData[]>();
      for (const notes of byChannel.values()) {
        for (const n of notes) {
          const at = Math.round(n.start * division);
          const list = notesAt.get(at) ?? [];
          list.push(n);
          notesAt.set(at, list);
        }
      }
      let unmatched = 0;
      for (const [at, text] of byTick) {
        const list = notesAt.get(at);
        if (!list || list.length === 0) {
          unmatched += 1;
          continue;
        }
        let target = list[0]!;
        for (const n of list) if (n.pitch > target.pitch) target = n;
        if (!target.lyric) target.lyric = text;
      }
      if (unmatched > 0) warnings.push(`${unmatched} 条歌词没有对应的音符，已忽略`);
    }
  }

  if (byChannel.size === 0) return;

  // 一个 MTrk 里可能混着多个通道（format 0 就是这样）：按通道拆成多条轨，DAW 里才好分别处理。
  const channels = [...byChannel.keys()].sort((a, b) => a - b);
  for (const channel of channels) {
    const notes = byChannel.get(channel)!;
    notes.sort((a, b) => a.start - b.start || a.pitch - b.pitch);
    out.push({
      name: channels.length > 1 && name ? `${name} · Ch ${channel + 1}` : name,
      channel,
      program,
      notes,
    });
  }
}

// ---------------------------------------------------------------------------
// 转工程
// ---------------------------------------------------------------------------

/** 把解析结果铺成一个新工程。 */
export function midiToProject(parsed: ParsedMidi, name: string): Project {
  const baseTempo = clampTempo(parsed.tempos[0]?.bpm ?? 120);
  const baseNum = parsed.meters[0]?.num ?? 4;
  const baseDen = parsed.meters[0]?.den ?? 4;

  // 速度和拍号各走各的时间线，合成一张「从第几拍起变成什么」。
  const changes = new Map<number, { tempo?: number; num?: number; den?: number }>();
  const at = (beat: number) => {
    const found = changes.get(beat) ?? {};
    changes.set(beat, found);
    return found;
  };
  for (const t of parsed.tempos) {
    if (t.beat <= 0) continue;
    at(t.beat).tempo = clampTempo(t.bpm);
  }
  for (const m of parsed.meters) {
    if (m.beat <= 0) continue;
    const slot = at(m.beat);
    slot.num = m.num;
    slot.den = m.den;
  }

  const beats = [...changes.keys()].sort((a, b) => a - b);
  let tempo = baseTempo;
  let num = baseNum;
  let den = baseDen;
  const keyframes: Keyframe[] = beats.map((beat) => {
    const slot = changes.get(beat)!;
    if (slot.tempo !== undefined) tempo = slot.tempo;
    if (slot.num !== undefined) num = slot.num;
    if (slot.den !== undefined) den = slot.den;
    return { beat, tempo, time_sig_num: num, time_sig_den: den };
  });

  const barBeats = (n: number, d: number): number => n * (4 / Math.max(1, d));
  const tracks: Track[] = parsed.tracks.map((data, index) => {
    const end = data.notes.reduce((max, n) => Math.max(max, n.start + n.duration), 0);
    const bar = barBeats(baseNum, baseDen);
    const length = Math.max(bar, Math.ceil(end / bar) * bar);
    const drum = data.channel === 9;
    const notes: Note[] = data.notes.map((n) => ({
      pitch: n.pitch,
      start: n.start,
      duration: n.duration,
      velocity: Math.max(1, Math.min(127, Math.round(n.velocity))),
      volume: 127,
      ...(n.lyric ? { lyric: n.lyric } : {}),
    }));
    return {
      id: index + 1,
      name: data.name || `Track ${index + 1}`,
      // 没有音色的事件就落到内置钢琴 / 鼓组：不然导进来是哑的，用户第一反应是"坏了"。
      instrument: {
        plugin_id: drum ? "builtin.drums" : "builtin.piano",
        params: {},
        state: [],
      },
      effects: [],
      mixer: { gain_db: 0, pan: 0, mute: false, solo: false },
      clips: [{ id: index + 1, start: 0, length, notes }],
    };
  });

  return {
    name,
    transport: {
      tempo: baseTempo,
      time_sig_num: baseNum,
      time_sig_den: baseDen,
      ...(keyframes.length > 0 ? { keyframes } : {}),
    },
    tracks,
    master: { gain_db: 0, pan: 0, mute: false, solo: false },
  };
}

function clampTempo(bpm: number): number {
  if (!Number.isFinite(bpm)) return 120;
  return Math.max(20, Math.min(400, Math.round(bpm * 100) / 100));
}
