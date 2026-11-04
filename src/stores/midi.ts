/**
 * MIDI 输入状态：设备选择、按住的音符、试听与录音入口。
 *
 * 三个来源共用同一条路径：下窗口的虚拟键盘（鼠标）、硬件 MIDI 端口、电脑键盘模拟。
 * 它们都调用 `noteOn` / `noteOff`，因此：
 * - 高亮只认「这个音高有没有被任何一个来源按住」；
 * - 试听走既有的 `audition_note`（不改工程、不进撤销历史）；
 * - 录音时同时写进 `transport` 的 MIDI take，停止时落成一条片段。
 *
 * 设备选择记在 localStorage：下次启动会自动重新打开它；设备不在了就静默保持未连接，
 * 不打扰用户。
 */
import { defineStore } from "pinia";
import { computed, ref } from "vue";
import * as api from "@/api/tauri";
import type { MidiInputDeviceInfo, MidiInputEvent } from "@/api/types";
import { useI18n } from "@/i18n";
import { useLayoutStore } from "@/stores/layout";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { useTransportStore } from "@/stores/transport";

/** 音符来源：鼠标点虚拟键盘 / 硬件 MIDI / 电脑键盘模拟。 */
export type NoteSource = "mouse" | "hw" | "computer";

const KEY_DEVICE = "ai-daw-midi-device";
const KEY_VELOCITY = "ai-daw-midi-velocity";
const KEY_OCTAVE = "ai-daw-midi-octave";
const KEY_COMPUTER = "ai-daw-midi-computer";

/** 虚拟键盘的起始音高（C3）与八度范围：默认三个八度，正好覆盖写和弦最常用的中音区。 */
const BASE_NOTE = 48;
const MIN_OCTAVE = -2;
const MAX_OCTAVE = 3;

function readString(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function readNumber(key: string, fallback: number): number {
  try {
    const raw = localStorage.getItem(key);
    if (raw === null) return fallback;
    const n = Number(raw);
    return Number.isFinite(n) ? n : fallback;
  } catch {
    return fallback;
  }
}

function write(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // 存不住只影响下次启动的默认值。
  }
}

export const useMidiStore = defineStore("midi", () => {
  const transport = useTransportStore();
  const selection = useSelectionStore();
  const project = useProjectStore();
  const layout = useLayoutStore();
  const i18n = useI18n();

  /** 本机可用的 MIDI 输入端口。 */
  const inputs = ref<MidiInputDeviceInfo[]>([]);
  /** 当前选中的端口名（`null` = 未连接）。空字符串表示用户显式关掉过，也归一成 `null`。 */
  const input = ref<string | null>(readString(KEY_DEVICE) || null);
  /** 电脑键盘是否模拟 MIDI 输入（默认关：打开下窗口本身不等于「我要用字母键弹琴」）。 */
  const computer = ref(readString(KEY_COMPUTER) === "on");
  /** 虚拟键盘的力度（1..127）。 */
  const velocity = ref(Math.min(127, Math.max(1, readNumber(KEY_VELOCITY, 100))));
  /** 八度位移（相对 C3）。 */
  const octaveShift = ref(
    Math.min(MAX_OCTAVE, Math.max(MIN_OCTAVE, Math.round(readNumber(KEY_OCTAVE, 0)))),
  );

  /** 虚拟键盘的起始音高。 */
  const baseNote = computed(() => BASE_NOTE + octaveShift.value * 12);
  /** 起始音高的音名（如 `C3`）：表头只显示它，完整音域放 tooltip。 */
  const baseLabel = computed(() => {
    const names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    return `${names[baseNote.value % 12]}${Math.floor(baseNote.value / 12) - 1}`;
  });
  /** 虚拟键盘覆盖的音域文案。 */
  const rangeLabel = computed(() => {
    const name = (p: number): string => {
      const names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
      return `${names[p % 12]}${Math.floor(p / 12) - 1}`;
    };
    return `${name(baseNote.value)} – ${name(baseNote.value + 36)}`;
  });

  /**
   * 正在发声的音高 → 按住它的来源集合。
   *
   * 用集合而不是布尔：同一个音可能同时被鼠标和电脑键盘按着，先松一个不该把声音掐掉。
   */
  const heldSources = new Map<number, Set<NoteSource>>();
  /** 正在发声的音高（响应式，供键盘高亮）。 */
  const heldNotes = ref<number[]>([]);
  /** 每个音第一次按下时用的轨道：松手仍送回那条轨道，避免中途换轨留下挂音。 */
  const heldTrack = new Map<number, number>();

  let unlisten: (() => void) | null = null;
  let started = false;

  function syncHeld(): void {
    heldNotes.value = [...heldSources.keys()].sort((a, b) => a - b);
  }

  /** 当前该试听的轨道：选中轨道，否则第一条。 */
  function auditionTrack(): number | null {
    return selection.track?.id ?? project.project?.tracks[0]?.id ?? null;
  }

  /**
   * 按下一个音（三个来源共用）。
   *
   * 只有「从无到有」时才开始发声 / 记录；重复来源只加计数。
   */
  function noteOn(note: number, vel: number, source: NoteSource, channel = 0): void {
    const tone = Math.min(127, Math.max(0, Math.round(note)));
    let sources = heldSources.get(tone);
    if (sources?.has(source)) return;
    if (!sources) {
      sources = new Set();
      heldSources.set(tone, sources);
      const track = auditionTrack();
      if (track !== null) {
        heldTrack.set(tone, track);
        void api.auditionNote(track, tone, Math.min(127, Math.max(1, Math.round(vel))), true);
      }
      transport.noteInput(tone, vel, true, channel);
      syncHeld();
    }
    sources.add(source);
  }

  /** 松开一个音（只在该来源是最后一个按住者时真正停止）。 */
  function noteOff(note: number, source: NoteSource): void {
    const tone = Math.min(127, Math.max(0, Math.round(note)));
    const sources = heldSources.get(tone);
    if (!sources || !sources.has(source)) return;
    sources.delete(source);
    if (sources.size > 0) return;
    heldSources.delete(tone);
    const track = heldTrack.get(tone) ?? auditionTrack();
    heldTrack.delete(tone);
    if (track !== null) void api.auditionNote(track, tone, 0, false);
    transport.noteInput(tone, 0, false);
    syncHeld();
  }

  /** 松开某个来源按住的全部音（关掉电脑键盘 / 换设备 / 窗口失焦时用）。 */
  function releaseSource(source: NoteSource): void {
    for (const [note, sources] of [...heldSources]) {
      if (sources.has(source)) noteOff(note, source);
    }
  }

  /** 松开所有音（panic）。 */
  function releaseAll(): void {
    for (const note of [...heldSources.keys()]) {
      for (const source of [...(heldSources.get(note) ?? [])]) noteOff(note, source);
    }
  }

  /** 拉取端口列表。 */
  async function refresh(): Promise<void> {
    try {
      inputs.value = await api.listMidiInputs();
    } catch {
      inputs.value = [];
    }
  }

  /**
   * 选择端口。`null` = 断开并释放。
   *
   * 失败（设备正被别的程序独占 / 已拔出）时落到「未连接」并提示：后端换设备是「先关旧连接、
   * 再开新连接」，失败后旧连接已经没了，界面不能继续显示旧端口。
   */
  async function select(device: string | null): Promise<void> {
    if (device === input.value) return;
    try {
      await api.setMidiInput(device);
      input.value = device;
      if (device === null) write(KEY_DEVICE, "");
      else write(KEY_DEVICE, device);
      releaseSource("hw");
    } catch (err) {
      // 后端已经先关掉了旧连接（换设备是一条「先关再开」的路径），所以失败后界面不能继续
      // 显示旧端口——那会变成「看着已连接、其实收不到消息」。落回未连接，让用户重选。
      input.value = null;
      write(KEY_DEVICE, "");
      layout.notify(i18n.t("midi.inputOpenFailed", { error: String(err) }));
    }
  }

  /** 启动时自动重开上次的设备（找不到就静默保持未连接）。 */
  async function openSaved(): Promise<void> {
    const saved = input.value;
    if (!saved) return;
    if (!inputs.value.some((d) => d.name === saved)) {
      input.value = null;
      return;
    }
    try {
      await api.setMidiInput(saved);
    } catch {
      input.value = null;
    }
  }

  /** 硬件事件：只接受当前端口来的消息，避免换设备瞬间的迟到事件。 */
  function onMidiEvent(event: MidiInputEvent): void {
    if (input.value !== null && event.device !== input.value) return;
    const message = event.message;
    if (message.kind === "note_on") {
      noteOn(message.note, message.velocity, "hw", message.channel);
    } else if (message.kind === "note_off") {
      noteOff(message.note, "hw");
    }
  }

  /** 订阅事件并加载端口（幂等）。 */
  async function init(): Promise<void> {
    if (started) return;
    started = true;
    try {
      unlisten = await api.onMidiEvent(onMidiEvent);
    } catch (err) {
      started = false;
      throw err;
    }
    window.addEventListener("blur", onWindowBlur);
    await refresh();
    // 后端可能已经开着某个端口（界面热重载）：以后端为准，避免重复打开。
    try {
      const current = await api.midiInputDevice();
      if (current) {
        // 端口还在：直接沿用后端已打开的那条连接（界面热重载时最常见）。
        if (inputs.value.some((d) => d.name === current)) {
          input.value = current;
          write(KEY_DEVICE, current);
          return;
        }
        // 端口已经不在了（拔掉 / 设备名变了）：把后端的死连接关掉，别让它占着一个不存在的名字。
        try {
          await api.setMidiInput(null);
        } catch {
          // 关闭失败也无所谓，进程退出时会释放。
        }
        input.value = null;
        write(KEY_DEVICE, "");
        return;
      }
    } catch {
      // 查询失败就按本地记录尝试重开。
    }
    await openSaved();
  }

  /** 退订并松开所有音。 */
  function dispose(): void {
    releaseAll();
    window.removeEventListener("blur", onWindowBlur);
    unlisten?.();
    unlisten = null;
    started = false;
  }

  /** 窗口失焦：把所有音松开，避免切走以后留下挂住的音（硬件 key-up 不会送到后台窗口）。 */
  function onWindowBlur(): void {
    releaseAll();
  }

  /** 切换电脑键盘模拟。 */
  function setComputer(on: boolean): void {
    if (computer.value === on) return;
    computer.value = on;
    write(KEY_COMPUTER, on ? "on" : "off");
    if (!on) releaseSource("computer");
  }

  /** 设置力度（夹在 1..127）。 */
  function setVelocity(value: number): void {
    const next = Math.min(127, Math.max(1, Math.round(value)));
    velocity.value = next;
    write(KEY_VELOCITY, String(next));
  }

  /** 移调八度；换键位前先松开电脑键盘按住的音，免得 keyup 落到新键位。 */
  function shiftOctave(delta: number): void {
    const next = Math.min(MAX_OCTAVE, Math.max(MIN_OCTAVE, octaveShift.value + delta));
    if (next === octaveShift.value) return;
    releaseSource("computer");
    octaveShift.value = next;
    write(KEY_OCTAVE, String(next));
  }

  return {
    inputs,
    input,
    computer,
    velocity,
    octaveShift,
    baseNote,
    baseLabel,
    rangeLabel,
    heldNotes,
    init,
    dispose,
    refresh,
    select,
    noteOn,
    noteOff,
    releaseSource,
    releaseAll,
    setComputer,
    setVelocity,
    shiftOctave,
  };
});
