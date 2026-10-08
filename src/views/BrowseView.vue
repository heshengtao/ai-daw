<script setup lang="ts">
/** 浏览视图：乐器/素材清单骨架（插件扫描与加载后续接入）。 */
const items = [
  { id: "builtin.synth", name: "Builtin Synth", kind: "音源", builtin: true, note: "唯一内置音色插件 · 16 复音正弦 + ADSR" },
  { id: "vst3", name: "VST3 插件", kind: "音源 / 效果", builtin: false, note: "VST3 宿主将在下一版接入" },
];
</script>

<template>
  <div class="browse">
    <header class="browse-head">
      <h2 class="text-title-md">乐器与效果</h2>
      <span class="text-label-sm browse-sub">所有音色都是插件；内置只有一个合成器</span>
    </header>
    <ul class="browse-list">
      <li v-for="it in items" :key="it.id" class="browse-item card-outlined">
        <div class="item-main">
          <span class="text-title-sm">{{ it.name }}</span>
          <span class="text-label-sm item-note">{{ it.note }}</span>
        </div>
        <span class="item-kind" :class="{ builtin: it.builtin }">{{ it.builtin ? "内置" : it.kind }}</span>
      </li>
    </ul>
  </div>
</template>

<style scoped>
.browse {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  padding: 18px;
  background-color: rgb(var(--md-surface-container-lowest));
}
.browse-head {
  display: flex;
  align-items: baseline;
  gap: 12px;
  margin-bottom: 14px;
}
.browse-sub {
  color: rgb(var(--md-on-surface-variant));
}
.browse-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
  max-width: 640px;
}
.browse-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 16px;
}
.item-main {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}
.item-note {
  color: rgb(var(--md-on-surface-variant));
}
.item-kind {
  font-size: 10px;
  letter-spacing: 0.5px;
  padding: 3px 9px;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
}
.item-kind.builtin {
  background-color: rgb(var(--md-primary-container));
  color: rgb(var(--md-on-primary-container));
}
</style>
