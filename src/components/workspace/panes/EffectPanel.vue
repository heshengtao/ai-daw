<script setup lang="ts">
/**
 * 内置效果器的界面：一排旋钮 + 「恢复默认值」。
 *
 * 效果器参数**不放在检查器里就地调**——链上效果一多，检查器会被几十个滑块塞满。
 * 与内置合成器同一个约定：检查器只给入口，编辑来它自己的界面。
 *
 * 拖动中用本地草稿保证跟手，松手才提交（一次拖动 = 一个撤销步骤）。
 */
import { computed, ref } from "vue";

import Md3Knob from "@/components/common/Md3Knob.vue";
import { usePluginsStore } from "@/stores/plugins";
import { useI18n } from "@/i18n";

const props = defineProps<{
  pluginId: string;
  /** 当前已保存的参数值。 */
  values: Record<string, number>;
}>();

const emit = defineEmits<{
  (e: "commit", id: string, value: number): void;
  /** 一次改多个参数（恢复默认值）：父组件必须合成**一个**撤销步骤。 */
  (e: "commitMany", params: Record<string, number>): void;
}>();

const plugins = usePluginsStore();
const { t } = useI18n();

const specs = computed(() => plugins.paramsOf(props.pluginId));

/** 拖动中的草稿：面板是受控的，没有草稿旋钮不会跟手。 */
const drafts = ref<Record<string, number>>({});

function base(id: string): number {
  const raw = props.values[id];
  if (typeof raw === "number" && Number.isFinite(raw)) return raw;
  return specs.value.find((p) => p.id === id)?.default ?? 0;
}

function shown(id: string): number {
  return drafts.value[id] ?? base(id);
}

function onLive(id: string, value: number): void {
  drafts.value = { ...drafts.value, [id]: value };
}

function onCommit(id: string, value: number): void {
  const next = { ...drafts.value };
  delete next[id];
  drafts.value = next;
  emit("commit", id, value);
}

/** 全部参数回到插件自报的默认值。 */
function defaults(): Record<string, number> {
  return Object.fromEntries(specs.value.map((p) => [p.id, p.default]));
}

/** 已经在默认值上就没必要再点一次。 */
const atDefaults = computed(() => specs.value.every((p) => shown(p.id) === p.default));

function restore(): void {
  drafts.value = {};
  emit("commitMany", defaults());
}

function display(id: string): string {
  const spec = specs.value.find((p) => p.id === id);
  return `${shown(id).toFixed(spec?.digits ?? 2)}${spec?.unit ?? ""}`;
}
</script>

<template>
  <div class="panel">
    <div class="knobs">
      <Md3Knob
        v-for="p in specs"
        :key="p.id"
        :model-value="shown(p.id)"
        :min="p.min"
        :max="p.max"
        :default="p.default"
        :size="78"
        :label="p.name"
        :display="display(p.id)"
        @update:model-value="(v: number) => onLive(p.id, v)"
        @commit="(v: number) => onCommit(p.id, v)"
      />
    </div>

    <p v-if="specs.length === 0" class="hint">{{ t("params.empty") }}</p>

    <footer class="foot">
      <button class="btn-tonal reset" :disabled="atDefaults" @click="restore">
        {{ t("panel.restoreDefaults") }}
      </button>
    </footer>
  </div>
</template>

<style scoped>
/* 铺满窗口：旋钮居中、底部一条动作栏。没有外层大圆角容器——面板本身就是窗口内容。 */
.panel {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px 14px 12px;
}
.knobs {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  align-content: center;
  justify-content: center;
  gap: 16px 24px;
}
.foot {
  flex: 0 0 auto;
  display: flex;
  justify-content: center;
}
.reset {
  height: 32px;
  padding: 0 18px;
  font-size: 12px;
}
.reset:disabled {
  opacity: 0.45;
  cursor: default;
}
.hint {
  margin: 0;
  text-align: center;
  font-size: 11.5px;
  color: rgb(var(--md-on-surface-muted));
}
</style>
