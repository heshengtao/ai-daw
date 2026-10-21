/** 与引擎 `ai_daw_engine::model` 对应的前端视图类型（snake_case，id 为裸数字）。 */

/** 音符级音高表达：推弦 / 滑音 / 揉弦。 */
export interface PitchExpression {
  /** 起音时的音高偏移（音分，100 = 1 个半音）。预推弦 = 200。 */
  bend?: number;
  /** 推弦 / 滑音的目标偏移（音分）。 */
  bend_to?: number;
  /** 从 `bend` 到 `bend_to` 的时间（拍）；0 = 立即到达。 */
  bend_time?: number;
  /** 揉弦深度（音分，0 = 不揉）。 */
  vibrato?: number;
  /** 揉弦速率（Hz）；0 = 默认 5.5。 */
  vibrato_rate?: number;
  /** 起音后延迟多少拍开始揉。 */
  vibrato_delay?: number;
  /** 揉弦渐入时长（拍）；0 = 立即满深度。 */
  vibrato_fade?: number;
}

export interface Note {
  /**
   * 片段内**稳定**标识（批量工具用它定位，而不是数组下标）。
   *
   * 旧工程没有这个字段，读入时由引擎按片段顺序补齐；界面构造新音符时可以不填。
   */
  id?: number;
  pitch: number;
  start: number;
  duration: number;
  /** 演奏强度（0..=127）：手弹得多重。内置合成器把它折算成振幅。 */
  velocity: number;
  /** 单音音量（0..=127，127 = 不衰减）：纯增益微调，不改变音色。 */
  volume: number;
  /**
   * 起始偏移（拍，正 = 延后发声）：**记谱位置**（`start`）与实际发声位置分开。
   *
   * 扫弦就是给同一 `start` 的一组音符写递增偏移；「自由网格」下的手感微调也用它。
   * 缺省 0 = 与 `start` 同时发声。
   */
  offset?: number;
  /** 连奏：NoteOff 延到同音高的下一个音起点（同片段内）；缺省 false。 */
  legato?: boolean;
  /** 音高表达（推弦 / 揉弦 / 滑音）；缺省 = 无。 */
  expr?: PitchExpression;
}

/** 按 id 定位的音符改写项：只带要改的字段。 */
export interface NotePatch {
  id: number;
  pitch?: number;
  start?: number;
  duration?: number;
  velocity?: number;
  volume?: number;
  /** 起始偏移（拍）；`0` = 回到「同时发声」。 */
  offset?: number;
  legato?: boolean;
  /** 音高表达；给了就整体替换，`{}` = 清除。 */
  expr?: PitchExpression;
}

/** 片段内拍范围（左闭右开）：批量变换只作用于起点落在其中的音符。 */
export interface NoteRange {
  start: number;
  end: number;
}

/**
 * 力度变换的形状（与 Rust `VelocityShape` 的外部标记一致）。
 *
 * 三选一：`{ fixed }` 固定值 / `{ ramp: { from, to } }` 渐变 / `{ scale }` 相对缩放。
 */
export type VelocityShape =
  | { fixed: number }
  | { ramp: { from: number; to: number } }
  | { scale: number };

/** 音域约束的处理方式：八度折返 / 只报告。 */
export type FitMode = "fold" | "report";

/** 音频素材引用：片段指向一段外部音频文件（绝对路径）。 */
export interface AudioSource {
  /** 素材文件路径。 */
  path: string;
  /** 素材内的起始帧（crop 起点）。 */
  offset_frames: number;
  /**
   * 素材窗口长度（源帧）；`undefined` = 取到素材结尾（旧工程）。
   *
   * 切分音频片段时必须知道它，否则「左半段」会一路播到结尾、被拉伸变调。
   */
  frames?: number;
  /** 播放速度倍率（1 = 原速）。**与工程 BPM 解耦**：改它只改时长，不变调。缺省 = 1。 */
  rate?: number;
  /** 升降调（半音，0 = 不变，正 = 升高）。改它只变调，不改时长。缺省 = 0。 */
  pitch_semitones?: number;
}

/** 片段：MIDI（`notes`）或音频（`audio`）。 */
export interface Clip {
  id: number;
  start: number;
  length: number;
  notes: Note[];
  /** 音频素材；存在即表示这是**音频片段**（此时 `notes` 必为空）。MIDI 片段不带该键。 */
  audio?: AudioSource;
}

/** 片段引用（轨道 + 片段）：框选删除需要跨轨道寻址。 */
export interface ClipRef {
  track: number;
  clip: number;
}

/** 音符改写项：片段内序号 + 新值（批量改写用）。 */
export interface NoteEdit {
  index: number;
  note: Note;
}

/** 片段放置：把某个片段移动到新的位置（批量拖动 / 跨轨道拖动用）。 */
export interface ClipPlacement {
  /** 片段**当前**所在轨道。 */
  track: number;
  clip: number;
  /** 目标轨道；与 `track` 相同表示只改起点。 */
  to_track: number;
  start: number;
}

export interface MixerChannel {
  gain_db: number;
  pan: number;
  mute: boolean;
  solo: boolean;
}

/** 一路发送（aux send）：把本轨推子后的信号按增益送进目标轨道。 */
export interface Send {
  /** 目标轨道 id（通常是一条总线轨道）。 */
  target: number;
  /** 发送增益（dB）。 */
  gain_db: number;
}

/** 自动化曲线上的一个关键点。 */
export interface AutomationPoint {
  /** 位置（拍）。 */
  beat: number;
  /** 参数值。 */
  value: number;
}

/** 一条自动化车道（参数 + 折线）。 */
export interface AutomationLane {
  /** 参数名：`gain_db` 或 `pan`。 */
  param: string;
  /** 关键点（按 beat 升序）。 */
  points: AutomationPoint[];
}

export interface PluginInstance {
  plugin_id: string;
  params: Record<string, number>;
  state: number[];
}

export interface Track {
  id: number;
  name: string;
  instrument: PluginInstance | null;
  /** 效果器链：按顺序处理（乐器输出 → effects[0] → … → 轨道混音）。 */
  effects: PluginInstance[];
  clips: Clip[];
  mixer: MixerChannel;
  /** 发送（aux send）；空数组时后端不写这个键，所以是可选。 */
  sends?: Send[];
  /** 自动化车道；空数组时后端不写这个键。 */
  automation?: AutomationLane[];
  /** 编曲元数据（分工 / 目标音域）；为空时后端不写这个键。 */
  meta?: TrackMeta;
}

/** 轨道目标音域（MIDI 音高，含两端）。 */
export interface PitchRange {
  min: number;
  max: number;
}

/** 编曲元数据：轨道在曲子里的分工与目标音域。 */
export interface TrackMeta {
  /** `lead` / `chords` / `bass` / `drums` / `pad` / `fx`… */
  role?: string;
  pitch_range?: PitchRange;
  /** MIDI 弯音范围（半音，默认 2）。 */
  pitch_bend_range?: number;
}

/** 一个小节的和弦标记（工程元数据）。 */
export interface HarmonyBar {
  start: number;
  length: number;
  symbol: string;
  source?: string;
}

/** 段落标记（前奏 / 主歌 / 副歌…）。 */
export interface Section {
  name: string;
  start: number;
  end: number;
  role?: string;
}

/** 时间轴上的一个关键点：从 `beat` 起 BPM 与拍号切换。 */
export interface Keyframe {
  beat: number;
  tempo: number;
  time_sig_num: number;
  time_sig_den: number;
}

export interface Transport {
  tempo: number;
  time_sig_num: number;
  time_sig_den: number;
  /**
   * 速度/拍号关键点（按 beat 升序）。空表示整首恒定。
   *
   * **可选**：Rust 侧用了 `#[serde(default, skip_serializing_if = "Vec::is_empty")]`，
   * 没有任何关键点时这个字段**不会出现在 IPC 载荷里**（这也是老工程零改动的原因），
   * 所以前端读到的可能是 `undefined`——所有读取处都要 `?? []`。
   */
  keyframes?: Keyframe[];
}

export interface Project {
  name: string;
  transport: Transport;
  tracks: Track[];
  master: MixerChannel;
  /** 小节和弦表；空时后端不写这个键。 */
  harmony?: HarmonyBar[];
  /** 段落表；空时后端不写这个键。 */
  sections?: Section[];
  /** 音符级标签；键是 `"track:clip:id"`，空时后端不写这个键。 */
  note_tags?: Record<string, NoteTag>;
}

/** 音符级标签：装饰音 / 奏法。不改变播放。 */
export interface NoteTag {
  ornament?: string;
  articulation?: string;
}

// ---------------------------------------------------------------------------
// 编曲分析结果（只读工具的输出形状，与 `crates/ipc/src/report.rs` 一一对应）
// ---------------------------------------------------------------------------

/** 严重程度：`info < warning < error`。 */
export type Severity = "info" | "warning" | "error";

/** 每小节的音符密度。 */
export interface BarDensity {
  bar: number;
  start: number;
  count: number;
}

/** 单条轨道（或其中某个片段）的音符统计。 */
export interface NoteStats {
  track: number;
  clip?: number;
  note_count: number;
  lowest_pitch?: number;
  highest_pitch?: number;
  mean_pitch?: number;
  range_semitones?: number;
  mean_velocity?: number;
  span_beats: number;
  mean_notes_per_bar: number;
  bars: BarDensity[];
  overlapping_notes: number;
  /** 同轨内小二度 / 小九度重叠的音符对数。 */
  semitone_clashes: number;
  /** 这些半音冲突总共持续的拍数。 */
  semitone_clash_beats: number;
  notes_outside_clip: number;
}

/** 两条轨道之间的音区重叠。 */
export interface TrackOverlap {
  a: number;
  b: number;
  overlap_beats: number;
  unison_notes: number;
  /** 小二度 / 小九度、且时间重叠的音符对数。 */
  semitone_clashes: number;
  /** 这些半音冲突总共持续的拍数。 */
  semitone_clash_beats: number;
  shared_pitch_classes: number[];
  pitch_gap_semitones?: number;
  conflict: boolean;
  suggestion: string;
}

/** 跨轨重叠报告。 */
export interface OverlapReport {
  tracks: number[];
  pairs: TrackOverlap[];
  summary: string[];
}

/** 和声问题类别。 */
export type HarmonyIssueKind =
  | "non_chord_tone"
  | "ornament"
  | "scale_tone"
  | "unresolved_leading_tone"
  | "suspension"
  | "extension"
  | "track_overlap";

/** 一条和声问题。 */
export interface HarmonyIssue {
  kind: HarmonyIssueKind;
  severity: Severity;
  bar: number;
  bar_start: number;
  /** 小节内拍位（0 起）。 */
  beat: number;
  track: number;
  pitch: number;
  pitch_name: string;
  duration: number;
  message: string;
}

/** 每个小节的和声判定。 */
export interface HarmonyBarReport {
  bar: number;
  start: number;
  length: number;
  symbol: string;
  chord_tones: string[];
  tension: Severity;
  issues: number;
}

/** 和声检查报告。 */
export interface HarmonyReport {
  bars: HarmonyBarReport[];
  issues: HarmonyIssue[];
  conflict_level: Severity;
  notes: string[];
}

/** 和声检查的宽松策略（默认：允许延伸音与短经过音）。 */
export interface HarmonyPolicy {
  allow_extensions: boolean;
  allow_approach_notes: boolean;
  allow_non_chord_tone_duration: number;
}

/** 撤销 / 重做之后的返回：工程快照 + 最新历史深度 + 是否有未保存改动。 */
export interface HistoryState {
  project: Project;
  undo_depth: number;
  redo_depth: number;
  dirty: boolean;
}

/** 当前可撤销 / 可重做的次数，以及是否有未保存改动。 */
export interface HistoryDepths {
  undo: number;
  redo: number;
  dirty: boolean;
}

/** 历史缩略图里的一段片段。 */
export interface HistoryClip {
  start: number;
  length: number;
  /** 音频片段（画成条纹 / 低透明度）。 */
  audio: boolean;
}

/** 历史缩略图里的一条轨道。 */
export interface HistoryTrack {
  name: string;
  clips: HistoryClip[];
}

/** 一份历史工程的展示信息（与后端 `history::HistoryEntry` 一一对应）。 */
export interface ProjectHistoryEntry {
  id: string;
  name: string;
  /** 记录时间（Unix 毫秒）。 */
  saved_at: number;
  /** 当时的工程文件路径；未保存过为 `null`。 */
  path: string | null;
  /** 当时是否有未保存改动。 */
  dirty: boolean;
  tempo: number;
  /** 最后一个片段的结束位置（拍）。 */
  end_beat: number;
  track_count: number;
  clip_count: number;
  note_count: number;
  tracks: HistoryTrack[];
  /** 内容指纹（后端用于连续重复记录去重；界面不展示）。 */
  fingerprint?: string;
}

/** 实时传输状态快照（与后端 `audio::TransportSnapshot` 一一对应）。 */
export interface TransportSnapshot {
  /** 实时播放是否可用（音频设备是否打开成功）。 */
  available: boolean;
  playing: boolean;
  /** 当前位置（帧）。 */
  position_frames: number;
  /** 设备采样率（Hz）；不可用时为 0。 */
  sample_rate: number;
  /** 是否开启循环播放。 */
  loop: boolean;
  /** 播放范围起点（帧）；与 `loop_end_frames` 一起构成循环区间。 */
  loop_start_frames: number;
  /** 播放范围终点（帧）。`<= loop_start_frames` = 没设范围。 */
  loop_end_frames: number;
  /** 不可用时的原因。 */
  error: string | null;
}

/** 混音通道的四个参数（推子/声像/静音/独奏一次给全，避免多次往返）。 */
export interface MixerState {
  gain_db: number;
  pan: number;
  mute: boolean;
  solo: boolean;
}

/** 扫描到的外部插件条目（与后端 `host_vst3::CatalogPlugin` 一一对应）。 */
export interface CatalogPlugin {
  /** 全局唯一 id：`vst3:<uid>` / `clap:<unique_id>`。 */
  id: string;
  name: string;
  vendor: string;
  kind: "instrument" | "effect";
  /** 格式标识：`"vst3"` / `"clap"`。 */
  format: string;
  accepts_midi: boolean;
  has_editor: boolean;
  path: string;
}

/** 插件自报的参数元信息（与 `plugin-api` 的 `ParamInfo` 同构）。 */
export interface BackendParam {
  id: string;
  name: string;
  min: number;
  max: number;
  default: number;
  automatable: boolean;
}

/** 一条用户预设（与后端 `presets::Preset` 一一对应）。 */
export interface Preset {
  id: string;
  name: string;
  /** 归属的插件 id（`builtin.synth` / `vst3:…` / `clap:…`）。 */
  plugin_id: string;
  /** 参数快照：参数 id → 值。 */
  params: Record<string, number>;
  /** 创建时间（Unix 毫秒），用于排序。 */
  created_at: number;
}

/** 音色记忆的混音部分（不含发送）。 */
export interface SoundMixer {
  gain_db: number;
  pan: number;
}

/** 音色记忆里的一个插件快照。`state` 是 base64；空字符串 = 无二进制状态。 */
export interface PluginSnapshot {
  plugin_id: string;
  params: Record<string, number>;
  state: string;
}

/** 列表用的摘要（不含 params / state）。 */
export interface SoundMemorySummary {
  id: string;
  name: string;
  description: string;
  tags: string[];
  role: string | null;
  instrument_plugin_id: string | null;
  effect_plugin_ids: string[];
  mixer: SoundMixer;
  source_project: string | null;
  created_at: number;
  updated_at: number;
}

/** 一条完整的音色记忆。 */
export interface SoundMemory {
  id: string;
  name: string;
  description: string;
  tags: string[];
  role: string | null;
  instrument: PluginSnapshot | null;
  effects: PluginSnapshot[];
  mixer: SoundMixer;
  source_project: string | null;
  created_at: number;
  updated_at: number;
}

/**
 * 响度与峰值（EBU R128 / ITU-R BS.1770）。
 *
 * 注意：静音 / 无信号时这些值是负无穷（-∞），经 JSON 会变成 `null`，所以每个字段都可能是 `null`。
 */
export interface AnalysisLoudness {
  /** 积分响度（LUFS）。 */
  integrated_lufs: number | null;
  /** 短时（3 秒窗）响度最大值（LUFS）。 */
  short_term_max_lufs: number | null;
  /** 响度范围（LU）。 */
  loudness_range_lu: number | null;
  /** 采样峰值（dBFS）。 */
  sample_peak_dbfs: number | null;
  /** 真峰值（dBTP）。 */
  true_peak_dbtp: number | null;
}

/** 平均幅度谱。 */
export interface AnalysisSpectrum {
  /** 每个 bin 的带宽（Hz）。 */
  bin_hz: number;
  /** 各 bin 的平均幅度（dB）；静音 bin 为 `null`。 */
  magnitudes_db: (number | null)[];
}

/** 一次工程分析的完整结果（与后端 `analysis::Report` 一一对应）。 */
export interface AnalysisReport {
  loudness: AnalysisLoudness;
  /** 全曲 RMS（dBFS）；静音为 `null`。 */
  rms_dbfs: number | null;
  spectrum: AnalysisSpectrum;
}

/** MCP 服务器配置（与后端 `mcp::McpConfig` 一一对应）。 */
export interface McpConfig {
  /** 是否运行（随应用启动，也可在设置里即时开关）。 */
  enabled: boolean;
  /** 监听端口；`0` = 由系统分配临时端口。 */
  port: number;
}

/** MCP 服务器的配置 + 运行状态（与后端 `mcp::McpStatus` 一一对应）。 */
export interface McpStatus extends McpConfig {
  /** 是否正在监听。 */
  running: boolean;
  /** 正在监听时的接入地址（形如 `http://127.0.0.1:8787/mcp`），否则为 `null`。 */
  url: string | null;
}

/** `aidaw` 命令的安装状态（与后端 `cli::CliState` 一一对应）。 */
export type CliState = "installed" | "missing" | "conflict" | "system" | "unsupported";

/** 无法自动挂载时的原因（与后端 `cli::CliHint` 一一对应）。 */
export type CliHint = "dev_build" | "appimage" | "needs_manual";

/** `aidaw` 命令行工具的挂载状态（与后端 `cli::CliStatus` 一一对应）。 */
export interface CliStatus {
  state: CliState;
  /** 命令名（`aidaw`）。 */
  command: string;
  /** 已挂载时：软链（macOS）或 PATH 条目（Windows）指向的位置。 */
  target: string | null;
  /** 包内 `aidaw` 的位置；`null` = 这个构建里没有 sidecar。 */
  bundled: string | null;
  /** 冲突时：现有那份 `aidaw` 在哪。 */
  conflict: string | null;
  /** 挂不了时的原因。 */
  hint: CliHint | null;
  /** 手工安装命令（可直接复制到终端）。 */
  manual_command: string | null;
  /** 不挂也能用：包内二进制的**全路径**，可直接调用。 */
  full_path: string | null;
}

/** 一个音频输入设备（与后端 `recorder::InputDeviceInfo` 一一对应）。 */
export interface InputDeviceInfo {
  /** 设备显示名。 */
  name: string;
  /** 默认输入采样率（Hz）。 */
  sample_rate: number;
  /** 默认输入声道数。 */
  channels: number;
  /** 连接类型（仅 macOS 尽力提供）：见 `InputTransport`；取不到为 `null`。 */
  transport: InputTransport | null;
  /** 是否系统默认输入设备。 */
  is_default: boolean;
}

/** 输入设备的连接类型（后端的稳定短名）。 */
export type InputTransport =
  | "builtin"
  | "bluetooth"
  | "usb"
  | "virtual"
  | "aggregate"
  | "airplay"
  | "network"
  | "hdmi"
  | "displayport"
  | "thunderbolt"
  | "firewire"
  | "pci"
  | "other";

/** 本地 AI 任务状态（与后端 `ai::AiStatus` 一一对应）。 */
export interface AiStatus {
  /** 是否有任务在跑。 */
  running: boolean;
  /** 任务类型：`"to_midi"` / `"enhance"`。 */
  kind: string | null;
  /** 进度（0..1）。 */
  progress: number;
  /** 结束后的摘要或错误。 */
  message: string | null;
}

/** 本地 AI 的 ONNX 权重是否已就位。 */
export interface AiModels {
  /** Basic Pitch（音频转 MIDI）权重是否就位。 */
  basic_pitch: boolean;
  /** LocalVQE（人声降噪）权重是否就位。 */
  localvqe: boolean;
  /** HTDemucs（音源分离）权重是否就位。 */
  separate: boolean;
  /** Beat This!（节拍 / 速度检测）权重是否就位。 */
  beat_this: boolean;
}

/**
 * 一个模型清单条目 + 本地状态（与后端 `models::ModelStatus` 一一对应）。
 *
 * 用户到「管理器 → 模型」里查看 / 下载 / 删除；小模型随包（`bundled = true`）。
 */
export interface ModelStatus {
  /** 稳定标识（传给 `modelDownload` / `modelRemove`）。 */
  id: string;
  /** 展示名。 */
  name: string;
  /** 所属功能。 */
  feature: string;
  /** SPDX 许可。 */
  license: string;
  /** 预估体积（字节）。 */
  approx_bytes: number;
  /** 落地文件名。 */
  filename: string;
  /** 是否随安装包分发。 */
  bundled: boolean;
  /** 是否已就位。 */
  installed: boolean;
  /** 已就位时文件大小。 */
  bytes: number | null;
  /** 已就位时文件路径。 */
  path: string | null;
  /** 是否有可下载的源。 */
  downloadable: boolean;
  /** 组内文件总数（单文件模型为 1）。 */
  total_files: number;
  /** 组内已就位的文件数。 */
  installed_files: number;
  /** 组内每个文件的状态。 */
  files: ModelFileStatus[];
}

/**
 * 组内单个文件的状态（与后端 `models::ModelFileStatus` 一一对应）。
 *
 * 只有多文件模型才有实际意义：一个几 GB 的组光有整组百分比不够，
 * 还需要「下到第几个文件了」。
 */
export interface ModelFileStatus {
  /** 文件名。 */
  filename: string;
  /** 预估体积（字节）。 */
  approx_bytes: number;
  /** 是否已在位。 */
  installed: boolean;
  /** 已在位时的实际大小。 */
  bytes: number | null;
}

/**
 * 一次模型下载的进度（与后端 `models::ModelProgress` 一一对应）。
 *
 * 由 `engine://model` 事件推送。
 */
export interface ModelProgress {
  /** 模型 id。 */
  id: string;
  /** 状态：`downloading` / `verifying` / `done` / `error`。 */
  state: "downloading" | "verifying" | "done" | "error";
  /** 已接收字节。 */
  received: number;
  /** 总字节（0 = 未知）。 */
  total: number;
  /** 出错 / 取消时的一句话。 */
  message: string | null;
}

/**
 * 执行后端（Execution Provider）种类。三个都**随包、零下载**。
 *
 * - `cpu`：纯 CPU，全平台可用，**默认**；
 * - `coreml`：macOS Apple Silicon（Intel Mac 没有对应预编译构建）；
 * - `directml`：Windows。
 */
export type BackendKind = "cpu" | "coreml" | "directml";

/** 执行后端偏好（与后端 `manager::BackendPreference` 一一对应）。 */
export type BackendPreference = BackendKind;

/** 一个可选后端（与后端 `backends::BackendOption` 一一对应）。 */
export interface BackendOption {
  /** 稳定标识。 */
  kind: BackendKind;
  /** 展示名。 */
  label: string;
  /** 一句话说明。 */
  note: string;
  /** 是否本机平台支持（不可用的不会出现在设置页里）。 */
  available: boolean;
}

/** 后端选择 + 本机情况（与后端 `backends::BackendInfo` 一一对应）。 */
export interface BackendInfo {
  /** 已保存的偏好。 */
  preference: BackendPreference;
  /** 当前生效的后端（偏好在本机不支持时回落 CPU）。 */
  current: BackendKind;
  /** 本机平台描述（展示用）。 */
  platform: string;
  /** 全部选项（含本机不支持的）。 */
  options: BackendOption[];
}

/**
 * 母带参数（与后端 `ai::MasterArgs` 一一对应）。
 *
 * 三项都可省略，省略项用后端（`crates/mastering`）的默认值：-14 LUFS / -1 dBTP / 宽度 1.0。
 * 与 AI 任务不同，母带是**纯 DSP**，不需要任何模型权重。
 */
export interface MasterArgs {
  /** 目标积分响度（LUFS，负数）。 */
  target_lufs?: number;
  /** 真峰值上限（dBTP，负数或 0）。 */
  ceiling_dbtp?: number;
  /** 立体声宽度：0 = 单声道，1 = 原样，>1 加宽。 */
  stereo_width?: number;
}

// ---------------------------------------------------------------------------
// 插件管理器（E1）
// ---------------------------------------------------------------------------

/** 注册表里的一个可下载文件（与后端 `manager::RegistryFile` 一一对应）。 */
export interface ManagerFile {
  url: string;
  sha256: string;
  size: number;
  /** `archive`（可一键安装）或 `installer`（只能打开下载页，绝不自动执行）。 */
  kind: string;
  /** 归档里包含的插件格式（只留 `vst3` / `clap`）。 */
  contains: string[];
  /** 支持的系统（`mac` / `win` / `linux`）。 */
  platforms: string[];
  /** 支持的架构（`arm64` / `x64` / …）。 */
  archs: string[];
  /** 与当前系统 + 架构匹配，可直接安装。 */
  compatible: boolean;
}

/** 注册表里的一个插件（与后端 `manager::RegistryPlugin` 一一对应）。 */
export interface ManagerPlugin {
  slug: string;
  name: string;
  author: string;
  description: string;
  license: string;
  /** `instrument` / `effect` / … */
  kind: string;
  tags: string[];
  image: string | null;
  homepage: string | null;
  version: string;
  verified: boolean;
  files: ManagerFile[];
}

/** 一次注册表查询的结果（与后端 `manager::RegistryIndex` 一一对应）。 */
export interface ManagerRegistry {
  plugins: ManagerPlugin[];
  /** 当前系统（`mac` / `win` / `linux`）。 */
  platform: string;
  /** 当前架构（`arm64` / `x64` / …）。 */
  arch: string;
  /** 数据时间戳（Unix 秒）。 */
  fetched_at: number;
  /** 是否来自缓存（未联网拉取）。 */
  from_cache: boolean;
}

/** 插件管理器装过的一条记录（与后端 `manager::InstallRecord` 一一对应）。 */
export interface ManagerInstalled {
  /** 唯一键：注册表安装 = slug；本地导入 = `local:<文件名>`。 */
  key: string;
  /** 注册表 slug；本地导入为 `null`。 */
  slug: string | null;
  name: string;
  version: string;
  /** `registry` 或 `local`。 */
  source: string;
  /** 装进去的文件 / 目录的绝对路径。 */
  files: string[];
  installed_at: number;
}

/** 从注册表安装的请求（与后端 `manager::InstallRequest` 一一对应）。 */
export interface ManagerInstallRequest {
  slug: string;
  name: string;
  version: string;
  url: string;
  sha256: string;
  size: number;
}

/** 代理取用方式（与后端 `manager::ProxyMode` 一一对应）。 */
export type ProxyMode = "direct" | "system" | "manual";

/**
 * 模型下载源的区域偏好（与后端 `manager::ModelSourcePreference` 一一对应）。
 *
 * `auto` = 下载前探测连通性后决定（默认）；`international` = 优先 HF / GitHub；`domestic` = 优先魔搭。
 */
export type ModelSourcePreference = "auto" | "international" | "domestic";

/** 网络配置（与后端 `manager::StoreNetwork` 一一对应）。 */
export interface StoreNetwork {
  /** 注册表镜像（按顺序在官方源之前尝试）；空数组 = 只用官方源。 */
  registry_mirrors: string[];
  /** 下载地址重写前缀（只重写 GitHub 地址）；空串 = 不改写。可含 `{url}` 占位符。
   *
   * 注意：这是**地址重写**，不是 HTTP 代理——真正走代理要用 `proxy_mode`。 */
  download_prefix: string;
  /** 代理取用方式；`system` = 跟随系统代理（默认）。 */
  proxy_mode: ProxyMode;
  /** 手动模式的代理地址（仅 `manual` 用）。 */
  proxy_url: string;
  /** **模型权重**的镜像 / 前置代理前缀（与插件共用同一份代理设置）；空数组 = 只用官方源。
   *
   * 每条是前缀模板：含 `{url}` 就替换，否则直接拼在原地址前。始终校验 sha256。 */
  model_mirrors: string[];
  /** 模型下载源的区域偏好；`auto` = 探测连通性后决定（默认）。 */
  model_source_preference: ModelSourcePreference;
  /** **执行后端**偏好；`auto` = 按本机硬件推荐（默认）。 */
  backend_preference: BackendPreference;
}

/** 当前配置会解析成什么代理（不联网）。 */
export interface ProxyStatus {
  /** 解析出的代理地址（不含账号密码）；`null` = 直连。 */
  proxy: string | null;
  /** 来源说明，例如「系统代理（操作系统设置）http://127.0.0.1:7892」。 */
  source: string;
}

/** 一次代理探测的结果（会真的发一次请求）。 */
export interface ProxyProbe extends ProxyStatus {
  /** 是否真的连上了。 */
  ok: boolean;
  /** 成功是耗时，失败是原因。 */
  detail: string;
}

// ---------------------------------------------------------------------------
// 写歌智能体（与后端 `src-tauri/src/agent/` 一一对应）
// ---------------------------------------------------------------------------

/** OpenAI 兼容端点配置（与后端 `agent::AgentConfig` 一一对应）。 */
export interface AgentConfig {
  /** 形如 `https://api.openai.com/v1`。 */
  base_url: string;
  /** API Key。 */
  api_key: string;
  /** 模型名。 */
  model: string;
}

/** 连通性探测结果（与后端 `agent::openai::AgentProbe` 一一对应）。 */
export interface AgentProbe {
  ok: boolean;
  /** 成功=延迟说明，失败=原因。 */
  detail: string;
  /** 耗时（毫秒）。 */
  ms: number;
}

/** 助手发起的一次工具调用。 */
export interface AgentToolCallRef {
  id: string;
  name: string;
  arguments: string;
}

/** 会话树上的一个节点（与后端 `agent::store::Node` 一一对应）。 */
export interface AgentNode {
  id: string;
  parent: string | null;
  role: "user" | "assistant" | "tool";
  content: string;
  reasoning: string | null;
  tool_calls: AgentToolCallRef[];
  tool_call_id: string | null;
  name: string | null;
  created_at: number;
  error: string | null;
}

/** 一个完整会话（与后端 `agent::store::Session` 一一对应）。 */
export interface AgentSession {
  id: string;
  title: string;
  created_at: number;
  updated_at: number;
  nodes: AgentNode[];
  head: string | null;
}

/** 历史列表的一项（不含节点）。 */
export interface AgentSessionSummary {
  id: string;
  title: string;
  updated_at: number;
  count: number;
}

/** `engine://agent` 推来的事件（与后端 `agent::AgentEvent` 一一对应，按 `type` 区分）。 */
export type AgentEvent =
  | { type: "turn_started"; session_id: string; user_node: AgentNode }
  | { type: "delta"; node_id: string; text: string }
  | { type: "reasoning"; node_id: string; text: string }
  | { type: "tool_call"; node_id: string; call_id: string; name: string; args: unknown }
  | { type: "tool_result"; node_id: string; call_id: string; ok: boolean; output: string }
  | { type: "node_added"; node: AgentNode }
  | { type: "turn_finished"; session_id: string; head: string; title: string; stopped: boolean }
  | { type: "error"; message: string };

/**
 * 用户**自己安装**的插件（与后端 `manager::ForeignPlugin` 一一对应）。
 *
 * 从系统标准目录扫描到的 VST3 / CLAP，由其他安装器管理——本管理器只读，不提供删除。
 */
export interface ForeignPlugin {
  /** 全局唯一 id（`vst3:<uid>` / `clap:<id>`）。 */
  id: string;
  name: string;
  vendor: string;
  kind: "instrument" | "effect";
  /** 格式标识（`vst3` / `clap`）。 */
  format: string;
  /** 插件所在路径（目录或文件）。 */
  path: string;
}

/**
 * 一次插件安装的进度（与后端 `manager::ManagerProgress` 一一对应）。
 *
 * 由 `engine://manager` 事件推送；前端据此在卡片**下边缘**画进度条。
 */
export interface ManagerProgress {
  /** 正在处理的注册表 slug。 */
  slug: string;
  /** 阶段：`downloading` / `extracting` / `installing` / `error`。 */
  phase: "downloading" | "extracting" | "installing" | "error";
  /** 已接收字节（仅 downloading 有意义）。 */
  received: number;
  /** 总字节（0 = 未知）。 */
  total: number;
  /** 失败原因（仅 error 有意义）。 */
  message: string | null;
}

/**
 * 一次音频导出的进度（与后端 `commands::ExportProgress` 一一对应）。
 *
 * 由 `engine://export` 事件推送；前端据此在**顶栏下边缘**画进度条（与导入共用同一条）。
 */
export interface ExportProgress {
  /** 目标文件路径。 */
  path: string;
  /** 阶段：`preparing`（建插件实例 / 等外部插件装载）→ `rendering` → `encoding` → `done` / `error`。 */
  phase: "preparing" | "rendering" | "encoding" | "done" | "error";
  /** 本阶段已完成的工作量（渲染阶段 = 已渲染帧数）。 */
  frames: number;
  /** 本阶段的总量；未知时 `null`（画不确定态）。 */
  total_frames: number | null;
  /** 失败原因（只有 `error` 阶段带）。 */
  message: string | null;
}

/**
 * 一次音频导入的解码进度（与后端 `commands::ImportProgress` 一一对应）。
 *
 * 由 `engine://import` 事件推送；前端据此在**顶栏下边缘**画进度条。
 */
export interface ImportProgress {
  /** 正在解码的素材路径。 */
  path: string;
  /** 阶段：`decoding` / `done` / `error`。 */
  state: "decoding" | "done" | "error";
  /** 已解出的帧数（decoding 时有意义）。 */
  frames: number;
  /** 总帧数；容器头预知不到时为 `null`（界面据此画不确定态）。 */
  total_frames: number | null;
  /** 失败原因（仅 error 有意义）。 */
  message: string | null;
}
