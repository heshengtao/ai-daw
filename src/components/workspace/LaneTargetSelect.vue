<script setup lang="ts">
/**
 * 底部窗格的「参数目标」下拉：力度 / 压力 / 通道 / 自动化参数。
 *
 * 视图切换（控制器 / 乐谱 / 键盘）已经独立到表头左侧的 `LaneViewSwitch`，这里只负责
 * 「当前窗格里编辑哪个参数」——两者分开后，切视图和切参数互不干扰，也不会出现
 * 「选了下拉里的乐谱，再想切回力度却找不到」的情况。
 */
import { computed } from "vue";
import Md3Select from "@/components/common/Md3Select.vue";
import { useI18n } from "@/i18n";
import type { AutomationTarget } from "@/utils/automation";

const props = defineProps<{
  /** 当前目标键（`velocity` / `pressure` / `channel` / 自动化参数键）。 */
  targetKey: string;
  /** 该轨道可用的自动化目标（外壳已按轨道收窄）。 */
  targets: AutomationTarget[];
}>();

const emit = defineEmits<{
  (e: "update:targetKey", value: string): void;
}>();

const i18n = useI18n();

const options = computed(() => [
  { id: "velocity", label: i18n.t("lane.velocity"), group: i18n.t("auto.group.note") },
  { id: "pressure", label: i18n.t("lane.pressure"), group: i18n.t("auto.group.note") },
  { id: "channel", label: i18n.t("lane.channel"), group: i18n.t("auto.group.note") },
  ...props.targets.map((t) => ({ id: t.key, label: t.label, group: t.group })),
]);

function onChoose(id: string): void {
  emit("update:targetKey", id);
}
</script>

<template>
  <div class="lane-target">
    <Md3Select
      :model-value="targetKey"
      :options="options"
      :label="i18n.t('auto.param')"
      @select="onChoose"
    />
  </div>
</template>

<style scoped>
/* 目标下拉的**唯一**一份样式：控制器泳道和乐谱窗格都通过它渲染，
   所以两处切换时按钮的大小、字号、圆角完全一样。 */
.lane-target {
  flex: 0 0 auto;
  display: inline-flex;
  min-width: 0;
}
.lane-target :deep(.md3-select) {
  width: 120px;
  height: 22px;
  padding: 0 8px 0 10px;
  gap: 6px;
  font-size: 11px;
}
.lane-target :deep(.caret) {
  width: 13px;
  height: 13px;
}
</style>
