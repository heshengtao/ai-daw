<script setup lang="ts">
/** 空状态引导卡片（面向新手）。 */
import { SparklesIcon, ArrowDownTrayIcon, PlusIcon } from "@heroicons/vue/24/outline";

defineProps<{ aiReady?: boolean }>();
const emit = defineEmits<{ (e: "demo"): void; (e: "new"): void }>();
</script>

<template>
  <div class="empty-state">
    <div class="card-elevated empty-card">
      <h2 class="text-headline-sm">开始做一段音乐</h2>
      <p class="text-body-md empty-hint">
        先载入一段示例听听看，或者从空白工程开始。你也可以随时从右上角展开专业面板。
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
.empty-state {
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
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
