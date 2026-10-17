<script setup lang="ts">
/**
 * 节拍器图标（自绘）：三角外壳 + 底线 + 摆杆 + 配重。
 *
 * 刻意**不用音符**这类通用音乐标识——底栏的节拍器开关与检查器旁的拍号控件共用这一个轮廓，
 * 用户学一次认两处。原本只有拍号控件有它，底栏用的是 `MusicalNoteIcon`，两边不一致。
 *
 * 颜色走 `currentColor`（尺寸与配色由调用方决定）。
 */
</script>

<template>
  <svg class="metro-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <path d="M12 3.6 L16.6 20.4 H7.4 Z" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" />
    <path d="M6.2 20.4 h11.6" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
    <path d="M12 6.6 V15.2" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
    <circle cx="12" cy="15.6" r="1.5" fill="currentColor" />
  </svg>
</template>

<style scoped>
.metro-icon {
  width: 17px;
  height: 17px;
  flex: 0 0 auto;
}
</style>
