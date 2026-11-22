<script setup lang="ts">
/**
 * 生成记录里的一首。
 *
 * 版式照 SUNO 的作品行（封面 / 标题 / 徽章 / 描述），但右边那些商业动作全部去掉：
 * **行底就是这条音频的播放进度条**。封面是 `InspireCover` 按曲子 id 生成的程序化图案，
 * 上面压一个**常显**的播放/暂停键——这一格能点、点哪儿，用户不用猜。
 */
import { computed } from "vue";
import { ArrowDownTrayIcon, ArrowPathIcon, ArrowUturnRightIcon, TrashIcon } from "@heroicons/vue/24/outline";
import { CpuChipIcon, CloudIcon } from "@heroicons/vue/24/outline";
import InspireCover from "./InspireCover.vue";
import Tooltip from "@/components/common/Tooltip.vue";
import InspireProgress from "./InspireProgress.vue";
import { useI18n } from "@/i18n";
import { useInspireStore, type InspireItem } from "@/stores/inspire";

const props = defineProps<{
  item: InspireItem;
  /** 轨道色相（与编排页一致）。 */
  hue: number;
  /** 这首歌是否已经在工程里（决定按钮文案与高亮）。 */
  placed?: boolean;
  /** 正在导入这一条。 */
  busy?: boolean;
}>();

const emit = defineEmits<{
  (e: "toggle"): void;
  (e: "seek", ratio: number): void;
  (e: "remove"): void;
  (e: "import"): void;
  (e: "continue"): void;
}>();

const i18n = useI18n();
const inspire = useInspireStore();

/** 这一首是否装着播放器（暂停后位置仍属于它）。 */
const current = computed(() => inspire.loadedId === props.item.id);
const playing = computed(() => inspire.playingId === props.item.id);
const loading = computed(() => inspire.loadingId === props.item.id);

const total = computed(() => (current.value && inspire.duration > 0 ? inspire.duration : props.item.seconds));
const position = computed(() => (current.value ? inspire.position : 0));
const ratio = computed(() => {
  const length = total.value;
  if (!current.value || length <= 0) return 0;
  return Math.min(1, Math.max(0, position.value / length));
});

/**
 * 描述：优先用提交时那段风格 / 场景描述；没有就退回歌词首句（从云端服务回来时可能只有词）。
 */
const description = computed(() => {
  const prompt = props.item.prompt.trim();
  if (prompt) return prompt;
  for (const line of props.item.lyrics.split("\n")) {
    const text = line.trim();
    if (text && !/^\[[^\]]+\]$/.test(text)) return text;
  }
  return "";
});

function onSeek(ratio: number): void {
  emit("seek", ratio);
}
</script>

<template>
  <article class="row" :class="{ playing, current }">
    <div class="thumb">
      <InspireCover
        :seed="item.id"
        :hue="hue"
        :playing="playing"
        :loading="loading"
        @toggle="emit('toggle')"
      />
    </div>

    <div class="body">
      <div class="head">
        <h3 class="title" :title="item.title">{{ item.title }}</h3>
        <span class="badge" :title="item.model || (item.source === 'cloud' ? i18n.t('inspire.cloud') : i18n.t('inspire.local'))">
          <CloudIcon v-if="item.source === 'cloud'" class="badge-icon" />
          <CpuChipIcon v-else class="badge-icon" />
          <span class="badge-text">{{ item.model || i18n.t(item.source === "cloud" ? "inspire.cloud" : "inspire.local") }}</span>
        </span>
        <div class="actions">
          <!-- 续写：接这首的语义码往下写一分钟（云端结果没有语义码，不出现这个按钮） -->
          <Tooltip v-if="item.source === 'local'" :text="i18n.t('inspire.continue')" align="end">
            <button
              class="continue"
              type="button"
              :disabled="busy"
              :aria-label="i18n.t('inspire.continue')"
              @click="emit('continue')"
            >
              <ArrowUturnRightIcon class="icon" />
            </button>
          </Tooltip>
          <Tooltip :text="placed ? i18n.t('inspire.imported') : i18n.t('inspire.import')" align="end">
            <button
              class="import"
              :class="{ on: placed }"
              type="button"
              :disabled="busy"
              :aria-label="placed ? i18n.t('inspire.imported') : i18n.t('inspire.import')"
              @click="emit('import')"
            >
              <ArrowPathIcon v-if="busy" class="icon spin" />
              <ArrowDownTrayIcon v-else class="icon" />
            </button>
          </Tooltip>
          <Tooltip :text="i18n.t('inspire.removeHint')" align="end">
            <button class="remove" type="button" :aria-label="i18n.t('inspire.remove')" @click="emit('remove')">
              <TrashIcon class="icon" />
            </button>
          </Tooltip>
        </div>
      </div>

      <p v-if="description" class="desc" :title="description">{{ description }}</p>

      <InspireProgress
        :value="ratio"
        :position="position"
        :total="total"
        :active="playing"
        :busy="loading"
        @seek="onSeek"
      />
    </div>
  </article>
</template>

<style scoped>
.row {
  display: flex;
  align-items: flex-start;
  gap: 16px;
  padding: 12px 14px;
  border-radius: var(--md-shape-corner-lg);
  border: 1px solid transparent;
  transition: background-color var(--md-duration-short-2) var(--md-ease-standard),
    border-color var(--md-duration-short-2) var(--md-ease-standard);
}
.row:hover {
  background-color: rgb(var(--md-on-surface) / 0.035);
}
.row.current {
  border-color: rgb(var(--md-outline-variant) / 0.45);
  background-color: rgb(var(--md-on-surface) / 0.035);
}

/* ---- 封面：程序化图案 + 常显播放键（见 InspireCover） ---- */
.thumb {
  position: relative;
  flex: 0 0 auto;
  width: 76px;
  height: 76px;
  border-radius: var(--md-shape-corner-md);
  overflow: hidden;
  /* 播放键的直径：窄列表里收一档，别把封面盖满 */
  --cover-play: 32px;
}

/* ---- 正文 ---- */
.body {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding-top: 2px;
}
.head {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.title {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
  line-height: 1.3;
  color: rgb(var(--md-on-surface));
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
  /* 标题抢剩余空间：徽章负责缩，标题负责说清楚这是哪一首 */
  flex: 1 1 auto;
}
/* 模型徽章：等宽小字，弱到只做背景信息；空间不够时**先让徽章缩**，标题优先 */
.badge {
  flex: 0 3 auto;
  min-width: 0;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  max-width: 190px;
  padding: 1px 7px;
  border-radius: 999px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.55);
  color: rgb(var(--md-on-surface-muted));
  font-size: 9.5px;
  font-weight: 600;
  letter-spacing: 0.6px;
  text-transform: uppercase;
  white-space: nowrap;
  overflow: hidden;
}
.badge-icon {
  width: 11px;
  height: 11px;
  flex: 0 0 auto;
}
.badge-text {
  overflow: hidden;
  text-overflow: ellipsis;
}
/* 导入与删除并排：导入常显（它是这张卡的主要动作），删除仍然 hover 才出现。 */
.actions {
  display: flex;
  align-items: center;
  gap: 2px;
  margin-left: auto;
}
/* 两个动作按钮共用一个底：尺寸、圆角、对齐必须一致，只有「显示时机」不同。 */
.import,
.continue,
.remove {
  flex: 0 0 auto;
  width: 26px;
  height: 26px;
  padding: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-muted));
  cursor: pointer;
  transition: opacity var(--md-duration-short-2) var(--md-ease-standard),
    background-color var(--md-duration-short-2) var(--md-ease-standard),
    color var(--md-duration-short-2) var(--md-ease-standard);
}
/* 图标尺寸也要一起写：heroicons 的 <svg> 没有 width/height 属性，不写尺寸时它会撑满按钮
   （26px 见方），比旁边删除按钮的 15px 图标大一圈——这正是两个按钮「大小不统一」的原因。 */
.import .icon,
.continue .icon,
.remove .icon {
  width: 15px;
  height: 15px;
}
.import:hover:not(:disabled) {
  background-color: rgb(var(--md-primary) / 0.12);
  color: rgb(var(--md-primary));
}
.continue:hover:not(:disabled) {
  background-color: rgb(var(--md-primary) / 0.12);
  color: rgb(var(--md-primary));
}
.continue:disabled,
.import:disabled {
  cursor: default;
  opacity: 0.6;
}
.continue:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.import.on {
  color: rgb(var(--md-primary));
}
.import:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
/* 删除是「破坏性」动作：平时藏起来，hover / 键盘聚焦才出现。 */
.remove {
  opacity: 0;
}
.row:hover .remove,
.remove:focus-visible {
  opacity: 1;
}
.remove:hover {
  background-color: rgb(var(--md-error) / 0.12);
  color: rgb(var(--md-error));
}
.remove:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}

.desc {
  margin: 0;
  font-size: 11.5px;
  line-height: 1.4;
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.spin {
  animation: spin 1s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

/* 列表窄的时候（左右两栏都开着就是这样）整体收一档，别让标题只剩三个字 */
@container (max-width: 520px) {
  .row {
    gap: 12px;
    padding: 10px 10px;
  }
  .thumb {
    width: 62px;
    height: 62px;
    --cover-play: 28px;
  }
  .badge {
    max-width: 118px;
  }
}
@container (max-width: 430px) {
  .thumb {
    width: 54px;
    height: 54px;
    --cover-play: 26px;
  }
  .badge {
    max-width: 92px;
  }
  /* 收一档时两个按钮一起收，别一个 26 一个 22 */
  .import,
  .continue,
  .remove {
    width: 22px;
    height: 22px;
  }
  .import .icon,
  .continue .icon,
  .remove .icon {
    width: 13px;
    height: 13px;
  }
}

@media (prefers-reduced-motion: reduce) {
  .row,
  .import,
  .remove {
    transition: none;
  }
  .spin {
    animation: none;
  }
}
</style>
