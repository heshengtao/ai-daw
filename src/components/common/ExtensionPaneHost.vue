<script setup lang="ts">
/**
 * 扩展面板的**错误边界**。
 *
 * 扩展的 Client 组件是第三方代码：渲染时抛错不能把整块右栏（甚至整页）带崩。这里用
 * `onErrorCaptured` 拦住子树里的错误，换成一页可关闭的错误占位——用户还能切走、
 * 关掉这个面板，其余界面照常。
 */
import { onErrorCaptured, ref, watch, type Component } from "vue";
import { ExclamationTriangleIcon } from "@heroicons/vue/24/outline";
import { useI18n } from "@/i18n";

const props = defineProps<{ pane: string; component: Component }>();
const i18n = useI18n();
const error = ref<string | null>(null);

onErrorCaptured((err) => {
  error.value = String(err);
  // 已经处理过：不再向上冒泡。
  return false;
});

watch(
  () => props.pane,
  () => {
    error.value = null;
  },
);
</script>

<template>
  <div v-if="error" class="pane-error">
    <ExclamationTriangleIcon class="w-4 h-4" />
    <div>
      <p class="title">{{ i18n.t("extensions.paneCrashed") }}</p>
      <pre class="detail">{{ error }}</pre>
    </div>
  </div>
  <component :is="props.component" v-else />
</template>

<style scoped>
.pane-error {
  display: flex;
  gap: 8px;
  margin: 12px;
  padding: 10px;
  border-radius: 10px;
  background: var(--md-sys-color-error-container);
  color: var(--md-sys-color-on-error-container);
  font-size: 12px;
}
.title {
  font-weight: 600;
}
.detail {
  margin-top: 6px;
  white-space: pre-wrap;
  font-size: 10px;
  opacity: 0.9;
}
</style>
