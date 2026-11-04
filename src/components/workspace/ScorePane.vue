<script setup lang="ts">
/**
 * 下窗口的乐谱窗格。
 *
 * - 编辑视图：只画**当前片段**（和上面钢琴卷帘是同一条 MIDI），跟着播放头走；
 * - 编排视图：画**选中轨道**里所有带音符的片段（用户没选轨道 / 轨道只有音频时显示占位文字）。
 *
 * 视图切换器（编排：自动化 / 乐谱 / 键盘；编辑：控制器 / 乐谱 / 键盘）由 `LaneViewSwitch`
 * 统一画在表头最左侧；参数下拉只留在控制器 / 自动化窗格里。
 */
import { computed } from "vue";
import { XMarkIcon } from "@heroicons/vue/24/outline";
import Md3Select from "@/components/common/Md3Select.vue";
import LaneViewSwitch from "@/components/workspace/LaneViewSwitch.vue";
import { defaultNotationFor, type Notation } from "@/utils/notation";
import ScoreCanvas from "@/components/workspace/ScoreCanvas.vue";
import { useI18n } from "@/i18n";
import {
  ARRANGE_LANE_VIEWS,
  EDIT_LANE_VIEWS,
  useLayoutStore,
  type LaneView,
} from "@/stores/layout";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { barBeatAt } from "@/utils/tempo";

const props = withDefaults(
  defineProps<{
    /**
     * 宿主：编辑视图只画当前片段；编排视图画**选中轨道**的整段内容。
     */
    context?: "edit" | "arrange";
  }>(),
  { context: "edit" },
);

const emit = defineEmits<{
  /** 视图切换器：控制器 / 自动化 / 乐谱 / MIDI 键盘（由宿主决定可用项）。 */
  (e: "update:view", value: LaneView): void;
}>();

const isArrange = computed(() => props.context === "arrange");
/** 这个宿主能切哪几格：编排是「自动化 / 乐谱 / 键盘」，编辑是「控制器 / 乐谱 / 键盘」。 */
const views = computed(() => (isArrange.value ? ARRANGE_LANE_VIEWS : EDIT_LANE_VIEWS));

/** 收起下窗口：编排视图关 `showAutomation`，编辑视图关 `showControllerLane`。 */
function hidePane(): void {
  if (isArrange.value) layout.showAutomation = false;
  else layout.showControllerLane = false;
}

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

/** 编排视图里「选中轨道的内容范围」：第一颗有音符的片段到最后一颗，整首范围太大时更好读。 */
const arrangeRange = computed(() => {
  const t = selection.track;
  if (!t) return null;
  const clips = t.clips.filter((c) => c.notes.length > 0);
  if (clips.length === 0) return null;
  const start = Math.min(...clips.map((c) => c.start));
  const end = Math.max(...clips.map((c) => c.start + c.length));
  return { start, end };
});

/**
 * 编辑视图只出当前片段；编排视图出**选中轨道**里所有带音符的片段。
 * 片段不一定压在小节线上，`buildMusicXML` 会自动从所在小节起排。
 */
const range = computed(() => {
  if (isArrange.value) return arrangeRange.value ?? undefined;
  const c = clip.value;
  return c ? { start: c.start, end: c.start + c.length } : undefined;
});

/** 有没有可画的内容：编辑视图看片段，编排视图看选中轨道里有没有音符。 */
const hasScore = computed(() => (isArrange.value ? arrangeRange.value !== null : clip.value !== null));

/**
 * 只出当前轨道。
 *
 * 没有选中轨道时返回**空数组**而不是 `undefined`——`undefined` 在 `buildMusicXML` 里
 * 等于「全部轨道」，下窗口就会莫名其妙显示一张总谱。下窗口永远只出单轨。
 */
const tracks = computed(() => (selection.track ? [selection.track.id] : []));

/** 表头里的「轨道 · 第几到第几小节」，让用户知道这段谱对应哪里。 */
const rangeLabel = computed(() => {
  const r = range.value;
  const transport = project.project?.transport;
  if (!r || !transport) return "";
  const from = barBeatAt(r.start, transport).bar;
  const to = barBeatAt(Math.max(r.start, r.end - 1e-6), transport).bar;
  return i18n.t("score.bars", { from, to: Math.max(from, to) });
});
</script>

<template>
  <div class="score-block" :style="{ height: `${layout.laneHeight + LANE_CHROME}px` }">
    <div class="lane-head">
      <LaneViewSwitch
        :model-value="'score'"
        :views="views"
        @update:model-value="emit('update:view', $event)"
      />

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
        @click="hidePane"
      >
        <XMarkIcon class="lane-btn-icon" />
      </button>
    </div>

    <ScoreCanvas v-if="hasScore" :tracks="tracks" :range="range" />
    <p v-else class="score-empty">
      {{ i18n.t(isArrange ? (selection.track ? "score.empty" : "score.noTrack") : "score.noClip") }}
    </p>
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
  margin: auto;
  padding: 0 12px;
  text-align: center;
  font-size: 12.5px;
  color: rgb(var(--md-on-surface-muted));
}
</style>
