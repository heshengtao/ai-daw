<script setup lang="ts">
/**
 * 插件原生界面（远程）：容器里没有窗口系统，插件仍会在虚拟显示上画出自己的界面。
 * 后端把那块屏幕编码成 JPEG 帧推过来，这里贴在 canvas 上，鼠标/键盘再送回去——
 * 看上去就是"插件窗口被放进了网页"。
 *
 * 桌面壳里不会用到这个组件（那边直接弹原生窗口，保真度更高）。
 */
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import type { PluginGuiStream } from "@/api/tauri";
import { useI18n } from "@/i18n";

const props = defineProps<{ stream: PluginGuiStream; title: string }>();
const emit = defineEmits<{ close: []; error: [message: string] }>();

const i18n = useI18n();
const canvas = ref<HTMLCanvasElement | null>(null);
const status = ref<"connecting" | "live" | "closed" | "error">("connecting");
const hint = ref("");
let socket: WebSocket | null = null;
let lastSize = { width: 0, height: 0 };
let closeRequested = false;

/** 画布逻辑尺寸 = 虚拟屏尺寸；样式上等比缩放到容器宽度。 */
const viewSize = computed(() => ({
  width: props.stream.width,
  height: props.stream.height,
}));

function wsUrl(path: string): string {
  const url = new URL(window.location.href);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  url.pathname = path;
  url.search = "";
  url.hash = "";
  return url.toString();
}

function draw(bytes: Uint8Array<ArrayBuffer>): void {
  const target = canvas.value;
  if (!target) return;
  const context = target.getContext("2d");
  if (!context) return;
  const blob = new Blob([bytes], { type: "image/jpeg" });
  const bitmap = typeof createImageBitmap === "function"
    ? createImageBitmap(blob)
    : null;
  if (bitmap) {
    bitmap.then((image) => {
      if (target.width !== image.width || target.height !== image.height) {
        target.width = image.width;
        target.height = image.height;
        lastSize = { width: image.width, height: image.height };
      }
      context.drawImage(image, 0, 0);
      image.close?.();
    }).catch(() => {});
    return;
  }
  // 老浏览器退回 <img> 解码。
  const url = URL.createObjectURL(blob);
  const image = new Image();
  image.onload = () => {
    if (target.width !== image.width || target.height !== image.height) {
      target.width = image.width;
      target.height = image.height;
      lastSize = { width: image.width, height: image.height };
    }
    context.drawImage(image, 0, 0);
    URL.revokeObjectURL(url);
  };
  image.src = url;
}

function connect(): void {
  socket = new WebSocket(wsUrl(props.stream.url));
  socket.binaryType = "arraybuffer";
  socket.onopen = () => {
    status.value = "live";
  };
  socket.onmessage = (event) => {
    if (event.data instanceof ArrayBuffer) {
      draw(new Uint8Array(event.data as ArrayBuffer));
    }
  };
  socket.onerror = () => {
    status.value = "error";
    hint.value = i18n.t("editor.remoteError");
    emit("error", hint.value);
  };
  socket.onclose = () => {
    if (!closeRequested && status.value !== "error") {
      status.value = "closed";
      hint.value = i18n.t("editor.remoteClosed");
    }
  };
}

function send(payload: unknown): void {
  if (socket && socket.readyState === WebSocket.OPEN) {
    socket.send(JSON.stringify(payload));
  }
}

/** 浏览器坐标 → 虚拟屏像素（画布被 CSS 缩放过，必须按实际显示尺寸换算）。 */
function point(event: MouseEvent | WheelEvent): { x: number; y: number } {
  const target = canvas.value;
  if (!target) return { x: 0, y: 0 };
  const rect = target.getBoundingClientRect();
  const width = lastSize.width || props.stream.width;
  const height = lastSize.height || props.stream.height;
  const scaleX = width / Math.max(1, rect.width);
  const scaleY = height / Math.max(1, rect.height);
  return {
    x: (event.clientX - rect.left) * scaleX,
    y: (event.clientY - rect.top) * scaleY,
  };
}

function onMove(event: MouseEvent): void {
  const { x, y } = point(event);
  send({ type: "move", x, y });
}

function onDown(event: MouseEvent): void {
  const { x, y } = point(event);
  canvas.value?.focus();
  send({ type: "down", x, y, button: event.button + 1 });
}

function onUp(event: MouseEvent): void {
  const { x, y } = point(event);
  send({ type: "up", x, y, button: event.button + 1 });
}

function onWheel(event: WheelEvent): void {
  event.preventDefault();
  const { x, y } = point(event);
  send({ type: "wheel", x, y, delta: event.deltaY < 0 ? 1 : -1 });
}

function onKey(event: KeyboardEvent): void {
  // 让插件自己处理；单字符走 type（能处理大小写与输入法结果），其余按键发 keysym。
  if (event.key.length === 1 && !event.ctrlKey && !event.metaKey) {
    if (event.type === "keydown") send({ type: "text", text: event.key });
    event.preventDefault();
    return;
  }
  send({ type: "key", key: event.key, down: event.type === "keydown" });
  if (event.type === "keydown" && ["Tab", " ", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"].includes(event.key)) {
    event.preventDefault();
  }
}

function dispose(): void {
  closeRequested = true;
  socket?.close();
  socket = null;
}

function close(): void {
  dispose();
  emit("close");
}

onMounted(connect);
onBeforeUnmount(dispose);
</script>

<template>
  <div class="remote-gui">
    <header class="remote-gui__bar">
      <span class="remote-gui__title" :title="title">{{ title }}</span>
      <span class="remote-gui__status" :data-state="status">
        <template v-if="status === 'connecting'">{{ i18n.t("editor.remoteConnecting") }}</template>
        <template v-else-if="status === 'live'">{{ i18n.t("editor.remoteLive") }}</template>
        <template v-else-if="status === 'closed'">{{ i18n.t("editor.remoteClosed") }}</template>
        <template v-else>{{ i18n.t("editor.remoteError") }}</template>
      </span>
      <button class="remote-gui__close" type="button" @click="close">
        {{ i18n.t("editor.remoteClose") }}
      </button>
    </header>
    <div class="remote-gui__stage" :style="{ aspectRatio: `${viewSize.width} / ${viewSize.height}` }">
      <!-- eslint-disable-next-line vuejs-accessibility/no-static-element-interactions -->
      <canvas
        ref="canvas"
        class="remote-gui__canvas"
        tabindex="0"
        :width="viewSize.width"
        :height="viewSize.height"
        @mousemove="onMove"
        @mousedown="onDown"
        @mouseup="onUp"
        @wheel="onWheel"
        @keydown="onKey"
        @keyup="onKey"
        @contextmenu.prevent
      />
      <p v-if="hint" class="remote-gui__hint">{{ hint }}</p>
    </div>
  </div>
</template>

<style scoped>
.remote-gui {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-height: 0;
  flex: 1;
}
.remote-gui__bar {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
}
.remote-gui__title {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.remote-gui__status[data-state="live"] {
  color: var(--color-accent, #4ea1ff);
}
.remote-gui__status[data-state="error"],
.remote-gui__status[data-state="closed"] {
  color: var(--color-danger, #ff6b6b);
}
.remote-gui__stage {
  position: relative;
  width: 100%;
  background: #000;
  border-radius: 6px;
  overflow: hidden;
}
.remote-gui__canvas {
  display: block;
  width: 100%;
  height: 100%;
  outline: none;
  cursor: default;
}
.remote-gui__hint {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  margin: 0;
  color: #ddd;
  font-size: 12px;
}
</style>
