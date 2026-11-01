import { computed, ref, watch } from "vue";
import type { ComputedRef } from "vue";
import { voiceList, voiceSave } from "@/api/tauri";
import type { VoiceEntry } from "@/api/types";
import { pickAudioToOpen } from "@/api/dialogs";
import { useI18n } from "@/i18n";
import { useLayoutStore } from "@/stores/layout";
import { useSoundMemoriesStore } from "@/stores/soundMemories";

/** 能当参考声线的轨道（只要有 `id` 与 `name` 就够了）。 */
export interface ToneTrack {
  id: number;
  name: string;
}

/**
 * 「参考声线」选择器：**歌声合成**与**音色替换**两张卡片/两个页面共用同一套行为。
 *
 * 一路是工程内的音频轨（`track:<id>`），一路是人声音色库（`voice:<id>`）——两路互斥，
 * 所以候选列表跟着「来源」走，而不是把两种 id 混在同一个下拉里。这里只管「选哪条声线」，
 * 目标轨道（要唱的 MIDI / 要换的人声）由调用方自己管。
 *
 * @param audioTracks 可当参考的轨道（调用方从工程里筛）
 * @param busy        调用方是否正在推理（推理中冻结全部交互）
 */
export function useTonePicker(audioTracks: ComputedRef<ToneTrack[]>, busy: () => boolean) {
  const i18n = useI18n();
  const layout = useLayoutStore();
  const memories = useSoundMemoriesStore();

  /** 人声音色库（跨工程）。 */
  const voices = ref<VoiceEntry[]>([]);
  /** 参考声线的来源：工程内轨道 / 人声音色库。 */
  const source = ref<"track" | "voice">("track");
  /** 选中的那条声线：`track:<id>` / `voice:<id>`。 */
  const tone = ref("");
  /** 正在导入 / 保存人声（解码 + 挑段，几秒的活；别让按钮看起来没反应）。 */
  const importing = ref(false);
  /** 存进音色库的内联命名框。 */
  const saveOpen = ref(false);
  const draftName = ref("");
  /** 待保存的工程内轨道。 */
  const savingTrackId = ref<number | null>(null);

  const sourceOptions = computed(() => [
    { id: "track", label: i18n.t("ai.refSourceTrack") },
    { id: "voice", label: i18n.t("ai.refSourceVoice") },
  ]);

  /** 换来源时立刻落到该来源的第一条，避免下拉显示空值。 */
  const sourceChoice = computed({
    get: () => source.value,
    set: (v: string) => {
      source.value = v === "voice" ? "voice" : "track";
      tone.value =
        source.value === "voice"
          ? (voices.value[0] ? `voice:${voices.value[0].id}` : "")
          : (audioTracks.value[0] ? `track:${audioTracks.value[0].id}` : "");
    },
  });

  /** 当前来源下可选的参考（轨道 / 音色库）。 */
  const toneOptions = computed(() =>
    source.value === "voice"
      ? voices.value.map((v) => ({
          id: `voice:${v.id}`,
          label: v.name,
          hint: `${Math.round(v.seconds)}s${v.description ? ` · ${v.description}` : ""}`,
        }))
      : audioTracks.value.map((t) => ({ id: `track:${t.id}`, label: t.name })),
  );

  const toneVoiceId = computed(() =>
    tone.value.startsWith("voice:") ? tone.value.slice("voice:".length) : null,
  );
  const toneTrackId = computed(() =>
    tone.value.startsWith("track:") ? Number(tone.value.slice("track:".length)) : null,
  );
  /** 当前选中的音色库条目。 */
  const toneVoice = computed(() => voices.value.find((v) => v.id === toneVoiceId.value) ?? null);
  /** 有可用的参考声线（选中的那条还在）。 */
  const hasTone = computed(() => toneOptions.value.some((o) => o.id === tone.value));

  async function refresh(): Promise<void> {
    try {
      voices.value = await voiceList();
    } catch {
      voices.value = [];
    }
  }

  /** 选中某条人声音色（刚存进音色库 / 刚导入完）。 */
  function pickVoice(id: string): void {
    source.value = "voice";
    tone.value = `voice:${id}`;
  }

  /** 把当前选中的工程内轨道存成音色（默认用轨道名）。 */
  /** 取消这次保存（关掉命名框）。 */
  function cancelSave(): void {
    saveOpen.value = false;
  }

  function beginSave(): void {
    if (source.value !== "track") return;
    const track = audioTracks.value.find((t) => t.id === toneTrackId.value);
    savingTrackId.value = track?.id ?? null;
    draftName.value = track?.name ?? i18n.t("ai.voiceDefaultName");
    saveOpen.value = true;
  }

  async function confirmSave(): Promise<void> {
    const name = draftName.value.trim();
    if (!name || savingTrackId.value === null || importing.value) return;
    importing.value = true;
    try {
      const entry = await voiceSave({ track: savingTrackId.value, path: null, name });
      await refresh();
      pickVoice(entry.id);
      saveOpen.value = false;
      layout.notify(i18n.t("ai.voiceSaved", { name: entry.name }));
    } catch (err) {
      layout.notify(String(err));
    } finally {
      importing.value = false;
    }
  }

  /** 从音频文件直接存一条人声音色（与「存入音色库」互补：一个从文件、一个从工程轨道）。 */
  async function importFile(): Promise<void> {
    if (busy() || importing.value) return;
    try {
      const path = await pickAudioToOpen();
      if (!path) return;
      const stem = path.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, "") ?? "";
      importing.value = true;
      const entry = await voiceSave({ path, name: stem || i18n.t("ai.voiceDefaultName") });
      await refresh();
      pickVoice(entry.id);
      layout.notify(i18n.t("ai.voiceSaved", { name: entry.name }));
    } catch (err) {
      layout.notify(String(err));
    } finally {
      importing.value = false;
    }
  }

  /** 打开侧边栏的「音色库」：人声和乐器在同一个库里，只靠标签区分。 */
  function openLibrary(): void {
    memories.kind = "voice";
    layout.openTab("sounds");
  }

  // 候选变化后把默认值收敛到合法值；所选来源空了就自动换边。
  watch(
    () =>
      [audioTracks.value.map((t) => t.id).join(","), voices.value.map((v) => v.id).join(",")].join("|"),
    () => {
      if (source.value === "voice" && voices.value.length === 0 && audioTracks.value.length > 0) {
        source.value = "track";
      } else if (
        source.value === "track" &&
        audioTracks.value.length === 0 &&
        voices.value.length > 0
      ) {
        source.value = "voice";
      }
      if (!hasTone.value) tone.value = toneOptions.value[0]?.id ?? "";
    },
    { immediate: true },
  );

  return {
    voices,
    source,
    tone,
    sourceOptions,
    sourceChoice,
    toneOptions,
    toneVoice,
    toneVoiceId,
    toneTrackId,
    hasTone,
    importing,
    saveOpen,
    draftName,
    refresh,
    pickVoice,
    beginSave,
    confirmSave,
    cancelSave,
    importFile,
    openLibrary,
  };
}
