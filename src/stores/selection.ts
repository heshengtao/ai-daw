/**
 * 编辑选中态：当前轨道 / 片段 / 音符，支持多选。
 *
 * 三个视图（音轨栏、编排、钢琴卷帘）与检查器共用同一份选中态，因此「在音轨栏点一条
 * 轨道 → 检查器显示它、钢琴卷帘编辑它」是天然一致的。
 *
 * 选中态**不**复制工程数据：全部通过 id/索引在工程里查，工程变化后自动裁剪失效项，
 * 不会出现指向已删除对象的悬垂引用。
 *
 * 两类多选：
 * - `clipRefs`：编排视图里框选的片段（可跨轨道）；
 * - `noteIndices`：钢琴卷帘里框选的音符（片段内序号，升序；**末位是主选**，
 *   属性栏在「多个音符值不一致」时以主选为准显示）。
 */
import { defineStore } from "pinia";
import { computed, ref, watch } from "vue";
import type { Clip, Note, Track } from "@/api/types";
import { useProjectStore } from "@/stores/project";

/** 片段选择项（轨道 + 片段）。 */
export interface ClipSel {
  track: number;
  clip: number;
}

export const useSelectionStore = defineStore("selection", () => {
  const project = useProjectStore();

  const trackId = ref<number | null>(null);
  const clipId = ref<number | null>(null);
  const clipRefs = ref<ClipSel[]>([]);
  const noteIndices = ref<number[]>([]);

  /** 当前选中的轨道（不存在时为 null）。 */
  const track = computed<Track | null>(() => {
    if (trackId.value === null) return null;
    return project.project?.tracks.find((t) => t.id === trackId.value) ?? null;
  });

  /** 当前选中的片段（钢琴卷帘的编辑对象）。 */
  const clip = computed<Clip | null>(() => {
    const t = track.value;
    if (!t || clipId.value === null) return null;
    return t.clips.find((c) => c.id === clipId.value) ?? null;
  });

  /** 主选音符在片段内的序号（多选时为最后点选的那个）。 */
  const primaryNoteIndex = computed(() => noteIndices.value.at(-1) ?? null);

  /** 主选音符。 */
  const note = computed<Note | null>(() => {
    const c = clip.value;
    const i = primaryNoteIndex.value;
    if (!c || i === null || i < 0 || i >= c.notes.length) return null;
    return c.notes[i];
  });

  /** 已选音符（按序号升序）。 */
  const selectedNotes = computed<Note[]>(() => {
    const c = clip.value;
    if (!c) return [];
    return noteIndices.value
      .filter((i) => i >= 0 && i < c.notes.length)
      .map((i) => c.notes[i]);
  });

  /** 已选片段（编排视图）。 */
  const selectedClips = computed<ClipSel[]>(() => clipRefs.value);

  // ---- 轨道 ----

  /** 选中一条轨道。轨道变化时丢弃不再属于它的片段/音符选择。 */
  function selectTrack(id: number | null): void {
    trackId.value = id;
    if (id === null) {
      clipId.value = null;
      noteIndices.value = [];
      clipRefs.value = [];
      return;
    }
    const t = project.project?.tracks.find((x) => x.id === id);
    const stillValid = t?.clips.some((c) => c.id === clipId.value) ?? false;
    if (!stillValid) {
      clipId.value = null;
      noteIndices.value = [];
    }
    // 框选集合收敛到该轨道（除非其中仍有属于本轨道的项）。
    if (!clipRefs.value.some((r) => r.track === id)) clipRefs.value = [];
  }

  // ---- 片段 ----

  /** 选中一个片段（同时切换所属轨道，并把框选集合收敛为它）。 */
  function selectClip(trackIdValue: number, clipIdValue: number | null): void {
    trackId.value = trackIdValue;
    clipId.value = clipIdValue;
    noteIndices.value = [];
    clipRefs.value = clipIdValue === null ? [] : [{ track: trackIdValue, clip: clipIdValue }];
  }

  /** 用一组片段替换框选集合（框选结束后调用）。 */
  function setClipSelection(refs: ClipSel[]): void {
    clipRefs.value = refs;
    const first = refs[0];
    if (first) {
      trackId.value = first.track;
      clipId.value = first.clip;
    }
    noteIndices.value = [];
  }

  /** 切换单个片段的选中状态（Ctrl/Cmd + 点击）。 */
  function toggleClip(trackIdValue: number, clipIdValue: number): void {
    const exists = clipRefs.value.some((r) => r.track === trackIdValue && r.clip === clipIdValue);
    if (exists) {
      clipRefs.value = clipRefs.value.filter((r) => !(r.track === trackIdValue && r.clip === clipIdValue));
      return;
    }
    clipRefs.value = [...clipRefs.value, { track: trackIdValue, clip: clipIdValue }];
    trackId.value = trackIdValue;
    clipId.value = clipIdValue;
  }

  /** 某片段是否在框选集合里。 */
  function isClipSelected(trackIdValue: number, clipIdValue: number): boolean {
    return clipRefs.value.some((r) => r.track === trackIdValue && r.clip === clipIdValue);
  }

  // ---- 音符 ----

  /** 用一组序号替换音符选择（框选结束后调用）。 */
  function setNoteSelection(indices: number[]): void {
    noteIndices.value = [...new Set(indices)].sort((a, b) => a - b);
  }

  /** 单选一个音符。 */
  function selectNote(index: number | null): void {
    noteIndices.value = index === null ? [] : [index];
  }

  /** 切换单个音符的选中状态。 */
  function toggleNote(index: number): void {
    const exists = noteIndices.value.includes(index);
    noteIndices.value = exists
      ? noteIndices.value.filter((i) => i !== index)
      : [...noteIndices.value, index].sort((a, b) => a - b);
  }

  /** 某音符是否被选中。 */
  function isNoteSelected(index: number): boolean {
    return noteIndices.value.includes(index);
  }

  /** 清空音符选择（结构变化导致序号位移后必须调用）。 */
  function clearNotes(): void {
    noteIndices.value = [];
  }

  // ---- 记忆长度（单击空白快速创建时复用） ----

  /** 上一次选中的片段长度（拍）：单击空白新建片段时用它。 */
  const lastClipLength = ref(1);
  /** 上一次选中的音符时长（拍）：单击空白新建音符时用它。 */
  const lastNoteLength = ref(0.5);

  function rememberClipLength(n: number): void {
    if (Number.isFinite(n) && n > 0) lastClipLength.value = n;
  }

  function rememberNoteLength(n: number): void {
    if (Number.isFinite(n) && n > 0) lastNoteLength.value = n;
  }

  // ---- 通用 ----

  /**
   * 保证「有轨道且有片段」可选。
   * 首次载入工程、或选中对象被删除后调用；钢琴卷帘与检查器都依赖它。
   */
  function ensure(): void {
    const tracks = project.project?.tracks ?? [];
    if (tracks.length === 0) {
      clear();
      return;
    }
    if (!track.value) {
      selectTrack(tracks[0].id);
    }
    const t = track.value;
    if (t && !clip.value && t.clips.length > 0) {
      clipId.value = t.clips[0].id;
      noteIndices.value = [];
    }
  }

  /** 清空全部选择。 */
  function clear(): void {
    trackId.value = null;
    clipId.value = null;
    clipRefs.value = [];
    noteIndices.value = [];
  }

  // 工程每次变化（编辑、撤销、导入）后都校验一次：
  // - 已删除的轨道/片段从选择里剔除；
  // - 音符序号越界时清空（序号会因删除而位移，宁可清空也不能指向错误对象）；
  // - 选择为空时自动落到第一个可编辑对象，避免检查器/钢琴卷帘停在空状态。
  watch(
    () => project.project,
    () => {
      const tracks = project.project?.tracks ?? [];
      clipRefs.value = clipRefs.value.filter((r) =>
        tracks.some((t) => t.id === r.track && t.clips.some((c) => c.id === r.clip)),
      );
      const c = clip.value;
      if (noteIndices.value.length > 0 && (!c || noteIndices.value.some((i) => i >= c.notes.length))) {
        noteIndices.value = [];
      }
      ensure();
    },
    { immediate: true, deep: false },
  );

  return {
    trackId,
    clipId,
    clipRefs,
    noteIndices,
    track,
    clip,
    note,
    primaryNoteIndex,
    selectedNotes,
    selectedClips,
    selectTrack,
    selectClip,
    setClipSelection,
    toggleClip,
    isClipSelected,
    setNoteSelection,
    selectNote,
    toggleNote,
    isNoteSelected,
    clearNotes,
    lastClipLength,
    lastNoteLength,
    rememberClipLength,
    rememberNoteLength,
    ensure,
    clear,
  };
});
