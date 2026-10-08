<script setup lang="ts">
/** 首屏引导：导入现有工程，或新建一个空白工程。 */
import { FolderOpenIcon, PlusIcon } from "@heroicons/vue/24/outline";

const emit = defineEmits<{ (e: "import"): void; (e: "new"): void }>();
</script>

<template>
  <div class="empty-state">
    <div class="card-elevated empty-card">
      <h2 class="text-headline-sm">开始你的工程</h2>
      <p class="text-body-md empty-hint">导入一个已有的工程文件，或者新建一个空白工程。</p>
      <div class="empty-actions">
        <button class="btn-filled" @click="emit('import')">
          <FolderOpenIcon class="w-5 h-5" />
          导入工程
        </button>
        <button class="btn-tonal" @click="emit('new')">
          <PlusIcon class="w-5 h-5" />
          新建工程
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
  max-width: 440px;
  padding: 30px 28px 26px;
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
  margin-top: 8px;
}
</style>
