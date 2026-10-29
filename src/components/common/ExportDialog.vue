<script setup lang="ts">
/**
 * 导出音频对话框：格式（WAV / MP3）+ 采样率（+ MP3 的比特率）。
 *
 * 为什么要有这一步而不是直接开保存对话框：MP3 需要挑比特率，而两种格式能用的采样率不一样
 * （MP3 只有 MPEG-1 的三档，96 kHz 会被后端直接拒掉）。选完这里再去系统保存对话框选路径，
 * **扩展名决定最终格式**（见 `api/dialogs.ts` 的 `pickAudioToSave`）——所以文件名写成 `.mp3`
 * 永远会得到 MP3，与这里选了什么不冲突。
 *
 * 外壳与 `ConfirmDialog` 同一套（遮罩 + 圆角框 + 取消/确认），只是中间换成两三个字段。
 */
import { computed, ref, watch } from "vue";
import { XMarkIcon } from "@heroicons/vue/24/outline";
import Md3Select from "@/components/common/Md3Select.vue";
import { useI18n } from "@/i18n";
import type { ExportFormat } from "@/api/tauri";

/** 用户在对话框里选好的导出参数。 */
export type ExportChoice =
  | {
      kind: "audio";
      format: ExportFormat;
      sampleRate: number;
      /** 只对 MP3 有意义。 */
      bitrateKbps: number;
    }
  | {
      kind: "score";
      /** 记谱量化网格（拍）；0 = 不对齐。 */
      grid: number;
      /** 调号（升号为正）。 */
      keyFifths: number;
    };

defineProps<{ open: boolean }>();

const emit = defineEmits<{
  (e: "update:open", value: boolean): void;
  (e: "confirm", choice: ExportChoice): void;
}>();

const i18n = useI18n();

/** 对话框里的「格式」：WAV / MP3 / 乐谱。 */
type Format = ExportFormat | "score";

/** WAV 可选的采样率（后端接受 8000..=192000，界面只给常用三档）。 */
const WAV_RATES = [44_100, 48_000, 96_000];
/** MP3 只支持 MPEG-1 的 32 / 44.1 / 48 kHz；界面给常用的两档。 */
const MP3_RATES = [44_100, 48_000];
/** MP3 比特率档位（由高到低，取后端档位表里常用的一段）。 */
const MP3_BITRATES = [320, 256, 192, 128, 96];

/**
 * 记谱网格：量化到多细。
 *
 * 默认 1/16：与钢琴卷帘的默认吸附一致，绝大多数流行/电子乐都能原样记出来；1/12（三连音）
 * 不单独给选项——三连音由量化器自动识别，用户不用关心。
 */
const GRIDS: { id: string; label: string; beats: number }[] = [
  { id: "1/4", label: "1/4", beats: 1 },
  { id: "1/8", label: "1/8", beats: 0.5 },
  { id: "1/16", label: "1/16", beats: 0.25 },
  { id: "1/32", label: "1/32", beats: 0.125 },
  { id: "off", label: "off", beats: 0 },
];

/**
 * 调号。工程模型里没有调号字段，导出时先手选一个：选对调号，升降号会少一大半。
 *
 * 标签写成「大调 / 关系小调」——写谱的人认这个，比记「几个升号」直观。
 */
const KEYS: { id: string; label: string }[] = [
  { id: "0", label: "C / Am" },
  { id: "1", label: "G / Em（1♯）" },
  { id: "2", label: "D / Bm（2♯）" },
  { id: "3", label: "A / F#m（3♯）" },
  { id: "4", label: "E / C#m（4♯）" },
  { id: "5", label: "B / G#m（5♯）" },
  { id: "-1", label: "F / Dm（1♭）" },
  { id: "-2", label: "B♭ / Gm（2♭）" },
  { id: "-3", label: "E♭ / Cm（3♭）" },
  { id: "-4", label: "A♭ / Fm（4♭）" },
  { id: "-5", label: "D♭ / B♭m（5♭）" },
];

const format = ref<Format>("wav");
const sampleRate = ref(WAV_RATES[0]!);
const bitrateKbps = ref(MP3_BITRATES[0]!);
const gridId = ref("1/16");
const keyId = ref("0");

const rates = computed(() => (format.value === "mp3" ? MP3_RATES : WAV_RATES));

/** 切到 MP3 时把 WAV 专有的采样率（96 kHz）拉回 MP3 能用的值。 */
watch(format, () => {
  if (!rates.value.includes(sampleRate.value)) sampleRate.value = MP3_RATES[0]!;
});

const rateOptions = computed(() =>
  rates.value.map((rate) => ({ id: String(rate), label: `${(rate / 1000).toFixed(1)} kHz` })),
);
const bitrateOptions = computed(() =>
  MP3_BITRATES.map((kbps) => ({ id: String(kbps), label: `${kbps} kbps` })),
);

/** 下拉要的是字符串，值本身是数字：这里做一次双向映射。 */
const rateId = computed({
  get: () => String(sampleRate.value),
  set: (id: string) => (sampleRate.value = Number(id)),
});
const bitrateId = computed({
  get: () => String(bitrateKbps.value),
  set: (id: string) => (bitrateKbps.value = Number(id)),
});

function close(): void {
  emit("update:open", false);
}

function confirm(): void {
  if (format.value === "score") {
    const grid = GRIDS.find((g) => g.id === gridId.value)?.beats ?? 0.25;
    emit("confirm", { kind: "score", grid, keyFifths: Number(keyId.value) });
  } else {
    emit("confirm", {
      kind: "audio",
      format: format.value,
      sampleRate: sampleRate.value,
      bitrateKbps: bitrateKbps.value,
    });
  }
  emit("update:open", false);
}

/** Escape 关闭；`stopPropagation` 免得这一个按键再触发外层全局快捷键。 */
function onKey(e: KeyboardEvent): void {
  if (e.key === "Escape") {
    e.stopPropagation();
    close();
  }
}
</script>

<template>
  <Teleport to="body">
    <transition name="fade">
      <div v-if="open" class="scrim" @pointerdown.self="close">
        <div
          class="dialog"
          role="dialog"
          aria-modal="true"
          :aria-label="i18n.t('export.title')"
          tabindex="-1"
          @keydown="onKey"
        >
          <header class="head">
            <h2 class="title">{{ i18n.t("export.title") }}</h2>
            <button class="icon-btn" :aria-label="i18n.t('common.close')" @click="close">
              <XMarkIcon class="w-[18px] h-[18px]" />
            </button>
          </header>

          <div class="body">
            <div class="field">
              <span class="field-label">{{ i18n.t("export.format") }}</span>
              <div class="seg" role="radiogroup" :aria-label="i18n.t('export.format')">
                <button
                  class="seg-btn"
                  :class="{ on: format === 'wav' }"
                  role="radio"
                  :aria-checked="format === 'wav'"
                  @click="format = 'wav'"
                >
                  <span class="seg-title">{{ i18n.t("export.wav") }}</span>
                  <span class="seg-hint">{{ i18n.t("export.wavHint") }}</span>
                </button>
                <button
                  class="seg-btn"
                  :class="{ on: format === 'mp3' }"
                  role="radio"
                  :aria-checked="format === 'mp3'"
                  @click="format = 'mp3'"
                >
                  <span class="seg-title">{{ i18n.t("export.mp3") }}</span>
                  <span class="seg-hint">{{ i18n.t("export.mp3Hint") }}</span>
                </button>
                <button
                  class="seg-btn"
                  :class="{ on: format === 'score' }"
                  role="radio"
                  :aria-checked="format === 'score'"
                  @click="format = 'score'"
                >
                  <span class="seg-title">{{ i18n.t("export.score") }}</span>
                  <span class="seg-hint">{{ i18n.t("export.scoreHint") }}</span>
                </button>
              </div>
            </div>

            <!-- 乐谱：量化网格 + 调号。这两个选项直接决定谱面好不好看。 -->
            <template v-if="format === 'score'">
              <div class="field">
                <span class="field-label">{{ i18n.t("export.quantize") }}</span>
                <Md3Select v-model="gridId" :options="GRIDS" :label="i18n.t('export.quantize')" />
              </div>
              <div class="field">
                <span class="field-label">{{ i18n.t("export.keySignature") }}</span>
                <Md3Select v-model="keyId" :options="KEYS" :label="i18n.t('export.keySignature')" />
              </div>
            </template>

            <div v-else class="field">
              <span class="field-label">{{ i18n.t("export.sampleRate") }}</span>
              <Md3Select
                v-model="rateId"
                :options="rateOptions"
                :label="i18n.t('export.sampleRate')"
              />
            </div>

            <div v-if="format === 'mp3'" class="field">
              <span class="field-label">{{ i18n.t("export.bitrate") }}</span>
              <Md3Select
                v-model="bitrateId"
                :options="bitrateOptions"
                :label="i18n.t('export.bitrate')"
              />
            </div>

            <p class="note">
              {{
                format === "mp3"
                  ? i18n.t("export.mp3Note")
                  : format === "score"
                    ? i18n.t("export.scoreNote")
                    : i18n.t("export.wavNote")
              }}
            </p>
          </div>

          <footer class="foot">
            <button class="btn-text" @click="close">{{ i18n.t("common.cancel") }}</button>
            <button class="btn-filled" @click="confirm">{{ i18n.t("export.confirm") }}</button>
          </footer>
        </div>
      </div>
    </transition>
  </Teleport>
</template>

<style scoped>
.scrim {
  position: fixed;
  inset: 0;
  z-index: 130;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  background-color: rgb(var(--md-scrim) / 0.42);
}
.dialog {
  width: 420px;
  max-width: 100%;
  display: flex;
  flex-direction: column;
  border-radius: 24px;
  background-color: rgb(var(--md-surface-container-high));
  box-shadow: var(--md-elevation-3);
  overflow: hidden;
}
.head {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 16px 16px 8px 22px;
}
.title {
  flex: 1 1 auto;
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  color: rgb(var(--md-on-surface));
}
.icon-btn {
  width: 34px;
  height: 34px;
  flex: 0 0 auto;
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
.icon-btn:hover {
  background-color: rgb(var(--md-on-surface-variant) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.icon-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.body {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 6px 22px 4px;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.field-label {
  font-size: 11px;
  letter-spacing: 0.4px;
  color: rgb(var(--md-on-surface-variant));
}
/* 格式：三枚并列的药丸按钮，各自带一行小字说明（有损 / 无损 / 乐谱一眼可见）。 */
.seg {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 8px;
}
.seg-btn {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  padding: 8px 12px;
  border: 1px solid rgb(var(--md-outline) / 0.7);
  border-radius: 14px;
  background-color: transparent;
  color: rgb(var(--md-on-surface));
  text-align: left;
  cursor: pointer;
  transition: border-color 0.15s ease, background-color 0.15s ease;
}
.seg-btn:hover {
  border-color: rgb(var(--md-primary) / 0.8);
}
.seg-btn.on {
  border-color: rgb(var(--md-primary));
  background-color: rgb(var(--md-primary) / 0.1);
}
.seg-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.seg-title {
  font-size: 13px;
  font-weight: 500;
}
.seg-hint {
  font-size: 10.5px;
  color: rgb(var(--md-on-surface-variant));
}
.note {
  margin: 0;
  font-size: 11.5px;
  line-height: 1.6;
  color: rgb(var(--md-on-surface-muted));
}
.foot {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  padding: 12px 18px 16px;
}
.btn-text {
  height: 36px;
  padding: 0 16px;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-primary));
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: background-color 0.14s ease;
}
.btn-text:hover {
  background-color: rgb(var(--md-primary) / 0.08);
}
.btn-filled {
  height: 36px;
  padding: 0 20px;
  border: 0;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: filter 0.14s ease;
}
.btn-filled:hover {
  filter: brightness(1.06);
}
.btn-text:focus-visible,
.btn-filled:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.16s ease;
}
.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
@media (prefers-reduced-motion: reduce) {
  .icon-btn,
  .seg-btn,
  .btn-text,
  .btn-filled,
  .fade-enter-active,
  .fade-leave-active {
    transition: none;
  }
}
</style>
