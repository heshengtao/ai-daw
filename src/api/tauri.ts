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
  AnalysisReport,
  BackendParam,
  CatalogPlugin,
  ClipPlacement,
  ClipRef,
  HistoryDepths,
  HistoryState,
  McpStatus,
  MixerState,
  Note,
  NoteEdit,
  Preset,
  Project,
  TransportSnapshot,
} from "./types";

/** 引擎事件广播通道名，与后端 `state::EVENT_ENGINE` 保持一致。 */
export const EVENT_ENGINE = "engine://event";
/** 传输状态广播通道名，与后端 `state::EVENT_TRANSPORT` 保持一致。 */
export const EVENT_TRANSPORT = "engine://transport";

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
export function newProject(trackNames?: string[]): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.newProject(trackNames));
  return invoke<Project>("new_project", { trackNames });
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

/** 将当前工程导出为 WAV 文件，返回写入路径。 */
export function exportWav(path: string, sampleRate = 44100): Promise<string> {
  if (mockEnabled()) return Promise.resolve(mockApi.exportWav(path));
  return invoke<string>("export_wav", { path, sampleRate });
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
// 订阅
// ---------------------------------------------------------------------------

/** 订阅引擎事件流（原始 payload，仅供开发者抽屉消费）。 */
export function onEngineEvent(handler: (payload: unknown) => void): Promise<UnlistenFn> {
  if (mockEnabled()) {
    const onMock = (): void => handler({ TrackAdded: { actor: "Human", id: 4, name: "轨道 4" } });
    window.addEventListener("daw-party-mock-event", onMock);
    const off = mockApi.onEngineEvent();
    return Promise.resolve(() => {
      window.removeEventListener("daw-party-mock-event", onMock);
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
    window.addEventListener("daw-party-mock-transport", onMock);
    const off = mockApi.onTransportEvent();
    return Promise.resolve(() => {
      window.removeEventListener("daw-party-mock-transport", onMock);
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

/** 为某条轨道打开插件的原生界面（插件自己的窗口）。 */
export function openPluginWindow(track: number): Promise<void> {
  if (mockEnabled()) return mockApi.openPluginWindow();
  return invoke<void>("open_plugin_window", { track });
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

/** 打开内置合成器的独立窗口（已经开着就聚焦）。 */
export function openSynthWindow(track: number): Promise<void> {
  if (mockEnabled()) return mockApi.openSynthWindow();
  return invoke<void>("open_synth_window", { track });
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

/** 一次性写回一整组参数（切换预设走它：一次事务，只占一个撤销步骤）。 */
export function setParams(track: number, params: Record<string, number>): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setParams(track, params));
  return invoke<Project>("set_params", { track, params });
}
