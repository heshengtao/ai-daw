/**
 * 历史工程分组：同一个工程的多次快照只占一张卡片，卡片内再用版本下拉切换。
 *
 * 后端的 `project_history` 返回的是平铺快照（每份一个 id）；直接铺卡片时，一个工程保存
 * 十次就会刷满整面墙。这里先归并再交给界面渲染。
 */
import type { ProjectHistoryEntry } from "@/api/types";

/** 同一个工程的多次历史快照。 */
export interface ProjectHistoryGroup {
  /** 分组键：有路径时按路径，未保存工程按身份键，老索引各自成组。 */
  key: string;
  name: string;
  path: string | null;
  /** 按记录时间倒序：`versions[0]` 是最新版本。 */
  versions: ProjectHistoryEntry[];
}

/** 路径分隔符与结尾斜杠统一，免得 Windows / macOS 写法不同把同一工程拆成两组。 */
function normalizePath(path: string): string {
  return path.replace(/\\/g, "/").replace(/\/+$/, "");
}

/**
 * 把同一工程的多份快照归到一组。
 *
 * 优先级：
 * 1. 工程文件路径——已保存的工程最可靠的身份证（改工程名不影响）；
 * 2. `project_key`——未保存工程没有路径，用后端记下的身份键；
 * 3. 两者都没有（旧版本索引）——各自成组，宁可多一张卡片也不误合并。
 */
export function groupProjectHistory(
  entries: ProjectHistoryEntry[],
): ProjectHistoryGroup[] {
  const groups = new Map<string, ProjectHistoryGroup>();
  for (const entry of entries) {
    let key: string;
    if (entry.path) {
      key = `path:${normalizePath(entry.path)}`;
    } else if (entry.project_key) {
      key = `project:${entry.project_key}`;
    } else {
      key = `legacy:${entry.id}`;
    }

    const group = groups.get(key);
    if (group) {
      group.versions.push(entry);
    } else {
      groups.set(key, {
        key,
        name: entry.name,
        path: entry.path,
        versions: [entry],
      });
    }
  }

  const out = [...groups.values()];
  for (const group of out) {
    group.versions.sort((a, b) => b.saved_at - a.saved_at);
    // 名称 / 路径以最新版本为准：工程改名后卡片跟着改，旧版本仍留在下拉里。
    group.name = group.versions[0]?.name ?? group.name;
    group.path = group.versions[0]?.path ?? group.path;
  }
  out.sort(
    (a, b) => (b.versions[0]?.saved_at ?? 0) - (a.versions[0]?.saved_at ?? 0),
  );
  return out;
}
