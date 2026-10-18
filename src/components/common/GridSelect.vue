<script setup lang="ts">
/**
 * 「网格」字段：一行 **网格 + 药丸下拉**，供两条自动化车道的表头复用。
 *
 * 为什么要单独做一个组件：钢琴卷帘的控制器泳道与编排视图的自动化车道都要在表头放这一项，
 * 而且必须与旁边的参数下拉**同高同款**（22px 的小药丸）。样式写在这里，两边就永远一致。
 *
 * 组件只管展示与回传，选中的值由外壳绑到各自的设置上（卷帘网格 / 全局吸附网格）。
 */
import { computed } from "vue";
import Md3Select, { type SelectOption } from "@/components/common/Md3Select.vue";
import { useI18n } from "@/i18n";
import type { NoteGrid } from "@/stores/layout";

const props = defineProps<{
  modelValue: NoteGrid;
  /** 候选项（卷帘网格含「自由」，全局吸附不含）。 */
  options: SelectOption[];
  /** 前缀文字；默认「网格」。 */
  label?: string;
  /** 悬停提示（一行说明这个网格管什么）。 */
  title?: string;
}>();

const emit = defineEmits<{
  (e: "update:modelValue", value: NoteGrid): void;
}>();

const i18n = useI18n();

/** 无障碍名：前缀文字本身（「网格」），不用再单独传。 */
const ariaLabel = computed(() => props.label ?? i18n.t("edit.grid"));

function onPick(id: string): void {
  emit("update:modelValue", id as NoteGrid);
}
</script>

<template>
  <div class="grid-field" :title="title">
    <span class="grid-field-label">{{ ariaLabel }}</span>
    <Md3Select
      class="grid-field-select"
      :model-value="modelValue"
      :options="options"
      :label="ariaLabel"
      @update:model-value="onPick"
    />
  </div>
</template>

<style scoped>
.grid-field {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}
.grid-field-label {
  font-size: 11px;
  line-height: 1;
  letter-spacing: 0.2px;
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
}
/* 与相邻的参数下拉同款小药丸：22px 高、11px 字（`Md3Select` 默认是 34px 的宽药丸）。
   用 `:deep` 而不是直接给组件根元素写类：根元素的类与 `Md3Select` 自己的 `.md3-select`
   同权重，谁生效取决于样式表的先后顺序。 */
.grid-field :deep(.md3-select) {
  width: 68px;
  height: 22px;
  padding: 0 7px 0 9px;
  gap: 4px;
  font-size: 11px;
  font-variant-numeric: tabular-nums;
}
.grid-field :deep(.caret) {
  width: 12px;
  height: 12px;
}
</style>
