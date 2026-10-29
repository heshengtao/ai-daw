<script setup lang="ts">
/**
 * 导入对话框：音频 / MIDI / MusicXML 三选一。
 *
 * 和导出对话框对称：那里是「按什么格式交出去」，这里是「按什么来源拿进来」。之前音频导入
 * 是一颗图标直接弹系统文件框，现在多了两种来源，得先让用户选**要导入什么**再选文件——
 * 否则同一个过滤器列表里混着 wav 和 mid，用户根本不知道点开以后会发生什么。
 *
 * 三种来源的落地方式不同，所以卡片上的说明是必须的：
 * - 音频：插到当前轨道的播放头处，**不动**现有内容；
 * - MIDI / 乐谱：解析成工程模型后**整体替换**当前工程（替换前的工程会进历史，可以找回）。
 */
import { ref } from "vue";
import ModalShell from "@/components/common/ModalShell.vue";
import { useI18n } from "@/i18n";

export type ImportKind = "audio" | "midi" | "musicxml";

defineProps<{ open: boolean }>();

const emit = defineEmits<{
  (e: "update:open", value: boolean): void;
  (e: "confirm", kind: ImportKind): void;
}>();

const i18n = useI18n();

const kind = ref<ImportKind>("audio");

/** 三张卡片的顺序：最常用的音频放最上面。 */
const OPTIONS: { id: ImportKind; title: string; hint: string; note: string }[] = [
  { id: "audio", title: "import.audio", hint: "import.audioHint", note: "import.audioNote" },
  { id: "midi", title: "import.midi", hint: "import.midiHint", note: "import.replaceNote" },
  { id: "musicxml", title: "import.score", hint: "import.scoreHint", note: "import.replaceNote" },
];

function confirm(): void {
  emit("confirm", kind.value);
  emit("update:open", false);
}
</script>

<template>
  <ModalShell
    :open="open"
    :title="i18n.t('import.title')"
    width="470px"
    @update:open="emit('update:open', $event)"
  >
    <div class="body">
      <div class="choices" role="radiogroup" :aria-label="i18n.t('import.title')">
        <button
          v-for="opt in OPTIONS"
          :key="opt.id"
          class="choice"
          :class="{ on: kind === opt.id }"
          role="radio"
          :aria-checked="kind === opt.id"
          @click="kind = opt.id"
        >
          <span class="choice-title">{{ i18n.t(opt.title) }}</span>
          <span class="choice-hint">{{ i18n.t(opt.hint) }}</span>
        </button>
      </div>

      <p class="note">{{ i18n.t(OPTIONS.find((o) => o.id === kind)?.note ?? "import.audioNote") }}</p>
    </div>

    <template #foot>
      <footer class="foot">
        <button class="btn-text" @click="emit('update:open', false)">{{ i18n.t("common.cancel") }}</button>
        <button class="btn-filled" @click="confirm">{{ i18n.t("import.confirm") }}</button>
      </footer>
    </template>
  </ModalShell>
</template>

<style scoped>
.body {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 4px 22px 2px;
  overflow-y: auto;
}
.choices {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
/* 一张卡片 = 一个来源：标题一行、说明一行，选中时描边 + 淡底 */
.choice {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 3px;
  padding: 10px 14px;
  border: 1px solid rgb(var(--md-outline) / 0.7);
  border-radius: 16px;
  background-color: transparent;
  color: rgb(var(--md-on-surface));
  text-align: left;
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
.choice-title {
  font-size: 13.5px;
  font-weight: 500;
}
.choice-hint {
  font-size: 11px;
  line-height: 1.5;
  color: rgb(var(--md-on-surface-variant));
}
.note {
  margin: 2px 0 0;
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
}
.btn-filled:hover {
  filter: brightness(1.06);
}
.btn-text:focus-visible,
.btn-filled:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
</style>
