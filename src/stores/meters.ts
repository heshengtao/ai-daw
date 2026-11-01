/**
 * 通道电平表：30 Hz 轮询 + 前端弹道。
 *
 * 后端的语义是「上次读取以来的峰值」——每次 `mixer_meters()` 取走并清零，所以轮询再慢也
 * 不会漏掉峰值，只是时间分辨率变粗。真正的表头观感（回落速度、峰值保持、削波锁存）都在这里：
 * 后端不理解「表」，它只报数。
 *
 * 为什么是轮询而不是事件推送：电平是**高频、可丢**的数据，走事件会把 IPC 通道淹掉；
 * 30 Hz 的小载荷（每轨 20 字节）比一条推送管线便宜得多。页面不可见时停表。
 */
import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { clearMeterClip, mixerMeters } from "@/api/tauri";
import type { MeterValue } from "@/api/types";

/** 一轮轮询的间隔（毫秒）：30 Hz 足够看出「跳」，又不会把主线程拖慢。 */
const POLL_MS = 33;
/** 峰值回落速度（dB/秒）。 */
const FALL_DB_PER_SEC = 20;
/** 峰值保持时长（毫秒）。 */
const HOLD_MS = 1_200;

/** 一条通道的显示状态（全部是**线性**峰值，渲染时再换算 dB）。 */
export interface ChannelLevel {
  /** 当前电平（左 / 右）。 */
  l: number;
  r: number;
  /** 峰值保持（左 / 右）。 */
  holdL: number;
  holdR: number;
  /** 峰值保持的到期时间（`performance.now()` 毫秒）。 */
  holdUntil: number;
  /** 增益衰减（dB，正数 = 压下去多少）。 */
  gr: number;
  /** 削波锁存（点表头清除）。 */
  clipped: boolean;
}

function emptyLevel(): ChannelLevel {
  return { l: 0, r: 0, holdL: 0, holdR: 0, holdUntil: 0, gr: 0, clipped: false };
}

/** 每轮询周期的回落系数（20 dB/s ≈ 0.926 / 33 ms）。 */
const FALL = Math.pow(10, (-FALL_DB_PER_SEC * (POLL_MS / 1000)) / 20);

function step(prev: ChannelLevel, value: MeterValue, now: number): ChannelLevel {
  const next: ChannelLevel = {
    l: Math.max(value.peak_l, prev.l * FALL),
    r: Math.max(value.peak_r, prev.r * FALL),
    holdL: prev.holdL,
    holdR: prev.holdR,
    holdUntil: prev.holdUntil,
    gr: value.gr_db,
    clipped: prev.clipped || value.clipped,
  };
  // 峰值保持：新峰值顶上去并续期，到期后跟着回落。
  if (value.peak_l >= next.holdL || now >= prev.holdUntil) {
    next.holdL = Math.max(value.peak_l, next.l);
    next.holdUntil = now + HOLD_MS;
  }
  if (value.peak_r >= next.holdR || now >= prev.holdUntil) {
    next.holdR = Math.max(value.peak_r, next.r);
  }
  if (next.holdL < next.l) next.holdL = next.l;
  if (next.holdR < next.r) next.holdR = next.r;
  if (next.holdL < 1e-5) next.holdL = 0;
  if (next.holdR < 1e-5) next.holdR = 0;
  return next;
}

export const useMetersStore = defineStore("meters", () => {
  const levels = ref<Record<number, ChannelLevel>>({});
  const master = ref<ChannelLevel>(emptyLevel());
  const polling = ref(false);

  /** 正在看表的组件数：>0 才轮询（两个面板同时开着也只跑一条定时器）。 */
  let subscribers = 0;
  let timer: number | null = null;

  /** 某条轨道的表；没数据时返回一张空表（模板里不必判空）。 */
  function track(id: number): ChannelLevel {
    return levels.value[id] ?? emptyLevel();
  }

  async function poll(): Promise<void> {
    let snapshot;
    try {
      snapshot = await mixerMeters();
    } catch {
      return;
    }
    const now = performance.now();
    const next: Record<number, ChannelLevel> = {};
    for (const entry of snapshot.tracks) {
      next[entry.track] = step(levels.value[entry.track] ?? emptyLevel(), entry.value, now);
    }
    levels.value = next;
    master.value = step(master.value, snapshot.master, now);
  }

  /** 开始轮询（引用计数）。 */
  function start(): void {
    subscribers += 1;
    if (timer !== null) return;
    polling.value = true;
    void poll();
    timer = window.setInterval(() => {
      if (document.visibilityState === "hidden") return;
      void poll();
    }, POLL_MS);
  }

  /** 停止轮询（引用计数归零时真的停）。 */
  function stop(): void {
    subscribers = Math.max(0, subscribers - 1);
    if (subscribers > 0 || timer === null) return;
    window.clearInterval(timer);
    timer = null;
    polling.value = false;
  }

  /** 清掉削波锁存（点表头）。 */
  async function clearClip(): Promise<void> {
    master.value = { ...master.value, clipped: false };
    const next: Record<number, ChannelLevel> = {};
    for (const [id, level] of Object.entries(levels.value)) {
      next[Number(id)] = { ...level, clipped: false };
    }
    levels.value = next;
    try {
      await clearMeterClip();
    } catch {
      // 清不掉也不影响界面（下一轮会把后端的锁存重新带回来）。
    }
  }

  /** 主输出的表。 */
  const masterLevel = computed(() => master.value);

  return { levels, master, masterLevel, polling, track, start, stop, clearClip };
});
