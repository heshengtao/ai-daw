/**
 * 工程状态与命令封装：所有 project 变更都来自命令返回的快照。
 *
 * 每个编辑动作都对应后端的一条命令（引擎的 `Command`），因此撤销/重做、事件流与
 * 实时播放天然一致。界面层**不得**直接改写 `project`，否则历史会与状态脱节。
 */
import { defineStore } from "pinia";
import { ref } from "vue";
import * as api from "@/api/tauri";
import { useI18n } from "@/i18n";
import { useParamBindingsStore } from "@/stores/paramBindings";
import { useSettingsStore } from "@/stores/settings";
import type { AutomationPoint, ClipPlacement, ClipRef, HistoryState, Keyframe, MixerState, Note, NoteEdit, Project, Send } from "@/api/types";
import { meterAt, meterBeatAt, tempoAt } from "@/utils/tempo";

/**
 * 默认分轨名的 i18n key，**顺序必须与引擎的 `STARTER_TRACKS` 一致**。
 *
 * 名字只在**创建工程那一刻**按当前语言生成；写进工程之后它就是用户数据了，之后切换界面
 * 语言不会再改动它——否则用户亲手改过的轨道名会被语言切换冲掉。
 */
const STARTER_TRACK_KEYS = [
  "starter.lead",
  "starter.chords",
  "starter.bass",
  "starter.drums",
  "starter.pad",
] as const;

export const useProjectStore = defineStore("project", () => {
  const i18n = useI18n();
  const settings = useSettingsStore();
  const project = ref<Project | null>(null);
  const loading = ref(false);
  const loadError = ref<string | null>(null);
  const undoDepth = ref(0);
  const redoDepth = ref(0);
  /** 是否有未保存的改动。 */
  const dirty = ref(false);
  /** 当前工程对应的文件路径；尚未保存过时为 null。 */
  const currentPath = ref<string | null>(null);

  /** 按当前语言生成默认分轨名（顺序与引擎模板一致）。 */
  function starterTrackNames(): string[] {
    return STARTER_TRACK_KEYS.map((key) => i18n.t(key));
  }

  /** 重新查询历史深度（撤销/重做按钮的可用性）。 */
  async function refreshDepths(): Promise<void> {
    try {
      const d = await api.historyDepths();
      undoDepth.value = d.undo;
      redoDepth.value = d.redo;
      dirty.value = d.dirty;
    } catch {
      // 深度只影响按钮状态，查询失败不值得打断用户。
    }
  }

  function applyHistory(state: HistoryState): void {
    project.value = state.project;
    undoDepth.value = state.undo_depth;
    redoDepth.value = state.redo_depth;
    dirty.value = state.dirty;
  }

  /** 执行一条返回完整工程快照的编辑命令。 */
  async function edit(run: () => Promise<Project>): Promise<void> {
    project.value = await run();
    await refreshDepths();
  }

  /** 重新拉取工程快照。 */
  async function reload(): Promise<void> {
    loading.value = true;
    loadError.value = null;
    try {
      project.value = await api.getProject();
      await refreshDepths();
    } catch (err) {
      loadError.value = String(err);
    } finally {
      loading.value = false;
    }
  }

  // ---- 轨道 ----

  /** 新增轨道。 */
  function addTrack(name: string): Promise<void> {
    return edit(() => api.addTrack(name));
  }

  /** 删除轨道。 */
  function removeTrack(track: number): Promise<void> {
    return edit(() => api.removeTrack(track));
  }

  /** 重命名轨道。 */
  function renameTrack(track: number, name: string): Promise<void> {
    const trimmed = name.trim();
    if (!trimmed) return Promise.resolve();
    return edit(() => api.renameTrack(track, trimmed));
  }

  /** 设置轨道乐器。 */
  function setTrackInstrument(track: number, pluginId: string): Promise<void> {
    return edit(() => api.setTrackInstrument(track, pluginId));
  }

  /** 设置乐器参数。 */
  function setParam(track: number, param: string, value: number): Promise<void> {
    if (!Number.isFinite(value)) return Promise.resolve();
    return edit(() => api.setParam(track, param, value));
  }

  /** 一次性设置多个乐器参数（切换预设用：一次事务，只占一个撤销步骤）。 */
  function setParams(track: number, params: Record<string, number>): Promise<void> {
    const finite = Object.fromEntries(
      Object.entries(params).filter(([, value]) => Number.isFinite(value)),
    );
    if (Object.keys(finite).length === 0) return Promise.resolve();
    return edit(() => api.setParams(track, finite));
  }

  // ---- 效果链 ----

  /** 在轨道效果链末尾追加一个效果器。 */
  function addEffect(track: number, pluginId: string): Promise<void> {
    return edit(() => api.addEffect(track, pluginId));
  }

  /** 从轨道效果链移除一个效果器（按链内序号）。 */
  function removeEffect(track: number, index: number): Promise<void> {
    return edit(() => api.removeEffect(track, index));
  }

  /** 调整效果器在链中的位置。 */
  function moveEffect(track: number, from: number, to: number): Promise<void> {
    return edit(() => api.moveEffect(track, from, to));
  }

  /** 一次设置效果器的多个参数（「恢复默认值」走它）；一次事务，一个撤销步骤。 */
  function setEffectParams(
    track: number,
    index: number,
    params: Record<string, number>,
  ): Promise<void> {
    const finite = Object.fromEntries(
      Object.entries(params).filter(([, value]) => Number.isFinite(value)),
    );
    if (Object.keys(finite).length === 0) return Promise.resolve();
    return edit(() => api.setEffectParams(track, index, finite));
  }

  /** 设置效果器参数。 */
  function setEffectParam(
    track: number,
    index: number,
    param: string,
    value: number,
  ): Promise<void> {
    if (!Number.isFinite(value)) return Promise.resolve();
    return edit(() => api.setEffectParam(track, index, param, value));
  }

  // ---- 片段 ----

  /** 新增片段。 */
  function addClip(track: number, start: number, length: number): Promise<void> {
    return edit(() => api.addClip(track, start, length));
  }

  /**
   * 导入一个音频文件为音频片段，落在 `start` 拍。
   *
   * 片段的拍长由后端按素材的自然时长换算（导入时不变调）；之后拖宽窄或改速度才会变调。
   */
  function importAudio(path: string, track: number, start: number): Promise<void> {
    return edit(() => api.importAudio(path, track, start));
  }

  /**
   * 把一个已导入的音频素材再放一份到轨道上（复制粘贴音频片段走它）。
   *
   * 与 `importAudio` 的区别：不重新解码、不重新量长度——粘贴要的是一模一样的副本。
   */
  function addAudioClip(
    track: number,
    start: number,
    length: number,
    path: string,
    offsetFrames: number,
    frames?: number,
  ): Promise<void> {
    return edit(() => api.addAudioClip(track, start, length, path, offsetFrames, frames));
  }

  /** 在第 `beat` 拍切分片段（razor）：左半段沿用原 id，右半段是新片段。 */
  function splitClip(track: number, clip: number, beat: number): Promise<void> {
    return edit(() => api.splitClip(track, clip, beat));
  }

  /**
   * 套用一次录音返回的工程快照。
   *
   * 录音已经是后端的一次可撤销编辑（`AddAudioClip`），这里只把新快照装上并刷新历史深度。
   */
  async function applyRecorded(next: Project): Promise<void> {
    project.value = next;
    await refreshDepths();
  }

  /** 删除片段。 */
  function removeClip(track: number, clip: number): Promise<void> {
    return edit(() => api.removeClip(track, clip));
  }

  /** 移动片段。 */
  function moveClip(track: number, clip: number, start: number): Promise<void> {
    return edit(() => api.moveClip(track, clip, start));
  }

  /** 批量移动片段（多选拖动，一条撤销记录）。 */
  function moveClips(items: ClipPlacement[]): Promise<void> {
    if (items.length === 0) return Promise.resolve();
    return edit(() => api.moveClips(items));
  }

  /** 改变片段长度。 */
  function resizeClip(track: number, clip: number, length: number): Promise<void> {
    return edit(() => api.resizeClip(track, clip, length));
  }

  // ---- 音符 ----

  /** 新增音符。 */
  function addNote(track: number, clip: number, note: Note): Promise<void> {
    return edit(() => api.addNote(track, clip, note));
  }

  /** 改写音符（按片段内索引）。 */
  function setNote(track: number, clip: number, index: number, note: Note): Promise<void> {
    return edit(() => api.setNote(track, clip, index, note));
  }

  /**
   * 批量改写音符：多选拖动与属性面板都走这里。
   * 一次提交 = 一条撤销记录（否则改 10 个音符要按 10 次撤销）。
   */
  function setNotes(track: number, clip: number, entries: NoteEdit[]): Promise<void> {
    if (entries.length === 0) return Promise.resolve();
    return edit(() => api.setNotes(track, clip, entries));
  }

  /** 删除音符（按片段内索引）。 */
  function removeNote(track: number, clip: number, index: number): Promise<void> {
    return edit(() => api.removeNote(track, clip, index));
  }

  /** 批量删除音符（框选后删除）。 */
  function removeNotes(track: number, clip: number, indices: number[]): Promise<void> {
    if (indices.length === 0) return Promise.resolve();
    return edit(() => api.removeNotes(track, clip, indices));
  }

  /** 批量删除片段（可跨轨道）。 */
  function removeClips(items: ClipRef[]): Promise<void> {
    if (items.length === 0) return Promise.resolve();
    return edit(() => api.removeClips(items));
  }

  // ---- 混音 ----

  /** 设置轨道增益。 */
  function setGain(track: number, gainDb: number): Promise<void> {
    return edit(() => api.setGain(track, gainDb));
  }

  /** 设置轨道声像。 */
  function setPan(track: number, pan: number): Promise<void> {
    return edit(() => api.setPan(track, pan));
  }

  /** 设置轨道静音。 */
  function setMute(track: number, mute: boolean): Promise<void> {
    return edit(() => api.setMute(track, mute));
  }

  /** 设置轨道独奏。 */
  function setSolo(track: number, solo: boolean): Promise<void> {
    return edit(() => api.setSolo(track, solo));
  }

  /** 设置总输出增益。 */
  function setMasterGain(gainDb: number): Promise<void> {
    return edit(() => api.setMasterGain(gainDb));
  }

  /** 设置轨道的发送列表（整体替换；空数组 = 清空）。 */
  function setSends(track: number, sends: Send[]): Promise<void> {
    return edit(() => api.setSends(track, sends));
  }

  /**
   * 设置一条自动化车道（整体替换；空 `points` = 删除该车道）。
   *
   * 自动化是「仅混音」变更，后端会把它直接推给音频线程逐块求值，不会打断音频。
   */
  function setAutomation(track: number, param: string, points: AutomationPoint[]): Promise<void> {
    return edit(() => api.setAutomation(track, param, points));
  }

  /**
   * 混音监听预览：只改实时声音，不写工程、不进历史。
   * 拖动推子时高频调用；松手时再调用 `setGain` 落一次正式编辑。
   */
  function monitorMix(track: number, mix: MixerState): Promise<void> {
    return api.monitorTrackMix(track, mix);
  }

  // ---- 传输设置 ----

  /** 设置速度；非法值（空/NaN/越界）直接忽略，绝不把 0 发给引擎。 */
  async function setTempo(bpm: number): Promise<void> {
    if (!Number.isFinite(bpm) || bpm < 20 || bpm > 300) return;
    project.value = await api.setTempo(bpm);
    await refreshDepths();
  }

  /** 设置拍号；只在合法范围内发送（引擎还会再归一化一次）。 */
  async function setTimeSignature(num: number, den: number): Promise<void> {
    const n = Math.round(num);
    const d = Math.round(den);
    if (!Number.isFinite(n) || !Number.isFinite(d)) return;
    if (n < 1 || n > 32 || d < 1 || d > 32) return;
    project.value = await api.setTimeSignature(n, d);
    await refreshDepths();
  }

  // ---- 速度/拍号关键点 ----

  /** 第 `beat` 拍处是否「对准」某个关键点（半格容差）。 */
  function keyframeAt(beat: number): Keyframe | null {
    const t = project.value?.transport;
    if (!t) return null;
    const half = meterBeatAt(beat, t) / 8; // 半个 1/16 格
    return (t.keyframes ?? []).find((k) => Math.abs(k.beat - beat) < half) ?? null;
  }

  /** 按所在分段的 1/16 拍吸附。 */
  function snapBeat(beat: number): number {
    const t = project.value?.transport;
    if (!t) return beat;
    const s = meterBeatAt(beat, t) / 4;
    return Math.round(beat / s) * s;
  }

  /** 插入/更新关键点；未给的值继承该处的生效值。 */
  function setKeyframeAt(beat: number, v: { tempo?: number; num?: number; den?: number }): Promise<void> {
    const t = project.value?.transport;
    if (!t) return Promise.resolve();
    const cur = meterAt(beat, t);
    return edit(() => api.setKeyframe(beat, v.tempo ?? tempoAt(beat, t), v.num ?? cur.num, v.den ?? cur.den));
  }

  function removeKeyframeAt(beat: number): Promise<void> {
    return edit(() => api.removeKeyframe(beat));
  }

  /** 改 BPM：对准关键点 → 改该关键点；否则 → 改基准（第 0 拍）。 */
  function applyTempoAt(beat: number, bpm: number): Promise<void> {
    if (!Number.isFinite(bpm) || bpm < 20 || bpm > 300) return Promise.resolve();
    const kf = keyframeAt(beat);
    if (kf) return setKeyframeAt(kf.beat, { tempo: bpm });
    return setTempo(bpm);
  }

  /** 改拍号：对准关键点 → 改该关键点；否则 → 改基准（第 0 拍）。 */
  function applyMeterAt(beat: number, num: number, den: number): Promise<void> {
    const kf = keyframeAt(beat);
    if (kf) return setKeyframeAt(kf.beat, { num, den });
    return setTimeSignature(num, den);
  }

  // ---- 工程 ----

  /** 新建工程（铺默认分轨模板；分轨名按当前语言生成，之后固定；速度取设置里的默认 BPM）。 */
  async function newProject(): Promise<void> {
    project.value = await api.newProject(starterTrackNames(), settings.defaultTempo);
    currentPath.value = null;
    await refreshDepths();
  }

  /** 重命名当前工程；空名字忽略。 */
  async function renameProject(name: string): Promise<void> {
    const trimmed = name.trim();
    if (!trimmed) return;
    project.value = await api.renameProject(trimmed);
    await refreshDepths();
  }

  /**
   * 启动时把初始工程的分轨名改成当前语言。
   *
   * 初始工程是在界面起来之前建的，那时还不知道界面语言，只能先用引擎的兜底名。后端只在
   * 工程**仍是默认模板**时才改名（用户动过就一个字都不动），而且改名不进撤销栈。
   */
  async function localizeStarterTracks(): Promise<void> {
    project.value = await api.localizeStarterTracks(starterTrackNames());
  }

  /** 从文件导入工程（会清空历史），并记下它的路径。 */
  async function openFrom(path: string): Promise<void> {
    project.value = await api.openProject(path);
    currentPath.value = path;
    await refreshDepths();
  }

  /**
   * 把当前工程保存到文件，返回写入路径；成功后不再是 dirty。
   *
   * 路径**变了**就把参数绑定记录搬过去（`rekeyProject`）：未保存的工程用 `untitled` 当工程标识
   * （见 `paramBindings.scopeKey`），首次保存后作用域键从 `untitled::<id>` 变成 `<path>::<id>`——
   * 不搬的话用户刚绑好的参数会「变回默认」。另存为（旧路径 → 新路径）同理；同路径写回则无事可做
   * （`rekeyProject` 自己判 `from === to`）。打开既有工程不走这里，它的键本来就按路径现算。
   */
  async function saveTo(target: string): Promise<string> {
    const from = currentPath.value;
    const written = await api.saveProject(target);
    currentPath.value = written;
    useParamBindingsStore().rekeyProject(from, written);
    await refreshDepths();
    return written;
  }

  /** 读取后端记住的工程路径（启动还原会话时用）。 */
  async function loadPath(): Promise<void> {
    try {
      currentPath.value = await api.currentProjectPath();
    } catch {
      // 拿不到路径不影响编辑，忽略。
    }
  }

  /** 把当前工程写入会话缓存（退出前 / 编辑过程中定期调用）。 */
  function cacheSession(): Promise<void> {
    return api.cacheSession(currentPath.value);
  }

  /** 撤销；没有可撤销内容时返回 false。 */
  async function undo(): Promise<boolean> {
    if (undoDepth.value === 0) return false;
    applyHistory(await api.undo());
    return true;
  }

  /** 重做；没有可重做内容时返回 false。 */
  async function redo(): Promise<boolean> {
    if (redoDepth.value === 0) return false;
    applyHistory(await api.redo());
    return true;
  }

  return {
    project,
    loading,
    loadError,
    undoDepth,
    redoDepth,
    dirty,
    currentPath,
    reload,
    addTrack,
    removeTrack,
    renameTrack,
    setTrackInstrument,
    setParam,
    setParams,
    addEffect,
    removeEffect,
    moveEffect,
    setEffectParam,
    setEffectParams,
    addClip,
    importAudio,
    addAudioClip,
    removeClip,
    moveClip,
    moveClips,
    resizeClip,
    splitClip,
    applyRecorded,
    addNote,
    setNote,
    setNotes,
    removeNote,
    removeNotes,
    removeClips,
    setGain,
    setPan,
    setMute,
    setSolo,
    setMasterGain,
    setSends,
    setAutomation,
    monitorMix,
    setTempo,
    setTimeSignature,
    keyframeAt,
    snapBeat,
    setKeyframeAt,
    removeKeyframeAt,
    applyTempoAt,
    applyMeterAt,
    newProject,
    renameProject,
    localizeStarterTracks,
    openFrom,
    saveTo,
    loadPath,
    cacheSession,
    undo,
    redo,
    refreshDepths,
  };
});
