/**
 * 引擎事件流：既是开发者抽屉的数据源，也是 AI 可视化的信息来源。
 *
 * 事件是**外部标记**的枚举（`{ TrackAdded: { actor, id, name } }`），其中 `actor` 为
 * `"Human"` 或 `{ Agent: "mcp" }`。这里把代理发起的事件单独抽出来，界面据此判断「AI 正在写」
 * 并高亮被改动的轨道——人类自己的编辑**不会**点亮 AI 指示。
 */
import { defineStore } from "pinia";
import { ref } from "vue";
import { onEngineEvent } from "@/api/tauri";

const MAX_LINES = 200;
/** 保留的最近代理动作条数。 */
const MAX_ACTIONS = 40;

/** 一次由 AI 代理发起的变更。 */
export interface AgentAction {
  /** 代理标识（`Actor::Agent(..)` 携带的字符串）。 */
  agent: string;
  /** 事件名（枚举变体名，如 `TrackAdded`）。 */
  kind: string;
  /** 第一个相关轨道 id；与轨道无关的事件为 `null`。 */
  track: number | null;
  /** 发生时刻（`Date.now()`）。 */
  at: number;
}

/** 把 `{ Kind: body }` 拆成事件名与字段；形状不符返回 `null`。 */
function splitEvent(payload: unknown): { kind: string; body: Record<string, unknown> } | null {
  if (!payload || typeof payload !== "object") return null;
  const entries = Object.entries(payload as Record<string, unknown>);
  if (entries.length !== 1) return null;
  const [kind, body] = entries[0];
  if (!body || typeof body !== "object" || Array.isArray(body)) return null;
  return { kind, body: body as Record<string, unknown> };
}

/** 取代理标识；人类事件返回 `null`。 */
function agentOf(actor: unknown): string | null {
  if (!actor || typeof actor !== "object") return null;
  const name = (actor as { Agent?: unknown }).Agent;
  return typeof name === "string" ? name : null;
}

/** 事件涉及的轨道 id（批量片段移动/删除会涉及多条轨道）。 */
function tracksOf(kind: string, body: Record<string, unknown>): number[] {
  const out: number[] = [];
  if (typeof body.track === "number") out.push(body.track);
  if ((kind === "TrackAdded" || kind === "TrackRemoved") && typeof body.id === "number") {
    out.push(body.id);
  }
  if (Array.isArray(body.items)) {
    for (const item of body.items) {
      const track = (item as { track?: unknown } | null)?.track;
      if (typeof track === "number") out.push(track);
    }
  }
  return out;
}

export const useEngineEventsStore = defineStore("engineEvents", () => {
  const lines = ref<string[]>([]);
  /** 按发生顺序追加的最近代理动作。 */
  const agentActions = ref<AgentAction[]>([]);
  /** 轨道 id → 最近一次被代理改动的时刻（非响应式；用带时间窗的取值函数读）。 */
  const trackHits = new Map<number, number>();
  let unlisten: (() => void) | null = null;

  /** 开始订阅（幂等）。 */
  async function start(): Promise<void> {
    if (unlisten) return;
    const off = await onEngineEvent((payload) => {
      lines.value.push(JSON.stringify(payload));
      if (lines.value.length > MAX_LINES) lines.value.splice(0, lines.value.length - MAX_LINES);

      const parsed = splitEvent(payload);
      if (!parsed) return;
      const agent = agentOf(parsed.body.actor);
      if (agent === null) return; // 人类编辑与 AI 无关
      const at = Date.now();
      const tracks = tracksOf(parsed.kind, parsed.body);
      for (const track of tracks) trackHits.set(track, at);
      agentActions.value.push({ agent, kind: parsed.kind, track: tracks[0] ?? null, at });
      if (agentActions.value.length > MAX_ACTIONS) {
        agentActions.value.splice(0, agentActions.value.length - MAX_ACTIONS);
      }
    });
    unlisten = off;
  }

  /** 停止订阅。 */
  function stop(): void {
    unlisten?.();
    unlisten = null;
  }

  /** 清空日志与代理动作记录。 */
  function clear(): void {
    lines.value = [];
    agentActions.value = [];
    trackHits.clear();
  }

  /** 最近一次代理动作的时刻（没有则 0）。 */
  function lastAgentAt(): number {
    const last = agentActions.value[agentActions.value.length - 1];
    return last ? last.at : 0;
  }

  /** 最近 `withinMs` 毫秒内是否有代理在写。 */
  function agentActive(withinMs = 900): boolean {
    return Date.now() - lastAgentAt() < withinMs;
  }

  /**
   * 最近 `withinMs` 毫秒内被代理改动过的轨道（升序）。
   * 与 `agentActive` 一样是渲染时的取值，非响应式；需要响应性的地方配合定时器使用。
   */
  function agentTracks(withinMs = 1600): number[] {
    const now = Date.now();
    const out: number[] = [];
    for (const [track, at] of trackHits) {
      if (now - at < withinMs) out.push(track);
    }
    return out.sort((a, b) => a - b);
  }

  /** 最近一次代理动作的代理名（没有则 `null`）。 */
  function activeAgent(): string | null {
    const last = agentActions.value[agentActions.value.length - 1];
    return last ? last.agent : null;
  }

  return {
    lines,
    agentActions,
    start,
    stop,
    clear,
    agentActive,
    agentTracks,
    activeAgent,
  };
});
