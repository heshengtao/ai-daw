<script setup lang="ts">
/**
 * **audio.cpp 引擎安装向导**：一个引擎都没有 / 想换后端时，先在这里选一份再下载。
 *
 * 为什么单独做一个对话框：引擎包是 100 MB – 1 GB 的可选下载，安装包默认一个字节都不带。
 * 直接让用户面对一个「启动引擎」按钮，只会在点下去之后换来一句「还没装引擎」；这里把
 * 平台支持的几份（CPU / Vulkan / CUDA 12 / CUDA 13 / Metal）连同体积、适用显卡摆出来，
 * 选完就跳下载管理器开始下载，装完自动选中。
 *
 * 两种用法：
 * - 写歌页在「一份引擎都没有」时**强制**弹出（`dismissible = false`，没有关闭按钮），
 *   只能选一份去下载，或返回列表；
 * - 已经装好时作为「更换后端」的普通对话框（`dismissible = true`）。
 */
import { computed, ref, watch } from "vue";
import type { RuntimeStatus } from "@/api/types";
import ConfirmDialog from "@/components/common/ConfirmDialog.vue";
import ModalShell from "@/components/common/ModalShell.vue";
import { useI18n } from "@/i18n";
import { useDownloadsStore } from "@/stores/downloads";
import { useLayoutStore } from "@/stores/layout";
import { useRuntimesStore } from "@/stores/runtimes";
import { useYueStore } from "@/stores/yue";
import { humanSize } from "@/utils/models";

const props = withDefaults(
  defineProps<{
    open: boolean;
    /** 是否可以随便关掉；强制安装流程传 `false`。 */
    dismissible?: boolean;
  }>(),
  { dismissible: false },
);

const emit = defineEmits<{
  (e: "update:open", value: boolean): void;
  /** 用户选择「返回 / 取消」：强制流程里由父组件决定退回哪一页。 */
  (e: "cancel"): void;
  /** 引擎已装好并选中。 */
  (e: "done", id: string): void;
}>();

const i18n = useI18n();
const layout = useLayoutStore();
const runtimes = useRuntimesStore();
const downloads = useDownloadsStore();
const yue = useYueStore();

/** 平台支持的 audio.cpp 引擎包（ONNX Runtime / CUDA 组件不在这里）。 */
const packs = computed(() => runtimes.items.filter((r) => r.group === "audiocpp"));

const picked = ref("");
/** 刚切完后端：问一句要不要现在重启引擎（加载几 GB 权重，不替用户做主）。 */
const restartOpen = ref(false);
const restartName = ref("");

const selected = computed(() => packs.value.find((p) => p.id === picked.value) ?? null);
const canConfirm = computed(() => selected.value !== null);
const confirmLabel = computed(() =>
  selected.value?.installed ? i18n.t("yue.engineSetupUse") : i18n.t("yue.engineSetupGo"),
);

function deviceOf(item: RuntimeStatus): string {
  return item.id.startsWith("audiocpp-") ? item.id.slice("audiocpp-".length) : item.id;
}

function labelFor(item: RuntimeStatus): string {
  return i18n.t(`engine.device.${deviceOf(item)}`);
}

/** 后端清单里的中文说明按设备翻成当前语言；没有词条时退回后端那一份。 */
function noteFor(item: RuntimeStatus): string {
  const key = `engine.device.${deviceOf(item)}.note`;
  const text = i18n.t(key);
  return text === key ? item.note : text;
}

/**
 * 默认选哪一份：当前在用的 → 已装好的 → **本机最优**（不要求已装）→ CPU 兜底。
 *
 * 关键在第三档：以前这里直接退到 `core`（CPU），于是全新 N 卡机器打开向导看到的是
 * 「CPU」被选中，用户很容易以为软件不支持显卡。
 */
function pickDefault(): void {
  if (packs.value.some((p) => p.id === picked.value)) return;
  const best =
    packs.value.find((p) => p.installed && p.selected) ??
    packs.value.find((p) => p.usable && p.recommended) ??
    packs.value.find((p) => p.installed) ??
    packs.value.find((p) => p.core) ??
    packs.value[0];
  picked.value = best?.id ?? "";
}

async function ensurePacks(): Promise<void> {
  if (!packs.value.length && !runtimes.loading) await runtimes.load();
  pickDefault();
}

watch(
  () => props.open,
  (open) => {
    if (open) void ensurePacks();
  },
  { immediate: true },
);

// 清单晚到（或下载完刷新）时补一次默认选中。
watch(
  () => packs.value.map((p) => `${p.id}:${p.installed}`).join("|"),
  () => {
    if (props.open) pickDefault();
  },
);

function close(): void {
  emit("update:open", false);
}

function cancel(): void {
  close();
  emit("cancel");
}

/** 装一份引擎并选中它。
 *
 * `jump` 决定「跳下载管理器看着下」还是「留在本页后台下」——两条路都走同一条串行队列，
 * 不会因为用户先前点过别的下载而被静默吞掉。
 */
async function installPack(item: RuntimeStatus, jump: boolean): Promise<boolean> {
  const ok = await downloads.run(
    [{ section: "engine", id: item.id, label: labelFor(item), bytes: item.bytes }],
    { jump },
  );
  if (!ok) {
    if (downloads.running) layout.notify(i18n.t("setup.busyOther"));
    return false;
  }
  if (!(await runtimes.select(item.id))) return false;
  await stopIfRunning();
  layout.notify(i18n.t("engine.changed"));
  emit("done", item.id);
  return true;
}

async function confirm(): Promise<void> {
  const item = selected.value;
  if (!item) return;

  if (item.installed) {
    const ok = await runtimes.select(item.id);
    if (!ok) {
      if (runtimes.error) layout.notify(runtimes.error);
      return;
    }
    await stopIfRunning();
    restartName.value = labelFor(item);
    close();
    emit("done", item.id);
    restartOpen.value = true;
    return;
  }

  // 没装：关掉向导，走下载队列（默认跳管理器看着下）。
  close();
  await installPack(item, true);
}

/** 「在后台下载」：不跳转，留在当前页面（设置页 / 写歌页）等它装完。 */
async function background(): Promise<void> {
  const item = selected.value;
  if (!item || item.installed) return;
  close();
  await installPack(item, false);
}

const restartBody = computed(() =>
  i18n.t("engine.restartBody", { name: restartName.value || i18n.t("engine.backend") }),
);

async function restartEngine(): Promise<void> {
  restartOpen.value = false;
  await yue.start(null);
  layout.notify(yue.error ? yue.error : i18n.t("engine.restartStarted"));
}

function restartLater(): void {
  restartOpen.value = false;
  layout.notify(i18n.t("engine.changed"));
}

/** 换后端不会热切：引擎在跑就停掉，下次生成时按新后端拉起。 */
async function stopIfRunning(): Promise<void> {
  if (yue.engine && (yue.engine.state === "ready" || yue.engine.state === "starting")) {
    await yue.stop();
  }
}
</script>

<template>
  <ModalShell
    :open="open"
    :title="i18n.t('yue.engineSetupTitle')"
    :dismissible="dismissible"
    width="440px"
    @update:open="emit('update:open', $event)"
  >
    <div class="m3-dialog-body setup">
      <p class="m3-hint">{{ i18n.t("yue.engineSetupLead") }}</p>

      <div v-if="packs.length" class="options" role="radiogroup" :aria-label="i18n.t('yue.engineSetupTitle')">
        <button
          v-for="item in packs"
          :key="item.id"
          type="button"
          class="option"
          :class="{ on: picked === item.id, off: !item.usable }"
          role="radio"
          :aria-checked="picked === item.id"
          :aria-disabled="!item.usable"
          :disabled="!item.usable"
          @click="picked = item.id"
        >
          <span class="radio" aria-hidden="true" />
          <span class="m3-row-main">
            <span class="m3-row-label">
              {{ labelFor(item) }}
              <span v-if="item.installed && item.selected" class="pill">{{ i18n.t("yue.engineCurrent") }}</span>
              <span v-else-if="item.installed" class="pill">{{ i18n.t("yue.engineInstalled") }}</span>
              <span v-else-if="item.usable && item.recommended" class="pill rec">
                {{ i18n.t("engine.recommended") }}
              </span>
            </span>
            <span class="m3-row-sub">
              {{ item.usable ? noteFor(item) : i18n.t("engine.notUsable") }}
            </span>
          </span>
          <span class="m3-row-value">{{ humanSize(item.bytes) }}</span>
        </button>
      </div>
      <p v-else-if="runtimes.loading" class="m3-hint">{{ i18n.t("common.loading") }}</p>
      <p v-else class="m3-hint">{{ i18n.t("yue.noEnginePacks") }}</p>
    </div>

    <template #foot>
      <div class="m3-dialog-foot">
        <button class="m3-btn text" type="button" @click="cancel">
          {{ i18n.t(dismissible ? "common.cancel" : "yue.engineSetupBack") }}
        </button>
        <button
          v-if="selected && !selected.installed"
          class="m3-btn text"
          type="button"
          @click="background"
        >
          {{ i18n.t("setup.background") }}
        </button>
        <span class="spacer" />
        <button class="m3-btn filled" type="button" :disabled="!canConfirm" @click="confirm">
          {{ confirmLabel }}
        </button>
      </div>
    </template>
  </ModalShell>

  <!-- 切换成功后：不替用户自动加载几 GB 权重，先问一句 -->
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
</template>

<style scoped>
.setup {
  gap: 10px;
}
.options {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.option {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 10px 12px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.45);
  border-radius: 16px;
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface));
  text-align: left;
  cursor: pointer;
  transition: border-color 0.14s ease, background-color 0.14s ease;
}
.option:hover {
  border-color: rgb(var(--md-primary) / 0.55);
}
.option:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.option.on {
  border-color: rgb(var(--md-primary));
  background-color: rgb(var(--md-primary) / 0.08);
}
/* 本机跑不了的后端（AMD 机器上的 CUDA）：灰掉但保留，说明「为什么不能用」。 */
.option.off {
  opacity: 0.45;
  cursor: default;
}
.option.off:hover {
  border-color: rgb(var(--md-outline-variant) / 0.45);
}
.radio {
  width: 18px;
  height: 18px;
  flex: 0 0 auto;
  border: 2px solid rgb(var(--md-outline));
  border-radius: 999px;
  transition: border-color 0.14s ease, background-color 0.14s ease;
}
.option.on .radio {
  border-color: rgb(var(--md-primary));
  background-color: rgb(var(--md-primary));
  box-shadow: inset 0 0 0 4px rgb(var(--md-surface-container-highest));
}
.pill {
  display: inline-flex;
  align-items: center;
  height: 18px;
  margin-left: 6px;
  padding: 0 7px;
  border-radius: 999px;
  background-color: rgb(var(--md-primary) / 0.14);
  color: rgb(var(--md-primary));
  font-size: 10px;
  font-weight: 500;
}
.pill.rec {
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}
.spacer {
  flex: 1 1 auto;
}
</style>
