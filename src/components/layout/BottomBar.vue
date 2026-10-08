<script setup lang="ts">
/**
 * 底部条：传输控制居中，左侧读数、右侧状态。
 * 撤销 / 重做可用性来自工程状态；BPM 用自绘的 MD3 步进器（无原生控件）。
 */
import { computed, ref, watch } from "vue";
import {
  PlayIcon,
  StopIcon,
  ArrowUturnLeftIcon,
  ArrowUturnRightIcon,
} from "@heroicons/vue/24/outline";
import NumberField from "@/components/common/NumberField.vue";
import Tooltip from "@/components/common/Tooltip.vue";
import { useProjectStore } from "@/stores/project";

const props = defineProps<{ playing: boolean; aiActive: boolean }>();
const emit = defineEmits<{ (e: "play"): void; (e: "stop"): void }>();

const store = useProjectStore();
const tempo = ref(store.project?.transport.tempo ?? 120);

watch(
  () => store.project?.transport.tempo,
  (t) => {
    if (t != null) tempo.value = t;
  },
  { immediate: true },
);

// 步进器已把值夹在 20..300，store 还会再校验一次。
watch(tempo, (t) => {
  void store.setTempo(t);
});

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
    <!-- 左侧读数 -->
    <div class="zone">
      <span class="readout">
        <span class="strong">{{ bars ? `${bars} 小节` : "—" }}</span>
        <span class="dim">{{ timeSig }}</span>
      </span>
    </div>

    <!-- 传输：居中 -->
    <div class="transport">
      <Tooltip text="撤销 (Ctrl/Cmd+Z)" place="above" align="center">
        <button
          class="t-btn"
          :disabled="store.undoDepth === 0"
          aria-label="撤销"
          @click="store.undo()"
        >
          <ArrowUturnLeftIcon class="w-[17px] h-[17px]" />
        </button>
      </Tooltip>
      <Tooltip text="重做 (Ctrl/Cmd+Shift+Z)" place="above" align="center">
        <button
          class="t-btn"
          :disabled="store.redoDepth === 0"
          aria-label="重做"
          @click="store.redo()"
        >
          <ArrowUturnRightIcon class="w-[17px] h-[17px]" />
        </button>
      </Tooltip>

      <span class="t-sep" />

      <Tooltip :text="playing ? '停止（空格）' : '试听（空格）'" place="above" align="center">
        <button
          class="play-btn"
          :class="{ playing }"
          :aria-label="playing ? '停止' : '试听'"
          @click="playing ? emit('stop') : emit('play')"
        >
          <StopIcon v-if="playing" class="w-4 h-4" />
          <PlayIcon v-else class="w-[17px] h-[17px]" />
        </button>
      </Tooltip>

      <span class="t-sep" />

      <NumberField v-model="tempo" :min="20" :max="300" label="BPM" />
    </div>

    <!-- 右侧状态 -->
    <div class="zone end">
      <span class="ai-pill" :class="{ live: aiActive }">
        <span class="dot" />
        <span class="ai-text">{{ aiActive ? "AI 正在写…" : "AI 空闲" }}</span>
      </span>
      <span class="engine">引擎就绪 · 44.1 kHz · 块 256 帧</span>
    </div>
  </footer>
</template>

<style scoped>
.bottom-bar {
  height: 56px;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 0 14px;
  flex: 0 0 auto;
  background-color: rgb(var(--md-surface-container-low));
  border-top: 1px solid rgb(var(--md-outline-variant) / 0.3);
  font-size: 11.5px;
  letter-spacing: 0.3px;
  color: rgb(var(--md-on-surface-variant));
}
/* 左右两个等宽区把中间的传输组挤到正中 */
.zone {
  flex: 1 1 0;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
}
.zone.end {
  justify-content: flex-end;
}
.transport {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 3px;
  padding: 4px 6px;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-high));
  box-shadow: var(--md-elevation-1);
}
.t-btn {
  width: 32px;
  height: 32px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
  transition: background-color 0.15s ease, color 0.15s ease, transform 0.15s ease;
}
.t-btn:not(:disabled):hover {
  background-color: rgb(var(--md-on-surface-variant) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.t-btn:not(:disabled):active {
  transform: scale(0.92);
}
.t-btn:disabled {
  opacity: 0.35;
  cursor: default;
}
.t-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.t-sep {
  width: 1px;
  height: 18px;
  margin: 0 3px;
  background-color: rgb(var(--md-outline-variant) / 0.6);
  flex: 0 0 auto;
}
.play-btn {
  width: 36px;
  height: 36px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  cursor: pointer;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
  transition: transform 0.16s cubic-bezier(0.34, 1.56, 0.64, 1), background-color 0.2s ease;
}
.play-btn:hover {
  transform: scale(1.06);
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
.readout {
  display: inline-flex;
  align-items: baseline;
  gap: 7px;
  white-space: nowrap;
}
.strong {
  color: rgb(var(--md-on-surface));
  font-weight: 600;
}
.dim {
  opacity: 0.75;
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

/* 窄屏：先丢引擎信息，再丢 AI 文案与拍号，不换行 */
@media (max-width: 900px) {
  .engine {
    display: none;
  }
}
@media (max-width: 680px) {
  .ai-text {
    display: none;
  }
  .dim {
    display: none;
  }
}
@media (prefers-reduced-motion: reduce) {
  .play-btn,
  .t-btn {
    transition: background-color 0.15s ease;
  }
  .play-btn:hover,
  .play-btn:active,
  .t-btn:not(:disabled):active {
    transform: none;
  }
}
</style>
