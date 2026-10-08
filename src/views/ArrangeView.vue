<script setup lang="ts">
/** 编排视图：时间线网格 + 带轨道配色的片段块（编辑功能后续接入）。 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useProjectStore } from "@/stores/project";
import { trackHue } from "@/utils/trackColor";

const store = useProjectStore();
const zoom = ref(56); // 每拍像素

const tracks = computed(() => store.project?.tracks ?? []);
const beatsPerBar = computed(() => store.project?.transport.time_sig_num ?? 4);
const totalBars = computed(() => {
  const beats = tracks.value.flatMap((t) => t.clips).reduce((m, c) => Math.max(m, c.start + c.length), 0);
  return Math.max(4, Math.ceil(beats / beatsPerBar.value) + 1);
});

const gridStyle = computed(() => ({
  width: `${totalBars.value * beatsPerBar.value * zoom.value}px`,
  backgroundSize: `${zoom.value}px 100%, ${zoom.value * beatsPerBar.value}px 100%`,
}));

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
      <div class="ruler-inner" :style="{ width: gridStyle.width }">
        <span v-for="bar in totalBars" :key="bar" class="bar-mark" :style="{ width: `${beatsPerBar * zoom}px` }">
          {{ bar }}
        </span>
      </div>
    </div>

    <div class="lanes">
      <div v-for="(track, ti) in tracks" :key="track.id" class="lane">
        <div class="lane-grid" :style="gridStyle" />
        <div
          v-for="clip in track.clips"
          :key="clip.id"
          class="clip"
          :style="clipStyle(clip.start, clip.length, ti)"
          :title="`${clip.notes.length} 音符`"
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
  height: 28px;
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
  color: rgb(var(--md-on-surface-variant));
  border-left: 1px solid rgb(var(--md-outline-variant) / 0.4);
}
.lanes {
  position: relative;
  display: flex;
  flex-direction: column;
}
.lane {
  position: relative;
  height: 64px;
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.18);
}
.lane-grid {
  position: absolute;
  inset: 0;
  background-image:
    linear-gradient(to right, rgb(var(--md-outline-variant) / 0.16) 1px, transparent 1px),
    linear-gradient(to right, rgb(var(--md-outline-variant) / 0.38) 1px, transparent 1px);
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
