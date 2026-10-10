<script setup lang="ts">
/**
 * 列表工具条：搜索药丸 + 若干组筛选 chips + 排序分段控件。
 *
 * 原先这是 `PluginBrowser.vue` 里写死的一份（连同一段样式），且只服务「按来源筛选」**一组**
 * chip。参数页需要**两组互相独立**的 chip（绑定状态 + 仅可自动化，后者默认开、与前者叠加），
 * 所以这里把它改成 `chipGroups` 数组驱动；只传一组时渲染结果与改动前逐项一致。
 *
 * 本组件**不**承载任何取值逻辑：query / chip / sort 都是 props，改动用 emit 回写，
 * 「筛选一变就回到第 1 页」这类规矩由父组件负责。
 *
 * 搜索框的焦点也归父组件：`focusSearch()` 通过 `defineExpose` 暴露出去，因为只有父组件
 * 知道「什么时候算刚打开」（各模态的「打开即搜」约定）。
 */
import { computed, ref } from "vue";
import { MagnifyingGlassIcon, XMarkIcon } from "@heroicons/vue/24/outline";

/** 一组互斥 chip：`active` 为空串表示这组当前没有选中项（例如「仅可自动化」被关掉时）。 */
interface ChipGroup {
  /** 组的稳定 id（回写 `update:activeChip` 时用它区分是哪一组）。 */
  id: string;
  /** 无障碍名（`role="group"` 的 `aria-label`）。 */
  label: string;
  chips: { id: string; label: string }[];
  active: string;
}

/** 排序分段控件的一项。 */
interface ToolbarSort {
  id: string;
  label: string;
}

const props = defineProps<{
  query: string;
  /** 搜索框占位文案。 */
  placeholder: string;
  /** 搜索框的无障碍名（可能与占位文案不同）。 */
  searchLabel: string;
  /** 清空按钮的无障碍名。 */
  clearLabel: string;
  chipGroups: ChipGroup[];
  /** 「排序」这个小标签的可见文案。 */
  sortLabel: string;
  /** 排序分段控件的无障碍名。 */
  sortAriaLabel: string;
  sorts: ToolbarSort[];
  activeSort: string;
}>();

const emit = defineEmits<{
  (e: "update:query", value: string): void;
  /** chip 被点：带上组 id，父组件据此决定改哪一组。 */
  (e: "update:activeChip", value: { groupId: string; chipId: string }): void;
  (e: "update:activeSort", value: string): void;
}>();

const searchEl = ref<HTMLInputElement | null>(null);

/**
 * 搜索框走 `v-model`（可写计算属性），**不要**手写 `:value` + `@input`：
 * `v-model` 在文本输入上用 `vModelText`，会识别输入法组合（compositionstart/end），
 * 中文输入法下不会拿中间态去搜；手写 `@input` 会把这个行为丢掉。
 */
const text = computed({
  get: () => props.query,
  set: (value: string) => emit("update:query", value),
});

/** 把焦点放进搜索框（父组件在打开时调）。 */
function focusSearch(): void {
  searchEl.value?.focus();
}

defineExpose({ focusSearch });
</script>

<template>
  <div class="toolbar">
    <label class="search">
      <MagnifyingGlassIcon class="search-icon" />
      <input
        ref="searchEl"
        v-model="text"
        class="search-input"
        type="text"
        spellcheck="false"
        autocomplete="off"
        :placeholder="placeholder"
        :aria-label="searchLabel"
      />
      <button v-if="query" class="search-clear" :aria-label="clearLabel" @click="emit('update:query', '')">
        <XMarkIcon class="w-[14px] h-[14px]" />
      </button>
    </label>

    <div
      v-for="group in chipGroups"
      :key="group.id"
      class="chips"
      role="group"
      :aria-label="group.label"
    >
      <button
        v-for="c in group.chips"
        :key="c.id"
        class="chip"
        :class="{ on: group.active === c.id }"
        :aria-pressed="group.active === c.id"
        @click="emit('update:activeChip', { groupId: group.id, chipId: c.id })"
      >
        {{ c.label }}
      </button>
    </div>

    <div class="sorts">
      <span class="sorts-label">{{ sortLabel }}</span>
      <div class="seg" role="group" :aria-label="sortAriaLabel">
        <button
          v-for="s in sorts"
          :key="s.id"
          class="seg-btn"
          :class="{ on: activeSort === s.id }"
          :aria-pressed="activeSort === s.id"
          @click="emit('update:activeSort', s.id)"
        >
          {{ s.label }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* ---- 工具条 ---- */
.toolbar {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 0 22px 12px;
}
.search {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 34px;
  padding: 0 10px 0 12px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.6);
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container));
}
.search:focus-within {
  border-color: rgb(var(--md-primary));
  box-shadow: 0 0 0 2px rgb(var(--md-primary) / 0.2);
}
.search-icon {
  width: 16px;
  height: 16px;
  flex: 0 0 auto;
  color: rgb(var(--md-on-surface-variant));
}
.search-input {
  flex: 1 1 auto;
  min-width: 0;
  border: 0;
  background: transparent;
  color: rgb(var(--md-on-surface));
  font-size: 13px;
  outline: none;
}
.search-clear {
  width: 24px;
  height: 24px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
}
.search-clear:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.chip {
  height: 28px;
  padding: 0 12px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 11.5px;
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease, border-color 0.14s ease;
}
.chip:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.chip.on {
  background-color: rgb(var(--md-secondary-container));
  border-color: transparent;
  color: rgb(var(--md-on-secondary-container));
}
.chip:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.sorts {
  display: flex;
  align-items: center;
  gap: 8px;
}
.sorts-label {
  font-size: 10.5px;
  letter-spacing: 0.6px;
  color: rgb(var(--md-on-surface-variant));
}
.seg {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 2px;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container));
}
.seg-btn {
  height: 24px;
  padding: 0 12px;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 11px;
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease;
}
.seg-btn.on {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.seg-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
</style>
