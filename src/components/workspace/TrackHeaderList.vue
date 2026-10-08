<script setup lang="ts">
/** 轨道头列表：带轨道配色的轨道条，含选中态与新增入口。 */
import { ref } from "vue";
import { PlusIcon } from "@heroicons/vue/24/outline";
import { useProjectStore } from "@/stores/project";
import { trackHue } from "@/utils/trackColor";

const store = useProjectStore();
const selected = ref<number | null>(null);

function notesOf(trackId: number): number {
  const track = store.project?.tracks.find((t) => t.id === trackId);
  return (track?.clips ?? []).reduce((n, c) => n + c.notes.length, 0);
}

function addTrack(): void {
  const next = (store.project?.tracks.length ?? 0) + 1;
  void store.addTrack(`轨道 ${next}`);
}
</script>

<template>
  <div class="track-heads">
    <header class="heads-head">
      <span class="text-label-sm">轨道</span>
      <button class="btn-icon" title="新增轨道" @click="addTrack">
        <PlusIcon class="w-4 h-4" />
      </button>
    </header>

    <ul class="heads-list">
      <li
        v-for="(track, ti) in store.project?.tracks ?? []"
        :key="track.id"
        class="track-row"
        :class="{ selected: track.id === selected }"
        :style="{ '--track-hue': String(trackHue(ti)) }"
        @click="selected = track.id"
      >
        <span class="grip" />
        <div class="track-meta">
          <span class="track-name text-title-sm">{{ track.name }}</span>
          <span class="track-sub text-label-sm">
            {{ track.instrument ? "内置合成器" : "未加载乐器" }} · {{ notesOf(track.id) }} 音符
          </span>
        </div>
      </li>
    </ul>
  </div>
</template>

<style scoped>
.track-heads {
  display: flex;
  flex-direction: column;
  height: 100%;
  background-color: rgb(var(--md-surface-container-low));
  border-right: 1px solid rgb(var(--md-outline-variant) / 0.35);
  overflow: hidden;
}
.heads-head {
  height: 36px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 6px 0 12px;
  color: rgb(var(--md-on-surface-variant));
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.3);
  flex: 0 0 auto;
}
.heads-list {
  list-style: none;
  margin: 0;
  padding: 4px;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.track-row {
  display: flex;
  align-items: center;
  gap: 9px;
  height: 64px;
  padding: 0 10px;
  border-radius: 12px;
  cursor: pointer;
  transition: background-color 0.16s ease, transform 0.16s ease;
}
.track-row:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.track-row:active {
  transform: scale(0.99);
}
.track-row.selected {
  background-color: rgb(var(--md-secondary-container));
}
.grip {
  width: 3px;
  height: 30px;
  border-radius: 999px;
  background-color: hsl(var(--track-hue) 55% 55%);
  flex: 0 0 auto;
}
.track-meta {
  display: flex;
  flex-direction: column;
  min-width: 0;
}
.track-name {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.track-sub {
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
