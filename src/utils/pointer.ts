/**
 * 指针捕获的安全封装。
 *
 * `setPointerCapture` 在以下情形会抛 `NotFoundError`：合成事件（自动化测试）、
 * 指针已被释放、元素已从文档中移除。捕获失败只影响「指针移出元素后是否继续收到
 * 事件」，不应该让整个拖拽交互中断，因此这里吞掉异常。
 */
export function capturePointer(target: EventTarget | null, pointerId: number): void {
  const el = target as HTMLElement | null;
  if (!el || typeof el.setPointerCapture !== "function") return;
  try {
    el.setPointerCapture(pointerId);
  } catch {
    // 忽略：降级为「不捕获」，拖拽仍可在元素内部继续。
  }
}

/** 释放指针捕获（同样容忍失败）。 */
export function releasePointer(target: EventTarget | null, pointerId: number): void {
  const el = target as HTMLElement | null;
  if (!el || typeof el.releasePointerCapture !== "function") return;
  try {
    el.releasePointerCapture(pointerId);
  } catch {
    // 忽略。
  }
}
