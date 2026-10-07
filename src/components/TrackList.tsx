import { M3eButton } from "@m3e/react/button";
import type { Project } from "../lib/types";

/** `TrackList` 的 props：工程快照、当前选中轨道与两个回调。 */
export interface TrackListProps {
  /** 当前工程快照（真相源）。 */
  project: Project;
  /** 当前选中的轨道 id；`null` 表示未选中。 */
  selectedTrackId: number | null;
  /** 用户点击某条轨道时回调，传入该轨道 id。 */
  onSelect: (id: number) => void;
  /** 用户点击 "Add track" 时回调，传入新轨道名。 */
  onAddTrack: (name: string) => void;
}

/** 轨道列表：展示每条轨道的名称、乐器、片段数与音符数，并提供 "Add track" 入口。 */
export function TrackList({ project, selectedTrackId, onSelect, onAddTrack }: TrackListProps) {
  const add = () => onAddTrack(`Track ${project.tracks.length + 1}`);
  return (
    <section className="track-list">
      <header>
        <h2>Tracks</h2>
        <M3eButton onClick={add}>Add track</M3eButton>
      </header>
      {project.tracks.length === 0 ? (
        <p className="empty">No tracks yet — add one, or load the demo.</p>
      ) : (
        <ul>
          {project.tracks.map((t) => (
            <li
              key={t.id}
              className={t.id === selectedTrackId ? "selected" : ""}
              onClick={() => onSelect(t.id)}
            >
              <strong>{t.name}</strong>
              <span className="meta">
                {t.instrument ? t.instrument.plugin_id : "no instrument"} ·{" "}
                {t.clips.length} clip(s) ·{" "}
                {t.clips.reduce((n, c) => n + c.notes.length, 0)} note(s)
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
