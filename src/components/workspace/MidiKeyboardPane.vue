<script setup lang="ts">
/**
 * 下窗口的虚拟 MIDI 键盘视图。
 *
 * 编辑与编排都用表头最左侧的三格视图切换器（编辑：控制器 / 乐谱 / 键盘；编排：自动化 /
 * 乐谱 / 键盘）。两种宿主都展示选中轨道的键盘：三种输入（鼠标、硬件 MIDI、电脑键盘模拟）
 * 共用 MIDI store 的一条路径，所以高亮、试听与 MIDI 录音看到的是同一份状态；录音模式由
 * 底栏录音键旁的小开关决定。
 *
 * 表头 30px + 1px 顶边框，和控制器泳道 / 乐谱窗格逐像素一致，切换时窗格高度不跳。
 * 窗格拉高后不会马上分裂：只有单排键的「高 / 宽」≥ 5 时才分成双排键
 * （上排高音、下排低音），避免窗口一拉高就早早裂开。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { MinusIcon, PlusIcon, XMarkIcon } from "@heroicons/vue/24/outline";
import Md3Select from "@/components/common/Md3Select.vue";
import PianoKeyboard from "@/components/common/PianoKeyboard.vue";
import KeyboardIcon from "@/components/common/icons/KeyboardIcon.vue";
import LaneViewSwitch from "@/components/workspace/LaneViewSwitch.vue";
import { useI18n } from "@/i18n";
import {
  ARRANGE_LANE_VIEWS,
  EDIT_LANE_VIEWS,
  useLayoutStore,
  type LaneView,
} from "@/stores/layout";
import { useMidiStore } from "@/stores/midi";
import { useSelectionStore } from "@/stores/selection";
import { useTransportStore } from "@/stores/transport";
import { shouldDeferToBrowser } from "@/utils/editable";

const props = withDefaults(
  defineProps<{
    /** 宿主：编辑视图 / 编排视图。只影响视图切换器提供哪几格。 */
    context?: "edit" | "arrange";
  }>(),
  { context: "edit" },
);

const emit = defineEmits<{
  /** 视图切换器：控制器 / 自动化 / 乐谱 / MIDI 键盘（由宿主决定可用项）。 */
  (e: "update:view", value: LaneView): void;
}>();

const i18n = useI18n();
const layout = useLayoutStore();
const midi = useMidiStore();
const selection = useSelectionStore();
const transport = useTransportStore();

const isArrange = computed(() => props.context === "arrange");
/** 这个宿主能切哪几格。 */
const views = computed(() => (isArrange.value ? ARRANGE_LANE_VIEWS : EDIT_LANE_VIEWS));

/** 收起下窗口：编排视图关 `showAutomation`，编辑视图关 `showControllerLane`。 */
function hidePane(): void {
  if (isArrange.value) layout.showAutomation = false;
  else layout.showControllerLane = false;
}

/** 表头固定开销（1px 顶边框 + 30px 表头）——与控制器泳道 / 乐谱窗格一致。 */
const LANE_CHROME = 31;

/** 单排键覆盖三个八度，和 `midi.baseNote` 的默认音域一致。 */
const OCTAVES = 3;

/** 双排每排两个八度。 */
const ROW_OCTAVES = 2;

/** 单排 3 个八度的白键数（C..C）= 22。 */
const SINGLE_WHITE_KEYS = OCTAVES * 7 + 1;
/** `.keys` 的左右 / 上下内边距（px），要和样式保持一致。 */
const KEYS_PAD_X = 16;
const KEYS_PAD_Y = 11;
/** 白键之间的 1px 缝。 */
const WHITE_GAP = 1;

/**
 * 双排的触发条件是**单排键的高 / 宽比 ≥ 5**。
 *
 * 键宽是实测的（窗口 / 侧栏 / 面板宽度都会重算），所以宽窗口下要把窗格拉得非常高才会
 * 分裂；窄窗口下键本身窄，比例更容易达到，但也仍然是「5 倍高」的细长键才切。
 */
const TWO_ROW_RATIO = 5;

/** 力度快捷档：点一下 / C、V 切换（比在 30px 表头里塞滑块更稳）。 */
const VELOCITIES = [40, 60, 80, 100, 127];

/** `.keys` 的实测宽度（含左右内边距）：用来算单排键宽。 */
const keysEl = ref<HTMLElement | null>(null);
const keysWidth = ref(0);
let keysObserver: ResizeObserver | null = null;

/** 单排时一个白键的宽度（px）。 */
const singleKeyWidth = computed(() => {
  if (keysWidth.value <= 0) return 0;
  const inner = Math.max(
    1,
    keysWidth.value - KEYS_PAD_X - (SINGLE_WHITE_KEYS - 1) * WHITE_GAP,
  );
  return inner / SINGLE_WHITE_KEYS;
});

/** 单排时一个键的高度（px）：窗格高度减上下内边距。 */
const singleKeyHeight = computed(() => Math.max(1, layout.laneHeight - KEYS_PAD_Y));

/**
 * 只有单排键的高 / 宽比 ≥ 5 时才切双排。
 *
 * 宽度用**单排 3 个八度**来算，而不是切到双排后的 2 个八度——否则切换本身会改变键宽，
 * 条件可能在边界上来回抖。
 */
const twoRow = computed(
  () =>
    keysWidth.value > 0 &&
    singleKeyHeight.value >= singleKeyWidth.value * TWO_ROW_RATIO,
);

/** 上排比下排高几个八度：尽量两个八度，靠近 MIDI 上限时退到一个八度，避免出现 >127 的键。 */
const topOffset = computed(() => {
  if (!twoRow.value) return 0;
  const maxOffset = 127 - ROW_OCTAVES * 12 - midi.baseNote;
  return Math.max(12, Math.min(24, Math.floor(maxOffset / 12) * 12));
});

/** 上排的起始音高。 */
const topBase = computed(() => midi.baseNote + topOffset.value);

/** 每排用几个八度：单排 3 个，双排每排 2 个。 */
const rowOctaves = computed(() => (twoRow.value ? ROW_OCTAVES : OCTAVES));

const isRecording = computed(
  () => transport.recording && transport.recordingMode === "midi",
);

/** 设备下拉：第一项固定「未连接」。 */
const deviceOptions = computed(() => [
  { id: "", label: i18n.t("midi.inputNone"), hint: i18n.t("midi.inputNoneHint") },
  ...midi.inputs.map((d) => ({ id: d.name, label: d.name })),
]);

const device = computed({
  get: () => midi.input ?? "",
  set: (id: string) => void midi.select(id === "" ? null : id),
});

/** 点力度药丸：切到下一档。 */
function cycleVelocity(): void {
  const index = VELOCITIES.indexOf(midi.velocity);
  const next = index < 0 ? VELOCITIES[2] : VELOCITIES[(index + 1) % VELOCITIES.length];
  midi.setVelocity(next);
}

/** C / V：按档位升高 / 降低力度（和主流 DAW 的电脑键盘习惯一致）。 */
function stepVelocity(delta: number): void {
  const current = midi.velocity;
  const next =
    delta > 0
      ? (VELOCITIES.find((v) => v > current) ?? VELOCITIES[VELOCITIES.length - 1]!)
      : ([...VELOCITIES].reverse().find((v) => v < current) ?? VELOCITIES[0]!);
  if (next !== current) midi.setVelocity(next);
}

/**
 * 电脑键盘的 DAW 控制键：`Z` / `X` 降 / 升八度，`C` / `V` 降 / 升力度。
 *
 * 只在「电脑键盘模拟」开着时生效，并且焦点在输入框 / 文本域里时完全让开。字母键本身
 * （A W S E D F…）仍由 `PianoKeyboard` 处理，这里只接主流 DAW 的四个控制键。
 */
function onControlKey(e: KeyboardEvent): void {
  if (!midi.computer) return;
  if (e.repeat || e.metaKey || e.ctrlKey || e.altKey) return;
  if (shouldDeferToBrowser(e)) return;
  const key = e.key.toLowerCase();
  if (key === "z" || key === "x") {
    e.preventDefault();
    midi.shiftOctave(key === "z" ? -1 : 1);
    return;
  }
  if (key === "c" || key === "v") {
    e.preventDefault();
    stepVelocity(key === "c" ? -1 : 1);
  }
}

onMounted(() => {
  window.addEventListener("keydown", onControlKey);
  const el = keysEl.value;
  if (!el) return;
  keysWidth.value = el.clientWidth;
  if (typeof ResizeObserver !== "undefined") {
    keysObserver = new ResizeObserver(() => {
      keysWidth.value = keysEl.value?.clientWidth ?? 0;
    });
    keysObserver.observe(el);
  }
});
onUnmounted(() => {
  window.removeEventListener("keydown", onControlKey);
  keysObserver?.disconnect();
  keysObserver = null;
});

/** 鼠标按下键盘时走 MIDI store，而不是让键盘组件自己去试听（录音也要拿到这颗音）。 */
function onNoteOn(note: number, velocity: number, source: "mouse" | "key"): void {
  midi.noteOn(note, velocity, source === "key" ? "computer" : "mouse");
}

function onNoteOff(note: number, source: "mouse" | "key"): void {
  midi.noteOff(note, source === "key" ? "computer" : "mouse");
}
</script>

<template>
  <div class="keyboard-block" :style="{ height: `${layout.laneHeight + LANE_CHROME}px` }">
    <div class="lane-head">
      <LaneViewSwitch
        :model-value="'keyboard'"
        :views="views"
        @update:model-value="emit('update:view', $event)"
      />

      <!-- MIDI 输入设备：选了才占用端口；没有设备时只显示「未连接」 -->
      <div class="device-chooser" @pointerdown="midi.refresh()">
        <Md3Select
          v-model="device"
          :options="deviceOptions"
          :label="i18n.t('midi.inputLabel')"
          :placeholder="i18n.t('midi.inputNone')"
        />
      </div>

      <!-- 电脑键盘模拟 MIDI：关掉后字母键不再发声（也不会进录音） -->
      <button
        type="button"
        class="lane-toggle"
        :class="{ on: midi.computer }"
        :aria-pressed="midi.computer"
        :title="midi.computer ? i18n.t('midi.computerOff') : i18n.t('midi.computerOn')"
        :aria-label="i18n.t('midi.computerLabel')"
        @click="midi.setComputer(!midi.computer)"
      >
        <KeyboardIcon class="lane-btn-icon" />
      </button>

      <div class="octave" :title="twoRow ? i18n.t('midi.twoRowHint') : midi.rangeLabel">
        <button
          type="button"
          class="lane-btn"
          :disabled="midi.octaveShift <= -2"
          :aria-label="i18n.t('midi.octaveDown')"
          :title="i18n.t('midi.octaveDown')"
          @click="midi.shiftOctave(-1)"
        >
          <MinusIcon class="lane-btn-icon" />
        </button>
        <span class="octave-label tabular">{{ midi.baseLabel }}</span>
        <button
          type="button"
          class="lane-btn"
          :disabled="midi.octaveShift >= 3"
          :aria-label="i18n.t('midi.octaveUp')"
          :title="i18n.t('midi.octaveUp')"
          @click="midi.shiftOctave(1)"
        >
          <PlusIcon class="lane-btn-icon" />
        </button>
      </div>

      <button
        type="button"
        class="velocity"
        :title="i18n.t('midi.velocityHint')"
        :aria-label="`${i18n.t('midi.velocity')} ${midi.velocity}`"
        @click="cycleVelocity"
      >
        <span class="velocity-key">V</span><strong class="tabular">{{ midi.velocity }}</strong>
      </button>

      <!-- 电脑键盘开着时给一条极短的快捷键提示；详细映射在键盘按钮的 tooltip 里 -->
      <span
        v-if="midi.computer"
        class="shortcut-hint"
        :title="`${i18n.t('midi.computerHint')} · ${i18n.t('midi.octave')} Z/X · ${i18n.t('midi.velocity')} C/V`"
        aria-hidden="true"
      >
        Z/X · C/V
      </span>

      <span v-if="isRecording" class="rec-pill">
        <span class="rec-dot" />
        {{ i18n.t("midi.recording") }}
      </span>

      <!-- 编排视图：明确告诉用户这排键盘弹的是哪条轨道 -->
      <span v-if="isArrange && selection.track" class="track-label">{{ selection.track.name }}</span>

      <span class="lane-spacer" />

      <button
        class="lane-btn"
        type="button"
        :aria-label="i18n.t('lane.hide')"
        @click="hidePane"
      >
        <XMarkIcon class="lane-btn-icon" />
      </button>
    </div>

    <div ref="keysEl" class="keys" :class="{ double: twoRow }">
      <div class="keys-rows">
        <!-- 上排：高音区；只负责鼠标 / 硬件，不重复接管电脑键盘 -->
        <div v-if="twoRow" class="key-row">
          <PianoKeyboard
            :base-note="topBase"
            :octaves="rowOctaves"
            :velocity="midi.velocity"
            :held-notes="midi.heldNotes"
            :computer-keyboard="false"
            show-labels
            style="min-height: 0"
            @note-on="onNoteOn"
            @note-off="onNoteOff"
          />
        </div>
        <!-- 下排：低音区，电脑键盘映射固定落在这一排 -->
        <div class="key-row">
          <PianoKeyboard
            :base-note="midi.baseNote"
            :octaves="rowOctaves"
            :velocity="midi.velocity"
            :held-notes="midi.heldNotes"
            :computer-keyboard="midi.computer"
            show-labels
            style="min-height: 0"
            @note-on="onNoteOn"
            @note-off="onNoteOff"
          />
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.keyboard-block {
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  min-height: 0;
  border-top: 1px solid var(--grid-line-strong);
  background-color: var(--app-card);
  user-select: none;
  -webkit-user-select: none;
}
/* 表头：与控制器泳道 / 乐谱窗格同高同序，切过来时眼睛不用重新找 */
.lane-head {
  height: 30px;
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 10px 0 12px;
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.35);
}
.lane-title {
  flex: 0 0 auto;
  font-size: 11px;
  line-height: 1;
  letter-spacing: 0.2px;
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
}
.lane-chooser {
  flex: 0 0 auto;
  display: inline-flex;
  min-width: 0;
}
/* 这里比另外两个窗格多出设备 / 八度 / 力度，目标下拉收窄到 108px 给它们让位。 */
.lane-chooser :deep(.md3-select) {
  width: 108px;
}
/* 设备下拉：和「目标」下拉同高（22px），宽度给到能看清常见端口名；窄窗格可缩到 80px。 */
.device-chooser {
  flex: 0 1 160px;
  display: inline-flex;
  min-width: 80px;
}
.device-chooser :deep(.md3-select) {
  width: 100%;
  height: 22px;
  padding: 0 8px 0 10px;
  gap: 6px;
  font-size: 11px;
}
.device-chooser :deep(.caret) {
  width: 13px;
  height: 13px;
}
.lane-toggle,
.lane-btn {
  flex: 0 0 auto;
  width: 24px;
  height: 24px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 1px solid transparent;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease;
}
.lane-toggle:hover,
.lane-btn:not(:disabled):hover {
  background-color: rgb(var(--md-on-surface-variant) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.lane-toggle.on {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
  border-color: rgb(var(--md-outline-variant) / 0.4);
}
.lane-btn:disabled {
  opacity: 0.35;
  cursor: default;
}
.lane-btn-icon {
  width: 15px;
  height: 15px;
}
.octave {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 2px;
  height: 24px;
  padding: 0 2px;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-high));
}
.octave-label {
  min-width: 26px;
  text-align: center;
  font-size: 10.5px;
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
}
.velocity {
  flex: 0 0 auto;
  height: 22px;
  padding: 0 8px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font: inherit;
  font-size: 10.5px;
  cursor: pointer;
  white-space: nowrap;
}
.velocity-key {
  opacity: 0.65;
  margin-right: 2px;
}
.velocity:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.velocity strong {
  color: rgb(var(--md-on-surface));
  margin-left: 3px;
}
.rec-pill {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 22px;
  padding: 0 9px;
  border-radius: 999px;
  background-color: rgb(var(--md-error) / 0.14);
  color: rgb(var(--md-error));
  font-size: 10.5px;
  white-space: nowrap;
}
.rec-dot {
  width: 6px;
  height: 6px;
  border-radius: 999px;
  background-color: rgb(var(--md-error));
  box-shadow: 0 0 0 3px rgb(var(--md-error) / 0.18);
}
/* 编排视图的轨道名：只在这条键盘真的绑定了选中轨道时出现，窄窗口自动省略 */
.track-label {
  flex: 0 1 auto;
  min-width: 0;
  max-width: 140px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 10.5px;
  color: rgb(var(--md-on-surface-muted));
}
.lane-spacer {
  flex: 1 1 auto;
}
.keys {
  position: relative;
  flex: 1 1 auto;
  min-height: 0;
  padding: 5px 8px 6px;
  background-color: var(--app-inset);
}
/* 单排 / 双排共用一个纵向容器：双排时每排各占一半高度。 */
.keys-rows {
  height: 100%;
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-height: 0;
}
.key-row {
  flex: 1 1 0;
  min-height: 0;
  display: flex;
}
/* 键盘组件自己 height:100%；这里再给一层 flex，双排时无论宿主高度怎么变都不会塌 */
.key-row :deep(.keyboard) {
  flex: 1 1 auto;
  min-width: 0;
  min-height: 0;
}
.keys.double {
  padding-top: 4px;
  padding-bottom: 4px;
}
/* 虚拟键盘自带 min-height:84px（合成器面板的尺寸）；下窗口可以更矮，交给窗格拉伸。 */
.keys :deep(.keyboard) {
  min-height: 0;
}
/* 键盘开着时的快捷键提示：只占表头一小段，窄窗口直接收掉（tooltip 里还有完整说明） */
.shortcut-hint {
  flex: 0 0 auto;
  font-size: 9.5px;
  letter-spacing: 0.3px;
  color: rgb(var(--md-on-surface-muted));
  white-space: nowrap;
  opacity: 0.8;
}
.tabular {
  font-variant-numeric: tabular-nums;
}
/* 窄窗口（两块侧栏都开着时工作区只剩 400px 左右）：先收力度、再收八度读数，
   设备 / 电脑键盘 / 收起这些关键操作始终保留。 */
@media (max-width: 1180px) {
  .velocity {
    display: none;
  }
}
@media (max-width: 1080px) {
  .octave-label {
    display: none;
  }
  /* 底栏已经有录音指示；窗格再窄时把这条让给设备 / 八度 / 收起。 */
  .rec-pill {
    display: none;
  }
  .lane-title {
    display: none;
  }
  .track-label {
    display: none;
  }
}
</style>
