import { useEffect, useRef, useState } from "react";
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

/** 允许下发的 BPM 区间（与后端引擎的安全范围一致）。区间外的输入只留在本地。 */
const MIN_BPM = 20;
const MAX_BPM = 300;

/** 传输条：播放/停止按钮（离线渲染试听）与 BPM 数字输入。 */
export function TransportBar({ tempo, onTempoChange, onPlay, onStop, playing }: TransportBarProps) {
  // 本地输入值：让"清空输入框 / 输入中间态"不会被受控的 `tempo` 立即覆盖。
  const [draft, setDraft] = useState(String(tempo));
  // 编辑期间不回写外部 `tempo`，避免打断用户正在输入的文本。
  const editingRef = useRef(false);

  useEffect(() => {
    if (!editingRef.current) setDraft(String(tempo));
  }, [tempo]);

  const handleChange = (raw: string) => {
    setDraft(raw);
    const bpm = Number(raw);
    // 空字段得到 `Number("") === 0`；连同 NaN 与超范围值一起忽略，不下发 `set_tempo`。
    if (!Number.isFinite(bpm) || bpm < MIN_BPM || bpm > MAX_BPM) return;
    onTempoChange(bpm);
  };

  return (
    <section className="transport-bar">
      <M3eButton onClick={playing ? onStop : onPlay}>{playing ? "Stop" : "Play"}</M3eButton>
      <label>
        BPM
        <input
          type="number" min={MIN_BPM} max={MAX_BPM} value={draft}
          onFocus={() => { editingRef.current = true; }}
          onBlur={() => { editingRef.current = false; setDraft(String(tempo)); }}
          onChange={(e) => handleChange(e.target.value)}
        />
      </label>
    </section>
  );
}
