<script setup lang="ts">
/**
 * 内置合成器的**插件级独立界面**。
 *
 * 按一台真实减法合成器的面板布局组织（老式模拟合成器的分区顺序）：
 * `DCO 振荡器 → HPF 高通 → VCF 低通 → VCA 包络 → LFO 调制 → 总输出`。
 * 每个分区是一张卡片，控件是旋钮——不是「一列参数滑块」。
 *
 * 参数值来自工程的 `instrument.params`（缺省回落到参数表默认值）；拖动时用本地草稿保证跟手，
 * 松手才提交给引擎。
 */
import { computed, ref, watch } from "vue";

import Md3Knob from "@/components/common/Md3Knob.vue";
import PianoKeyboard from "@/components/common/PianoKeyboard.vue";
import WaveformDisplay from "@/components/common/WaveformDisplay.vue";
import { SYNTH_PARAMS, type ParamSpec } from "@/data/plugins";
import { useI18n } from "@/i18n";

const props = defineProps<{
  params: Record<string, number>;
  /** 试听键盘要发给哪条轨道；不给就不显示键盘（比如侧边栏里没选中轨道时）。 */
  track?: number;
}>();

const emit = defineEmits<{ (e: "commit", id: string, value: number): void }>();

const { t } = useI18n();

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

const WAVES = [
  { value: 0, key: "synth.wave.saw" },
  { value: 1, key: "synth.wave.square" },
  { value: 2, key: "synth.wave.sine" },
  { value: 3, key: "synth.wave.triangle" },
];

const wave = computed(() => Math.round(shown("wave")));

function setWave(value: number): void {
  onLive("wave", value);
  onCommit("wave", value);
}

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
</script>

<template>
  <div class="synth">
    <!-- 上面一排：显示屏 + 控件分组；下面一整条键盘 -->
    <div class="face">
      <div class="screen">
        <WaveformDisplay :height="150" />
        <span class="voice-count">{{ t("synth.voiceCount", { count: 16 }) }}</span>
      </div>

      <div class="sections">
      <!-- DCO -->
      <section class="section">
        <div class="section-title">{{ t("synth.osc") }}</div>
        <div class="waves" role="group" :aria-label="t('synth.wave')">
          <button
            v-for="w in WAVES"
            :key="w.value"
            class="wave"
            :class="{ on: wave === w.value }"
            :aria-pressed="wave === w.value"
            @click="setWave(w.value)"
          >
            {{ t(w.key) }}
          </button>
        </div>
        <div class="knobs">
          <Md3Knob
            :model-value="shown('sub')"
            :min="SPECS.sub.min"
            :max="SPECS.sub.max"
            :default="SPECS.sub.default"
            :label="t('synth.sub')"
            :display="display('sub', shown('sub'))"
            @update:model-value="(v: number) => onLive('sub', v)"
            @commit="(v: number) => onCommit('sub', v)"
          />
          <Md3Knob
            :model-value="shown('pwm')"
            :min="SPECS.pwm.min"
            :max="SPECS.pwm.max"
            :default="SPECS.pwm.default"
            :label="t('synth.pwm')"
            :display="display('pwm', shown('pwm'))"
            @update:model-value="(v: number) => onLive('pwm', v)"
            @commit="(v: number) => onCommit('pwm', v)"
          />
          <Md3Knob
            :model-value="shown('glide')"
            :min="SPECS.glide.min"
            :max="SPECS.glide.max"
            :default="SPECS.glide.default"
            :label="t('synth.glide')"
            :display="display('glide', shown('glide'))"
            @update:model-value="(v: number) => onLive('glide', v)"
            @commit="(v: number) => onCommit('glide', v)"
          />
        </div>
      </section>

      <!-- HPF -->
      <section class="section">
        <div class="section-title">{{ t("synth.hpf") }}</div>
        <div class="knobs">
          <Md3Knob
            :model-value="shown('hpf')"
            :min="SPECS.hpf.min"
            :max="SPECS.hpf.max"
            :default="SPECS.hpf.default"
            :label="t('synth.cutoff')"
            :display="display('hpf', shown('hpf'))"
            @update:model-value="(v: number) => onLive('hpf', v)"
            @commit="(v: number) => onCommit('hpf', v)"
          />
        </div>
      </section>

      <!-- VCF -->
      <section class="section">
        <div class="section-title">{{ t("synth.filter") }}</div>
        <div class="knobs">
          <Md3Knob
            :model-value="shown('cutoff')"
            :min="SPECS.cutoff.min"
            :max="SPECS.cutoff.max"
            :default="SPECS.cutoff.default"
            :label="t('synth.cutoff')"
            :display="display('cutoff', shown('cutoff'))"
            @update:model-value="(v: number) => onLive('cutoff', v)"
            @commit="(v: number) => onCommit('cutoff', v)"
          />
          <Md3Knob
            :model-value="shown('resonance')"
            :min="SPECS.resonance.min"
            :max="SPECS.resonance.max"
            :default="SPECS.resonance.default"
            :label="t('synth.resonance')"
            :display="display('resonance', shown('resonance'))"
            @update:model-value="(v: number) => onLive('resonance', v)"
            @commit="(v: number) => onCommit('resonance', v)"
          />
          <Md3Knob
            :model-value="shown('filterEnv')"
            :min="SPECS.filterEnv.min"
            :max="SPECS.filterEnv.max"
            :default="SPECS.filterEnv.default"
            :label="t('synth.envAmount')"
            :display="display('filterEnv', shown('filterEnv'))"
            @update:model-value="(v: number) => onLive('filterEnv', v)"
            @commit="(v: number) => onCommit('filterEnv', v)"
          />
          <Md3Knob
            :model-value="shown('keyFollow')"
            :min="SPECS.keyFollow.min"
            :max="SPECS.keyFollow.max"
            :default="SPECS.keyFollow.default"
            :label="t('synth.keyFollow')"
            :display="display('keyFollow', shown('keyFollow'))"
            @update:model-value="(v: number) => onLive('keyFollow', v)"
            @commit="(v: number) => onCommit('keyFollow', v)"
          />
        </div>
        <div class="scope">
          <svg class="scope-svg" viewBox="0 0 100 34" preserveAspectRatio="none">
            <polyline :points="envelope" />
          </svg>
          <span class="scope-label">{{ t("synth.envelope") }}</span>
        </div>
      </section>

      <!-- VCA -->
      <section class="section">
        <div class="section-title">{{ t("synth.vca") }}</div>
        <div class="knobs">
          <Md3Knob
            :model-value="shown('attack')"
            :min="SPECS.attack.min"
            :max="SPECS.attack.max"
            :default="SPECS.attack.default"
            :label="t('synth.attack')"
            :display="display('attack', shown('attack'))"
            @update:model-value="(v: number) => onLive('attack', v)"
            @commit="(v: number) => onCommit('attack', v)"
          />
          <Md3Knob
            :model-value="shown('decay')"
            :min="SPECS.decay.min"
            :max="SPECS.decay.max"
            :default="SPECS.decay.default"
            :label="t('synth.decay')"
            :display="display('decay', shown('decay'))"
            @update:model-value="(v: number) => onLive('decay', v)"
            @commit="(v: number) => onCommit('decay', v)"
          />
          <Md3Knob
            :model-value="shown('sustain')"
            :min="SPECS.sustain.min"
            :max="SPECS.sustain.max"
            :default="SPECS.sustain.default"
            :label="t('synth.sustain')"
            :display="display('sustain', shown('sustain'))"
            @update:model-value="(v: number) => onLive('sustain', v)"
            @commit="(v: number) => onCommit('sustain', v)"
          />
          <Md3Knob
            :model-value="shown('release')"
            :min="SPECS.release.min"
            :max="SPECS.release.max"
            :default="SPECS.release.default"
            :label="t('synth.release')"
            :display="display('release', shown('release'))"
            @update:model-value="(v: number) => onLive('release', v)"
            @commit="(v: number) => onCommit('release', v)"
          />
        </div>
      </section>

      <!-- LFO -->
      <section class="section">
        <div class="section-title">{{ t("synth.lfo") }}</div>
        <div class="knobs">
          <Md3Knob
            :model-value="shown('lfoRate')"
            :min="SPECS.lfoRate.min"
            :max="SPECS.lfoRate.max"
            :default="SPECS.lfoRate.default"
            :label="t('synth.lfoRate')"
            :display="display('lfoRate', shown('lfoRate'))"
            @update:model-value="(v: number) => onLive('lfoRate', v)"
            @commit="(v: number) => onCommit('lfoRate', v)"
          />
          <Md3Knob
            :model-value="shown('lfoDelay')"
            :min="SPECS.lfoDelay.min"
            :max="SPECS.lfoDelay.max"
            :default="SPECS.lfoDelay.default"
            :label="t('synth.lfoDelay')"
            :display="display('lfoDelay', shown('lfoDelay'))"
            @update:model-value="(v: number) => onLive('lfoDelay', v)"
            @commit="(v: number) => onCommit('lfoDelay', v)"
          />
          <Md3Knob
            :model-value="shown('lfoPitch')"
            :min="SPECS.lfoPitch.min"
            :max="SPECS.lfoPitch.max"
            :default="SPECS.lfoPitch.default"
            :label="t('synth.lfoPitch')"
            :display="display('lfoPitch', shown('lfoPitch'))"
            @update:model-value="(v: number) => onLive('lfoPitch', v)"
            @commit="(v: number) => onCommit('lfoPitch', v)"
          />
          <Md3Knob
            :model-value="shown('lfoFilter')"
            :min="SPECS.lfoFilter.min"
            :max="SPECS.lfoFilter.max"
            :default="SPECS.lfoFilter.default"
            :label="t('synth.lfoFilter')"
            :display="display('lfoFilter', shown('lfoFilter'))"
            @update:model-value="(v: number) => onLive('lfoFilter', v)"
            @commit="(v: number) => onCommit('lfoFilter', v)"
          />
        </div>
      </section>

      <!-- 总输出 -->
      <section class="section">
        <div class="section-title">{{ t("synth.amp") }}</div>
        <div class="knobs">
          <Md3Knob
            :model-value="shown('gain')"
            :min="SPECS.gain.min"
            :max="SPECS.gain.max"
            :default="SPECS.gain.default"
            :size="54"
            :label="t('synth.gain')"
            :display="display('gain', shown('gain'))"
            @update:model-value="(v: number) => onLive('gain', v)"
            @commit="(v: number) => onCommit('gain', v)"
          />
        </div>
      </section>
      </div>
    </div>

    <PianoKeyboard v-if="props.track !== undefined" :track="props.track" />
  </div>
</template>

<style scoped>
.synth {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
/* 上排：左边显示屏，右边控件分组（窄容器里自动换行） */
.face {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-start;
  gap: 12px;
}
.screen {
  flex: 1 1 210px;
  max-width: 320px;
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.voice-count {
  text-align: right;
  font-size: 9.5px;
  letter-spacing: 0.4px;
  color: rgb(var(--md-on-surface-muted));
}
/* 控件区：等宽格子铺开，像一台设备的控制面板 */
.sections {
  flex: 3 1 420px;
  min-width: 0;
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(148px, 1fr));
  gap: 10px;
  align-content: start;
}
.section {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;
  padding: 12px 10px 14px;
  border-radius: 20px;
  background-color: var(--app-inset);
}
.section-title {
  font-size: 10px;
  letter-spacing: 1px;
  text-transform: uppercase;
  color: rgb(var(--md-primary));
  padding-left: 6px;
}
.waves {
  display: flex;
  flex-wrap: wrap;
  gap: 5px;
  padding: 0 2px;
}
.wave {
  flex: 1 1 auto;
  height: 28px;
  padding: 0 10px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.8);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 11px;
  cursor: pointer;
  white-space: nowrap;
  transition: background-color 0.14s ease, color 0.14s ease, border-color 0.14s ease;
}
.wave:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.wave.on {
  background-color: rgb(var(--md-secondary-container));
  border-color: transparent;
  color: rgb(var(--md-on-secondary-container));
}
.wave:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.knobs {
  display: flex;
  flex-wrap: wrap;
  gap: 10px 4px;
  justify-content: space-around;
  flex: 1 1 auto;
  align-content: flex-start;
}
.scope {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 0 4px;
}
.scope-svg {
  width: 100%;
  height: 40px;
}
.scope-svg polyline {
  fill: none;
  stroke: rgb(var(--md-primary));
  stroke-width: 2;
  vector-effect: non-scaling-stroke;
  stroke-linejoin: round;
  stroke-linecap: round;
}
.scope-label {
  font-size: 9.5px;
  letter-spacing: 0.4px;
  color: rgb(var(--md-on-surface-muted));
}
</style>
