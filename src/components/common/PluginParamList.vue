<script setup lang="ts">
/**
 * 插件参数表：由插件**自报**的 `params()` 动态生成滑块。
 *
 * 内置插件与外部插件（VST3 / CLAP）走同一条路径——参数表来自 `stores/plugins`，
 * 外部插件第一次用到时才会向后端要一次。
 *
 * 拖动中只改本地草稿（滑块是受控组件，没有草稿把手不会跟手），松手才提交，
 * 因此一次拖动只产生一条撤销记录。
 */
import { computed, ref } from "vue";
import Md3Slider from "@/components/common/Md3Slider.vue";
import { usePluginsStore } from "@/stores/plugins";
import { useI18n } from "@/i18n";

const props = defineProps<{
  /** 插件 id；空表示没有插件。 */
  pluginId: string | undefined | null;
  /** 当前已保存的参数值（参数 id → 值）。 */
  values: Record<string, number>;
}>();

const emit = defineEmits<{ (e: "commit", id: string, value: number): void }>();

const plugins = usePluginsStore();
const i18n = useI18n();

const params = computed(() => plugins.paramsOf(props.pluginId));

/** 拖动中的参数草稿（键为参数 id）。 */
const draft = ref<Record<string, number>>({});

/** 显示值：优先草稿，其次工程里保存的值，最后插件默认值。 */
function shown(id: string, fallback: number): number {
  const d = draft.value[id];
  if (d !== undefined) return d;
  const raw = props.values[id];
  return typeof raw === "number" && Number.isFinite(raw) ? raw : fallback;
}

function onInput(id: string, value: number): void {
  draft.value = { ...draft.value, [id]: value };
}

function onCommit(id: string, value: number): void {
  const next = { ...draft.value };
  delete next[id];
  draft.value = next;
  emit("commit", id, value);
}
</script>

<template>
  <div class="param-list">
    <div v-for="p in params" :key="p.id" class="row param-row">
      <span class="row-label" :title="p.name">{{ p.name }}</span>
      <Md3Slider
        class="param-slider"
        :model-value="shown(p.id, p.default)"
        :min="p.min"
        :max="p.max"
        :step="p.step"
        :label="p.name"
        @update:model-value="(v: number) => onInput(p.id, v)"
        @commit="(v: number) => onCommit(p.id, v)"
      />
      <span class="row-value tabular">
        {{ shown(p.id, p.default).toFixed(p.digits ?? 2) }}{{ p.unit ?? "" }}
      </span>
    </div>

    <p v-if="params.length === 0" class="hint">{{ i18n.t("params.empty") }}</p>
  </div>
</template>

<style scoped>
.param-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
/* 参数行：标签固定宽度 + 滑块撑满 + 数值固定宽度，三列对齐 */
.param-row {
  display: grid;
  grid-template-columns: 72px 1fr 62px;
  gap: 8px;
}
.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  min-height: 34px;
  padding: 0 10px;
  border-radius: 12px;
  font-size: 12px;
}
.row-label {
  color: rgb(var(--md-on-surface-variant));
  flex: 0 0 auto;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.row-value {
  color: rgb(var(--md-on-surface));
  font-weight: 500;
  flex: 0 0 auto;
  font-variant-numeric: tabular-nums;
}
.param-slider {
  height: 20px;
}
.hint {
  margin: 0 8px;
  font-size: 11px;
  line-height: 1.5;
  color: rgb(var(--md-on-surface-muted));
}
.tabular {
  font-variant-numeric: tabular-nums;
}
</style>
