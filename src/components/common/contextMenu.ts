/**
 * 右键菜单的一项。
 *
 * 文案、图标与快捷键都由调用方给（各视图的语义不同）；`children` 让它成为一个二级菜单的
 * 入口。带图标 + 键帽 + AI 强调的观感与 `D:\ai-cut` 的右键菜单保持一致。
 */
import type { Component } from "vue";

export interface ContextMenuItem {
  /** 稳定标识（调用方在 `select` 里分支）。 */
  id: string;
  /** 展示文案（已翻译）。 */
  label: string;
  /** 行首图标（Heroicons 组件）；不给也会留出等宽空位，文字不会左右跳。 */
  icon?: Component;
  /** 行尾快捷键（拆成键帽，如 `["Ctrl", "C"]`）。 */
  keys?: string[];
  /** 悬停说明 / 补充信息。 */
  hint?: string;
  /** 行内小标签（例如 AI）。 */
  badge?: string;
  /** AI 动作：图标用主题色强调。 */
  ai?: boolean;
  /** 禁用（灰显、不可点）。 */
  disabled?: boolean;
  /** 危险操作（删除）用错误色。 */
  danger?: boolean;
  /** 在这一项之前画一条分隔线。 */
  divided?: boolean;
  /** 勾选状态（静音 / 独奏这类开关项）。 */
  checked?: boolean;
  /** 二级菜单；为空数组时按普通项处理。 */
  children?: ContextMenuItem[];
}
