<script setup lang="ts">
/**
 * 内置合成器的**独立窗口**内容。
 *
 * 窗口由后端以 `index.html#/synth?track=N` 打开；这里只渲染一台合成器：面板（面板自带预设垫子与预设条）。
 * 它和主窗口是两个独立的 Vue 实例，靠同一份后端工程状态 + `engine://event` 广播保持一致
 * （在主窗口拖旋钮，这里也会跟着动）。
 */
import { computed } from "vue";
import { useRoute } from "vue-router";

import SynthPanel from "@/components/workspace/panes/SynthPanel.vue";
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

/**
 * 切换预设：整组**乐器参数**一次写回（只占一个撤销步骤）。
 *
 * **不碰效果链**——音色与效果是解耦的，切音色不该动轨道上用户自己搭的效果器。
 */
async function applyPreset(values: Record<string, number>): Promise<void> {
  const t = track.value;
  if (!t) return;
  await store.setParams(t.id, values);
}

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
  flex: 0 0 auto;
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 8px 12px 6px;
  min-width: 0;
}
.titles {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
}
.title {
  margin: 0;
  font-size: 13px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.subtitle {
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
}
/* 面板自己铺满：这里**不留内边距**——键盘要贴死窗口的左、右、下三边。 */
.body {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
}
.empty {
  margin: 40px auto;
  font-size: 12.5px;
  color: rgb(var(--md-on-surface-muted));
}
</style>
