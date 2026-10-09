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
import type { EffectSpec } from "@/api/types";
import { canonicalPluginId, effectiveParams, SYNTH_ID } from "@/data/plugins";
import { useI18n } from "@/i18n";
import { useProjectStore } from "@/stores/project";

const route = useRoute();
const i18n = useI18n();
const store = useProjectStore();

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

/** 参数改动只写后端工程状态：两个窗口都靠 `engine://event` 广播保持一致。 */
async function commit(id: string, value: number): Promise<void> {
  const t = track.value;
  if (!t) return;
  await store.setParam(t.id, id, value);
}

/** 包络控制点这类「一次改多个参数」的提交：必须合成**一个**撤销步骤。 */
async function commitMany(params: Record<string, number>): Promise<void> {
  const t = track.value;
  if (!t) return;
  await store.setParams(t.id, params);
}

/** 切换预设：整组参数 + 自带的效果链，**一次事务**（只占一个撤销步骤）。 */
async function applyPreset(
  values: Record<string, number>,
  effects: EffectSpec[],
): Promise<void> {
  const t = track.value;
  if (!t) return;
  await store.applyPreset(t.id, values, effects);
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
        <h1 class="title">{{ track ? track.name : i18n.t("synth.windowTitle") }}</h1>
        <span class="subtitle">{{ i18n.t("synth.windowSubtitle") }}</span>
      </div>
    </header>

    <div v-if="track" class="body">
      <!-- 预设控件由 SynthPanel 自带（垫子 + 预设条），这里不再单独渲染，避免出现两个预设条。 -->
      <SynthPanel
        :params="params"
        :track="track.id"
        :effects="track.effects"
        @commit="commit"
        @commit-many="commitMany"
        @apply-preset="applyPreset"
      />
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
