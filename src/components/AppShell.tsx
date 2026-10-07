import { useEffect, useState, type ReactNode } from "react";
import { documentDir, join } from "@tauri-apps/api/path";
import { M3eButton } from "@m3e/react/button";

/** 导出文本框的默认文件名；目录取自系统文档目录。 */
const DEFAULT_EXPORT_NAME = "daw-party-export.wav";

/** `AppShell` 的 props：顶栏动作与主内容插槽。 */
export interface AppShellProps {
  /** 载入内置示例工程。 */
  onLoadDemo: () => void;
  /** 清空（新建）工程。 */
  onClear: () => void;
  /** 导出当前工程为 WAV，参数为用户在文本框里填写的目标路径。 */
  onExport: (path: string) => void;
  /** 非命令类失败（如默认导出路径解析）的回调，用于把错误显示到活动列表。 */
  onError?: (message: string) => void;
  /** 主内容区（`App.tsx` 组合的传输条、轨道列表等）。 */
  children: ReactNode;
}

/** MD3 应用外壳：顶栏（应用名 + 示例/清空/导出）与 `<main>` 插槽。 */
export function AppShell({ onLoadDemo, onClear, onExport, onError, children }: AppShellProps) {
  // 导出路径是纯 UI 状态，放在这里而不是 `App`：它只服务于这个文本框，
  // 也不会被任何后端返回值覆盖（导出结果只进活动列表）。
  const [exportPath, setExportPath] = useState("");

  useEffect(() => {
    let disposed = false;
    // 默认值 = `~/Documents/daw-party-export.wav`（由系统文档目录拼出）。
    // 解析是异步的：先用 `prev || defaultPath` 兜底，保证解析返回时不会
    // 清掉用户在这期间已经输入的内容。
    documentDir()
      .then((dir) => join(dir, DEFAULT_EXPORT_NAME))
      .then((defaultPath) => {
        if (!disposed) setExportPath((prev) => prev || defaultPath);
      })
      .catch((err) => {
        // 解析失败不静默：把原因交给 App 写进活动列表（文本框仍可用，用户可手动输入路径）。
        const message = err instanceof Error ? err.message : String(err);
        onError?.(`Could not resolve the default export path: ${message}`);
      });
    return () => {
      disposed = true;
    };
  }, [onError]);

  return (
    <div className="app-shell">
      <header className="app-bar">
        <h1>daw-party</h1>
        <div className="app-bar-actions">
          <M3eButton onClick={onLoadDemo}>Load demo</M3eButton>
          <M3eButton onClick={onClear}>New</M3eButton>
          <label className="export-path">
            Export to
            <input
              type="text"
              value={exportPath}
              placeholder={DEFAULT_EXPORT_NAME}
              onChange={(e) => setExportPath(e.target.value)}
            />
          </label>
          <M3eButton onClick={() => onExport(exportPath.trim())}>Export WAV</M3eButton>
        </div>
      </header>
      <main>{children}</main>
    </div>
  );
}
