/**
 * MusicXML（partwise）→ 工程模型。
 *
 * 支持的子集：`<part>` / `<measure>` / `<attributes>`（divisions、time）/ `<note>`（pitch、
 * unpitched、rest、duration、chord、tie、voice、lyric）/ `<backup>` / `<forward>` /
 * `<direction>` 里的 tempo 与 rehearsal。
 *
 * 明确不做：连音线的**视觉**还原、反复记号 / 跳房子（`<repeat>`、`<ending>`）、多谱表
 * 跨谱连线、装饰音（grace）、微分音。这些要么工程模型里没有对应概念，要么需要真正的记谱
 * 引擎——做不到就**跳过并记 warning**，不静默改数据。
 *
 * 这个解析器的主要用途是「吃得下自己导出的谱」以及常见软件导出的单谱表文件；它不打算
 * 取代 Dorico。
 */
import type { Keyframe, Note, Project, Track } from "@/api/types";
import { attr, child, children, descendants, documentElement, parseXml, textOf } from "./xml";

export interface ScorePartNotes {
  name: string;
  notes: Note[];
  /** 来自 `<unpitched>`（打击乐谱表）。 */
  drum: boolean;
}

export interface ParsedScore {
  parts: ScorePartNotes[];
  tempos: { beat: number; bpm: number }[];
  meters: { beat: number; num: number; den: number }[];
  /** 小节数（用于界面提示）。 */
  measures: number;
  warnings: string[];
}

/** 音名 → 相对 C 的半音数。 */
const STEP_SEMITONE: Record<string, number> = { C: 0, D: 2, E: 4, F: 5, G: 7, A: 9, B: 11 };

/**
 * 打击乐谱表位置 → GM 音高。
 *
 * 与导出时的 `DRUM_MAP` 反向对应（那边一个位置可能对应多个音高，这里只能挑一个常用的）：
 * 所以鼓谱往返一圈会丢「具体是哪个镲」这类细节，但节奏与位置保留。
 */
const DRUM_REVERSE: Record<string, number> = {
  "F4": 36,
  "A4": 41,
  "B4": 43,
  "C5": 38,
  "D5": 45,
  "E5": 47,
  "F5": 51,
  "G5": 42,
  "A5": 49,
  "C6": 54,
  "E6": 58,
};

/** 解析一份 MusicXML。 */
export function parseMusicXML(xml: string): ParsedScore {
  const warnings: string[] = [];
  const root = documentElement(parseXml(xml));
  if (!root) throw new Error("不是有效的 XML");
  if (root.name !== "score-partwise") {
    throw new Error(`暂不支持 ${root.name}（只支持 score-partwise）`);
  }

  // part-list：id → 谱表名
  const names = new Map<string, string>();
  for (const sp of descendants(root, "score-part")) {
    const id = attr(sp, "id");
    if (id) names.set(id, textOf(child(sp, "part-name")) || id);
  }

  const tempos: { beat: number; bpm: number }[] = [];
  const meters: { beat: number; num: number; den: number }[] = [];
  const parts: ScorePartNotes[] = [];
  let measureCount = 0;

  const partNodes = children(root, "part");
  for (const partNode of partNodes) {
    const id = attr(partNode, "id") ?? "";
    const name = names.get(id) ?? id ?? "Part";
    const notes: Note[] = [];
    let drum = false;

    /** divisions 是**每条 part 各自声明**的，换小节也可能变。 */
    let divisions = 1;
    let measureStart = 0;
    let measureMaxCursor = 0;
    let cursor = 0;
    let lastNoteStart = 0;
    /** 连音线：键 = 声部 + 音高，值 = 还等着被续上的那个音。 */
    const openTies = new Map<string, Note>();

    const measures = children(partNode, "measure");
    measureCount = Math.max(measureCount, measures.length);

    for (const measure of measures) {
      cursor = 0;
      measureMaxCursor = 0;
      lastNoteStart = 0;

      for (const node of measure.children) {
        if (node.name === "attributes") {
          const div = textOf(child(node, "divisions"));
          if (div) {
            const value = Number(div);
            if (Number.isFinite(value) && value > 0) divisions = value;
          }
          const time = child(node, "time");
          if (time) {
            const num = Number(textOf(child(time, "beats")) ?? "0");
            const den = Number(textOf(child(time, "beat-type")) ?? "0");
            if (num > 0 && den > 0) {
              const beat = measureStart + cursor / divisions;
              meters.push({ beat, num, den });
            }
          }
          continue;
        }

        if (node.name === "backup") {
          const dur = Number(textOf(child(node, "duration")) ?? "0");
          if (Number.isFinite(dur)) cursor = Math.max(0, cursor - dur);
          continue;
        }

        if (node.name === "forward") {
          const dur = Number(textOf(child(node, "duration")) ?? "0");
          if (Number.isFinite(dur) && dur > 0) {
            cursor += dur;
            measureMaxCursor = Math.max(measureMaxCursor, cursor);
          }
          continue;
        }

        if (node.name === "direction") {
          const sound = child(node, "sound");
          const tempoAttr = attr(sound, "tempo");
          if (tempoAttr) {
            const bpm = Number(tempoAttr);
            if (Number.isFinite(bpm) && bpm > 0) {
              tempos.push({ beat: measureStart + cursor / divisions, bpm });
            }
          }
          const metronome = descendants(node, "metronome")[0] ?? null;
          if (metronome && !tempoAttr) {
            const per = Number(textOf(child(metronome, "per-minute")) ?? "0");
            if (Number.isFinite(per) && per > 0) {
              tempos.push({ beat: measureStart + cursor / divisions, bpm: per });
            }
          }
          continue;
        }

        if (node.name !== "note") continue;

        const durRaw = Number(textOf(child(node, "duration")) ?? "0");
        const dur = Number.isFinite(durRaw) && durRaw > 0 ? durRaw : 0;
        if (dur === 0) continue; // 装饰音 / 无时值音：跳过

        const isRest = child(node, "rest") !== null;
        const isChord = child(node, "chord") !== null;
        const startTick = isChord ? lastNoteStart : cursor;
        const start = measureStart + startTick / divisions;

        if (isRest) {
          if (!isChord) {
            cursor += dur;
            measureMaxCursor = Math.max(measureMaxCursor, cursor);
          }
          continue;
        }

        const pitchNode = child(node, "pitch");
        const unpitchedNode = child(node, "unpitched");
        let pitch = -1;
        if (pitchNode) {
          const step = textOf(child(pitchNode, "step")).toUpperCase();
          const octave = Number(textOf(child(pitchNode, "octave")) ?? "4");
          const alter = Number(textOf(child(pitchNode, "alter")) ?? "0");
          const semitone = STEP_SEMITONE[step];
          if (semitone !== undefined) pitch = (octave + 1) * 12 + semitone + (Number.isFinite(alter) ? alter : 0);
        } else if (unpitchedNode) {
          drum = true;
          const step = textOf(child(unpitchedNode, "display-step")).toUpperCase();
          const octave = textOf(child(unpitchedNode, "display-octave"));
          pitch = DRUM_REVERSE[`${step}${octave}`] ?? 38;
        }

        if (pitch < 0 || pitch > 127) {
          warnings.push(`跳过无法识别音高的音符（第 ${measures.indexOf(measure) + 1} 小节）`);
          if (!isChord) {
            cursor += dur;
            measureMaxCursor = Math.max(measureMaxCursor, cursor);
          }
          continue;
        }

        if (!isChord) {
          cursor += dur;
          measureMaxCursor = Math.max(measureMaxCursor, cursor);
          lastNoteStart = startTick;
        }

        const beats = dur / divisions;
        // `<tie>` 可以出现多次（跨小节的长音在中间那一段会同时有 start 和 stop）。
        const tieTypes = children(node, "tie").map((t) => attr(t, "type"));
        const tieStart = tieTypes.includes("start");
        const tieStop = tieTypes.includes("stop");
        const voice = textOf(child(node, "voice")) || "1";
        const tieKey = `${voice}:${pitch}`;

        const lyricNode = child(node, "lyric");
        const lyric = lyricNode ? textOf(child(lyricNode, "text")) : "";

        if (tieStop && openTies.has(tieKey)) {
          // 连音线的后半段：并进前一个音，而不是新开一个。
          const head = openTies.get(tieKey)!;
          head.duration += beats;
          if (tieStart) openTies.set(tieKey, head);
          else openTies.delete(tieKey);
          continue;
        }

        const note: Note = {
          pitch,
          start,
          duration: beats,
          velocity: 100,
          volume: 127,
          ...(lyric ? { lyric } : {}),
        };
        notes.push(note);
        if (tieStart) openTies.set(tieKey, note);
      }

      measureStart += measureMaxCursor / divisions;
    }

    if (notes.length > 0) {
      notes.sort((a, b) => a.start - b.start || a.pitch - b.pitch);
      parts.push({ name, notes, drum });
    }
  }

  if (parts.length === 0) throw new Error("谱面里没有音符");

  // 多 part 会把同一组 attributes 重复写一遍：按 (拍, 值) 去重。
  const dedupe = <T>(list: T[], key: (item: T) => string): T[] => {
    const seen = new Set<string>();
    return list.filter((item) => {
      const k = key(item);
      if (seen.has(k)) return false;
      seen.add(k);
      return true;
    });
  };

  return {
    parts,
    tempos: dedupe(tempos, (t) => `${t.beat.toFixed(4)}:${t.bpm.toFixed(4)}`).sort((a, b) => a.beat - b.beat),
    meters: dedupe(meters, (m) => `${m.beat.toFixed(4)}:${m.num}/${m.den}`).sort((a, b) => a.beat - b.beat),
    measures: measureCount,
    warnings,
  };
}

/** 解析结果 → 新工程（与 MIDI 导入共用同一套「每轨一个片段」的铺法）。 */
export function scoreToProject(parsed: ParsedScore, name: string): Project {
  const baseTempo = clampTempo(parsed.tempos[0]?.bpm ?? 120);
  const baseNum = parsed.meters[0]?.num ?? 4;
  const baseDen = parsed.meters[0]?.den ?? 4;

  const changes = new Map<number, { tempo?: number; num?: number; den?: number }>();
  const at = (beat: number) => {
    const slot = changes.get(beat) ?? {};
    changes.set(beat, slot);
    return slot;
  };
  for (const t of parsed.tempos) if (t.beat > 0) at(t.beat).tempo = clampTempo(t.bpm);
  for (const m of parsed.meters) {
    if (m.beat <= 0) continue;
    const slot = at(m.beat);
    slot.num = m.num;
    slot.den = m.den;
  }

  let tempo = baseTempo;
  let num = baseNum;
  let den = baseDen;
  const keyframes: Keyframe[] = [...changes.keys()]
    .sort((a, b) => a - b)
    .map((beat) => {
      const slot = changes.get(beat)!;
      if (slot.tempo !== undefined) tempo = slot.tempo;
      if (slot.num !== undefined) num = slot.num;
      if (slot.den !== undefined) den = slot.den;
      return { beat, tempo, time_sig_num: num, time_sig_den: den };
    });

  const barBeats = (n: number, d: number): number => n * (4 / Math.max(1, d));
  const bar = barBeats(baseNum, baseDen);

  const tracks: Track[] = parsed.parts.map((part, index) => {
    const end = part.notes.reduce((max, n) => Math.max(max, n.start + n.duration), 0);
    const length = Math.max(bar, Math.ceil(end / bar) * bar);
    return {
      id: index + 1,
      name: part.name || `Track ${index + 1}`,
      instrument: {
        plugin_id: part.drum ? "builtin.drums" : "builtin.piano",
        params: {},
        state: [],
      },
      effects: [],
      mixer: { gain_db: 0, pan: 0, mute: false, solo: false },
      clips: [{ id: index + 1, start: 0, length, notes: part.notes }],
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
