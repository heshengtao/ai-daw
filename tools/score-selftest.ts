/**
 * 乐谱导出自检（不依赖测试框架，跑完退出码即结论）。
 *
 * 运行：`npx sucrase-node tools/score-selftest.ts`
 *
 * 覆盖的是「量化 → 分声部 → 跨小节连音线 → 补休止 → 序列化」这条链上最容易默默出错的地方：
 * - 每个小节里**每个声部的时值总和必须相等**（少一个休止符，后面的音就整体左移）；
 * - 连音线的 start / stop 必须配平；
 * - 每个音都要有 `<type>`（没有时值类型，MuseScore 会直接丢音）；
 * - 换拍号、换速度、歌词连音、鼓组、和弦标记解析失败，都要出现在该出现的地方。
 *
 * 产物写到 `tools/out/`，可以顺手丢进 MuseScore 看一眼真实排版。
 */
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { buildMusicXML } from "../src/score/musicxml";
import type { Note, Project, Track } from "../src/api/types";

// ---------------------------------------------------------------------------
// 样例工程
// ---------------------------------------------------------------------------

function note(pitch: number, start: number, duration: number, extra: Partial<Note> = {}): Note {
  return { pitch, start, duration, velocity: 100, volume: 127, ...extra };
}

const piano: Track = {
  id: 1,
  name: "Piano",
  instrument: null,
  effects: [],
  mixer: { gain_db: 0, pan: 0, mute: false, solo: false },
  clips: [
    {
      id: 1,
      start: 0,
      length: 24,
      notes: [
        // 第 1 小节：旋律 + 歌词（第二小节首音是连音记号 -）
        note(72, 0, 1, { lyric: "小4" }),
        note(74, 1, 0.5, { lyric: "星" }),
        note(76, 1.5, 0.5, { lyric: "-" }),
        note(79, 2, 1, { lyric: "亮" }),
        // 三连音（1/12 网格）
        note(77, 3, 1 / 3),
        note(79, 3 + 1 / 3, 1 / 3),
        note(81, 3 + 2 / 3, 1 / 3),
        // 和弦（同起点 = 一个声部里的和弦音）
        note(60, 4, 2),
        note(64, 4, 2),
        note(67, 4, 2),
        // 低音长音：起点错开旋律 → 必须落到第 2 声部（顺带测 `<backup>` 与跨小节连音线）
        note(48, 2.25, 6),
        // 跨小节线的长音（第 2 小节末尾 → 第 3 小节，必须拆成连音线）
        note(72, 6.5, 2),
        // 第 3 小节（关键点之后转 3/4）
        note(74, 8.5, 0.5),
        note(76, 9, 0.5),
        // 一个不可记谱的浮点时值，量化后应该变成 1/16 网格上的值
        note(77, 10.13, 0.37),
      ],
    },
  ],
};

const bass: Track = {
  id: 2,
  name: "Bass",
  instrument: null,
  effects: [],
  mixer: { gain_db: 0, pan: 0, mute: false, solo: false },
  meta: { role: "bass", pitch_range: { min: 28, max: 48 } },
  clips: [
    {
      id: 2,
      start: 0,
      length: 24,
      notes: [note(36, 0, 2), note(43, 2, 2), note(41, 4, 2), note(36, 6, 2), note(38, 8, 1)],
    },
  ],
};

const drums: Track = {
  id: 3,
  name: "Drums",
  instrument: null,
  effects: [],
  mixer: { gain_db: 0, pan: 0, mute: false, solo: false },
  meta: { role: "drums" },
  clips: [
    {
      id: 3,
      start: 0,
      length: 24,
      notes: [
        note(36, 0, 0.25),
        note(38, 1, 0.25),
        note(42, 2, 0.25),
        note(38, 3, 0.25),
        note(36, 4, 0.25),
        note(38, 5, 0.25),
        note(49, 8, 0.25),
      ],
    },
  ],
};

const project: Project = {
  name: "Score Self Test",
  transport: {
    tempo: 100,
    time_sig_num: 4,
    time_sig_den: 4,
    // 第 9 拍（第 3 小节）转 3/4 并提速
    keyframes: [{ beat: 8, tempo: 120, time_sig_num: 3, time_sig_den: 4 }],
  },
  tracks: [piano, bass, drums],
  master: { gain_db: 0, pan: 0, mute: false, solo: false },
  harmony: [
    { start: 0, length: 4, symbol: "Cmaj7" },
    { start: 4, length: 4, symbol: "Am7" },
    { start: 8, length: 3, symbol: "Fadd9" },
  ],
  sections: [{ name: "主歌", start: 0, end: 16 }],
};

// ---------------------------------------------------------------------------
// 断言
// ---------------------------------------------------------------------------

let failures = 0;

function check(ok: boolean, label: string): void {
  if (ok) {
    console.log(`  ok   ${label}`);
  } else {
    failures += 1;
    console.log(`  FAIL ${label}`);
  }
}

/** 极简 XML 良构检查：标签必须正确配对（不处理 CDATA / 注释，够用）。 */
function wellFormed(xml: string): boolean {
  const body = xml
    .replace(/<\?[\s\S]*?\?>/g, "")
    .replace(/<!DOCTYPE[\s\S]*?>/g, "");
  const stack: string[] = [];
  const re = /<(\/?)([A-Za-z][\w.-]*)((?:[^>"']|"[^"]*"|'[^']*')*?)(\/?)>/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(body))) {
    const closing = m[1] === "/";
    const name = m[2]!;
    const selfClosing = m[4] === "/";
    if (selfClosing) continue;
    if (closing) {
      if (stack.pop() !== name) return false;
    } else {
      stack.push(name);
    }
  }
  return stack.length === 0;
}

/** 取出所有小节体（跨 part 顺序展开）。 */
function measureBodies(xml: string): string[] {
  return [...xml.matchAll(/<measure [^>]*>([\s\S]*?)<\/measure>/g)].map((m) => m[1]!);
}

/**
 * 一个小节里每个声部的时值总和。
 *
 * `<backup>` 把时间轴拨回小节开头（换声部），所以各声部各自累加；和弦音（`<chord/>`）
 * 不推进时间。
 */
function voiceTotals(body: string): Map<string, number> {
  const totals = new Map<string, number>();
  const token = /<note>([\s\S]*?)<\/note>|<backup><duration>(\d+)<\/duration><\/backup>/g;
  let m: RegExpExecArray | null;
  while ((m = token.exec(body))) {
    if (m[2] !== undefined) continue; // backup：换声部，不参与累加
    const inner = m[1]!;
    const dur = Number(/<duration>(\d+)<\/duration>/.exec(inner)![1]);
    const voice = /<voice>(\d+)<\/voice>/.exec(inner)![1];
    const chord = /<chord\/>/.test(inner);
    if (!chord) totals.set(voice, (totals.get(voice) ?? 0) + dur);
  }
  return totals;
}

// ---------------------------------------------------------------------------
// 跑
// ---------------------------------------------------------------------------

console.log("buildMusicXML — 总谱");
const full = buildMusicXML(project, { grid: 0.25 });
check(full.parts === 3, `parts = ${full.parts}（期望 3）`);
// `trackIds` 决定渲染器里「第几个 part = 哪条轨道」——谱式/谱号/调弦全靠它逐轨对上，
// 顺序错了就会把鼓谱画到钢琴上。
check(
  JSON.stringify(full.trackIds) === JSON.stringify([1, 2, 3]),
  `trackIds = ${JSON.stringify(full.trackIds)}（期望 [1,2,3]，与 <part> 顺序一致）`,
);
check(full.notes === 27, `notes = ${full.notes}（期望 27：钢琴 15 + 贝斯 5 + 鼓 7）`);
check(/<backup><duration>\d+<\/duration><\/backup>/.test(full.xml), "多声部写了 <backup>");
check(wellFormed(full.xml), "XML 良构");
check(/<fifths>0<\/fifths>/.test(full.xml), "写了调号");
check(/<sign>F<\/sign><line>4<\/line>/.test(full.xml), "低音轨用了低音谱号");
check(/<sign>percussion<\/sign>/.test(full.xml), "鼓轨用了打击乐谱号");
check(/<unpitched>/.test(full.xml), "鼓轨写成了 unpitched");
check(/<rehearsal>主歌<\/rehearsal>/.test(full.xml), "段落排练号");
check(/<kind>major-seventh<\/kind>/.test(full.xml), "和弦标记 Cmaj7 → major-seventh");
check(/<words>Fadd9<\/words>/.test(full.xml), "解析不了的和弦退化成文字");
check(full.warnings.some((w) => w.includes("Fadd9")), "解析失败进了 warnings");
check(/<lyric number="1"><syllabic>begin<\/syllabic><text>星<\/text><extend\/><\/lyric>/.test(full.xml), "连音歌词写成 begin + extend");
check(!/<text>星4<\/text>/.test(full.xml), "歌词里的声调数字被去掉");
check(/<time><beats>3<\/beats><beat-type>4<\/beat-type><\/time>/.test(full.xml), "第 3 小节换了 3/4");
check(/<sound tempo="120"\/>/.test(full.xml), "速度变化写进谱面");
check(/<tie type="start"\/>/.test(full.xml) && /<tie type="stop"\/>/.test(full.xml), "跨小节长音拆出了连音线");

const bars = measureBodies(full.xml);
check(bars.length > 0, `小节数 = ${bars.length}`);
let voiceMismatch = 0;
let lengthMismatch = 0;
let missingType = 0;
let num = 4;
let den = 4;
for (const body of bars) {
  // 拍号变了就更新（没写 attributes 的小节沿用上一小节）
  const time = /<time><beats>(\d+)<\/beats><beat-type>(\d+)<\/beat-type><\/time>/.exec(body);
  if (time) {
    num = Number(time[1]);
    den = Number(time[2]);
  }
  const expected = Math.round(num * (4 / den) * 960);
  const totals = voiceTotals(body);
  const values = [...totals.values()];
  if (values.length === 0) continue;
  if (new Set(values).size > 1) voiceMismatch += 1;
  if (values.some((v) => v !== expected)) lengthMismatch += 1;
  for (const m of body.matchAll(/<note>([\s\S]*?)<\/note>/g)) {
    if (!/<type>/.test(m[1]!)) missingType += 1;
  }
}
check(voiceMismatch === 0, `各声部时值配平（不一致的小节 ${voiceMismatch} 个）`);
check(lengthMismatch === 0, `每小节时值 = 拍号长度（不一致的小节 ${lengthMismatch} 个）`);
check(missingType === 0, `每个音都有 <type>（缺失 ${missingType} 个）`);

/**
 * `<note>` 子元素的顺序必须符合 DTD（MusicXML 是 sequence，不是 all）。
 *
 * MuseScore 对顺序错误相当宽容，OSMD / 其它解析器就不一定了——所以这里按 DTD 的序列卡一遍。
 */
const NOTE_ORDER = [
  "grace",
  "cue",
  "chord",
  "pitch",
  "unpitched",
  "rest",
  "duration",
  "tie",
  "instrument",
  "footnote",
  "level",
  "voice",
  "type",
  "dot",
  "accidental",
  "time-modification",
  "stem",
  "notehead",
  "notehead-text",
  "staff",
  "beam",
  "notations",
  "lyric",
  "play",
  "listen",
];
let orderBroken = 0;
for (const body of bars) {
  for (const m of body.matchAll(/<note>([\s\S]*?)<\/note>/g)) {
    const inner = m[1]!;
    let last = -1;
    for (const child of inner.matchAll(/<([a-z-]+)[ />]/g)) {
      const name = child[1]!;
      const at = NOTE_ORDER.indexOf(name);
      if (at < 0) continue; // <text>/<syllabic> 之类属于 lyric 的子元素，不参与本层排序
      if (at < last) {
        orderBroken += 1;
        break;
      }
      last = at;
    }
  }
}
check(orderBroken === 0, `<note> 子元素顺序符合 DTD（违规 ${orderBroken} 处）`);

const starts = (full.xml.match(/<tie type="start"\/>/g) ?? []).length;
const stops = (full.xml.match(/<tie type="stop"\/>/g) ?? []).length;
check(starts === stops, `连音线配平（start ${starts} / stop ${stops}）`);

/** 每个小节的时值（取各声部里最大的那个），按文档顺序。 */
function measureLengths(xml: string): number[] {
  return measureBodies(xml).map((body) => Math.max(0, ...voiceTotals(body).values()));
}

console.log("buildMusicXML — 换拍号落在小节中间");
const midProject: Project = {
  ...project,
  // 第 7 拍（4/4 的第二小节中间）转 3/4：前一小节必须截成 2 拍的残小节
  transport: { tempo: 100, time_sig_num: 4, time_sig_den: 4, keyframes: [{ beat: 6, tempo: 100, time_sig_num: 3, time_sig_den: 4 }] },
  tracks: [piano],
  harmony: [],
  sections: [],
};
const mid = buildMusicXML(midProject, { grid: 0.25 });
check(mid.parts === 1, `parts = ${mid.parts}（期望 1）`);
check(wellFormed(mid.xml), "残小节 XML 良构");
const lengths = measureLengths(mid.xml);
check(lengths[0] === 3840 && lengths[1] === 1920 && lengths[2] === 2880, `小节长度 = ${lengths.join(" / ")}（期望 3840 / 1920 / 2880 …）`);

console.log("buildMusicXML — 音符落在小节线之外（量化边界）");
const edgeProject: Project = {
  ...project,
  // 3.99 + 0.01 量化后会跨到下一小节；这里必须靠「量化后的曲长」把最后小节补完整
  tracks: [{ ...piano, clips: [{ id: 9, start: 0, length: 8, notes: [note(60, 3.99, 0.01), note(62, 4.6, 1)] }] }],
  harmony: [],
  sections: [],
};
const edge = buildMusicXML(edgeProject, { grid: 0.25 });
check(wellFormed(edge.xml), "边界用例 XML 良构");
check(measureLengths(edge.xml).every((n) => n === 3840), `小节长度 = ${measureLengths(edge.xml).join(" / ")}（都应是 3840）`);

console.log("buildMusicXML — 只出一段（片段范围）");
const excerptProject: Project = {
  ...project,
  // 片段从第 7 拍开始（不是小节线！），谱面必须从它所在的第 2 小节起排
  tracks: [{ ...piano, clips: [{ id: 7, start: 6, length: 4, notes: [note(72, 0, 1), note(74, 1, 1)] }] }],
  harmony: [],
  sections: [],
};
const excerpt = buildMusicXML(excerptProject, { grid: 0.25, range: { start: 6, end: 10 } });
check(wellFormed(excerpt.xml), "片段谱 XML 良构");
check(
  excerpt.barStarts.length === 2 && excerpt.barStarts[0] === 4 && excerpt.barStarts[1] === 8,
  `小节起点 = ${JSON.stringify(excerpt.barStarts)}（期望 [4, 8]）`,
);
check(excerpt.notes === 2, `片段谱音符数 = ${excerpt.notes}`);
// 片段谱的 tick 从第 1 小节起算，播放头却是绝对拍：`barStarts[0]` 就是这两者之间的偏移，
// 谱面光标和「点谱跳转」都靠它，不能是 0 以外的任何别的值。
check(excerpt.barStarts[0] === 4, `片段谱起点偏移 = ${excerpt.barStarts[0]}（期望 4）`);
check(
  JSON.stringify(measureLengths(excerpt.xml)) === JSON.stringify([3840, 2880]),
  `片段谱小节长度 = ${JSON.stringify(measureLengths(excerpt.xml))}（第 1 小节 4/4，第 2 小节起转 3/4）`,
);

console.log("buildMusicXML — 六线谱");
const tab = buildMusicXML(project, { grid: 0.25, tracks: [1], notationFor: () => "guitar" });
check(wellFormed(tab.xml), "六线谱 XML 良构");
check(/<sign>TAB<\/sign>/.test(tab.xml), "谱号 = TAB");
check(/<staff-lines>6<\/staff-lines>/.test(tab.xml), "弦数 = 6");
check(/<staff-tuning line="1">/.test(tab.xml), "写了调弦");
check(/<technical><string>\d+<\/string><fret>\d+<\/fret><\/technical>/.test(tab.xml), "音符带弦/品分配");

console.log("buildMusicXML — 六线谱和弦的弦位");
// 同起点的 4 个音 = 一个和弦。逐个音各自挑「品数最小的弦」会让它们全挤到同一根弦上，
// 谱面上就是「两个数字叠在一个位置」。必须整组分配。
const chordGuitar: Track = {
  ...bass,
  id: 4,
  name: "Guitar",
  meta: { role: "guitar" },
  clips: [
    { id: 4, start: 0, length: 4, notes: [note(52, 0, 2), note(55, 0, 2), note(59, 0, 2), note(64, 0, 2)] },
  ],
};
const chordTab = buildMusicXML(
  { ...project, tracks: [chordGuitar], harmony: [], sections: [] },
  { grid: 0.25, tracks: [4], notationFor: () => "guitar" },
);
const chordStrings = [...chordTab.xml.matchAll(/<technical><string>(\d+)<\/string>/g)].map((m) => Number(m[1]!));
check(chordStrings.length === 4, `和弦 4 个音都带弦号（${chordStrings.length}）`);
check(
  new Set(chordStrings).size === chordStrings.length,
  `和弦各音落在不同的弦上（${chordStrings.join(",")}）`,
);

console.log("buildMusicXML — 分谱");
const part = buildMusicXML(project, { grid: 0.25, tracks: [1] });
check(part.parts === 1, `parts = ${part.parts}（期望 1）`);
check(JSON.stringify(part.trackIds) === JSON.stringify([1]), `trackIds = ${JSON.stringify(part.trackIds)}（期望 [1]）`);
check(!/<unpitched>/.test(part.xml), "分谱里没有鼓轨");
check(wellFormed(part.xml), "分谱 XML 良构");

console.log("buildMusicXML — 空工程");
const empty = buildMusicXML({ ...project, tracks: [] }, { grid: 0 });
check(empty.parts === 0 && empty.xml === "", "空工程不产出文件");

// ---------------------------------------------------------------------------
// 落盘（方便用 MuseScore 打开看排版）
// ---------------------------------------------------------------------------

const outDir = join(process.cwd(), "tools", "out");
mkdirSync(dirname(join(outDir, "score-full.musicxml")), { recursive: true });
writeFileSync(join(outDir, "score-full.musicxml"), full.xml, "utf8");
writeFileSync(join(outDir, "score-part.musicxml"), part.xml, "utf8");
console.log(`\n写出：tools/out/score-full.musicxml（${full.xml.length} 字节）`);

if (failures > 0) {
  console.error(`\n${failures} 项检查失败`);
  process.exit(1);
}
console.log("\n全部通过");
