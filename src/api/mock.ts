/**
 * 开发用假数据：只在「浏览器里且非 Tauri」时启用，用于在没有后端的情况下预览界面。
 * 生产构建（`vite build`）里 `import.meta.env.DEV` 为 false，本模块不会被用到。
 *
 * 除了工程数据，这里还**模拟**了一套实时传输：位置按设备采样率推进，并以与真实后端
 * 相同的 `engine://transport` 语义推送快照。这样界面在浏览器里就能完整验证播放头、
 * 定位与传输按钮，而不需要真的打开音频设备（浏览器里没有声音是预期行为）。
 */
import type {
  AgentConfig,
  AgentEvent,
  AgentProbe,
  AgentSession,
  AgentSessionSummary,
  AiModels,
  AiStatus,
  AnalysisReport,
  AutomationPoint,
  BackendInfo,
  BackendParam,
  BackendPreference,
  CatalogPlugin,
  Clip,
  ClipPlacement,
  ClipRef,
  FitMode,
  HarmonyBar,
  HarmonyReport,
  HistoryDepths,
  HistoryState,
  InputDeviceInfo,
  CliState,
  CliStatus,
  ForeignPlugin,
  ManagerInstallRequest,
  ManagerInstalled,
  ManagerProgress,
  ManagerPlugin,
  ManagerRegistry,
  ModelProgress,
  ModelSourcePreference,
  ModelStatus,
  ProxyMode,
  ProxyProbe,
  ProxyStatus,
  McpConfig,
  McpStatus,
  Note,
  NoteEdit,
  NotePatch,
  NoteRange,
  NoteStats,
  OverlapReport,
  PitchExpression,
  PitchRange,
  Preset,
  Project,
  ProjectHistoryEntry,
  Section,
  Send,
  StoreNetwork,
  TrackMeta,
  TrackOverlap,
  TransportSnapshot,
  VelocityShape,
} from "./types";

/** 写歌智能体：预览里没有真后端，配置与会话都只存在内存里。 */
let agentConfigState: AgentConfig = { base_url: "", api_key: "", model: "" };
const mockSessions = new Map<string, AgentSession>();

/** MCP 服务器：浏览器预览里没有真服务器，配置只存在内存里。 */
let mcpConfig: McpConfig = { enabled: true, port: 0 };
/** 端口填 0（自动）时，预览里假装分配到的端口。 */
const MCP_FALLBACK_PORT = 8787;

/** 预览版的 MCP 状态：开启即视为「正在监听」。 */
function mockMcpStatus(): McpStatus {
  const port = mcpConfig.port === 0 ? MCP_FALLBACK_PORT : mcpConfig.port;
  return {
    ...mcpConfig,
    running: mcpConfig.enabled,
    url: mcpConfig.enabled ? `http://127.0.0.1:${port}/mcp` : null,
  };
}

/** 预览版的 `aidaw` 挂载状态：开发构建没有真包，默认「未挂载」，但给得出全路径。 */
let cliState: CliState = "missing";

function mockCliStatus(): CliStatus {
  const bundled = "/Applications/AI DAW.app/Contents/MacOS/aidaw";
  return {
    state: cliState,
    command: "aidaw",
    target: cliState === "installed" ? "/usr/local/bin/aidaw" : null,
    bundled,
    conflict: null,
    hint: null,
    manual_command: null,
    full_path: bundled,
  };
}

/** 预览版的本地 AI：没有真实处理，模拟一段进度供界面演示进度条与停止。 */
let aiState: AiStatus = { running: false, kind: null, progress: 0, message: null };
let aiTimer: number | null = null;

function emitAi(): void {
  window.dispatchEvent(new CustomEvent("ai-daw-mock-ai", { detail: { ...aiState } }));
}

/** 插件管理器预览里的下载进度定时器（一次只跑一个）。 */
let mockManagerTimer: number | null = null;

function emitManager(p: ManagerProgress): void {
  window.dispatchEvent(new CustomEvent("ai-daw-mock-manager", { detail: p }));
}

/** 给单文件模型补齐派生字段（`files` / 计数），免得每条都手写一遍。 */
function oneFile(
  entry: Omit<ModelStatus, "files" | "total_files" | "installed_files">,
): ModelStatus {
  return {
    ...entry,
    total_files: 1,
    installed_files: entry.installed ? 1 : 0,
    files: [
      {
        filename: entry.filename,
        approx_bytes: entry.approx_bytes,
        installed: entry.installed,
        bytes: entry.bytes,
      },
    ],
  };
}

/** 预览里的模型清单（内存；下载完成后把 installed 置 true）。 */
let mockModels: ModelStatus[] = [
  oneFile({
    id: "basic-pitch",
    name: "Basic Pitch（音频转 MIDI）",
    feature: "to_midi",
    license: "Apache-2.0",
    approx_bytes: 230444,
    filename: "basic-pitch-nmp.onnx",
    bundled: true,
    installed: true,
    bytes: 230444,
    path: "/mock/models/basic-pitch-nmp.onnx",
    downloadable: true,
  }),
  oneFile({
    id: "localvqe",
    name: "LocalVQE（人声降噪 / 去混响）",
    feature: "enhance",
    license: "Apache-2.0",
    approx_bytes: 6152621,
    filename: "localvqe-ns.onnx",
    bundled: true,
    installed: true,
    bytes: 6152621,
    path: "/mock/models/localvqe-ns.onnx",
    downloadable: false,
  }),
  oneFile({
    id: "htdemucs-ort-v1",
    name: "HTDemucs（音源分离 · 4 分轨）",
    feature: "separate",
    license: "MIT",
    approx_bytes: 209884896,
    filename: "htdemucs.ort",
    bundled: false,
    installed: false,
    bytes: null,
    path: null,
    downloadable: true,
  }),
  oneFile({
    id: "htdemucs-6s",
    name: "HTDemucs 6s（音源分离 · 6 分轨）",
    feature: "separate",
    license: "MIT",
    approx_bytes: 258159781,
    filename: "htdemucs_6s.onnx",
    bundled: false,
    installed: false,
    bytes: null,
    path: null,
    downloadable: true,
  }),
  oneFile({
    id: "beat-this",
    name: "Beat This!（节拍 / 速度检测 · final0）",
    feature: "detect_tempo",
    license: "MIT",
    approx_bytes: 83162650,
    filename: "beat_this.onnx",
    bundled: false,
    installed: false,
    bytes: null,
    path: null,
    downloadable: true,
  }),
];

/** 模型下载预览定时器（一次只跑一个）。 */
let mockModelTimer: number | null = null;

function emitModel(p: ModelProgress): void {
  window.dispatchEvent(new CustomEvent("ai-daw-mock-model", { detail: p }));
}

/** 预览里的后端选择（内存）。设备固定成 Apple M4，所以 CoreML 可选。 */
let mockBackendPreference: BackendPreference = "cpu";

function mockBackendInfo(): BackendInfo {
  return {
    preference: mockBackendPreference,
    current: mockBackendPreference,
    platform: "macos / aarch64 · Apple M4",
    options: [
      { kind: "cpu", label: "CPU", note: "纯 CPU（默认，全平台可用）", available: true },
      {
        kind: "coreml",
        label: "Apple CoreML",
        note: "Apple 神经引擎 / GPU（实测在本机的这些小模型上比 CPU 慢）",
        available: true,
      },
      {
        kind: "directml",
        label: "DirectML",
        note: "DirectX 12 硬件加速（Windows）",
        available: false,
      },
    ],
  };
}

/** 是否使用假数据。 */
export function mockEnabled(): boolean {
  return import.meta.env.DEV && !("__TAURI_INTERNALS__" in window);
}

function triad(root: number, beat: number): Note[] {
  return [0, 4, 7].map((i) => ({ pitch: root + i, start: beat, duration: 1.6, velocity: 88, volume: 127 }));
}

function melody(): Note[] {
  return [72, 74, 76, 79, 76, 74, 72, 67].map((pitch, i) => ({
    pitch,
    start: i,
    duration: 0.7,
    velocity: 104,
    volume: 127,
  }));
}

function bass(): Note[] {
  return [36, 36, 41, 41, 43, 43, 36, 36].map((pitch, i) => ({
    pitch,
    start: i,
    duration: 0.9,
    velocity: 96,
    volume: 127,
  }));
}

function sample(): Project {
  return {
    name: "示例工程",
    transport: { tempo: 110, time_sig_num: 4, time_sig_den: 4, keyframes: [] },
    master: { gain_db: 0, pan: 0, mute: false, solo: false },
    tracks: [
      {
        id: 1,
        name: "和弦",
        instrument: { plugin_id: "builtin.synth.pad", params: { gain: 0.6 }, state: [] },
        effects: [],
        clips: [
          {
            id: 1,
            start: 0,
            length: 8,
            notes: [...triad(48, 0), ...triad(53, 2), ...triad(55, 4), ...triad(48, 6)],
          },
        ],
        mixer: { gain_db: -3, pan: 0, mute: false, solo: false },
      },
      {
        id: 2,
        name: "旋律",
        instrument: { plugin_id: "builtin.synth.lead", params: { gain: 0.8 }, state: [] },
        effects: [],
        clips: [{ id: 2, start: 0, length: 8, notes: melody() }],
        mixer: { gain_db: -1.5, pan: 0.15, mute: false, solo: false },
      },
      {
        id: 3,
        name: "低音",
        instrument: { plugin_id: "builtin.synth.bass", params: { gain: 0.7 }, state: [] },
        effects: [],
        clips: [{ id: 3, start: 0, length: 8, notes: bass() }],
        mixer: { gain_db: -4, pan: -0.1, mute: false, solo: false },
      },
    ],
  };
}

function empty(): Project {
  return {
    name: "untitled",
    transport: { tempo: 120, time_sig_num: 4, time_sig_den: 4, keyframes: [] },
    tracks: [],
    master: { gain_db: 0, pan: 0, mute: false, solo: false },
  };
}

/**
 * 新建工程的默认分轨，必须与引擎 `ai_daw_engine::STARTER_TRACKS` 保持一致
 * （顺序即自上而下的轨道顺序；`plugin_id` 是内置乐器 id）。
 */
const STARTER_TRACKS: { name: string; plugin_id: string }[] = [
  { name: "主旋律", plugin_id: "builtin.piano" },
  { name: "和弦", plugin_id: "builtin.nylon_guitar" },
  { name: "低音", plugin_id: "builtin.bass" },
  { name: "鼓组", plugin_id: "builtin.drums" },
  { name: "铺底", plugin_id: "builtin.synth.pad" },
];

/** 新增轨道时的默认音色。 */
const DEFAULT_INSTRUMENT = "builtin.synth";

/**
 * 浏览器预览用的假插件目录。
 *
 * 真实插件只能由 Tauri 后端扫描得到；这里给两个假条目，让「外部插件」这条 UI 通路
 * （清单、参数面板、效果链）在没有后端时也能跑通。
 */
const MOCK_PLUGINS: CatalogPlugin[] = [
  {
    id: "vst3:mock-juno",
    name: "Mock Juno（预览用）",
    vendor: "Preview",
    kind: "instrument",
    format: "vst3",
    accepts_midi: true,
    has_editor: true,
    path: "/mock/MockJuno.vst3",
  },
  {
    id: "vst3:mock-reverb",
    name: "Mock Reverb（预览用）",
    vendor: "Preview",
    kind: "effect",
    format: "vst3",
    accepts_midi: false,
    has_editor: true,
    path: "/mock/MockReverb.vst3",
  },
];

/** 假插件的参数表。 */
const MOCK_PARAMS: Record<string, BackendParam[]> = {
  "vst3:mock-juno": [
    { id: "0", name: "Cutoff", min: 0, max: 1, default: 0.6, automatable: true },
    { id: "1", name: "Resonance", min: 0, max: 1, default: 0.2, automatable: true },
    { id: "2", name: "Chorus", min: 0, max: 1, default: 0.5, automatable: true },
  ],
  "vst3:mock-reverb": [
    { id: "0", name: "Mix", min: 0, max: 1, default: 0.3, automatable: true },
    { id: "1", name: "Decay", min: 0, max: 1, default: 0.5, automatable: true },
  ],
};
/**
 * 插件管理器的假注册表（浏览器预览用）。
 *
 * 真数据来自开放注册表（Rust 侧拉取）；这里给几条覆盖各分支的样本：
 * 可安装的归档、只能打开下载页的安装器、以及一条可「更新」的旧版本记录。
 */
const MOCK_REGISTRY: ManagerPlugin[] = [
  {
    slug: "surge-synthesizer/surge",
    name: "Surge XT",
    author: "Surge Synth Team",
    description: "混合合成器：多种合成法、丰富的滤波器与调制引擎。",
    license: "gpl-3.0",
    kind: "instrument",
    tags: ["Synth", "Modulation"],
    image: null,
    homepage: "https://github.com/surge-synthesizer/surge",
    version: "1.3.4",
    verified: true,
    files: [
      {
        url: "https://example.com/surge-xt-macOS-1.3.4.dmg",
        sha256: "b56e8d51a8dc2382ae668c9ff4e96bae38dbd452af4e584d5a5ef50ab2d09b07",
        size: 416715686,
        kind: "installer",
        contains: ["vst3", "clap"],
        platforms: ["mac"],
        archs: ["arm64", "x64"],
        compatible: true,
      },
      {
        url: "https://example.com/surge-xt-linux-1.3.4-pluginsonly.tar.gz",
        sha256: "dd431b75f5fa197c4bffa6ca27ca46970f0a94c834119bb1db7decdeec4c28db",
        size: 97294804,
        kind: "archive",
        contains: ["vst3", "clap"],
        platforms: ["linux"],
        archs: ["x64"],
        compatible: false,
      },
    ],
  },
  {
    slug: "zl-audio/zlwarm",
    name: "ZL Warm",
    author: "ZL Audio",
    description: "失真 / 饱和效果器。",
    license: "gpl-3.0",
    kind: "effect",
    tags: ["Distortion", "Saturation"],
    image: null,
    homepage: "https://github.com/ZL-Audio/ZLWarm",
    version: "0.2.1",
    verified: true,
    files: [
      {
        url: "https://example.com/ZL.Warm-0.2.1.zip",
        sha256: "1e37bde56287423e075360cb44159bee4498a139144ff3c65c6d4e85e5198ef4",
        size: 14110770,
        kind: "archive",
        contains: ["vst3"],
        platforms: ["linux", "win", "mac"],
        archs: ["arm64", "x64"],
        compatible: true,
      },
    ],
  },
  {
    slug: "dfl/faveworm",
    name: "Faveworm",
    author: "David Löwenfels",
    description: "把音频画成发光模拟波束的 XY 示波器。",
    license: "mit",
    kind: "tool",
    tags: ["Oscilloscope", "Visualizer"],
    image: null,
    homepage: "https://github.com/dfl/faveworm",
    version: "1.0.1",
    verified: true,
    files: [
      {
        url: "https://example.com/faveworm-macos-universal.zip",
        sha256: "c459cd0646f5bede736d93ebeaf2353e04dcce70e1aaadb27e3201dfb2696cf3",
        size: 4471780,
        kind: "archive",
        contains: ["vst3", "clap"],
        platforms: ["mac"],
        archs: ["arm64", "x64"],
        compatible: true,
      },
    ],
  },
];

/** 预览里「本管理器装过的东西」；预置一条旧版本记录，用来演示「更新」。 */
let mockInstalled: ManagerInstalled[] = [
  {
    key: "zl-audio/zlwarm",
    slug: "zl-audio/zlwarm",
    name: "ZL Warm",
    version: "0.2.0",
    source: "registry",
    files: ["/mock/ZL Warm.vst3"],
    installed_at: 0,
  },
];

/** 预览里的插件源网络配置（内存，刷新即丢）。 */
let mockNetwork: StoreNetwork = {
  registry_mirrors: [],
  download_prefix: "",
  proxy_mode: "system",
  proxy_url: "",
  model_mirrors: [],
  model_source_preference: "auto",
  backend_preference: "cpu",
};

/** 铺上默认分轨的空工程。`names` 是界面按当前语言给出的分轨名（按模板顺序）。 */
function starterProject(names?: string[], tempo?: number): Project {
  const p = empty();
  if (Number.isFinite(tempo)) p.transport.tempo = Math.min(1000, Math.max(1, tempo!));
  p.tracks = STARTER_TRACKS.map(({ name, plugin_id }, i) => ({
    id: nextTrackId++,
    name: names?.[i] ?? name,
    instrument: { plugin_id, params: {}, state: [] },
    effects: [],
    clips: [],
    mixer: { gain_db: 0, pan: 0, mute: false, solo: false },
  }));
  return p;
}

/** 当前工程是否仍是「刚铺好的默认模板」（没动过）——与引擎侧同一套判断。 */
function isPristineStarter(): boolean {
  if (state.tracks.length !== STARTER_TRACKS.length) return false;
  return STARTER_TRACKS.every((starter, i) => {
    const track = state.tracks[i];
    return (
      track.name === starter.name &&
      track.clips.length === 0 &&
      track.effects.length === 0 &&
      track.instrument?.plugin_id === starter.plugin_id
    );
  });
}

// id 计数器必须先声明：初始状态（下方 `state = starterProject()`）在模块求值时就用到它。
let nextTrackId = 1;
let nextClipId = 1;

// 初始状态就是「新建工程」的结果：与真实后端 `Engine::new_project` 一致（带默认分轨、不带历史）。
let state: Project = starterProject();

/** 是否有未保存改动（预览里只按「有没有编辑过」粗略模拟）。 */
let dirty = false;
/** 当前工程的文件路径（预览里的假路径）。 */
let sessionPath: string | null = null;

/** 快照式撤销栈：浏览器预览里的撤销/重做与真实引擎行为一致。 */
const undoStack: Project[] = [];
const redoStack: Project[] = [];

/**
 * 历史工程：浏览器预览用 localStorage 模拟后端的 `history/` 目录。
 *
 * 只保留展示用的元信息 + 完整工程快照；最多 50 份，与后端一致。
 */
const HISTORY_KEY = "ai-daw-project-history";
const HISTORY_MAX = 50;

interface MockHistoryRecord {
  entry: ProjectHistoryEntry;
  project: Project;
}

function readHistory(): MockHistoryRecord[] {
  try {
    return JSON.parse(localStorage.getItem(HISTORY_KEY) ?? "[]") as MockHistoryRecord[];
  } catch {
    return [];
  }
}

function writeHistory(records: MockHistoryRecord[]): void {
  try {
    localStorage.setItem(HISTORY_KEY, JSON.stringify(records));
  } catch {
    // 存不下（配额满）就只保留本次会话可见的部分。
  }
}

function historyEntry(
  id: string,
  project: Project,
  path: string | null,
  wasDirty: boolean,
  savedAt: number,
): ProjectHistoryEntry {
  const tracks = project.tracks.map((track) => ({
    name: track.name,
    clips: track.clips.map((clip) => ({
      start: clip.start,
      length: clip.length,
      audio: clip.audio !== undefined,
    })),
  }));
  return {
    id,
    name: project.name,
    saved_at: savedAt,
    path,
    dirty: wasDirty,
    tempo: project.transport.tempo,
    end_beat: project.tracks
      .flatMap((track) => track.clips)
      .reduce((max, clip) => Math.max(max, clip.start + clip.length), 0),
    track_count: tracks.length,
    clip_count: tracks.reduce((sum, track) => sum + track.clips.length, 0),
    note_count: project.tracks
      .flatMap((track) => track.clips)
      .reduce((sum, clip) => sum + clip.notes.length, 0),
    tracks,
    fingerprint: JSON.stringify(project),
  };
}

/** 把当前工程记进历史（新建 / 打开 / 恢复前调用；空白模板不记）。 */
function recordHistory(): void {
  if (sessionPath === null && !dirty && state.tracks.every((track) => track.clips.length === 0)) return;
  const records = readHistory();
  const fp = JSON.stringify(state);
  const existing = records.findIndex(
    (record) => record.entry.fingerprint === fp && record.entry.path === sessionPath,
  );
  if (existing >= 0) {
    if (existing > 0) {
      const [record] = records.splice(existing, 1);
      if (record) records.unshift(record);
      writeHistory(records);
    }
    return;
  }

  const savedAt = Date.now();
  const id = `${savedAt.toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
  records.unshift({
    entry: historyEntry(id, state, sessionPath, dirty, savedAt),
    project: clone(state),
  });
  writeHistory(records.slice(0, HISTORY_MAX));
}

function clone(p: Project): Project {
  return structuredClone(p);
}

/** 先压栈旧状态再执行变更，返回新快照。 */
function commit(mutate: () => void): Project {
  undoStack.push(clone(state));
  if (undoStack.length > 200) undoStack.shift();
  redoStack.length = 0;
  mutate();
  dirty = true;
  return clone(state);
}

function track(id: number) {
  return state.tracks.find((t) => t.id === id);
}

/**
 * 预设：浏览器预览里用 localStorage 顶替后端的 `presets.json`。
 *
 * 桌面端预设落在应用配置目录（跨工程），预览里没有那个目录，用 localStorage 模拟同样的语义。
 */
const PRESET_KEY = "ai-daw-presets";

function readPresets(): Preset[] {
  try {
    return JSON.parse(localStorage.getItem(PRESET_KEY) ?? "[]") as Preset[];
  } catch {
    return [];
  }
}

function writePresets(presets: Preset[]): void {
  localStorage.setItem(PRESET_KEY, JSON.stringify(presets));
}

function clip(trackId: number, clipId: number) {
  return track(trackId)?.clips.find((c) => c.id === clipId);
}

// ---------------------------------------------------------------------------
// 编曲辅助：与引擎里的纯函数同语义（浏览器预览用）
// ---------------------------------------------------------------------------

/** 片段内下一个音符 id（最大值 + 1，与引擎一致）。 */
function nextNoteId(c: { notes: Note[] }): number {
  return c.notes.reduce((max, n) => Math.max(max, n.id ?? 0), 0) + 1;
}

/** 夹进片段范围：起点不早于 0、时值至少 1/32、不越出片段。 */
function clampNote(n: Note, length: number): Note {
  const MIN = 1 / 32;
  const expr = normalizeExpr(n.expr);
  const base: Note = expr ? { ...n, expr } : { ...n };
  if (!expr) delete base.expr;
  if (n.start < 0) return { ...base, duration: Math.max(MIN, n.duration) };
  const start = Math.min(Math.max(0, n.start), Math.max(0, length - MIN));
  const duration = Math.min(Math.max(MIN, n.duration), Math.max(MIN, length - start));
  return { ...base, start, duration };
}

/** 与引擎同语义：音分 ±2400、时间/延迟 0..256 拍、速率 0..20 Hz；全零 = 无表达。 */
function normalizeExpr(expr?: PitchExpression): PitchExpression | undefined {
  if (!expr) return undefined;
  const num = (v: number | undefined) => (Number.isFinite(v) ? (v as number) : 0);
  const cents = (v: number | undefined) => Math.max(-2400, Math.min(2400, num(v)));
  const out: PitchExpression = {};
  const bend = cents(expr.bend);
  const bendTo = cents(expr.bend_to);
  const bendTime = Math.max(0, Math.min(256, num(expr.bend_time)));
  const vibrato = Math.max(0, Math.min(2400, num(expr.vibrato)));
  const rate = Math.max(0, Math.min(20, num(expr.vibrato_rate)));
  const delay = Math.max(0, Math.min(256, num(expr.vibrato_delay)));
  const fade = Math.max(0, Math.min(256, num(expr.vibrato_fade)));
  if (bend) out.bend = bend;
  if (bendTo) out.bend_to = bendTo;
  if (bendTime) out.bend_time = bendTime;
  if (vibrato) {
    out.vibrato = vibrato;
    if (rate) out.vibrato_rate = rate;
    if (delay) out.vibrato_delay = delay;
    if (fade) out.vibrato_fade = fade;
  }
  return Object.keys(out).length > 0 ? out : undefined;
}

/** 范围是**片段内**拍、左闭右开；不给 = 整段。 */
function inRange(n: Note, range?: NoteRange): boolean {
  if (!range) return true;
  const lo = Math.min(range.start, range.end);
  const hi = Math.max(range.start, range.end);
  return n.start >= lo && n.start < hi;
}

/** 只对范围内的音符做变换，并夹回片段范围。 */
function transform(c: Clip, range: NoteRange | undefined, fn: (n: Note) => Note): void {
  c.notes = c.notes.map((n) => (inRange(n, range) ? clampNote(fn(n), c.length) : n));
}

/** 一小节的拍数（4/4 = 4）。 */
function barLength(): number {
  const den = state.transport.time_sig_den || 4;
  return (state.transport.time_sig_num * 4) / den;
}

/** 确定性伪随机（mulberry32）：同一个种子给同一串数。 */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// ---------------------------------------------------------------------------
// 模拟实时传输
// ---------------------------------------------------------------------------

const transport: TransportSnapshot = {
  available: true,
  playing: false,
  position_frames: 0,
  sample_rate: 48_000,
  loop: false,
  loop_start_frames: 0,
  loop_end_frames: 0,
  error: null,
};

let transportTimer: number | null = null;
let lastTickAt = 0;
let lastEmitted = "";

/** 工程内容结束帧（用于「播到结尾即停」）。 */
function projectEndFrames(): number {
  const beats = state.tracks
    .flatMap((t) => t.clips)
    .reduce((m, c) => Math.max(m, c.start + c.length), 0);
  const tempo = state.transport.tempo || 120;
  return Math.round((beats * 60) / tempo * transport.sample_rate);
}

/** 当前生效的播放范围（帧）；`null` = 没设范围。与引擎 `Player::loop_range` 同义。 */
function loopRange(): { start: number; end: number } | null {
  const start = transport.loop_start_frames;
  const end = transport.loop_end_frames;
  return end > start ? { start, end } : null;
}

/** 播放头到达此帧就该回卷 / 停下。与引擎 `Player::playback_end_frame` 同义。 */
function playbackBoundary(): number {
  if (transport.loop) {
    const range = loopRange();
    if (range) return range.end;
  }
  return projectEndFrames();
}

function emitTransport(): void {
  const key = JSON.stringify(transport);
  if (key === lastEmitted) return;
  lastEmitted = key;
  window.dispatchEvent(new CustomEvent("ai-daw-mock-transport", { detail: { ...transport } }));
}

function tick(): void {
  const now = performance.now();
  const dt = Math.max(0, (now - lastTickAt) / 1000);
  lastTickAt = now;
  if (transport.playing) {
    transport.position_frames += Math.round(dt * transport.sample_rate);
    const end = playbackBoundary();
    if (end > 0 && transport.position_frames >= end) {
      if (transport.loop) {
        // 有范围就在范围内回卷（起点 + 溢出量），否则整首回卷。
        const range = loopRange();
        const from = range ? range.start : 0;
        transport.position_frames = from + ((transport.position_frames - from) % (end - from));
      } else {
        transport.position_frames = end;
        transport.playing = false;
      }
    }
  }
  emitTransport();
  if (!transport.playing && transportTimer !== null) {
    window.clearInterval(transportTimer);
    transportTimer = null;
  }
}

function ensureTransportTimer(): void {
  if (transportTimer !== null) return;
  lastTickAt = performance.now();
  transportTimer = window.setInterval(tick, 33);
}

// ---------------------------------------------------------------------------
// 假数据版的后端 API
// ---------------------------------------------------------------------------

/** 假数据版的后端 API。 */
export const mockApi = {
  getProject: (): Project => clone(state),

  addTrack: (name: string): Project =>
    commit(() => {
      state.tracks.push({
        id: nextTrackId++,
        name,
        instrument: { plugin_id: DEFAULT_INSTRUMENT, params: {}, state: [] },
        effects: [],
        clips: [{ id: nextClipId++, start: 0, length: 4, notes: [] }],
        mixer: { gain_db: 0, pan: 0, mute: false, solo: false },
      });
    }),

  removeTrack: (id: number): Project =>
    commit(() => {
      state.tracks = state.tracks.filter((t) => t.id !== id);
    }),

  renameTrack: (id: number, name: string): Project =>
    commit(() => {
      const t = track(id);
      if (t) t.name = name;
    }),

  setTrackInstrument: (id: number, pluginId: string): Project =>
    commit(() => {
      const t = track(id);
      if (t) t.instrument = pluginId ? { plugin_id: pluginId, params: {}, state: [] } : null;
    }),

  setParam: (id: number, param: string, value: number): Project =>
    commit(() => {
      const t = track(id);
      if (t?.instrument && Number.isFinite(value)) t.instrument.params[param] = value;
    }),

  // ---- 效果链 ----

  addEffect: (trackId: number, pluginId: string): Project =>
    commit(() => {
      const t = track(trackId);
      if (t) t.effects.push({ plugin_id: pluginId, params: {}, state: [] });
    }),

  removeEffect: (trackId: number, index: number): Project =>
    commit(() => {
      const t = track(trackId);
      if (t && index >= 0 && index < t.effects.length) t.effects.splice(index, 1);
    }),

  moveEffect: (trackId: number, from: number, to: number): Project =>
    commit(() => {
      const t = track(trackId);
      if (!t || from < 0 || from >= t.effects.length) return;
      const [fx] = t.effects.splice(from, 1);
      t.effects.splice(Math.min(to, t.effects.length), 0, fx);
    }),

  setEffectParam: (trackId: number, index: number, param: string, value: number): Project =>
    commit(() => {
      const fx = track(trackId)?.effects[index];
      if (fx && Number.isFinite(value)) fx.params[param] = value;
    }),

  // ---- 插件目录 ----

  listPlugins: (): CatalogPlugin[] => [...MOCK_PLUGINS],

  rescanPlugins: (): CatalogPlugin[] => [...MOCK_PLUGINS],

  pluginParams: (pluginId: string): BackendParam[] => MOCK_PARAMS[pluginId] ?? [],

  // ---- 插件管理器（预览：内存里的假注册表与假安装记录）----

  managerRegistry: (_force?: boolean): ManagerRegistry => ({
    plugins: MOCK_REGISTRY.map((p) => ({ ...p, files: p.files.map((f) => ({ ...f })) })),
    platform: "mac",
    arch: "arm64",
    fetched_at: Math.floor(Date.now() / 1000),
    from_cache: false,
  }),

  managerInstalled: (): ManagerInstalled[] => mockInstalled.map((r) => ({ ...r, files: [...r.files] })),

  // 预览里把假插件目录当成「用户自己装的」（它们不在托管库里）。
  managerForeign: (): ForeignPlugin[] =>
    MOCK_PLUGINS.map((p) => ({
      id: p.id,
      name: p.name,
      vendor: p.vendor,
      kind: p.kind,
      format: p.format,
      path: p.path,
    })),

  // 预览里模拟一次下载：跑几条进度事件（含「断点续传」的 80% 处顿一下），
  // 好让浏览器里也能看到卡片下边缘的进度条。
  managerInstall: (req: ManagerInstallRequest): void => {
    const total = req.size || 48 * 1024 * 1024;
    let received = 0;
    mockManagerTimer = window.setInterval(() => {
      received = Math.min(total, received + total * 0.12);
      emitManager({
        slug: req.slug,
        phase: "downloading",
        received,
        total,
        message: null,
      });
      if (received >= total) {
        window.clearInterval(mockManagerTimer!);
        mockManagerTimer = null;
        emitManager({ slug: req.slug, phase: "installing", received: 0, total: 0, message: null });
        mockInstalled = mockInstalled.filter((r) => r.key !== req.slug);
        mockInstalled.push({
          key: req.slug,
          slug: req.slug,
          name: req.name,
          version: req.version,
          source: "registry",
          files: [`/mock/${req.name}.vst3`],
          installed_at: Math.floor(Date.now() / 1000),
        });
      }
    }, 160);
  },

  managerResolveUrl: (url: string): string =>
    mockNetwork.download_prefix && url.includes("github.com")
      ? `${mockNetwork.download_prefix}${url}`
      : url,

  onManagerEvent: (): (() => void) => () => {
    if (mockManagerTimer !== null) {
      window.clearInterval(mockManagerTimer);
      mockManagerTimer = null;
    }
  },

  managerImport: (path: string): void => {
    const name = path.split("/").pop() ?? "Imported";
    const key = `local:${name}`;
    mockInstalled = mockInstalled.filter((r) => r.key !== key);
    mockInstalled.push({
      key,
      slug: null,
      name,
      version: "",
      source: "local",
      files: [path],
      installed_at: Math.floor(Date.now() / 1000),
    });
  },

  managerRemove: (key: string): void => {
    mockInstalled = mockInstalled.filter((r) => r.key !== key);
  },

  managerOpen: (_url?: string): Promise<void> => Promise.resolve(),

  // 预览里网络配置只存在内存里（刷新即丢），语义与真实后端一致。
  managerNetwork: (): StoreNetwork => ({ ...mockNetwork }),

  managerSetNetwork: (
    registryMirrors: string[],
    downloadPrefix: string,
    proxyMode: ProxyMode,
    proxyUrl: string,
    modelMirrors: string[] = [],
    modelSourcePreference: ModelSourcePreference = "auto",
  ): StoreNetwork => {
    mockNetwork = {
      registry_mirrors: [...registryMirrors],
      download_prefix: downloadPrefix,
      proxy_mode: proxyMode,
      proxy_url: proxyUrl,
      model_mirrors: [...modelMirrors],
      model_source_preference: modelSourcePreference,
      // 执行后端偏好不在这里改（由 `aiSetBackend` 单独管）。
      backend_preference: mockNetwork.backend_preference,
    };
    return { ...mockNetwork };
  },

  // 浏览器里读不到系统代理（那是操作系统的设置），如实说明。
  managerProxyStatus: (): ProxyStatus => {
    if (mockNetwork.proxy_mode === "manual" && mockNetwork.proxy_url) {
      return { proxy: mockNetwork.proxy_url, source: `手动代理 ${mockNetwork.proxy_url}` };
    }
    if (mockNetwork.proxy_mode === "system") {
      return { proxy: null, source: "浏览器预览：读不到系统代理 → 直连" };
    }
    return { proxy: null, source: "直连（不使用代理）" };
  },

  managerProbeProxy: (proxyMode: ProxyMode, proxyUrl: string): ProxyProbe => {
    const status = mockApi.managerProxyStatus();
    const proxy = proxyMode === "manual" && proxyUrl ? proxyUrl : status.proxy;
    const source =
      proxyMode === "manual"
        ? `手动代理 ${proxyUrl}`
        : proxyMode === "system"
          ? "浏览器预览：读不到系统代理 → 直连"
          : "直连（不使用代理）";
    return { proxy, source, ok: false, detail: "浏览器预览不发真实请求" };
  },

  // ---- 模型下载器（预览：内存里的假清单 + 假进度）----

  modelCatalog: (): ModelStatus[] => mockModels.map((m) => ({ ...m })),

  modelDownload: (id: string): void => {
    if (mockModelTimer !== null) return;
    const entry = mockModels.find((m) => m.id === id);
    if (!entry) return;
    const total = entry.approx_bytes;
    let received = 0;
    mockModelTimer = window.setInterval(() => {
      received = Math.min(total, received + total * 0.08);
      emitModel({ id, state: "downloading", received, total, message: null });
      if (received >= total) {
        window.clearInterval(mockModelTimer!);
        mockModelTimer = null;
        mockModels = mockModels.map((m) =>
          m.id === id
            ? {
                ...m,
                installed: true,
                bytes: total,
                path: `/mock/models/${m.filename}`,
                // 组条目一并把「组内文件」全点亮，否则面板上的「已下 n/N」对不上。
                installed_files: m.total_files,
                files: m.files.map((f) => ({
                  ...f,
                  installed: true,
                  bytes: f.approx_bytes,
                })),
              }
            : m,
        );
        emitModel({ id, state: "done", received: total, total, message: null });
      }
    }, 140);
  },

  modelCancel: (): void => {
    if (mockModelTimer !== null) {
      window.clearInterval(mockModelTimer);
      mockModelTimer = null;
    }
  },

  modelRemove: (id: string): void => {
    mockModels = mockModels.map((m) =>
      m.id === id
        ? {
            ...m,
            installed: false,
            bytes: null,
            path: null,
            installed_files: 0,
            files: m.files.map((f) => ({ ...f, installed: false, bytes: null })),
          }
        : m,
    );
  },

  // 「复制下载链接」：套上配置的模型镜像前缀（与真实后端一致）。
  modelResolveUrl: (id: string): string => {
    const url = `https://huggingface.co/gentij/${id}/resolve/main/${id}`;
    const prefix = mockNetwork.model_mirrors[0] ?? "";
    if (!prefix) return url;
    return prefix.includes("{url}") ? prefix.replace("{url}", url) : `${prefix}${url}`;
  },

  onModelEvent: (): (() => void) => () => {
    if (mockModelTimer !== null) {
      window.clearInterval(mockModelTimer);
      mockModelTimer = null;
    }
  },

  // ---- 执行后端（预览：内存里的假清单 + 假进度）----

  aiBackendInfo: (): BackendInfo => mockBackendInfo(),

  aiSetBackend: (preference: BackendPreference): BackendInfo => {
    mockBackendPreference = preference;
    return mockBackendInfo();
  },

  // ---- 插件原生界面（浏览器里没有原生窗口） ----

  openPluginWindow: (): Promise<void> =>
    Promise.reject(new Error("浏览器预览不支持原生插件窗口，请在桌面端使用")),

  closePluginWindow: (): Promise<void> => Promise.resolve(),

  pluginWindowOpen: (): boolean => false,

  // 浏览器里没有独立窗口：调用方会退回侧边栏面板。
  openSynthWindow: (): Promise<void> =>
    Promise.reject(new Error("浏览器预览没有独立窗口，已改用侧边栏面板")),

  openEffectWindow: (): Promise<void> =>
    Promise.reject(new Error("浏览器预览没有独立窗口，已改用侧边栏面板")),

  // 外部效果器的原生界面同理：桌面端才有。
  openEffectPluginWindow: (): Promise<void> =>
    Promise.reject(new Error("浏览器预览不支持原生插件窗口，请在桌面端使用")),

  closeEffectPluginWindow: (): Promise<void> => Promise.resolve(),

  effectPluginWindowOpen: (): boolean => false,

  // 预览里没有真实音频：画一段正弦，让示波器看得出在动。
  scopeData: (points: number): number[] => {
    const t = performance.now() / 1000;
    const count = Math.max(16, points);
    return Array.from({ length: count }, (_, i) => {
      const phase = i / count + t * 0.5;
      const env = 0.35 + 0.25 * Math.sin(t * 0.7);
      return env * Math.sin(phase * Math.PI * 12);
    });
  },

  auditionNote: (): Promise<void> => Promise.resolve(),

  // ---- 用户预设（浏览器预览用 localStorage 顶替 presets.json） ----

  setParams: (trackId: number, params: Record<string, number>): Project =>
    commit(() => {
      const inst = track(trackId)?.instrument;
      if (!inst) return;
      for (const [id, value] of Object.entries(params)) {
        if (Number.isFinite(value)) inst.params[id] = value;
      }
    }),

  setEffectParams: (trackId: number, index: number, params: Record<string, number>): Project =>
    commit(() => {
      const fx = track(trackId)?.effects[index];
      if (!fx) return;
      for (const [id, value] of Object.entries(params)) {
        if (Number.isFinite(value)) fx.params[id] = value;
      }
    }),

  listPresets: (): Preset[] => readPresets(),

  savePreset: (name: string, pluginId: string, params: Record<string, number>): Preset => {
    const now = Date.now();
    const preset: Preset = {
      id: `p${now}-${readPresets().length}`,
      name: name.trim(),
      plugin_id: pluginId,
      params: { ...params },
      created_at: now,
    };
    writePresets([...readPresets(), preset]);
    return preset;
  },

  updatePreset: (id: string, name: string, params: Record<string, number>): Preset => {
    const all = readPresets();
    const found = all.find((p) => p.id === id);
    if (!found) throw new Error(`预设不存在：${id}`);
    found.name = name.trim();
    found.params = { ...params };
    writePresets(all);
    return found;
  },

  deletePreset: (id: string): void => {
    writePresets(readPresets().filter((p) => p.id !== id));
  },

  setTempo: (bpm: number): Project =>
    commit(() => {
      state.transport.tempo = bpm;
    }),

  setTimeSignature: (num: number, den: number): Project =>
    commit(() => {
      state.transport.time_sig_num = num;
      state.transport.time_sig_den = den;
    }),

  setKeyframe: (beat: number, tempo: number, num: number, den: number): Project =>
    commit(() => {
      const kf = { beat, tempo: Math.min(1000, Math.max(1, tempo)), time_sig_num: num, time_sig_den: den };
      state.transport.keyframes = (state.transport.keyframes ?? [])
        .filter((k) => Math.abs(k.beat - beat) > 1e-6)
        .concat(kf)
        .sort((a, b) => a.beat - b.beat);
    }),

  removeKeyframe: (beat: number): Project =>
    commit(() => {
      state.transport.keyframes = (state.transport.keyframes ?? []).filter(
        (k) => Math.abs(k.beat - beat) > 1e-6,
      );
    }),

  newProject: (trackNames?: string[], tempo?: number): Project => {
    // 新建会清空当前工程：先记进历史（与后端 `new_project` 同一行为）。
    recordHistory();
    transport.position_frames = 0;
    transport.playing = false;
    const next = commit(() => {
      state = starterProject(trackNames, tempo);
    });
    // 新建的工程是干净的，也没有对应的文件。
    dirty = false;
    sessionPath = null;
    emitTransport();
    return next;
  },

  renameProject: (name: string): Project =>
    commit(() => {
      state.name = name;
    }),

  /** 与后端一致：只在工程仍是默认模板时才按语言改名，且不进撤销栈。 */
  localizeStarterTracks: (names: string[]): Project => {
    if (names.length !== STARTER_TRACKS.length || !isPristineStarter()) return clone(state);
    state.tracks.forEach((track, i) => {
      track.name = names[i];
    });
    return clone(state);
  },

  addClip: (trackId: number, start: number, length: number): Project =>
    commit(() => {
      const t = track(trackId);
      if (t) t.clips.push({ id: nextClipId++, start: Math.max(0, start), length: Math.max(0.125, length), notes: [] });
    }),

  removeClip: (trackId: number, clipId: number): Project =>
    commit(() => {
      const t = track(trackId);
      if (t) t.clips = t.clips.filter((c) => c.id !== clipId);
    }),

  moveClip: (trackId: number, clipId: number, start: number): Project =>
    commit(() => {
      const c = clip(trackId, clipId);
      if (c) c.start = Math.max(0, start);
    }),

  moveClips: (items: ClipPlacement[]): Project =>
    commit(() => {
      for (const item of items) {
        const src = track(item.track);
        const dest = track(item.to_track);
        if (!src || !dest) continue; // 目标轨道不存在：整条跳过，不丢片段
        const idx = src.clips.findIndex((c) => c.id === item.clip);
        if (idx < 0) continue;
        const [c] = src.clips.splice(idx, 1);
        c.start = Math.max(0, item.start);
        // 与引擎一致：按起点有序插入
        const pos = dest.clips.findIndex((x) => x.start > c.start);
        if (pos < 0) dest.clips.push(c);
        else dest.clips.splice(pos, 0, c);
      }
    }),

  resizeClip: (trackId: number, clipId: number, length: number): Project =>
    commit(() => {
      const c = clip(trackId, clipId);
      if (c) c.length = Math.max(0.125, length);
    }),

  setClipRate: (trackId: number, clipId: number, rate: number, pitchSemitones: number): Project =>
    commit(() => {
      const c = clip(trackId, clipId);
      if (c?.audio) {
        c.audio.rate = rate;
        c.audio.pitch_semitones = pitchSemitones;
      }
    }),

  trimClip: (trackId: number, clipId: number, start: number, length: number): Project =>
    commit(() => {
      const c = clip(trackId, clipId);
      if (!c) return;
      c.start = Math.max(0, start);
      c.length = Math.max(0.125, length);
    }),

  addNote: (trackId: number, clipId: number, note: Note): Project =>
    commit(() => {
      const c = clip(trackId, clipId);
      if (c) c.notes.push({ ...note });
    }),

  setNote: (trackId: number, clipId: number, index: number, note: Note): Project =>
    commit(() => {
      const c = clip(trackId, clipId);
      if (c && index >= 0 && index < c.notes.length) c.notes[index] = { ...note };
    }),

  setNotes: (trackId: number, clipId: number, entries: NoteEdit[]): Project =>
    commit(() => {
      const c = clip(trackId, clipId);
      if (!c) return;
      for (const e of entries) {
        if (e.index >= 0 && e.index < c.notes.length) c.notes[e.index] = { ...e.note };
      }
    }),

  removeNote: (trackId: number, clipId: number, index: number): Project =>
    commit(() => {
      const c = clip(trackId, clipId);
      if (c && index >= 0 && index < c.notes.length) c.notes.splice(index, 1);
    }),

  removeNotes: (trackId: number, clipId: number, indices: number[]): Project =>
    commit(() => {
      const c = clip(trackId, clipId);
      if (!c) return;
      // 与引擎一致：去重 + 越界过滤 + 倒序删除。
      const targets = [...new Set(indices)].filter((i) => i >= 0 && i < c.notes.length).sort((a, b) => b - a);
      for (const i of targets) c.notes.splice(i, 1);
    }),

  removeClips: (items: ClipRef[]): Project =>
    commit(() => {
      for (const item of items) {
        const t = track(item.track);
        if (t) t.clips = t.clips.filter((c) => c.id !== item.clip);
      }
    }),

  // ---- 编曲：批量音符与变换（与引擎同语义，供浏览器预览） ----

  addNotes: (trackId: number, clipId: number, notes: Note[]): Project =>
    commit(() => {
      const c = clip(trackId, clipId);
      if (!c) return;
      let next = nextNoteId(c);
      for (const n of notes) c.notes.push(clampNote({ ...n, id: next++ }, c.length));
    }),

  replaceNotes: (trackId: number, clipId: number, notes: Note[]): Project =>
    commit(() => {
      const c = clip(trackId, clipId);
      if (!c) return;
      c.notes = notes.map((n, i) => clampNote({ ...n, id: i + 1 }, c.length));
    }),

  updateNotes: (trackId: number, clipId: number, updates: NotePatch[]): Project =>
    commit(() => {
      const c = clip(trackId, clipId);
      if (!c) return;
      for (const patch of updates) {
        const i = c.notes.findIndex((n) => n.id === patch.id);
        if (i < 0) continue;
        const merged = { ...c.notes[i], ...patch, id: c.notes[i].id };
        c.notes[i] = clampNote(merged, c.length);
      }
    }),

  removeNotesById: (trackId: number, clipId: number, ids: number[]): Project =>
    commit(() => {
      const c = clip(trackId, clipId);
      if (!c) return;
      const drop = new Set(ids);
      c.notes = c.notes.filter((n) => !drop.has(n.id ?? 0));
    }),

  transposeNotes: (trackId: number, clipId: number, semitones: number, range?: NoteRange): Project =>
    commit(() => {
      const c = clip(trackId, clipId);
      if (!c || semitones === 0) return;
      transform(c, range, (n) => ({ ...n, pitch: Math.min(127, Math.max(0, n.pitch + semitones)) }));
    }),

  setNoteVelocity: (
    trackId: number,
    clipId: number,
    shape: VelocityShape,
    range?: NoteRange,
  ): Project =>
    commit(() => {
      const c = clip(trackId, clipId);
      if (!c) return;
      const picked = c.notes.filter((n) => inRange(n, range));
      if (picked.length === 0) return;
      const lo = range ? Math.min(range.start, range.end) : Math.min(...picked.map((n) => n.start));
      const hi = range ? Math.max(range.start, range.end) : Math.max(...picked.map((n) => n.start));
      transform(c, range, (n) => {
        let value: number;
        if ("fixed" in shape) value = shape.fixed;
        else if ("scale" in shape) value = n.velocity * shape.scale;
        else {
          const span = hi - lo;
          const t = span > 0 ? Math.min(1, Math.max(0, (n.start - lo) / span)) : 0;
          value = shape.ramp.from + (shape.ramp.to - shape.ramp.from) * t;
        }
        return { ...n, velocity: Math.min(127, Math.max(0, Math.round(value))) };
      });
    }),

  quantizeNotes: (
    trackId: number,
    clipId: number,
    grid: number,
    strength: number,
    range?: NoteRange,
  ): Project =>
    commit(() => {
      const c = clip(trackId, clipId);
      if (!c) return;
      const g = Math.min(4, Math.max(1 / 64, grid));
      const s = Math.min(1, Math.max(0, strength));
      if (s <= 0) return;
      transform(c, range, (n) => ({ ...n, start: n.start + (Math.round(n.start / g) * g - n.start) * s }));
    }),

  strumNotes: (
    trackId: number,
    clipId: number,
    stepMs = 30,
    direction: "down" | "up" = "down",
  ): Project =>
    commit(() => {
      const c = clip(trackId, clipId);
      if (!c) return;
      // 与引擎同一套语义：按**起点**分组、组内按音高排序、写递增偏移。毫秒→拍用基础速度
      // （mock 不处理速度关键点，预览里够用）。
      const beats = (Math.max(0, stepMs) / 1000) * ((state.transport.tempo || 120) / 60);
      const groups = new Map<number, number[]>();
      c.notes.forEach((n, i) => {
        const key = Math.round(n.start * 1e6);
        const bucket = groups.get(key);
        if (bucket) bucket.push(i);
        else groups.set(key, [i]);
      });
      for (const indices of groups.values()) {
        const ordered = [...indices].sort((a, b) => {
          const left = c.notes[a]!.pitch;
          const right = c.notes[b]!.pitch;
          return direction === "down" ? left - right || a - b : right - left || a - b;
        });
        ordered.forEach((index, rank) => {
          c.notes[index] = { ...c.notes[index]!, offset: rank * beats };
        });
      }
    }),

  applySwing: (
    trackId: number,
    clipId: number,
    subdivision: number,
    ratio: number,
    strength: number,
    range?: NoteRange,
  ): Project =>
    commit(() => {
      const c = clip(trackId, clipId);
      if (!c) return;
      const sub = Math.min(4, Math.max(1 / 64, subdivision));
      const r = Math.min(0.8, Math.max(0.5, ratio));
      const s = Math.min(1, Math.max(0, strength));
      if (s <= 0 || Math.abs(r - 0.5) < 1e-9) return;
      transform(c, range, (n) => {
        if (n.start < 0) return n;
        const slot = Math.floor(n.start / sub);
        if (slot % 2 === 0) return n;
        const target = (slot - 1) * sub + r * 2 * sub;
        return { ...n, start: n.start + (target - n.start) * s };
      });
    }),

  humanizeNotes: (
    trackId: number,
    clipId: number,
    timing: number,
    velocity: number,
    range?: NoteRange,
    seed?: number,
  ): Project =>
    commit(() => {
      const c = clip(trackId, clipId);
      if (!c) return;
      const rnd = mulberry32(seed ?? 0x5eed1234);
      const t = Math.min(0.5, Math.max(0, timing));
      const v = Math.min(127, Math.max(0, velocity));
      // 每个音符都取两个随机数（与引擎一致：范围不改变其他音符的抖动）。
      const draws = c.notes.map(() => ({ dt: (rnd() * 2 - 1) * t, dv: (rnd() * 2 - 1) * v }));
      c.notes = c.notes.map((n, i) => {
        if (!inRange(n, range) || n.start < 0) return n;
        return clampNote(
          {
            ...n,
            start: n.start + draws[i].dt,
            velocity: Math.min(127, Math.max(0, Math.round(n.velocity + draws[i].dv))),
          },
          c.length,
        );
      });
    }),

  fitNotesToRange: (
    trackId: number,
    clipId: number,
    minPitch: number,
    maxPitch: number,
    mode: FitMode,
    range?: NoteRange,
  ): Project =>
    commit(() => {
      const c = clip(trackId, clipId);
      if (!c || mode === "report") return;
      const lo = Math.min(minPitch, maxPitch);
      const hi = Math.max(minPitch, maxPitch);
      transform(c, range, (n) => {
        if (n.pitch >= lo && n.pitch <= hi) return n;
        let pitch = n.pitch;
        while (pitch > hi && pitch - 12 >= 0) pitch -= 12;
        while (pitch < lo && pitch + 12 <= 127) pitch += 12;
        return { ...n, pitch: Math.min(hi, Math.max(lo, pitch)) };
      });
    }),

  duplicateClip: (
    trackId: number,
    clipId: number,
    toTrack?: number,
    start?: number,
  ): Project =>
    commit(() => {
      const src = clip(trackId, clipId);
      const dest = track(toTrack ?? trackId);
      if (!src || !dest) return;
      let next = 1;
      dest.clips.push({
        id: nextClipId++,
        start: Math.max(0, start ?? src.start + src.length),
        length: src.length,
        notes: src.notes.map((n) => ({ ...n, id: next++ })),
        ...(src.audio ? { audio: { ...src.audio } } : {}),
      });
    }),

  mergeClips: (trackId: number, clipIds: number[]): Project =>
    commit(() => {
      const t = track(trackId);
      if (!t || clipIds.length < 2) return;
      const picked = clipIds
        .map((id) => t.clips.find((c) => c.id === id))
        .filter((c): c is NonNullable<typeof c> => !!c && !c.audio)
        .sort((a, b) => a.start - b.start);
      if (picked.length < 2) return;
      const start = picked[0].start;
      const end = Math.max(...picked.map((c) => c.start + c.length));
      const target = picked[0];
      let next = 1;
      target.notes = picked.flatMap((c) =>
        c.notes.map((n) => ({ ...n, start: n.start + (c.start - start), id: next++ })),
      );
      target.start = start;
      target.length = end - start;
      const drop = new Set(picked.slice(1).map((c) => c.id));
      t.clips = t.clips.filter((c) => !drop.has(c.id));
    }),

  getTrackNoteStats: (trackId: number, clipId?: number): NoteStats => {
    const t = track(trackId);
    if (!t) throw new Error(`没有轨道 ${trackId}`);
    const clips = clipId == null ? t.clips : t.clips.filter((c) => c.id === clipId);
    const spans = clips.flatMap((c) =>
      c.notes.map((n) => ({ pitch: n.pitch, velocity: n.velocity, start: c.start + n.start, end: c.start + n.start + n.duration, outside: n.start < 0 || n.start >= c.length })),
    );
    const barBeats = barLength();
    const counts = new Map<number, number>();
    for (const s of spans) {
      if (s.start < 0) continue;
      const bar = Math.floor(s.start / barBeats);
      counts.set(bar, (counts.get(bar) ?? 0) + 1);
    }
    const bars = [...counts.entries()]
      .sort((a, b) => a[0] - b[0])
      .map(([bar, count]) => ({ bar, start: bar * barBeats, count }));
    const pitches = spans.map((s) => s.pitch);
    const mean = (xs: number[]): number | undefined =>
      xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : undefined;
    const byPitch = new Map<number, { start: number; end: number }[]>();
    for (const s of spans) {
      const list = byPitch.get(s.pitch) ?? [];
      list.push({ start: s.start, end: s.end });
      byPitch.set(s.pitch, list);
    }
    let overlapping = 0;
    for (const list of byPitch.values()) {
      list.sort((a, b) => a.start - b.start);
      let end = list[0]?.end ?? 0;
      for (const span of list.slice(1)) {
        if (span.start < end - 1e-9) overlapping += 1;
        end = Math.max(end, span.end);
      }
    }
    const lo = pitches.length ? Math.min(...pitches) : undefined;
    const hi = pitches.length ? Math.max(...pitches) : undefined;
    let semitoneClashes = 0;
    let semitoneClashBeats = 0;
    for (let i = 0; i < spans.length; i += 1) {
      for (let j = i + 1; j < spans.length; j += 1) {
        const overlap = Math.min(spans[i].end, spans[j].end) - Math.max(spans[i].start, spans[j].start);
        if (overlap > 0 && Math.abs(spans[i].pitch - spans[j].pitch) % 12 === 1) {
          semitoneClashes += 1;
          semitoneClashBeats += overlap;
        }
      }
    }
    return {
      track: trackId,
      clip: clipId,
      note_count: spans.length,
      lowest_pitch: lo,
      highest_pitch: hi,
      mean_pitch: mean(pitches),
      range_semitones: lo != null && hi != null ? hi - lo : undefined,
      mean_velocity: mean(spans.map((s) => s.velocity)),
      span_beats: spans.length ? Math.max(...spans.map((s) => s.end)) - Math.min(...spans.map((s) => s.start)) : 0,
      mean_notes_per_bar: bars.length ? spans.length / bars.length : 0,
      bars,
      overlapping_notes: overlapping,
      semitone_clashes: semitoneClashes,
      semitone_clash_beats: Math.round(semitoneClashBeats * 1000) / 1000,
      notes_outside_clip: spans.filter((s) => s.outside).length,
    };
  },

  analyzeTrackOverlap: (trackIds: number[], clipId?: number): OverlapReport => {
    const picked = trackIds.map((id) => {
      const t = track(id);
      if (!t) throw new Error(`没有轨道 ${id}`);
      return t;
    });
    const spansOf = (id: number) => {
      const t = track(id)!;
      const clips = clipId == null ? t.clips : t.clips.filter((c) => c.id === clipId);
      return clips.flatMap((c) =>
        c.notes
          .filter((n) => n.start >= 0)
          .map((n) => ({ pitch: n.pitch, start: c.start + n.start, end: c.start + n.start + n.duration })),
      );
    };
    const pairs: TrackOverlap[] = [];
    for (let i = 0; i < picked.length; i += 1) {
      for (let j = i + 1; j < picked.length; j += 1) {
        const a = spansOf(picked[i].id);
        const b = spansOf(picked[j].id);
        let overlap = 0;
        let unison = 0;
        let semitoneClashes = 0;
        let semitoneClashBeats = 0;
        for (const x of a) {
          for (const y of b) {
            const span = Math.min(x.end, y.end) - Math.max(x.start, y.start);
            if (span > 0) {
              overlap += span;
              if (x.pitch === y.pitch) unison += 1;
              if (Math.abs(x.pitch - y.pitch) % 12 === 1) {
                semitoneClashes += 1;
                semitoneClashBeats += span;
              }
            }
          }
        }
        const meanOf = (v: typeof a) => (v.length ? v.reduce((s, n) => s + n.pitch, 0) / v.length : undefined);
        const [ma, mb] = [meanOf(a), meanOf(b)];
        const gap = ma != null && mb != null ? Math.round(ma - mb) : undefined;
        const conflict = (overlap >= 4 && gap != null && Math.abs(gap) < 12) || semitoneClashBeats >= 0.25;
        const clashText =
          semitoneClashes > 0
            ? `检测到 ${semitoneClashes} 处小二度 / 小九度重叠（共 ${semitoneClashBeats.toFixed(2)} 拍）。`
            : "";
        pairs.push({
          a: picked[i].id,
          b: picked[j].id,
          overlap_beats: Math.round(overlap * 1000) / 1000,
          unison_notes: unison,
          semitone_clashes: semitoneClashes,
          semitone_clash_beats: Math.round(semitoneClashBeats * 1000) / 1000,
          shared_pitch_classes: [...new Set(a.map((n) => n.pitch % 12))].filter((pc) =>
            b.some((n) => n.pitch % 12 === pc),
          ),
          pitch_gap_semitones: gap,
          conflict,
          suggestion: conflict
            ? `轨道 ${picked[i].id} 与 ${picked[j].id} 在相近音区同时发声，建议拉开一个八度或收窄音域。${clashText}`
            : clashText,
        });
      }
    }
    return { tracks: trackIds, pairs, summary: pairs.filter((p) => p.conflict).map((p) => p.suggestion) };
  },

  analyzeHarmony: (bars?: HarmonyBar[], tracks?: number[]): HarmonyReport => {
    const chart = bars ?? state.harmony ?? [];
    if (chart.length === 0) throw new Error("没有和弦信息：传 bars，或先写入工程和弦表");
    // 浏览器预览不做完整和声分析（和弦解析在后端）：给出小节表，提示未检查。
    const report: HarmonyReport = {
      bars: chart.map((b, i) => ({
        bar: i,
        start: b.start,
        length: b.length,
        symbol: b.symbol,
        chord_tones: [],
        tension: "info",
        issues: 0,
      })),
      issues: [],
      conflict_level: "info",
      notes: [`浏览器预览不做和声分析（${chart.length} 个小节未检查${tracks ? `，轨道 ${tracks.join("/")}` : ""}）`],
    };
    return report;
  },

  setProjectHarmony: (bars: HarmonyBar[]): Project =>
    commit(() => {
      state.harmony = [...bars].sort((a, b) => a.start - b.start);
    }),

  setProjectSections: (sections: Section[]): Project =>
    commit(() => {
      state.sections = [...sections].sort((a, b) => a.start - b.start);
    }),

  setTrackMeta: (
    trackId: number,
    role?: string,
    pitchRange?: PitchRange,
    pitchBendRange?: number,
  ): Project =>
    commit(() => {
      const t = track(trackId);
      if (!t) return;
      const meta: TrackMeta = {};
      if (role) meta.role = role;
      if (pitchRange) meta.pitch_range = pitchRange;
      if (pitchBendRange != null && Number.isFinite(pitchBendRange))
        meta.pitch_bend_range = Math.max(0.01, Math.min(24, pitchBendRange));
      if (Object.keys(meta).length === 0) delete t.meta;
      else t.meta = meta;
    }),

  setGain: (id: number, gainDb: number): Project =>
    commit(() => {
      const t = track(id);
      if (t) t.mixer.gain_db = gainDb;
    }),

  setPan: (id: number, pan: number): Project =>
    commit(() => {
      const t = track(id);
      if (t) t.mixer.pan = Math.min(1, Math.max(-1, pan));
    }),

  setMute: (id: number, mute: boolean): Project =>
    commit(() => {
      const t = track(id);
      if (t) t.mixer.mute = mute;
    }),

  setSolo: (id: number, solo: boolean): Project =>
    commit(() => {
      const t = track(id);
      if (t) t.mixer.solo = solo;
    }),

  setMasterGain: (gainDb: number): Project =>
    commit(() => {
      state.master.gain_db = gainDb;
    }),

  setSends: (trackId: number, sends: Send[]): Project =>
    commit(() => {
      const t = track(trackId);
      if (t) {
        // 自发送无意义：与引擎一致地丢弃。
        t.sends = sends.filter((s) => s.target !== trackId).map((s) => ({ ...s }));
      }
    }),

  setAutomation: (trackId: number, param: string, points: AutomationPoint[]): Project =>
    commit(() => {
      const t = track(trackId);
      if (!t) return;
      const lanes = (t.automation ?? []).filter((l) => l.param !== param);
      if (points.length > 0) {
        const sorted = [...points].sort((a, b) => a.beat - b.beat);
        lanes.push({ param, points: sorted });
      }
      t.automation = lanes;
    }),

  historyDepths: (): HistoryDepths => ({ undo: undoStack.length, redo: redoStack.length, dirty }),

  undo: (): HistoryState => {
    const previous = undoStack.pop();
    if (previous) {
      redoStack.push(clone(state));
      state = previous;
    }
    return { project: clone(state), undo_depth: undoStack.length, redo_depth: redoStack.length, dirty };
  },

  redo: (): HistoryState => {
    const next = redoStack.pop();
    if (next) {
      undoStack.push(clone(state));
      state = next;
    }
    return { project: clone(state), undo_depth: undoStack.length, redo_depth: redoStack.length, dirty };
  },

  saveProject: (path: string): string => {
    dirty = false;
    sessionPath = path;
    // 保存成功的这一版也进历史（与后端 `save_project` 一致）。
    recordHistory();
    return path;
  },

  currentProjectPath: (): string | null => sessionPath,

  projectHistory: (): ProjectHistoryEntry[] => readHistory().map((record) => record.entry),

  restoreProjectHistory: (id: string): Project => {
    // 当前工程先入历史；被恢复的那份从列表取出（它马上就是当前工程了）。
    recordHistory();
    const records = readHistory();
    const index = records.findIndex((record) => record.entry.id === id);
    if (index < 0) throw new Error("历史工程不存在");
    const [record] = records.splice(index, 1);
    writeHistory(records);
    if (!record) throw new Error("历史工程不存在");

    state = clone(record.project);
    dirty = record.entry.dirty;
    sessionPath = record.entry.path;
    nextTrackId = state.tracks.reduce((max, track) => Math.max(max, track.id), 0) + 1;
    nextClipId = state.tracks
      .flatMap((track) => track.clips)
      .reduce((max, clip) => Math.max(max, clip.id), 0) + 1;
    undoStack.length = 0;
    redoStack.length = 0;
    transport.position_frames = 0;
    transport.playing = false;
    emitTransport();
    return clone(state);
  },

  cacheSession: (path: string | null): void => {
    sessionPath = path;
  },

  deleteProjectHistory: (id: string): void => {
    const records = readHistory();
    const rest = records.filter((record) => record.entry.id !== id);
    if (rest.length === records.length) throw new Error("历史工程不存在");
    writeHistory(rest);
  },

  openProject: (): Project => {
    recordHistory();
    state = sample();
    nextTrackId = 4;
    nextClipId = 4;
    undoStack.length = 0;
    redoStack.length = 0;
    dirty = false;
    transport.position_frames = 0;
    transport.playing = false;
    emitTransport();
    return clone(state);
  },

  renderPreview: (): Blob => new Blob([new Uint8Array(0)], { type: "audio/wav" }),
  exportAudio: (path: string): string => path,

  // 浏览器预览没有真实渲染：给一份「静音」报告，字段与后端对齐（非有限值 → null）。
  analyzeProject: (): AnalysisReport => ({
    loudness: {
      integrated_lufs: null,
      short_term_max_lufs: null,
      loudness_range_lu: 0,
      sample_peak_dbfs: null,
      true_peak_dbtp: null,
    },
    rms_dbfs: null,
    spectrum: {
      bin_hz: 48_000 / 2_048,
      magnitudes_db: Array.from({ length: 2_048 / 2 + 1 }, () => null),
    },
  }),

  mcpStatus: (): McpStatus => mockMcpStatus(),
  setMcpConfig: (enabled: boolean, port: number): McpStatus => {
    mcpConfig = { enabled, port };
    return mockMcpStatus();
  },

  // 写歌智能体：预览里配置存内存、会话为空、发送只回一个假 id（不模拟流式回答）。
  agentConfig: (): AgentConfig => agentConfigState,
  agentSetConfig: (baseUrl: string, apiKey: string, model: string): AgentConfig => {
    agentConfigState = { base_url: baseUrl, api_key: apiKey, model };
    return agentConfigState;
  },
  agentModels: (): string[] => ["gpt-4o-mini", "gpt-4o", "deepseek-chat"],
  agentTest: (): AgentProbe => ({ ok: true, detail: "预览模式：假装连接成功", ms: 12 }),
  agentSessions: (): AgentSessionSummary[] => [],
  agentSession: (id: string): AgentSession => {
    const found = mockSessions.get(id);
    if (!found) throw new Error(`会话不存在：${id}`);
    return found;
  },
  agentDeleteSession: (id: string): void => {
    mockSessions.delete(id);
  },
  agentSetHead: (id: string, nodeId: string): AgentSession => {
    const found = mockSessions.get(id);
    if (!found) throw new Error(`会话不存在：${id}`);
    found.head = nodeId;
    return found;
  },
  agentSend: (_sessionId: string | null, _parentId: string | null, _text: string): string => "mock-session",
  agentCancel: (): void => {},
  onAgentEvent: (_handler: (event: AgentEvent) => void): Promise<() => void> => Promise.resolve(() => {}),

  cliStatus: (): CliStatus => mockCliStatus(),
  installCli: (): CliStatus => {
    cliState = "installed";
    return mockCliStatus();
  },
  uninstallCli: (): CliStatus => {
    cliState = "missing";
    return mockCliStatus();
  },

  // 浏览器预览没有真实解码：造一段音频片段与一条假的波形包络。
  importAudio: (path: string, trackId: number, start: number): Project =>
    commit(() => {
      const t = track(trackId);
      if (t) {
        t.clips.push({
          id: nextClipId++,
          start: Math.max(0, start),
          length: 2,
          notes: [],
          // 假素材：给一个窗口长度，让预览里的切分也能走通。
          audio: { path, offset_frames: 0, frames: 96000 },
        });
      }
    }),

  addAudioClip: (
    trackId: number,
    start: number,
    length: number,
    path: string,
    offsetFrames: number,
    frames?: number,
  ): Project =>
    commit(() => {
      const t = track(trackId);
      if (t) {
        t.clips.push({
          id: nextClipId++,
          start: Math.max(0, start),
          length,
          notes: [],
          audio: { path, offset_frames: offsetFrames, frames },
        });
      }
    }),

  // ---- 录音（浏览器预览没有真实采集：设备是假的，录完如实返回 null）----

  listInputDevices: (): InputDeviceInfo[] => [
    { name: "内置麦克风", sample_rate: 48000, channels: 1, transport: "builtin", is_default: true },
  ],
  setInputDevice: (): void => undefined,
  inputLevel: (points: number): number[] =>
    Array.from({ length: Math.max(1, points) }, (_, i) => Math.sin(i / 6) * 0.3),
  recordStart: (): void => undefined,
  recordStop: (): Project | null => null,

  // ---- 本地 AI（预览：模拟进度，不真正处理）----

  aiStatus: (): AiStatus => ({ ...aiState }),
  aiModels: (): AiModels => ({
    basic_pitch: false,
    localvqe: false,
    separate: false,
    beat_this: false,
  }),
  aiStart: (kind: string): void => {
    if (aiTimer !== null) return;
    aiState = { running: true, kind, progress: 0, message: null };
    emitAi();
    aiTimer = window.setInterval(() => {
      aiState.progress = Math.min(1, aiState.progress + 0.08);
      if (aiState.progress >= 1) {
        window.clearInterval(aiTimer!);
        aiTimer = null;
        aiState = { running: false, kind: null, progress: 0, message: "浏览器预览：未做真实处理" };
      }
      emitAi();
    }, 120);
  },
  aiCancel: (): void => {
    if (aiTimer !== null) {
      window.clearInterval(aiTimer);
      aiTimer = null;
    }
    aiState = { running: false, kind: null, progress: 0, message: "已停止" };
    emitAi();
  },
  onAiEvent: (): (() => void) => {
    window.setTimeout(emitAi, 0);
    return () => {
      if (aiTimer !== null) {
        window.clearInterval(aiTimer);
        aiTimer = null;
      }
    };
  },

  /** 在第 `beat` 拍切分片段（预览版按拍数近似切音频窗口）。 */
  splitClip: (trackId: number, clipId: number, beat: number): Project =>
    commit(() => {
      const t = track(trackId);
      const c = t?.clips.find((x) => x.id === clipId);
      if (!t || !c) return;
      const end = c.start + c.length;
      const eps = 1e-6;
      if (!(Number.isFinite(beat) && beat > c.start + eps && beat < end - eps)) return;
      const rel = beat - c.start;

      const leftNotes: Note[] = [];
      const rightNotes: Note[] = [];
      for (const n of c.notes) {
        const nEnd = n.start + n.duration;
        if (nEnd <= rel + eps) leftNotes.push(n);
        else if (n.start >= rel - eps) rightNotes.push({ ...n, start: n.start - rel });
        else {
          leftNotes.push({ ...n, duration: rel - n.start });
          rightNotes.push({ ...n, start: 0, duration: nEnd - rel });
        }
      }

      let rightAudio = c.audio;
      if (c.audio) {
        if (c.audio.frames === undefined) return; // 无窗口长度 → 不切（与引擎一致）
        const window = c.audio.frames;
        if (window < 2) return;
        const leftFrames = Math.min(window - 1, Math.max(1, Math.round((window * rel) / c.length)));
        c.audio = { ...c.audio, frames: leftFrames };
        rightAudio = { ...c.audio, offset_frames: c.audio.offset_frames + leftFrames, frames: window - leftFrames };
      }

      c.length = rel;
      c.notes = leftNotes;
      const right = {
        id: nextClipId++,
        start: beat,
        length: end - beat,
        notes: rightNotes,
        audio: rightAudio,
      };
      const at = t.clips.findIndex((x) => x.start > right.start);
      if (at < 0) t.clips.push(right);
      else t.clips.splice(at, 0, right);
    }),

  audioWaveform: (
    _path: string,
    buckets: number,
    offsetFrames?: number,
    frames?: number,
  ) => {
    const peaks: number[] = [];
    for (let i = 0; i < buckets; i += 1) {
      const v = Math.abs(Math.sin((i / buckets) * Math.PI * 8)) * 0.9 + 0.05;
      peaks.push(-v, v);
    }
    return {
      peaks,
      sample_rate: 48000,
      frames: frames ?? 48000 * 4,
      offset_frames: offsetFrames ?? 0,
    };
  },

  // 预览环境不夹取（素材长度未知）：给一个宽松范围。
  clipBounds: (_track: number, _clip: number) => ({ min_start: 0, max_end: 1e6 }),

  // ---- 传输 ----

  transportStatus: (): TransportSnapshot => ({ ...transport }),

  transportPlay: (): void => {
    // 与引擎 `Player::apply_command(Play)` 同一条策略：已经停在边界上时再播放 →
    // 开循环且有范围就从范围起点起播，否则从头开始。
    const end = playbackBoundary();
    if (end > 0 && transport.position_frames >= end) {
      transport.position_frames = transport.loop ? loopRange()?.start ?? 0 : 0;
    }
    transport.playing = true;
    ensureTransportTimer();
    emitTransport();
  },

  transportPause: (): void => {
    transport.playing = false;
    emitTransport();
  },

  transportStop: (): void => {
    transport.playing = false;
    transport.position_frames = 0;
    emitTransport();
  },

  transportSeek: (frame: number): void => {
    transport.position_frames = Math.max(0, Math.round(frame));
    emitTransport();
  },

  transportSetLoop: (on: boolean): void => {
    transport.loop = on;
    emitTransport();
  },

  transportSetLoopRange: (start: number, end: number): void => {
    // 与引擎同一条归一化：`end <= start` = 取消范围（写回 0）。
    const valid = end > start;
    transport.loop_start_frames = valid ? Math.max(0, Math.round(start)) : 0;
    transport.loop_end_frames = valid ? Math.round(end) : 0;
    emitTransport();
  },

  transportSetMetronome: (_on: boolean): void => {},

  // 浏览器预览里没有真实设备可探测：重试是空操作，状态照旧。
  retryAudio: (): Promise<void> => Promise.resolve(),

  onEngineEvent: (): (() => void) => {
    const t = window.setInterval(() => {
      // 模拟引擎事件，便于预览开发者抽屉与 AI 胶囊。
      window.dispatchEvent(new CustomEvent("ai-daw-mock-event"));
    }, 4000);
    return () => window.clearInterval(t);
  },

  onTransportEvent: (): (() => void) => {
    // 订阅时立刻推一次当前状态，让界面与真实后端行为一致。
    window.setTimeout(emitTransport, 0);
    return () => {
      if (transportTimer !== null) {
        window.clearInterval(transportTimer);
        transportTimer = null;
      }
    };
  },
};
