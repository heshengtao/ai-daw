/**
 * 导入自检（MIDI / MusicXML），不依赖测试框架。
 *
 * 运行：`npm run selftest:import`
 *
 * MIDI 这边用手写字节造一个两轨文件（格式 1、480 division、120 BPM、4/4），验证
 * 「tick → 拍」的换算和 note on/off 配对；
 * MusicXML 这边走**往返**：工程 → `buildMusicXML` → 解析回来 → 工程，验证连音线合并、
 * 声部、歌词、鼓谱、变拍号都能回到原样。
 */
import { midiToProject, parseMidi } from "../src/import/midi";
import { parseMusicXML, scoreToProject } from "../src/import/musicxml";
import { buildMusicXML } from "../src/score/musicxml";
import type { Note, Project } from "../src/api/types";

let failures = 0;

function check(ok: boolean, label: string): void {
  if (ok) {
    console.log(`  ok   ${label}`);
  } else {
    failures += 1;
    console.log(`  FAIL ${label}`);
  }
}

// ---------------------------------------------------------------------------
// MIDI：手写字节
// ---------------------------------------------------------------------------

/** 变长整数（MIDI 的 delta time）。 */
function vlq(value: number): number[] {
  const out = [value & 0x7f];
  let rest = value >> 7;
  while (rest > 0) {
    out.unshift((rest & 0x7f) | 0x80);
    rest >>= 7;
  }
  return out;
}

function chunk(id: string, data: number[]): number[] {
  const out: number[] = [];
  for (const ch of id) out.push(ch.charCodeAt(0));
  const n = data.length;
  out.push((n >>> 24) & 0xff, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff);
  return out.concat(data);
}

function textMeta(text: string): number[] {
  const bytes = [...text].map((c) => c.charCodeAt(0) & 0xff);
  return [0x00, 0xff, 0x03, bytes.length, ...bytes];
}

function buildMidiFile(): Uint8Array {
  const header = chunk("MThd", [
    0x00,
    0x01, // format 1
    0x00,
    0x02, // 2 tracks
    0x01,
    0xe0, // 480 ticks / quarter
  ]);

  const track1 = chunk("MTrk", [
    ...textMeta("Piano"),
    0x00, 0xff, 0x51, 0x03, 0x07, 0xa1, 0x20, // 500000 µs → 120 BPM
    0x00, 0xff, 0x58, 0x04, 0x04, 0x02, 0x18, 0x08, // 4/4
    0x00, 0x90, 0x3c, 0x64, // C4 on
    0x00, 0x90, 0x40, 0x64, // E4 on（同一拍 = 和弦）
    0x00, 0xff, 0x05, 0x02, 0x6c, 0x61, // Lyric "la"
    ...vlq(480), 0x80, 0x3c, 0x40, // 1 拍后 off
    0x00, 0x80, 0x40, 0x40,
    0x00, 0x90, 0x43, 0x64, // G4 on
    0x00, 0xff, 0x05, 0x02, 0x6c, 0x75, // Lyric "lu"
    ...vlq(480), 0x80, 0x43, 0x40,
    0x00, 0xff, 0x2f, 0x00,
  ]);

  const track2 = chunk("MTrk", [
    ...textMeta("Bass"),
    0x00, 0xc0, 0x21, // program 33
    0x00, 0x90, 0x30, 0x50, // C3 on
    ...vlq(240), 0x80, 0x30, 0x40, // 半拍
    ...vlq(240), 0x90, 0x30, 0x50, // 第 1 拍
    ...vlq(480), 0x80, 0x30, 0x40, // 1 拍
    0x00, 0xff, 0x2f, 0x00,
  ]);

  return new Uint8Array([...header, ...track1, ...track2]);
}

console.log("MIDI 解析");
const midi = parseMidi(buildMidiFile());
check(midi.division === 480, `division = ${midi.division}`);
check(midi.tracks.length === 2, `tracks = ${midi.tracks.length}`);
check(midi.tracks[0]!.name === "Piano", `轨道名 = ${midi.tracks[0]!.name}`);
const first = midi.tracks[0]!.notes[0]!;
check(first.pitch === 60 && first.start === 0 && first.duration === 1, `第一个音 = ${JSON.stringify(first)}`);
// 歌词是时间线上的文本事件：同一拍的和弦要挂到**最高音**（旋律声部）上，不能挂到最低音
const chordAtZero = midi.tracks[0]!.notes.filter((n) => n.start === 0);
check(chordAtZero.length === 2, `第 1 拍有两个音（和弦）= ${chordAtZero.length}`);
check(chordAtZero.find((n) => n.pitch === 64)?.lyric === "la", `歌词挂在最高音上 = ${JSON.stringify(chordAtZero)}`);
check(chordAtZero.find((n) => n.pitch === 60)?.lyric === undefined, "低音不带歌词");
check(midi.tracks[0]!.notes.find((n) => n.pitch === 67)?.lyric === "lu", "第二拍的歌词");
const bassNotes = midi.tracks[1]!.notes;
check(
  bassNotes.length === 2 && Math.abs(bassNotes[1]!.start - 1) < 1e-9 && Math.abs(bassNotes[0]!.duration - 0.5) < 1e-9,
  `贝斯音符 = ${JSON.stringify(bassNotes)}`,
);
check(Math.abs((midi.tempos[0]?.bpm ?? 0) - 120) < 1e-6, `速度 = ${midi.tempos[0]?.bpm}`);
check(midi.meters[0]?.num === 4 && midi.meters[0]?.den === 4, `拍号 = ${JSON.stringify(midi.meters[0])}`);

const midiProject = midiToProject(midi, "imported-midi");
check(midiProject.tracks.length === 2, `工程轨道数 = ${midiProject.tracks.length}`);
check(midiProject.tracks[0]!.instrument?.plugin_id === "builtin.piano", "默认音色 = 内置钢琴");
check(midiProject.transport.tempo === 120, `工程速度 = ${midiProject.transport.tempo}`);
check(midiProject.tracks.every((t) => t.clips.length === 1 && t.clips[0]!.notes.length > 0), "每轨一个片段且有音符");
check(
  midiProject.tracks[0]!.clips[0]!.notes.filter((n) => n.lyric).length === 2,
  `工程里保留了 MIDI 歌词（${JSON.stringify(midiProject.tracks[0]!.clips[0]!.notes.map((n) => n.lyric))}）`,
);

/** 只有 0x01(Text) 的 MIDI：没有 Lyric 事件时应该退回用它。 */
function buildTextLyricMidi(): Uint8Array {
  const header = chunk("MThd", [0x00, 0x00, 0x00, 0x01, 0x01, 0xe0]);
  const track = chunk("MTrk", [
    0x00, 0x90, 0x3c, 0x64,
    0x00, 0xff, 0x01, 0x02, 0x6c, 0x61, // Text "la"
    ...vlq(480), 0x80, 0x3c, 0x40,
    0x00, 0xff, 0x2f, 0x00,
  ]);
  return new Uint8Array([...header, ...track]);
}
check(
  parseMidi(buildTextLyricMidi()).tracks[0]!.notes[0]!.lyric === "la",
  "整轨没有 Lyric 事件时退回 Text 事件",
);

// ---------------------------------------------------------------------------
// MusicXML：往返
// ---------------------------------------------------------------------------

function note(pitch: number, start: number, duration: number, extra: Partial<Note> = {}): Note {
  return { pitch, start, duration, velocity: 100, volume: 127, ...extra };
}

const source: Project = {
  name: "Round Trip",
  transport: {
    tempo: 100,
    time_sig_num: 4,
    time_sig_den: 4,
    keyframes: [{ beat: 8, tempo: 120, time_sig_num: 3, time_sig_den: 4 }],
  },
  tracks: [
    {
      id: 1,
      name: "Piano",
      instrument: null,
      effects: [],
      mixer: { gain_db: 0, pan: 0, mute: false, solo: false },
      clips: [
        {
          id: 1,
          start: 0,
          length: 16,
          notes: [
            note(72, 0, 1, { lyric: "小4" }),
            note(74, 1, 0.5, { lyric: "星" }),
            note(76, 1.5, 0.5, { lyric: "-" }),
            // 跨小节：会被拆成 2 段连音线，导入时必须合回 1 个音
            note(79, 3, 2.5),
            note(60, 4, 2),
            note(64, 4, 2),
            note(67, 4, 2),
            note(72, 6.5, 2),
          ],
        },
      ],
    },
    {
      id: 2,
      name: "Drums",
      instrument: null,
      effects: [],
      mixer: { gain_db: 0, pan: 0, mute: false, solo: false },
      meta: { role: "drums" },
      clips: [
        {
          id: 2,
          start: 0,
          length: 16,
          notes: [note(36, 0, 0.25, { lyric: "" }), note(38, 1, 0.25), note(42, 2, 0.25), note(49, 8, 0.25)],
        },
      ],
    },
  ],
  master: { gain_db: 0, pan: 0, mute: false, solo: false },
};

const exported = buildMusicXML(source, { grid: 0.25 });
check(exported.parts === 2, `导出 parts = ${exported.parts}`);

console.log("MusicXML 往返");
const parsed = parseMusicXML(exported.xml);
check(parsed.parts.length === 2, `解析出谱表 = ${parsed.parts.length}`);
check(parsed.parts[0]!.notes.length === 8, `钢琴音符数 = ${parsed.parts[0]!.notes.length}（期望 8：连音线已合并）`);
check(parsed.parts[0]!.notes.some((n) => n.lyric === "小"), "歌词往返");
check(parsed.parts[1]!.drum, "鼓谱被识别成打击乐谱表");
check(parsed.parts[1]!.notes.map((n) => n.pitch).includes(36), "鼓谱音高往返（F4 → 36）");
check(Math.abs((parsed.tempos[0]?.bpm ?? 0) - 100) < 1e-6, `速度 = ${parsed.tempos[0]?.bpm}`);
check(
  parsed.meters.some((m) => Math.abs(m.beat - 8) < 1e-6 && m.num === 3 && m.den === 4),
  `第 3 小节的 3/4 被读出来（${JSON.stringify(parsed.meters)}）`,
);

const back = scoreToProject(parsed, "round-trip");
check(back.tracks.length === 2, `工程轨道数 = ${back.tracks.length}`);
check(back.tracks[1]!.instrument?.plugin_id === "builtin.drums", "鼓轨拿到内置鼓组音色");
check(
  back.transport.keyframes?.some((k) => Math.abs(k.beat - 8) < 1e-6 && k.time_sig_num === 3) ?? false,
  "变拍号变成了关键点",
);
const backPiano = back.tracks[0]!.clips[0]!.notes;
check(backPiano.length === 8, `往返后钢琴音符数 = ${backPiano.length}`);
check(
  backPiano.some((n) => Math.abs(n.start - 3) < 1e-6 && Math.abs(n.duration - 2.5) < 1e-6),
  "跨小节长音往返后仍是 2.5 拍",
);

// ---------------------------------------------------------------------------
// 坏数据
// ---------------------------------------------------------------------------

console.log("坏数据");
let threw = false;
try {
  parseMusicXML("<score-partwise><part id='P1'></part></score-partwise>");
} catch {
  threw = true;
}
check(threw, "空谱面抛出可读错误");

let midiThrew = false;
try {
  parseMidi(new Uint8Array([1, 2, 3, 4]));
} catch {
  midiThrew = true;
}
check(midiThrew, "非 MIDI 文件抛错");

if (failures > 0) {
  console.error(`\n${failures} 项检查失败`);
  process.exit(1);
}
console.log("\n全部通过");
