/** 与引擎 `daw_party_engine::model` 对应的前端视图类型（snake_case，id 为裸数字）。 */

export interface Note {
  pitch: number;
  start: number;
  duration: number;
  /** 演奏强度（0..=127）：手弹得多重。内置合成器把它折算成振幅。 */
  velocity: number;
  /** 单音音量（0..=127，127 = 不衰减）：纯增益微调，不改变音色。 */
  volume: number;
}

export interface Clip {
  id: number;
  start: number;
  length: number;
  notes: Note[];
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
}

/** 撤销 / 重做之后的返回：工程快照 + 最新历史深度。 */
export interface HistoryState {
  project: Project;
  undo_depth: number;
  redo_depth: number;
}

/** 当前可撤销 / 可重做的次数。 */
export interface HistoryDepths {
  undo: number;
  redo: number;
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
