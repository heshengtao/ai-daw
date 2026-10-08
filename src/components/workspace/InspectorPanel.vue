<script setup lang="ts">
/** 右侧检查器：工程/主输出信息（选中轨道的参数编辑将在后续版本接入）。 */
import { computed } from "vue";
import { useProjectStore } from "@/stores/project";

const store = useProjectStore();

const rows = computed(() => {
  const p = store.project;
  if (!p) return [];
  const instruments = p.tracks.filter((t) => t.instrument).length;
  const clips = p.tracks.reduce((n, t) => n + t.clips.length, 0);
  const notes = p.tracks.reduce((n, t) => n + t.clips.reduce((m, c) => m + c.notes.length, 0), 0);
  return [
    { label: "工程", value: p.name },
    { label: "轨道", value: String(p.tracks.length) },
    { label: "已加载乐器", value: String(instruments) },
    { label: "片段", value: String(clips) },
    { label: "音符", value: String(notes) },
    { label: "主增益", value: `${p.master.gain_db.toFixed(1)} dB` },
  ];
});
</script>

<template>
  <aside class="inspector">
    <header class="insp-head">
      <span class="text-label-sm">检查器</span>
    </header>
    <dl class="insp-body">
      <div v-for="row in rows" :key="row.label" class="insp-row">
        <dt>{{ row.label }}</dt>
        <dd>{{ row.value }}</dd>
      </div>
    </dl>
    <p class="insp-note">选中某个轨道后，这里会显示它的乐器与效果参数。</p>
  </aside>
</template>

<style scoped>
.inspector {
  display: flex;
  flex-direction: column;
  height: 100%;
  background-color: rgb(var(--md-surface-container-low));
  border-left: 1px solid rgb(var(--md-outline-variant) / 0.35);
  overflow: hidden;
}
.insp-head {
  height: 36px;
  display: flex;
  align-items: center;
  padding: 0 12px;
  color: rgb(var(--md-on-surface-variant));
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.3);
  flex: 0 0 auto;
}
.insp-body {
  margin: 0;
  padding: 10px 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.insp-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 10px;
  font-size: 12px;
}
.insp-row dt {
  color: rgb(var(--md-on-surface-variant));
}
.insp-row dd {
  margin: 0;
  color: rgb(var(--md-on-surface));
  font-weight: 500;
}
.insp-note {
  margin: auto 12px 12px;
  font-size: 11px;
  line-height: 1.5;
  color: rgb(var(--md-on-surface-muted));
}
</style>
