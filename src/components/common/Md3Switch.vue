<script setup lang="ts">
/**
 * MD3 开关。
 *
 * 视觉完全走 `main.css` 的 `.m3-switch`，这里只补上无障碍语义与「混合态」：
 * 多选时一部分开着、一部分关着，用一条短横的把手表示，点一下统一打开。
 */
const props = withDefaults(
  defineProps<{
    modelValue: boolean;
    /** 无障碍名（开关旁边已经有可见标签时用它复述）。 */
    label: string;
    disabled?: boolean;
    /** 混合态：部分选中。 */
    mixed?: boolean;
  }>(),
  { disabled: false, mixed: false },
);

const emit = defineEmits<{ (e: "update:modelValue", value: boolean): void }>();

function toggle(): void {
  if (props.disabled) return;
  emit("update:modelValue", !props.modelValue);
}
</script>

<template>
  <button
    class="m3-switch"
    :class="{ on: modelValue, mixed }"
    type="button"
    role="switch"
    :aria-checked="mixed ? 'mixed' : modelValue"
    :aria-label="label"
    :disabled="disabled"
    @click="toggle"
  />
</template>

<style scoped>
.m3-switch:disabled {
  opacity: 0.45;
  cursor: default;
}
.m3-switch:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 2px;
}
/* 混合态：轨道用主色容器，把手压成一条短线 */
.m3-switch.mixed {
  background-color: rgb(var(--md-primary-container));
  border-color: rgb(var(--md-primary));
}
.m3-switch.mixed::after {
  left: 15px;
  width: 18px;
  height: 8px;
  margin-top: -4px;
  background-color: rgb(var(--md-primary));
}
</style>
