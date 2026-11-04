<script setup lang="ts">
/**
 * AI 功能名称旁的**后端标签**（统一成一种含义）。
 *
 * 以前每个条目各写各的（「模型」/「未下载」/「内置 DSP」/ 设备名），既不一致也不说明
 * 这套能力到底跑在什么上。这里只回答一个问题：**它由哪种后端驱动**——
 *
 * - `onnx`：ONNX Runtime 里的 `.onnx` 权重（Basic Pitch / HTDemucs / Beat This! / SoulX…）；
 * - `audiocpp`：独立的 `audiocpp_server` 进程 + GGUF 权重（YuE2 / ACE-Step / SheetSage2 / ASR）；
 * - `dsp`：不加载模型，走内置信号处理（转 MIDI / 降噪的兜底路径）。
 *
 * `ready` 只影响左侧那个小圆点（实心 = 现在就能跑，空心 = 还缺权重 / 引擎），文字始终是后端名。
 */
import { computed } from "vue";
import Tooltip from "@/components/common/Tooltip.vue";
import { useI18n } from "@/i18n";

const props = defineProps<{
  backend: "onnx" | "audiocpp" | "dsp";
  /** `false` = 这套后端 / 权重还没就位；`undefined` = 不表示状态。 */
  ready?: boolean;
}>();

const i18n = useI18n();
const label = computed(() => i18n.t(`ai.backend.${props.backend}`));
const hint = computed(() => {
  const base = i18n.t(`ai.backend.${props.backend}Hint`);
  return props.ready === false ? `${base} · ${i18n.t("ai.backendNotReady")}` : base;
});
</script>

<template>
  <Tooltip :text="hint" place="above" align="center">
    <span class="ai-backend" :class="[backend, { missing: ready === false }]">
      <span class="dot" aria-hidden="true" />
      <span class="text">{{ label }}</span>
    </span>
  </Tooltip>
</template>

<style scoped>
.ai-backend {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 19px;
  margin-left: 6px;
  padding: 0 7px 0 6px;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.2px;
  line-height: 1;
  vertical-align: 1px;
  white-space: nowrap;
}
/* ONNX：tertiary（青绿系）——和 audio.cpp 一眼分得开，又不抢主操作的主色。 */
.ai-backend.onnx {
  background-color: rgb(var(--md-tertiary-container) / 0.62);
  color: rgb(var(--md-on-tertiary-container));
}
/* audio.cpp：主色（引擎 + GGUF 是「重」后端，给更强的存在感）。 */
.ai-backend.audiocpp {
  background-color: rgb(var(--md-primary-container) / 0.7);
  color: rgb(var(--md-on-primary-container));
}
/* 内置 DSP：中性灰，表示「没走模型」。 */
.ai-backend.dsp {
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface-variant));
}
.dot {
  width: 5px;
  height: 5px;
  flex: 0 0 auto;
  border-radius: 999px;
  background-color: currentColor;
}
/* 还没就位：整枚压暗，圆点留成空心环（文字仍然是后端名，不改成「未下载」）。 */
.ai-backend.missing {
  opacity: 0.66;
}
.ai-backend.missing .dot {
  background-color: transparent;
  box-shadow: inset 0 0 0 1px currentColor;
}
</style>
