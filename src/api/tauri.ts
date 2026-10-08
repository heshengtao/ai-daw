/**
 * Tauri 后端命令的类型化封装（与 `src-tauri/src/commands.rs` 一一对应）。
 * 命令名、参数名（camelCase，Tauri 会映射到 Rust 的 snake_case）必须与后端一致。
 *
 * 在浏览器里（非 Tauri）开发时回退到 `mock.ts`，方便无后端预览界面。
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { mockApi, mockEnabled } from "./mock";
import type { Project } from "./types";

/** 引擎事件广播通道名，与后端 `state::EVENT_ENGINE` 保持一致。 */
export const EVENT_ENGINE = "engine://event";

/** 读取当前工程。 */
export function getProject(): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.getProject());
  return invoke<Project>("get_project");
}

/** 新增一条轨道。 */
export function addTrack(name: string): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.addTrack(name));
  return invoke<Project>("add_track", { name });
}

/** 设置速度（BPM）。 */
export function setTempo(bpm: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.setTempo(bpm));
  return invoke<Project>("set_tempo", { bpm });
}

/** 清空为新工程。 */
export function clearProject(): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.clearProject());
  return invoke<Project>("clear_project");
}

/** 载入内置示例工程。 */
export function loadDemo(): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.loadDemo());
  return invoke<Project>("load_demo");
}

/** 离线渲染当前工程，返回可播放的 WAV Blob。 */
export async function renderPreview(sampleRate = 44100): Promise<Blob> {
  if (mockEnabled()) return mockApi.renderPreview();
  const bytes = await invoke<ArrayBuffer>("render_preview", { sampleRate });
  return new Blob([bytes], { type: "audio/wav" });
}

/** 将当前工程导出为 WAV 文件，返回写入路径。 */
export function exportWav(path: string, sampleRate = 44100): Promise<string> {
  if (mockEnabled()) return Promise.resolve(mockApi.exportWav(path));
  return invoke<string>("export_wav", { path, sampleRate });
}

/** 订阅引擎事件流（原始 payload，仅供开发者抽屉消费）。 */
export function onEngineEvent(handler: (payload: unknown) => void): Promise<UnlistenFn> {
  if (mockEnabled()) {
    const onMock = (): void => handler({ TrackAdded: { actor: "Human", id: 4, name: "Track 4" } });
    window.addEventListener("daw-party-mock-event", onMock);
    const off = mockApi.onEngineEvent();
    return Promise.resolve(() => {
      window.removeEventListener("daw-party-mock-event", onMock);
      off();
    });
  }
  return listen(EVENT_ENGINE, (event) => handler(event.payload));
}
