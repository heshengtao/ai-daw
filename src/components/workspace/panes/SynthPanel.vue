<script setup lang="ts">
/**
 * 内置合成器的**面板**：预设垫子 + 屏幕（示波器 / 包络）+ 旋钮 + 试听键盘。
 *
 * **面板铺满宿主容器，自己不带外框**：独立窗口里它直接贴到窗口边缘（键盘贴死底边，
 * 圆角由窗口本身切出），嵌在侧边栏里时也是铺满。所以没有外层大圆角容器、没有留白缝隙。
 *
 * 旋钮分两层：**基础**是两排 4×2 共 8 个最常用的，其余（副振 / 脉宽 / 滑音 / 失谐 / FM /
 * 噪声 / 压碎 / 音高包络 / 滤波包络 / 键跟随 / LFO）收在「高级」里，默认不打扰小白。
 *
 * 参数值来自工程的 `instrument.params`（缺省回落到参数表默认值）；拖动时用本地草稿保证跟手，
 * 松手才提交给引擎。
 */
import { computed, ref, watch } from "vue";

import Md3Knob from "@/components/common/Md3Knob.vue";
import Oscilloscope from "@/components/common/Oscilloscope.vue";
import PianoKeyboard from "@/components/common/PianoKeyboard.vue";
import PresetBar from "@/components/common/PresetBar.vue";
import { SYNTH_DEFAULTS, SYNTH_ID, SYNTH_PARAMS, type ParamSpec } from "@/data/plugins";
import { useI18n } from "@/i18n";
import { usePresetsStore, type PresetOption } from "@/stores/presets";
import type { WaveParams } from "@/utils/waveform";

const props = defineProps<{
  params: Record<string, number>;
  /** 试听键盘要发给哪条轨道；不给就不显示键盘（比如侧边栏里没选中轨道时）。 */
  track?: number;
}>();

const emit = defineEmits<{
  (e: "commit", id: string, value: number): void;
  /**
   * 一次提交**多个**参数。包络控制点就会同时改两个（衰减点 = 衰减 + 延音），
   * 分两次提交会变成**两个撤销步骤**——拖一下要按两次 Cmd+Z 才回得去，所以必须合成一次。
   */
  (e: "commitMany", params: Record<string, number>): void;
  /** 点预设垫子 / 预设条：整组**乐器参数**一次写回（只占一个撤销步骤；不碰效果链）。 */
  (e: "applyPreset", params: Record<string, number>): void;
}>();

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
  // 扩展音源能力
  noise: "synth.noise",
  crush: "synth.crush",
  detune: "synth.detune",
  fm: "synth.fm",
  fmRatio: "synth.fmRatio",
  pitchEnv: "synth.pitchEnv",
  pitchDecay: "synth.pitchDecay",
  filterType: "synth.filterType",
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

/**
 * 基础面板：8 个最常用的参数，排成两排 4×2（波形单独用分段按钮渲染）。
 *
 * 选的是「任何音色都先拧这几个」的那批：亮度 / 尖锐度 / 四个包络段 / 失谐（厚度）/ 总音量。
 */
const BASIC_IDS = [
  "cutoff",
  "resonance",
  "attack",
  "decay",
  "sustain",
  "release",
  "detune",
  "gain",
];

/** 高级区：按合成器信号链的顺序分区（振荡器 → FM → 噪声/位深 → 音高包络 → 滤波 → LFO）。 */
const ADVANCED_GROUPS: { title: string; ids: string[] }[] = [
  { title: "synth.osc", ids: ["sub", "pwm", "glide", "detune"] },
  { title: "synth.fm", ids: ["fm", "fmRatio"] },
  { title: "synth.noise", ids: ["noise", "crush"] },
  { title: "synth.pitchEnv", ids: ["pitchEnv", "pitchDecay"] },
  { title: "synth.hpf", ids: ["hpf"] },
  { title: "synth.filter", ids: ["filterEnv", "keyFollow"] },
  { title: "synth.lfo", ids: ["lfoRate", "lfoDelay", "lfoPitch", "lfoFilter"] },
];

/**
 * 滤波类型：三选一的枚举，用旋钮很难选，所以做成分段按钮（与波形同一套样式）。
 * 值必须与 Rust 侧 `FilterType::from_param` 一致：0/1/2 = 低通/带通/高通。
 */
const FILTER_TYPES = [
  { value: 0, key: "synth.filterType.lp" },
  { value: 1, key: "synth.filterType.bp" },
  { value: 2, key: "synth.filterType.hp" },
];

/** 高级区是否展开（只存组件内，不持久化）。 */
const advanced = ref(false);

/**
 * 试听键盘的音域。
 *
 * **默认落在中音区**：从 C2（MIDI 36）起四个八度 = C2..C6，正好以中央 C（C4）为中心，
 * 是「中间、略偏低」的那一块。之前从 C4 起往上四个八度（C4..C8）——太高了，
 * 试听低音和和弦都不对劲。
 */
const KB_BASE = 36;
const KB_OCTAVES = 4;
/** 八度位移上限：再低/再高就出 MIDI 音域了。 */
const KB_SHIFT_MAX = 2;

/** 八度位移：鼠标与电脑键盘**一起**走（两边锚在同一个起始音上）。 */
const octaveShift = ref(0);
const keyboardBase = computed(() => KB_BASE + octaveShift.value * 12);

/** 夹在 ±KB_SHIFT_MAX 内，免得把键盘推到 MIDI 音域外。 */
function shiftOctave(by: number): void {
  octaveShift.value = Math.max(-KB_SHIFT_MAX, Math.min(KB_SHIFT_MAX, octaveShift.value + by));
}
const canShiftDown = computed(() => octaveShift.value > -KB_SHIFT_MAX);
const canShiftUp = computed(() => octaveShift.value < KB_SHIFT_MAX);

const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/** MIDI 音高 → 音名（中央 C = C4 = 60）。 */
function noteName(note: number): string {
  return `${NOTE_NAMES[note % 12]}${Math.floor(note / 12) - 1}`;
}

/** 当前音域，写成 `C2 – C6`。 */
const keyboardRange = computed(
  () => `${noteName(keyboardBase.value)} – ${noteName(keyboardBase.value + KB_OCTAVES * 12)}`,
);

const basicKnobs = computed(() =>
  BASIC_IDS.map((id) =>
    // 展开高级区时基础旋钮一并收小：给高级卡片腾出整行的地方。
    advanced.value ? knob(id, 46) : knob(id, id === "gain" ? 72 : 62),
  ),
);

/** 全部参数都还在插件默认值上（恢复默认按钮就没必要点）。 */
const atDefaults = computed(() => SYNTH_PARAMS.every((p) => shown(p.id) === p.default));

/**
 * 恢复默认参数。
 *
 * 只动**乐器参数**，不碰效果链——效果链是音轨的东西，不是这台合成器的一部分。
 * 走 `commitMany`：一次事务，只占一个撤销步骤。
 */
function restoreDefaults(): void {
  drafts.value = {};
  emit("commitMany", { ...SYNTH_DEFAULTS });
}

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

const filterType = computed(() => Math.round(shown("filterType")));

function setFilterType(value: number): void {
  onLive("filterType", value);
  onCommit("filterType", value);
}

/** 屏幕上的波形由这几个参数决定（与真实 DSP 的形状对应）。 */
const scopeParams = computed<WaveParams>(() => ({
  wave: shown("wave"),
  sub: shown("sub"),
  pwm: shown("pwm"),
  cutoff: shown("cutoff"),
  resonance: shown("resonance"),
  noise: shown("noise"),
  crush: shown("crush"),
}));

/**
 * ADSR 包络：**三个可直接拖动的控制点**（起音 / 衰减+延音 / 释音）。
 *
 * 横轴刻意用**固定分段**（起音 0–28%、衰减 28–50%、延音 50–64%、释音 64–100%），而不是按当前
 * 时长归一化——归一化的话拖一个点会让整条曲线跟着重排，手感是「点从鼠标底下逃走」。固定分段里每个点
 * 只在自己那一段内移动，拖起来可预测、也好抓。纵轴就是包络值（上 = 1，下 = 0）。
 */
const ENV = { attackMax: 1, decayMax: 2, releaseMax: 4, ax: 28, dx: 22, hx: 64, rx: 36 };

/** 控制点在百分比坐标里的位置（y 向下为正，与 CSS 一致）。 */
const envPoints = computed(() => {
  const yS = (1 - shown("sustain")) * 100;
  return {
    attack: { x: ENV.ax * (shown("attack") / ENV.attackMax), y: 0 },
    decay: { x: ENV.ax + ENV.dx * (shown("decay") / ENV.decayMax), y: yS },
    release: { x: ENV.hx + ENV.rx * (shown("release") / ENV.releaseMax), y: 100 },
    hold: { x: ENV.hx, y: yS },
  };
});

const envPolyline = computed(() => {
  const p = envPoints.value;
  const pts: [number, number][] = [
    [0, 100],
    [p.attack.x, p.attack.y],
    [p.decay.x, p.decay.y],
    [p.hold.x, p.hold.y],
    [p.release.x, p.release.y],
  ];
  return pts.map(([x, y]) => `${x.toFixed(2)},${y.toFixed(2)}`).join(" ");
});

/** 可拖动的三个控制点。 */
const ENV_HANDLES = ["attack", "decay", "release"] as const;

const envEl = ref<HTMLElement | null>(null);
/** 正在拖哪个控制点（null = 没在拖）。 */
const envDrag = ref<"attack" | "decay" | "release" | null>(null);
/** 拖动中的最新值，松手时一次性提交（整段拖动只占一个撤销步骤）。 */
const envCommit = ref<Record<string, number>>({});

function clamp01(v: number): number {
  return Math.min(1, Math.max(0, v));
}

/** 指针位置 → 百分比坐标。 */
function envAt(e: PointerEvent): { px: number; py: number } | null {
  const el = envEl.value;
  if (!el) return null;
  const r = el.getBoundingClientRect();
  if (r.width <= 0 || r.height <= 0) return null;
  return {
    px: ((e.clientX - r.left) / r.width) * 100,
    py: ((e.clientY - r.top) / r.height) * 100,
  };
}

function envMove(e: PointerEvent): void {
  const which = envDrag.value;
  const at = which ? envAt(e) : null;
  if (!which || !at) return;
  if (which === "attack") {
    const v = clamp01(at.px / ENV.ax) * ENV.attackMax;
    onLive("attack", v);
    envCommit.value = { attack: v };
  } else if (which === "decay") {
    // 这一个点同时管两个参数：横向 = 衰减时长，纵向 = 延音量。
    const d = clamp01((at.px - ENV.ax) / ENV.dx) * ENV.decayMax;
    const s = clamp01(1 - at.py / 100);
    onLive("decay", d);
    onLive("sustain", s);
    envCommit.value = { decay: d, sustain: s };
  } else {
    const v = clamp01((at.px - ENV.hx) / ENV.rx) * ENV.releaseMax;
    onLive("release", v);
    envCommit.value = { release: v };
  }
}

function envStart(which: "attack" | "decay" | "release", e: PointerEvent): void {
  envDrag.value = which;
  (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
  envMove(e);
}

function envEnd(e: PointerEvent): void {
  if (!envDrag.value) return;
  (e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
  envDrag.value = null;
  const changes = envCommit.value;
  envCommit.value = {};
  // 一次拖动 = 一个撤销步骤（哪怕这个点同时改了衰减与延音）。
  if (Object.keys(changes).length > 0) emit("commitMany", changes);
}

/** 控制点的提示文本：带上当前值，拖的时候能直接看到数字。 */
function envTitle(id: "attack" | "decay" | "release"): string {
  if (id === "decay") {
    return `${t("synth.decay")} ${display("decay", shown("decay"))} · ${t("synth.sustain")} ${display("sustain", shown("sustain"))}`;
  }
  return `${t(`synth.${id}`)} ${display(id, shown(id))}`;
}

// ---- 预设垫子 ----
const presets = usePresetsStore();
/** 当前翻看的类别（出厂预设的 slug）。自存预设没有类别，归到第一类「8-bit 键盘」里。 */
const coarse = ref("8bit");
const options = computed(() => presets.forPlugin(SYNTH_ID));

/** 有哪几个类别（按出厂预设的顺序去重）。 */
const coarseOptions = computed(() => {
  const out: string[] = [];
  for (const option of options.value) {
    const slug = option.category ?? "8bit";
    if (!out.includes(slug)) out.push(slug);
  }
  return out;
});

/** 当前类别下的垫子。 */
const pads = computed(() => options.value.filter((o) => (o.category ?? "8bit") === coarse.value));

/** 当前参数命中的那条预设（用来高亮垫子；只看参数，与效果链无关）。 */
const activePresetId = computed(() => presets.match(SYNTH_ID, props.params)?.id ?? "");

/** 切预设：把整组乐器参数交给父组件写回当前轨道（只有它知道是哪条轨道）。 */
function applyPreset(values: Record<string, number>): void {
  emit("applyPreset", values);
}

function pickPad(option: PresetOption): void {
  applyPreset(option.params);
}
</script>

<template>
  <!-- 面板本身就是窗口内容：没有外层大圆角容器，键盘直接贴到窗口边缘。 -->
  <div class="panel">
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

      <!-- 中部：左边屏幕（示波器 + 可拖动的 ADSR，两块等大），右边基础旋钮 + 「高级」 -->
      <section class="middle">
        <div class="screen">
          <Oscilloscope :params="scopeParams" />

          <div
            class="env"
            :class="{ dragging: envDrag !== null }"
            role="group"
            :aria-label="t('synth.envelope')"
            @pointermove="envMove"
            @pointerup="envEnd"
            @pointercancel="envEnd"
          >
            <!-- 绘图区**内缩一圈**：控制点在 0% / 100% 时也离边与圆角有距离，不会被裁掉。 -->
            <div ref="envEl" class="env-plot">
              <svg class="env-svg" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
                <polyline :points="envPolyline" />
              </svg>
              <button
                v-for="id in ENV_HANDLES"
                :key="id"
                class="env-point"
                :class="{ on: envDrag === id }"
                :style="{ left: `${envPoints[id].x}%`, top: `${envPoints[id].y}%` }"
                :aria-label="envTitle(id)"
                :title="envTitle(id)"
                @pointerdown="envStart(id, $event)"
                @pointermove="envMove"
                @pointerup="envEnd"
                @pointercancel="envEnd"
              />
            </div>
            <span class="env-label">{{ t("synth.envelope") }}</span>
          </div>
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
                <!-- 滤波类型：三选一的枚举，横跨整行（旋钮选不出来的东西） -->
                <div class="waves filter-type" role="group" :aria-label="t('synth.filterType')">
                  <button
                    v-for="f in FILTER_TYPES"
                    :key="f.value"
                    class="seg"
                    :class="{ on: filterType === f.value }"
                    :aria-pressed="filterType === f.value"
                    @click="setFilterType(f.value)"
                  >
                    <span class="seg-text">{{ t(f.key) }}</span>
                  </button>
                </div>

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

          <div class="foot-row">
            <div class="foot-left">
              <button
                class="btn-tonal reset-params"
                :disabled="atDefaults"
                @click="restoreDefaults"
              >
                {{ t("panel.restoreDefaults") }}
              </button>
              <!-- 八度按钮与「恢复默认参数」同一套 `btn-tonal` 样式、同一种尺寸，挨在一起。 -->
              <button
                class="btn-tonal octave-btn"
                :disabled="!canShiftDown"
                :title="t('synth.octaveDown')"
                :aria-label="t('synth.octaveDown')"
                @click="shiftOctave(-1)"
              >
                {{ t("synth.octaveDown") }}
              </button>
              <button
                class="btn-tonal octave-btn"
                :disabled="!canShiftUp"
                :title="t('synth.octaveUp')"
                :aria-label="t('synth.octaveUp')"
                @click="shiftOctave(1)"
              >
                {{ t("synth.octaveUp") }}
              </button>
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
        </div>
      </section>

      <!-- 底部：键盘**贴死窗口边缘**（面板不留下内边距，左右负边距拉满整宽，
           圆角由窗口本身切出来）。上面一条细杆显示当前音域（八度按钮在动作栏里）。 -->
      <div v-if="props.track !== undefined" class="keyboard-row">
        <div class="kb-bar">
          <span class="kb-range">{{ keyboardRange }}</span>
        </div>
        <div class="kb-keys">
          <PianoKeyboard :track="props.track" :base-note="keyboardBase" :octaves="KB_OCTAVES" />
        </div>
      </div>
  </div>
</template>

<style scoped>
/* 面板铺满宿主：内部纵向分区都用固定高度 / `flex: 0 0 auto`，键盘不会被压扁变形。
   左右留 10px 呼吸，**下边距为 0**——键盘要贴死窗口底边。 */
.panel {
  flex: 1 1 auto;
  min-height: 420px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 10px 10px 0;
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
/* 示波器与包络图**等大**：都按剩余高度等分。 */
.screen > * {
  flex: 1 1 0;
  min-height: 0;
}
/* 包络图：一块可拖动的「屏幕」，控制点用绝对定位的按钮画（与 polyline 共用百分比坐标）。 */
.env {
  position: relative;
  border-radius: 16px;
  background-color: rgb(var(--md-scrim) / 0.35);
  border: 1px solid rgb(var(--md-outline-variant) / 0.45);
  overflow: hidden;
  touch-action: none;
}
.env.dragging {
  cursor: grabbing;
}
/* 绘图区内缩一圈：控制点落在 0% / 100% 时也离边框与圆角有余量，不会被裁掉。 */
.env-plot {
  position: absolute;
  inset: 16px 14px;
}
.env-svg {
  width: 100%;
  height: 100%;
}
.env-svg polyline {
  fill: none;
  stroke: rgb(var(--md-primary));
  stroke-width: 2;
  /* `preserveAspectRatio="none"` 会把描边一起拉伸，这条让描边宽度保持常量 */
  vector-effect: non-scaling-stroke;
  stroke-linejoin: round;
  stroke-linecap: round;
}
/* 控制点的命中区比视觉圆点大一圈（视觉 10px、命中 18px），小屏幕上也好抓。 */
.env-point {
  position: absolute;
  width: 18px;
  height: 18px;
  margin: -9px 0 0 -9px;
  padding: 0;
  border: 0;
  background: transparent;
  cursor: grab;
  touch-action: none;
}
.env-point::before {
  content: "";
  position: absolute;
  inset: 4px;
  box-sizing: border-box;
  border-radius: 999px;
  border: 2px solid rgb(var(--md-surface));
  background-color: rgb(var(--md-primary));
  transition: transform 0.12s ease;
}
.env-point:hover::before,
.env-point.on::before {
  transform: scale(1.3);
}
.env-label {
  position: absolute;
  left: 12px;
  top: 8px;
  font-size: 9.5px;
  letter-spacing: 0.4px;
  color: rgb(var(--md-on-surface-muted));
  pointer-events: none;
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
/* 基础旋钮：**两排 4×2**，格子均分（展开高级区时旋钮变小，格子不变）。 */
.basic {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  justify-items: center;
  gap: 10px 6px;
}

/* 高级区：老式减法合成器的分区卡片 */
.advanced {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 10px;
}
/* 滤波类型横跨整行（枚举做成分段按钮，不跟旋钮挤在两列里） */
.filter-type {
  grid-column: 1 / -1;
  height: 30px;
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

/* 底部动作栏：「恢复默认参数」+「高级」 */
.foot-row {
  flex: 0 0 auto;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
/* 左侧一组：恢复默认 + 两个八度按钮（窄侧边栏里可以换行，不挤变形） */
.foot-left {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
/* 「恢复默认参数」（`btn-tonal` 来自全局样式，这里只管尺寸与置灰态） */
.reset-params {
  height: 36px;
  padding: 0 18px;
  font-size: 12.5px;
}
.reset-params:disabled,
.octave-btn:disabled {
  opacity: 0.45;
  cursor: default;
}
/* 八度按钮：与「恢复默认参数」同一套 `btn-tonal`，同尺寸、文字写明「降低/升高八度」 */
.octave-btn {
  height: 36px;
  padding: 0 14px;
  font-size: 12.5px;
  white-space: nowrap;
}

/* 「高级」开关（`btn-tonal` 来自全局样式，这里只管尺寸与展开态） */
.advanced-toggle {
  flex: 0 0 auto;
  height: 36px;
  padding: 0 20px;
  font-size: 12.5px;
}
.advanced-toggle.on {
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}

/* ---- 底部：键盘（贴死窗口底边） ---- */
.keyboard-row {
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  /* 抵消面板左右内边距，让键盘横跨整宽；下边不留缝，直接贴底。 */
  margin: 0 -10px;
}
/* 键盘上方那条细杆：显示当前音域（八度按钮已移到动作栏里）。 */
.kb-bar {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 6px;
  height: 22px;
  padding: 0 10px;
}
.kb-range {
  flex: 1 1 auto;
  min-width: 0;
  font-size: 10.5px;
  letter-spacing: 0.4px;
  color: rgb(var(--md-on-surface-variant));
  font-variant-numeric: tabular-nums;
}
.kb-keys {
  flex: 0 0 auto;
  height: 96px;
}

.pad:focus-visible,
.chip:focus-visible,
.seg:focus-visible,
.env-point:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
/* 分段按钮在 overflow: hidden 的容器里，焦点环画在里面才不会被裁掉 */
.seg:focus-visible {
  outline-offset: -2px;
}
</style>
