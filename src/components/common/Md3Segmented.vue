<script setup lang="ts">
/**
 * MD3 分段按钮（滑块风格）：活动胶囊由 DOM 量出来，带弹簧动画。
 *
 * 这是**设置页 / AI 面板统一使用**的分段控件：ONNX Runtime 后端、audio.cpp 引擎后端、
 * 语言 / 主题 / 代理方式 / 下载源都用它，避免每个地方各写一套 px/圆角/动画。
 * （对话框里的 `.m3-seg` 仍是旧的静态样式，尚未迁移。）
 *
 * 和 `SegmentedNav` 的区别：那个是路由导航（点击即 `router.push`），这个是受控输入
 * （`v-model`），选项由调用方给。
 */
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type { SegmentedOption } from "@/components/common/segmented";

const props = withDefaults(
  defineProps<{
    modelValue: string | null;
    options: SegmentedOption[];
    /** `md`（36px，设置页） / `sm`（24px，AI 面板）。 */
    size?: "md" | "sm";
    ariaLabel?: string;
  }>(),
  { size: "md", ariaLabel: "" },
);

const emit = defineEmits<{ (e: "update:modelValue", value: string): void }>();

const root = ref<HTMLElement | null>(null);
const buttons = ref<(HTMLElement | null)[]>([]);
const pill = ref({ left: "0px", width: "0px" });
const activeIndex = computed(() => props.options.findIndex((o) => o.value === props.modelValue));

let resizeObserver: ResizeObserver | null = null;

function updatePill(): void {
  void nextTick(() => {
    const el = activeIndex.value >= 0 ? buttons.value[activeIndex.value] : null;
    if (el) pill.value = { left: `${el.offsetLeft}px`, width: `${el.offsetWidth}px` };
  });
}

watch(
  [activeIndex, () => props.options.map((o) => `${o.value}:${o.label}`).join("|")],
  updatePill,
);
onMounted(() => {
  updatePill();
  window.addEventListener("resize", updatePill);
  // 容器自身变宽 / 变窄（侧栏、窗口、字体回退）时按钮宽度都会变，只监听 window 不够。
  if (root.value && typeof ResizeObserver !== "undefined") {
    resizeObserver = new ResizeObserver(updatePill);
    resizeObserver.observe(root.value);
  }
  // Web 字体换上之后按钮会重新排版，此时量出来的胶囊位置 / 宽度已经过期。
  void document.fonts?.ready.then(updatePill);
});
onUnmounted(() => {
  window.removeEventListener("resize", updatePill);
  resizeObserver?.disconnect();
  resizeObserver = null;
});
</script>

<template>
  <div ref="root" class="md3-seg" :class="size" role="group" :aria-label="ariaLabel || undefined">
    <div
      v-if="activeIndex >= 0"
      class="md3-seg-pill"
      :style="{ left: pill.left, width: pill.width }"
    />
    <button
      v-for="(option, index) in options"
      :key="option.value"
      :ref="(el) => { if (el) buttons[index] = el as HTMLElement }"
      type="button"
      class="md3-seg-btn"
      :class="{ on: activeIndex === index }"
      :disabled="option.disabled"
      :title="option.title"
      :aria-pressed="activeIndex === index"
      @click="emit('update:modelValue', option.value)"
    >
      {{ option.label }}
    </button>
  </div>
</template>

<style scoped>
.md3-seg {
  position: relative;
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 4px;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-high));
  max-width: 100%;
  /* 分段控件必须是**一条**：换行会让胶囊只量到第一行，整个控件看起来像两张卡片叠在一起。 */
  flex-wrap: nowrap;
  /* 窄容器（例如收窄后的右侧 AI 面板）里不换行、也不裁掉最后一项，而是可横向滚动。
     滚动条隐藏，滚轮 / 触控板 / 键盘焦点仍然能到达被挡住的选项。 */
  overflow-x: auto;
  scrollbar-width: none;
}
.md3-seg::-webkit-scrollbar {
  display: none;
}
.md3-seg.sm {
  padding: 2px;
  background-color: rgb(var(--md-surface-container-highest));
}
.md3-seg-pill {
  position: absolute;
  top: 4px;
  bottom: 4px;
  border-radius: 999px;
  background-color: rgb(var(--md-secondary-container));
  box-shadow: var(--md-elevation-1);
  pointer-events: none;
  transition:
    left 0.32s cubic-bezier(0.34, 1.56, 0.64, 1),
    width 0.32s cubic-bezier(0.34, 1.56, 0.64, 1);
}
.md3-seg.sm .md3-seg-pill {
  top: 2px;
  bottom: 2px;
  box-shadow: none;
}
.md3-seg-btn {
  position: relative;
  z-index: 1;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  height: 36px;
  padding: 0 16px;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 14px;
  font-weight: 500;
  white-space: nowrap;
  cursor: pointer;
  user-select: none;
  transition: color var(--md-duration-short-2) var(--md-ease-spring-fast);
}
.md3-seg.sm .md3-seg-btn {
  height: 24px;
  padding: 0 10px;
  font-size: 11px;
}
.md3-seg-btn:hover:not(:disabled) {
  color: rgb(var(--md-on-surface));
}
.md3-seg-btn.on {
  color: rgb(var(--md-on-secondary-container));
}
.md3-seg-btn:disabled {
  opacity: 0.5;
  cursor: default;
}
@media (prefers-reduced-motion: reduce) {
  .md3-seg-pill {
    transition: none;
  }
}
</style>
