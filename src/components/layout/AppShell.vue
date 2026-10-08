<script setup lang="ts">
/** 应用外壳：唯一顶部栏 + 工作区 + 底部传输/状态条 + 开发者抽屉。 */
import TopBar from "./TopBar.vue";
import BottomBar from "./BottomBar.vue";
import DevDrawer from "./DevDrawer.vue";

defineProps<{ playing: boolean; aiActive: boolean }>();
const emit = defineEmits<{
  (e: "play"): void;
  (e: "stop"): void;
  (e: "demo"): void;
  (e: "new"): void;
  (e: "export"): void;
}>();
</script>

<template>
  <div class="app-root">
    <TopBar @demo="emit('demo')" @new="emit('new')" @export="emit('export')" />

    <main class="app-main">
      <slot />
      <DevDrawer />
    </main>

    <BottomBar :playing="playing" :ai-active="aiActive" @play="emit('play')" @stop="emit('stop')" />
  </div>
</template>

<style scoped>
.app-root {
  display: flex;
  flex-direction: column;
  height: 100vh;
  overflow: hidden;
  background-color: rgb(var(--md-surface));
  color: rgb(var(--md-on-surface));
}
.app-main {
  position: relative;
  flex: 1;
  min-height: 0;
  min-width: 0;
  display: flex;
  overflow: hidden;
}
</style>
