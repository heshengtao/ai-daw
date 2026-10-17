/**
 * 复制文本到系统剪贴板。
 *
 * 优先用异步剪贴板 API（`navigator.clipboard`）。它在**不安全上下文**（自定义 scheme 的
 * WebView）、**文档未聚焦**、或**权限被拒**时要么不存在、要么直接 reject——Tauri 的 WebView
 * 就踩在第一条上。所以这里再留一条老路：临时 `<textarea>` + `document.execCommand("copy")`。
 *
 * 返回是否成功，调用方据此决定要不要提示「请手动复制」。
 */
export async function copyText(text: string): Promise<boolean> {
  if (!text) return false;
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // 落到下面的兜底。
  }
  return legacyCopy(text);
}

/** 老路：临时 textarea + `execCommand("copy")`。 */
function legacyCopy(text: string): boolean {
  const area = document.createElement("textarea");
  area.value = text;
  // 放视口外并设成透明：**不能**用 `display: none`（那样选不中，也就复制不了）。
  area.setAttribute("readonly", "");
  area.style.position = "fixed";
  area.style.top = "-1000px";
  area.style.left = "0";
  area.style.opacity = "0";
  document.body.appendChild(area);

  // 记下用户原本选中的东西，复制完还回去——别为了复制一段文字把人家选中的文字弄丢。
  const selection = document.getSelection();
  const previous = selection && selection.rangeCount > 0 ? selection.getRangeAt(0).cloneRange() : null;

  let ok = false;
  try {
    area.select();
    ok = document.execCommand("copy");
  } catch {
    ok = false;
  } finally {
    area.remove();
    if (previous && selection) {
      selection.removeAllRanges();
      selection.addRange(previous);
    }
  }
  return ok;
}
