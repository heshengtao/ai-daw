<script setup lang="ts">
/**
 * 内置效果器的**独立窗口**内容。
 *
 * 窗口由后端以 `index.html#/effect?track=N&index=I&plugin=<id>` 打开；这里只渲染那一个效果器。
 *
 * 关键点：效果链是**按序号**存的效果器（没有稳定实例 id），链一变（换顺序 / 被预设整条替换）
 * 序号就可能失效。所以：
 * 1. 每个窗口都在 `App.vue` 里订阅 `engine://event`、节流重拉工程快照（`useProjectSync`），
 *    窗口不会停在旧状态；
 * 2. 定位目标时**优先按 URL 里的 plugin id 在链里找回自己**（换顺序也能跟上），序号只作兜底；
 * 3. 如果这个效果器**已经不在链上了**（被删掉 / 被预设换掉），窗口**自己关掉**，
 *    而不是显示「这条轨道上没有这个效果器」。
 */
import { computed, ref, watch } from "vue";
import { useRoute } from "vue-router";
import { getCurrentWindow } from "@tauri-apps/api/window";

import EffectPanel from "@/components/workspace/panes/EffectPanel.vue";
import { useI18n } from "@/i18n";
import { usePluginsStore } from "@/stores/plugins";
import { useProjectStore } from "@/stores/project";
import { isTauri } from "@/utils/platform";
import type { PluginInstance } from "@/api/types";

const route = useRoute();
const i18n = useI18n();
const store = useProjectStore();
const plugins = usePluginsStore();

/** 要显示哪条轨道的效果器。 */
const trackId = computed(() => {
  const raw = Number(route.query.track);
  const tracks = store.project?.tracks ?? [];
  return Number.isFinite(raw) && tracks.some((t) => t.id === raw) ? raw : null;
});

/** URL 里带的序号（兜底用）。 */
const urlIndex = computed(() => {
  const raw = Number(route.query.index);
  return Number.isFinite(raw) && raw >= 0 ? Math.floor(raw) : null;
});

/** URL 里带的效果器 plugin id（首选定位依据）。 */
const wantPlugin = computed(() => {
  const raw = route.query.plugin;
  return typeof raw === "string" && raw.length > 0 ? raw : null;
});

const track = computed(() => store.project?.tracks.find((t) => t.id === trackId.value) ?? null);

/** 解析出目标效果器与它在**当前**链里的真实序号。 */
const resolved = computed<{ fx: PluginInstance; index: number } | null>(() => {
  const t = track.value;
  if (!t) return null;
  const want = wantPlugin.value;
  if (want) {
    // 先看原序号是否还对得上（顺序没变时最省事）。
    const i = urlIndex.value;
    if (i !== null && t.effects[i]?.plugin_id === want) return { fx: t.effects[i], index: i };
    // 否则在链里按 id 找回自己（换顺序 / 前面插删过效果器）。
    const j = t.effects.findIndex((f) => f.plugin_id === want);
    // 带 id 却找不到 → 视为「已被移出链」，返回 null（窗口据此自行关闭），
    // 绝不退而显示序号上碰巧存在的**另一个**效果器。
    return j >= 0 ? { fx: t.effects[j], index: j } : null;
  }
  // 老 URL（没有 plugin 参数）：只能按序号定位。
  const i = urlIndex.value;
  return i !== null && t.effects[i] ? { fx: t.effects[i], index: i } : null;
});

const fx = computed(() => resolved.value?.fx ?? null);
const title = computed(() => (fx.value ? plugins.name(fx.value.plugin_id) : ""));

/** 是否曾经成功定位到效果器（用来区分「还没载入」与「真的没了」）。 */
const everResolved = ref(false);
watch(resolved, (r) => {
  if (r) {
    everResolved.value = true;
    return;
  }
  // 曾经有、现在没了 → 效果器已被移出链（删掉 / 被预设换掉）：自己关掉窗口。
  if (everResolved.value && store.project && isTauri()) {
    void getCurrentWindow().close();
  }
});

/** 参数改动只写后端工程状态：两个窗口都靠 `engine://event` 广播保持一致。 */
async function commit(id: string, value: number): Promise<void> {
  const t = track.value;
  const i = resolved.value?.index ?? null;
  if (!t || i === null) return;
  await store.setEffectParam(t.id, i, id, value);
}

/** 恢复默认值：一次事务，只占一个撤销步骤。 */
async function commitMany(params: Record<string, number>): Promise<void> {
  const t = track.value;
  const i = resolved.value?.index ?? null;
  if (!t || i === null) return;
  await store.setEffectParams(t.id, i, params);
}
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
