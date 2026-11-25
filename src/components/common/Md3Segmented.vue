<script setup lang="ts">
/**
 * MD3 Expressive Connected Button Group（连接按钮组）：
 * 设置页 / AI 面板统一使用的分段控件：外侧大圆角、项间 3px 缝隙、按压横向弹簧扩张、选中项变形为完整胶囊。
 */
import type { SegmentedOption } from "@/components/common/segmented";

withDefaults(
  defineProps<{
    modelValue: string | null;
    options: SegmentedOption[];
    /** `md`（36px，设置页） / `sm`（28px，AI 面板）。 */
    size?: "md" | "sm";
    ariaLabel?: string;
  }>(),
  { size: "md", ariaLabel: "" },
);

const emit = defineEmits<{ (e: "update:modelValue", value: string): void }>();
</script>

<template>
  <div class="md3-seg" :class="size" role="group" :aria-label="ariaLabel || undefined">
    <button
      v-for="option in options"
      :key="option.value"
      type="button"
      class="md3-seg-btn"
      :class="{ on: modelValue === option.value }"
      :disabled="option.disabled"
      :title="option.title"
      :aria-pressed="modelValue === option.value"
      @click="emit('update:modelValue', option.value)"
    >
      {{ option.label }}
    </button>
  </div>
</template>

<style scoped>
.md3-seg {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  max-width: 100%;
  min-width: 0;
  flex-wrap: nowrap;
  overflow-x: auto;
  scrollbar-width: none;
}
.md3-seg::-webkit-scrollbar {
  display: none;
}
.md3-seg-btn {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  height: 36px;
  padding: 0 15px;
  border: 0;
  border-radius: 8px;
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface-variant));
  font-size: 13px;
  font-weight: 500;
  white-space: nowrap;
  cursor: pointer;
  user-select: none;
  transition:
    border-radius var(--md-dur-spatial-fast) var(--md-spring-spatial-bouncy),
    padding var(--md-dur-spatial-fast) var(--md-spring-spatial-bouncy),
    background-color var(--md-dur-effects-default) var(--md-spring-effects-default),
    color var(--md-dur-effects-default) var(--md-spring-effects-default);
}
.md3-seg-btn:first-child {
  border-radius: 18px 8px 8px 18px;
}
.md3-seg-btn:last-child {
  border-radius: 8px 18px 18px 8px;
}
.md3-seg-btn:only-child {
  border-radius: 18px;
}
.md3-seg-btn:hover:not(:disabled):not(.on) {
  color: rgb(var(--md-on-surface));
  filter: brightness(1.06);
}
.md3-seg-btn:active:not(:disabled) {
  padding: 0 19px;
  border-radius: 13px;
}
.md3-seg-btn.on {
  border-radius: 18px;
  padding: 0 18px;
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
  font-weight: 600;
}
.md3-seg-btn:disabled {
  opacity: 0.45;
  cursor: default;
}
.md3-seg-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}

.md3-seg.sm {
  gap: 2px;
}
.md3-seg.sm .md3-seg-btn {
  height: 28px;
  padding: 0 11px;
  border-radius: 7px;
  font-size: 11.5px;
}
.md3-seg.sm .md3-seg-btn:first-child {
  border-radius: 14px 7px 7px 14px;
}
.md3-seg.sm .md3-seg-btn:last-child {
  border-radius: 7px 14px 14px 7px;
}
.md3-seg.sm .md3-seg-btn:only-child {
  border-radius: 14px;
}
.md3-seg.sm .md3-seg-btn:active:not(:disabled) {
  padding: 0 14px;
  border-radius: 10px;
}
.md3-seg.sm .md3-seg-btn.on {
  border-radius: 14px;
  padding: 0 13px;
}

@media (prefers-reduced-motion: reduce) {
  .md3-seg-btn {
    transition: none;
  }
}
</style>
