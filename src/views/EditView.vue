<script setup lang="ts">
/** 编辑视图：钢琴卷帘骨架（真实编辑功能后续接入）。 */
import { computed } from "vue";
import { useProjectStore } from "@/stores/project";

const store = useProjectStore();
const tracks = computed(() => store.project?.tracks ?? []);
const notes = computed(() => tracks.value.flatMap((t) => t.clips).flatMap((c) => c.notes));

const LOW = 48; // C3
const HIGH = 84; // C6
const rows = computed(() => {
  const out: number[] = [];
  for (let p = HIGH; p >= LOW; p--) out.push(p);
  return out;
});
</script>

<template>
  <div class="piano">
    <div class="keys">
      <div v-for="p in rows" :key="p" class="key" :class="{ black: [1, 3, 6, 8, 10].includes(p % 12) }">
        <span v-if="p % 12 === 0">{{ p }}</span>
      </div>
    </div>
    <div class="roll">
      <div v-for="p in rows" :key="p" class="roll-row" :class="{ black: [1, 3, 6, 8, 10].includes(p % 12) }" />
      <div
        v-for="(n, i) in notes"
        :key="i"
        class="roll-note"
        :style="{ top: `${((HIGH - n.pitch) / rows.length) * 100}%`, left: `${n.start * 60}px`, width: `${Math.max(8, n.duration * 60)}px` }"
      />
      <p v-if="notes.length === 0" class="roll-empty text-body-md">这条轨道还没有音符</p>
    </div>
  </div>
</template>

<style scoped>
.piano {
  flex: 1;
  min-width: 0;
  display: flex;
  overflow: auto;
  background-color: rgb(var(--md-surface-container-lowest));
}
.keys {
  flex: 0 0 56px;
  position: sticky;
  left: 0;
  z-index: 2;
}
.key {
  height: 14px;
  display: flex;
  align-items: center;
  padding-left: 6px;
  font-size: 9px;
  color: rgb(var(--md-on-surface-variant));
  background-color: rgb(var(--md-surface-container-high));
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.3);
}
.key.black {
  background-color: rgb(var(--md-surface-container-highest));
}
.roll {
  position: relative;
  flex: 1;
  min-width: 1200px;
}
.roll-row {
  height: 14px;
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.16);
}
.roll-row.black {
  background-color: rgb(var(--md-surface-container) / 0.5);
}
.roll-note {
  position: absolute;
  height: 12px;
  border-radius: 3px;
  background-color: rgb(var(--md-primary));
  box-shadow: var(--md-elevation-1);
}
.roll-empty {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  color: rgb(var(--md-on-surface-muted));
}
</style>
