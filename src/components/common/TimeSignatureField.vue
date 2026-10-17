<script setup lang="ts">
/**
 * 拍号控件：节拍器图标 + 当前拍号，点开弹出预设与自定义。
 * 图标刻意不是一条横线——用可辨识的节拍器轮廓，避免与「减号」混淆。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { ChevronDownIcon } from "@heroicons/vue/24/outline";
import MetronomeIcon from "@/components/common/icons/MetronomeIcon.vue";
import NumberField from "@/components/common/NumberField.vue";
import Tooltip from "@/components/common/Tooltip.vue";
import { useI18n } from "@/i18n";

const i18n = useI18n();

const props = defineProps<{
  num: number;
  den: number;
  /** 常用预设。默认是通用列表；音乐生成那边只传模型训练过的拍号。 */
  presets?: [number, number][];
}>();
const emit = defineEmits<{ (e: "change", num: number, den: number): void }>();

/** 通用拍号预设（调用方可用 `presets` 覆盖）。 */
const DEFAULT_PRESETS: [number, number][] = [
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

const presets = computed(() => props.presets ?? DEFAULT_PRESETS);

const open = ref(false);
const root = ref<HTMLElement | null>(null);
/** 弹层本体：它挂在 `body` 上，所以要单独判「点在里面」。 */
const pop = ref<HTMLElement | null>(null);
const customNum = ref(props.num);

/**
 * 弹层位置（`fixed`，相对视口）。
 *
 * 弹层必须 `Teleport` 到 `body`：它原来是相对控件的绝对定位，而调用方可能在**可滚动容器**里
 * （AI 音乐生成那张卡片就是），容器上的 `overflow-y: auto` 会把弹层裁掉——被裁掉的部分点下去
 * 会被当成「点在控件外面」，`pointerdown` 先把弹层关掉，于是按钮永远点不到。
 */
const popStyle = ref<Record<string, string>>({});
/** 弹层宽度（与 CSS 里的 `width` 一致），用于把弹层夹在视口内。 */
const POP_WIDTH = 256;
/** 弹层高度的估计值：上方不够就翻到下方去。 */
const POP_HEIGHT = 300;
/** 这次是往上弹还是往下弹（影响弹出动画方向）。 */
const dropUp = ref(true);

/** 按控件位置重新摆弹层（打开、滚动、改窗口大小都要重算）。 */
function place(): void {
  const el = root.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  // 弹层是 `left` 定位（不再用 translateX 居中），所以这里算的是**左边缘**。
  const left = Math.min(
    Math.max(rect.left + rect.width / 2 - POP_WIDTH / 2, 8),
    window.innerWidth - POP_WIDTH - 8,
  );
  // 上方放得下就往上弹（贴控件）；放不下就翻到下方，免得顶出屏幕被截断。
  dropUp.value = rect.top > POP_HEIGHT + 8;
  popStyle.value = dropUp.value
    ? { left: `${Math.round(left)}px`, bottom: `${Math.round(window.innerHeight - rect.top + 10)}px` }
    : { left: `${Math.round(left)}px`, top: `${Math.round(rect.bottom + 10)}px` };
}

watch(
  () => props.num,
  (v) => {
    customNum.value = v;
  },
);

const label = computed(() => `${props.num}/${props.den}`);

function toggle(): void {
  if (!open.value) place();
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
  const target = e.target as Node;
  // 弹层已不在控件子树里（挂在 body 上），所以要两处都判。
  if (root.value?.contains(target) || pop.value?.contains(target)) return;
  open.value = false;
}

function onKey(e: KeyboardEvent): void {
  if (e.key === "Escape" && open.value) open.value = false;
}

/** 滚动 / 改窗口大小时跟着控件走（弹层是 fixed，不跟会飘）。 */
function onReflow(): void {
  if (open.value) place();
}

onMounted(() => {
  document.addEventListener("pointerdown", onDocPointer);
  window.addEventListener("keydown", onKey);
  window.addEventListener("scroll", onReflow, true);
  window.addEventListener("resize", onReflow);
});
onUnmounted(() => {
  document.removeEventListener("pointerdown", onDocPointer);
  window.removeEventListener("keydown", onKey);
  window.removeEventListener("scroll", onReflow, true);
  window.removeEventListener("resize", onReflow);
});
</script>

<template>
  <div ref="root" class="ts-root">
    <!-- 弹层开着的时候不显示提示：提示会压在弹层下半部分上 -->
    <Tooltip :text="open ? '' : i18n.t('ts.title')" place="above" align="center">
      <button
        class="ts-btn"
        :class="{ open }"
        :aria-expanded="open"
        aria-haspopup="dialog"
        :aria-label="i18n.t('ts.label')"
        @click="toggle"
      >
        <!-- 节拍器图标（非横线）；与底栏的节拍器开关共用同一个组件。 -->
        <MetronomeIcon />
        <span class="ts-value">{{ label }}</span>
        <ChevronDownIcon class="ts-chevron" :class="{ flip: open }" />
      </button>
    </Tooltip>

    <Teleport to="body">
      <transition name="pop">
        <div
          v-if="open"
          ref="pop"
          class="ts-pop"
          :style="popStyle"
          :class="{ down: !dropUp }"
          role="dialog"
          :aria-label="i18n.t('ts.choose')"
        >
        <div class="ts-pop-title">{{ i18n.t("ts.label") }}</div>
        <div class="ts-grid">
          <button
            v-for="[n, d] in presets"
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
    </Teleport>
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
  position: fixed;
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
  transform: translateY(8px) scale(0.94);
}
/* 往下弹时动画方向反过来 */
.ts-pop.down.pop-enter-from,
.ts-pop.down.pop-leave-to {
  transform: translateY(-8px) scale(0.94);
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
