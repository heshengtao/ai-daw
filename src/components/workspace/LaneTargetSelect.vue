<script setup lang="ts">
/**
 * 底部窗格的「目标」下拉：力度 / 压力 / 通道 / 自动化参数 / 乐谱。
 *
 * 为什么单独抽出来：现在有**两个宿主**——控制器泳道（`ControllerLane`）和乐谱窗格
 * （`ScorePane`）。选「乐谱」会把整个窗格换掉，所以两边必须是同一个下拉，否则用户切过去
 * 就再也切不回来了。
 *
 * 「乐谱」不是自动化参数，单独成一组（`auto.group.view`），免得和插件参数混在一起。
 */
import { computed } from "vue";
import Md3Select from "@/components/common/Md3Select.vue";
import { useI18n } from "@/i18n";
import type { AutomationTarget } from "@/utils/automation";

const props = defineProps<{
  /** 当前目标键（`velocity` / `pressure` / `channel` / 自动化键 / `score`）。 */
  targetKey: string;
  /** 该轨道可用的自动化目标（外壳已按轨道收窄）。 */
  targets: AutomationTarget[];
}>();

const emit = defineEmits<{
  (e: "update:targetKey", value: string): void;
}>();

const i18n = useI18n();

/**
 * 「乐谱」这一项的 id。
 *
 * 两个宿主都写这个字面量（`ControllerLane` 不处理它，`EditView` 用它切换窗格内容）；
 * 常量放在 `.ts` 里更规范，但这里只有一处用到，没必要为一行字符串单开模块。
 */
const SCORE_TARGET = "score";

const options = computed(() => [
  // 「乐谱」放最前面：它是视图切换（把整个窗格换掉），不是参数，混在音符属性里容易找不到。
  { id: SCORE_TARGET, label: i18n.t("lane.score"), group: i18n.t("auto.group.view") },
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
      :label="i18n.t('lane.property')"
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
