<script setup lang="ts">
/**
 * 通道电平表：左右两条柱 + 峰值保持线 + 削波指示，下面挂一条「增益衰减」小柱。
 *
 * 数据来自 `stores/meters`（已带弹道），这里只做「线性峰值 → 柱高」的换算与绘制。
 * 刻度固定 -60…+6 dB：比按峰值归一化更能看出「电量感」，也与主流调音台一致。
 */
import { computed } from "vue";
import type { ChannelLevel } from "@/stores/meters";
import { useI18n } from "@/i18n";

const i18n = useI18n();

const props = withDefaults(
  defineProps<{
    value: ChannelLevel;
    /** 无障碍标签 / 悬停提示里的通道名。 */
    label?: string;
  }>(),
  { label: "" },
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

/** 单个声道的读数文本（-∞ 表示没信号）。 */
function dbText(peak: number): string {
  return peak > 0 && toDb(peak) > MIN_DB ? `${toDb(peak).toFixed(1)}` : "-∞";
}

/**
 * 悬停提示：说清楚「这两条是什么」。
 *
 * 表本身只有 20 px 宽，光看是猜不出左右声道的；把 L / R 读数、GR 与削波都写进 title，
 * 用户第一次把鼠标放上去就明白了。
 */
const title = computed(() => {
  const parts = [`L ${dbText(props.value.l)} / R ${dbText(props.value.r)} dBFS`];
  if (props.value.gr > 0.1) parts.push(`GR ${props.value.gr.toFixed(1)} dB`);
  if (props.value.clipped) parts.push("削波（点一下清除）");
  return props.label ? `${props.label} · ${parts.join(" · ")}` : parts.join(" · ");
});
</script>

<template>
  <div
    class="meter"
    :class="{ clipped }"
    role="img"
    :aria-label="title"
    :title="title"
    @click="emit('reset')"
  >
    <!-- 灯与两条柱同一个「芯」里：灯按柱子的中线居中，并且**排在柱子上方**（不是浮在上面） -->
    <div class="core">
      <span class="clip-led" :class="{ on: clipped }" :title="i18n.t('meter.clipHint')" />
      <div class="bars">
        <div class="bar" :title="`L ${dbText(value.l)} dBFS`">
          <span class="fill" :style="{ height: `${left * 100}%` }" />
          <span v-if="holdLeft > 0" class="hold" :style="{ bottom: `${holdLeft * 100}%` }" />
        </div>
        <div class="bar" :title="`R ${dbText(value.r)} dBFS`">
          <span class="fill" :style="{ height: `${right * 100}%` }" />
          <span v-if="holdRight > 0" class="hold" :style="{ bottom: `${holdRight * 100}%` }" />
        </div>
      </div>
    </div>
    <div class="gr" :class="{ active: gr > 0.01 }">
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
  cursor: pointer;
  user-select: none;
}
/* 灯 + 两条柱：同一个竖列，灯在柱子上方、按柱子中线居中 */
.core {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-height: 0;
}
.clip-led {
  flex: 0 0 auto;
  align-self: center;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background-color: rgb(var(--md-on-surface) / 0.12);
  box-shadow: 0 0 0 1px rgb(var(--md-outline-variant) / 0.7);
  transition: background-color 0.12s ease;
  cursor: pointer;
}
.clip-led.on {
  background-color: rgb(var(--md-error));
  box-shadow: 0 0 5px rgb(var(--md-error) / 0.85);
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
</style>
