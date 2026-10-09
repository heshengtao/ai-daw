<script setup lang="ts">
/**
 * 插件界面：把当前轨道音色的编辑区放进右侧面板（一个独立标签页）。
 *
 * - **内置合成器**：直接由它自报的参数表渲染控件——这就是内置音色的「界面」。
 * - **外部插件（VST3 / CLAP）**：同样渲染参数表。插件**自带的原生界面**是一个原生窗口，
 *   没法嵌进网页视图里，因此这里给参数面板；原生窗口接入后会在这里提供「打开界面」入口。
 *
 * 之所以占一个标签页（而不是塞进检查器）：复杂插件参数动辄几十上百个，
 * 侧边栏可以拖到很宽，正好用来慢慢调。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import PluginParamList from "@/components/common/PluginParamList.vue";
import PresetBar from "@/components/common/PresetBar.vue";
import SynthPanel from "@/components/workspace/panes/SynthPanel.vue";
import { canonicalPluginId, effectiveParams, SYNTH_ID } from "@/data/plugins";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { useLayoutStore } from "@/stores/layout";
import { usePluginsStore } from "@/stores/plugins";
import { useI18n } from "@/i18n";
import { closePluginWindow, openPluginWindow, pluginWindowOpen } from "@/api/tauri";
import type { EffectSpec } from "@/api/types";

const store = useProjectStore();
const selection = useSelectionStore();
const layout = useLayoutStore();
const plugins = usePluginsStore();

const track = computed(() => selection.track);
const i18n = useI18n();
const info = computed(() => plugins.info(track.value?.instrument?.plugin_id));

/** 当前音色是不是内置合成器（老工程里的预设 id 也算）。 */
const isBuiltinSynth = computed(
  () => canonicalPluginId(track.value?.instrument?.plugin_id ?? "") === SYNTH_ID,
);

/** 面板上显示的参数：老预设音色把出厂补丁摊平后再叠加工程里存过的值。 */
const instrumentParams = computed(() =>
  effectiveParams(track.value?.instrument?.plugin_id, track.value?.instrument?.params ?? {}),
);

/** 该插件的原生界面是否开着（独立窗口）。 */
const editorOpen = ref(false);
let statusTimer: number | null = null;

async function refreshEditorOpen(): Promise<void> {
  const t = track.value;
  if (!t) {
    editorOpen.value = false;
    return;
  }
  try {
    editorOpen.value = await pluginWindowOpen(t.id);
  } catch {
    editorOpen.value = false;
  }
}

async function toggleEditorWindow(): Promise<void> {
  const t = track.value;
  if (!t) return;
  try {
    if (editorOpen.value) {
      await closePluginWindow(t.id);
      editorOpen.value = false;
    } else {
      await openPluginWindow(t.id);
      editorOpen.value = true;
    }
  } catch (err) {
    layout.notify(`插件界面：${String(err)}`);
  }
}

onMounted(() => {
  void refreshEditorOpen();
  // 用户可能自己关掉那个窗口，定期对齐一下状态。
  statusTimer = window.setInterval(() => void refreshEditorOpen(), 1000);
});
onUnmounted(() => {
  if (statusTimer !== null) window.clearInterval(statusTimer);
});

/** 参数表懒加载：乐器 + 链上每个效果器。 */
watch(
  () =>
    [track.value?.instrument?.plugin_id, ...(track.value?.effects ?? []).map((f) => f.plugin_id)].join(
      "|",
    ),
  () => {
    void plugins.ensureParams(track.value?.instrument?.plugin_id);
    for (const fx of track.value?.effects ?? []) void plugins.ensureParams(fx.plugin_id);
  },
  { immediate: true },
);

async function commitInstrument(id: string, value: number): Promise<void> {
  const t = track.value;
  if (!t) return;
  try {
    await store.setParam(t.id, id, value);
  } catch (err) {
    layout.notify(i18n.t("error.setParam", { error: String(err) }));
  }
}

/** 包络控制点这类「一次改多个参数」的提交：必须合成**一个**撤销步骤。 */
async function commitInstrumentMany(params: Record<string, number>): Promise<void> {
  const t = track.value;
  if (!t) return;
  try {
    await store.setParams(t.id, params);
  } catch (err) {
    layout.notify(i18n.t("error.setParam", { error: String(err) }));
  }
}

/** 切换预设：整组参数 + 自带的效果链，**一次事务**（只占一个撤销步骤）。 */
async function applyPreset(
  params: Record<string, number>,
  effects: EffectSpec[],
): Promise<void> {
  const t = track.value;
  if (!t) return;
  try {
    await store.applyPreset(t.id, params, effects);
  } catch (err) {
    layout.notify(i18n.t("error.applyPreset", { error: String(err) }));
  }
}

async function commitEffect(index: number, id: string, value: number): Promise<void> {
  const t = track.value;
  if (!t) return;
  try {
    await store.setEffectParam(t.id, index, id, value);
  } catch (err) {
    layout.notify(i18n.t("error.setEffectParam", { error: String(err) }));
  }
}

/** 「恢复默认值」这类一次改多个：后端合成一次事务，只占一个撤销步骤。 */
async function commitEffectMany(index: number, params: Record<string, number>): Promise<void> {
  const t = track.value;
  if (!t) return;
  try {
    await store.setEffectParams(t.id, index, params);
  } catch (err) {
    layout.notify(i18n.t("error.setEffectParam", { error: String(err) }));
  }
}
</script>

<template>
  <div class="pane">
    <template v-if="track">
      <section v-if="track.instrument" class="group">
        <div class="group-head">
          <span class="group-title">{{ i18n.t("editor.timbre") }}</span>
          <span v-if="info && !info.builtin && info.format" class="badge">
            {{ info.format.toUpperCase() }}
          </span>
          <span v-if="info && !info.builtin" class="badge muted">
            {{ info.hasEditor ? i18n.t("editor.hasNative") : i18n.t("editor.noEditor") }}
          </span>
        </div>

        <div class="card">
          <div class="name-row">
            <span class="plugin-name">{{ plugins.name(track.instrument.plugin_id) }}</span>
            <button
              v-if="info && !info.builtin && info.hasEditor"
              class="editor-btn"
              :class="{ on: editorOpen }"
              @click="toggleEditorWindow"
            >
              {{ editorOpen ? i18n.t("editor.closeWindow") : i18n.t("editor.openWindow") }}
            </button>
          </div>
          <!-- 内置合成器的预设控件在面板内部（垫子 + 预设条）；只有外部插件在这里用共用预设条。 -->
          <PresetBar
            v-if="!isBuiltinSynth"
            :plugin-id="track.instrument.plugin_id"
            :params="instrumentParams"
            @apply="applyPreset"
          />
          <!-- 内置合成器有完整的合成器面板；其余插件渲染参数表。 -->
          <SynthPanel
            v-if="isBuiltinSynth"
            :params="instrumentParams"
            :track="track.id"
            :effects="track.effects"
            @commit="commitInstrument"
            @commit-many="commitInstrumentMany"
            @apply-preset="applyPreset"
          />
          <PluginParamList
            v-else
            :plugin-id="track.instrument.plugin_id"
            :values="track.instrument.params"
            resettable
            @commit="commitInstrument"
            @commit-many="commitInstrumentMany"
          />
        </div>

        <p v-if="info && !info.builtin" class="hint">{{ i18n.t("editor.nativeHint") }}</p>
      </section>

      <section v-if="track.effects.length > 0" class="group">
        <div class="group-title">{{ i18n.t("inspector.fxChain") }}</div>
        <div v-for="(fx, i) in track.effects" :key="i" class="card">
          <div class="plugin-name">{{ i + 1 }} · {{ plugins.name(fx.plugin_id) }}</div>
          <PluginParamList
            :plugin-id="fx.plugin_id"
            :values="fx.params"
            resettable
            @commit="(id: string, v: number) => commitEffect(i, id, v)"
            @commit-many="(p: Record<string, number>) => commitEffectMany(i, p)"
          />
        </div>
      </section>

      <p v-if="!track.instrument" class="hint">{{ i18n.t("editor.noInstrument") }}</p>
    </template>

    <p v-else class="hint">{{ i18n.t("editor.selectTrack") }}</p>
  </div>
</template>

<style scoped>
.pane {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 12px;
  gap: 14px;
}
.group {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.group-head {
  display: flex;
  align-items: center;
  gap: 6px;
  padding-left: 4px;
}
.group-title {
  font-size: 10.5px;
  letter-spacing: 0.8px;
  text-transform: uppercase;
  color: rgb(var(--md-on-surface-variant));
}
.badge {
  font-size: 9.5px;
  padding: 2px 7px;
  border-radius: 999px;
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
  letter-spacing: 0.4px;
}
.badge.muted {
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface-muted));
}
.card {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 8px;
  border-radius: 16px;
  background-color: var(--app-inset);
}
.name-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 2px 10px 4px;
}
.plugin-name {
  flex: 1 1 auto;
  min-width: 0;
  font-size: 12.5px;
  font-weight: 600;
  color: rgb(var(--md-on-surface));
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
/* 打开 / 关闭插件的原生窗口 */
.editor-btn {
  flex: 0 0 auto;
  height: 26px;
  padding: 0 12px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-primary));
  font-size: 11.5px;
  cursor: pointer;
  white-space: nowrap;
  transition: background-color 0.14s ease;
}
.editor-btn:hover {
  background-color: rgb(var(--md-primary) / 0.08);
}
.editor-btn.on {
  background-color: rgb(var(--md-secondary-container));
  border-color: transparent;
  color: rgb(var(--md-on-secondary-container));
}
.hint {
  margin: 0 8px;
  font-size: 11px;
  line-height: 1.6;
  color: rgb(var(--md-on-surface-muted));
}
</style>
