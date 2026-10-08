/** 工程状态与命令封装：所有 project 变更都来自命令返回的快照。 */
import { defineStore } from "pinia";
import { ref } from "vue";
import * as api from "@/api/tauri";
import type { Project } from "@/api/types";

export const useProjectStore = defineStore("project", () => {
  const project = ref<Project | null>(null);
  const loading = ref(false);
  const loadError = ref<string | null>(null);

  /** 重新拉取工程快照。 */
  async function reload(): Promise<void> {
    loading.value = true;
    loadError.value = null;
    try {
      project.value = await api.getProject();
    } catch (err) {
      loadError.value = String(err);
    } finally {
      loading.value = false;
    }
  }

  /** 新增轨道。 */
  async function addTrack(name: string): Promise<void> {
    project.value = await api.addTrack(name);
  }

  /** 设置速度；非法值（空/NaN/越界）直接忽略，绝不把 0 发给引擎。 */
  async function setTempo(bpm: number): Promise<void> {
    if (!Number.isFinite(bpm) || bpm < 20 || bpm > 300) return;
    project.value = await api.setTempo(bpm);
  }

  /** 清空为新工程。 */
  async function clearAll(): Promise<void> {
    project.value = await api.clearProject();
  }

  /** 载入示例。 */
  async function loadDemo(): Promise<void> {
    project.value = await api.loadDemo();
  }

  return { project, loading, loadError, reload, addTrack, setTempo, clearAll, loadDemo };
});
