<script setup lang="ts">
/**
 * **灵感**视图（顶栏最左）：整页的「写 → 生成 → 试听」工作台。
 *
 * 版面就是两栏：
 * - 左 `InspireComposer`：模式 / 模型 / 描述 / 歌词 / 生成；
 * - 右 `InspireLibrary`：生成记录，每行一条可拖动的播放进度条。
 *
 * 这一页**没有下窗口**（自动化 / 乐谱 / 键盘那些只属于编排与编辑视图），也不需要左音轨栏
 * 与右侧面板——外壳在 `App.vue` 里按路由让位，与设置页同一套处理。
 *
 * 引擎状态要在这里订阅一次：本地生成时面板要显示「引擎启动中 / 已就绪」，而它会随
 * `engine://yue-engine` 事件变；离开页面就退订，别让后台一直挂着。
 */
import { onMounted, onUnmounted } from "vue";
import InspireComposer from "@/components/workspace/inspire/InspireComposer.vue";
import InspireLibrary from "@/components/workspace/inspire/InspireLibrary.vue";
import { useInspireStore } from "@/stores/inspire";
import { useYueStore } from "@/stores/yue";

const inspire = useInspireStore();
const yue = useYueStore();
let stopYue: (() => void) | null = null;

onMounted(() => {
  stopYue = yue.watch();
});

onUnmounted(() => {
  stopYue?.();
  stopYue = null;
  // 离开页面就停掉试听：这一页没有播放器 UI 了，响着没法关。
  inspire.stopPlayback();
});
</script>

<template>
  <div class="inspire">
    <InspireComposer />
    <InspireLibrary />
  </div>
</template>

<style scoped>
.inspire {
  flex: 1 1 auto;
  min-width: 0;
  min-height: 0;
  display: flex;
  overflow: hidden;
  /* 底色用卡片色（与编排页一致），左栏再压一层 --app-inset 做区分 */
  background-color: var(--app-card);
}
</style>
