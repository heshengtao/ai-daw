<script setup lang="ts">
/**
 * 记谱设置（谱号 + 调弦）：从检查器的「谱号」那一行点开。
 *
 * 只影响**这条轨道怎么排**（显示偏好，存在本机），不改音符本身：
 * - 谱号：自动 / 高音 / 低音 / 中音 / 打击乐；
 * - 调弦：只在六线谱下有意义（标准 / Drop D / 降半音 / 贝斯 5 弦）。
 *
 * 点选即生效、即关闭——这里没有「确定才会写」的中间态，用户点一下就应该看到谱面变了。
 */
import { computed } from "vue";
import ModalShell from "@/components/common/ModalShell.vue";
import { useI18n } from "@/i18n";
import { useLayoutStore } from "@/stores/layout";
import { useSelectionStore } from "@/stores/selection";
import { defaultNotationFor, isTabNotation } from "@/utils/notation";

const props = defineProps<{ open: boolean; trackId: number | null }>();
const emit = defineEmits<{ (e: "update:open", value: boolean): void }>();

const i18n = useI18n();
const layout = useLayoutStore();
const selection = useSelectionStore();

const CLEFS: { id: "G" | "F" | "C" | "percussion"; labelKey: string }[] = [
  { id: "G", labelKey: "score.clefTreble" },
  { id: "F", labelKey: "score.clefBass" },
  { id: "C", labelKey: "score.clefAlto" },
  { id: "percussion", labelKey: "score.clefPercussion" },
];

/** 调弦预设（低 → 高）。 */
const TUNINGS: { id: string; labelKey: string; values: number[] | null }[] = [
  { id: "standard", labelKey: "score.tuningStandard", values: null },
  { id: "dropd", labelKey: "score.tuningDropD", values: [38, 45, 50, 55, 59, 64] },
  { id: "eb", labelKey: "score.tuningEb", values: [39, 44, 49, 54, 58, 63] },
  { id: "bass5", labelKey: "score.tuningBass5", values: [23, 28, 33, 38, 43] },
];

const currentClef = computed(() => (props.trackId === null ? null : (layout.trackClefs[String(props.trackId)] ?? null)));
const currentTuning = computed(() => {
  if (props.trackId === null) return "standard";
  const values = layout.trackTunings[String(props.trackId)];
  if (!values) return "standard";
  const hit = TUNINGS.find((t) => t.values && t.values.join(",") === values.join(","));
  return hit?.id ?? "standard";
});

/** 只有六线谱才显示调弦——五线谱下调弦没有意义。 */
const showTuning = computed(() => {
  const track = selection.track;
  if (!track) return false;
  return isTabNotation(layout.trackNotationOf(track.id) ?? defaultNotationFor(track));
});

function pickClef(clef: "G" | "F" | "C" | "percussion" | null): void {
  if (props.trackId === null) return;
  layout.setTrackClef(props.trackId, clef);
}

function pickTuning(id: string): void {
  if (props.trackId === null) return;
  const preset = TUNINGS.find((t) => t.id === id);
  layout.setTrackTuning(props.trackId, preset?.values ?? null);
}

function close(): void {
  emit("update:open", false);
}
</script>

<template>
  <ModalShell
    :open="open"
    :title="i18n.t('score.notationTitle')"
    width="460px"
    @update:open="emit('update:open', $event)"
  >
    <div class="body">
      <div class="field">
        <span class="field-label">{{ i18n.t("score.clef") }}</span>
        <div class="choices">
          <button class="choice" :class="{ on: currentClef === null }" @click="pickClef(null)">
            {{ i18n.t("score.clefAuto") }}
          </button>
          <button
            v-for="c in CLEFS"
            :key="c.id"
            class="choice"
            :class="{ on: currentClef === c.id }"
            @click="pickClef(c.id)"
          >
            {{ i18n.t(c.labelKey) }}
          </button>
        </div>
      </div>

      <div v-if="showTuning" class="field">
        <span class="field-label">{{ i18n.t("score.tuning") }}</span>
        <div class="choices">
          <button
            v-for="t in TUNINGS"
            :key="t.id"
            class="choice"
            :class="{ on: currentTuning === t.id }"
            @click="pickTuning(t.id)"
          >
            {{ i18n.t(t.labelKey) }}
          </button>
        </div>
      </div>
    </div>

    <template #foot>
      <footer class="foot">
        <button class="btn-filled" @click="close">{{ i18n.t("common.close") }}</button>
      </footer>
    </template>
  </ModalShell>
</template>

<style scoped>
.body {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 4px 22px 8px;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.field-label {
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
}
.choices {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.choice {
  height: 30px;
  padding: 0 14px;
  border: 1px solid rgb(var(--md-outline) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface));
  font-size: 12px;
  cursor: pointer;
  transition: border-color 0.15s ease, background-color 0.15s ease;
}
.choice:hover {
  border-color: rgb(var(--md-primary) / 0.8);
}
.choice.on {
  border-color: rgb(var(--md-primary));
  background-color: rgb(var(--md-primary) / 0.1);
}
.choice:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.foot {
  display: flex;
  justify-content: flex-end;
  padding: 8px 18px 16px;
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
}
</style>
