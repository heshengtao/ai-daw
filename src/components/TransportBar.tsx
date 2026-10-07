import { M3eButton } from "@m3e/react/button";

/** `TransportBar` 的 props：当前速度、改速回调与播放/停止占位。 */
export interface TransportBarProps {
  /** 当前 BPM。 */
  tempo: number;
  /** 用户改速时回调，传入新的 BPM。 */
  onTempoChange: (bpm: number) => void;
  /** 播放（Task 5 接上行为）。 */
  onPlay: () => void;
  /** 停止（Task 5 接上行为）。 */
  onStop: () => void;
  /** 是否处于播放态，决定按钮显示播放还是停止。 */
  playing: boolean;
}

/** 传输条：播放/停止占位按钮与 BPM 数字输入。 */
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
