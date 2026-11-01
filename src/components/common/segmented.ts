/** MD3 分段按钮（`Md3Segmented`）的一个选项。 */
export interface SegmentedOption {
  /** 稳定值；`v-model` 与它比对。 */
  value: string;
  /** 显示文案。 */
  label: string;
  /** 不可点（例如正在保存）。 */
  disabled?: boolean;
  /** 悬停提示。 */
  title?: string;
}
