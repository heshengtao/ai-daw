<script setup lang="ts">
/**
 * 可弹的钢琴键盘（合成器界面里的那个键盘）。
 *
 * 按下即**试听**：走实时通道直接送给轨道的乐器，不写工程、不进撤销历史、不等传输
 * （传输停着也能弹）。松开走 NoteOff，让插件按自己的释音收尾。
 *
 * 除了鼠标，也认电脑键盘：`A W S E D F T G Y H U J K` 对应 C..C（和大多数 DAW 一样）。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { auditionNote } from "@/api/tauri";
import { useI18n } from "@/i18n";

const i18n = useI18n();

const props = withDefaults(
  defineProps<{
    track: number;
    /** 起始音高（默认 C4 = 60）。 */
    baseNote?: number;
    /** 几个八度。 */
    octaves?: number;
    /** 力度（0..=127）。 */
    velocity?: number;
  }>(),
  { baseNote: 60, octaves: 2, velocity: 100 },
);

/** 电脑键盘 → 相对起始音的偏移（两个八度）。 */
const KEY_MAP: Record<string, number> = {
  a: 0, w: 1, s: 2, e: 3, d: 4, f: 5, t: 6, g: 7, y: 8, h: 9, u: 10, j: 11,
  k: 12, o: 13, l: 14, p: 15, ";": 16, "'": 17,
};

const BLACK_OFFSETS = new Set([1, 3, 6, 8, 10]);

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

function isHeld(note: number): boolean {
  return held.value.has(note);
}

function press(note: number, source: "mouse" | "key"): void {
  if (held.value.has(note)) return;
  held.value.set(note, source);
  void auditionNote(props.track, note, props.velocity, true);
}

function release(note: number): void {
  if (!held.value.has(note)) return;
  held.value.delete(note);
  void auditionNote(props.track, note, 0, false);
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
  release(note);
}

function onDocPointerUp(): void {
  if (!dragging.value) return;
  dragging.value = false;
  for (const [note, source] of [...held.value]) {
    if (source === "mouse") release(note);
  }
}

function onKeyDown(e: KeyboardEvent): void {
  if (e.repeat || e.metaKey || e.ctrlKey || e.altKey) return;
  const target = e.target as HTMLElement | null;
  if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA")) return;
  const offset = KEY_MAP[e.key.toLowerCase()];
  if (offset === undefined) return;
  e.preventDefault();
  press(props.baseNote + offset, "key");
}

function onKeyUp(e: KeyboardEvent): void {
  const offset = KEY_MAP[e.key.toLowerCase()];
  if (offset === undefined) return;
  release(props.baseNote + offset);
}

/** 失焦时把所有音松开，免得留下挂住的音。 */
function releaseAll(): void {
  for (const note of [...held.value.keys()]) release(note);
}

onMounted(() => {
  window.addEventListener("pointerup", onDocPointerUp);
  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("keyup", onKeyUp);
  window.addEventListener("blur", releaseAll);
});
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
      />
    </div>
    <div class="blacks">
      <button
        v-for="k in blackKeys"
        :key="k.note"
        class="black"
        :class="{ on: isHeld(k.note) }"
        :style="{ left: `calc(${((k.whiteIndex + 1) / whiteKeys.length) * 100}% - 1.4%)` }"
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
.keyboard {
  position: relative;
  height: 108px;
  padding: 8px 8px 10px;
  border-radius: 18px;
  background-color: var(--app-inset);
  user-select: none;
  touch-action: none;
}
.whites {
  display: flex;
  gap: 1px;
  height: 100%;
}
.white {
  flex: 1 1 0;
  min-width: 0;
  border: 0;
  border-radius: 3px 3px 5px 5px;
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
.blacks {
  position: absolute;
  inset: 8px 8px auto 8px;
  height: 62%;
  pointer-events: none;
}
.black {
  position: absolute;
  top: 0;
  width: 2.8%;
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
