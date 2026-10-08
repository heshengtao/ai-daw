<script setup lang="ts">
/** 空状态引导卡片（面向新手），在可用区域中居中。 */
import { SparklesIcon, ArrowDownTrayIcon, PlusIcon } from "@heroicons/vue/24/outline";

defineProps<{ aiReady?: boolean }>();
const emit = defineEmits<{ (e: "demo"): void; (e: "new"): void }>();
</script>

<template>
  <div class="empty-state">
    <div class="card-elevated empty-card">
      <h2 class="text-headline-sm">开始做一段音乐</h2>
      <p class="text-body-md empty-hint">
        先载入一段示例听听看，或者从空白工程开始。你也可以随时用左上角与右上角的按钮展开专业面板。
      </p>
      <div class="empty-actions">
        <button class="btn-filled" @click="emit('demo')">
          <ArrowDownTrayIcon class="w-5 h-5" />
          载入示例
        </button>
        <button class="btn-tonal" @click="emit('new')">
          <PlusIcon class="w-5 h-5" />
          从零开始
        </button>
        <button class="btn-outlined" disabled :title="aiReady ? '' : '即将到来：让 AI 写一段'">
          <SparklesIcon class="w-5 h-5" />
          让 AI 写一段
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* flex:1 + min-width:0 才能真正在可用区域里居中（否则会缩到内容宽度并贴左）。 */
.empty-state {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  overflow: auto;
}
.empty-card {
  max-width: 460px;
  padding: 28px 28px 24px;
  text-align: center;
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.empty-hint {
  color: rgb(var(--md-on-surface-variant));
  margin: 0;
}
.empty-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  justify-content: center;
  margin-top: 6px;
}
</style>
