import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Project } from "./types";

/** 引擎事件广播名，与后端 `state::EVENT_ENGINE` 保持一致。 */
export const EVENT_ENGINE = "engine://event";

/** 读取当前工程。 */
export function getProject(): Promise<Project> {
  return invoke<Project>("get_project");
}
/** 新增轨道。 */
export function addTrack(name: string): Promise<Project> {
  return invoke<Project>("add_track", { name });
}
/** 设置速度（BPM）。 */
export function setTempo(bpm: number): Promise<Project> {
  return invoke<Project>("set_tempo", { bpm });
}
/** 清空工程。 */
export function clearProject(): Promise<Project> {
  return invoke<Project>("clear_project");
}
/** 载入示例工程。 */
export function loadDemo(): Promise<Project> {
  return invoke<Project>("load_demo");
}
/** 订阅引擎事件流。 */
export function onEngineEvent(handler: (event: unknown) => void): Promise<UnlistenFn> {
  return listen(EVENT_ENGINE, (e) => handler(e.payload));
}
