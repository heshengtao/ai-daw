<script setup lang="ts">
/** MD3 自绘 tooltip：不用原生 title，统一风格。 */
import { onUnmounted, ref } from "vue";

const props = withDefaults(
  defineProps<{ text: string; place?: "below" | "above"; align?: "center" | "end"; delay?: number }>(),
  { place: "below", align: "center", delay: 300 },
);

const open = ref(false);
let timer: number | null = null;

function clearTimer(): void {
  if (timer !== null) {
    window.clearTimeout(timer);
    timer = null;
  }
}

function show(): void {
  clearTimer();
  timer = window.setTimeout(() => {
    open.value = true;
    timer = null;
  }, props.delay);
}

function hide(): void {
  clearTimer();
  open.value = false;
}

onUnmounted(clearTimer);
</script>

<template>
  <span
    class="tip-wrap"
    @pointerenter="show"
    @pointerleave="hide"
    @pointerdown="hide"
    @focusin="show"
    @focusout="hide"
  >
    <slot />
    <transition name="tip">
      <span v-if="open" class="tip" :class="[`tip-${place}`, `tip-${align}`]" role="tooltip">{{ text }}</span>
    </transition>
  </span>
</template>

<style scoped>
.tip-wrap {
  position: relative;
  display: inline-flex;
}
.tip {
  position: absolute;
  z-index: 90;
  padding: 6px 10px;
  border-radius: 7px;
  font-size: 11px;
  font-weight: 500;
  line-height: 1.35;
  letter-spacing: 0.3px;
  white-space: nowrap;
  pointer-events: none;
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface));
  border: 1px solid rgb(var(--md-outline-variant) / 0.45);
  box-shadow: var(--md-elevation-2);
}
.tip-below {
  top: calc(100% + 7px);
}
.tip-above {
  bottom: calc(100% + 7px);
}
.tip-center {
  left: 50%;
  transform: translateX(-50%);
}
.tip-end {
  right: 0;
}
.tip-enter-active,
.tip-leave-active {
  transition: opacity 0.14s ease, transform 0.14s ease;
}
.tip-enter-from,
.tip-leave-to {
  opacity: 0;
}
@media (prefers-reduced-motion: reduce) {
  .tip-enter-active,
  .tip-leave-active {
    transition: none;
  }
}
</style>
