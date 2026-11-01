/** 右键菜单的一项。文案与快捷键都由调用方给（各视图的语义不同）。 */
export interface ContextMenuItem {
  /** 稳定标识（调用方在 `select` 里分支）。 */
  id: string;
  /** 展示文案（已翻译）。 */
  label: string;
  /** 右侧快捷键提示（如 `⌘C` / `Ctrl+C`），可省略。 */
  shortcut?: string;
  /** 禁用（灰显、不可点）。 */
  disabled?: boolean;
  /** 危险操作（删除）用错误色。 */
  danger?: boolean;
  /** 在这一项之前画一条分隔线。 */
  divided?: boolean;
}
