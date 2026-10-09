<script setup lang="ts">
/**
 * 内置合成器的**独立窗口**内容。
 *
 * 窗口由后端以 `index.html#/synth?track=N` 打开；这里只渲染一台合成器：面板（面板自带预设垫子与预设条）。
 * 它和主窗口是两个独立的 Vue 实例，靠同一份后端工程状态 + `engine://event` 广播保持一致
 * （在主窗口拖旋钮，这里也会跟着动）。
 */
import { computed, onMounted } from "vue";
import { useRoute } from "vue-router";

import SynthPanel from "@/components/workspace/panes/SynthPanel.vue";
import { canonicalPluginId, effectiveParams, SYNTH_ID } from "@/data/plugins";
import { useI18n } from "@/i18n";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";

const route = useRoute();
const i18n = useI18n();
const store = useProjectStore();
const selection = useSelectionStore();

/**
 * 要显示哪条轨道的合成器。
 *
 * URL 里带了 `track` 就用它；万一丢了（比如窗口 URL 被系统改写），退回第一条挂着内置合成器
 * 的轨道——总比开出一个空白窗口强。
 */
const trackId = computed<number | null>(() => {
  const raw = Number(route.query.track);
  const tracks = store.project?.tracks ?? [];
  if (Number.isFinite(raw) && tracks.some((t) => t.id === raw)) return raw;
  const fallback = tracks.find(
    (t) => canonicalPluginId(t.instrument?.plugin_id ?? "") === SYNTH_ID,
  );
  return fallback?.id ?? null;
});

const track = computed(() => store.project?.tracks.find((t) => t.id === trackId.value) ?? null);

const params = computed(() =>
  effectiveParams(track.value?.instrument?.plugin_id, track.value?.instrument?.params ?? {}),
);

/** 在这个窗口里改参数，也会同步选中主窗口的那条轨道（两边状态一致更不容易懵）。 */
async function commit(id: string, value: number): Promise<void> {
  const t = track.value;
  if (!t) return;
  await store.setParam(t.id, id, value);
}

async function applyPreset(values: Record<string, number>): Promise<void> {
  const t = track.value;
  if (!t) return;
  await store.setParams(t.id, values);
}

function selectTrack(): void {
  if (trackId.value !== null) selection.selectTrack(trackId.value);
}

/**
 * 自己确保工程已经载入。
 *
 * 这个窗口和主窗口各自跑一份应用：主窗口那个 `onMounted` 里的装载链一旦哪一步失败，
 * 这里就会永远停在「没有内置合成器」。所以二级窗口不指望别人，自己再要一次。
 */
onMounted(() => {
  if (!store.project) void store.reload();
});
</script>

<template>
  <div class="synth-window">
    <header class="head">
      <div class="titles">
        <h1 class="title">{{ track ? track.name : "内置合成器" }}</h1>
        <span class="subtitle">内置合成器</span>
      </div>
      <button v-if="track" class="focus-btn" @click="selectTrack">在主窗口选中这条轨道</button>
    </header>

    <div v-if="track" class="body">
      <!-- 预设控件由 SynthPanel 自带（垫子 + 预设条），这里不再单独渲染，避免出现两个预设条。 -->
      <SynthPanel :params="params" :track="track.id" @commit="commit" @apply-preset="applyPreset" />
    </div>

    <p v-else-if="store.loadError" class="empty">
      {{ i18n.t("error.projectLoad", { error: store.loadError }) }}
    </p>
    <p v-else-if="!store.project" class="empty">{{ i18n.t("app.loading") }}</p>
    <p v-else class="empty">{{ i18n.t("synth.windowEmpty") }}</p>
  </div>
</template>

<style scoped>
.synth-window {
  display: flex;
  flex-direction: column;
  height: 100vh;
  overflow: hidden;
  background-color: rgb(var(--md-surface));
  color: rgb(var(--md-on-surface));
}
.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 18px 10px;
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.4);
}
.titles {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
}
.title {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.subtitle {
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
}
.focus-btn {
  flex: 0 0 auto;
  height: 30px;
  padding: 0 12px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-primary));
  font-size: 11px;
  cursor: pointer;
}
.focus-btn:hover {
  background-color: rgb(var(--md-primary) / 0.08);
}
/* 窗口是固定尺寸的（后端 `resizable(false)`），内容按固定尺寸排，不再需要纵向滚动。 */
.body {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  justify-content: center;
  padding: 10px 18px 18px;
}
.empty {
  margin: 40px auto;
  font-size: 12.5px;
  color: rgb(var(--md-on-surface-muted));
}
</style>
