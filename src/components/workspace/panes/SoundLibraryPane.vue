<script setup lang="ts">
/**
 * 音色库：查看 AI 存下的整轨音色快照，应用到选中轨道，编辑元数据或删除。
 *
 * 保存入口不在这里——用户让 AI「记住这个音色」时才写入；本面板只做查看与清理。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import {
  BookmarkSquareIcon,
  PencilSquareIcon,
  TrashIcon,
} from "@heroicons/vue/24/outline";

import { useSoundMemoriesStore } from "@/stores/soundMemories";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { usePluginsStore } from "@/stores/plugins";
import { useLayoutStore } from "@/stores/layout";
import { useI18n } from "@/i18n";
import type { SoundMemorySummary } from "@/api/types";

const sounds = useSoundMemoriesStore();
const project = useProjectStore();
const selection = useSelectionStore();
const plugins = usePluginsStore();
const layout = useLayoutStore();
const { t } = useI18n();

const editingId = ref<string | null>(null);
const draftName = ref("");
const draftDescription = ref("");
const draftTags = ref("");
const confirmingId = ref<string | null>(null);
const busyId = ref<string | null>(null);
let stop: (() => void) | null = null;

const track = computed(() => selection.track);

function pluginName(id: string | null): string {
  return id ? plugins.name(id) : t("common.none");
}

function chainLabel(memory: SoundMemorySummary): string {
  return memory.effect_plugin_ids.length === 0
    ? t("sounds.noEffects")
    : memory.effect_plugin_ids.map((id) => plugins.name(id)).join(" → ");
}

function dateLabel(ms: number): string {
  return new Date(ms).toLocaleDateString();
}

/** 保留标签 `vocal` 在界面上按当前语言显示（筛选仍用原始值）。 */
function tagLabel(tag: string): string {
  return tag === "vocal" ? t("sounds.voiceTag") : tag;
}

/** 这条记忆是不是人声参考（没有乐器 / 效果链，只有一段 WAV）。 */
function isVoice(memory: SoundMemorySummary): boolean {
  return Boolean(memory.voice);
}

/** 人声条目只显示时长（以及自动裁剪的起点），不显示乐器链。 */
function voiceLabel(memory: SoundMemorySummary): string {
  const base = t("sounds.voiceMeta", { seconds: Math.round(memory.voice?.seconds ?? 0) });
  const offset = memory.voice?.offset_seconds ?? 0;
  if (offset < 1) return base;
  const mm = Math.floor(offset / 60);
  const ss = String(Math.round(offset % 60)).padStart(2, "0");
  return `${base} · ${t("sounds.voiceFrom", { time: `${mm}:${ss}` })}`;
}

async function apply(memory: SoundMemorySummary): Promise<void> {
  const target = track.value;
  if (!target) {
    layout.notify(t("sounds.selectTrackFirst"));
    return;
  }
  busyId.value = memory.id;
  try {
    await project.applySoundMemory(memory.id, target.id);
    layout.notify(t("sounds.applied", { name: memory.name }));
  } catch (err) {
    layout.notify(t("sounds.applyFailed", { error: String(err) }));
  } finally {
    busyId.value = null;
  }
}

/** 在音色库里把人声直接选作歌声参考：切到 AI 面板，并带过去这条 id。 */
function useVoice(memory: SoundMemorySummary): void {
  sounds.pickVoice(memory.id);
  layout.openTab("ai");
}

function startEdit(memory: SoundMemorySummary): void {
  editingId.value = memory.id;
  draftName.value = memory.name;
  draftDescription.value = memory.description;
  draftTags.value = memory.tags.join(", ");
}

async function commitEdit(memory: SoundMemorySummary): Promise<void> {
  const name = draftName.value.trim();
  if (!name) return;
  busyId.value = memory.id;
  try {
    await sounds.updateMetadata(memory.id, {
      name,
      description: draftDescription.value.trim(),
      tags: draftTags.value
        .split(",")
        .map((tag) => tag.trim())
        .filter(Boolean),
    });
    editingId.value = null;
  } catch (err) {
    layout.notify(t("sounds.saveFailed", { error: String(err) }));
  } finally {
    busyId.value = null;
  }
}

async function remove(memory: SoundMemorySummary): Promise<void> {
  busyId.value = memory.id;
  try {
    await sounds.remove(memory.id);
    confirmingId.value = null;
  } catch (err) {
    layout.notify(t("sounds.deleteFailed", { error: String(err) }));
  } finally {
    busyId.value = null;
  }
}

// 切换「乐器 / 人声」时清掉上一类的标签过滤，免得列表被一个不存在的标签筛空。
watch(
  () => sounds.kind,
  () => {
    sounds.activeTag = "";
  },
);

onMounted(() => {
  void sounds.refresh();
  stop = sounds.watchChanges();
});
onUnmounted(() => stop?.());
</script>

<template>
  <section class="sound-library">
    <header class="head">
      <BookmarkSquareIcon class="head-icon" />
      <div class="head-text">
        <h2>{{ t("sounds.title") }}</h2>
        <p>{{ t("sounds.count", { count: sounds.filtered.length }) }}</p>
      </div>
      <!-- 乐器 / 人声二选一：同一个库，两套东西 -->
      <div class="kind-switch" role="tablist" :aria-label="t('sounds.kindLabel')">
        <button
          class="kind-btn"
          :class="{ on: sounds.kind === 'instrument' }"
          type="button"
          role="tab"
          :aria-selected="sounds.kind === 'instrument'"
          @click="sounds.kind = 'instrument'"
        >
          {{ t("sounds.kindInstrument") }}
        </button>
        <button
          class="kind-btn"
          :class="{ on: sounds.kind === 'voice' }"
          type="button"
          role="tab"
          :aria-selected="sounds.kind === 'voice'"
          @click="sounds.kind = 'voice'"
        >
          {{ t("sounds.kindVoice") }}
        </button>
      </div>
    </header>

    <div class="toolbar">
      <input
        v-model="sounds.query"
        class="search md3-field"
        type="search"
        :placeholder="t('sounds.search')"
      />
    </div>

    <div v-if="sounds.tags.length" class="tags">
      <button
        class="chip"
        :class="{ active: sounds.activeTag === '' }"
        @click="sounds.activeTag = ''"
      >
        {{ t("sounds.tagAll") }}
      </button>
      <button
        v-for="tag in sounds.tags"
        :key="tag"
        class="chip"
        :class="{ active: sounds.activeTag === tag }"
        @click="sounds.activeTag = tag"
      >
        {{ tagLabel(tag) }}
      </button>
    </div>

    <div class="list">
      <div v-if="sounds.loading" class="empty">{{ t("common.loading") }}</div>
      <div v-else-if="sounds.filtered.length === 0" class="empty">
        <p>{{ t(sounds.kind === "voice" ? "sounds.emptyVoiceTitle" : "sounds.emptyTitle") }}</p>
        <p class="hint">
          {{ t(sounds.kind === "voice" ? "sounds.emptyVoiceHint" : "sounds.emptyHint") }}
        </p>
      </div>

      <article v-for="memory in sounds.filtered" :key="memory.id" class="card">
        <template v-if="editingId === memory.id">
          <input v-model="draftName" class="md3-field" :placeholder="t('sounds.name')" />
          <textarea
            v-model="draftDescription"
            class="net-input"
            rows="3"
            spellcheck="false"
            :placeholder="t('sounds.description')"
          />
          <textarea
            v-model="draftTags"
            class="net-input"
            rows="2"
            spellcheck="false"
            :placeholder="t('sounds.tags')"
          />
          <div class="actions">
            <button
              class="btn primary"
              :disabled="!draftName.trim() || busyId === memory.id"
              @click="commitEdit(memory)"
            >
              {{ t("common.save") }}
            </button>
            <button class="btn" @click="editingId = null">{{ t("common.cancel") }}</button>
          </div>
        </template>

        <template v-else-if="isVoice(memory)">
          <!-- 与乐器卡片同一副骨架：标题行 + 元信息 + 标签 + 页脚 + 操作行，只是元信息换成
               「人声参考 · N 秒」。不另起一套头像/底色，免得两边风格分裂。 -->
          <div class="card-head">
            <h3>{{ memory.name }}</h3>
          </div>
          <p v-if="memory.description" class="desc">{{ memory.description }}</p>
          <p class="meta">{{ voiceLabel(memory) }}</p>
          <div v-if="memory.tags.length" class="tag-row">
            <button
              v-for="tag in memory.tags"
              :key="tag"
              class="pill"
              type="button"
              :title="t('sounds.filterByTag')"
              @click="sounds.activeTag = tag"
            >
              {{ tagLabel(tag) }}
            </button>
          </div>
          <p class="foot">
            <span>
              {{ t("sounds.savedAt", { date: dateLabel(memory.updated_at || memory.created_at) }) }}
            </span>
          </p>
          <div class="actions">
            <button class="btn primary" type="button" @click="useVoice(memory)">
              {{ t("sounds.useVoice") }}
            </button>
            <button class="icon-btn" :title="t('sounds.edit')" @click="startEdit(memory)">
              <PencilSquareIcon />
            </button>
            <button
              v-if="confirmingId !== memory.id"
              class="icon-btn"
              :title="t('sounds.delete')"
              @click="confirmingId = memory.id"
            >
              <TrashIcon />
            </button>
            <button
              v-else
              class="btn danger"
              :disabled="busyId === memory.id"
              @click="remove(memory)"
            >
              {{ t("sounds.deleteConfirm") }}
            </button>
          </div>
        </template>

        <template v-else>
          <div class="card-head">
            <h3>{{ memory.name }}</h3>
            <span v-if="memory.role" class="role">{{ memory.role }}</span>
          </div>
          <p v-if="memory.description" class="desc">{{ memory.description }}</p>
          <p class="meta">
            {{ t("sounds.instrument") }}：{{ pluginName(memory.instrument_plugin_id) }}
          </p>
          <p class="meta">{{ t("sounds.effects") }}：{{ chainLabel(memory) }}</p>
          <p class="meta">
            {{
              t("sounds.mixer", {
                gain: memory.mixer.gain_db.toFixed(1),
                pan: memory.mixer.pan.toFixed(2),
              })
            }}
          </p>
          <div v-if="memory.tags.length" class="tag-row">
            <button
              v-for="tag in memory.tags"
              :key="tag"
              class="pill"
              type="button"
              :title="t('sounds.filterByTag')"
              @click="sounds.activeTag = tag"
            >
              {{ tagLabel(tag) }}
            </button>
          </div>
          <p class="foot">
            <span>
              {{ t("sounds.savedAt", { date: dateLabel(memory.updated_at || memory.created_at) }) }}
            </span>
            <span v-if="memory.source_project">
              {{ t("sounds.sourceProject", { name: memory.source_project }) }}
            </span>
          </p>
          <div class="actions">
            <button
              class="btn primary"
              :disabled="!track || busyId === memory.id"
              :title="t('sounds.applyHint')"
              @click="apply(memory)"
            >
              {{ t("sounds.apply") }}
            </button>
            <button class="icon-btn" :title="t('sounds.edit')" @click="startEdit(memory)">
              <PencilSquareIcon />
            </button>
            <button
              v-if="confirmingId !== memory.id"
              class="icon-btn"
              :title="t('sounds.delete')"
              @click="confirmingId = memory.id"
            >
              <TrashIcon />
            </button>
            <button
              v-else
              class="btn danger"
              :disabled="busyId === memory.id"
              @click="remove(memory)"
            >
              {{ t("sounds.deleteConfirm") }}
            </button>
          </div>
        </template>
      </article>
    </div>
  </section>
</template>

<style scoped>
.sound-library {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px 12px 6px;
}
.head-icon {
  width: 18px;
  height: 18px;
  color: rgb(var(--md-primary));
}
.head-text h2 {
  margin: 0;
  font-size: 13px;
  font-weight: 600;
}
.head-text p {
  margin: 1px 0 0;
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
}
.kind-switch {
  margin-left: auto;
  display: inline-flex;
  padding: 2px;
  gap: 2px;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-high) / 0.7);
}
.kind-btn {
  padding: 4px 12px;
  border: none;
  border-radius: 999px;
  background: transparent;
  font: inherit;
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
}
.kind-btn.on {
  background-color: rgb(var(--md-primary) / 0.18);
  color: rgb(var(--md-on-surface));
  font-weight: 600;
}
.toolbar {
  padding: 0 12px 8px;
}
.search {
  width: 100%;
}
.tags {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  padding: 0 12px 8px;
}
.chip,
.pill {
  cursor: pointer;
  font: inherit;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 10px;
  padding: 2px 8px;
}
.chip {
  cursor: pointer;
}
.pill:hover {
  background-color: rgb(var(--md-primary) / 0.12);
  color: rgb(var(--md-on-surface));
}
.chip.active {
  border-color: transparent;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}
.list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 0 12px 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.empty {
  padding: 24px 8px;
  text-align: center;
  font-size: 12px;
  color: rgb(var(--md-on-surface-variant));
}
.empty .hint {
  margin-top: 6px;
  font-size: 11px;
  opacity: 0.8;
}
.card {
  width: 100%;
  box-sizing: border-box;
  border: 1px solid rgb(var(--md-outline-variant) / 0.6);
  border-radius: 10px;
  padding: 10px;
  display: flex;
  flex-direction: column;
  gap: 5px;
}
/* 与设置页的多行输入框同一套样式。 */
.net-input {
  width: 100%;
  box-sizing: border-box;
  padding: 8px 10px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 10px;
  background-color: var(--app-card);
  color: rgb(var(--md-on-surface));
  font-family: inherit;
  font-size: 12px;
  line-height: 1.5;
  resize: vertical;
}
.net-input:focus {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 0;
}
.card-head {
  display: flex;
  align-items: center;
  gap: 6px;
}
.card-head h3 {
  margin: 0;
  font-size: 12px;
  font-weight: 600;
}
.role {
  font-size: 10px;
  color: rgb(var(--md-primary));
}
.desc {
  margin: 0;
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
}
.meta {
  margin: 0;
  font-size: 10px;
  color: rgb(var(--md-on-surface-variant));
}
.tag-row {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}
.foot {
  margin: 2px 0 0;
  display: flex;
  justify-content: space-between;
  gap: 8px;
  font-size: 10px;
  color: rgb(var(--md-on-surface-variant));
}
.actions {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 4px;
}
.btn {
  height: 26px;
  padding: 0 10px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 11px;
  cursor: pointer;
  white-space: nowrap;
}
.btn.primary {
  border-color: transparent;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}
.btn.danger {
  border-color: transparent;
  background-color: rgb(var(--md-error));
  color: rgb(var(--md-on-error));
}
.btn:disabled {
  opacity: 0.45;
  cursor: default;
}
.icon-btn {
  width: 26px;
  height: 26px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
}
.icon-btn svg {
  width: 14px;
  height: 14px;
}
.icon-btn:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
</style>
