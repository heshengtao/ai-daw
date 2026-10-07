import { M3eButton } from "@m3e/react/button";

/** `TransportBar` 的 props：当前速度、改速回调与播放/停止控制。 */
export interface TransportBarProps {
  /** 当前 BPM。 */
  tempo: number;
  /** 用户改速时回调，传入新的 BPM。 */
  onTempoChange: (bpm: number) => void;
  /** 开始试听：离线渲染当前工程并播放。 */
  onPlay: () => void;
  /** 停止试听并释放播放资源。 */
  onStop: () => void;
  /** 是否处于播放态，决定按钮显示播放还是停止。 */
  playing: boolean;
}

/** 传输条：播放/停止按钮（离线渲染试听）与 BPM 数字输入。 */
export function TransportBar({ tempo, onTempoChange, onPlay, onStop, playing }: TransportBarProps) {
  return (
    <section className="transport-bar">
      <M3eButton onClick={playing ? onStop : onPlay}>{playing ? "Stop" : "Play"}</M3eButton>
      <label>
        BPM
        <input
          type="number" min={20} max={300} value={tempo}
          onChange={(e) => onTempoChange(Number(e.target.value))}
        />
      </label>
    </section>
  );
}
