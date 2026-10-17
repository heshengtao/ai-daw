/**
 * 把后端「一轮一个助手节点」的节点树，投影成 DSH 风格的「Turn」视图。
 *
 * DSH 的聊天展示有两条硬规则（见其 `ui-chat` 的 README / TurnProcessNodeView）：
 * 1. 一轮（Turn）的探索过程（思考、中间回复、工具调用）收在一个可折叠的过程里；
 * 2. **最终回答**是最后一个「有正文、没有工具调用」的助手节点，它留在折叠之外。
 *
 * 后端每轮会产出多个助手节点（模型每回答一次、调一批工具、再回答），所以直接按节点
 * 渲染会把思考夹在工具列表中间。这里先把节点按用户消息分组到 Turn，再按上面的规则
 * 拆成「过程条目 + 最终回答」。
 */
import type { AgentNode, AgentToolCallRef } from "@/api/types";
import type { ToolRun } from "@/stores/agent";
import type { ToolCard } from "./types";

/** 过程里的一条：思考 / 中间回复 / 一批工具调用。 */
export type ProcessEntry =
  | { kind: "reasoning"; key: string; text: string; running: boolean }
  | { kind: "text"; key: string; text: string }
  | { kind: "tools"; key: string; nodeId: string; tools: ToolCard[]; running: boolean };

export type TurnStatus = "running" | "done" | "stopped" | "error";

/** 渲染一轮所需的全部信息。 */
export interface TurnView {
  key: string;
  /** 用户消息节点；历史里若只有助手节点则为 null。 */
  user: AgentNode | null;
  /** 用户消息的兄弟分支（`‹ i/n ›`）。 */
  branches: AgentNode[];
  /** 折叠在「过程」里的条目，按发生顺序。 */
  entries: ProcessEntry[];
  /** 最终回答正文（流式时是尚未定稿的那条）。 */
  answer: string;
  /** 最终回答是否还在流式生成。 */
  answerStreaming: boolean;
  running: boolean;
  status: TurnStatus;
  /** 流式失败的原因（`status === "error"` 时）。 */
  error: string | null;
  startedAt: number;
  endedAt: number;
  durationMs: number;
}

export interface TurnSource {
  /** 当前活跃分支的节点（显示顺序，含工具节点）。 */
  path: AgentNode[];
  running: boolean;
  stopped: boolean;
  streamingNodeId: string | null;
  streamingContent: string;
  streamingReasoning: string;
  toolRuns: Record<string, ToolRun[]>;
  branchesOf: (nodeId: string) => AgentNode[];
}

/** 安全解析工具参数文本。 */
export function safeParse(text: string): unknown {
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

/**
 * 上一帧的 Turn 视图，按轮次 key 记住。
 *
 * 流式期间每个刷新窗口都会重算整棵 Turn 列表，如果每次都产出新对象，**所有**轮次都会
 * 重新渲染；而 Vue 的 `v-html` 是每次 patch 都无条件重赋 `innerHTML`（不做等值比较），
 * 等于把整段历史 DOM 拆了重建一遍。文本越长越慢，主线程会被压死。
 *
 * 这里让「内容没变的轮次」直接复用上一帧的对象：引用不变 → 子组件 props 不变 →
 * Vue 跳过整棵子树，只有真正在变的那一轮重新渲染。
 */
const previous = new Map<string, TurnView>();

function sameTools(a: ToolCard[], b: ToolCard[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const x = a[i];
    const y = b[i];
    // `args` 不比较：同一次调用的参数不会变，复用旧对象即可（避免每次重新 parse 出的
    // 新对象让比较永远失败）。
    if (x.id !== y.id || x.name !== y.name || x.ok !== y.ok || x.output !== y.output) return false;
  }
  return true;
}

function sameEntry(a: ProcessEntry, b: ProcessEntry): boolean {
  if (a.kind !== b.kind || a.key !== b.key) return false;
  if (a.kind === "reasoning" && b.kind === "reasoning") {
    return a.text === b.text && a.running === b.running;
  }
  if (a.kind === "text" && b.kind === "text") return a.text === b.text;
  if (a.kind === "tools" && b.kind === "tools") {
    return a.running === b.running && sameTools(a.tools, b.tools);
  }
  return false;
}

function sameBranches(a: AgentNode[], b: AgentNode[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i].id !== b[i].id) return false;
  return true;
}

/** 两个 Turn 视图渲染出来的东西是否完全一致。 */
function sameTurn(a: TurnView, b: TurnView): boolean {
  if (
    a.user !== b.user ||
    a.answer !== b.answer ||
    a.answerStreaming !== b.answerStreaming ||
    a.running !== b.running ||
    a.status !== b.status ||
    a.error !== b.error ||
    a.startedAt !== b.startedAt ||
    a.endedAt !== b.endedAt ||
    a.entries.length !== b.entries.length ||
    !sameBranches(a.branches, b.branches)
  ) {
    return false;
  }
  for (let i = 0; i < a.entries.length; i++) {
    if (!sameEntry(a.entries[i], b.entries[i])) return false;
  }
  return true;
}

function toolCard(call: AgentToolCallRef, result: AgentNode | undefined, runs: ToolRun[] | undefined): ToolCard {
  const run = runs?.find((r) => r.callId === call.id);
  // 有落盘结果就以结果为准；正在跑看流式状态；都没有 = 还没开始 / 被中断。
  const ok = result ? result.error == null : run ? run.ok : null;
  const output = result?.content ?? run?.output ?? "";
  return {
    id: call.id || call.name,
    name: call.name,
    args: safeParse(call.arguments),
    ok,
    output,
  };
}

const assistantText = (node: AgentNode): string => node.content;

/** 按用户消息切分节点，保持 path 的顺序。 */
function groupTurns(path: AgentNode[]): { user: AgentNode | null; nodes: AgentNode[] }[] {
  const turns: { user: AgentNode | null; nodes: AgentNode[] }[] = [];
  for (const node of path) {
    if (node.role === "user") {
      turns.push({ user: node, nodes: [] });
      continue;
    }
    if (turns.length === 0) turns.push({ user: null, nodes: [] });
    turns[turns.length - 1].nodes.push(node);
  }
  return turns;
}

export function buildTurns(source: TurnSource): TurnView[] {
  const raw = groupTurns(source.path);
  const liveId = source.streamingNodeId;
  const liveVisible = source.running && !!liveId && !source.path.some((n) => n.id === liveId);
  if ((liveVisible || source.running) && raw.length === 0) {
    raw.push({ user: null, nodes: [] });
  }

  const views = raw.map((group, index) => {
    const isLast = index === raw.length - 1;
    const running = isLast && source.running;
    const streamLive = isLast && liveVisible;

    const assistantNodes = group.nodes.filter((n) => n.role === "assistant");
    const results = new Map<string, AgentNode>();
    for (const node of group.nodes) {
      if (node.role === "tool" && node.tool_call_id) results.set(node.tool_call_id, node);
    }

    // 最终回答：最后一个「有正文、没有工具调用」的助手节点。
    let answerNode: AgentNode | undefined;
    for (const node of assistantNodes) {
      if (node.tool_calls.length === 0 && assistantText(node).trim() !== "") answerNode = node;
    }

    const entries: ProcessEntry[] = [];
    for (const node of assistantNodes) {
      if (node.reasoning && node.reasoning.trim() !== "") {
        entries.push({ kind: "reasoning", key: `${node.id}:r`, text: node.reasoning, running: false });
      }
      if (node !== answerNode && assistantText(node).trim() !== "") {
        entries.push({ kind: "text", key: `${node.id}:t`, text: node.content });
      }
      if (node.tool_calls.length > 0) {
        const tools = node.tool_calls.map((call) =>
          toolCard(call, results.get(call.id), source.toolRuns[node.id]),
        );
        entries.push({
          kind: "tools",
          key: `${node.id}:tools`,
          nodeId: node.id,
          tools,
          running: running && tools.some((t) => t.ok === null),
        });
      }
    }

    // 流式中的助手节点还没落盘：它的思考和正文单独接在最后。
    if (streamLive) {
      if (source.streamingReasoning.trim() !== "") {
        entries.push({
          kind: "reasoning",
          key: `${liveId}:r`,
          text: source.streamingReasoning,
          running: true,
        });
      }
    }

    const answer = streamLive ? source.streamingContent : answerNode?.content ?? "";

    const errored = assistantNodes.some((n) => n.error != null);
    const unanswered = assistantNodes.some((n) => n.tool_calls.some((c) => !results.has(c.id)));
    const status: TurnStatus = running
      ? "running"
      : errored
        ? "error"
        : (isLast && source.stopped) || unanswered
          ? "stopped"
          : "done";
    const errorNode = [...assistantNodes].reverse().find((n) => n.error != null);

    const startedAt = group.user?.created_at ?? group.nodes[0]?.created_at ?? Date.now();
    const lastAt = group.nodes.reduce((max, n) => Math.max(max, n.created_at), startedAt);
    const endedAt = lastAt;

    const next: TurnView = {
      key: group.user?.id ?? group.nodes[0]?.id ?? `turn-${index}`,
      user: group.user,
      branches: group.user ? source.branchesOf(group.user.id) : [],
      entries,
      answer,
      answerStreaming: streamLive && source.streamingContent !== "",
      running,
      status,
      error: status === "error" ? errorNode?.error ?? null : null,
      startedAt,
      endedAt,
      durationMs: Math.max(1000, endedAt - startedAt),
    };
    // 没变化的轮次沿用上一帧的对象——**并且不要把 `previous` 覆盖成新对象**，
    // 否则引用会在两次重算之间来回翻转，历史轮次又会被整棵子树重渲染。
    const last = previous.get(next.key);
    if (last && sameTurn(last, next)) return last;
    previous.set(next.key, next);
    return next;
  });

  // 历史轮次（切会话 / 删会话）从缓存里清掉，避免 Map 无限增长。
  for (const key of previous.keys()) {
    if (!views.some((v) => v.key === key)) previous.delete(key);
  }
  return views;
}
