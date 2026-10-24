<script setup lang="ts">
/**
 * 音高偏移输入：对外仍是音分，对内显示**半音**。
 *
 * 引擎里 `bend` / `vibrato` 都以音分存储，但正常人只会说「推一个全音」「四分之一音」，
 * 不会说「推 200 音分」。输入框统一按半音步进，右侧再给一个人话音程名做参照。
 */
import { computed } from "vue";
import NumberField from "@/components/common/NumberField.vue";
import { useI18n } from "@/i18n";
import { centsToSemitones, intervalKey, semitonesToCents } from "@/utils/pitch";

const props = withDefaults(
  defineProps<{
    /** 音分。 */
    modelValue: number;
    /** 音分。 */
    min?: number;
    max?: number;
    /** 半音步进（0.25 = 四分之一音）。 */
    step?: number;
    /** 只给读屏用的名称。 */
    label: string;
    /** 是否显示右侧的音程名（例如「全音」）。 */
    showInterval?: boolean;
  }>(),
  { min: -2400, max: 2400, step: 0.25, showInterval: true },
);

const emit = defineEmits<{ (e: "update:modelValue", value: number): void }>();

const i18n = useI18n();

const semis = computed(() => centsToSemitones(props.modelValue));

const interval = computed(() => {
  const key = intervalKey(props.modelValue);
  return key ? i18n.t(key) : "";
});

function commit(value: number): void {
  emit("update:modelValue", semitonesToCents(value));
}
</script>

<template>
  <div class="pitch-field">
    <NumberField
      :model-value="semis"
      :min="min / 100"
      :max="max / 100"
      :step="step"
      :decimals="2"
      :label="i18n.t('pitch.semitoneUnit')"
      :sr-label="label"
      @update:model-value="commit"
    />
    <span v-if="showInterval && interval" class="interval">{{ interval }}</span>
  </div>
</template>

<style scoped>
.pitch-field {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}
.interval {
  padding: 3px 9px;
  border-radius: 999px;
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
  font-size: 10.5px;
  font-weight: 500;
  white-space: nowrap;
}
</style>
