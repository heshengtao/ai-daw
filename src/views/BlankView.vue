<script setup lang="ts">
/**
 * 空白页。
 *
 * 宿主给「只能嵌入」的插件（CLAP 的 `floating=false`）开的窗口会加载这一页，然后把插件的
 * 原生视图挂到同一个窗口的内容视图上。网页这一层必须是空的，否则会在插件界面背后透出应用
 * 界面（插件视图通常还带一点透明边）。
 */
</script>

<template>
  <div class="blank" />
</template>

<style scoped>
.blank {
  position: fixed;
  inset: 0;
  background-color: rgb(var(--md-surface));
}
</style>
