import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/** 后端返回的工程快照（Task 3 会用完整的 TS 类型替换这里）。 */
type ProjectSummary = {
  name: string;
  tracks: { name: string }[];
};

/** 占位外壳：只在启动时读取工程，并把工程名与轨道数渲染成纯文本。 */
function App() {
  const [project, setProject] = useState<ProjectSummary | null>(null);

  useEffect(() => {
    invoke<ProjectSummary>("get_project")
      .then(setProject)
      .catch((err) => console.error("get_project failed", err));
  }, []);

  if (!project) {
    return <main>loading…</main>;
  }

  return (
    <main>
      <p>project: {project.name}</p>
      <p>tracks: {project.tracks.length}</p>
    </main>
  );
}

export default App;
