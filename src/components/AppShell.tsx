import type { ReactNode } from "react";
import { M3eButton } from "@m3e/react/button";

/** `AppShell` 的 props：顶栏动作与主内容插槽。 */
export interface AppShellProps {
  /** 载入内置示例工程。 */
  onLoadDemo: () => void;
  /** 清空（新建）工程。 */
  onClear: () => void;
  /** 主内容区（`App.tsx` 组合的传输条、轨道列表等）。 */
  children: ReactNode;
}

/** MD3 应用外壳：顶栏（应用名 + 示例/清空按钮）与 `<main>` 插槽。 */
export function AppShell({ onLoadDemo, onClear, children }: AppShellProps) {
  return (
    <div className="app-shell">
      <header className="app-bar">
        <h1>daw-party</h1>
        <div className="app-bar-actions">
          <M3eButton onClick={onLoadDemo}>Load demo</M3eButton>
          <M3eButton onClick={onClear}>New</M3eButton>
        </div>
      </header>
      <main>{children}</main>
    </div>
  );
}
