<script setup lang="ts">
/**
 * 通道电平表：左右两条柱 + 峰值保持线 + 削波指示，下面挂一条「增益衰减」小柱。
 *
 * 数据来自 `stores/meters`（已带弹道），这里只做「线性峰值 → 柱高」的换算与绘制。
 * 刻度固定 -60…+6 dB：比按峰值归一化更能看出「电量感」，也与主流调音台一致。
 */
import { computed } from "vue";
import type { ChannelLevel } from "@/stores/meters";

const props = withDefaults(
  defineProps<{
    value: ChannelLevel;
    /** 紧凑模式：右侧面板里的小表（不画刻度、不画 GR）。 */
    compact?: boolean;
    /** 无障碍标签（屏幕阅读器读这个通道叫什么）。 */
    label?: string;
  }>(),
  { compact: false, label: "" },
);

const emit = defineEmits<{ (e: "reset"): void }>();

/** 柱高范围（dB）：-60 以下视为静音，+6 给一点余量。 */
const MIN_DB = -60;
const MAX_DB = 6;

function toDb(peak: number): number {
  if (!(peak > 0)) return MIN_DB;
  return 20 * Math.log10(peak);
}

/** 线性峰值 → 0..1 柱高。 */
function ratio(peak: number): number {
  return Math.min(1, Math.max(0, (toDb(peak) - MIN_DB) / (MAX_DB - MIN_DB)));
}

const left = computed(() => ratio(props.value.l));
const right = computed(() => ratio(props.value.r));
const holdLeft = computed(() => ratio(props.value.holdL));
const holdRight = computed(() => ratio(props.value.holdR));
/** 增益衰减：正数 dB → 柱子高度（0…24 dB 满格）。 */
const gr = computed(() => Math.min(1, Math.max(0, props.value.gr / 24)));
const clipped = computed(() => props.value.clipped);

/** 给读屏的读数：取两声道里大的那个。 */
const readout = computed(() => {
  const peak = Math.max(props.value.l, props.value.r);
  return peak > 0 ? `${toDb(peak).toFixed(1)} dBFS` : "-∞ dBFS";
});
</script>

<template>
  <div
    class="meter"
    :class="{ compact, clipped }"
    role="img"
    :aria-label="`${label} ${readout}`"
    :title="label ? `${label} · ${readout}` : readout"
    @click="emit('reset')"
  >
    <span class="clip-led" :class="{ on: clipped }" aria-hidden="true" />
    <div class="bars">
      <div class="bar">
        <span class="fill" :style="{ height: `${left * 100}%` }" />
        <span v-if="holdLeft > 0" class="hold" :style="{ bottom: `${holdLeft * 100}%` }" />
      </div>
      <div class="bar">
        <span class="fill" :style="{ height: `${right * 100}%` }" />
        <span v-if="holdRight > 0" class="hold" :style="{ bottom: `${holdRight * 100}%` }" />
      </div>
    </div>
    <div v-if="!compact" class="gr" :class="{ active: gr > 0.01 }">
      <span class="gr-fill" :style="{ height: `${gr * 100}%` }" />
    </div>
  </div>
</template>

<style scoped>
.meter {
  position: relative;
  display: flex;
  align-items: stretch;
  gap: 3px;
  width: 20px;
  min-width: 20px;
  height: 100%;
  padding: 5px 0 0;
  cursor: pointer;
  user-select: none;
}
.meter.compact {
  width: 12px;
  min-width: 12px;
  gap: 2px;
}
.clip-led {
  position: absolute;
  top: 0;
  left: 50%;
  width: 5px;
  height: 5px;
  margin-left: -2.5px;
  border-radius: 50%;
  background-color: rgb(var(--md-outline-variant));
  transition: background-color 0.12s ease;
}
.clip-led.on {
  background-color: rgb(var(--md-error));
  box-shadow: 0 0 6px rgb(var(--md-error) / 0.8);
}
.bars {
  display: flex;
  gap: 2px;
  flex: 1 1 auto;
  min-height: 0;
}
.bar {
  position: relative;
  flex: 1 1 0;
  min-width: 4px;
  border-radius: 3px;
  background-color: rgb(var(--md-on-surface) / 0.18);
  overflow: hidden;
}
.fill {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 0;
  /* 峰值柱由下往上长；用渐变把「-12 以上变黄、-3 以上变红」写死在颜色里。 */
  background: linear-gradient(
    to top,
    rgb(var(--md-primary)) 0%,
    rgb(var(--md-primary)) 55%,
    rgb(0 190 120) 78%,
    rgb(240 180 0) 90%,
    rgb(var(--md-error)) 100%
  );
  background-size: 100% 260%;
  background-position: bottom;
  transition: height 33ms linear;
}
.hold {
  position: absolute;
  left: 0;
  right: 0;
  height: 2px;
  background-color: rgb(var(--md-on-surface) / 0.75);
  transition: bottom 33ms linear;
}
.meter.clipped .hold {
  background-color: rgb(var(--md-error));
}
.gr {
  position: relative;
  width: 3px;
  min-width: 3px;
  border-radius: 2px;
  background-color: rgb(var(--md-on-surface) / 0.08);
  overflow: hidden;
  opacity: 0;
  transition: opacity 0.2s ease;
}
.gr.active {
  opacity: 1;
}
.gr-fill {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  background-color: rgb(var(--md-tertiary, 120 120 120));
}
.meter.compact .bars {
  gap: 1px;
}
</style>
