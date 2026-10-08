<script setup lang="ts">
/**
 * 左侧音轨栏：全高面板，顶部品牌区与顶栏同高，与顶栏融为一体（不是浮起的圆角卡片）。
 * 表头 40px、行高 64px 且行间无缝隙——与编排网格的标尺/lane 严格对齐；
 * 纵向滚动通过 laneScrollTop 与编排视图双向同步。
 *
 * 每行可：选中（驱动检查器与钢琴卷帘）、静音 / 独奏、删除。
 */
import { ref, watch } from "vue";
import { PlusIcon, TrashIcon } from "@heroicons/vue/24/outline";
import Logo from "@/components/common/Logo.vue";
import Tooltip from "@/components/common/Tooltip.vue";
import PanelIcon from "@/components/common/PanelIcon.vue";
import { useProjectStore } from "@/stores/project";
import { useLayoutStore } from "@/stores/layout";
import { useSelectionStore } from "@/stores/selection";
import { trackHue } from "@/utils/trackColor";
import { usePluginsStore } from "@/stores/plugins";
import { useI18n } from "@/i18n";
import { laneScrollTop } from "@/composables/useLaneScroll";
import { isMacOS } from "@/utils/platform";

const store = useProjectStore();
const layout = useLayoutStore();
const selection = useSelectionStore();
const plugins = usePluginsStore();
const i18n = useI18n();

const listEl = ref<HTMLElement | null>(null);
let applying = false;

const isMac = isMacOS();

function notesOf(trackId: number): number {
  const track = store.project?.tracks.find((t) => t.id === trackId);
  return (track?.clips ?? []).reduce((n, c) => n + c.notes.length, 0);
}

function addTrack(): void {
  const next = (store.project?.tracks.length ?? 0) + 1;
  void store.addTrack(`轨道 ${next}`);
}

async function removeTrack(id: number, name: string): Promise<void> {
  try {
    await store.removeTrack(id);
    layout.notify(i18n.t("tracks.removed", { name }));
  } catch (err) {
    layout.notify(i18n.t("error.removeTrack", { error: String(err) }));
  }
}

async function toggleMute(id: number, mute: boolean): Promise<void> {
  try {
    await store.setMute(id, !mute);
  } catch (err) {
    layout.notify(i18n.t("error.toggleMute", { error: String(err) }));
  }
}

async function toggleSolo(id: number, solo: boolean): Promise<void> {
  try {
    await store.setSolo(id, !solo);
  } catch (err) {
    layout.notify(i18n.t("error.toggleSolo", { error: String(err) }));
  }
}

/** 自己滚动时广播位置（避免回声）。 */
function onScroll(): void {
  const el = listEl.value;
  if (!el || applying) return;
  laneScrollTop.value = el.scrollTop;
}

// 编排视图滚动时跟随。
watch(laneScrollTop, (v) => {
  const el = listEl.value;
  if (!el || Math.abs(el.scrollTop - v) < 1) return;
  applying = true;
  el.scrollTop = v;
  requestAnimationFrame(() => {
    applying = false;
  });
});
</script>

<template>
  <div class="track-panel">
    <!-- 品牌区：与顶栏同高；macOS 下折叠按钮紧随红绿灯之后 -->
    <div class="brand" :class="{ 'pad-mac': isMac }">
      <Tooltip :text="i18n.t('tracks.hide')" align="end">
        <button class="brand-toggle" :aria-label="i18n.t('tracks.hideLabel')" @click="layout.showTrackHeaders = false">
          <PanelIcon side="left" :filled="true" class="brand-toggle-icon" />
        </button>
      </Tooltip>
      <Logo class="brand-logo" />
      <span class="brand-name">daw-party</span>
    </div>

    <header class="heads-head">
      <span class="text-label-sm">{{ i18n.t("tracks.title") }}</span>
      <Tooltip :text="i18n.t('tracks.add')" align="end">
        <button class="btn-icon" :aria-label="i18n.t('tracks.add')" @click="addTrack">
          <PlusIcon class="w-4 h-4" />
        </button>
      </Tooltip>
    </header>

    <ul ref="listEl" class="heads-list" @scroll="onScroll">
      <li
        v-for="(track, ti) in store.project?.tracks ?? []"
        :key="track.id"
        class="track-row"
        :class="{ selected: track.id === selection.trackId, dimmed: track.mixer.mute }"
        :style="{ '--track-hue': String(trackHue(ti)) }"
        @click="selection.selectTrack(track.id)"
      >
        <span class="grip" />
        <div class="track-meta">
          <span class="track-name text-title-sm">{{ track.name }}</span>
          <span class="track-sub text-label-sm">
            {{ plugins.name(track.instrument?.plugin_id) }} · {{ notesOf(track.id) }} 音符
          </span>
        </div>

        <div class="row-actions">
          <Tooltip :text="track.mixer.mute ? i18n.t('tracks.unmute') : i18n.t('tracks.mute')">
            <button
              class="row-btn mini"
              :class="{ on: track.mixer.mute }"
              :aria-pressed="track.mixer.mute"
              :aria-label="i18n.t('tracks.muteNamed', { name: track.name })"
              @click.stop="toggleMute(track.id, track.mixer.mute)"
            >
              M
            </button>
          </Tooltip>
          <Tooltip :text="track.mixer.solo ? i18n.t('tracks.unsolo') : i18n.t('tracks.solo')">
            <button
              class="row-btn mini solo"
              :class="{ on: track.mixer.solo }"
              :aria-pressed="track.mixer.solo"
              :aria-label="i18n.t('tracks.soloNamed', { name: track.name })"
              @click.stop="toggleSolo(track.id, track.mixer.solo)"
            >
              S
            </button>
          </Tooltip>
          <Tooltip :text="i18n.t('tracks.remove')">
            <button
              class="row-del"
              :aria-label="i18n.t('tracks.removeNamed', { name: track.name })"
              @click.stop="removeTrack(track.id, track.name)"
            >
              <TrashIcon class="w-[15px] h-[15px]" />
            </button>
          </Tooltip>
        </div>
      </li>
      <li v-if="(store.project?.tracks.length ?? 0) === 0" class="heads-empty text-body-sm">
        还没有轨道——点右上角的 + 新增
      </li>
    </ul>
  </div>
</template>

<style scoped>
/* 全高、方角，与顶栏融合 */
.track-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-width: 0;
  background-color: var(--app-card);
  overflow: hidden;
}
.brand {
  height: 52px;
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 0 10px 0 12px;
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.4);
  user-select: none;
}
.brand.pad-mac {
  padding-left: 78px;
}
/* 折叠按钮：紧跟红绿灯，命中区 30px */
.brand-toggle {
  width: 30px;
  height: 30px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
  transition: background-color 0.15s ease, color 0.15s ease;
}
.brand-toggle:hover {
  background-color: rgb(var(--md-on-surface-variant) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.brand-toggle:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.brand-toggle-icon {
  width: 18px;
  height: 18px;
}
.brand-logo {
  width: 20px;
  height: 20px;
  flex: 0 0 auto;
  color: rgb(var(--md-primary));
}
.brand-name {
  font-size: 13px;
  font-weight: 600;
  letter-spacing: -0.2px;
  color: rgb(var(--md-on-surface));
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
/* 表头与编排标尺同高（40px），保证两侧第一行起点一致 */
.heads-head {
  height: 40px;
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 8px 0 16px;
  color: rgb(var(--md-on-surface-variant));
  background-color: var(--app-inset);
  border-bottom: 1px solid var(--grid-line-strong);
}
.heads-list {
  list-style: none;
  margin: 0;
  padding: 0;
  flex: 1 1 auto;
  overflow-y: auto;
  overflow-x: hidden;
}
/* 行：整行 64px、无缝隙，与 lane 一一对应 */
.track-row {
  position: relative;
  display: flex;
  align-items: center;
  gap: 11px;
  height: 64px;
  padding: 0 8px 0 16px;
  cursor: pointer;
  border-bottom: 1px solid var(--grid-line-row);
}
.track-row::before {
  content: "";
  position: absolute;
  inset: 0;
  background-color: transparent;
  pointer-events: none;
  transition: background-color 0.16s ease;
}
.track-row:hover::before {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.track-row.selected::before {
  background-color: rgb(var(--md-secondary-container));
}
.grip {
  position: relative;
  width: 4px;
  height: 32px;
  border-radius: 999px;
  background-color: hsl(var(--track-hue) 55% 55%);
  flex: 0 0 auto;
}
.track-meta {
  position: relative;
  display: flex;
  flex-direction: column;
  min-width: 0;
  flex: 1 1 auto;
}
.track-name {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.track-sub {
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
/* 删除按钮：悬停或选中时才出现 */
.row-del {
  position: relative;
  width: 30px;
  height: 30px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
  opacity: 0;
  transition: opacity 0.15s ease, background-color 0.15s ease, color 0.15s ease;
}
.track-row:hover .row-del,
.track-row.selected .row-del,
.row-del:focus-visible {
  opacity: 1;
}
.row-del:hover {
  background-color: rgb(var(--md-error) / 0.14);
  color: rgb(var(--md-error));
}
.row-del:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
/* 行内操作：M / S / 删除。静音或独奏生效时常驻可见，其余情况悬停/选中才出现。 */
.row-actions {
  position: relative;
  display: flex;
  align-items: center;
  gap: 3px;
  flex: 0 0 auto;
}
.row-btn {
  width: 22px;
  height: 22px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 7px;
  padding: 0;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 10px;
  font-weight: 700;
  cursor: pointer;
  opacity: 0;
  transition: opacity 0.15s ease, background-color 0.14s ease, color 0.14s ease,
    border-color 0.14s ease;
}
.track-row:hover .row-btn,
.track-row.selected .row-btn,
.row-btn.on,
.row-btn:focus-visible {
  opacity: 1;
}
.row-btn:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.row-btn.on {
  background-color: rgb(var(--md-error));
  border-color: rgb(var(--md-error));
  color: rgb(var(--md-on-error));
}
.row-btn.solo.on {
  background-color: rgb(var(--md-primary));
  border-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}
.row-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
/* 静音的轨道整行变暗，与听感一致 */
.track-row.dimmed .track-meta {
  opacity: 0.5;
}
.heads-empty {
  padding: 20px 16px;
  color: rgb(var(--md-on-surface-muted));
}
</style>
