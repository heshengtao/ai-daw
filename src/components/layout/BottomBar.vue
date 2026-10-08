<script setup lang="ts">
/**
 * 底部条：三栏网格（1fr / auto / 1fr），传输键因此始终严格居中，左右容器等宽。
 * 左：撤销 / 重做 / 拍号 / 位置读数；中：回到起点 · 播放暂停 · 停止；右：BPM / 状态。
 *
 * 传输直接调用 `transport` store：实时播放走原生音频线程，设备不可用时自动降级为
 * 离线试听，两种模式下这些按钮的行为完全一致。
 */
import { computed, ref, watch } from "vue";
import {
  PlayIcon,
  PauseIcon,
  StopIcon,
  BackwardIcon,
  ArrowUturnLeftIcon,
  ArrowUturnRightIcon,
} from "@heroicons/vue/24/outline";
import NumberField from "@/components/common/NumberField.vue";
import TimeSignatureField from "@/components/common/TimeSignatureField.vue";
import Tooltip from "@/components/common/Tooltip.vue";
import { useProjectStore } from "@/stores/project";
import { useTransportStore } from "@/stores/transport";
import { useI18n } from "@/i18n";

const props = defineProps<{ aiActive: boolean }>();

const store = useProjectStore();
const transport = useTransportStore();
const i18n = useI18n();
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

const timeSigNum = computed(() => store.project?.transport.time_sig_num ?? 4);
const timeSigDen = computed(() => store.project?.transport.time_sig_den ?? 4);

/** 拍号变化：交给 store（内部会做范围校验，引擎再归一化一次）。 */
function onTimeSignature(num: number, den: number): void {
  void store.setTimeSignature(num, den);
}

/** 工程内容长度（小节）。 */
const bars = computed(() => {
  const clips = (store.project?.tracks ?? []).flatMap((t) => t.clips);
  const beats = clips.reduce((m, c) => Math.max(m, c.start + c.length), 0);
  const num = timeSigNum.value;
  return beats > 0 ? Math.ceil(beats / num) : 0;
});

/** 当前位置（小节.拍），随实时位置推进。 */
const positionText = computed(() => {
  const beats = transport.positionBeats;
  const num = timeSigNum.value;
  const bar = Math.floor(beats / num) + 1;
  const beat = Math.floor(beats % num) + 1;
  return `${bar}.${beat}`;
});

/** 采样率展示（kHz，保留一位小数）。 */
const rateText = computed(() => {
  const khz = transport.effectiveSampleRate / 1000;
  return `${khz.toFixed(1)} kHz`;
});

const isPlaying = computed(() => transport.playing);
const isAiActive = computed(() => props.aiActive);
const isPreview = computed(() => transport.mode === "preview");
</script>

<template>
  <footer class="bottom-bar">
    <!-- 左：编辑历史 + 拍号 + 位置 -->
    <div class="zone left">
      <Tooltip :text="i18n.t('bottom.undo')" place="above" align="center">
        <button
          class="t-btn"
          :disabled="store.undoDepth === 0"
          :aria-label="i18n.t('bottom.undoLabel')"
          @click="store.undo()"
        >
          <ArrowUturnLeftIcon class="w-[17px] h-[17px]" />
        </button>
      </Tooltip>
      <Tooltip :text="i18n.t('bottom.redo')" place="above" align="center">
        <button
          class="t-btn"
          :disabled="store.redoDepth === 0"
          :aria-label="i18n.t('bottom.redoLabel')"
          @click="store.redo()"
        >
          <ArrowUturnRightIcon class="w-[17px] h-[17px]" />
        </button>
      </Tooltip>

      <span class="t-sep" />

      <TimeSignatureField :num="timeSigNum" :den="timeSigDen" @change="onTimeSignature" />

      <span class="readout" :title="i18n.t('bottom.positionTitle', { bar: positionText })">
        <span class="readout-label">{{ i18n.t("bottom.position") }}</span>
        <span class="strong tabular">{{ positionText }}</span>
      </span>
    </div>

    <!-- 中：传输键（严格居中） -->
    <div class="core">
      <Tooltip :text="i18n.t('bottom.toStart')" place="above" align="center">
        <button class="t-btn" :aria-label="i18n.t('bottom.toStartLabel')" @click="transport.rewind()">
          <BackwardIcon class="w-[17px] h-[17px]" />
        </button>
      </Tooltip>

      <Tooltip :text="isPlaying ? i18n.t('bottom.pause') : i18n.t('bottom.play')" place="above" align="center">
        <button
          class="play-btn"
          :class="{ playing: isPlaying }"
          :aria-label="isPlaying ? i18n.t('bottom.pauseLabel') : i18n.t('bottom.playLabel')"
          @click="transport.toggle()"
        >
          <PauseIcon v-if="isPlaying" class="w-[18px] h-[18px]" />
          <PlayIcon v-else class="w-[20px] h-[20px]" />
        </button>
      </Tooltip>

      <Tooltip :text="i18n.t('bottom.stop')" place="above" align="center">
        <button class="t-btn" :aria-label="i18n.t('bottom.stopLabel')" @click="transport.stop()">
          <StopIcon class="w-[17px] h-[17px]" />
        </button>
      </Tooltip>
    </div>

    <!-- 右：BPM + 状态 -->
    <div class="zone right">
      <NumberField v-model="tempo" :min="20" :max="300" label="BPM" />

      <span class="t-sep" />

      <span class="readout">
        <span class="strong tabular">{{ bars }}</span>
        <span class="readout-label">{{ i18n.t("bottom.bar") }}</span>
      </span>

      <span class="ai-pill" :class="{ live: isAiActive }">
        <span class="dot" />
        <span class="ai-text">{{ isAiActive ? i18n.t("bottom.aiBusy") : i18n.t("bottom.aiIdle") }}</span>
      </span>
      <span class="engine" :class="{ fallback: isPreview }">
        {{ isPreview ? i18n.t("bottom.offline") : i18n.t("bottom.realtime") }} · {{ rateText }}
      </span>
    </div>
  </footer>
</template>

<style scoped>
/* 三栏等宽（1fr / auto / 1fr）→ 中栏永远居中，左右容器宽度对称。 */
.bottom-bar {
  height: 60px;
  display: grid;
  grid-template-columns: 1fr auto 1fr;
  align-items: center;
  gap: 12px;
  padding: 0 14px;
  flex: 0 0 auto;
  background-color: var(--app-card);
  border-top: 1px solid rgb(var(--md-outline-variant) / 0.4);
  font-size: 11.5px;
  letter-spacing: 0.3px;
  color: rgb(var(--md-on-surface-variant));
}
.zone {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}
.zone.left {
  justify-content: flex-end;
}
.zone.right {
  justify-content: flex-start;
}
.core {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
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
  transition:
    background-color var(--md-ease-emphasized),
    color var(--md-ease-emphasized),
    transform var(--md-ease-emphasized);
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
  margin: 0 4px;
  background-color: rgb(var(--md-outline-variant) / 0.6);
  flex: 0 0 auto;
}
.play-btn {
  width: 44px;
  height: 44px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  cursor: pointer;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
  box-shadow: var(--md-elevation-1);
  transition:
    transform var(--md-ease-emphasized),
    background-color var(--md-ease-emphasized),
    box-shadow var(--md-ease-emphasized);
}
.play-btn:hover {
  transform: scale(1.07);
  box-shadow: var(--md-elevation-2);
}
.play-btn:active {
  transform: scale(0.93);
}
.play-btn.playing {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.play-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 3px;
}
.readout {
  display: inline-flex;
  align-items: baseline;
  gap: 6px;
  white-space: nowrap;
  flex: 0 0 auto;
}
.readout-label {
  opacity: 0.72;
}
.tabular {
  font-variant-numeric: tabular-nums;
}
.strong {
  color: rgb(var(--md-on-surface));
  font-weight: 600;
  min-width: 20px;
  text-align: right;
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
  transition: background-color var(--md-ease-emphasized), color var(--md-ease-emphasized);
}
.ai-pill .dot {
  width: 6px;
  height: 6px;
  border-radius: 999px;
  background-color: rgb(var(--md-on-surface-muted));
  flex: 0 0 auto;
  transition: background-color var(--md-ease-emphasized), box-shadow var(--md-ease-emphasized);
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
.engine.fallback {
  color: rgb(var(--md-tertiary));
  opacity: 0.95;
}

/* 窄屏：先丢引擎信息与读数，保持左右对称不被破坏 */
@media (max-width: 1080px) {
  .engine {
    display: none;
  }
}
@media (max-width: 900px) {
  .readout {
    display: none;
  }
}
@media (max-width: 760px) {
  .ai-text {
    display: none;
  }
}
@media (prefers-reduced-motion: reduce) {
  .play-btn,
  .t-btn,
  .ai-pill,
  .ai-pill .dot {
    transition: background-color 0.15s ease;
  }
  .play-btn:hover,
  .play-btn:active,
  .t-btn:not(:disabled):active {
    transform: none;
  }
}
</style>
