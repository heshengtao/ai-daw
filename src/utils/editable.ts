/**
 * 「这个键盘事件该不该原样交给浏览器」的判断。
 *
 * 两个视图都在 `window` 上监听 `keydown` 做 DAW 快捷键（Cmd+C 复制片段、Delete 删片段…），
 * 但它们只挡住了 `<input>`。于是：
 * - 焦点在 `<textarea>` 里输入时，`1` / `2` / `3` 会被当成「切换鼠标模式」，`Cmd+S` 会被当成
 *   「保存工程」——文本域里根本打不了字；
 * - 只要有一条片段/音符被选中，**用户在别处选中的文本**一按 Cmd+C 就被当成「复制片段」，
 *   系统复制被 `preventDefault()` 吞掉（选中地址栏外的文字复制不出来，就是这个原因）。
 *
 * 规则：焦点在可编辑控件里、或页面上存在**真实的文本选区**时，一律不抢这些按键。
 */

/** 事件目标是不是可编辑控件（输入框 / 文本域 / 下拉 / `contenteditable`）。 */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  return (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement
  );
}

/** 页面上是否有一段**真实的、非空的**文本选区（用户手动选中的文字）。 */
export function hasTextSelection(): boolean {
  const selection = document.getSelection();
  if (!selection || selection.isCollapsed) return false;
  return selection.toString().trim().length > 0;
}

/**
 * 这个键盘事件是否该交给浏览器处理（不当作 DAW 快捷键）。
 *
 * 所有 `window` 级的 keydown 处理器都该先过这一关。
 */
export function shouldDeferToBrowser(e: KeyboardEvent): boolean {
  return isEditableTarget(e.target) || hasTextSelection();
}
