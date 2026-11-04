<script setup lang="ts">
/**
 * 下窗口的视图切换器：编排用「自动化 / 乐谱 / 键盘」，编辑用「控制器 / 乐谱 / 键盘」。
 *
 * 三个下窗格共用它，放在表头最左侧——三个按钮比「文字标题 + 下拉」省地方，也把
 * 「这是一个可切换的窗格」直接摆在明面上。参数选择（力度 / 通道 / 自动化参数）留在
 * 控制器 / 自动化窗格自己的下拉里，不再和视图混在一起。
 */
import {
  AdjustmentsHorizontalIcon,
  AdjustmentsVerticalIcon,
  MusicalNoteIcon,
} from "@heroicons/vue/24/outline";
import KeyboardIcon from "@/components/common/icons/KeyboardIcon.vue";
import Tooltip from "@/components/common/Tooltip.vue";
import { useI18n } from "@/i18n";
import { ARRANGE_LANE_VIEWS, type LaneView } from "@/stores/layout";

const props = withDefaults(
  defineProps<{
    modelValue: LaneView;
    /** 这个宿主能切哪几种视图；默认编排的三格。 */
    views?: LaneView[];
  }>(),
  { views: () => ARRANGE_LANE_VIEWS },
);
const emit = defineEmits<{ (e: "update:modelValue", value: LaneView): void }>();

const i18n = useI18n();

const ICONS: Record<LaneView, unknown> = {
  automation: AdjustmentsHorizontalIcon,
  lane: AdjustmentsVerticalIcon,
  score: MusicalNoteIcon,
  keyboard: KeyboardIcon,
};

const LABELS: Record<LaneView, string> = {
  automation: "auto.title",
  lane: "lane.controller",
  score: "lane.score",
  keyboard: "lane.keyboard",
};
</script>

<template>
  <div class="lane-view-switch" role="group" :aria-label="i18n.t('lane.viewSwitch')">
    <Tooltip v-for="view in props.views" :key="view" :text="i18n.t(LABELS[view])" place="above">
      <button
        type="button"
        class="view-btn"
        :class="{ on: modelValue === view }"
        :aria-pressed="modelValue === view"
        :aria-label="i18n.t(LABELS[view])"
        @click="emit('update:modelValue', view)"
      >
        <component :is="ICONS[view]" class="view-icon" />
      </button>
    </Tooltip>
  </div>
</template>

<style scoped>
.lane-view-switch {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 1px;
  height: 24px;
  padding: 2px;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-high));
  border: 1px solid rgb(var(--md-outline-variant) / 0.35);
}
.view-btn {
  width: 26px;
  height: 20px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease;
}
.view-btn:hover {
  background-color: rgb(var(--md-on-surface-variant) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.view-btn.on {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.view-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.view-icon {
  width: 14px;
  height: 14px;
}
@media (prefers-reduced-motion: reduce) {
  .view-btn {
    transition: none;
  }
}
</style>
