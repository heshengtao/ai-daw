<script setup lang="ts">
/**
 * 预设条：一键切音色。
 *
 * - 选一个预设 → 把它的参数整组写回当前轨道（一次事务，一个撤销步骤）；
 * - 「另存为…」把**当前**参数存成一条新预设；
 * - 自存预设可以删除；出厂预设（内置合成器那几组）只能另存为。
 *
 * 应用动作交给父组件做：只有它知道这是哪条轨道。
 */
import { computed, nextTick, ref, watch } from "vue";
import { BookmarkIcon, PlusIcon, TrashIcon } from "@heroicons/vue/24/outline";

import { usePresetsStore } from "@/stores/presets";
import { useI18n } from "@/i18n";

const props = defineProps<{
  pluginId: string;
  /** 当前参数（用来判断命中哪个预设、以及「另存为」存什么）。 */
  params: Record<string, number>;
}>();

const emit = defineEmits<{ (e: "apply", params: Record<string, number>): void }>();

const presets = usePresetsStore();
const { t } = useI18n();

const options = computed(() => presets.forPlugin(props.pluginId));
const active = computed(() => presets.match(props.pluginId, props.params));

const naming = ref(false);
const draftName = ref("");
const nameEl = ref<HTMLInputElement | null>(null);
const busy = ref(false);

function startNaming(): void {
  draftName.value = active.value ? `${active.value.name} 副本` : "";
  naming.value = true;
  void nextTick(() => nameEl.value?.focus());
}

function cancelNaming(): void {
  naming.value = false;
  draftName.value = "";
}

async function commitName(): Promise<void> {
  const name = draftName.value.trim();
  if (!name || busy.value) return;
  busy.value = true;
  try {
    await presets.save(name, props.pluginId, props.params);
    cancelNaming();
  } finally {
    busy.value = false;
  }
}

function pick(id: string): void {
  const option = options.value.find((p) => p.id === id);
  if (option) emit("apply", option.params);
}

async function remove(): Promise<void> {
  const option = active.value;
  if (!option || option.factory || busy.value) return;
  busy.value = true;
  try {
    await presets.remove(option.id);
  } finally {
    busy.value = false;
  }
}

// 换插件时收起输入框，避免把名字存到别的插件上。
watch(
  () => props.pluginId,
  () => cancelNaming(),
);
</script>

<template>
  <div class="preset-bar">
    <BookmarkIcon class="icon" />
    <select
      class="select"
      :value="active?.id ?? ''"
      :aria-label="t('preset.choose')"
      @change="pick(($event.target as HTMLSelectElement).value)"
    >
      <option value="" disabled>{{ t("preset.unsaved") }}</option>
      <option v-for="p in options" :key="p.id" :value="p.id">
        {{ p.name }}{{ p.factory ? ` · ${t("preset.factory")}` : "" }}
      </option>
    </select>

    <template v-if="naming">
      <input
        ref="nameEl"
        v-model="draftName"
        class="name-input"
        type="text"
        spellcheck="false"
        :placeholder="t('preset.namePlaceholder')"
        :aria-label="t('preset.namePlaceholder')"
        @keydown.enter.prevent="commitName"
        @keydown.esc.prevent="cancelNaming"
      />
      <button class="btn primary" :disabled="!draftName.trim() || busy" @click="commitName">
        {{ t("common.save") }}
      </button>
      <button class="btn" @click="cancelNaming">{{ t("common.cancel") }}</button>
    </template>

    <template v-else>
      <button class="icon-btn" :title="t('preset.saveAsHint')" :aria-label="t('preset.saveAs')" @click="startNaming">
        <PlusIcon class="w-[15px] h-[15px]" />
      </button>
      <button
        class="icon-btn"
        :title="t('preset.deleteHint')"
        :aria-label="t('preset.deletePreset')"
        :disabled="!active || active.factory || busy"
        @click="remove"
      >
        <TrashIcon class="w-[15px] h-[15px]" />
      </button>
    </template>
  </div>
</template>

<style scoped>
.preset-bar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 2px 10px 6px;
}
.icon {
  width: 15px;
  height: 15px;
  flex: 0 0 auto;
  color: rgb(var(--md-on-surface-variant));
}
.select {
  flex: 1 1 auto;
  min-width: 0;
  height: 28px;
  padding: 0 8px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.6);
  border-radius: 8px;
  background-color: rgb(var(--md-surface-container-high));
  color: rgb(var(--md-on-surface));
  font-size: 11.5px;
  cursor: pointer;
}
.select:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.name-input {
  flex: 1 1 auto;
  min-width: 0;
  height: 28px;
  padding: 0 8px;
  border: 1px solid rgb(var(--md-primary) / 0.7);
  border-radius: 8px;
  background-color: rgb(var(--md-surface-container-high));
  color: rgb(var(--md-on-surface));
  font-size: 11.5px;
  outline: none;
}
.btn {
  flex: 0 0 auto;
  height: 28px;
  padding: 0 10px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 11px;
  cursor: pointer;
  white-space: nowrap;
}
.btn:hover:not(:disabled) {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.btn.primary {
  border-color: transparent;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}
.btn:disabled {
  opacity: 0.45;
  cursor: default;
}
.icon-btn {
  width: 26px;
  height: 26px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
}
.icon-btn:hover:not(:disabled) {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.icon-btn:disabled {
  opacity: 0.35;
  cursor: default;
}
</style>
