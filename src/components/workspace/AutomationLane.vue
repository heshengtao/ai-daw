<script setup lang="ts">
/**
 * 自动化车道的**外壳**：编排视图底部的一条车道（对标钢琴卷帘下方的控制器泳道）。
 *
 * 编辑对象是**当前选中轨道**上的一条参数曲线，键取自共享的目标列表
 * （`gain_db` / `pan` / `inst:<id>` / `fx<i>:<id>`，见 `utils/automation.ts`）。曲线本身由
 * `AutomationCurve` 画（与钢琴卷帘下的参数车道共用）；这里只管表头：目标下拉、读数、清空、
 * 添加参数、收起，以及把曲线交回的编辑结果提交给工程（提交失败在这里提示）。
 *
 * 横向与编排视图共用同一条时间轴：坐标原点从编排视图的 `TIMELINE_INSET_PX` 起算并按共享滚动
 * 位置平移（由 `insetPx` 传进曲线），所以曲线的小节线严格压在编排网格的小节线上。
 */
import { computed, ref, watch } from "vue";
import { PlusIcon, XMarkIcon } from "@heroicons/vue/24/outline";
import Md3Select from "@/components/common/Md3Select.vue";
import AutomationCurve from "./AutomationCurve.vue";
import { useI18n } from "@/i18n";
import { useProjectStore } from "@/stores/project";
import { usePluginsStore } from "@/stores/plugins";
import { useSelectionStore } from "@/stores/selection";
import { useTransportStore } from "@/stores/transport";
import { useLayoutStore } from "@/stores/layout";
import { useParamBindingsStore } from "@/stores/paramBindings";
import { useParamBrowserStore } from "@/stores/paramBrowser";
import {
  automationTargetsFor,
  curateTargets,
  defaultBoundKeys,
  laneValueAtOrNull,
} from "@/utils/automation";
import type { AutomationPoint } from "@/api/types";

const props = defineProps<{
  /** 车道本体高度（px）。 */
  height: number;
  /** 画布左边缘相对第 0 拍的偏移（编排视图的 `TIMELINE_INSET_PX`）。 */
  insetPx: number;
}>();

const store = useProjectStore();
const plugins = usePluginsStore();
const selection = useSelectionStore();
const transport = useTransportStore();
const layout = useLayoutStore();
const bindings = useParamBindingsStore();
const browser = useParamBrowserStore();
const i18n = useI18n();

/** 当前选中的目标键（`Track.automation[].param` 里存的就是它）。 */
const param = ref<string>("gain_db");

const track = computed(() => selection.track);
const trackId = computed(() => track.value?.id ?? null);

/** 该轨道可自动化的**全部**目标（轨道混音 / 乐器 / 效果器）——下拉的取数面，全量口径。 */
const all = computed(() => automationTargetsFor(track.value, plugins.paramsOf, i18n.t));
/** 显式绑定的键；`null` = 没有记录 → 用默认精选（`[]` = 用户显式全解绑，不回落）。 */
const bound = computed(() => (trackId.value === null ? null : bindings.explicit(trackId.value)));
/** 默认绑定集（精选表拼上 `inst:` / `fx<i>:` 前缀）：没有显式记录时用它。 */
const fallback = computed(() => defaultBoundKeys(track.value));
/** 已经有曲线的键（已用但可能已解绑）——这些必须一直留在下拉里。 */
const used = computed(() =>
  (track.value?.automation ?? []).filter((l) => l.points.length > 0).map((l) => l.param),
);
/** 下拉里实际看到的收窄结果：增益 / 声像 + 已绑定 + 已有曲线。 */
const targets = computed(() =>
  curateTargets(all.value, {
    bound: bound.value,
    fallbackBound: fallback.value,
    usedKeys: used.value,
  }),
);
const chooserOptions = computed(() =>
  targets.value.map((t) => ({ id: t.key, label: t.label, group: t.group })),
);
/** 当前目标的取值范围与刻度；轨道没了就是 null。 */
const current = computed(() => targets.value.find((t) => t.key === param.value) ?? null);

/** 工程里该目标的车道（升序）。 */
const stored = computed<AutomationPoint[]>(() => {
  const lane = track.value?.automation?.find((l) => l.param === param.value);
  return lane?.points ?? [];
});

/** 播放头处的曲线值（有曲线时给一个即时读数）。 */
const valueAtPlayhead = computed(() =>
  laneValueAtOrNull(stored.value, transport.positionBeats),
);

/** 读数里的数值：小数位与单位取自当前目标（与曲线内部的格式一致）。 */
function formatValue(v: number): string {
  const spec = current.value;
  return spec ? `${v.toFixed(spec.digits)}${spec.unit ? ` ${spec.unit}` : ""}` : v.toFixed(2);
}

/** 曲线交回的编辑结果：一次拖拽 / 一次右键删 = 一条命令（提交失败在这里提示）。 */
async function applyPoints(points: AutomationPoint[]): Promise<void> {
  const id = trackId.value;
  if (id === null) return;
  try {
    await store.setAutomation(id, param.value, points);
  } catch (err) {
    layout.notify(i18n.t("error.setAutomation", { error: String(err) }));
  }
}

async function clearLane(): Promise<void> {
  const id = trackId.value;
  if (id === null) return;
  try {
    await store.setAutomation(id, param.value, []);
  } catch (err) {
    layout.notify(i18n.t("error.setAutomation", { error: String(err) }));
  }
}

function onChoose(id: string): void {
  param.value = id;
}

/** 表头的「＋ 添加参数…」：打开参数页，作用域是当前轨道（没有轨道就什么也不做）。 */
function onBrowse(): void {
  const id = trackId.value;
  if (id === null) return;
  browser.open({ trackId: id });
}

// 外部插件是懒加载的：进车道时确保**乐器与整条效果链**的参数表已在（`ensureParams` 对已缓存
// 的表是空操作）。外部效果器的参数表也要在，否则目标下拉里会缺项。
watch(
  () => [
    track.value?.instrument?.plugin_id,
    ...(track.value?.effects ?? []).map((fx) => fx.plugin_id),
  ],
  (ids) => {
    for (const id of ids) if (id) void plugins.ensureParams(id);
  },
  { immediate: true },
);

/** 选中的目标消失时（乐器 / 效果器没了、或被解绑且没曲线）回落到 gain_db。 */
watch(targets, (list) => {
  if (!list.some((t) => t.key === param.value)) param.value = "gain_db";
});
</script>

<template>
  <div class="auto-lane">
    <!-- 表头：目标选择 + 读数 + 清空 / 添加参数 / 收起 -->
    <div class="auto-head">
      <div class="auto-chooser">
        <Md3Select
          :model-value="param"
          :options="chooserOptions"
          :label="i18n.t('auto.param')"
          @select="onChoose"
        />
      </div>
      <span class="auto-name text-label-sm">{{ i18n.t("auto.title") }}</span>

      <!-- 曲线模式：表头渲染当前目标的说明（`hint`），没有 `hint` 时回落显示名——与卷帘下方参数车道对齐 -->
      <span v-if="current" class="auto-readout">{{ current.hint ?? current.label }}</span>

      <span v-if="!track" class="auto-readout muted">{{ i18n.t("auto.noTrack") }}</span>
      <span v-else-if="stored.length === 0" class="auto-readout muted">{{ i18n.t("auto.hint") }}</span>
      <span v-else class="auto-readout">
        {{ i18n.t("auto.points", { count: stored.length }) }}
        <template v-if="valueAtPlayhead !== null">
          · {{ i18n.t("auto.atPlayhead") }}
          <strong class="tabular">{{ formatValue(valueAtPlayhead) }}</strong>
        </template>
      </span>

      <span class="auto-spacer" />

      <button v-if="track && stored.length > 0" class="auto-btn" @click="clearLane">
        {{ i18n.t("auto.clear") }}
      </button>
      <!-- 「＋ 添加参数…」：常驻在表头，不藏进下拉（下拉可能被压出视口） -->
      <button
        class="auto-btn icon"
        type="button"
        :aria-label="i18n.t('auto.browse')"
        @click="onBrowse"
      >
        <PlusIcon class="auto-icon" />
      </button>
      <button
        class="auto-btn icon"
        type="button"
        :aria-label="i18n.t('auto.hide')"
        @click="layout.showAutomation = false"
      >
        <XMarkIcon class="auto-icon" />
      </button>
    </div>

    <!-- 车道本体：曲线画布（没有目标可选时曲线不渲染） -->
    <AutomationCurve
      v-if="current"
      :points="stored"
      :min="current.min"
      :max="current.max"
      :step="current.step"
      :digits="current.digits"
      :unit="current.unit"
      :height="props.height"
      :inset-px="props.insetPx"
      @apply="applyPoints"
    />
  </div>
</template>

<style scoped>
.auto-lane {
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  min-width: 0;
  background-color: var(--app-card);
  border-top: 1px solid var(--grid-line-strong);
  user-select: none;
  -webkit-user-select: none;
}
.auto-head {
  height: 30px;
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 10px;
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.35);
}
/* 参数下拉：压到与表头同高的小尺寸（`Md3Select` 默认是 34px 的宽药丸） */
.auto-chooser {
  flex: 0 0 auto;
  display: inline-flex;
  min-width: 0;
}
.auto-chooser :deep(.md3-select) {
  width: 128px;
  height: 22px;
  padding: 0 8px 0 10px;
  gap: 6px;
  font-size: 11px;
}
.auto-chooser :deep(.caret) {
  width: 13px;
  height: 13px;
}
.auto-name {
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
}
.auto-readout {
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.auto-readout.muted {
  color: rgb(var(--md-on-surface-muted));
}
.auto-readout strong {
  color: rgb(var(--md-on-surface));
  margin-left: 4px;
}
.auto-spacer {
  flex: 1 1 auto;
}
.auto-btn {
  flex: 0 0 auto;
  height: 22px;
  padding: 0 10px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 10.5px;
  cursor: pointer;
  white-space: nowrap;
  transition: background-color 0.14s ease, color 0.14s ease;
}
.auto-btn:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.auto-btn.icon {
  width: 22px;
  padding: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
.auto-icon {
  width: 14px;
  height: 14px;
}
/* 本体的样式在 `AutomationCurve` 里（曲线画布整块搬过去了）。 */
.tabular {
  font-variant-numeric: tabular-nums;
}
</style>
