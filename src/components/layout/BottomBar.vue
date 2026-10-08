<script setup lang="ts">
/**
 * 底部条：传输控制与状态信息合并成一条（避免出现多个顶部栏）。
 */
import { computed, ref, watch } from "vue";
import { PlayIcon, StopIcon, ArrowUturnLeftIcon, ArrowsRightLeftIcon } from "@heroicons/vue/24/outline";
import { useProjectStore } from "@/stores/project";

const props = defineProps<{ playing: boolean; aiActive: boolean }>();
const emit = defineEmits<{ (e: "play"): void; (e: "stop"): void }>();

const store = useProjectStore();
const tempoDraft = ref("120");

watch(
  () => store.project?.transport.tempo,
  (t) => {
    if (t != null) tempoDraft.value = String(t);
  },
  { immediate: true },
);

/** 提交 BPM；空/NaN/越界一律回退显示，不发命令（引擎侧另有一层 clamp）。 */
function commitTempo(): void {
  const raw = tempoDraft.value.trim();
  const n = Number(raw);
  if (!raw || !Number.isFinite(n) || n < 20 || n > 300) {
    tempoDraft.value = String(store.project?.transport.tempo ?? 120);
    return;
  }
  void store.setTempo(n);
}

const timeSig = computed(() => {
  const t = store.project?.transport;
  return t ? `${t.time_sig_num}/${t.time_sig_den}` : "4/4";
});

const bars = computed(() => {
  const clips = (store.project?.tracks ?? []).flatMap((t) => t.clips);
  const beats = clips.reduce((m, c) => Math.max(m, c.start + c.length), 0);
  const num = store.project?.transport.time_sig_num ?? 4;
  return beats > 0 ? Math.ceil(beats / num) : 0;
});
</script>

<template>
  <footer class="bottom-bar">
    <button class="play-btn" :class="{ playing }" :title="playing ? '停止' : '试听'" @click="playing ? emit('stop') : emit('play')">
      <StopIcon v-if="playing" class="w-4 h-4" />
      <PlayIcon v-else class="w-4 h-4" />
    </button>
    <button class="bar-icon" disabled title="回到开头（实时传输接入后可用）">
      <ArrowUturnLeftIcon class="w-4 h-4" />
    </button>
    <button class="bar-icon" disabled title="循环（实时传输接入后可用）">
      <ArrowsRightLeftIcon class="w-4 h-4" />
    </button>

    <span class="sep" />

    <span class="readout">
      <span class="strong">{{ bars ? `${bars} 小节` : "—" }}</span>
      <span class="dim">{{ timeSig }}</span>
    </span>

    <label class="bpm">
      <span class="bpm-label">BPM</span>
      <input
        v-model="tempoDraft"
        class="bpm-input"
        type="number"
        min="20"
        max="300"
        @change="commitTempo"
        @blur="commitTempo"
      />
    </label>

    <div class="flex-1" />

    <span class="ai-pill" :class="{ live: aiActive }">
      <span class="dot" />
      <span class="ai-text">{{ aiActive ? "AI 正在写…" : "AI 空闲" }}</span>
    </span>
    <span class="engine">引擎就绪 · 44.1 kHz · 块 256 帧</span>
  </footer>
</template>

<style scoped>
.bottom-bar {
  height: 40px;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 10px;
  flex: 0 0 auto;
  background-color: rgb(var(--md-surface-container-low));
  border-top: 1px solid rgb(var(--md-outline-variant) / 0.3);
  font-size: 11.5px;
  letter-spacing: 0.3px;
  color: rgb(var(--md-on-surface-variant));
}
.play-btn {
  width: 30px;
  height: 30px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  cursor: pointer;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
  transition: transform 0.15s ease, background-color 0.2s ease;
}
.play-btn:hover {
  transform: scale(1.05);
}
.play-btn:active {
  transform: scale(0.94);
}
.play-btn.playing {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.play-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 2px;
}
.bar-icon {
  width: 28px;
  height: 28px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
}
.bar-icon:disabled {
  opacity: 0.45;
  cursor: default;
}
.bar-icon:not(:disabled):hover {
  background-color: rgb(var(--md-on-surface-variant) / var(--md-state-hover-alpha));
}
.sep {
  width: 1px;
  height: 16px;
  background-color: rgb(var(--md-outline-variant) / 0.55);
  flex: 0 0 auto;
}
.readout {
  display: inline-flex;
  align-items: baseline;
  gap: 6px;
  white-space: nowrap;
  flex: 0 0 auto;
}
.strong {
  color: rgb(var(--md-on-surface));
  font-weight: 600;
}
.dim {
  opacity: 0.75;
}
.bpm {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  flex: 0 0 auto;
  white-space: nowrap;
}
.bpm-label {
  font-size: 10px;
  letter-spacing: 0.8px;
  opacity: 0.75;
}
.bpm-input {
  width: 56px;
  height: 24px;
  padding: 0 6px;
  border-radius: 7px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface));
  font-size: 11.5px;
  outline: none;
}
.bpm-input:focus-visible {
  border-color: rgb(var(--md-primary));
  box-shadow: 0 0 0 2px rgb(var(--md-primary) / 0.25);
}
.ai-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 24px;
  padding: 0 10px;
  border-radius: 999px;
  white-space: nowrap;
  flex: 0 0 auto;
  background-color: rgb(var(--md-surface-container-high));
  transition: background-color 0.25s ease, color 0.25s ease;
}
.ai-pill .dot {
  width: 6px;
  height: 6px;
  border-radius: 999px;
  background-color: rgb(var(--md-on-surface-muted));
  flex: 0 0 auto;
  transition: background-color 0.25s ease, box-shadow 0.25s ease;
}
.ai-pill.live {
  background-color: rgb(var(--md-primary-container));
  color: rgb(var(--md-on-primary-container));
}
.ai-pill.live .dot {
  background-color: rgb(var(--md-primary));
  box-shadow: 0 0 0 3px rgb(var(--md-primary) / 0.25);
}
.engine {
  white-space: nowrap;
  opacity: 0.7;
  flex: 0 0 auto;
}

/* 窄屏：先丢引擎信息，再丢 AI 文案，不换行 */
@media (max-width: 860px) {
  .engine {
    display: none;
  }
}
@media (max-width: 620px) {
  .ai-text {
    display: none;
  }
  .dim {
    display: none;
  }
}
</style>
