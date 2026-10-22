/**
 * 操作记录：本次编辑会话里人 / 内置助手 / 外部 AI 的每一步改动。
 *
 * 数据完全来自后端的撤销历史（每条记录与一个撤销批次一一对应），这里只负责拉取、
 * 回撤 / 重做到指定操作，并把返回的工程快照交给 project store。
 */
import { defineStore } from "pinia";
import { ref } from "vue";
import * as api from "@/api/tauri";
import { useProjectStore } from "@/stores/project";
import type { OpRecord } from "@/api/types";

export const useOpLogStore = defineStore("opLog", () => {
  const entries = ref<OpRecord[]>([]);
  const loading = ref(false);
  /** 正在执行回撤 / 重做：防止连点导致多次重放。 */
  const busy = ref(false);
  const error = ref<string | null>(null);

  /** 重新拉取操作记录。 */
  async function load(): Promise<void> {
    loading.value = true;
    try {
      entries.value = await api.listOperations();
      error.value = null;
    } catch (err) {
      error.value = String(err);
    } finally {
      loading.value = false;
    }
  }

  /** 回撤到某条操作之前（该操作及其之后的改动都会撤销）。 */
  async function revertTo(id: number): Promise<void> {
    if (busy.value) return;
    busy.value = true;
    try {
      useProjectStore().applyHistoryState(await api.revertToOperation(id));
      await load();
    } catch (err) {
      error.value = String(err);
    } finally {
      busy.value = false;
    }
  }

  /** 重做到某条被撤销的操作（包含它本身）。 */
  async function reapplyTo(id: number): Promise<void> {
    if (busy.value) return;
    busy.value = true;
    try {
      useProjectStore().applyHistoryState(await api.reapplyToOperation(id));
      await load();
    } catch (err) {
      error.value = String(err);
    } finally {
      busy.value = false;
    }
  }

  return { entries, loading, busy, error, load, revertTo, reapplyTo };
});
