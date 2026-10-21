<script setup lang="ts">
/**
 * 历史工程：宽模态框里铺一排工程卡片，缩略图用迷你时间线画出轨道与片段。
 *
 * 列表每次打开都重新取——后端在「保存 / 新建 / 打开 / 退出 / 恢复」时写入历史，
 * 界面不该拿旧快照。点卡片只负责**交出选择并关闭**：恢复由 `App.vue` 执行，
 * 期间用顶栏下边缘的进度条提示（与音频导入同一套，不再弹旋转动画）。
 */
import { computed, ref, watch } from "vue";
import { ClockIcon, ExclamationTriangleIcon, TrashIcon } from "@heroicons/vue/24/outline";
import ModalShell from "@/components/common/ModalShell.vue";
import ConfirmDialog from "@/components/common/ConfirmDialog.vue";
import { useProjectStore } from "@/stores/project";
import { useI18n } from "@/i18n";
import { trackHue } from "@/utils/trackColor";
import type { HistoryClip, ProjectHistoryEntry } from "@/api/types";

const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{
  (e: "update:open", value: boolean): void;
  (e: "select", entry: ProjectHistoryEntry): void;
}>();

const project = useProjectStore();
const i18n = useI18n();

const entries = ref<ProjectHistoryEntry[]>([]);
const loading = ref(false);
const deleting = ref<string | null>(null);
const pendingDelete = ref<ProjectHistoryEntry | null>(null);
const error = ref<string | null>(null);

const dateFormat = computed(
  () =>
    new Intl.DateTimeFormat(i18n.locale.value, {
      dateStyle: "medium",
      timeStyle: "short",
    }),
);

watch(
  () => props.open,
  (open) => {
    if (open) void load();
    else pendingDelete.value = null;
  },
);

async function load(): Promise<void> {
  loading.value = true;
  error.value = null;
  try {
    entries.value = await project.listHistory();
  } catch (err) {
    error.value = i18n.t("history.failed", { error: String(err) });
  } finally {
    loading.value = false;
  }
}

function displayName(entry: ProjectHistoryEntry): string {
  return !entry.name || entry.name === "untitled" ? i18n.t("settings.untitled") : entry.name;
}

function fileName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

function updatedAt(entry: ProjectHistoryEntry): string {
  return dateFormat.value.format(new Date(entry.saved_at));
}

function meta(entry: ProjectHistoryEntry): string {
  return [
    i18n.t("history.bpm", { bpm: Math.round(entry.tempo) }),
    i18n.t("history.tracks", { count: entry.track_count }),
    i18n.t("history.clips", { count: entry.clip_count }),
  ].join(" · ");
}

function choose(entry: ProjectHistoryEntry): void {
  if (deleting.value) return;
  emit("select", entry);
  emit("update:open", false);
}

function askDelete(entry: ProjectHistoryEntry): void {
  if (deleting.value) return;
  pendingDelete.value = entry;
}

function closeDeletePrompt(open: boolean): void {
  if (!open) pendingDelete.value = null;
}

async function confirmDelete(): Promise<void> {
  const entry = pendingDelete.value;
  if (!entry || deleting.value) return;

  deleting.value = entry.id;
  error.value = null;
  try {
    await project.deleteHistory(entry.id);
    entries.value = entries.value.filter((item) => item.id !== entry.id);
  } catch (err) {
    error.value = i18n.t("history.deleteFailed", { error: String(err) });
  } finally {
    deleting.value = null;
  }
}

// ---- 缩略图：把轨道与片段压进 160×90 的迷你时间线 ----

const MAX_LANES = 7;

function lanes(entry: ProjectHistoryEntry): ProjectHistoryEntry["tracks"] {
  return entry.tracks.slice(0, MAX_LANES);
}

function laneHeight(entry: ProjectHistoryEntry): number {
  return 80 / Math.max(Math.min(entry.tracks.length, MAX_LANES), 1);
}

function laneY(entry: ProjectHistoryEntry, index: number): number {
  return 5 + index * laneHeight(entry);
}

function span(entry: ProjectHistoryEntry): number {
  return Math.max(entry.end_beat || 0, 16);
}

function clipX(entry: ProjectHistoryEntry, clip: HistoryClip): number {
  return (clip.start / span(entry)) * 160;
}

function clipWidth(entry: ProjectHistoryEntry, clip: HistoryClip): number {
  return Math.max((clip.length / span(entry)) * 160, 0.8);
}

function clipFill(trackIndex: number, clip: HistoryClip): string {
  const hue = trackHue(trackIndex);
  return clip.audio ? `hsla(${hue}, 45%, 68%, 0.5)` : `hsla(${hue}, 62%, 58%, 0.92)`;
}
</script>

<template>
  <ModalShell
    :open="open"
    :title="i18n.t('history.title')"
    width="min(1100px, calc(100vw - 48px))"
    @update:open="emit('update:open', $event)"
  >
    <div class="body">
      <div v-if="error" class="error-bar" role="alert">
        <ExclamationTriangleIcon class="w-4 h-4" />
        <span class="error-text">{{ error }}</span>
        <button class="retry" type="button" @click="load">{{ i18n.t("app.retry") }}</button>
      </div>

      <div v-if="loading" class="state">
        <p>{{ i18n.t("history.loading") }}</p>
      </div>

      <div v-else-if="!entries.length" class="state">
        <ClockIcon class="w-8 h-8 state-icon" />
        <p class="state-title">{{ i18n.t("history.empty") }}</p>
        <p class="state-hint">{{ i18n.t("history.emptyHint") }}</p>
      </div>

      <div v-else class="grid">
        <div v-for="entry in entries" :key="entry.id" class="card">
          <button
            class="card-open"
            type="button"
            :disabled="deleting !== null"
            :aria-label="displayName(entry)"
            @click="choose(entry)"
          >
            <span class="thumb">
              <svg viewBox="0 0 160 90" preserveAspectRatio="none" aria-hidden="true">
                <line
                  v-for="n in 7"
                  :key="`grid-${n}`"
                  :x1="(n * 160) / 8"
                  y1="0"
                  :x2="(n * 160) / 8"
                  y2="90"
                  class="thumb-grid"
                />
                <g v-for="(track, i) in lanes(entry)" :key="i">
                  <rect
                    x="0"
                    :y="laneY(entry, i)"
                    width="160"
                    :height="laneHeight(entry)"
                    :fill="i % 2 === 0 ? 'rgba(255,255,255,0.035)' : 'transparent'"
                  />
                  <rect
                    v-for="(clip, c) in track.clips"
                    :key="c"
                    :x="clipX(entry, clip)"
                    :y="laneY(entry, i) + 1"
                    :width="clipWidth(entry, clip)"
                    :height="Math.max(laneHeight(entry) - 2, 1)"
                    rx="1.2"
                    :fill="clipFill(i, clip)"
                  />
                </g>
              </svg>

              <span class="thumb-fade" aria-hidden="true" />

              <span v-if="entry.dirty" class="unsaved-badge">{{ i18n.t("history.dirty") }}</span>
            </span>

            <span class="card-body">
              <span class="name-row">
                <span class="name">{{ displayName(entry) }}</span>
                <span v-if="entry.dirty" class="dirty-dot" aria-hidden="true" />
              </span>
              <span class="meta">{{ meta(entry) }}</span>
              <span class="foot-row">
                <span class="date">{{ updatedAt(entry) }}</span>
                <span class="path" :title="entry.path ?? undefined">
                  {{ entry.path ? fileName(entry.path) : i18n.t("history.unsaved") }}
                </span>
              </span>
            </span>
          </button>

          <button
            class="delete-btn"
            type="button"
            :disabled="deleting !== null"
            :aria-label="i18n.t('history.delete')"
            :title="i18n.t('history.delete')"
            @click="askDelete(entry)"
          >
            <TrashIcon class="w-[15px] h-[15px]" />
          </button>
        </div>
      </div>
    </div>
  </ModalShell>

  <ConfirmDialog
    :open="pendingDelete !== null"
    :title="i18n.t('history.deleteTitle')"
    :message="
      i18n.t('history.deleteMessage', {
        name: pendingDelete ? displayName(pendingDelete) : '',
      })
    "
    :confirm-label="i18n.t('common.delete')"
    danger
    @update:open="closeDeletePrompt"
    @confirm="confirmDelete"
  />
</template>

<style scoped>
.body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 4px 22px 22px;
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(230px, 1fr));
  gap: 14px;
}

.card {
  position: relative;
  border: 1px solid rgb(var(--md-outline-variant) / 0.55);
  border-radius: 16px;
  background-color: rgb(var(--md-surface-container));
  overflow: hidden;
  transition: transform 0.15s ease, border-color 0.15s ease, box-shadow 0.15s ease;
}
.card:hover {
  transform: translateY(-2px);
  border-color: rgb(var(--md-primary) / 0.65);
  box-shadow: var(--md-elevation-2);
}
.card-open {
  display: flex;
  width: 100%;
  flex-direction: column;
  padding: 0;
  border: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
}
.card-open:disabled {
  cursor: default;
}
.card-open:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: -2px;
}
.delete-btn {
  position: absolute;
  top: 7px;
  right: 7px;
  width: 26px;
  height: 26px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background-color: rgba(10, 8, 13, 0.62);
  color: rgba(255, 255, 255, 0.82);
  cursor: pointer;
  opacity: 0;
  transition: opacity 0.14s ease, background-color 0.14s ease, color 0.14s ease;
  backdrop-filter: blur(4px);
}
.card:hover .delete-btn,
.delete-btn:focus-visible {
  opacity: 1;
}
.delete-btn:hover {
  background-color: rgb(var(--md-error));
  color: rgb(var(--md-on-error));
}
.delete-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.delete-btn:disabled {
  opacity: 0;
  cursor: default;
}
.thumb {
  position: relative;
  display: block;
  width: 100%;
  aspect-ratio: 16 / 9;
  background:
    radial-gradient(120% 130% at 12% 0%, rgb(var(--md-primary) / 0.16), transparent 58%),
    linear-gradient(150deg, #1d1a24, #141218 72%);
  overflow: hidden;
}
.thumb svg {
  display: block;
  width: 100%;
  height: 100%;
}
.thumb-grid {
  stroke: rgba(255, 255, 255, 0.07);
  stroke-width: 1;
  vector-effect: non-scaling-stroke;
}
.thumb-fade {
  position: absolute;
  inset: 0;
  pointer-events: none;
  box-shadow: inset 0 -18px 26px -18px rgba(0, 0, 0, 0.75);
}

.unsaved-badge {
  position: absolute;
  top: 7px;
  left: 7px;
  padding: 2px 7px;
  border-radius: 999px;
  background-color: rgba(10, 8, 13, 0.68);
  color: rgba(255, 255, 255, 0.86);
  font-size: 10px;
  line-height: 1.4;
  letter-spacing: 0.2px;
  backdrop-filter: blur(4px);
}

.card-body {
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: 9px 12px 11px;
  min-width: 0;
}
.name-row {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}
.name {
  font-size: 13px;
  font-weight: 600;
  color: rgb(var(--md-on-surface));
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.dirty-dot {
  width: 6px;
  height: 6px;
  flex: 0 0 auto;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
}
.meta {
  font-size: 11.5px;
  color: rgb(var(--md-on-surface-variant));
}
.foot-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 10px;
  min-width: 0;
}
.date {
  font-size: 10.5px;
  color: rgb(var(--md-on-surface-variant) / 0.85);
  white-space: nowrap;
}
.path {
  font-size: 10.5px;
  color: rgb(var(--md-on-surface-variant) / 0.7);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  min-height: 300px;
  color: rgb(var(--md-on-surface-variant));
}
.state p {
  margin: 0;
  font-size: 12.5px;
}
.state-icon {
  opacity: 0.55;
}
.state .state-title {
  font-size: 14px;
  font-weight: 500;
  color: rgb(var(--md-on-surface));
}
.state-hint {
  max-width: 380px;
  text-align: center;
  line-height: 1.5;
}

.error-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
  padding: 8px 12px;
  border-radius: 12px;
  background-color: rgb(var(--md-error) / 0.1);
  color: rgb(var(--md-error));
  font-size: 12px;
}
.error-text {
  flex: 1;
  min-width: 0;
  word-break: break-word;
}
.retry {
  flex: 0 0 auto;
  border: 0;
  border-radius: 999px;
  padding: 4px 10px;
  background: transparent;
  color: rgb(var(--md-error));
  font-size: 11.5px;
  font-weight: 500;
  cursor: pointer;
}
.retry:hover {
  background-color: rgb(var(--md-error) / 0.12);
}

@media (prefers-reduced-motion: reduce) {
  .card {
    transition: none;
  }
  .delete-btn {
    transition: none;
  }
}
</style>
