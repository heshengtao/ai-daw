<script setup lang="ts">
/**
 * 分段胶囊导航：活动胶囊位置由 DOM 量出，带弹簧动画。
 * 交互模式参考 ExamBot（Apache-2.0）。
 */
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { useI18n } from "@/i18n";
import { useRoute, useRouter } from "vue-router";
import type { NavItem } from "./nav";

const props = defineProps<{ items: NavItem[] }>();

const route = useRoute();
const router = useRouter();
const i18n = useI18n();
const buttons = ref<(HTMLElement | null)[]>([]);
const pill = ref<{ left: string; width: string }>({ left: "0px", width: "0px" });

const activeIndex = computed(() =>
  props.items.findIndex((i) => route.path === i.path || route.path.startsWith(`${i.path}/`)),
);

function updatePill(): void {
  void nextTick(() => {
    const idx = activeIndex.value;
    const el = idx >= 0 ? buttons.value[idx] : null;
    if (el) pill.value = { left: `${el.offsetLeft}px`, width: `${el.offsetWidth}px` };
  });
}

watch([activeIndex, () => props.items.length], updatePill);
onMounted(() => {
  updatePill();
  window.addEventListener("resize", updatePill);
});
onUnmounted(() => window.removeEventListener("resize", updatePill));
</script>

<template>
  <nav
    class="relative flex items-center p-1 rounded-full gap-0.5"
    :style="{ backgroundColor: 'rgb(var(--md-surface-container-high))' }"
  >
    <div
      v-if="activeIndex >= 0"
      class="absolute top-1 bottom-1 rounded-full pointer-events-none motion-pill"
      :style="{ left: pill.left, width: pill.width, backgroundColor: 'rgb(var(--md-secondary-container))' }"
    />
    <button
      v-for="(item, idx) in items"
      :key="item.path"
      :ref="(el) => { if (el) buttons[idx] = el as HTMLElement }"
      class="relative z-10 flex items-center gap-2 px-4 h-9 rounded-full text-sm font-medium cursor-pointer select-none border-0 bg-transparent whitespace-nowrap"
      :style="{ color: activeIndex === idx ? 'rgb(var(--md-on-secondary-container))' : 'rgb(var(--md-on-surface-variant))' }"
      @click="router.push(item.path)"
    >
      <component :is="item.icon" class="w-4 h-4" />
      <span>{{ i18n.t(item.labelKey) }}</span>
    </button>
  </nav>
</template>

<style scoped>
.motion-pill {
  transition:
    left 0.32s cubic-bezier(0.34, 1.56, 0.64, 1),
    width 0.32s cubic-bezier(0.34, 1.56, 0.64, 1);
}
@media (prefers-reduced-motion: reduce) {
  .motion-pill {
    transition: none;
  }
}
</style>
