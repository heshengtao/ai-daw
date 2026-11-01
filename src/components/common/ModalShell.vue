<script setup lang="ts">
/**
 * 通用模态外壳：遮罩 + 圆角对话框 + 标题栏（标题 + 关闭按钮）。
 *
 * 这份外壳原先在 `PluginBrowser.vue` 里独有一份（`Teleport` → `.scrim` → `.dialog` → `.head`，
 * 连同一段样式）。参数页要用同一套外壳，所以抽到这里，两边**共用一份**——类名与样式原样搬过来，
 * 视觉与交互逐项不变（`z-index:120`、`width:620px`、`max-height:78vh`、Escape 关闭、点遮罩关闭）。
 *
 * 只管四件事：Escape 关闭、点遮罩关闭、把标题同时用作 `role="dialog"` 的可访问名，
 * 以及用 `dismissible` 支持「必须先做一个选择才能继续」的强制流程。
 * 内容按插槽拼：`toolbar`（工具条）/ 默认（主体）/ `foot`（页脚）；**焦点归内容自己管**
 * （打开时聚焦搜索框这类约定由各页自己实现，见 `ListToolbar` 暴露的 `focusSearch()`）。
 */
import { XMarkIcon } from "@heroicons/vue/24/outline";
import { useI18n } from "@/i18n";

const props = withDefaults(
  defineProps<{
    open: boolean;
    /** 标题：既是标题栏的可见文案，也是对话框的 `aria-label`。 */
    title: string;
    /** 自定义宽度（默认 620px）；宽内容（例如历史工程卡片墙）用它。 */
    width?: string;
    /**
     * 是否允许关闭（X / Escape / 点遮罩）。
     *
     * 默认允许；「一个引擎都没有时必须先选一个」这类强制流程传 `false`——
     * 此时标题栏没有关闭按钮，只能走页脚里的显式动作。
     */
    dismissible?: boolean;
  }>(),
  { width: "", dismissible: true },
);

const emit = defineEmits<{ (e: "update:open", value: boolean): void }>();

const i18n = useI18n();

function close(): void {
  if (!props.dismissible) return;
  emit("update:open", false);
}

/** Escape 关闭；`stopPropagation` 免得这一个按键再触发外层的全局快捷键。 */
function onKey(e: KeyboardEvent): void {
  if (e.key !== "Escape") return;
  // 强制流程里 Escape 不关闭，但也不该漏给后面的全局快捷键。
  e.stopPropagation();
  if (props.dismissible) close();
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
          :aria-label="title"
          :style="props.width ? { width: props.width } : undefined"
          tabindex="-1"
          @keydown="onKey"
        >
          <header class="head">
            <h2 class="title">{{ title }}</h2>
            <button
              v-if="dismissible"
              class="icon-btn"
              :aria-label="i18n.t('common.close')"
              @click="close"
            >
              <XMarkIcon class="w-[18px] h-[18px]" />
            </button>
          </header>

          <slot name="toolbar" />

          <slot />

          <slot name="foot" />
        </div>
      </div>
    </transition>
  </Teleport>
</template>

<style scoped>
.scrim {
  position: fixed;
  inset: 0;
  z-index: 120;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  background-color: rgb(var(--md-scrim) / 0.42);
}
.dialog {
  width: 620px;
  max-width: 100%;
  max-height: 78vh;
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
  justify-content: space-between;
  gap: 12px;
  padding: 16px 16px 10px 22px;
}
.title {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  color: rgb(var(--md-on-surface));
}
.icon-btn {
  width: 34px;
  height: 34px;
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
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.16s ease;
}
.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
@media (prefers-reduced-motion: reduce) {
  .fade-enter-active,
  .fade-leave-active {
    transition: none;
  }
}
</style>
