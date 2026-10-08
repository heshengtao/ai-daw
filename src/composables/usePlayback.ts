/**
 * 试听播放：离线渲染 → Blob URL → HTMLAudioElement。
 * 三条终态路径都必须回收对象 URL（停止、自然结束、播放失败）。
 */
import { onUnmounted, ref } from "vue";
import { renderPreview } from "@/api/tauri";

export interface UsePlaybackOptions {
  /** 渲染/播放失败时的回调（用于把错误呈现给用户）。 */
  onError?: (message: string, err: unknown) => void;
}

export function usePlayback(options: UsePlaybackOptions = {}) {
  const playing = ref(false);
  let audio: HTMLAudioElement | null = null;
  let url: string | null = null;
  let token = 0;

  function stop(): void {
    token++; // 让进行中的渲染失效
    playing.value = false;
    if (audio) {
      audio.onended = null;
      audio.pause();
      audio = null;
    }
    if (url) {
      URL.revokeObjectURL(url);
      url = null;
    }
  }

  async function play(sampleRate = 44100): Promise<void> {
    if (playing.value) return;
    const my = ++token;
    try {
      const blob = await renderPreview(sampleRate);
      if (my !== token) return; // 期间被停止
      url = URL.createObjectURL(blob);
      const el = new Audio(url);
      audio = el;
      el.onended = stop;
      await el.play();
      playing.value = true;
    } catch (err) {
      stop();
      options.onError?.("试听失败", err);
    }
  }

  onUnmounted(stop);

  return { playing, play, stop };
}
