/**
 * 内存剪贴板：只服务「复制/粘贴音频块与音符」这一件事，不落盘、不进撤销历史。
 * v1 只承载单条内容（单个片段 / 单个音符）。
 *
 * 片段要连**音频引用**一起带走：音频片段的内容不在 `notes` 里，而在 `audio` 上，
 * 漏了它粘出来就是个等长的空块（曾经就是这样）。
 */
import { defineStore } from "pinia";
import { ref } from "vue";
import type { AudioSource, Clip, Note } from "@/api/types";

export type ClipboardPayload =
  | { kind: "clip"; notes: Note[]; length: number; audio?: AudioSource }
  | { kind: "note"; note: Note };

export const useClipboardStore = defineStore("clipboard", () => {
  const payload = ref<ClipboardPayload | null>(null);

  function copyClip(clip: Clip): void {
    payload.value = {
      kind: "clip",
      notes: clip.notes.map((n) => ({ ...n })),
      length: clip.length,
      // 有音频就一并带上（`audio` 只在音频片段上存在）。
      ...(clip.audio ? { audio: { ...clip.audio } } : {}),
    };
  }

  function copyNote(note: Note): void {
    payload.value = { kind: "note", note: { ...note } };
  }

  function clear(): void {
    payload.value = null;
  }

  return { payload, copyClip, copyNote, clear };
});
