/** 运行平台探测。 */

/** 是否运行在 Tauri 窗口里（而非浏览器）。 */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** 是否 macOS。 */
export function isMacOS(): boolean {
  return typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent);
}

/** 是否 Windows。 */
export function isWindows(): boolean {
  return typeof navigator !== "undefined" && /Win/i.test(navigator.userAgent);
}

/** 是否 Linux。 */
export function isLinux(): boolean {
  return typeof navigator !== "undefined" && /Linux/i.test(navigator.userAgent);
}
