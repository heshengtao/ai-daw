<script setup lang="ts">
/**
 * 灵感页右半：**生成记录**。
 *
 * 一条记录 = 一首歌。列表按天分组，每行一个 `InspireRow`；生成中的那一次以「骨架行」
 * 插在最前面（带真实进度），完成时它原地变成一条正式记录——用户不需要刷新或切换视图。
 *
 * 这里只有列表该有的动作：播放 / 拖动进度 / 从列表移除 / 清空。没有点赞、没有分享、
 * 没有「升级解锁」——那些是商业产品的钩子，不是 DAW 的。
 */
import { computed, ref } from "vue";
import { SparklesIcon } from "@heroicons/vue/24/outline";
import ConfirmDialog from "@/components/common/ConfirmDialog.vue";
import InspireRow from "./InspireRow.vue";
import { dayLabel, formatDuration } from "./format";
import { useI18n } from "@/i18n";
import { useInspireStore, type InspireItem } from "@/stores/inspire";
import { useLayoutStore } from "@/stores/layout";

const i18n = useI18n();
const inspire = useInspireStore();
const layout = useLayoutStore();
const clearOpen = ref(false);

interface Group {
  key: string;
  label: string;
  items: InspireItem[];
}

/** 按天分组（记录本身已按时间倒序，分组的顺序就是日期的顺序）。 */
const groups = computed<Group[]>(() => {
  const out: Group[] = [];
  for (const item of inspire.items) {
    const key = new Date(item.createdAt).toDateString();
    const last = out[out.length - 1];
    if (last && last.key === key) {
      last.items.push(item);
      continue;
    }
    out.push({ key, label: dayLabel(item.createdAt, i18n.locale.value, i18n.t), items: [item] });
  }
  return out;
});

const count = computed(() => inspire.items.length);

function pick(item: InspireItem): void {
  void inspire.toggle(item);
}

function seek(item: InspireItem, ratio: number): void {
  void inspire.seekTo(item, ratio);
}

function clearAll(): void {
  inspire.clear();
  clearOpen.value = false;
}

/** 导入工程：成功才提示轨道名，失败由 store 写进 error 横幅。 */
async function importItem(item: InspireItem): Promise<void> {
  const name = await inspire.importToProject(item);
  if (name) layout.notify(i18n.t("inspire.importedToast", { name }));
}

/** 骨架行的进度：云端有真实百分比，本地只能不定长。 */
const craftPct = computed(() => inspire.progressPct);
</script>

<template>
  <section class="library" :aria-label="i18n.t('inspire.library')">
    <header class="library-head">
      <h2 class="library-title">{{ i18n.t("inspire.library") }}</h2>
      <span v-if="count > 0" class="library-count">{{ i18n.t("inspire.count", { count }) }}</span>
      <button
        v-if="count > 0"
        class="m3-btn text clear"
        type="button"
        @click="clearOpen = true"
      >
        {{ i18n.t("inspire.clear") }}
      </button>
    </header>

    <!-- 播放失败（文件被删 / 解不开）：贴在列表顶上，滚到哪都看得见 -->
    <div v-if="inspire.playError" class="banner play-banner" role="alert">
      <span class="banner-text">{{ inspire.playError }}</span>
      <button class="m3-btn text" type="button" @click="inspire.stopPlayback()">
        {{ i18n.t("inspire.dismiss") }}
      </button>
    </div>

    <!-- 上一次生成的失败原因：留在列表顶上，直到下一次提交 -->
    <div v-if="inspire.error" class="banner" role="alert">
      <span class="banner-text">{{ inspire.error }}</span>
      <button class="m3-btn text" type="button" @click="inspire.clearError()">
        {{ i18n.t("inspire.dismiss") }}
      </button>
    </div>

    <div class="scroll">
      <!-- 生成中：骨架行。完成时由 store 换成正式记录，位置不变 -->
      <div v-if="inspire.craft" class="skel" role="status" aria-live="polite">
        <div class="skel-thumb">
          <SparklesIcon class="skel-icon" />
        </div>
        <div class="skel-body">
          <div class="skel-head">
            <span class="skel-title">{{ inspire.craftTitle }}</span>
            <span class="skel-badge">{{ inspire.craft.request.model }}</span>
          </div>
          <p v-if="inspire.craft.request.prompt.trim()" class="skel-desc">
            {{ inspire.craft.request.prompt }}
          </p>
          <div class="skel-bar">
            <span
              class="skel-fill"
              :class="{ indeterminate: craftPct === null }"
              :style="craftPct === null ? undefined : { width: `${craftPct}%` }"
            />
          </div>
          <div class="skel-line">
            <span>{{ inspire.craftLine }}</span>
            <span class="skel-time">{{ formatDuration(inspire.elapsed) }}</span>
          </div>
        </div>
      </div>

      <!-- 空态：只在一首都没有、也没在生成时出现 -->
      <div v-if="count === 0 && !inspire.craft" class="empty">
        <span class="empty-icon" aria-hidden="true"><SparklesIcon /></span>
        <h3 class="empty-title">{{ i18n.t("inspire.emptyTitle") }}</h3>
        <p class="empty-body">{{ i18n.t("inspire.emptyBody") }}</p>
      </div>

      <template v-for="group in groups" :key="group.key">
        <h3 class="group">{{ group.label }}</h3>
        <InspireRow
          v-for="item in group.items"
          :key="item.id"
          :item="item"
          :hue="inspire.hueOf(item)"
          :placed="inspire.placedOf(item)"
          :busy="inspire.importingId === item.id"
          @toggle="pick(item)"
          @seek="(ratio) => seek(item, ratio)"
          @import="importItem(item)"
          @remove="inspire.remove(item.id)"
        />
      </template>

    </div>

    <ConfirmDialog
      v-model:open="clearOpen"
      :title="i18n.t('inspire.clearTitle')"
      :message="i18n.t('inspire.clearMessage')"
      :confirm-label="i18n.t('inspire.clear')"
      danger
      @confirm="clearAll"
    />
  </section>
</template>

<style scoped>
.library {
  flex: 1 1 auto;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background-color: var(--app-card);
  /* 列表自己是一条容器：左右两栏都开着时这里只有 400–650px，
     行内元素（封面 / 徽章 / 时间）要按**这里的**宽度收，而不是按窗口。 */
  container-type: inline-size;
}

.library-head {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 18px 22px 10px;
}
.library-title {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
  color: rgb(var(--md-on-surface));
}
.library-count {
  font-size: 11px;
  color: rgb(var(--md-on-surface-muted));
  font-variant-numeric: tabular-nums;
}
.clear {
  margin-left: auto;
}

.banner {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 12px;
  margin: 0 22px 8px;
  padding: 8px 10px 8px 14px;
  border-radius: var(--md-shape-corner-md);
  background-color: rgb(var(--md-error-container));
  color: rgb(var(--md-on-error-container));
}
.banner-text {
  flex: 1 1 auto;
  min-width: 0;
  font-size: 12px;
  line-height: 1.4;
  word-break: break-word;
}
.banner :deep(.m3-btn.text) {
  color: rgb(var(--md-on-error-container));
  flex: 0 0 auto;
}

.scroll {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  padding: 0 12px 24px;
  scrollbar-width: thin;
}

/* 分组标题：吸顶，滚动时一直知道现在是哪一天 */
.group {
  position: sticky;
  top: 0;
  z-index: 2;
  margin: 0;
  padding: 14px 10px 8px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.4px;
  color: rgb(var(--md-on-surface-muted));
  background-image: linear-gradient(
    to bottom,
    var(--app-card) 72%,
    rgb(var(--md-surface) / 0)
  );
  backdrop-filter: blur(2px);
}

/* ---- 骨架行 ---- */
.skel {
  display: flex;
  align-items: flex-start;
  gap: 16px;
  padding: 12px 14px;
  border-radius: var(--md-shape-corner-lg);
  border: 1px solid rgb(var(--md-primary) / 0.28);
  background-color: rgb(var(--md-primary) / 0.04);
  animation: m3-fade var(--md-duration-medium-2) var(--md-ease-standard);
}
.skel-thumb {
  flex: 0 0 auto;
  width: 76px;
  height: 76px;
  border-radius: var(--md-shape-corner-md);
  display: flex;
  align-items: center;
  justify-content: center;
  background: radial-gradient(
    circle at 30% 25%,
    rgb(var(--md-primary) / 0.28),
    rgb(var(--md-primary) / 0.08) 70%
  );
  color: rgb(var(--md-on-primary-container));
}
.skel-icon {
  width: 24px;
  height: 24px;
  animation: breathe 2.4s ease-in-out infinite;
}
.skel-body {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding-top: 2px;
}
.skel-head {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.skel-title {
  font-size: 14px;
  font-weight: 600;
  color: rgb(var(--md-on-surface));
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.skel-badge {
  flex: 0 0 auto;
  padding: 1px 7px;
  border-radius: 999px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.55);
  color: rgb(var(--md-on-surface-muted));
  font-size: 9.5px;
  font-weight: 600;
  letter-spacing: 0.6px;
  text-transform: uppercase;
  white-space: nowrap;
}
.skel-desc {
  margin: 0;
  font-size: 11.5px;
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.skel-bar {
  position: relative;
  height: 4px;
  border-radius: 999px;
  overflow: hidden;
  background-color: rgb(var(--md-on-surface) / 0.1);
}
.skel-fill {
  display: block;
  height: 100%;
  border-radius: inherit;
  background-color: rgb(var(--md-primary));
  transition: width var(--md-duration-medium-1) var(--md-ease-standard);
}
.skel-fill.indeterminate {
  width: 34%;
  animation: slide 1.5s var(--md-ease-emphasized) infinite;
}
.skel-line {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 10.5px;
  color: rgb(var(--md-on-surface-muted));
}
.skel-time {
  margin-left: auto;
  font-family: var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace);
  font-variant-numeric: tabular-nums;
}

/* ---- 空态 ---- */
.empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  min-height: 100%;
  padding: 72px 24px;
  text-align: center;
}
.empty-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 44px;
  height: 44px;
  margin-bottom: 4px;
  border-radius: 999px;
  background-color: rgb(var(--md-primary) / 0.1);
  color: rgb(var(--md-primary));
}
.empty-icon :deep(svg) {
  width: 22px;
  height: 22px;
}
.empty-title {
  margin: 0;
  font-size: 13px;
  font-weight: 600;
  color: rgb(var(--md-on-surface));
}
.empty-body {
  margin: 0;
  max-width: 320px;
  font-size: 11.5px;
  line-height: 1.5;
  color: rgb(var(--md-on-surface-muted));
}

/* 播放失败的横幅与生成失败区分开：一个是在「听」上出错，一个是在「做」上出错，用字色分 */
.play-banner {
  background-color: rgb(var(--md-surface-container-high));
  color: rgb(var(--md-on-surface));
  border: 1px solid rgb(var(--md-outline-variant) / 0.6);
}

@keyframes slide {
  0% {
    transform: translateX(-110%);
  }
  100% {
    transform: translateX(300%);
  }
}
@keyframes breathe {
  0%,
  100% {
    opacity: 0.55;
    transform: scale(0.94);
  }
  50% {
    opacity: 1;
    transform: scale(1);
  }
}
@media (prefers-reduced-motion: reduce) {
  .skel,
  .skel-icon,
  .skel-fill {
    animation: none;
    transition: none;
  }
  .skel-fill.indeterminate {
    width: 100%;
    opacity: 0.4;
  }
}
</style>
