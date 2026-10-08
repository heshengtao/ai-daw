/** 工作区导航项（供 SegmentedNav 使用）。 */
export interface NavItem {
  /** i18n key：文案在渲染时翻译。 */
  labelKey: string;
  path: string;
  /** heroicon 组件。 */
  icon: unknown;
}
