import { useEffect, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { ActivityFeed } from "./components/ActivityFeed";
import { AppShell } from "./components/AppShell";
import { TrackList } from "./components/TrackList";
import { TransportBar } from "./components/TransportBar";
import { addTrack, clearProject, getProject, loadDemo, onEngineEvent, setTempo } from "./lib/api";
import type { Project } from "./lib/types";

/** 活动列表最多保留的行数，避免长时间会话里无限增长。 */
const MAX_ACTIVITY_LINES = 100;

/** 应用根组件：持有工程状态与本地 UI 状态，读取工程、订阅引擎事件，并把命令结果写回状态。 */
function App() {
  const [project, setProject] = useState<Project | null>(null);
  const [playing, setPlaying] = useState(false);
  const [selectedTrackId, setSelectedTrackId] = useState<number | null>(null);
  const [activity, setActivity] = useState<string[]>([]);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let disposed = false;

    getProject()
      .then(setProject)
      .catch((err) => console.error("get_project failed", err));

    onEngineEvent((event) => {
      // 事件流只喂给活动列表（唯一消费者）；`project` 一律由命令返回的快照写回，
      // 所以同一次变更不会被"事件 + 返回值"重复应用。事件形状未知，序列化即展示。
      const line = JSON.stringify(event);
      setActivity((prev) => [...prev, line].slice(-MAX_ACTIVITY_LINES));
    })
      .then((fn) => {
        // 卸载（含 React StrictMode 的二次挂载）时立即退订，避免泄漏。
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch((err) => console.error("onEngineEvent failed", err));

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const handleTempoChange = (bpm: number) => {
    setTempo(bpm)
      .then(setProject)
      .catch((err) => console.error("set_tempo failed", err));
  };

  const handleLoadDemo = () => {
    // `load_demo` 既发事件又返回快照；以返回的快照为真相源，避免重复应用。
    // 轨道 id 会被重新分配，所以清掉旧的选中态。
    loadDemo()
      .then((next) => {
        setProject(next);
        setSelectedTrackId(null);
      })
      .catch((err) => console.error("load_demo failed", err));
  };

  const handleClear = () => {
    clearProject()
      .then((next) => {
        setProject(next);
        setSelectedTrackId(null);
      })
      .catch((err) => console.error("clear_project failed", err));
  };

  const handleAddTrack = (name: string) => {
    // 与其它命令一致：以返回的快照写回，事件只进活动列表。
    addTrack(name)
      .then(setProject)
      .catch((err) => console.error("add_track failed", err));
  };

  return (
    <AppShell onLoadDemo={handleLoadDemo} onClear={handleClear}>
      <TransportBar
        tempo={project?.transport.tempo ?? 120}
        onTempoChange={handleTempoChange}
        onPlay={() => setPlaying(true)}
        onStop={() => setPlaying(false)}
        playing={playing}
      />
      {/* 首帧 `get_project` 未返回时没有可渲染的轨道；拿到快照后再挂载工作区。 */}
      {project && (
        <div className="workspace">
          <TrackList
            project={project}
            selectedTrackId={selectedTrackId}
            onSelect={setSelectedTrackId}
            onAddTrack={handleAddTrack}
          />
          <ActivityFeed lines={activity} />
        </div>
      )}
    </AppShell>
  );
}

export default App;
