<script setup lang="ts">
/** 混音视图：通道条骨架（真实推子/发送后续接入）。 */
import { computed } from "vue";
import { useProjectStore } from "@/stores/project";

const store = useProjectStore();
const strips = computed(() => {
  const tracks = store.project?.tracks ?? [];
  return [
    ...tracks.map((t) => ({ id: `t${t.id}`, name: t.name, gain: t.mixer.gain_db, instrument: Boolean(t.instrument) })),
    { id: "master", name: "主输出", gain: store.project?.master.gain_db ?? 0, instrument: true },
  ];
});
</script>

<template>
  <div class="mix">
    <div v-for="s in strips" :key="s.id" class="strip" :class="{ master: s.id === 'master' }">
      <span class="strip-name text-label-sm">{{ s.name }}</span>
      <div class="fader-track"><div class="fader-knob" :style="{ bottom: `${Math.min(96, Math.max(4, 50 + s.gain * 8))}%` }" /></div>
      <span class="strip-gain text-label-sm">{{ s.gain.toFixed(1) }} dB</span>
      <span class="strip-tag" :class="{ on: s.instrument }">{{ s.instrument ? "已加载乐器" : "空" }}</span>
    </div>
  </div>
</template>

<style scoped>
.mix {
  flex: 1;
  min-width: 0;
  display: flex;
  gap: 10px;
  padding: 14px;
  overflow-x: auto;
  background-color: rgb(var(--md-surface-container-lowest));
}
.strip {
  flex: 0 0 96px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  padding: 12px 8px;
  border-radius: 18px;
  background-color: rgb(var(--md-surface-container));
  border: 1px solid rgb(var(--md-outline-variant) / 0.35);
}
.strip.master {
  background-color: rgb(var(--md-primary-container) / 0.5);
  border-color: rgb(var(--md-primary) / 0.45);
}
.strip-name {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 100%;
}
.fader-track {
  position: relative;
  width: 12px;
  flex: 1;
  min-height: 120px;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-highest));
}
.fader-knob {
  position: absolute;
  left: 50%;
  transform: translate(-50%, 50%);
  width: 26px;
  height: 12px;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  box-shadow: var(--md-elevation-1);
}
.strip-gain {
  color: rgb(var(--md-on-surface-variant));
}
.strip-tag {
  font-size: 10px;
  padding: 2px 8px;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface-muted));
}
.strip-tag.on {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
</style>
