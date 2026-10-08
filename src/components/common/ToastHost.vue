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
  border-radius: 999px;
  font-size: 12.5px;
  letter-spacing: 0.2px;
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface));
  box-shadow: var(--md-elevation-3);
  z-index: 80;
}
.toast-enter-active,
.toast-leave-active {
  transition: opacity 0.22s ease, transform 0.22s cubic-bezier(0.2, 0, 0, 1);
}
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translate(-50%, 8px);
}
@media (prefers-reduced-motion: reduce) {
  .toast-enter-active,
  .toast-leave-active {
    transition: none;
  }
}
</style>
