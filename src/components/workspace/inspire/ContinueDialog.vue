<script setup lang="ts">
/**
 * **续写时长**：点卡片上的「续写」后问一句要接多长。
 *
 * 默认 1 分钟（大多数人就是想「再来一段」）；上限是引擎的单次上限 600 秒减去原曲长度，
 * 所以原曲越长能加得越少——按钮上直接把这个上限夹住，别让人选了之后才报错。
 */
import { computed, ref, watch } from "vue";
import { MinusIcon, PlusIcon } from "@heroicons/vue/24/outline";
import Md3Chip from "@/components/common/Md3Chip.vue";
import ModalShell from "@/components/common/ModalShell.vue";
import { useI18n } from "@/i18n";
import type { InspireItem } from "@/stores/inspire";
import { formatDuration } from "./format";

const props = defineProps<{
  open: boolean;
  item: InspireItem | null;
}>();

const emit = defineEmits<{
  (e: "update:open", value: boolean): void;
  (e: "confirm", seconds: number): void;
}>();

const i18n = useI18n();

/** 预设档位（秒）。用 `formatDuration` 显示成 0:30 / 1:00 / 2:00，不需要另外的文案。 */
const PRESETS = [30, 60, 120];

/** 原曲多长（不知道就按 60 秒算，至少能续出东西来）。 */
const original = computed(() => {
  const seconds = Math.round(props.item?.seconds ?? 0);
  return seconds > 0 ? seconds : 60;
});

/** 加长的上限：引擎单次最多 600 秒，扣掉原曲。 */
const maxExtend = computed(() => Math.max(15, 600 - original.value));

const extend = ref(60);
const total = computed(() => original.value + extend.value);

/** 加减的步长：5 秒。太细没有意义（模型出的是整段音乐），太粗又不好微调。 */
const STEP = 5;

/** 上下限：至少 15 秒，最多把总长顶到 600 秒。 */
const minExtend = 15;

watch(
  () => props.open,
  (open) => {
    if (!open) return;
    extend.value = clamp(60);
  },
  { immediate: true },
);

function clamp(value: number): number {
  if (!Number.isFinite(value)) return 60;
  return Math.min(maxExtend.value, Math.max(minExtend, Math.round(value)));
}

function preset(seconds: number): void {
  extend.value = clamp(seconds);
}

/** 减号 / 加号：按步长走，边界处按钮自己会禁用，这里再夹一次保险。 */
function step(delta: number): void {
  extend.value = clamp(extend.value + delta);
}

function close(): void {
  emit("update:open", false);
}

function confirm(): void {
  emit("confirm", extend.value);
  emit("update:open", false);
}
</script>

<template>
  <ModalShell
    :open="open"
    :title="i18n.t('inspire.continue')"
    width="420px"
    @update:open="emit('update:open', $event)"
  >
    <div class="body">
      <p class="line">{{ i18n.t("inspire.continueBody", { length: formatDuration(original) }) }}</p>

      <div class="presets">
        <Md3Chip
          v-for="seconds in PRESETS"
          :key="seconds"
          size="sm"
          :selected="extend === seconds"
          :disabled="seconds > maxExtend"
          @click="preset(seconds)"
        >
          {{ formatDuration(seconds) }}
        </Md3Chip>
      </div>

      <div class="row">
        <span class="label">{{ i18n.t("inspire.continueExtend") }}</span>
        <!-- MD3 步进器：− 数字 +，两头到边界自动禁用 -->
        <div class="stepper" role="group" :aria-label="i18n.t('inspire.continueExtend')">
          <button
            class="step"
            type="button"
            :disabled="extend <= minExtend"
            :aria-label="i18n.t('inspire.continueLess')"
            @click="step(-STEP)"
          >
            <MinusIcon class="step-icon" />
          </button>
          <span class="value" aria-live="polite">{{ extend }}</span>
          <button
            class="step"
            type="button"
            data-autofocus
            :disabled="extend >= maxExtend"
            :aria-label="i18n.t('inspire.continueMore')"
            @click="step(STEP)"
          >
            <PlusIcon class="step-icon" />
          </button>
        </div>
        <span class="unit">{{ i18n.t("inspire.continueUnit") }}</span>
      </div>

      <p class="total">{{ i18n.t("inspire.continueTotal", { length: formatDuration(total) }) }}</p>
    </div>

    <template #foot>
      <footer class="foot">
        <button class="m3-btn text" type="button" @click="close">{{ i18n.t("common.cancel") }}</button>
        <button class="m3-btn filled" type="button" @click="confirm">
          {{ i18n.t("inspire.continueStart") }}
        </button>
      </footer>
    </template>
  </ModalShell>
</template>

<style scoped>
.body {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 4px 22px 12px;
}
.line {
  margin: 0;
  font-size: 14px;
  line-height: 1.5;
  color: rgb(var(--md-on-surface-variant));
}
.presets {
  display: flex;
  gap: 8px;
}
.chip {
  height: 30px;
  padding: 0 14px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 12.5px;
  font-variant-numeric: tabular-nums;
  cursor: pointer;
  transition: background-color var(--md-duration-short-2) var(--md-ease-standard),
    color var(--md-duration-short-2) var(--md-ease-standard),
    border-color var(--md-duration-short-2) var(--md-ease-standard);
}
.chip:hover:not(:disabled) {
  background-color: rgb(var(--md-primary) / 0.1);
  color: rgb(var(--md-primary));
}
.chip.on {
  border-color: rgb(var(--md-primary) / 0.7);
  background-color: rgb(var(--md-primary) / 0.14);
  color: rgb(var(--md-primary));
}
.chip:disabled {
  opacity: 0.45;
  cursor: default;
}
.row {
  display: flex;
  align-items: center;
  gap: 12px;
}
.label,
.unit {
  font-size: 13px;
  color: rgb(var(--md-on-surface-variant));
}
/* MD3 步进器：一颗胶囊里「− 数字 +」 */
.stepper {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  height: 40px;
  padding: 0 4px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.8);
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-highest) / 0.55);
}
.step {
  width: 32px;
  height: 32px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease;
}
.step:hover:not(:disabled) {
  background-color: rgb(var(--md-primary) / 0.14);
  color: rgb(var(--md-primary));
}
.step:active:not(:disabled) {
  background-color: rgb(var(--md-primary) / 0.22);
}
.step:disabled {
  opacity: 0.38;
  cursor: default;
}
.step:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.step-icon {
  width: 16px;
  height: 16px;
}
.value {
  min-width: 56px;
  text-align: center;
  font-size: 15px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  color: rgb(var(--md-on-surface));
}
.total {
  margin: 0;
  font-size: 12.5px;
  color: rgb(var(--md-on-surface-muted));
  font-variant-numeric: tabular-nums;
}
.foot {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  padding: 4px 18px 16px;
}
</style>
