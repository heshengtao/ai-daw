<script setup lang="ts">
/** 轻量提示（snackbar）。 */
import { useLayoutStore } from "@/stores/layout";

const layout = useLayoutStore();
</script>

<template>
  <transition name="toast">
    <div v-if="layout.toast" class="snackbar" role="status">{{ layout.toast }}</div>
  </transition>
</template>

<style scoped>
.snackbar {
  position: fixed;
  left: 50%;
  bottom: 44px;
  transform: translateX(-50%);
  max-width: 70vw;
  padding: 11px 18px;
  border-radius: 16px;
  font-size: 12.5px;
  letter-spacing: 0.2px;
  background-color: rgb(var(--md-inverse-surface));
  color: rgb(var(--md-inverse-on-surface));
  box-shadow: var(--md-elevation-3);
  z-index: 80;
}
.toast-enter-active {
  transition:
    opacity var(--md-dur-effects-default) var(--md-spring-effects-default),
    transform var(--md-dur-spatial-default) var(--md-spring-spatial-fast);
}
.toast-leave-active {
  transition:
    opacity var(--md-dur-effects-fast) var(--md-spring-effects-fast),
    transform var(--md-dur-effects-fast) var(--md-spring-effects-fast);
}
.toast-enter-from {
  opacity: 0;
  transform: translate(-50%, 16px) scale(0.94);
}
.toast-leave-to {
  opacity: 0;
  transform: translate(-50%, 8px) scale(0.96);
}
@media (prefers-reduced-motion: reduce) {
  .toast-enter-active,
  .toast-leave-active {
    transition: none;
  }
}
</style>
