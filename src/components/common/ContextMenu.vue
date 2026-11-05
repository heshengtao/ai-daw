<script setup lang="ts">
/**
 * 右键菜单的宿主：固定定位交给 `ContextMenuList`，这里只负责「什么时候关」。
 *
 * 关闭时机：点外面 / 窗口尺寸变化 / 菜单外的滚动 / Esc。菜单自身滚动（层级多时会出现
 * 纵向滚动条）不能把菜单关掉，所以滚动的来源要先排除菜单内部。
 *
 * 用 `Teleport` 挂到 `body`：时间轴的滚动容器有 `overflow: auto`，留在里面会被裁掉；
 * 二级菜单用 `position: fixed`，同样不会被父级滚动容器裁切。
 */
import { onMounted, onUnmounted, ref } from "vue";
import type { ContextMenuItem } from "./contextMenu";
import ContextMenuList from "./ContextMenuList.vue";

const props = defineProps<{
  open: boolean;
  x: number;
  y: number;
  items: ContextMenuItem[];
}>();

const emit = defineEmits<{
  (e: "select", id: string): void;
  (e: "close"): void;
}>();

/** 菜单宿主：点它内部不算「点外面」。`display: contents` 不产生盒子，只用来做包含判断。 */
const hostEl = ref<HTMLElement | null>(null);

function onPointerDown(e: PointerEvent): void {
  if (!props.open) return;
  const el = hostEl.value;
  if (el && e.target instanceof Node && el.contains(e.target)) return;
  emit("close");
}

function onKeyDown(e: KeyboardEvent): void {
  if (props.open && e.key === "Escape") {
    e.preventDefault();
    emit("close");
  }
}

/** 滚动时间轴 / 缩放窗口后菜单的锚点就失效了，直接收起；菜单自己滚不算。 */
function onDismiss(e: Event): void {
  if (!props.open) return;
  const el = hostEl.value;
  if (el && e.target instanceof Node && el.contains(e.target)) return;
  emit("close");
}

function onResize(): void {
  if (props.open) emit("close");
}

/** 菜单外的滚轮同样让锚点失效（菜单内部滚轮不关）。 */
function onWheel(e: WheelEvent): void {
  if (!props.open) return;
  const el = hostEl.value;
  if (el && e.target instanceof Node && el.contains(e.target)) return;
  emit("close");
}

function onSelect(id: string): void {
  emit("select", id);
  emit("close");
}

onMounted(() => {
  window.addEventListener("pointerdown", onPointerDown, true);
  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("wheel", onWheel, { passive: true, capture: true });
  window.addEventListener("resize", onResize);
  window.addEventListener("blur", onResize);
  window.addEventListener("scroll", onDismiss, true);
});
onUnmounted(() => {
  window.removeEventListener("pointerdown", onPointerDown, true);
  window.removeEventListener("keydown", onKeyDown);
  window.removeEventListener("wheel", onWheel, true);
  window.removeEventListener("resize", onResize);
  window.removeEventListener("blur", onResize);
  window.removeEventListener("scroll", onDismiss, true);
});
</script>

<template>
  <Teleport to="body">
    <div v-if="open" ref="hostEl" class="ctx-host">
      <ContextMenuList :items="items" :x="x" :y="y" @select="onSelect" />
    </div>
  </Teleport>
</template>

<style scoped>
.ctx-host {
  display: contents;
}
</style>
