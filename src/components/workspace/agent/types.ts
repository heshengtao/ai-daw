/** 写歌智能体面板内部的展示类型。 */

/** 一张工具卡片（流式或定稿后都归一到这个形状）。 */
export interface ToolCard {
  id: string;
  name: string;
  args: unknown;
  /** `null` = 还在跑。 */
  ok: boolean | null;
  output: string;
}
