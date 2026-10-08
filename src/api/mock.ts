/**
 * 开发用假数据：只在「浏览器里且非 Tauri」时启用，用于在没有后端的情况下预览界面。
 * 生产构建（`vite build`）里 `import.meta.env.DEV` 为 false，本模块不会被用到。
 */
import type { Note, Project } from "./types";

/** 是否使用假数据。 */
export function mockEnabled(): boolean {
  return import.meta.env.DEV && !("__TAURI_INTERNALS__" in window);
}

function triad(root: number, beat: number): Note[] {
  return [0, 4, 7].map((i) => ({
    pitch: root + i,
    start: beat,
    duration: 1.6,
    velocity: 88,
  }));
}

function melody(): Note[] {
  const pitches = [72, 74, 76, 79, 76, 74, 72, 67];
  return pitches.map((pitch, i) => ({
    pitch,
    start: i,
    duration: 0.7,
    velocity: 104,
  }));
}

function bass(): Note[] {
  const pitches = [36, 36, 41, 41, 43, 43, 36, 36];
  return pitches.map((pitch, i) => ({
    pitch,
    start: i,
    duration: 0.9,
    velocity: 96,
  }));
}

function sample(): Project {
  return {
    name: "示例工程",
    transport: { tempo: 110, time_sig_num: 4, time_sig_den: 4 },
    master: { gain_db: 0, pan: 0, mute: false, solo: false },
    tracks: [
      {
        id: 1,
        name: "和弦",
        instrument: { plugin_id: "builtin.synth", params: { gain: 0.6 }, state: [] },
        clips: [{ id: 1, start: 0, length: 8, notes: [...triad(48, 0), ...triad(53, 2), ...triad(55, 4), ...triad(48, 6)] }],
        mixer: { gain_db: -3, pan: 0, mute: false, solo: false },
      },
      {
        id: 2,
        name: "旋律",
        instrument: { plugin_id: "builtin.synth", params: { gain: 0.8 }, state: [] },
        clips: [{ id: 2, start: 0, length: 8, notes: melody() }],
        mixer: { gain_db: -1.5, pan: 0.15, mute: false, solo: false },
      },
      {
        id: 3,
        name: "低音",
        instrument: { plugin_id: "builtin.synth", params: { gain: 0.7 }, state: [] },
        clips: [{ id: 3, start: 0, length: 8, notes: bass() }],
        mixer: { gain_db: -4, pan: -0.1, mute: false, solo: false },
      },
    ],
  };
}

let state: Project = sample();
let nextTrackId = 4;
let nextClipId = 4;

function clone(p: Project): Project {
  return structuredClone(p);
}

/** 假数据版的后端 API。 */
export const mockApi = {
  getProject: (): Project => clone(state),
  addTrack: (name: string): Project => {
    state.tracks.push({
      id: nextTrackId++,
      name,
      instrument: { plugin_id: "builtin.synth", params: {}, state: [] },
      clips: [{ id: nextClipId++, start: 0, length: 4, notes: [] }],
      mixer: { gain_db: 0, pan: 0, mute: false, solo: false },
    });
    return clone(state);
  },
  setTempo: (bpm: number): Project => {
    state.transport.tempo = bpm;
    return clone(state);
  },
  clearProject: (): Project => {
    state = { name: "untitled", transport: { tempo: 120, time_sig_num: 4, time_sig_den: 4 }, tracks: [], master: { gain_db: 0, pan: 0, mute: false, solo: false } };
    return clone(state);
  },
  loadDemo: (): Project => {
    state = sample();
    return clone(state);
  },
  renderPreview: (): Blob => new Blob([new Uint8Array(0)], { type: "audio/wav" }),
  exportWav: (path: string): string => path,
  onEngineEvent: (): (() => void) => {
    const t = window.setInterval(() => {
      // 模拟引擎事件，便于预览开发者抽屉与 AI 胶囊。
      window.dispatchEvent(new CustomEvent("daw-party-mock-event"));
    }, 4000);
    return () => window.clearInterval(t);
  },
};
