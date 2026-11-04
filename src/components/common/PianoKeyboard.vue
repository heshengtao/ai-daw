<script setup lang="ts">
/**
 * 可弹的钢琴键盘（合成器界面 / 下窗口虚拟 MIDI 键盘共用）。
 *
 * 传了 `track` 时按下即**试听**：走实时通道直接送给轨道的乐器，不写工程、不进撤销历史、
 * 不等传输（传输停着也能弹）。`track = null` 时只发 `noteOn` / `noteOff` 事件，由宿主
 * 统一走 MIDI store（下窗口要同时处理硬件输入与录音）。
 *
 * 除了鼠标，也认电脑键盘：`A W S E D F T G Y H U J K` 对应 C..C（和大多数 DAW 一样）；
 * 宿主可以把 `computerKeyboard` 关掉，避免两个键盘抢同一批字母键。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { auditionNote } from "@/api/tauri";
import { shouldDeferToBrowser } from "@/utils/editable";
import { useI18n } from "@/i18n";

const i18n = useI18n();

const props = withDefaults(
  defineProps<{
    /** 试听目标轨道；`null` = 不自己发声，只发事件（由宿主统一走 MIDI store）。 */
    track?: number | null;
    /** 起始音高（默认 C4 = 60）。 */
    baseNote?: number;
    /** 几个八度。 */
    octaves?: number;
    /** 力度（0..=127）。 */
    velocity?: number;
    /** 外部按住的音高（硬件 MIDI / 录音侧高亮），只影响显示。 */
    heldNotes?: number[];
    /** 是否把电脑键盘当钢琴弹（宿主可关掉）。 */
    computerKeyboard?: boolean;
    /** 在白键底部标出 C 音名（虚拟 MIDI 键盘用；合成器面板不标）。 */
    showLabels?: boolean;
  }>(),
  {
    track: null,
    baseNote: 60,
    octaves: 2,
    velocity: 100,
    heldNotes: () => [],
    computerKeyboard: true,
    showLabels: false,
  },
);

const emit = defineEmits<{
  (e: "noteOn", note: number, velocity: number, source: "mouse" | "key"): void;
  (e: "noteOff", note: number, source: "mouse" | "key"): void;
}>();

/** 电脑键盘 → 相对起始音的偏移（两个八度）。 */
const KEY_MAP: Record<string, number> = {
  a: 0, w: 1, s: 2, e: 3, d: 4, f: 5, t: 6, g: 7, y: 8, h: 9, u: 10, j: 11,
  k: 12, o: 13, l: 14, p: 15, ";": 16, "'": 17,
};

const BLACK_OFFSETS = new Set([1, 3, 6, 8, 10]);

const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/** MIDI 音高 → 音名（中央 C = C4 = 60）。 */
function noteName(note: number): string {
  return `${NOTE_NAMES[note % 12]}${Math.floor(note / 12) - 1}`;
}

/** 正被按住的音高 → 来源（鼠标 / 键盘），用来正确处理「同时按住」。 */
const held = ref<Map<number, "mouse" | "key">>(new Map());

interface Key {
  note: number;
  black: boolean;
  /** 白键序号（黑键用它定位）。 */
  whiteIndex: number;
}

const keys = computed<Key[]>(() => {
  const total = props.octaves * 12 + 1;
  const out: Key[] = [];
  let whiteIndex = 0;
  for (let i = 0; i < total; i += 1) {
    const note = props.baseNote + i;
    const black = BLACK_OFFSETS.has(i % 12);
    out.push({ note, black, whiteIndex: black ? whiteIndex - 1 : whiteIndex });
    if (!black) whiteIndex += 1;
  }
  return out;
});

const whiteKeys = computed(() => keys.value.filter((k) => !k.black));
const blackKeys = computed(() => keys.value.filter((k) => k.black));

/** 黑键宽度：白键宽度的 62%（白键宽度 = 100% / 白键数）。 */
const blackWidthPct = computed(() => (100 / Math.max(1, whiteKeys.value.length)) * 0.62);

/** 黑键压在两白键的交线上：左移半个黑键宽。 */
function blackStyle(k: Key): Record<string, string> {
  const whites = Math.max(1, whiteKeys.value.length);
  const left = ((k.whiteIndex + 1) / whites) * 100 - blackWidthPct.value / 2;
  return { left: `${left}%`, width: `${blackWidthPct.value}%` };
}

function isHeld(note: number): boolean {
  return held.value.has(note) || props.heldNotes.includes(note);
}

function press(note: number, source: "mouse" | "key"): void {
  if (held.value.has(note)) return;
  held.value.set(note, source);
  if (props.track !== null) void auditionNote(props.track, note, props.velocity, true);
  emit("noteOn", note, props.velocity, source);
}

function release(note: number, source?: "mouse" | "key"): void {
  const heldBy = held.value.get(note);
  if (!heldBy) return;
  // 同一个音可能先被鼠标按住、随后又按了电脑键盘：keyup / pointerup 只该松开自己那一路。
  if (source !== undefined && heldBy !== source) return;
  held.value.delete(note);
  if (props.track !== null) void auditionNote(props.track, note, 0, false);
  emit("noteOff", note, heldBy);
}

/** 鼠标按下后拖过别的键也要跟着响（像真的滑过键盘）。 */
const dragging = ref(false);

function onDown(note: number, e: PointerEvent): void {
  dragging.value = true;
  press(note, "mouse");
  (e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
}

function onEnter(note: number): void {
  if (dragging.value) press(note, "mouse");
}

function onUp(note: number): void {
  release(note, "mouse");
}

function onDocPointerUp(): void {
  if (!dragging.value) return;
  dragging.value = false;
  for (const [note, source] of [...held.value]) {
    if (source === "mouse") release(note);
  }
}

function onKeyDown(e: KeyboardEvent): void {
  if (!props.computerKeyboard) return;
  if (e.repeat || e.metaKey || e.ctrlKey || e.altKey) return;
  // 输入框 / 文本域 / 可编辑区里的按键归浏览器；否则在检查器里改参数会顺手弹出音符。
  if (shouldDeferToBrowser(e)) return;
  const offset = KEY_MAP[e.key.toLowerCase()];
  if (offset === undefined) return;
  e.preventDefault();
  press(props.baseNote + offset, "key");
}

function onKeyUp(e: KeyboardEvent): void {
  if (!props.computerKeyboard) return;
  const offset = KEY_MAP[e.key.toLowerCase()];
  if (offset === undefined) return;
  release(props.baseNote + offset, "key");
}

/** 失焦时把所有音松开，免得留下挂住的音。 */
function releaseAll(): void {
  for (const note of [...held.value.keys()]) release(note);
}

/** 只松开某个来源按住的音（关掉电脑键盘开关时用）。 */
function releaseBySource(source: "mouse" | "key"): void {
  for (const [note, heldBy] of [...held.value]) {
    if (heldBy === source) release(note, source);
  }
}

onMounted(() => {
  window.addEventListener("pointerup", onDocPointerUp);
  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("keyup", onKeyUp);
  window.addEventListener("blur", releaseAll);
});

/**
 * 换了起始音高（八度按钮）就把按住的音全松开。
 *
 * 按住的音高是**绝对 MIDI 号**，键位一换，那些号已经不在键盘上了——不放会留下挂住的音。
 */
watch(
  () => props.baseNote,
  () => releaseAll(),
);
watch(
  () => props.computerKeyboard,
  (on) => {
    if (!on) releaseBySource("key");
  },
);
onUnmounted(() => {
  window.removeEventListener("pointerup", onDocPointerUp);
  window.removeEventListener("keydown", onKeyDown);
  window.removeEventListener("keyup", onKeyUp);
  window.removeEventListener("blur", releaseAll);
  releaseAll();
});
</script>

<template>
  <div class="keyboard" role="group" :aria-label="i18n.t('keyboard.group')">
    <div class="whites">
      <button
        v-for="k in whiteKeys"
        :key="k.note"
        class="white"
        :class="{ on: isHeld(k.note) }"
        :aria-label="i18n.t('keyboard.audition', { note: k.note })"
        @pointerdown="onDown(k.note, $event)"
        @pointerenter="onEnter(k.note)"
        @pointerup="onUp(k.note)"
        @pointerleave="onUp(k.note)"
      >
        <span v-if="showLabels && k.note % 12 === 0" class="key-label">{{ noteName(k.note) }}</span>
      </button>
    </div>
    <div class="blacks">
      <button
        v-for="k in blackKeys"
        :key="k.note"
        class="black"
        :class="{ on: isHeld(k.note) }"
        :style="blackStyle(k)"
        :aria-label="i18n.t('keyboard.audition', { note: k.note })"
        @pointerdown="onDown(k.note, $event)"
        @pointerenter="onEnter(k.note)"
        @pointerup="onUp(k.note)"
        @pointerleave="onUp(k.note)"
      />
    </div>
  </div>
</template>

<style scoped>
/* 键盘**自带没有托盘**：不铺底色、不留内边距，直接贴在宿主的下边缘上
   （合成器面板把它贴到设备底边，由设备的大圆角切出形状）。 */
.keyboard {
  position: relative;
  height: 100%;
  min-height: 84px;
  user-select: none;
  touch-action: none;
}
.whites {
  display: flex;
  gap: 1px;
  height: 100%;
}
.white {
  position: relative;
  flex: 1 1 0;
  min-width: 0;
  border: 0;
  border-radius: 2px 2px 4px 4px;
  background-color: rgb(var(--md-surface-container-highest));
  cursor: pointer;
  transition: background-color 0.06s ease;
}
.white:hover {
  background-color: rgb(var(--md-surface-container-high));
}
.white.on {
  background-color: rgb(var(--md-primary));
}
/* C 音名：只标 C，既不挡视线又能一眼看出每个八度的分界。 */
.key-label {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 3px;
  text-align: center;
  font-size: 9px;
  line-height: 1;
  letter-spacing: 0.2px;
  color: rgb(var(--md-on-surface-muted));
  pointer-events: none;
  user-select: none;
}
.white.on .key-label {
  color: rgb(var(--md-on-primary));
}
.blacks {
  position: absolute;
  inset: 0 0 auto 0;
  height: 62%;
  pointer-events: none;
}
.black {
  position: absolute;
  top: 0;
  height: 100%;
  border: 0;
  border-radius: 0 0 4px 4px;
  background-color: rgb(var(--md-on-surface) / 0.88);
  cursor: pointer;
  pointer-events: auto;
  transition: background-color 0.06s ease;
}
.black:hover {
  background-color: rgb(var(--md-on-surface));
}
.black.on {
  background-color: rgb(var(--md-primary));
}
</style>
