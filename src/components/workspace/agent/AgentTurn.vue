<script setup lang="ts">
/**
 * 一轮对话（一条用户消息 + 它的整段回答）。
 *
 * 展示规则对齐 deepseek-harness 的「标准模式」：
 * - 思考、中间回复、工具调用一起收进「过程」，整轮一个折叠头；
 * - 最终回答留在折叠之外，永远可见；
 * - 运行中 / 已停止 / 处理失败的过程不折叠（DSH 对这几种结束原因不做整轮收起）；
 * - 正常完成后自动收起，只留「已完成，用时 X」一行。
 */
import { computed, ref } from "vue";
import { ChevronDownIcon } from "@heroicons/vue/24/outline";
import { useI18n } from "@/i18n";
import AgentMessage from "./AgentMessage.vue";
import ReasoningRow from "./ReasoningRow.vue";
import ToolGroup from "./ToolGroup.vue";
import { renderMarkdown } from "./markdown";
import type { TurnView } from "./turns";

const props = defineProps<{
  turn: TurnView;
  /** 每秒更新的当前时间：只给正在跑的那一轮算实时时长。 */
  liveNow?: number;
}>();

const emit = defineEmits<{ edit: [id: string, text: string]; branch: [id: string] }>();
const i18n = useI18n();

/** 用户手动开合过就尊重手动选择；否则正常结束的轮次默认收起。 */
const manualOpen = ref<boolean | null>(null);
const canFold = computed(() => props.turn.status === "done");
const open = computed(() => {
  if (!canFold.value) return true;
  return manualOpen.value ?? false;
});

function toggle(): void {
  if (!canFold.value) return;
  manualOpen.value = !open.value;
}

/** 正在跑的工具名（取最后一条还没出结果的）。 */
const activeTool = computed(() => {
  for (let i = props.turn.entries.length - 1; i >= 0; i--) {
    const entry = props.turn.entries[i];
    if (entry?.kind !== "tools" || !entry.running) continue;
    let pending = "";
    for (const tool of entry.tools) {
      if (tool.ok === null) pending = tool.name;
    }
    if (pending) return pending;
  }
  return "";
});

const elapsedMs = computed(() =>
  props.turn.running && props.liveNow
    ? Math.max(1000, props.liveNow - props.turn.startedAt)
    : props.turn.durationMs,
);

function formatDuration(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor(total / 60) % 60;
  const s = total % 60;
  if (h > 0) return i18n.t("agent.durationHours", { h, m, s });
  if (total >= 60) return i18n.t("agent.durationMinutes", { m, s });
  return i18n.t("agent.durationSeconds", { s });
}

const headerLabel = computed(() => {
  switch (props.turn.status) {
    case "running":
      return activeTool.value
        ? i18n.t("agent.process.runningTools", { name: activeTool.value })
        : i18n.t("agent.process.runningThinking");
    case "stopped":
      return i18n.t("agent.process.stopped");
    case "error":
      return i18n.t("agent.process.failed");
    default:
      return i18n.t("agent.process.took", { duration: formatDuration(elapsedMs.value) });
  }
});

const answerHtml = computed(() => renderMarkdown(props.turn.answer));
const hasProcess = computed(() => props.turn.entries.length > 0);
const showProcess = computed(() => hasProcess.value || props.turn.status !== "done");
</script>

<template>
  <div class="turn">
    <AgentMessage
      v-if="turn.user"
      :node="turn.user"
      :branches="turn.branches"
      @edit="(id, text) => emit('edit', id, text)"
      @branch="(id) => emit('branch', id)"
    />

    <!-- 整轮过程：折叠头 + 过程条目 -->
    <div v-if="showProcess" class="process">
      <button
        class="process-head"
        :disabled="!canFold"
        :aria-expanded="open"
        :data-status="turn.status"
        @click="toggle"
      >
        <span v-if="turn.status !== 'done'" class="status-dot" />
        <span class="process-label">{{ headerLabel }}</span>
        <span v-if="turn.running" class="live-clock">{{ formatDuration(elapsedMs) }}</span>
        <ChevronDownIcon v-if="canFold" class="chev" :class="{ open }" />
      </button>

      <div v-if="open && hasProcess" class="process-body">
        <template v-for="entry in turn.entries" :key="entry.key">
          <ReasoningRow v-if="entry.kind === 'reasoning'" :text="entry.text" :running="entry.running" />
          <div v-else-if="entry.kind === 'text'" class="step-text learn-markdown" v-html="renderMarkdown(entry.text)" />
          <ToolGroup
            v-else
            :tools="entry.tools"
            :running="entry.running"
            :settled="turn.status !== 'running'"
          />
        </template>
      </div>
      <p v-if="turn.error" class="turn-error">{{ turn.error }}</p>
    </div>

    <!-- 最终回答：留在折叠之外 -->
    <div
      v-if="turn.answer"
      class="answer learn-markdown"
      :class="{ streaming: turn.answerStreaming }"
      v-html="answerHtml"
    />
    <span v-else-if="turn.running" class="typing">…</span>
  </div>
</template>

<style scoped>
.turn {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;
}

/* ---- 整轮过程 ---- */
.process {
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.process-head {
  width: 100%;
  min-width: 0;
  height: 30px;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 0 6px;
  border: 0;
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.5);
  background: transparent;
  color: rgb(var(--md-on-surface-muted));
  font-size: 12px;
  text-align: left;
  cursor: pointer;
  transition: color 0.12s ease;
}
.process-head:disabled {
  cursor: default;
}
.process-head:not(:disabled):hover {
  color: rgb(var(--md-on-surface-variant));
}
.process-head[data-status="running"] {
  color: rgb(var(--md-on-surface-variant));
}
.process-label {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.live-clock {
  flex: 0 0 auto;
  font-size: 11px;
  color: rgb(var(--md-on-surface-muted));
  font-variant-numeric: tabular-nums;
}
.chev {
  flex: 0 0 auto;
  width: 14px;
  height: 14px;
  transition: transform 0.15s ease;
}
.chev.open {
  transform: rotate(180deg);
}
.status-dot {
  flex: 0 0 auto;
  width: 6px;
  height: 6px;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
}
.process-head[data-status="running"] .status-dot {
  animation: breathing 1.4s ease-in-out infinite;
}
.process-head[data-status="stopped"] .status-dot {
  background-color: rgb(var(--md-outline));
}
.process-head[data-status="error"] .status-dot {
  background-color: rgb(var(--md-error));
}
@keyframes breathing {
  50% {
    opacity: 0.35;
  }
}

.process-body {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  margin: 6px 0 0 7px;
  padding-left: 9px;
  border-left: 1px solid rgb(var(--md-outline-variant) / 0.6);
}
.step-text {
  min-width: 0;
  padding: 1px 0;
  font-size: 12.5px;
  color: rgb(var(--md-on-surface-variant));
}
.turn-error {
  margin: 6px 0 0;
  padding: 6px 10px;
  border-radius: 10px;
  background-color: rgb(var(--md-error) / 0.08);
  color: rgb(var(--md-error));
  font-size: 12px;
  line-height: 1.5;
  word-break: break-word;
}

/* ---- 最终回答 ---- */
.answer {
  min-width: 0;
  font-size: 13px;
  line-height: 1.65;
  color: rgb(var(--md-on-surface));
}
.answer.streaming > :last-child::after {
  content: "";
  display: inline-block;
  width: 6px;
  height: 13px;
  margin-left: 2px;
  vertical-align: -2px;
  border-radius: 1px;
  background-color: rgb(var(--md-primary));
  animation: blink 1s step-end infinite;
}
.typing {
  font-size: 13px;
  color: rgb(var(--md-on-surface-muted));
}
@keyframes blink {
  50% {
    opacity: 0;
  }
}

@media (prefers-reduced-motion: reduce) {
  .chev {
    transition: none;
  }
  .process-head[data-status="running"] .status-dot,
  .answer.streaming::after {
    animation: none;
  }
}
</style>
