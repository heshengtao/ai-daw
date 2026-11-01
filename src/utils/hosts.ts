/**
 * 下载域名的信任判断（与后端 `src-tauri/src/manager.rs` 的 `is_trusted_download_host` /
 * `download_host` 一一对应，规则必须同步改）。
 *
 * 只有「厂商自己的域名」和「国内几家大厂镜像站」算官方源；**其余一律算第三方**
 * （插件作者自己的站、个人反代、来路不明的加速站）。第三方域名在下载前必须先把
 * 域名亮给用户看并让用户点头，后端还会再比对一次——两道闸，前端这道是为了让人看见。
 */

/** 受信任的根域名（含其子域）。 */
const TRUSTED_ROOTS = [
  // 代码托管 / 发布
  "github.com",
  "githubusercontent.com",
  "gitee.com",
  // 模型仓库
  "modelscope.cn",
  "huggingface.co",
  "hf.co",
  // 大厂镜像站（系统包 / NuGet 等）
  "mirrors.tuna.tsinghua.edu.cn",
  "mirrors.aliyun.com",
  "mirrors.huaweicloud.com",
  "mirrors.ustc.edu.cn",
  "mirrors.cloud.tencent.com",
  // 运行时厂商自己
  "api.nuget.org",
  "nvidia.com",
  "nvidia.cn",
  "microsoft.com",
] as const;

/** 域名归一化：小写、去 `www.`、去结尾的点。 */
function normalizeHost(host: string): string {
  return host
    .trim()
    .replace(/^www\./i, "")
    .replace(/\.+$/, "")
    .toLowerCase();
}

/** 从一个下载地址里取出域名；解析不出来返回空串。 */
export function downloadHost(url: string): string {
  try {
    return normalizeHost(new URL(url).hostname);
  } catch {
    // 地址不合法时手写兜底：`scheme://[user:pass@]host[:port]/…`
    const authority = /^[a-z][a-z0-9+.-]*:\/\/([^/?#]*)/i.exec(url.trim())?.[1] ?? "";
    const hostPort = authority.split("@").pop() ?? "";
    return normalizeHost(hostPort.split(":")[0] ?? "");
  }
}

/** 这个域名是不是官方 / 大厂（按 label 边界匹配，`github.com.evil.tld` 不算）。 */
export function isTrustedDownloadHost(host: string): boolean {
  const normalized = normalizeHost(host);
  if (!normalized) return false;
  return TRUSTED_ROOTS.some(
    (root) => normalized === root || normalized.endsWith(`.${root}`),
  );
}

/**
 * 这个下载地址要不要先把域名给用户确认。
 *
 * 注意：这里只看**注册表里的原始地址**；用户如果配了「下载代理前缀」，真实请求会先走
 * 那个前缀——那一层由后端在下载前判断（前端拿不到重写后的地址）。
 */
export function needsHostConfirmation(url: string): boolean {
  return !isTrustedDownloadHost(downloadHost(url));
}
