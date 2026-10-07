import { useEffect, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { AppShell } from "./components/AppShell";
import { TransportBar } from "./components/TransportBar";
import { clearProject, getProject, loadDemo, onEngineEvent, setTempo } from "./lib/api";
import type { Project } from "./lib/types";

/** 应用根组件：持有工程状态，挂载时读取工程、订阅引擎事件，并把命令结果写回状态。 */
function App() {
  const [project, setProject] = useState<Project | null>(null);
  const [playing, setPlaying] = useState(false);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let disposed = false;

    getProject()
      .then(setProject)
      .catch((err) => console.error("get_project failed", err));

    onEngineEvent((event) => {
      // 所有命令都会返回变更后的快照（真相源），事件流暂只记录，Task 4/5 再消费。
      console.debug("engine event", event);
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
    loadDemo()
      .then(setProject)
      .catch((err) => console.error("load_demo failed", err));
  };

  const handleClear = () => {
    clearProject()
      .then(setProject)
      .catch((err) => console.error("clear_project failed", err));
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
    </AppShell>
  );
}

export default App;
