<script setup lang="ts">
/** 传输条：试听/停止 + 小节读数 + BPM + 拍号 + AI 状态胶囊。 */
import { computed, ref, watch } from "vue";
import { PlayIcon, StopIcon, ArrowUturnLeftIcon, ArrowsRightLeftIcon } from "@heroicons/vue/24/outline";
import { useProjectStore } from "@/stores/project";

defineProps<{ playing: boolean; aiActive: boolean }>();
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
  <div class="transport">
    <div class="group">
      <button class="btn-tonal tbtn" title="停止" @click="emit('stop')">
        <StopIcon class="w-5 h-5" />
      </button>
      <button class="btn-filled tbtn" :title="playing ? '播放中…' : '试听'" @click="emit('play')">
        <PlayIcon class="w-5 h-5" />
      </button>
      <button class="btn-icon" disabled title="回到开头（实时传输接入后可用）">
        <ArrowUturnLeftIcon class="w-4 h-4" />
      </button>
      <button class="btn-icon" disabled title="循环（实时传输接入后可用）">
        <ArrowsRightLeftIcon class="w-4 h-4" />
      </button>
    </div>

    <div class="group readout">
      <span class="strong">{{ bars ? `${bars} 小节` : "—" }}</span>
      <span class="sep">·</span>
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
      <span class="sep">·</span>
      <span>{{ timeSig }}</span>
    </div>

    <div class="flex-1" />

    <span class="ai-pill" :class="{ live: aiActive }">
      <span class="dot" />
      {{ aiActive ? "AI 正在写…" : "AI 空闲" }}
    </span>
  </div>
</template>

<style scoped>
.transport {
  height: 48px;
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 0 14px;
  background-color: rgb(var(--md-surface-container-low));
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.3);
  flex: 0 0 auto;
}
.group {
  display: flex;
  align-items: center;
  gap: 6px;
}
.tbtn {
  width: 40px;
  padding: 0;
  height: 40px;
  border-radius: 999px;
}
.readout {
  font-size: 12px;
  letter-spacing: 0.4px;
  color: rgb(var(--md-on-surface-variant));
}
.strong {
  color: rgb(var(--md-on-surface));
  font-weight: 600;
}
.sep {
  opacity: 0.45;
}
.bpm {
  display: flex;
  align-items: center;
  gap: 6px;
}
.bpm-label {
  font-size: 10px;
  letter-spacing: 0.8px;
  opacity: 0.75;
}
.bpm-input {
  width: 62px;
  height: 28px;
  padding: 0 8px;
  border-radius: 8px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface));
  font-size: 12px;
  outline: none;
}
.bpm-input:focus-visible {
  border-color: rgb(var(--md-primary));
  box-shadow: 0 0 0 2px rgb(var(--md-primary) / 0.25);
}
.ai-pill {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  height: 28px;
  padding: 0 12px;
  border-radius: 999px;
  font-size: 11px;
  letter-spacing: 0.4px;
  background-color: rgb(var(--md-surface-container-high));
  color: rgb(var(--md-on-surface-variant));
  transition: background-color 0.25s ease, color 0.25s ease;
}
.ai-pill .dot {
  width: 7px;
  height: 7px;
  border-radius: 999px;
  background-color: rgb(var(--md-on-surface-muted));
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
</style>
