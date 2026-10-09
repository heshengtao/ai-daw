/**
 * 内存剪贴板：只服务「复制/粘贴音频块与音符」这一件事，不落盘、不进撤销历史。
 * v1 只承载单条内容（单个片段 / 单个音符）。
 */
import { defineStore } from "pinia";
import { ref } from "vue";
import type { Clip, Note } from "@/api/types";

export type ClipboardPayload =
  | { kind: "clip"; notes: Note[]; length: number }
  | { kind: "note"; note: Note };

export const useClipboardStore = defineStore("clipboard", () => {
  const payload = ref<ClipboardPayload | null>(null);

  function copyClip(clip: Clip): void {
    payload.value = { kind: "clip", notes: clip.notes.map((n) => ({ ...n })), length: clip.length };
  }

  function copyNote(note: Note): void {
    payload.value = { kind: "note", note: { ...note } };
  }

  function clear(): void {
    payload.value = null;
  }

  return { payload, copyClip, copyNote, clear };
});
