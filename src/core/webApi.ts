/** Web 访问设置（桌面端）：本机 / 局域网地址与「在浏览器打开」。 */
import { invoke } from "@/api/ipc";
import { isTauri } from "@/utils/platform";
import { openExternal } from "@/utils/external";

export interface WebAccessInfo {
  lan: boolean;
  port: number;
  token: string;
  loopbackUrl: string;
  lanUrl: string | null;
  running: boolean;
  restartRequired: boolean;
}

export async function webAccessInfo(): Promise<WebAccessInfo | null> {
  if (!isTauri()) return null;
  return invoke<WebAccessInfo>("web_access_info");
}

export async function webAccessSetLan(lan: boolean): Promise<WebAccessInfo | null> {
  if (!isTauri()) return null;
  return invoke<WebAccessInfo>("web_access_set_lan", { lan });
}

export async function webAccessOpen(): Promise<void> {
  if (!isTauri()) return;
  const info = await webAccessInfo();
  if (info) await openExternal(info.loopbackUrl);
}
