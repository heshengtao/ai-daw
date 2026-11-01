<script setup lang="ts">
/**
 * 插入槽：一行一个效果器，左侧小圆点是旁通开关，点名字进参数。
 *
 * 之所以把「开关」和「打开」拆成两个点击目标：混音时最常做的是**盲拧**（不打开界面、只
 * 开关对比），把两者混在一个按钮里会让人每次都要先进界面才能旁通。
 */
import { computed } from "vue";
import { useI18n } from "@/i18n";

const props = withDefaults(
  defineProps<{
    /** 效果器显示名。 */
    name: string;
    /** 是否启用（`false` = 旁通）。 */
    enabled: boolean;
    /** 侧链来源的显示名（空 = 没接）。 */
    sidechain?: string | null;
  }>(),
  { sidechain: null },
);

const emit = defineEmits<{
  (e: "toggle"): void;
  (e: "open"): void;
}>();

const i18n = useI18n();

const title = computed(() =>
  props.sidechain
    ? i18n.t("insert.withSidechain", { name: props.name, source: props.sidechain })
    : props.name,
);
</script>

<template>
  <div class="insert" :class="{ off: !enabled }">
    <button
      class="power"
      :aria-pressed="enabled"
      :aria-label="enabled ? i18n.t('insert.bypass') : i18n.t('insert.enable')"
      :title="enabled ? i18n.t('insert.bypass') : i18n.t('insert.enable')"
      @click.stop="emit('toggle')"
    >
      <span class="dot" />
    </button>
    <button class="label" :title="title" @click.stop="emit('open')">
      {{ name }}
    </button>
    <span v-if="sidechain" class="sc" :title="i18n.t('insert.sidechainFrom', { source: sidechain })"
      >SC</span
    >
  </div>
</template>

<style scoped>
.insert {
  display: flex;
  align-items: center;
  gap: 3px;
  min-width: 0;
  height: 18px;
  padding: 0 3px;
  border-radius: 6px;
  background-color: rgb(var(--md-on-surface) / 0.05);
}
.insert.off {
  opacity: 0.5;
}
.power {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 10px;
  height: 14px;
  padding: 0;
  border: 0;
  background: transparent;
  cursor: pointer;
}
.dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background-color: rgb(var(--md-primary));
  transition: background-color 0.14s ease, box-shadow 0.14s ease;
}
.insert.off .dot {
  background-color: rgb(var(--md-on-surface) / 0.35);
}
.label {
  flex: 1 1 auto;
  min-width: 0;
  padding: 0;
  border: 0;
  background: transparent;
  color: rgb(var(--md-on-surface));
  font-size: 9.5px;
  line-height: 14px;
  text-align: left;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  cursor: pointer;
}
.label:hover {
  color: rgb(var(--md-primary));
}
.sc {
  flex: 0 0 auto;
  font-size: 8px;
  font-weight: 700;
  letter-spacing: 0.3px;
  padding: 0 2px;
  border-radius: 3px;
  color: rgb(var(--md-on-primary));
  background-color: rgb(var(--md-primary));
}
</style>
