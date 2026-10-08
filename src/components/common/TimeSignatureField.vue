<script setup lang="ts">
/**
 * 拍号控件：节拍器图标 + 当前拍号，点开弹出预设与自定义。
 * 图标刻意不是一条横线——用可辨识的节拍器轮廓，避免与「减号」混淆。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { ChevronDownIcon } from "@heroicons/vue/24/outline";
import NumberField from "@/components/common/NumberField.vue";
import Tooltip from "@/components/common/Tooltip.vue";
import { useI18n } from "@/i18n";

const i18n = useI18n();

const props = defineProps<{ num: number; den: number }>();
const emit = defineEmits<{ (e: "change", num: number, den: number): void }>();

/** 常用拍号预设。 */
const PRESETS: [number, number][] = [
  [2, 4],
  [3, 4],
  [4, 4],
  [5, 4],
  [6, 8],
  [7, 8],
  [9, 8],
  [12, 8],
];

/** 合法分母（2 的幂）。 */
const DENS = [2, 4, 8, 16];

const open = ref(false);
const root = ref<HTMLElement | null>(null);
const customNum = ref(props.num);

watch(
  () => props.num,
  (v) => {
    customNum.value = v;
  },
);

const label = computed(() => `${props.num}/${props.den}`);

function toggle(): void {
  open.value = !open.value;
}

function pick(n: number, d: number): void {
  emit("change", n, d);
  open.value = false;
}

function setNum(n: number): void {
  emit("change", n, props.den);
}

function setDen(d: number): void {
  emit("change", customNum.value, d);
}

function onDocPointer(e: PointerEvent): void {
  if (!open.value) return;
  if (root.value && !root.value.contains(e.target as Node)) open.value = false;
}

function onKey(e: KeyboardEvent): void {
  if (e.key === "Escape" && open.value) open.value = false;
}

onMounted(() => {
  document.addEventListener("pointerdown", onDocPointer);
  window.addEventListener("keydown", onKey);
});
onUnmounted(() => {
  document.removeEventListener("pointerdown", onDocPointer);
  window.removeEventListener("keydown", onKey);
});
</script>

<template>
  <div ref="root" class="ts-root">
    <Tooltip :text="i18n.t('ts.title')" place="above" align="center">
      <button
        class="ts-btn"
        :class="{ open }"
        :aria-expanded="open"
        aria-haspopup="dialog"
        :aria-label="i18n.t('ts.label')"
        @click="toggle"
      >
        <!-- 节拍器图标（非横线） -->
        <svg class="ts-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true">
          <path d="M12 3.6 L16.6 20.4 H7.4 Z" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" />
          <path d="M6.2 20.4 h11.6" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
          <path d="M12 6.6 V15.2" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
          <circle cx="12" cy="15.6" r="1.5" fill="currentColor" />
        </svg>
        <span class="ts-value">{{ label }}</span>
        <ChevronDownIcon class="ts-chevron" :class="{ flip: open }" />
      </button>
    </Tooltip>

    <transition name="pop">
      <div v-if="open" class="ts-pop" role="dialog" :aria-label="i18n.t('ts.choose')">
        <div class="ts-pop-title">{{ i18n.t("ts.label") }}</div>
        <div class="ts-grid">
          <button
            v-for="[n, d] in PRESETS"
            :key="`${n}/${d}`"
            class="ts-preset"
            :class="{ on: n === num && d === den }"
            @click="pick(n, d)"
          >
            {{ n }}/{{ d }}
          </button>
        </div>

        <div class="ts-divider" />

        <div class="ts-custom">
          <div class="ts-custom-row">
            <span class="ts-custom-label">{{ i18n.t("ts.numerator") }}</span>
            <NumberField v-model="customNum" :min="1" :max="32" @update:model-value="setNum" />
          </div>
          <div class="ts-custom-row">
            <span class="ts-custom-label">{{ i18n.t("ts.denominator") }}</span>
            <div class="ts-dens" role="group" :aria-label="i18n.t('ts.denominator')">
              <button
                v-for="d in DENS"
                :key="d"
                class="ts-den"
                :class="{ on: d === den }"
                @click="setDen(d)"
              >
                {{ d }}
              </button>
            </div>
          </div>
        </div>
      </div>
    </transition>
  </div>
</template>

<style scoped>
.ts-root {
  position: relative;
  display: inline-flex;
}
.ts-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 30px;
  padding: 0 10px 0 8px;
  border: 0;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-high));
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
  transition:
    background-color var(--md-spring-fast),
    color var(--md-spring-fast);
}
.ts-btn:hover,
.ts-btn.open {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.ts-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.ts-icon {
  width: 17px;
  height: 17px;
  flex: 0 0 auto;
}
.ts-value {
  font-size: 12px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  letter-spacing: 0.3px;
}
.ts-chevron {
  width: 13px;
  height: 13px;
  flex: 0 0 auto;
  transition: transform var(--md-spring-fast);
}
.ts-chevron.flip {
  transform: rotate(180deg);
}

/* ---- 弹出面板 ---- */
.ts-pop {
  position: absolute;
  bottom: calc(100% + 10px);
  left: 50%;
  transform: translateX(-50%);
  z-index: 60;
  width: 256px;
  padding: 12px;
  border-radius: 20px;
  background-color: rgb(var(--md-surface-container-high));
  border: 1px solid rgb(var(--md-outline-variant) / 0.4);
  box-shadow: var(--md-elevation-3);
}
.ts-pop-title {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.6px;
  color: rgb(var(--md-on-surface-muted));
  padding: 0 4px 8px;
}
.ts-grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 6px;
}
.ts-preset {
  height: 34px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 10px;
  background: transparent;
  color: rgb(var(--md-on-surface));
  font-size: 12px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  cursor: pointer;
  transition:
    background-color var(--md-spring-fast),
    border-color var(--md-spring-fast),
    transform var(--md-spring-fast);
}
.ts-preset:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.ts-preset:active {
  transform: scale(0.94);
}
.ts-preset.on {
  background-color: rgb(var(--md-primary));
  border-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}
.ts-divider {
  height: 1px;
  margin: 12px 0;
  background-color: rgb(var(--md-outline-variant) / 0.5);
}
.ts-custom {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.ts-custom-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 0 2px;
}
.ts-custom-label {
  font-size: 11px;
  letter-spacing: 0.6px;
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
}
.ts-dens {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}
.ts-den {
  width: 24px;
  height: 24px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 8px;
  background: transparent;
  color: rgb(var(--md-on-surface));
  font-size: 11px;
  font-weight: 600;
  cursor: pointer;
  transition: background-color var(--md-spring-fast), color var(--md-spring-fast);
}
.ts-den:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.ts-den.on {
  background-color: rgb(var(--md-primary));
  border-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}

/* 弹簧弹入 */
.pop-enter-active {
  transition: opacity var(--md-spring-fast), transform var(--md-spring-default);
}
.pop-leave-active {
  transition: opacity 140ms var(--md-ease-emphasized-accel), transform 140ms var(--md-ease-emphasized-accel);
}
.pop-enter-from,
.pop-leave-to {
  opacity: 0;
  transform: translateX(-50%) translateY(8px) scale(0.94);
}

@media (prefers-reduced-motion: reduce) {
  .ts-chevron,
  .ts-btn,
  .ts-preset,
  .pop-enter-active,
  .pop-leave-active {
    transition: none;
  }
}
</style>
