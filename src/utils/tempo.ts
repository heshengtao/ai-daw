/**
 * 拍 ↔ 秒的分段换算（镜像 Rust 的 `crates/engine/src/tempo.rs`）。
 *
 * 两个方向都必须与引擎**逐位可对拍**：`beatsToSeconds` 用 `(delta) * 60 / bpm`，
 * `secondsToBeats` 用 `(delta) * bpm / 60`，不要改写数值形式。
 */
import type { Transport } from "@/api/types";

export interface Segment {
  startBeat: number;
  endBeat: number;
  tempo: number;
  num: number;
  den: number;
}

/** 按拍号/速度关键点切段（第 0 段来自基准字段）。 */
export function segments(t: Transport): Segment[] {
  const out: Segment[] = [];
  const kfs = [...(t.keyframes ?? [])].sort((a, b) => a.beat - b.beat);
  let start = 0;
  let tempo = t.tempo;
  let num = t.time_sig_num;
  let den = t.time_sig_den;
  for (const kf of kfs) {
    if (kf.beat <= start) {
      tempo = kf.tempo;
      num = kf.time_sig_num;
      den = kf.time_sig_den;
      continue;
    }
    out.push({ startBeat: start, endBeat: kf.beat, tempo, num, den });
    start = kf.beat;
    tempo = kf.tempo;
    num = kf.time_sig_num;
    den = kf.time_sig_den;
  }
  out.push({ startBeat: start, endBeat: Infinity, tempo, num, den });
  return out;
}

function segmentAt(beat: number, t: Transport): Segment {
  const b = beat > 0 ? beat : 0;
  const segs = segments(t);
  let found = segs[0]!;
  for (const s of segs) {
    if (s.startBeat <= b) found = s;
    else break;
  }
  return found;
}

export function tempoAt(beat: number, t: Transport): number {
  return segmentAt(beat, t).tempo;
}

export function meterAt(beat: number, t: Transport): { num: number; den: number } {
  const s = segmentAt(beat, t);
  return { num: s.num, den: s.den };
}

export function beatsToSeconds(beats: number, t: Transport): number {
  if (!(beats > 0)) return 0;
  let secs = 0;
  const segs = segments(t);
  for (let i = 0; i < segs.length; i += 1) {
    const s = segs[i]!;
    if (beats <= s.startBeat) break;
    const next = segs[i + 1]?.startBeat;
    const end = next !== undefined && beats > next ? next : beats;
    secs += ((end - s.startBeat) * 60) / s.tempo;
    if (next !== undefined && beats <= next) break;
  }
  return secs;
}

export function secondsToBeats(secs: number, t: Transport): number {
  if (!(secs > 0)) return 0;
  let acc = 0;
  const segs = segments(t);
  for (let i = 0; i < segs.length; i += 1) {
    const s = segs[i]!;
    const next = segs[i + 1]?.startBeat;
    const segSecs = next !== undefined ? ((next - s.startBeat) * 60) / s.tempo : Infinity;
    if (secs <= acc + segSecs) return s.startBeat + ((secs - acc) * s.tempo) / 60;
    acc += segSecs;
  }
  return 0;
}

/** 一小节的长度（四分音符数）。 */
export function barBeatsAt(beat: number, t: Transport): number {
  const s = segmentAt(beat, t);
  return s.num * (4 / Math.max(1, s.den));
}

/** 一个「拍」（拍号单位）的长度（四分音符数）。 */
export function meterBeatAt(beat: number, t: Transport): number {
  return 4 / Math.max(1, segmentAt(beat, t).den);
}

/** 拍 → 小节.拍（跨段累计小节数）。 */
export function barBeatAt(beat: number, t: Transport): { bar: number; beatInBar: number } {
  let bar = 1;
  for (const s of segments(t)) {
    const barBeats = s.num * (4 / Math.max(1, s.den));
    const end = Number.isFinite(s.endBeat) ? s.endBeat : Infinity;
    if (beat < end) {
      const within = beat - s.startBeat;
      return {
        bar: bar + Math.floor(within / barBeats),
        beatInBar: Math.floor((within % barBeats) / (4 / Math.max(1, s.den))) + 1,
      };
    }
    // 本段整段计入小节数（段长未必是整数个小节，按「向上取整的整小节」推进）。
    bar += Math.max(1, Math.round((end - s.startBeat) / barBeats));
  }
  return { bar, beatInBar: 1 };
}
