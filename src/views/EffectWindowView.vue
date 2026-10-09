<script setup lang="ts">
/**
 * 内置效果器的**独立窗口**内容。
 *
 * 窗口由后端以 `index.html#/effect?track=N&index=I` 打开；这里只渲染那一个效果器的面板。
 * 和主窗口是两个独立的 Vue 实例，靠同一份后端工程状态 + `engine://event` 广播保持一致
 * （在检查器里换顺序、在主窗口里改参数，这里都会跟着动）。
 */
import { computed, onMounted } from "vue";
import { useRoute } from "vue-router";

import EffectPanel from "@/components/workspace/panes/EffectPanel.vue";
import { useI18n } from "@/i18n";
import { usePluginsStore } from "@/stores/plugins";
import { useProjectStore } from "@/stores/project";

const route = useRoute();
const i18n = useI18n();
const store = useProjectStore();
const plugins = usePluginsStore();

/** 要显示哪条轨道的第几个效果器。 */
const trackId = computed(() => {
  const raw = Number(route.query.track);
  const tracks = store.project?.tracks ?? [];
  return Number.isFinite(raw) && tracks.some((t) => t.id === raw) ? raw : null;
});

const index = computed(() => {
  const raw = Number(route.query.index);
  return Number.isFinite(raw) && raw >= 0 ? Math.floor(raw) : null;
});

const track = computed(() => store.project?.tracks.find((t) => t.id === trackId.value) ?? null);

/** 效果器实例：链被改动（换顺序 / 删掉）后这里的下标可能已经指向别人了，所以每次现取。 */
const fx = computed(() => {
  const i = index.value;
  if (i === null) return null;
  return track.value?.effects[i] ?? null;
});

const title = computed(() => (fx.value ? plugins.name(fx.value.plugin_id) : ""));

/** 参数改动只写后端工程状态：两个窗口都靠 `engine://event` 广播保持一致。 */
async function commit(id: string, value: number): Promise<void> {
  const t = track.value;
  const i = index.value;
  if (!t || i === null) return;
  await store.setEffectParam(t.id, i, id, value);
}

/** 恢复默认值：一次事务，只占一个撤销步骤。 */
async function commitMany(params: Record<string, number>): Promise<void> {
  const t = track.value;
  const i = index.value;
  if (!t || i === null) return;
  await store.setEffectParams(t.id, i, params);
}

/**
 * 自己确保工程已经载入：二级窗口不指望主窗口的装载链成功。
 */
onMounted(() => {
  if (!store.project) void store.reload();
});
</script>

<template>
  <div class="fx-window">
    <header v-if="fx" class="head">
      <h1 class="title">{{ title }}</h1>
      <span v-if="track" class="subtitle">{{ track.name }}</span>
    </header>

    <EffectPanel
      v-if="fx"
      :plugin-id="fx.plugin_id"
      :values="fx.params"
      @commit="commit"
      @commit-many="commitMany"
    />

    <p v-else-if="store.loadError" class="empty">
      {{ i18n.t("error.projectLoad", { error: store.loadError }) }}
    </p>
    <p v-else-if="!store.project" class="empty">{{ i18n.t("app.loading") }}</p>
    <p v-else class="empty">{{ i18n.t("effect.windowEmpty") }}</p>
  </div>
</template>

<style scoped>
/* 面板铺满整个窗口：不留外框、不留大缝隙。 */
.fx-window {
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
  padding: 10px 14px 0;
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
.empty {
  margin: 40px auto;
  font-size: 12.5px;
  color: rgb(var(--md-on-surface-muted));
}
</style>
