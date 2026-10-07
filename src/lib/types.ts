/** 与引擎 `model` 对应的前端视图类型。 */
export interface Note { pitch: number; start: number; duration: number; velocity: number }
export interface Clip { id: number; start: number; length: number; notes: Note[] }
export interface MixerChannel { gain_db: number; pan: number; mute: boolean; solo: boolean }
export interface PluginInstance { plugin_id: string; params: Record<string, number>; state: number[] }
export interface Track {
  id: number;
  name: string;
  instrument: PluginInstance | null;
  clips: Clip[];
  mixer: MixerChannel;
}
export interface Transport { tempo: number; time_sig_num: number; time_sig_den: number }
export interface Project { name: string; transport: Transport; tracks: Track[]; master: MixerChannel }
