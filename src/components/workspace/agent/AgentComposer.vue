<script setup lang="ts">
/**
 * 输入框 + 发送 / 停止按钮（运行中「发送」变成「停止」）。
 *
 * 输入框随内容**向上长高**，最多 10 行；再多的内容不换行而是滚动（滚动条隐藏）。
 * 发送按钮浮在输入框内部右下角。
 */
import { nextTick, onMounted, ref, watch } from "vue";
import { PaperAirplaneIcon, StopIcon } from "@heroicons/vue/24/outline";
import { useI18n } from "@/i18n";

const props = defineProps<{
  running: boolean;
  disabled?: boolean;
}>();

const emit = defineEmits<{ send: [text: string]; stop: [] }>();
const i18n = useI18n();
const text = ref("");
const field = ref<HTMLTextAreaElement | null>(null);

/** 行高（13px × 1.5）；`scrollHeight` 不含边框，而全局是 `border-box`，所以要手动补上边框。 */
const LINE_HEIGHT = 19.5;
const PADDING_Y = 18;
const BORDER = 2;
const MAX_LINES = 10;
const MAX_HEIGHT = MAX_LINES * LINE_HEIGHT + PADDING_Y + BORDER;

/** 按内容重算高度：到 10 行封顶（超出靠内部滚动，CSS 里把滚动条藏掉）。 */
function resize(): void {
  const el = field.value;
  if (!el) return;
  el.style.height = "auto";
  el.style.height = `${Math.min(el.scrollHeight + BORDER, MAX_HEIGHT)}px`;
}

watch(text, () => void nextTick(resize));

function submit(): void {
  const value = text.value.trim();
  if (!value || props.running || props.disabled) return;
  emit("send", value);
  text.value = "";
  void nextTick(resize);
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    submit();
  }
}

onMounted(resize);
</script>

<template>
  <div class="composer">
    <div class="field">
      <textarea
        ref="field"
        v-model="text"
        class="input"
        rows="1"
        :placeholder="i18n.t('agent.placeholder')"
        :disabled="disabled"
        @keydown="onKeydown"
      />
      <button
        v-if="running"
        class="action stop"
        type="button"
        :aria-label="i18n.t('agent.stop')"
        @click="emit('stop')"
      >
        <StopIcon class="w-[15px] h-[15px]" />
      </button>
      <button
        v-else
        class="action"
        type="button"
        :disabled="disabled || !text.trim()"
        :aria-label="i18n.t('agent.send')"
        @click="submit"
      >
        <PaperAirplaneIcon class="w-[15px] h-[15px]" />
      </button>
    </div>
  </div>
</template>

<style scoped>
.composer {
  padding: 8px 12px 12px;
}
.field {
  position: relative;
  display: flex;
  align-items: flex-end;
}
.input {
  width: 100%;
  resize: none;
  /* 右侧给按钮留位，免得文字钻到按钮底下。 */
  padding: 9px 44px 9px 14px;
  border-radius: 18px;
  border: 1px solid rgb(var(--md-outline) / 0.7);
  background-color: rgb(var(--md-surface-container-high));
  color: rgb(var(--md-on-surface));
  font-size: 13px;
  line-height: 1.5;
  font-family: inherit;
  outline: none;
  /* 到 10 行封顶后滚动；滚动条藏起来。 */
  overflow-y: auto;
  max-height: 215px;
  scrollbar-width: none;
  transition: border-color 0.15s ease;
}
.input::-webkit-scrollbar {
  display: none;
}
.input:focus {
  border-color: rgb(var(--md-primary));
  box-shadow: 0 0 0 2px rgb(var(--md-primary) / 0.22);
}
.input::placeholder {
  color: rgb(var(--md-on-surface-muted));
}
.action {
  position: absolute;
  right: 6px;
  /* 单行时正好居中（(39.5 - 30) / 2 ≈ 4.75）；多行时自然贴着底部。 */
  bottom: 5px;
  width: 30px;
  height: 30px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
  cursor: pointer;
  transition: filter 0.15s ease, background-color 0.15s ease;
}
.action:hover:not(:disabled) {
  filter: brightness(1.08);
}
.action:disabled {
  opacity: 0.45;
  cursor: default;
}
.action.stop {
  background-color: rgb(var(--md-error));
  color: rgb(var(--md-on-error));
}
</style>
