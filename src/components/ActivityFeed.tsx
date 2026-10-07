/** `ActivityFeed` 的 props：已累积的引擎事件行。 */
export interface ActivityFeedProps {
  /** 事件行，按发生顺序追加（最新的在数组末尾）。 */
  lines: string[];
}

/** 活动列表：把引擎事件行倒序（最新在上）展示，是 "AI 操作可视化" 的最小雏形。 */
export function ActivityFeed({ lines }: ActivityFeedProps) {
  // 列表只追加，因此原始下标就是稳定 key；倒序后即为"最新在上"。
  const newestFirst = lines.map((line, id) => ({ id, line })).reverse();
  return (
    <section className="activity-feed">
      <header>
        <h2>Activity</h2>
      </header>
      {lines.length === 0 ? (
        <div className="empty">
          <p className="empty-title">No activity yet</p>
          <p className="empty-hint">Engine events and command results show up here.</p>
        </div>
      ) : (
        <ul>
          {newestFirst.map(({ id, line }) => (
            <li key={id}>{line}</li>
          ))}
        </ul>
      )}
    </section>
  );
}
