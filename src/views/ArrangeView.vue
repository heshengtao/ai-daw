<script setup lang="ts">
/** 编排视图：网格铺满整宽，片段按轨道配色。 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useProjectStore } from "@/stores/project";
import { trackHue } from "@/utils/trackColor";

const store = useProjectStore();
const zoom = ref(56); // 每拍像素

const tracks = computed(() => store.project?.tracks ?? []);
const beatsPerBar = computed(() => store.project?.transport.time_sig_num ?? 4);

/** 内容长度（小节）。 */
const contentBars = computed(() => {
  const beats = tracks.value.flatMap((t) => t.clips).reduce((m, c) => Math.max(m, c.start + c.length), 0);
  return Math.max(4, Math.ceil(beats / beatsPerBar.value) + 2);
});

/** 标尺多画一段，保证拉伸或滚动时右侧仍有刻度与网格。 */
const totalBars = computed(() => contentBars.value + 24);

const beatsTotal = computed(() => totalBars.value * beatsPerBar.value);

/** 至少铺满可视宽度，避免右侧出现空白。 */
const widthStyle = computed(() => ({ width: `max(100%, ${beatsTotal.value * zoom.value}px)` }));
const beatPx = computed(() => `${zoom.value}px`);
const barPx = computed(() => `${zoom.value * beatsPerBar.value}px`);

function clipStyle(start: number, length: number, index: number): Record<string, string> {
  return {
    left: `${start * zoom.value}px`,
    width: `${Math.max(6, length * zoom.value)}px`,
    "--track-hue": String(trackHue(index)),
  };
}

// 播放头：离线预览没有实时位置，仅作视觉指示。
const playheadPct = ref(0);
let timer: number | null = null;
onMounted(() => {
  timer = window.setInterval(() => {
    playheadPct.value = (playheadPct.value + 0.35) % 100;
  }, 33);
});
onUnmounted(() => {
  if (timer !== null) window.clearInterval(timer);
});
</script>

<template>
  <div class="arrange">
    <div class="ruler">
      <div class="ruler-inner" :style="widthStyle">
        <span v-for="bar in totalBars" :key="bar" class="bar-mark" :style="{ width: barPx }">{{ bar }}</span>
      </div>
    </div>

    <div class="lanes" :style="widthStyle">
      <div v-for="(track, ti) in tracks" :key="track.id" class="lane">
        <div
          v-for="clip in track.clips"
          :key="clip.id"
          class="clip"
          :style="clipStyle(clip.start, clip.length, ti)"
        >
          <span
            v-for="(n, i) in clip.notes"
            :key="i"
            class="note-blip"
            :style="{ left: `${(n.start / clip.length) * 100}%`, width: `${Math.max(1.5, (n.duration / clip.length) * 100)}%` }"
          />
        </div>
      </div>
      <div v-if="tracks.length === 0" class="lane-empty text-body-md">还没有轨道</div>
      <div class="playhead" :style="{ left: `${playheadPct}%` }" />
    </div>
  </div>
</template>

<style scoped>
.arrange {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow: auto;
  background-color: rgb(var(--md-surface-container-lowest));
}
.ruler {
  height: 26px;
  flex: 0 0 auto;
  position: sticky;
  top: 0;
  z-index: 2;
  background-color: rgb(var(--md-surface-container-low));
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.3);
}
.ruler-inner {
  display: flex;
  height: 100%;
}
.bar-mark {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  padding-left: 7px;
  font-size: 10px;
  letter-spacing: 0.5px;
  white-space: nowrap;
  color: rgb(var(--md-on-surface-variant));
  border-left: 1px solid rgb(var(--md-outline-variant) / 0.4);
}
.lanes {
  position: relative;
  flex: 1 0 auto;
  display: flex;
  flex-direction: column;
}
/* 网格用 repeating 背景铺满整条 lane，不再依赖固定宽度 */
.lane {
  position: relative;
  height: 64px;
  flex: 0 0 auto;
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.18);
  background-image:
    repeating-linear-gradient(to right, rgb(var(--md-outline-variant) / 0.16) 0 1px, transparent 1px v-bind(beatPx)),
    repeating-linear-gradient(to right, rgb(var(--md-outline-variant) / 0.38) 0 1px, transparent 1px v-bind(barPx));
}
.clip {
  position: absolute;
  top: 9px;
  bottom: 9px;
  border-radius: 10px;
  background-color: hsl(var(--track-hue) 42% 30%);
  border: 1px solid hsl(var(--track-hue) 55% 48% / 0.85);
  overflow: hidden;
}
.note-blip {
  position: absolute;
  top: 44%;
  height: 14%;
  min-height: 3px;
  border-radius: 2px;
  background-color: hsl(var(--track-hue) 65% 82%);
  opacity: 0.9;
}
.lane-empty {
  padding: 24px;
  color: rgb(var(--md-on-surface-muted));
}
.playhead {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 2px;
  background-color: rgb(var(--md-primary) / 0.85);
  pointer-events: none;
}
@media (prefers-reduced-motion: reduce) {
  .playhead {
    display: none;
  }
}
</style>
