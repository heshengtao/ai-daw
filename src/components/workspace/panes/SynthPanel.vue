<script setup lang="ts">
/**
 * 内置合成器的**插件级独立界面**：一台固定尺寸（900×560）的 MD3 设备面板。
 *
 * 布局照硬件合成器来（AKAI MPK mini 那一类）：
 * 顶部是**预设垫子**——当前类别的几条预设排成一块块大圆角方块，点一下整组参数就切过去；
 * 中部左边是「屏幕」（一个周期示波器 + ADSR 曲线），右边是基础旋钮；
 * 底部是一条固定高度的试听键盘。设备里所有纵向区域都用固定高度 / `flex: 0 0 auto`，
 * 键盘因此永远不会被压扁变形；容器窄于 `DEVICE_W` 时面板整体等比缩小（`transform: scale`），
 * 比例照样不变。容器宽于 `DEVICE_W` 时不放大，保持 1。
 *
 * 旋钮分两层：**基础**只露 8 个最常用的，其余（副振 / 脉宽 / 滑音 / 高通 / 滤波包络 / 键跟随 / LFO）
 * 收在「高级」里，默认不打扰小白。
 *
 * 参数值来自工程的 `instrument.params`（缺省回落到参数表默认值）；拖动时用本地草稿保证跟手，
 * 松手才提交给引擎。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";

import Md3Knob from "@/components/common/Md3Knob.vue";
import Oscilloscope from "@/components/common/Oscilloscope.vue";
import PianoKeyboard from "@/components/common/PianoKeyboard.vue";
import PresetBar from "@/components/common/PresetBar.vue";
import { SYNTH_ID, SYNTH_PARAMS, type ParamSpec } from "@/data/plugins";
import { useI18n } from "@/i18n";
import { usePresetsStore, type PresetOption } from "@/stores/presets";
import type { WaveParams } from "@/utils/waveform";

/** 设计尺寸（px）：固定不变，Task 9 的等比缩放按这两个常量算。 */
const DEVICE_W = 900;
const DEVICE_H = 560;

const props = defineProps<{
  params: Record<string, number>;
  /** 试听键盘要发给哪条轨道；不给就不显示键盘（比如侧边栏里没选中轨道时）。 */
  track?: number;
}>();

const emit = defineEmits<{
  (e: "commit", id: string, value: number): void;
  /** 点预设垫子 / 预设条：整组参数一次写回（只占一个撤销步骤）。 */
  (e: "applyPreset", params: Record<string, number>): void;
}>();

const { t } = useI18n();

/**
 * 容器宽度不够时整体等比缩小。
 *
 * 设备永远是 900×560，缩放只做一层 `transform: scale`——所以键盘（乃至任何一块）的比例都不会变形。
 * 超过设计宽度就保持 1，不放大。`.fit` 的高度跟着缩，免得缩完在容器里留一大片空白。
 */
const fitEl = ref<HTMLElement | null>(null);
const scale = ref(1);
let ro: ResizeObserver | null = null;

function measure(): void {
  const el = fitEl.value;
  if (!el) return;
  // 分母是常量 `DEVICE_W`，不存在为 0 的情况。
  scale.value = Math.min(1, el.clientWidth / DEVICE_W);
}

onMounted(() => {
  measure();
  ro = new ResizeObserver(measure);
  if (fitEl.value) ro.observe(fitEl.value);
});

onUnmounted(() => ro?.disconnect());

const SPECS: Record<string, ParamSpec> = Object.fromEntries(SYNTH_PARAMS.map((p) => [p.id, p]));

/** 拖动中的草稿：面板是受控的，没有草稿旋钮不会跟手。 */
const drafts = ref<Record<string, number>>({});

function base(id: string): number {
  return props.params[id] ?? SPECS[id]?.default ?? 0;
}

function shown(id: string): number {
  return drafts.value[id] ?? base(id);
}

function onLive(id: string, value: number): void {
  drafts.value[id] = value;
}

function onCommit(id: string, value: number): void {
  emit("commit", id, value);
}

// 工程里的值追上草稿后就把草稿丢掉（否则会一直压着后续从别处来的修改）。
watch(
  () => props.params,
  (next) => {
    for (const [id, value] of Object.entries(drafts.value)) {
      if (next[id] === value) delete drafts.value[id];
    }
  },
  { deep: true },
);

/** 旋钮的显示文本：按参数表的小数位与单位格式化。 */
function display(id: string, value: number): string {
  const spec = SPECS[id];
  if (!spec) return value.toFixed(2);
  const digits = spec.digits ?? 2;
  return `${value.toFixed(digits)}${spec.unit ?? ""}`;
}

/** 参数 id → 显示名（i18n 键）。没列到的直接退回 id，至少不会显示成空。 */
const LABELS: Record<string, string> = {
  wave: "synth.wave",
  sub: "synth.sub",
  pwm: "synth.pwm",
  glide: "synth.glide",
  hpf: "synth.hpf",
  // 面板上露出的显示名走小白化词典（设计稿 §4.2）：截止→亮度、共振→尖锐度、包络量→滤波包络。
  // 改指向后 `synth.cutoff` / `synth.resonance` / `synth.envAmount` 已无代码引用（成为孤儿键，
  // 保留不删，避免误伤将来可能的复用）；值不动，这里只换旋钮指向的键。
  cutoff: "synth.brightness",
  resonance: "synth.sharpness",
  filterEnv: "synth.filterEnvName",
  keyFollow: "synth.keyFollow",
  attack: "synth.attack",
  decay: "synth.decay",
  sustain: "synth.sustain",
  release: "synth.release",
  lfoRate: "synth.lfoRate",
  lfoDelay: "synth.lfoDelay",
  lfoPitch: "synth.lfoPitch",
  lfoFilter: "synth.lfoFilter",
  gain: "synth.gain",
};

/** 一个旋钮需要的全部信息（模板只遍历这份数据，不手写 18 份旋钮）。 */
interface KnobDef {
  id: string;
  label: string;
  min: number;
  max: number;
  default: number;
  size?: number;
}

function knob(id: string, size?: number): KnobDef {
  const spec = SPECS[id];
  return {
    id,
    label: t(LABELS[id] ?? id),
    min: spec?.min ?? 0,
    max: spec?.max ?? 1,
    default: spec?.default ?? 0,
    size,
  };
}

/** 基础面板：只有这 8 个参数常驻（波形单独用分段按钮渲染）。 */
const BASIC_IDS = ["wave", "cutoff", "resonance", "attack", "decay", "sustain", "release", "gain"];

/** 高级区：按老式减法合成器的分区组织（DCO / HPF / VCF / LFO）。 */
const ADVANCED_GROUPS: { title: string; ids: string[] }[] = [
  { title: "synth.osc", ids: ["sub", "pwm", "glide"] },
  { title: "synth.hpf", ids: ["hpf"] },
  { title: "synth.filter", ids: ["filterEnv", "keyFollow"] },
  { title: "synth.lfo", ids: ["lfoRate", "lfoDelay", "lfoPitch", "lfoFilter"] },
];

/** 高级区是否展开（只存组件内，不持久化）。 */
const advanced = ref(false);

const basicKnobs = computed(() =>
  BASIC_IDS.filter((id) => id !== "wave").map((id) =>
    // 展开高级区时基础旋钮一并收小：设备高度固定，只能这样给高级卡片腾出整行的地方。
    advanced.value ? knob(id, 46) : knob(id, id === "gain" ? 72 : 62),
  ),
);

/** 高级区：按 `ADVANCED_GROUPS` 摊成「分区标题 + 旋钮」；卡里的旋钮更小，
    免得展开后把固定高度的面板撑得要滚很久。 */
const advancedGroups = computed(() =>
  ADVANCED_GROUPS.map((group) => ({
    title: t(group.title),
    knobs: group.ids.map((id) => knob(id, 46)),
  })),
);

/**
 * 波形分段按钮上的小图标：一条 `viewBox="0 0 20 12"` 的示意曲线，纯内联 SVG（`currentColor`），
 * 与按钮文字同色、随选中态一起变色，不引图标库。
 */
const WAVES = [
  {
    value: 0,
    key: "synth.wave.saw",
    // 锯齿：线性上升 + 竖直回落
    d: "M1 9 L6 3 L6 9 L11 3 L11 9 L16 3 L16 9",
  },
  {
    value: 1,
    key: "synth.wave.square",
    // 方波：平顶 + 竖直翻转
    d: "M1 9 L4 9 L4 3 L9 3 L9 9 L14 9 L14 3 L18 3",
  },
  {
    value: 2,
    key: "synth.wave.sine",
    // 正弦：两段二次曲线（T 自动镜像控制点）
    d: "M1 6 Q3 1 5 6 T9 6 T13 6 T17 6",
  },
  {
    value: 3,
    key: "synth.wave.triangle",
    // 三角：折线
    d: "M1 9 L5 3 L9 9 L13 3 L17 9",
  },
];

const wave = computed(() => Math.round(shown("wave")));

function setWave(value: number): void {
  onLive("wave", value);
  onCommit("wave", value);
}

/** 屏幕上的波形由这几个参数决定（与真实 DSP 的形状对应）。 */
const scopeParams = computed<WaveParams>(() => ({
  wave: shown("wave"),
  sub: shown("sub"),
  pwm: shown("pwm"),
  cutoff: shown("cutoff"),
  resonance: shown("resonance"),
}));

/**
 * ADSR 包络曲线。
 *
 * 时间轴用平方根压缩，否则长释音会把起音压成一条竖线。延音段画一个固定的示意长度。
 */
const envelope = computed(() => {
  const a = Math.sqrt(shown("attack") / 1);
  const d = Math.sqrt(shown("decay") / 2);
  const s = shown("sustain");
  const r = Math.sqrt(shown("release") / 4);
  const hold = 0.35;
  const total = a + d + hold + r || 1;
  const x = (v: number) => (v / total) * 100;
  const y = (v: number) => 30 - v * 26;
  const points = [
    [0, y(0)],
    [x(a), y(1)],
    [x(a + d), y(s)],
    [x(a + d + hold), y(s)],
    [x(a + d + hold + r), y(0)],
  ];
  return points.map(([px, py]) => `${px.toFixed(2)},${py.toFixed(2)}`).join(" ");
});

// ---- 预设垫子 ----
const presets = usePresetsStore();
/** 当前翻看的类别（出厂预设的 slug）。自存预设没有类别，归到「主音」那一档里。 */
const coarse = ref("lead");
const options = computed(() => presets.forPlugin(SYNTH_ID));

/** 有哪几个类别（按出厂预设的顺序去重）。 */
const coarseOptions = computed(() => {
  const out: string[] = [];
  for (const option of options.value) {
    const slug = option.category ?? "lead";
    if (!out.includes(slug)) out.push(slug);
  }
  return out;
});

/** 当前类别下的垫子。 */
const pads = computed(() => options.value.filter((o) => (o.category ?? "lead") === coarse.value));

/** 当前参数命中的那条预设（用来高亮垫子）。 */
const activePresetId = computed(() => presets.match(SYNTH_ID, props.params)?.id ?? "");

/** 切预设：把整组参数交给父组件写回当前轨道（只有它知道是哪条轨道）。 */
function applyPreset(values: Record<string, number>): void {
  emit("applyPreset", values);
}

function pickPad(option: PresetOption): void {
  applyPreset(option.params);
}
</script>

<template>
  <div ref="fitEl" class="fit" :style="{ height: `${DEVICE_H * scale}px` }">
    <div class="device" :style="{ transform: `scale(${scale})`, transformOrigin: 'top center' }">
      <!-- 顶部：预设垫子（当前类别的几条，点一下就切）＋ 类别 chips ＋ 预设条 -->
      <section class="presets">
        <div class="pads" role="group" :aria-label="t('preset.label')">
          <button
            v-for="pad in pads"
            :key="pad.id"
            class="pad"
            :class="{ on: pad.id === activePresetId }"
            :aria-pressed="pad.id === activePresetId"
            :title="pad.note || pad.name"
            @click="pickPad(pad)"
          >
            <span class="pad-name">{{ pad.name }}</span>
            <span v-if="pad.note" class="pad-note">{{ pad.note }}</span>
          </button>
        </div>

        <div class="preset-foot">
          <div class="chips" role="group" :aria-label="t('preset.choose')">
            <button
              v-for="slug in coarseOptions"
              :key="slug"
              class="chip"
              :class="{ on: slug === coarse }"
              :aria-pressed="slug === coarse"
              @click="coarse = slug"
            >
              {{ t(`presetCategory.${slug}`) }}
            </button>
          </div>
          <div class="bar">
            <PresetBar :plugin-id="SYNTH_ID" :params="props.params" @apply="applyPreset" />
          </div>
        </div>
      </section>

      <!-- 中部：左边屏幕（一个周期示波器 + ADSR），右边基础旋钮 + 「高级」 -->
      <section class="middle">
        <div class="screen">
          <Oscilloscope :params="scopeParams" :height="170" />
          <div class="env">
            <svg class="env-svg" viewBox="0 0 100 34" preserveAspectRatio="none" aria-hidden="true">
              <polyline :points="envelope" />
            </svg>
            <span class="env-label">{{ t("synth.envelope") }}</span>
          </div>
          <span class="voice-count">{{ t("synth.voiceCount", { count: 16 }) }}</span>
        </div>

        <div class="controls">
          <div class="controls-scroll">
            <div class="waves" role="group" :aria-label="t('synth.wave')">
              <button
                v-for="w in WAVES"
                :key="w.value"
                class="seg"
                :class="{ on: wave === w.value }"
                :aria-pressed="wave === w.value"
                @click="setWave(w.value)"
              >
                <svg class="seg-icon" viewBox="0 0 20 12" aria-hidden="true" focusable="false">
                  <path :d="w.d" />
                </svg>
                <span class="seg-text">{{ t(w.key) }}</span>
              </button>
            </div>

            <div class="knobs basic" :class="{ compact: advanced }">
              <Md3Knob
                v-for="k in basicKnobs"
                :key="k.id"
                :model-value="shown(k.id)"
                :min="k.min"
                :max="k.max"
                :default="k.default"
                :size="k.size"
                :label="k.label"
                :display="display(k.id, shown(k.id))"
                @update:model-value="(v: number) => onLive(k.id, v)"
                @commit="(v: number) => onCommit(k.id, v)"
              />
            </div>

            <!-- 高级区：默认收起，点「高级」才出现（弹簧动画展开） -->
            <transition name="adv">
              <div v-if="advanced" class="advanced">
                <section v-for="group in advancedGroups" :key="group.title" class="group">
                  <div class="group-title">{{ group.title }}</div>
                  <div class="knobs">
                    <Md3Knob
                      v-for="k in group.knobs"
                      :key="k.id"
                      :model-value="shown(k.id)"
                      :min="k.min"
                      :max="k.max"
                      :default="k.default"
                      :size="k.size"
                      :label="k.label"
                      :display="display(k.id, shown(k.id))"
                      @update:model-value="(v: number) => onLive(k.id, v)"
                      @commit="(v: number) => onCommit(k.id, v)"
                    />
                  </div>
                </section>
              </div>
            </transition>
          </div>

          <button
            class="btn-tonal advanced-toggle"
            :class="{ on: advanced }"
            :aria-pressed="advanced"
            @click="advanced = !advanced"
          >
            {{ t("synth.advanced") }}
          </button>
        </div>
      </section>

      <!-- 底部：固定高度键盘（`flex: 0 0 auto`，不参与压缩，比例永不变形） -->
      <div v-if="props.track !== undefined" class="keyboard-row">
        <PianoKeyboard :track="props.track" />
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 外层负责居中与等比缩放：高度由内联样式跟着 `scale` 走。
   `overflow: hidden` 兜住 `transform` 不改变布局盒子这件事——缩到一半时设备右边缘不会溢出去。 */
.fit {
  width: 100%;
  display: flex;
  justify-content: center;
  overflow: hidden;
}

/* 一台固定尺寸的设备面板：内部一律固定高度 / `flex: 0 0 auto`，键盘不会被压扁。
   `flex: 0 0 auto` 是必须的——否则 `.fit` 这个 flex 容器一窄就把设备挤扁，
   等比缩放（Task 9）要的正是「设备永远是 900×560、只整体缩放」。 */
.device {
  width: v-bind(DEVICE_W + 'px');
  height: v-bind(DEVICE_H + 'px');
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

/* ---- 顶部：预设垫子 ---- */
.presets {
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.pads {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 8px;
}
/* 垫子：大圆角方块，像硬件上的打击垫 */
.pad {
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 3px;
  height: 60px;
  padding: 8px 12px;
  overflow: hidden;
  border: 0;
  border-radius: 20px;
  background-color: rgb(var(--md-surface-container-high));
  color: rgb(var(--md-on-surface));
  text-align: left;
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease;
}
.pad:hover {
  background-color: rgb(var(--md-surface-container-highest));
}
.pad.on {
  background-color: rgb(var(--md-primary-container));
  color: rgb(var(--md-on-primary-container));
}
.pad-name {
  font-size: 12px;
  font-weight: 600;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
.pad-note {
  display: -webkit-box;
  overflow: hidden;
  font-size: 10px;
  line-height: 1.4;
  color: rgb(var(--md-on-surface-variant));
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
}
.pad.on .pad-note {
  color: rgb(var(--md-on-primary-container) / 0.8);
}

.preset-foot {
  display: flex;
  align-items: center;
  gap: 10px;
}
.chips {
  flex: 0 0 auto;
  display: flex;
  gap: 6px;
}
/* 类别筛选：MD3 filter chip */
.chip {
  height: 30px;
  padding: 0 12px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.8);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 11.5px;
  white-space: nowrap;
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease, border-color 0.14s ease;
}
.chip:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.chip.on {
  border-color: transparent;
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.bar {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
}

/* ---- 中部：屏幕 + 基础旋钮 ---- */
.middle {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  gap: 12px;
}
.screen {
  flex: 0 0 330px;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.env {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.env-svg {
  width: 100%;
  height: 44px;
  /* 和上面的示波器同一套圆角，两块「屏幕」看起来是一体的 */
  border-radius: 20px;
  background-color: rgb(var(--md-scrim) / 0.35);
}
.env-svg polyline {
  fill: none;
  stroke: rgb(var(--md-primary));
  stroke-width: 2;
  vector-effect: non-scaling-stroke;
  stroke-linejoin: round;
  stroke-linecap: round;
}
.env-label {
  font-size: 9.5px;
  letter-spacing: 0.4px;
  color: rgb(var(--md-on-surface-muted));
}
.voice-count {
  margin-top: auto;
  text-align: right;
  font-size: 9.5px;
  letter-spacing: 0.4px;
  color: rgb(var(--md-on-surface-muted));
}

.controls {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
}
/* 高级区展开后内容会超过固定高度，这里内部滚动（设备本身尺寸不变）。 */
.controls-scroll {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding-right: 2px;
  overflow-y: auto;
}

/* 波形：MD3 分段按钮 */
.waves {
  flex: 0 0 auto;
  display: flex;
  height: 34px;
  overflow: hidden;
  border: 1px solid rgb(var(--md-outline-variant) / 0.8);
  border-radius: 999px;
}
.seg {
  flex: 1 1 0;
  min-width: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  border: 0;
  border-left: 1px solid rgb(var(--md-outline-variant) / 0.5);
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 11.5px;
  white-space: nowrap;
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease;
}
.seg:first-child {
  border-left: 0;
}
.seg:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.seg.on {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
/* 波形小图标：跟着按钮文字变色（`currentColor`），选出「哪条是波形本身」 */
.seg-icon {
  flex: 0 0 auto;
  width: 20px;
  height: 12px;
  overflow: visible;
}
.seg-icon path {
  fill: none;
  stroke: currentColor;
  stroke-width: 1.4;
  stroke-linecap: round;
  stroke-linejoin: round;
}

.knobs {
  display: flex;
  flex-wrap: wrap;
  align-content: flex-start;
  justify-content: space-around;
  gap: 8px 4px;
}
/* 基础旋钮：一行 4 颗、铺两行占住固定面板的高度（最后一行的 3 颗由 space-around 居中）。
   展开高级区时压成一行，把位置让给高级卡片——设备高度不变，所以靠这个腾地方。 */
.basic > * {
  flex: 0 0 128px;
}
.basic.compact > * {
  flex: 1 1 0;
}

/* 高级区：老式减法合成器的分区卡片 */
.advanced {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 10px;
}
/* 展开/收起：MD3 弹簧（设计稿 §4.2 的「弹簧动画」），从上方略缩略降地弹入 */
.adv-enter-active,
.adv-leave-active {
  transition: opacity var(--md-spring-fast), transform var(--md-spring-fast);
  transform-origin: top center;
}
.adv-enter-from,
.adv-leave-to {
  opacity: 0;
  transform: translateY(-8px) scale(0.98);
}
/* 系统开了「减弱动态效果」就直出、直隐 */
@media (prefers-reduced-motion: reduce) {
  .adv-enter-active,
  .adv-leave-active {
    transition: none;
  }
  .adv-enter-from,
  .adv-leave-to {
    opacity: 1;
    transform: none;
  }
}
.group {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 10px 12px;
  border-radius: 20px;
  background-color: rgb(var(--md-surface-container-low));
}
.group-title {
  padding-left: 4px;
  font-size: 10px;
  letter-spacing: 1px;
  text-transform: uppercase;
  color: rgb(var(--md-primary));
}

/* 「高级」开关（`btn-tonal` 来自全局样式，这里只管尺寸与展开态） */
.advanced-toggle {
  flex: 0 0 auto;
  align-self: flex-end;
  height: 36px;
  padding: 0 20px;
  font-size: 12.5px;
}
.advanced-toggle.on {
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}

/* ---- 底部：键盘 ---- */
.keyboard-row {
  flex: 0 0 auto;
}

.pad:focus-visible,
.chip:focus-visible,
.seg:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
/* 分段按钮在 overflow: hidden 的容器里，焦点环画在里面才不会被裁掉 */
.seg:focus-visible {
  outline-offset: -2px;
}
</style>
