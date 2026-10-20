/**
 * Tauri 后端命令的类型化封装（与 `src-tauri/src/commands.rs` 一一对应）。
 * 命令名、参数名（camelCase，Tauri 会映射到 Rust 的 snake_case）必须与后端一致。
 *
 * 在浏览器里（非 Tauri）开发时回退到 `mock.ts`，方便无后端预览界面。
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { mockApi, mockEnabled } from "./mock";
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
  CliStatus,
  ClipPlacement,
  ClipRef,
  FitMode,
  ForeignPlugin,
  HarmonyBar,
  HarmonyPolicy,
  HarmonyReport,
  HistoryDepths,
  HistoryState,
  ExportProgress,
  ImportProgress,
  InputDeviceInfo,
  ManagerInstallRequest,
  ManagerInstalled,
  ManagerRegistry,
  ManagerProgress,
  MasterArgs,
  McpStatus,
  MixerState,
  ModelProgress,
  ModelSourcePreference,
  ModelStatus,
  Note,
  NoteEdit,
  NotePatch,
  NoteRange,
  NoteStats,
  OverlapReport,
  PitchRange,
  Preset,
  Project,
  ProjectHistoryEntry,
  ProxyMode,
  ProxyProbe,
  ProxyStatus,
  Section,
  Send,
  StoreNetwork,
  TransportSnapshot,
  VelocityShape,
} from "./types";

/** 引擎事件广播通道名，与后端 `state::EVENT_ENGINE` 保持一致。 */
export const EVENT_ENGINE = "engine://event";
/** 传输状态广播通道名，与后端 `state::EVENT_TRANSPORT` 保持一致。 */
export const EVENT_TRANSPORT = "engine://transport";
/** 本地 AI 状态广播通道名，与后端 `ai::EVENT_AI` 保持一致。 */
export const EVENT_AI = "engine://ai";

/** 插件管理器进度广播通道名，与后端 `manager::EVENT_MANAGER` 保持一致。 */
export const EVENT_MANAGER = "engine://manager";
/** 模型下载进度广播通道名，与后端 `models::EVENT_MODEL` 保持一致。 */
export const EVENT_MODEL = "engine://model";
/** 音频导入解码进度广播通道名，与后端 `state::EVENT_IMPORT` 保持一致。 */
export const EVENT_IMPORT = "engine://import";
/** 音频导出（渲染 + 编码）进度广播通道名，与后端 `state::EVENT_EXPORT` 保持一致。 */
export const EVENT_EXPORT = "engine://export";
/** 写歌智能体事件广播通道名，与后端 `agent::EVENT_AGENT` 保持一致。 */
export const EVENT_AGENT = "engine://agent";

/** 读取当前工程。 */
export function getProject(): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.getProject());
  return invoke<Project>("get_project");
}

// ---------------------------------------------------------------------------
// 工程与轨道
// ---------------------------------------------------------------------------

/** 新增一条轨道。 */
export function addTrack(name: string): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.addTrack(name));
  return invoke<Project>("add_track", { name });
}

/** 删除一条轨道（含其片段与音符）。 */
export function removeTrack(track: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.removeTrack(track));
  return invoke<Project>("remove_track", { track });
}

/** 重命名一条轨道。 */
export function renameTrack(track: number, name: string): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.renameTrack(track, name));
  return invoke<Project>("rename_track", { track, name });
}

/** 为轨道设置乐器插件。 */
export function setTrackInstrument(track: number, pluginId: string): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setTrackInstrument(track, pluginId));
  return invoke<Project>("set_track_instrument", { track, pluginId });
}

/** 设置轨道乐器的某个参数。 */
export function setParam(track: number, param: string, value: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setParam(track, param, value));
  return invoke<Project>("set_param", { track, param, value });
}

/** 设置速度（BPM）。 */
export function setTempo(bpm: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setTempo(bpm));
  return invoke<Project>("set_tempo", { bpm });
}

/** 设置拍号；越界值由引擎归一化。 */
export function setTimeSignature(num: number, den: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setTimeSignature(num, den));
  return invoke<Project>("set_time_signature", { num, den });
}

/** 在第 beat 拍插入/更新关键点。 */
export function setKeyframe(beat: number, tempo: number, num: number, den: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setKeyframe(beat, tempo, num, den));
  return invoke<Project>("set_keyframe", { beat, tempo, num, den });
}

/** 删除第 beat 拍的关键点。 */
export function removeKeyframe(beat: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.removeKeyframe(beat));
  return invoke<Project>("remove_keyframe", { beat });
}

/** 新建工程（铺默认分轨模板）。`trackNames` 按模板顺序给出，用于按语言命名分轨。 */
export function newProject(trackNames?: string[], tempo?: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.newProject(trackNames, tempo));
  return invoke<Project>("new_project", { trackNames, tempo });
}

/** 重命名当前工程。 */
export function renameProject(name: string): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.renameProject(name));
  return invoke<Project>("rename_project", { name });
}

/** 当前工程对应的文件路径（尚未保存过时为 null）。 */
export function currentProjectPath(): Promise<string | null> {
  if (mockEnabled()) return Promise.resolve(mockApi.currentProjectPath());
  return invoke<string | null>("current_project_path");
}

/** 把当前工程写入会话缓存（退出前 / 编辑过程中定期调用，供下次启动还原）。 */
export function cacheSession(path: string | null): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.cacheSession(path));
  return invoke<void>("cache_session", { path });
}

/** 历史工程列表（展示用元信息）。 */
export function projectHistory(): Promise<ProjectHistoryEntry[]> {
  if (mockEnabled()) return Promise.resolve(mockApi.projectHistory());
  return invoke<ProjectHistoryEntry[]>("project_history");
}

/** 回到一份历史工程（当前工程会先被收进历史）。 */
export function restoreProjectHistory(id: string): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.restoreProjectHistory(id));
  return invoke<Project>("restore_project_history", { id });
}

/** 把启动时铺的默认分轨名改成当前语言的名字（仅当工程仍是默认模板）。 */
export function localizeStarterTracks(names: string[]): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.localizeStarterTracks(names));
  return invoke<Project>("localize_starter_tracks", { names });
}

// ---------------------------------------------------------------------------
// 片段与音符
// ---------------------------------------------------------------------------

/** 新增一个 MIDI 片段。 */
export function addClip(track: number, start: number, length: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.addClip(track, start, length));
  return invoke<Project>("add_clip", { track, start, length });
}

/** 删除一个片段。 */
export function removeClip(track: number, clip: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.removeClip(track, clip));
  return invoke<Project>("remove_clip", { track, clip });
}

/** 把片段移动到新的起始拍。 */
export function moveClip(track: number, clip: number, start: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.moveClip(track, clip, start));
  return invoke<Project>("move_clip", { track, clip, start });
}

/** 批量移动片段（多选拖动）。 */
export function moveClips(items: ClipPlacement[]): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.moveClips(items));
  return invoke<Project>("move_clips", { items });
}

/** 改变片段长度（拍）。 */
export function resizeClip(track: number, clip: number, length: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.resizeClip(track, clip, length));
  return invoke<Project>("resize_clip", { track, clip, length });
}

/** 设置音频片段的播放速度 / 升降调（缺省 1 / 0）。 */
export function setClipRate(
  track: number,
  clip: number,
  rate: number,
  pitchSemitones: number,
): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setClipRate(track, clip, rate, pitchSemitones));
  return invoke<Project>("set_clip_rate", { track, clip, rate, pitchSemitones });
}

/** 裁剪 / 移动片段：新的起点与拍长（音频按采样率换算成读写窗口）。 */
export function trimClip(
  track: number,
  clip: number,
  start: number,
  length: number,
): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.trimClip(track, clip, start, length));
  return invoke<Project>("trim_clip", { track, clip, start, length });
}

/** 在片段中新增一个音符。 */
export function addNote(track: number, clip: number, note: Note): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.addNote(track, clip, note));
  return invoke<Project>("add_note", { track, clip, note });
}

/** 改写片段内的某个音符（按片段内索引定位）。 */
export function setNote(track: number, clip: number, index: number, note: Note): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setNote(track, clip, index, note));
  return invoke<Project>("set_note", { track, clip, index, note });
}

/** 批量改写片段内的多个音符（多选拖动 / 属性面板一次改多个）。 */
export function setNotes(track: number, clip: number, entries: NoteEdit[]): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setNotes(track, clip, entries));
  return invoke<Project>("set_notes", { track, clip, entries });
}

/** 删除片段内的某个音符（按片段内索引定位）。 */
export function removeNote(track: number, clip: number, index: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.removeNote(track, clip, index));
  return invoke<Project>("remove_note", { track, clip, index });
}

/** 批量删除片段内的多个音符（框选后删除）。 */
export function removeNotes(track: number, clip: number, indices: number[]): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.removeNotes(track, clip, indices));
  return invoke<Project>("remove_notes", { track, clip, indices });
}

/** 批量删除片段（可跨轨道）。 */
export function removeClips(items: ClipRef[]): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.removeClips(items));
  return invoke<Project>("remove_clips", { items });
}

// ---------------------------------------------------------------------------
// 编曲：批量音符、律动与音域（一次调用 = 一个撤销步骤）
// ---------------------------------------------------------------------------

/** 一次写入一组音符。 */
export function addNotes(track: number, clip: number, notes: Note[]): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.addNotes(track, clip, notes));
  return invoke<Project>("add_notes", { track, clip, notes });
}

/** 原子替换片段内的全部音符（空数组 = 清空）。 */
export function replaceNotes(track: number, clip: number, notes: Note[]): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.replaceNotes(track, clip, notes));
  return invoke<Project>("replace_notes", { track, clip, notes });
}

/** 按稳定 id 批量改写音符（只改给出的字段）。 */
export function updateNotes(track: number, clip: number, updates: NotePatch[]): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.updateNotes(track, clip, updates));
  return invoke<Project>("update_notes", { track, clip, updates });
}

/** 按稳定 id 批量删除音符。 */
export function removeNotesById(track: number, clip: number, ids: number[]): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.removeNotesById(track, clip, ids));
  return invoke<Project>("remove_notes_by_id", { track, clip, ids });
}

/** 批量移调（半音）。 */
export function transposeNotes(
  track: number,
  clip: number,
  semitones: number,
  range?: NoteRange,
): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.transposeNotes(track, clip, semitones, range));
  return invoke<Project>("transpose_notes", { track, clip, semitones, range: range ?? null });
}

/** 批量设置力度（固定 / 渐变 / 缩放）。 */
export function setNoteVelocity(
  track: number,
  clip: number,
  shape: VelocityShape,
  range?: NoteRange,
  beatFilter?: { grid: number; beat: number },
): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setNoteVelocity(track, clip, shape, range));
  return invoke<Project>("set_note_velocity", {
    track,
    clip,
    shape,
    range: range ?? null,
    beatFilter: beatFilter ?? null,
  });
}

/** 量化：把起点吸到 grid 的整数倍。 */
export function quantizeNotes(
  track: number,
  clip: number,
  grid: number,
  strength = 1,
  range?: NoteRange,
  quantizeDuration?: boolean,
): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.quantizeNotes(track, clip, grid, strength, range));
  return invoke<Project>("quantize_notes", {
    track,
    clip,
    grid,
    strength,
    range: range ?? null,
    quantizeDuration: quantizeDuration ?? null,
  });
}

/** 扫弦：把**起点相同**的音符按音高依次错开（`stepMs = 0` = 回到同时发声）。 */
export function strumNotes(
  track: number,
  clip: number,
  stepMs: number,
  direction: "down" | "up" = "down",
  range?: NoteRange,
): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.strumNotes(track, clip));
  return invoke<Project>("strum_notes", { track, clip, stepMs, direction, range: range ?? null });
}

/** Swing：把指定子拍的第二个往后推。 */
export function applySwing(
  track: number,
  clip: number,
  subdivision = 0.5,
  ratio = 0.66,
  strength = 1,
  range?: NoteRange,
): Promise<Project> {
  if (mockEnabled())
    return Promise.resolve(mockApi.applySwing(track, clip, subdivision, ratio, strength, range));
  return invoke<Project>("apply_swing", {
    track,
    clip,
    subdivision,
    ratio,
    strength,
    range: range ?? null,
  });
}

/** 人性化：位置 / 力度的确定性抖动。 */
export function humanizeNotes(
  track: number,
  clip: number,
  timing = 0.02,
  velocity = 8,
  range?: NoteRange,
  seed?: number,
  minDuration?: number,
  groupByStart?: boolean,
): Promise<Project> {
  if (mockEnabled())
    return Promise.resolve(mockApi.humanizeNotes(track, clip, timing, velocity, range, seed));
  return invoke<Project>("humanize_notes", {
    track,
    clip,
    timing,
    velocity,
    range: range ?? null,
    seed: seed ?? null,
    minDuration: minDuration ?? null,
    groupByStart: groupByStart ?? null,
  });
}

/** 把音符收进目标音域（`fold` 八度折返 / `report` 只报告）。 */
export function fitNotesToRange(
  track: number,
  clip: number,
  minPitch: number,
  maxPitch: number,
  mode: FitMode = "fold",
  range?: NoteRange,
): Promise<Project> {
  if (mockEnabled())
    return Promise.resolve(mockApi.fitNotesToRange(track, clip, minPitch, maxPitch, mode, range));
  return invoke<Project>("fit_notes_to_range", {
    track,
    clip,
    minPitch,
    maxPitch,
    mode,
    range: range ?? null,
  });
}

/** 复制片段（缺省贴在原片段之后）。 */
export function duplicateClip(
  track: number,
  clip: number,
  toTrack?: number,
  start?: number,
): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.duplicateClip(track, clip, toTrack, start));
  return invoke<Project>("duplicate_clip", {
    track,
    clip,
    toTrack: toTrack ?? null,
    start: start ?? null,
  });
}

/** 合并同一轨道上的多个 MIDI 片段。 */
export function mergeClips(track: number, clips: number[]): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.mergeClips(track, clips));
  return invoke<Project>("merge_clips", { track, clips });
}

/** 只读：轨道的音域、密度、重叠统计。 */
export function getTrackNoteStats(track: number, clip?: number): Promise<NoteStats> {
  if (mockEnabled()) return Promise.resolve(mockApi.getTrackNoteStats(track, clip));
  return invoke<NoteStats>("get_track_note_stats", { track, clip: clip ?? null });
}

/** 只读：多条轨道之间的音区重叠与分工冲突。 */
export function analyzeTrackOverlap(tracks: number[], clip?: number): Promise<OverlapReport> {
  if (mockEnabled()) return Promise.resolve(mockApi.analyzeTrackOverlap(tracks, clip));
  return invoke<OverlapReport>("analyze_track_overlap", { tracks, clip: clip ?? null });
}

/** 只读：按小节和弦表检查和声（`bars` 缺省用工程里存的那份）。 */
export function analyzeHarmony(
  bars?: HarmonyBar[],
  tracks?: number[],
  policy?: Partial<HarmonyPolicy>,
  key?: string,
  scale?: string,
): Promise<HarmonyReport> {
  if (mockEnabled()) return Promise.resolve(mockApi.analyzeHarmony(bars, tracks));
  return invoke<HarmonyReport>("analyze_harmony", {
    bars: bars ?? null,
    tracks: tracks ?? null,
    allowExtensions: policy?.allow_extensions ?? null,
    allowApproachNotes: policy?.allow_approach_notes ?? null,
    allowNonChordToneDuration: policy?.allow_non_chord_tone_duration ?? null,
    timeTolerance: null,
    key: key ?? null,
    scale: scale ?? null,
  });
}

/** 写入工程的小节和弦表（整体替换；空数组 = 清除）。 */
export function setProjectHarmony(bars: HarmonyBar[]): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setProjectHarmony(bars));
  return invoke<Project>("set_project_harmony", { bars });
}

/** 写入工程的段落表（整体替换）。 */
export function setProjectSections(sections: Section[]): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setProjectSections(sections));
  return invoke<Project>("set_project_sections", { sections });
}

/** 设置轨道的编曲元数据（分工 / 目标音域）。 */
export function setTrackMeta(
  track: number,
  role?: string,
  pitchRange?: PitchRange,
): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setTrackMeta(track, role, pitchRange));
  return invoke<Project>("set_track_meta", {
    track,
    role: role ?? null,
    minPitch: pitchRange?.min ?? null,
    maxPitch: pitchRange?.max ?? null,
  });
}

// ---------------------------------------------------------------------------
// 混音
// ---------------------------------------------------------------------------

/** 设置轨道增益（dB）。 */
export function setGain(track: number, gainDb: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setGain(track, gainDb));
  return invoke<Project>("set_gain", { track, gainDb });
}

/** 设置轨道声像（-1.0 最左，1.0 最右）。 */
export function setPan(track: number, pan: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setPan(track, pan));
  return invoke<Project>("set_pan", { track, pan });
}

/** 设置轨道静音。 */
export function setMute(track: number, mute: boolean): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setMute(track, mute));
  return invoke<Project>("set_mute", { track, mute });
}

/** 设置轨道独奏。 */
export function setSolo(track: number, solo: boolean): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setSolo(track, solo));
  return invoke<Project>("set_solo", { track, solo });
}

/** 设置总输出增益（dB）。 */
export function setMasterGain(gainDb: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setMasterGain(gainDb));
  return invoke<Project>("set_master_gain", { gainDb });
}

/** 设置轨道的发送列表（整体替换；空数组 = 清空）。 */
export function setSends(track: number, sends: Send[]): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setSends(track, sends));
  return invoke<Project>("set_sends", { track, sends });
}

/** 设置一条自动化车道（整体替换该参数的曲线；空 `points` = 删除）。 */
export function setAutomation(
  track: number,
  param: string,
  points: AutomationPoint[],
): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setAutomation(track, param, points));
  return invoke<Project>("set_automation", { track, param, points });
}

/**
 * 实时监听用的混音预览：不写入工程、不进入撤销历史。
 * 拖动推子/声像时高频调用，松手时才发一条真正的编辑命令。
 */
export function monitorTrackMix(track: number, mix: MixerState): Promise<void> {
  if (mockEnabled()) return Promise.resolve();
  return invoke<void>("monitor_track_mix", {
    track,
    gainDb: mix.gain_db,
    pan: mix.pan,
    mute: mix.mute,
    solo: mix.solo,
  });
}

// ---------------------------------------------------------------------------
// 历史
// ---------------------------------------------------------------------------

/** 当前可撤销 / 可重做次数。 */
export function historyDepths(): Promise<HistoryDepths> {
  if (mockEnabled()) return Promise.resolve(mockApi.historyDepths());
  return invoke<HistoryDepths>("history_depths");
}

/** 撤销最后一次命令。 */
export function undo(): Promise<HistoryState> {
  if (mockEnabled()) return Promise.resolve(mockApi.undo());
  return invoke<HistoryState>("undo");
}

/** 重做上一次撤销。 */
export function redo(): Promise<HistoryState> {
  if (mockEnabled()) return Promise.resolve(mockApi.redo());
  return invoke<HistoryState>("redo");
}

// ---------------------------------------------------------------------------
// 传输（实时播放）
// ---------------------------------------------------------------------------

/** 读取当前传输状态。 */
export function transportStatus(): Promise<TransportSnapshot> {
  if (mockEnabled()) return Promise.resolve(mockApi.transportStatus());
  return invoke<TransportSnapshot>("transport_status");
}

/** 开始播放。 */
export function transportPlay(): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.transportPlay());
  return invoke<void>("transport_play");
}

/** 暂停（保留当前位置）。 */
export function transportPause(): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.transportPause());
  return invoke<void>("transport_pause");
}

/** 停止并回到起点。 */
export function transportStop(): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.transportStop());
  return invoke<void>("transport_stop");
}

/** 定位到指定帧。 */
export function transportSeek(frame: number): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.transportSeek(frame));
  return invoke<void>("transport_seek", { frame });
}

/** 开关循环播放。 */
export function transportSetLoop(on: boolean): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.transportSetLoop(on));
  return invoke<void>("transport_set_loop", { on });
}

/** 设定播放范围（帧）；`end <= start` = 取消范围。 */
export function transportSetLoopRange(start: number, end: number): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.transportSetLoopRange(start, end));
  return invoke<void>("transport_set_loop_range", { start, end });
}

/** 开关节拍器（按工程速度 / 拍号打点）。 */
export function transportSetMetronome(on: boolean): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.transportSetMetronome(on));
  return invoke<void>("transport_set_metronome", { on });
}

/** 重新探测输出设备并重建实时宿主（设备变化后无需重启）。 */
export function retryAudio(): Promise<void> {
  if (mockEnabled()) return mockApi.retryAudio();
  return invoke<void>("retry_audio");
}

// ---------------------------------------------------------------------------
// 文件与渲染
// ---------------------------------------------------------------------------

/** 把工程保存为 JSON 文件，返回写入路径。 */
export function saveProject(path: string): Promise<string> {
  if (mockEnabled()) return Promise.resolve(mockApi.saveProject(path));
  return invoke<string>("save_project", { path });
}

/** 从 JSON 文件导入工程。 */
export function openProject(path: string): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.openProject());
  return invoke<Project>("open_project", { path });
}

/** 离线渲染当前工程，返回可播放的 WAV Blob（实时播放不可用时的回退路径）。 */
export async function renderPreview(sampleRate = 44100): Promise<Blob> {
  if (mockEnabled()) return mockApi.renderPreview();
  const bytes = await invoke<ArrayBuffer>("render_preview", { sampleRate });
  return new Blob([bytes], { type: "audio/wav" });
}

/** 导出格式：与后端 `ExportFormat` 一一对应。 */
export type ExportFormat = "wav" | "mp3";

/**
 * 将当前工程导出为音频文件，返回写入路径。
 *
 * `format` 决定编码器（WAV = 16-bit PCM 无损；MP3 = LAME，有损）；`bitrateKbps` 只对 MP3 有意义。
 */
export function exportAudio(
  path: string,
  format: ExportFormat,
  sampleRate = 44100,
  bitrateKbps?: number,
): Promise<string> {
  if (mockEnabled()) return Promise.resolve(mockApi.exportAudio(path));
  return invoke<string>("export_audio", { path, sampleRate, format, bitrateKbps });
}

/** 离线分析当前工程：响度 / 频谱 / 峰值（AI 的「听感」入口）。 */
export function analyzeProject(sampleRate = 44100): Promise<AnalysisReport> {
  if (mockEnabled()) return Promise.resolve(mockApi.analyzeProject());
  return invoke<AnalysisReport>("analyze_project", { sampleRate });
}

/** 读取 MCP 服务器的配置与运行状态。 */
export function mcpStatus(): Promise<McpStatus> {
  if (mockEnabled()) return Promise.resolve(mockApi.mcpStatus());
  return invoke<McpStatus>("mcp_status");
}

/** 更新 MCP 配置（写盘并即时启停），返回最新状态。 */
export function setMcpConfig(enabled: boolean, port: number): Promise<McpStatus> {
  if (mockEnabled()) return Promise.resolve(mockApi.setMcpConfig(enabled, port));
  return invoke<McpStatus>("set_mcp_config", { enabled, port });
}

// ---------------------------------------------------------------------------
// 写歌智能体
// ---------------------------------------------------------------------------

/** 读取写歌智能体配置。 */
export function agentConfig(): Promise<AgentConfig> {
  if (mockEnabled()) return Promise.resolve(mockApi.agentConfig());
  return invoke<AgentConfig>("agent_config");
}

/** 保存写歌智能体配置。 */
export function agentSetConfig(baseUrl: string, apiKey: string, model: string): Promise<AgentConfig> {
  if (mockEnabled()) return Promise.resolve(mockApi.agentSetConfig(baseUrl, apiKey, model));
  return invoke<AgentConfig>("agent_set_config", { baseUrl, apiKey, model });
}

/** 拉取端点上的模型 id 列表（用传入的配置，不写盘）。 */
export function agentModels(baseUrl: string, apiKey: string, model: string): Promise<string[]> {
  if (mockEnabled()) return Promise.resolve(mockApi.agentModels());
  return invoke<string[]>("agent_models", { baseUrl, apiKey, model });
}

/** 测试连通性（用传入的配置，不写盘）。 */
export function agentTest(baseUrl: string, apiKey: string, model: string): Promise<AgentProbe> {
  if (mockEnabled()) return Promise.resolve(mockApi.agentTest());
  return invoke<AgentProbe>("agent_test", { baseUrl, apiKey, model });
}

/** 全部会话摘要（历史列表）。 */
export function agentSessions(): Promise<AgentSessionSummary[]> {
  if (mockEnabled()) return Promise.resolve(mockApi.agentSessions());
  return invoke<AgentSessionSummary[]>("agent_sessions");
}

/** 载入一个会话（含完整节点树）。 */
export function agentSession(id: string): Promise<AgentSession> {
  if (mockEnabled()) return Promise.resolve(mockApi.agentSession(id));
  return invoke<AgentSession>("agent_session", { id });
}

/** 删除一个会话。 */
export function agentDeleteSession(id: string): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.agentDeleteSession(id));
  return invoke<void>("agent_delete_session", { id });
}

/** 切换某会话的活跃分支（`head`）。 */
export function agentSetHead(id: string, nodeId: string): Promise<AgentSession> {
  if (mockEnabled()) return Promise.resolve(mockApi.agentSetHead(id, nodeId));
  return invoke<AgentSession>("agent_set_head", { id, nodeId });
}

/** 发起一轮对话：返回会话 id（回答经 `engine://agent` 流式推回）。 */
export function agentSend(sessionId: string | null, parentId: string | null, text: string): Promise<string> {
  if (mockEnabled()) return Promise.resolve(mockApi.agentSend(sessionId, parentId, text));
  return invoke<string>("agent_send", { sessionId, parentId, text });
}

/** 停止当前一轮。 */
export function agentCancel(): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.agentCancel());
  return invoke<void>("agent_cancel");
}

/** 订阅写歌智能体事件。 */
export function onAgentEvent(handler: (event: AgentEvent) => void): Promise<UnlistenFn> {
  if (mockEnabled()) return mockApi.onAgentEvent(handler);
  return listen<AgentEvent>(EVENT_AGENT, (e) => handler(e.payload));
}

/** 读取 `aidaw` 命令行工具的安装状态。 */
export function cliStatus(): Promise<CliStatus> {
  if (mockEnabled()) return Promise.resolve(mockApi.cliStatus());
  return invoke<CliStatus>("cli_status");
}

/** 把 `aidaw` 接进 PATH（macOS 会弹一次管理员授权）。 */
export function installCli(): Promise<CliStatus> {
  if (mockEnabled()) return Promise.resolve(mockApi.installCli());
  return invoke<CliStatus>("install_cli");
}

/** 撤掉本应用装的 `aidaw`。 */
export function uninstallCli(): Promise<CliStatus> {
  if (mockEnabled()) return Promise.resolve(mockApi.uninstallCli());
  return invoke<CliStatus>("uninstall_cli");
}

/** 导入一个音频文件为音频片段（自然时长会换算成拍，落在 `start` 处）。 */
export function importAudio(path: string, track: number, start: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.importAudio(path, track, start));
  return invoke<Project>("import_audio", { path, track, start });
}

/**
 * 把一个**已导入**的音频素材再放一份到轨道上（复制粘贴音频片段走它）。
 *
 * 与 `import_audio` 的区别：素材早就解码过、长度也由调用方给（粘贴要的是一模一样的副本），
 * 所以不重新解码、也不重新按素材量长度。
 */
export function addAudioClip(
  track: number,
  start: number,
  length: number,
  path: string,
  offsetFrames: number,
  frames?: number,
): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.addAudioClip(track, start, length, path, offsetFrames, frames));
  return invoke<Project>("add_audio_clip", { track, start, length, path, offsetFrames, frames });
}

/** 在第 `beat` 拍切分片段（razor）：左半段沿用原 id，右半段是新片段。 */
export function splitClip(track: number, clip: number, beat: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.splitClip(track, clip, beat));
  return invoke<Project>("split_clip", { track, clip, beat });
}

/** 片段波形的数据：包络 + 素材采样率（用来把波形画在它真实的时间位置上）。 */
export interface WaveformSlice {
  peaks: number[];
  sample_rate: number;
  frames: number;
  offset_frames: number;
}

/**
 * 取某个音频文件的波形包络（每桶 min/max 各一个值，长度 = `buckets * 2`）。
 *
 * `offsetFrames` / `frames` 只取素材的**窗口**（片段引用窗口而不是整段素材）；缺省 = 整段。
 * 返回值带上采样率与夹紧后的窗口，界面据此把波形画在真实时间位置、并按方框裁切。
 */
export function audioWaveform(
  path: string,
  buckets: number,
  offsetFrames?: number,
  frames?: number,
): Promise<WaveformSlice> {
  if (mockEnabled())
    return Promise.resolve(mockApi.audioWaveform(path, buckets, offsetFrames, frames));
  return invoke<WaveformSlice>("audio_waveform", { path, buckets, offsetFrames, frames });
}

/** 片段可展开的范围（拍）：方框起点不能早于 `min_start`，末尾不能晚于 `max_end`。 */
export interface ClipBounds {
  min_start: number;
  max_end: number;
}

/**
 * 取片段可展开的范围（拍）。
 *
 * 左右拖拽据此把边缘**夹在素材之内**：「展开无法展开出原始的音频之外」。音频用素材首尾，
 * MIDI 用音符的实际跨度。
 */
export function clipBounds(track: number, clip: number): Promise<ClipBounds> {
  if (mockEnabled()) return Promise.resolve(mockApi.clipBounds(track, clip));
  return invoke<ClipBounds>("clip_bounds", { track, clip });
}

/** 订阅音频导入的解码进度（顶栏下边缘的进度条据此绘制）。 */
export function onImportEvent(handler: (p: ImportProgress) => void): Promise<UnlistenFn> {
  if (mockEnabled()) return Promise.resolve(() => {});
  return listen<ImportProgress>(EVENT_IMPORT, (event) => handler(event.payload));
}

/** 订阅音频导出进度（返回取消订阅函数）。 */
export function onExportEvent(handler: (p: ExportProgress) => void): Promise<UnlistenFn> {
  if (mockEnabled()) return Promise.resolve(() => {});
  return listen<ExportProgress>(EVENT_EXPORT, (event) => handler(event.payload));
}

// ---------------------------------------------------------------------------
// 录音（C2）
// ---------------------------------------------------------------------------

/** 本机可用的音频输入设备。 */
export function listInputDevices(): Promise<InputDeviceInfo[]> {
  if (mockEnabled()) return Promise.resolve(mockApi.listInputDevices());
  return invoke<InputDeviceInfo[]>("list_input_devices");
}

/** 选择输入设备（`null` = 关闭输入流）。选定后输入电平表即可工作。 */
export function setInputDevice(device: string | null): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.setInputDevice());
  return invoke<void>("set_input_device", { device });
}

/** 输入电平表：最近 `points` 个采样。 */
export function inputLevel(points: number): Promise<number[]> {
  if (mockEnabled()) return Promise.resolve(mockApi.inputLevel(points));
  return invoke<number[]>("input_level", { points });
}

/** 开始录音（从 `start` 拍起，后端同时走带播放）。 */
export function recordStart(track: number, start: number, device: string | null): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.recordStart());
  return invoke<void>("record_start", { track, start, device });
}

/** 结束录音；返回带新片段的工程，没录到内容则为 `null`。 */
export function recordStop(): Promise<Project | null> {
  if (mockEnabled()) return Promise.resolve(mockApi.recordStop());
  return invoke<Project | null>("record_stop");
}

// ---------------------------------------------------------------------------
// 本地 AI（音频转 MIDI / 降噪）
// ---------------------------------------------------------------------------

/** 当前 AI 任务状态。 */
export function aiStatus(): Promise<AiStatus> {
  if (mockEnabled()) return Promise.resolve(mockApi.aiStatus());
  return invoke<AiStatus>("ai_status");
}

/** 查询两个本地 AI 模型（ONNX 权重）是否已就位。 */
export function aiModels(): Promise<AiModels> {
  if (mockEnabled()) return Promise.resolve(mockApi.aiModels());
  return invoke<AiModels>("ai_models");
}

/** 开始一个本地 AI 任务：`kind` 为 `"to_midi"` / `"enhance"`，作用于 `track` 的音频片段。
 *
 * `kind === "master"` 时作用于**整个工程的离线混音**，`master` 参数才有效（`track` 被忽略）。 */
export function aiStart(
  kind: string,
  track: number,
  master?: MasterArgs,
  model?: string,
): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.aiStart(kind));
  return invoke<void>("ai_start", {
    kind,
    track,
    master: master ?? null,
    model: model ?? null,
  });
}

/** 停止当前 AI 任务。 */
export function aiCancel(): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.aiCancel());
  return invoke<void>("ai_cancel");
}

/** 订阅 AI 任务状态。 */
export function onAiEvent(handler: (status: AiStatus) => void): Promise<UnlistenFn> {
  if (mockEnabled()) {
    const onMock = (e: Event): void => handler((e as CustomEvent<AiStatus>).detail);
    window.addEventListener("ai-daw-mock-ai", onMock);
    const off = mockApi.onAiEvent();
    return Promise.resolve(() => {
      window.removeEventListener("ai-daw-mock-ai", onMock);
      off();
    });
  }
  return listen<AiStatus>(EVENT_AI, (e) => handler(e.payload));
}

// ---------------------------------------------------------------------------
// 订阅
// ---------------------------------------------------------------------------

/** 订阅引擎事件流（原始 payload，供开发者抽屉与 AI 可视化消费）。 */
export function onEngineEvent(handler: (payload: unknown) => void): Promise<UnlistenFn> {
  if (mockEnabled()) {
    // 预览里轮流放几种**代理**动作，用于演示 AI 胶囊与轨道高亮。
    const samples: unknown[] = [
      { ClipAdded: { actor: { Agent: "mcp" }, track: 1, clip: 1, start: 0, length: 2 } },
      { GainChanged: { actor: { Agent: "mcp" }, track: 2, gain_db: -3 } },
      { NoteAdded: { actor: { Agent: "mcp" }, track: 1, clip: 1, note: { pitch: 60, start: 0, duration: 1, velocity: 100, volume: 127 } } },
    ];
    let i = 0;
    const onMock = (): void => {
      handler(samples[i % samples.length]);
      i += 1;
    };
    window.addEventListener("ai-daw-mock-event", onMock);
    const off = mockApi.onEngineEvent();
    return Promise.resolve(() => {
      window.removeEventListener("ai-daw-mock-event", onMock);
      off();
    });
  }
  return listen(EVENT_ENGINE, (event) => handler(event.payload));
}

/** 订阅实时传输状态（位置/播放态）。 */
export function onTransportEvent(handler: (snapshot: TransportSnapshot) => void): Promise<UnlistenFn> {
  if (mockEnabled()) {
    const onMock = (e: Event): void => {
      handler((e as CustomEvent<TransportSnapshot>).detail);
    };
    window.addEventListener("ai-daw-mock-transport", onMock);
    const off = mockApi.onTransportEvent();
    return Promise.resolve(() => {
      window.removeEventListener("ai-daw-mock-transport", onMock);
      off();
    });
  }
  return listen(EVENT_TRANSPORT, (event) => handler(event.payload as TransportSnapshot));
}

// ---------------------------------------------------------------------------
// 插件
// ---------------------------------------------------------------------------

/** 插件扫描完成广播通道名，与后端 `state::EVENT_PLUGINS` 保持一致。 */
export const EVENT_PLUGINS = "engine://plugins";

/** 已扫描到的外部插件目录。 */
export function listPlugins(): Promise<CatalogPlugin[]> {
  if (mockEnabled()) return Promise.resolve(mockApi.listPlugins());
  return invoke<CatalogPlugin[]>("list_plugins");
}

/** 重新扫描本机插件（会 dlopen 每个插件，较慢）。 */
export function rescanPlugins(): Promise<CatalogPlugin[]> {
  if (mockEnabled()) return Promise.resolve(mockApi.rescanPlugins());
  return invoke<CatalogPlugin[]>("rescan_plugins");
}

/** 取某个插件的参数表（内置插件直接给，外部插件按目录加载）。 */
export function pluginParams(pluginId: string): Promise<BackendParam[]> {
  if (mockEnabled()) return Promise.resolve(mockApi.pluginParams(pluginId));
  return invoke<BackendParam[]>("plugin_params", { pluginId });
}

/** 在轨道效果链末尾追加一个效果器。 */
export function addEffect(track: number, pluginId: string): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.addEffect(track, pluginId));
  return invoke<Project>("add_effect", { track, pluginId });
}

/** 从轨道效果链移除一个效果器（按链内序号）。 */
export function removeEffect(track: number, index: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.removeEffect(track, index));
  return invoke<Project>("remove_effect", { track, index });
}

/** 调整效果器在链中的位置。 */
export function moveEffect(track: number, from: number, to: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.moveEffect(track, from, to));
  return invoke<Project>("move_effect", { track, from, to });
}

/** 设置效果器参数（参数 id 由插件自身定义）。 */
export function setEffectParam(
  track: number,
  index: number,
  param: string,
  value: number,
): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setEffectParam(track, index, param, value));
  return invoke<Project>("set_effect_param", { track, index, param, value });
}

/** 订阅插件扫描完成事件（启动时后台扫描结束会推送一次）。 */
export function onPluginsEvent(handler: (plugins: CatalogPlugin[]) => void): Promise<UnlistenFn> {
  if (mockEnabled()) return Promise.resolve(() => {});
  return listen<CatalogPlugin[]>(EVENT_PLUGINS, (event) => handler(event.payload));
}

// ---------------------------------------------------------------------------
// 插件原生界面（独立窗口，仅桌面端）
// ---------------------------------------------------------------------------

/**
 * 为某条轨道打开插件的原生界面（插件自己的窗口）。
 *
 * `title` 是插件展示名（按当前语言）；后端会拼成「轨道名 - 插件名」作为窗口标题。
 * 已经开着就把它带到最前面，不再开新窗。
 */
export function openPluginWindow(track: number, title: string): Promise<void> {
  if (mockEnabled()) return mockApi.openPluginWindow();
  return invoke<void>("open_plugin_window", { track, title });
}

/** 关闭某条轨道的插件窗口。 */
export function closePluginWindow(track: number): Promise<void> {
  if (mockEnabled()) return mockApi.closePluginWindow();
  return invoke<void>("close_plugin_window", { track });
}

/** 该轨道是否开着插件窗口（用户自己关掉后会变回 false）。 */
export function pluginWindowOpen(track: number): Promise<boolean> {
  if (mockEnabled()) return Promise.resolve(mockApi.pluginWindowOpen());
  return invoke<boolean>("plugin_window_open", { track });
}

/** 打开内置合成器的独立窗口（已经开着就聚焦）。`title` = 插件展示名。 */
export function openSynthWindow(track: number, title: string): Promise<void> {
  if (mockEnabled()) return mockApi.openSynthWindow();
  return invoke<void>("open_synth_window", { track, title });
}

/**
 * 打开某条轨道上某个效果器的独立界面（已经开着就聚焦）。
 *
 * 传的是 **plugin id**，序号由后端在**权威的**效果链里现算——调用方可能拿着一条过时的链
 * （别的窗口刚改过），按序号开窗会开到不存在的槽位上。
 */
export function openEffectWindow(track: number, pluginId: string, title: string): Promise<void> {
  if (mockEnabled()) return mockApi.openEffectWindow();
  return invoke<void>("open_effect_window", { track, pluginId, title });
}

/**
 * 为某条轨道上的某个**外部效果器**打开它**自带的**原生界面（已经开着就聚焦）。
 *
 * 与 {@link openEffectWindow}（内置面板窗口）不同：这条开的是插件自己的窗口。
 * 传 **plugin id**，序号由后端在权威效果链里现算，避免开到过时的槽位。
 */
export function openEffectPluginWindow(
  track: number,
  pluginId: string,
  title: string,
): Promise<void> {
  if (mockEnabled()) return mockApi.openEffectPluginWindow();
  return invoke<void>("open_effect_plugin_window", { track, pluginId, title });
}

/** 关闭某条轨道上某个外部效果器的原生界面。 */
export function closeEffectPluginWindow(track: number, pluginId: string): Promise<void> {
  if (mockEnabled()) return mockApi.closeEffectPluginWindow();
  return invoke<void>("close_effect_plugin_window", { track, pluginId });
}

/** 某条轨道上某个外部效果器的原生界面是否开着（用户自己关掉后会变回 false）。 */
export function effectPluginWindowOpen(track: number, pluginId: string): Promise<boolean> {
  if (mockEnabled()) return Promise.resolve(mockApi.effectPluginWindowOpen());
  return invoke<boolean>("effect_plugin_window_open", { track, pluginId });
}

/** 一次设置效果器的多个参数（「恢复默认值」走它：一次事务，一个撤销步骤）。 */
export function setEffectParams(
  track: number,
  index: number,
  params: Record<string, number>,
): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setEffectParams(track, index, params));
  return invoke<Project>("set_effect_params", { track, index, params });
}

/** 取最近的输出采样，供界面画实时波形。 */
export function scopeData(points: number): Promise<number[]> {
  if (mockEnabled()) return Promise.resolve(mockApi.scopeData(points));
  return invoke<number[]>("scope_data", { points });
}

/** 试听一个音（合成器界面里的键盘）。 */
export function auditionNote(
  track: number,
  note: number,
  velocity: number,
  on: boolean,
): Promise<void> {
  if (mockEnabled()) return mockApi.auditionNote();
  return invoke<void>("audition_note", { track, note, velocity, on });
}

// ---------------------------------------------------------------------------
// 用户预设（快速切音色）
// ---------------------------------------------------------------------------

/** 全部用户预设（跨工程，存在应用配置目录的 presets.json）。 */
export function listPresets(): Promise<Preset[]> {
  if (mockEnabled()) return Promise.resolve(mockApi.listPresets());
  return invoke<Preset[]>("list_presets");
}

/** 把「当前参数快照」存成一条预设。 */
export function savePreset(
  name: string,
  pluginId: string,
  params: Record<string, number>,
): Promise<Preset> {
  if (mockEnabled()) return Promise.resolve(mockApi.savePreset(name, pluginId, params));
  return invoke<Preset>("save_preset", { name, pluginId, params });
}

/** 覆盖一条已存在的预设（改名 / 更新参数）。 */
export function updatePreset(
  id: string,
  name: string,
  params: Record<string, number>,
): Promise<Preset> {
  if (mockEnabled()) return Promise.resolve(mockApi.updatePreset(id, name, params));
  return invoke<Preset>("update_preset", { id, name, params });
}

/** 删除一条预设。 */
export function deletePreset(id: string): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.deletePreset(id));
  return invoke<void>("delete_preset", { id });
}

/**
 * 一次性写回一整组参数（切换**音色**预设走它：一次事务，只占一个撤销步骤）。
 *
 * 预设只改乐器参数，**不碰效果链**——音色与效果解耦。
 */
export function setParams(track: number, params: Record<string, number>): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setParams(track, params));
  return invoke<Project>("set_params", { track, params });
}

// ---------------------------------------------------------------------------
// 插件管理器（E1：浏览 / 安装 / 导入 / 更新 / 删除）
// ---------------------------------------------------------------------------

/** 拉取插件注册表（带缓存；`force` 强制联网刷新）。 */
export function managerRegistry(force = false): Promise<ManagerRegistry> {
  if (mockEnabled()) return Promise.resolve(mockApi.managerRegistry(force));
  return invoke<ManagerRegistry>("plugin_manager_registry", { force });
}

/** 本管理器装过的插件清单。 */
export function managerInstalled(): Promise<ManagerInstalled[]> {
  if (mockEnabled()) return Promise.resolve(mockApi.managerInstalled());
  return invoke<ManagerInstalled[]>("plugin_manager_installed");
}

/** 用户自己装的插件（扫描到但不在托管库里；只读）。 */
export function managerForeign(): Promise<ForeignPlugin[]> {
  if (mockEnabled()) return Promise.resolve(mockApi.managerForeign());
  return invoke<ForeignPlugin[]>("plugin_manager_foreign");
}

/** 按当前网络配置解析出**实际会请求**的下载地址（「复制下载链接」用）。 */
export function managerResolveUrl(url: string): Promise<string> {
  if (mockEnabled()) return Promise.resolve(url);
  return invoke<string>("plugin_manager_resolve_url", { url });
}

/** 订阅插件管理器的安装/下载进度。 */
export function onManagerEvent(handler: (p: ManagerProgress) => void): Promise<UnlistenFn> {
  if (mockEnabled()) {
    const onMock = (e: Event): void => handler((e as CustomEvent<ManagerProgress>).detail);
    window.addEventListener("ai-daw-mock-manager", onMock);
    const off = mockApi.onManagerEvent();
    return Promise.resolve(() => {
      window.removeEventListener("ai-daw-mock-manager", onMock);
      off();
    });
  }
  return listen<ManagerProgress>(EVENT_MANAGER, (e) => handler(e.payload));
}

/** 从注册表下载并安装（也是更新：先替换旧文件）。 */
export function managerInstall(req: ManagerInstallRequest): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.managerInstall(req));
  return invoke<void>("plugin_manager_install", { req });
}

/** 导入本地 `.vst3` / `.clap` 到托管库。 */
export function managerImport(path: string): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.managerImport(path));
  return invoke<void>("plugin_manager_import", { path });
}

/** 删除一条托管安装。 */
export function managerRemove(key: string): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.managerRemove(key));
  return invoke<void>("plugin_manager_remove", { key });
}

/** 在系统浏览器里打开下载页（安装器类型用）。 */
export function managerOpen(url: string): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.managerOpen(url));
  return invoke<void>("plugin_manager_open", { url });
}

/** 读取插件源的网络配置（镜像 / 下载代理）。 */
export function managerNetwork(): Promise<StoreNetwork> {
  if (mockEnabled()) return Promise.resolve(mockApi.managerNetwork());
  return invoke<StoreNetwork>("plugin_manager_network");
}

/** 写入网络配置（落 `store.json`）。 */
export function managerSetNetwork(
  registryMirrors: string[],
  downloadPrefix: string,
  proxyMode: ProxyMode,
  proxyUrl: string,
  modelMirrors: string[] = [],
  modelSourcePreference: ModelSourcePreference = "auto",
): Promise<StoreNetwork> {
  if (mockEnabled()) {
    return Promise.resolve(
      mockApi.managerSetNetwork(
        registryMirrors,
        downloadPrefix,
        proxyMode,
        proxyUrl,
        modelMirrors,
        modelSourcePreference,
      ),
    );
  }
  return invoke<StoreNetwork>("plugin_manager_set_network", {
    registryMirrors,
    downloadPrefix,
    proxyMode,
    proxyUrl,
    modelMirrors,
    modelSourcePreference,
  });
}

/** 当前配置会解析成什么代理（不联网，用于设置页显示）。 */
export function managerProxyStatus(): Promise<ProxyStatus> {
  if (mockEnabled()) return Promise.resolve(mockApi.managerProxyStatus());
  return invoke<ProxyStatus>("plugin_manager_proxy_status");
}

/** 用**给定**的代理设置实际发一次请求，验证通不通（不会写盘）。 */
export function managerProbeProxy(proxyMode: ProxyMode, proxyUrl: string): Promise<ProxyProbe> {
  if (mockEnabled()) return Promise.resolve(mockApi.managerProbeProxy(proxyMode, proxyUrl));
  return invoke<ProxyProbe>("plugin_manager_probe_proxy", { proxyMode, proxyUrl });
}

// ---------------------------------------------------------------------------
// 模型下载器（Stage G：大模型按需下载，与插件共用同一份网络配置）
// ---------------------------------------------------------------------------

/** 列出全部本地 AI 模型与本地状态。 */
export function modelCatalog(): Promise<ModelStatus[]> {
  if (mockEnabled()) return Promise.resolve(mockApi.modelCatalog());
  return invoke<ModelStatus[]>("model_catalog");
}

/** 开始下载某个模型（后台线程；进度走 `engine://model`）。 */
export function modelDownload(id: string): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.modelDownload(id));
  return invoke<void>("model_download", { id });
}

/** 取消正在进行的模型下载。 */
export function modelCancel(): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.modelCancel());
  return invoke<void>("model_cancel");
}

/** 删除已下载的模型文件。 */
export function modelRemove(id: string): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.modelRemove(id));
  return invoke<void>("model_remove", { id });
}

/** 在系统文件管理器里显示模型文件。 */
export function modelReveal(id: string): Promise<void> {
  if (mockEnabled()) return Promise.resolve();
  return invoke<void>("model_reveal", { id });
}

/** 按当前网络配置解析出模型**实际会先请求**的下载地址（「复制下载链接」用）。 */
export function modelResolveUrl(id: string): Promise<string> {
  if (mockEnabled()) return Promise.resolve(mockApi.modelResolveUrl(id));
  return invoke<string>("model_resolve_url", { id });
}

/** 订阅模型下载进度。 */
export function onModelEvent(handler: (p: ModelProgress) => void): Promise<UnlistenFn> {
  if (mockEnabled()) {
    const onMock = (e: Event): void => handler((e as CustomEvent<ModelProgress>).detail);
    window.addEventListener("ai-daw-mock-model", onMock);
    const off = mockApi.onModelEvent();
    return Promise.resolve(() => {
      window.removeEventListener("ai-daw-mock-model", onMock);
      off();
    });
  }
  return listen<ModelProgress>(EVENT_MODEL, (e) => handler(e.payload));
}

// ---------------------------------------------------------------------------
// 执行后端（Execution Provider）：只是「选哪个」，没有下载
// ---------------------------------------------------------------------------

/** 当前后端选择 + 本机可用项（设置页加载时调用）。 */
export function aiBackendInfo(): Promise<BackendInfo> {
  if (mockEnabled()) return Promise.resolve(mockApi.aiBackendInfo());
  return invoke<BackendInfo>("ai_backend_info");
}

/** 保存执行后端偏好（写 `store.json` 并立即生效）。 */
export function aiSetBackend(preference: BackendPreference): Promise<BackendInfo> {
  if (mockEnabled()) return Promise.resolve(mockApi.aiSetBackend(preference));
  return invoke<BackendInfo>("ai_set_backend", { preference });
}
