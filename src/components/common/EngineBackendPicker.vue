<script setup lang="ts">
/**
 * audio.cpp 引擎的**后端选择器**：CPU / Vulkan / CUDA 12 / CUDA 13。
 *
 * 它和设置页上面的「ONNX Runtime 引擎后端」是**两套独立**的东西：ONNX Runtime 在主进程里，
 * 决定歌声 / 分离跑在哪；audio.cpp 是独立子进程，自己带一套 CUDA，换它不需要重启应用，
 * 只要重启引擎。
 *
 * 控件本身直接复用 `Md3Segmented`（滑块胶囊），和 ONNX Runtime 那个完全同一个组件。
 */
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import type { RuntimeStatus } from "@/api/types";
import ConfirmDialog from "@/components/common/ConfirmDialog.vue";
import Md3Segmented from "@/components/common/Md3Segmented.vue";
import type { SegmentedOption } from "@/components/common/segmented";
import { useI18n } from "@/i18n";
import { lastWorkspace } from "@/router";
import { useDownloadsStore } from "@/stores/downloads";
import { useLayoutStore } from "@/stores/layout";
import { useRuntimesStore } from "@/stores/runtimes";
import { useYueStore } from "@/stores/yue";
import { humanSize } from "@/utils/models";

withDefaults(defineProps<{ compact?: boolean; row?: boolean }>(), { compact: false, row: false });

const i18n = useI18n();
const layout = useLayoutStore();
const runtimes = useRuntimesStore();
const downloads = useDownloadsStore();
const yue = useYueStore();
const router = useRouter();

/** 点了但还没装的包：先确认再下（一个引擎包几百 MB 到 1 GB，不静默开始）。 */
const pending = ref<RuntimeStatus | null>(null);
/** 正在切换：清单刷新 + 写偏好之间不允许再点，避免连点把确认框吞掉。 */
const switching = ref(false);
/** 刚切完后端：问一句要不要现在重启引擎（加载几 GB 权重，不替用户做主）。 */
const restartOpen = ref(false);
const restartName = ref("");

/** 确认框正文：知道体积就报体积。 */
const pendingBody = computed(() => {
  const item = pending.value;
  if (!item) return "";
  const name = labelFor(item);
  const size = humanSize(item.bytes);
  return size
    ? i18n.t("engine.downloadBody", { name, size })
    : i18n.t("engine.downloadBodyUnknown", { name });
});

/**
 * 只列 audio.cpp 的引擎包（ONNX Runtime / CUDA 组件由上面那套控件管）。
 *
 * 只留本机能跑的：NVIDIA 机器上摆 ROCm、AMD 机器上摆 CUDA，点了也只会得到
 * 「在本机不可用」。当前选中的那一项例外——万一换过显卡，得让它看得见、切得走。
 */
const options = computed(() =>
  runtimes.items.filter((r) => r.group === "audiocpp" && (r.usable || r.selected)),
);

function deviceOf(item: RuntimeStatus): string {
  return item.id.startsWith("audiocpp-") ? item.id.slice("audiocpp-".length) : item.id;
}

function labelFor(item: RuntimeStatus): string {
  return i18n.t(`engine.device.${deviceOf(item)}`);
}

const segOptions = computed<SegmentedOption[]>(() =>
  options.value.map((item) => ({
    value: item.id,
    label: labelFor(item),
    // 刷新 / 写入偏好期间禁点：防止连点把确认框吞掉。
    disabled: switching.value,
    // 不要在滑块上挂 `title`：未安装的后端会冒出一句「CUDA 13 还没下载」，
    // 明明当前是「自动选择 · CPU」，看起来像多出来的报错。要说明就点它，
    // 会走 `pick()` 的确认框 → 下载管理器；已安装/推荐状态由上面那行文字负责。
  })),
);

/**
 * 当前生效的那一份：后端即使处于「没手动选过」也会在 `selected` 上标出它
 * 实际会用的后端（CPU / Vulkan / CUDA）。界面不再提供「自动」这一项，
 * 避免用户把「没下 CUDA 组件」误读成「显卡不支持 CUDA」。
 */
const selectedValue = computed(() => options.value.find((o) => o.selected)?.id ?? "");

/** 当前选中那一项（用于「当前使用 + 设备」这行说明）。 */
const current = computed(() => options.value.find((o) => o.selected) ?? null);

/**
 * 这一段**永远渲染**：没有引擎包时给一句「当前平台没有包」，而不是整块消失。
 * 以前父组件按「有没有引擎包」把整张卡片藏掉，用户只会看到 ONNX Runtime 的后端，
 * 以为 audio.cpp 引擎后端不见了。
 */
const statusText = computed(() => {
  if (!options.value.length && runtimes.loading) return i18n.t("common.loading");
  if (!options.value.length) return i18n.t("engine.noPack");
  const item = current.value;
  if (!item) return i18n.t("engine.empty");
  const mode = i18n.t(item.automatic ? "engine.auto" : "engine.manual");
  return i18n.t(item.installed ? "engine.current" : "engine.notReady", {
    name: labelFor(item),
    mode,
  });
});

/** 当前那一份装没装（未安装时标题已经说了，这里只在已安装时加一个确认）。 */
const statusHint = computed(() => {
  const item = current.value;
  if (!item?.installed) return "";
  return i18n.t("engine.installedHint");
});

/**
 * 打开下载管理器并落到「运行时」。
 *
 * 设置页是独立路由，只切侧栏标签用户是看不到的——得先回工作区，
 * 否则点完像「什么也没发生」。
 */
function openManager(target?: string): void {
  downloads.jumpTo("runtime", target ?? "*");
  if (router.currentRoute.value.path.startsWith("/settings")) {
    void router.push(lastWorkspace.value);
  }
}

async function pick(value: string): Promise<void> {
  if (switching.value) return;
  const item = options.value.find((o) => o.id === value);
  if (!item) return;
  if (!item.installed) {
    // 没装：先问一句，确认后跳去下载并自动切过去（和 ONNX Runtime 那套一致）。
    pending.value = item;
    return;
  }
  switching.value = true;
  try {
    // 清单可能是上一次切换之前的快照：先用磁盘上的真实状态刷新，
    // 否则「刚切到 CPU 又点回 CUDA」会被旧的 selected 当成「已经选中」而吞掉。
    await runtimes.load();
    const fresh = options.value.find((o) => o.id === value) ?? item;
    if (!fresh.installed) {
      pending.value = fresh;
      return;
    }
    if (selectedValue.value === value) return;
    await apply(fresh);
  } catch (err) {
    layout.notify(String(err));
  } finally {
    switching.value = false;
  }
}

/**
 * 「去下载」：打开管理器 → 开始下载 → 等装完 → 选中它。
 *
 * 以前这里只弹一句提示 + 切标签：设置页里标签根本看不见，也不会真的开始下载，
 * 看起来就像 audio.cpp 引擎后端没有实现。
 */
async function download(): Promise<void> {
  const item = pending.value;
  pending.value = null;
  if (!item) return;
  openManager(item.id);
  const ok = await runtimes.installAndWait(item.id);
  if (!ok) return;
  await apply(item);
}

async function apply(item: RuntimeStatus): Promise<void> {
  if (!(await runtimes.select(item.id))) {
    // 设置页 / AI 面板没有管理器的 error watcher：不在这里说，用户只会看到「点了没反应」。
    if (runtimes.error) layout.notify(runtimes.error);
    return;
  }
  // 引擎在跑的时候换后端不会热切：先停掉，再问要不要现在重启。
  if (yue.engine && (yue.engine.state === "ready" || yue.engine.state === "starting")) {
    await yue.stop();
  }
  restartName.value = labelFor(item);
  restartOpen.value = true;
}

/** 现在重启引擎（调用方已经确认）；失败原因直接弹出来。 */
async function restartEngine(): Promise<void> {
  restartOpen.value = false;
  await yue.start(null);
  layout.notify(yue.error ? yue.error : i18n.t("engine.restartStarted"));
}

/** 稍后：引擎保持停止，下次生成时会按新后端拉起。 */
function restartLater(): void {
  restartOpen.value = false;
  layout.notify(i18n.t("engine.changed"));
}

const restartBody = computed(() =>
  i18n.t("engine.restartBody", { name: restartName.value || i18n.t("engine.backend") }),
);

function dismissPending(open: boolean): void {
  if (!open) pending.value = null;
}

onMounted(() => {
  // 设置页 / AI 面板会自己拉清单；这里只兜底，且避免和父组件的 load 撞车。
  if (runtimes.items.length === 0 && !runtimes.loading) void runtimes.load();
});
</script>

<template>
  <div class="engine-backend" :class="{ compact, inline: row }">
    <div class="engine-head">
      <span class="engine-title">{{ i18n.t("engine.backend") }}</span>
      <span class="engine-hint">{{ i18n.t("engine.backendHint") }}</span>
      <span class="engine-status">
        {{ statusText }}
        <span v-if="statusHint" class="engine-state">{{ statusHint }}</span>
      </span>
    </div>
    <Md3Segmented
      v-if="options.length"
      :model-value="selectedValue"
      :options="segOptions"
      :size="compact ? 'sm' : 'md'"
      :aria-label="i18n.t('engine.backend')"
      @update:model-value="pick"
    />

    <!-- 没装的后端：确认后下载，装完自动切过去 -->
    <ConfirmDialog
      :open="pending !== null"
      :title="i18n.t('engine.downloadTitle')"
      :message="pendingBody"
      :confirm-label="i18n.t('engine.downloadGo')"
      @confirm="download"
      @update:open="dismissPending"
    />

    <!-- 切完后端：不替用户自动加载几 GB 权重，先问一句 -->
    <ConfirmDialog
      :open="restartOpen"
      :title="i18n.t('engine.restartTitle')"
      :message="restartBody"
      :confirm-label="i18n.t('engine.restartNow')"
      :cancel-label="i18n.t('engine.restartLater')"
      @confirm="restartEngine"
      @cancel="restartLater"
      @update:open="(open: boolean) => { if (!open) restartOpen = false; }"
    />
  </div>
</template>

<style scoped>
.engine-backend {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 10px;
}
.engine-backend.compact {
  gap: 8px;
}
.engine-head {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.engine-title {
  font-size: 13px;
  font-weight: 600;
}
.engine-hint {
  font-size: 11px;
  line-height: 1.45;
  opacity: 0.62;
}
.engine-status {
  font-size: 12px;
  line-height: 1.5;
  color: rgb(var(--md-on-surface));
}
.engine-state {
  margin-left: 6px;
  opacity: 0.62;
}

/*
 * `row`：放进设置页那种「左标题 / 右控件」的行里，和 ONNX Runtime 后端那一行
 * 用完全相同的排版（字号、间距、行高都一样）。
 */
.engine-backend.inline {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  column-gap: 20px;
  min-height: 62px;
  padding: 10px 0;
  box-sizing: border-box;
}
.engine-backend.inline .engine-head {
  grid-column: 1;
  grid-row: 1;
  gap: 3px;
  min-width: 0;
}
.engine-backend.inline .engine-title {
  font-size: 14px;
  font-weight: 500;
  color: rgb(var(--md-on-surface));
}
.engine-backend.inline .engine-hint {
  font-size: 12px;
  line-height: 1.5;
  color: rgb(var(--md-on-surface-variant));
  opacity: 1;
}
.engine-backend.inline .engine-status {
  font-size: 12px;
  line-height: 1.5;
  color: rgb(var(--md-on-surface-variant));
}
.engine-backend.inline :deep(.md3-seg) {
  grid-column: 2;
  grid-row: 1;
  justify-self: end;
}
</style>
