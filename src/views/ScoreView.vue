<script setup lang="ts">
/**
 * 乐谱视图（顶栏「乐谱」，夹在编辑与混音之间）：像 Guitar Pro 那样整首看谱。
 *
 * 谱式四选一：
 * - 五线谱 / 鼓谱 / 吉他谱 / 贝斯谱：全部由 alphaTab 渲染（我们的 MusicXML 里带
 *   `<technical><string>/<fret>`，所以六线谱是引擎排的，不是手画的）。
 *
 * 谱式是「一条轨道」的事，所以选了六线谱 / 鼓谱会自动把范围切到单条轨道；
 * 反过来，切回总谱时谱式会自动落回五线谱（总谱里不可能全是鼓或全是六线谱）。
 */
import { computed, ref } from "vue";
import Md3Select from "@/components/common/Md3Select.vue";
import ScoreCanvas from "@/components/workspace/ScoreCanvas.vue";
import NotationSettingsDialog from "@/components/workspace/NotationSettingsDialog.vue";
import { useI18n } from "@/i18n";
import { useSelectionStore } from "@/stores/selection";
import { useLayoutStore } from "@/stores/layout";
import { defaultNotationFor, type Notation } from "@/utils/notation";

const i18n = useI18n();
const selection = useSelectionStore();
const layout = useLayoutStore();

/**
 * 谱面范围：轨道 / 总谱。
 *
 * 「哪条轨道」不在这里选：交给左侧音轨栏（点轨道头就切），所以这份下拉只表达**范围**。
 * 默认「总谱」——进乐谱页第一眼就该看到整首。
 */
const scope = ref<"track" | "all">("all");

const scopeOptions = computed(() => [
  { id: "track", label: i18n.t("edit.scopeTrack") },
  { id: "all", label: i18n.t("edit.scopeAll") },
]);

const notationOptions = computed(() => [
  { id: "score", label: i18n.t("score.notationStaff") },
  { id: "guitar", label: i18n.t("score.notationGuitar") },
  { id: "bass", label: i18n.t("score.notationBass") },
  { id: "drums", label: i18n.t("score.notationDrums") },
]);

/** 当前选中的轨道（左侧音轨栏决定）。 */
const trackId = computed(() => selection.trackId);

/** 只出这些轨道：轨道模式只出当前那条；没有选中轨道时退回总谱。 */
const tracks = computed(() => {
  if (scope.value === "all" || trackId.value === null) return undefined;
  return [trackId.value];
});

/**
 * 当前轨道的谱式（和检查器里改的是同一个值，存在本机的显示偏好里）。
 *
 * **总谱里不提供**整体切换：每条轨道按自己的谱式排（吉他带六线谱、鼓用打击乐谱表），
 * 一刀切反而会毁掉这种混合排版。
 */
const notation = computed<Notation>({
  get: () => {
    const track = selection.track;
    if (!track) return "score";
    return layout.trackNotationOf(track.id) ?? defaultNotationFor(track);
  },
  set: (value) => {
    const track = selection.track;
    if (track) layout.setTrackNotation(track.id, value);
  },
});

const isTab = computed(() => notation.value === "guitar" || notation.value === "bass");

/** 双击谱面打开的「记谱设置」（谱号 / 调弦）。 */
const notationOpen = ref(false);

/** 表头里的当前轨道名（范围是总谱时为空）：让人一眼确认在看哪条轨。 */
const trackLabel = computed(() => (scope.value === "all" ? "" : (selection.track?.name ?? "")));
</script>

<template>
  <div class="score-view">
    <header class="head">
      <h1 class="title">{{ i18n.t("score.title") }}</h1>

      <div class="field">
        <span class="field-label">{{ i18n.t("score.scope") }}</span>
        <Md3Select v-model="scope" :options="scopeOptions" :label="i18n.t('score.scope')" />
      </div>

      <!-- 谱式只在「轨道」范围下可改；总谱里每条轨道按自己的谱式排 -->
      <div v-if="scope === 'track'" class="field">
        <span class="field-label">{{ i18n.t("score.notation") }}</span>
        <Md3Select v-model="notation" :options="notationOptions" :label="i18n.t('score.notation')" />
      </div>

      <span class="hint">
        <template v-if="trackLabel">{{ trackLabel }} · </template>{{ isTab ? i18n.t("score.tabHint") : i18n.t("score.readonlyHint") }}
      </span>
      <span class="spacer" />
    </header>

    <ScoreCanvas :tracks="tracks" @edit-notation="notationOpen = true" />

    <NotationSettingsDialog v-model:open="notationOpen" :track-id="selection.trackId" />
  </div>
</template>

<style scoped>
.score-view {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  /* 与 App 其他工作区同一个页面底色 */
  background-color: var(--app-bg);
}
.head {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 10px 16px;
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.4);
}
.title {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
  letter-spacing: -0.1px;
  color: rgb(var(--md-on-surface));
}
.field {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.field-label {
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
  /* 两个字的标签绝不能被挤成两行 */
  white-space: nowrap;
  flex: 0 0 auto;
}
.hint {
  font-size: 11px;
  color: rgb(var(--md-on-surface-muted));
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.spacer {
  flex: 1 1 auto;
}
</style>
