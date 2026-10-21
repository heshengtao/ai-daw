<script setup lang="ts">
/** 应用标志（波形条）；`filled` 时是一枚薰衣草紫的圆角底。 */
withDefaults(defineProps<{ filled?: boolean }>(), { filled: false });
</script>

<template>
  <span class="logo" :class="{ filled }">
    <svg viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
      <rect x="1" y="6" width="3" height="8" rx="1.5" />
      <rect x="6" y="3" width="3" height="14" rx="1.5" fill="#FFB74D" />
      <rect x="11" y="4.5" width="3" height="11" rx="1.5" />
      <rect x="16" y="7" width="3" height="6" rx="1.5" />
    </svg>
  </span>
</template>

<style scoped>
.logo {
  display: inline-flex;
  color: inherit;
}
.logo.filled {
  width: 24px;
  height: 24px;
  align-items: center;
  justify-content: center;
  border-radius: 8px;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}
.logo.filled svg {
  width: 15px;
  height: 15px;
}
</style>
