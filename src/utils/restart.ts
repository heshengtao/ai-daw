/**
 * 重启应用（让新装的运行时 / 组件 / 新版本生效）。
 *
 * 为什么一定要走这里而不是各处直接调 `relaunch()`：
 * **开发模式下重启会把前端 dev server 一起带走**——旧进程退出时 `tauri dev` 的 CLI
 * 会关掉 vite，新进程再去连 `devUrl` 就只剩一个「拒绝连接」的空白窗口。
 * 设置页原本就有这个守卫，AI 面板 / 下载管理器却是直接 `relaunch()`，同一个按钮
 * 在两个地方行为不同。现在统一到这一处：开发模式只提示手动重启。
 */
import { relaunchApp } from "@/api/updater";
import { useI18n } from "@/i18n";
import { useLayoutStore } from "@/stores/layout";

/** 真正重启了返回 `true`；开发模式下只提示、不重启，返回 `false`。 */
export async function restartApp(): Promise<boolean> {
  if (import.meta.env.DEV) {
    useLayoutStore().notify(useI18n().t("settings.ai.restartDevHint"));
    return false;
  }
  try {
    await relaunchApp();
  } catch (err) {
    useLayoutStore().notify(String(err));
    return false;
  }
  return true;
}
