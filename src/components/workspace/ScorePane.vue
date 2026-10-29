<script setup lang="ts">
/**
 * 下窗口的乐谱窗格：只画**当前片段**（和上面钢琴卷帘是同一条 MIDI），跟着播放头走。
 *
 * 它和控制器泳道共用同一个「目标」下拉（`LaneTargetSelect`）：选「乐谱」就换成这里，选回去
 * 就回到力度 / 自动化。所以这里不再有自己的「总谱 / 分谱」——下窗口的谱必须和上面的片段
 * 一一对应，整首谱去独立的乐谱视图看。
 */
import { computed } from "vue";
import { XMarkIcon } from "@heroicons/vue/24/outline";
import Md3Select from "@/components/common/Md3Select.vue";
import LaneTargetSelect from "@/components/workspace/LaneTargetSelect.vue";
import { defaultNotationFor, type Notation } from "@/utils/notation";
import ScoreCanvas from "@/components/workspace/ScoreCanvas.vue";
import { useI18n } from "@/i18n";
import { useLayoutStore } from "@/stores/layout";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { barBeatAt } from "@/utils/tempo";
import type { AutomationTarget } from "@/utils/automation";

defineProps<{
  /** 「目标」下拉的当前值（一定是 `score`；这里只是为了让下拉保持选中）。 */
  targetKey: string;
  targets: AutomationTarget[];
}>();

const emit = defineEmits<{
  (e: "update:targetKey", value: string): void;
}>();

const i18n = useI18n();
const layout = useLayoutStore();
const project = useProjectStore();
const selection = useSelectionStore();

/** 表头固定开销（1px 顶边框 + 30px 表头），必须与控制器泳道逐像素一致，否则切换时会跳。 */
const LANE_CHROME = 31;

const clip = computed(() => selection.clip);
const track = computed(() => selection.track);

/** 谱式：跟着**轨道自己的设置**走（和检查器、乐谱视图改的是同一个值）。 */
const notation = computed<Notation>({
  get: () => {
    const t = selection.track;
    if (!t) return "score";
    return layout.trackNotationOf(t.id) ?? defaultNotationFor(t);
  },
  set: (value) => {
    const t = selection.track;
    if (t) layout.setTrackNotation(t.id, value);
  },
});
const notationOptions = computed(() => [
  { id: "score", label: i18n.t("score.notationStaff") },
  { id: "guitar", label: i18n.t("score.notationGuitar") },
  { id: "bass", label: i18n.t("score.notationBass") },
  { id: "drums", label: i18n.t("score.notationDrums") },
]);

/** 只出当前这一小段：片段不一定压在小节线上，`buildMusicXML` 会自动从所在小节起排。 */
const range = computed(() => {
  const c = clip.value;
  return c ? { start: c.start, end: c.start + c.length } : undefined;
});

/**
 * 只出当前轨道。
 *
 * 没有选中轨道时返回**空数组**而不是 `undefined`——`undefined` 在 `buildMusicXML` 里
 * 等于「全部轨道」，下窗口就会莫名其妙显示一张总谱。下窗口永远只出单轨。
 */
const tracks = computed(() => (selection.trackId === null ? [] : [selection.trackId]));

/** 表头里的「轨道 · 第几到第几小节」，让用户知道这段谱对应哪里。 */
const rangeLabel = computed(() => {
  const c = clip.value;
  const transport = project.project?.transport;
  if (!c || !transport) return "";
  const from = barBeatAt(c.start, transport).bar;
  const to = barBeatAt(Math.max(c.start, c.start + c.length - 1e-6), transport).bar;
  return i18n.t("score.bars", { from, to: Math.max(from, to) });
});
</script>

<template>
  <div class="score-block" :style="{ height: `${layout.laneHeight + LANE_CHROME}px` }">
    <div class="lane-head">
      <span class="lane-title">{{ i18n.t("lane.score") }}</span>
      <div class="lane-chooser">
        <LaneTargetSelect
          :target-key="targetKey"
          :targets="targets"
          @update:target-key="emit('update:targetKey', $event)"
        />
      </div>

      <!-- 谱式：和乐谱视图同一套选项 -->
      <div class="notation-chooser">
        <Md3Select v-model="notation" :options="notationOptions" :label="i18n.t('score.notation')" />
      </div>

      <span v-if="track" class="range-label">
        {{ track.name }}<template v-if="rangeLabel"> · {{ rangeLabel }}</template>
      </span>

      <span class="lane-spacer" />

      <button
        class="lane-btn"
        type="button"
        :aria-label="i18n.t('lane.hide')"
        @click="layout.showControllerLane = false"
      >
        <XMarkIcon class="lane-btn-icon" />
      </button>
    </div>

    <ScoreCanvas v-if="clip" :tracks="tracks" :range="range" />
    <p v-else class="score-empty">{{ i18n.t("score.noClip") }}</p>
  </div>
</template>

<style scoped>
.score-block {
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  min-height: 0;
  border-top: 1px solid var(--grid-line-row);
  background-color: var(--app-inset);
}
/* 表头：与控制器泳道同高同序，切过来时眼睛不用重新找 */
.lane-head {
  height: 30px;
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 10px 0 12px;
  border-bottom: 1px solid var(--grid-line-row);
}
.lane-title {
  font-size: 11px;
  letter-spacing: 0.4px;
  color: rgb(var(--md-on-surface-muted));
  flex: 0 0 auto;
}
.lane-chooser {
  flex: 0 0 auto;
}
/* 谱式下拉：和「目标」下拉同高（22px），宽度按四字标签给够。
   两个 wrapper 都必须是 `inline-flex`——块级 wrapper 里的 inline-flex 按钮会带一行行高，
   整体会比左边高出一两个像素，看起来就不齐。 */
.notation-chooser {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  min-width: 0;
}
.notation-chooser :deep(.md3-select) {
  width: 88px;
  height: 22px;
  padding: 0 8px 0 10px;
  gap: 6px;
  font-size: 11px;
}
.notation-chooser :deep(.caret) {
  width: 13px;
  height: 13px;
}
.range-label {
  font-size: 10.5px;
  color: rgb(var(--md-on-surface-muted));
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}
.lane-spacer {
  flex: 1 1 auto;
}
.lane-btn {
  width: 24px;
  height: 24px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
}
.lane-btn:hover {
  background-color: rgb(var(--md-on-surface-variant) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.lane-btn-icon {
  width: 15px;
  height: 15px;
}
.score-empty {
  margin: 24px 0;
  text-align: center;
  font-size: 12.5px;
  color: rgb(var(--md-on-surface-muted));
}
</style>
