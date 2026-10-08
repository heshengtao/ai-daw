/** 工程状态与命令封装：所有 project 变更都来自命令返回的快照。 */
import { defineStore } from "pinia";
import { ref } from "vue";
import * as api from "@/api/tauri";
import type { HistoryState, Project } from "@/api/types";

export const useProjectStore = defineStore("project", () => {
  const project = ref<Project | null>(null);
  const loading = ref(false);
  const loadError = ref<string | null>(null);
  const undoDepth = ref(0);
  const redoDepth = ref(0);
  /** 用户是否已主动开始（新建或导入）。首屏引导只在尚未开始时出现。 */
  const started = ref(false);

  /** 重新查询历史深度（撤销/重做按钮的可用性）。 */
  async function refreshDepths(): Promise<void> {
    try {
      const d = await api.historyDepths();
      undoDepth.value = d.undo;
      redoDepth.value = d.redo;
    } catch {
      // 深度只影响按钮状态，查询失败不值得打断用户。
    }
  }

  function applyHistory(state: HistoryState): void {
    project.value = state.project;
    undoDepth.value = state.undo_depth;
    redoDepth.value = state.redo_depth;
  }

  /** 重新拉取工程快照。 */
  async function reload(): Promise<void> {
    loading.value = true;
    loadError.value = null;
    try {
      project.value = await api.getProject();
      await refreshDepths();
    } catch (err) {
      loadError.value = String(err);
    } finally {
      loading.value = false;
    }
  }

  /** 新增轨道。 */
  async function addTrack(name: string): Promise<void> {
    project.value = await api.addTrack(name);
    await refreshDepths();
  }

  /** 设置速度；非法值（空/NaN/越界）直接忽略，绝不把 0 发给引擎。 */
  async function setTempo(bpm: number): Promise<void> {
    if (!Number.isFinite(bpm) || bpm < 20 || bpm > 300) return;
    project.value = await api.setTempo(bpm);
    await refreshDepths();
  }

  /** 新建空工程。 */
  async function newProject(): Promise<void> {
    project.value = await api.clearProject();
    started.value = true;
    await refreshDepths();
  }

  /** 从文件导入工程（会清空历史）。 */
  async function openFrom(path: string): Promise<void> {
    project.value = await api.openProject(path);
    started.value = true;
    await refreshDepths();
  }

  /** 把当前工程保存到文件，返回写入路径。 */
  function saveTo(path: string): Promise<string> {
    return api.saveProject(path);
  }

  /** 撤销；没有可撤销内容时返回 false。 */
  async function undo(): Promise<boolean> {
    if (undoDepth.value === 0) return false;
    applyHistory(await api.undo());
    return true;
  }

  /** 重做；没有可重做内容时返回 false。 */
  async function redo(): Promise<boolean> {
    if (redoDepth.value === 0) return false;
    applyHistory(await api.redo());
    return true;
  }

  return {
    project,
    loading,
    loadError,
    undoDepth,
    redoDepth,
    started,
    reload,
    addTrack,
    setTempo,
    newProject,
    openFrom,
    saveTo,
    undo,
    redo,
    refreshDepths,
  };
});
