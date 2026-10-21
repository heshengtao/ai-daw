/**
 * 写歌智能体：会话视图 + 一轮对话的流式状态。
 *
 * 后端是「会话的唯一真相源」（落盘为节点树）；这里保存当前会话的节点、活跃分支，
 * 以及正在流式生成的那条助手消息的正文 / 思考 / 工具卡片。
 */
import { defineStore } from "pinia";
import { computed, ref } from "vue";
import {
  agentCancel,
  agentConfig,
  agentDeleteSession,
  agentModels,
  agentRunning,
  agentSend,
  agentSession,
  agentSessions,
  agentSetConfig,
  agentSetHead,
  agentTest,
  onAgentEvent,
} from "@/api/tauri";
import type {
  AgentConfig,
  AgentEvent,
  AgentNode,
  AgentProbe,
  AgentSession,
  AgentSessionSummary,
} from "@/api/types";
import { useLayoutStore } from "./layout";

/** 流式期间一张工具卡片的运行状态。 */
export interface ToolRun {
  callId: string;
  name: string;
  args: unknown;
  /** `null` = 还在跑。 */
  ok: boolean | null;
  output: string;
}

export const useAgentStore = defineStore("agent", () => {
  const layout = useLayoutStore();

  const config = ref<AgentConfig | null>(null);
  const sessions = ref<AgentSessionSummary[]>([]);
  const session = ref<AgentSession | null>(null);
  const running = ref(false);
  /** 已按过停止、正在等后端收尾（按钮立刻给出反馈，不必干等 `turn_finished`）。 */
  const stopping = ref(false);
  /** 最近一轮是否被用户中途停止（由 `turn_finished.stopped` 给出）。 */
  const stopped = ref(false);

  /** 正在流式生成的助手节点 id（便于把 delta 贴到对应气泡）。 */
  const streamingNodeId = ref<string | null>(null);
  const streamingContent = ref("");
  const streamingReasoning = ref("");
  /**
   * 工具运行态，按**发起的助手节点 id** 分组。
   *
   * 一轮对话里模型会反复「回答 → 调工具 → 再回答」，每个助手节点都有自己的一批
   * 工具调用。只有按节点分组，界面才能把「运行中 / 完成 / 输出」贴回它所属的那一轮，
   * 而不是把整轮的工具堆到最后一条流式消息下面。
   */
  const toolRuns = ref<Record<string, ToolRun[]>>({});
  /** 工具事件计数：给界面当滚动 / 重算的信号，本身不参与渲染。 */
  const toolTick = ref(0);

  let unlisten: (() => void) | null = null;
  let subscribing: Promise<void> | null = null;
  let wanted = false;

  /**
   * 流式增量先攒着，按固定窗口合并后再写进响应式状态。
   *
   * 后端是**按 token** 推 `delta` / `reasoning` 的（长思考时每秒上百条）。如果每来一条就
   * 改一次 `streamingContent`，整棵会话树会跟着重算一遍、每轮对话的 `v-html` 也会重新赋值
   * `innerHTML`（Vue 对 `v-html` 不做等值比较，等于每次都重建 DOM）。文本越长越慢，
   * 主线程永远追不上——表现就是「点一下展开就卡死、计时不走」。
   */
  const FLUSH_MS = 80;
  let pendingContent = "";
  let pendingReasoning = "";
  let pendingNodeId: string | null = null;
  let flushTimer: ReturnType<typeof setTimeout> | null = null;

  /** 把攒下的增量一次性写进响应式状态。 */
  function flushStreaming(): void {
    if (flushTimer !== null) {
      clearTimeout(flushTimer);
      flushTimer = null;
    }
    if (pendingNodeId !== null) streamingNodeId.value = pendingNodeId;
    if (pendingContent) {
      streamingContent.value += pendingContent;
      pendingContent = "";
    }
    if (pendingReasoning) {
      streamingReasoning.value += pendingReasoning;
      pendingReasoning = "";
    }
  }

  /** 丢弃还没落地的增量（新一轮 / 定稿 / 出错时）。 */
  function dropPending(): void {
    if (flushTimer !== null) {
      clearTimeout(flushTimer);
      flushTimer = null;
    }
    pendingContent = "";
    pendingReasoning = "";
    pendingNodeId = null;
  }

  /** 攒一条增量，窗口到期后统一刷新。 */
  function queueDelta(nodeId: string, text: string, kind: "content" | "reasoning"): void {
    if (pendingNodeId !== nodeId) {
      // 换了一条节点：上一条的尾巴先落地，避免两段文字串到同一个气泡里。
      flushStreaming();
      pendingNodeId = nodeId;
    }
    if (kind === "content") pendingContent += text;
    else pendingReasoning += text;
    if (flushTimer === null) {
      flushTimer = setTimeout(() => {
        flushTimer = null;
        flushStreaming();
      }, FLUSH_MS);
    }
  }

  /** 配置是否完整。 */
  const configured = computed(() => {
    const c = config.value;
    return !!c && !!c.base_url.trim() && !!c.api_key.trim() && !!c.model.trim();
  });

  /** 当前活跃分支：从 `head` 沿 `parent` 回溯到根（显示顺序）。 */
  const activePath = computed<AgentNode[]>(() => {
    const s = session.value;
    if (!s) return [];
    const path: AgentNode[] = [];
    let cursor = s.head;
    while (cursor) {
      const node = s.nodes.find((n) => n.id === cursor);
      if (!node) break;
      path.push(node);
      cursor = node.parent;
    }
    return path.reverse();
  });

  /** 某节点的兄弟分支（同父，按创建时间升序）。用于 `‹ i/n ›` 切换。 */
  function branchesOf(nodeId: string): AgentNode[] {
    const s = session.value;
    if (!s) return [];
    const node = s.nodes.find((n) => n.id === nodeId);
    if (!node) return [];
    return s.nodes
      .filter((n) => n.parent === node.parent)
      .sort((a, b) => a.created_at - b.created_at);
  }

  // ---- 配置 ----

  async function loadConfig(): Promise<void> {
    config.value = await agentConfig().catch(() => null);
  }

  async function saveConfig(baseUrl: string, apiKey: string, model: string): Promise<void> {
    config.value = await agentSetConfig(baseUrl, apiKey, model);
  }

  async function fetchModels(baseUrl: string, apiKey: string, model: string): Promise<string[]> {
    return agentModels(baseUrl, apiKey, model);
  }

  async function test(baseUrl: string, apiKey: string, model: string): Promise<AgentProbe> {
    return agentTest(baseUrl, apiKey, model);
  }

  // ---- 会话 ----

  async function loadSessions(): Promise<void> {
    sessions.value = await agentSessions().catch(() => []);
  }

  // 运行中切会话会让流式增量落进另一条会话（事件里只有 node_id），所以先挡住。
  async function openSession(id: string): Promise<void> {
    if (running.value) return;
    session.value = await agentSession(id);
    stopped.value = false;
    resetStream();
  }

  function newSession(): void {
    if (running.value) return;
    session.value = null;
    stopped.value = false;
    resetStream();
  }

  async function removeSession(id: string): Promise<void> {
    if (running.value) return;
    await agentDeleteSession(id);
    if (session.value?.id === id) session.value = null;
    await loadSessions();
  }

  async function setHead(nodeId: string): Promise<void> {
    if (!session.value || running.value) return;
    session.value = await agentSetHead(session.value.id, nodeId);
  }

  // ---- 发送 ----

  /** 清掉「正在流式的这一条」的正文 / 思考缓冲（工具状态保留给已定稿的节点用）。 */
  function resetStreaming(): void {
    dropPending();
    streamingNodeId.value = null;
    streamingContent.value = "";
    streamingReasoning.value = "";
  }

  function resetStream(): void {
    resetStreaming();
    toolRuns.value = {};
    toolTick.value = 0;
  }

  /** 从 `parentId` 处挂一条用户输入并开始回答（`parentId` 为 `null` 时接在当前 head 之后）。 */
  async function sendFrom(parentId: string | null, text: string): Promise<void> {
    const trimmed = text.trim();
    if (!trimmed || running.value) return;
    running.value = true;
    stopping.value = false;
    stopped.value = false;
    clearStopFallback();
    resetStream();
    try {
      await agentSend(session.value?.id ?? null, parentId, trimmed);
    } catch (err) {
      running.value = false;
      layout.notify(String(err));
    }
  }

  /** 接在当前会话末尾发送。 */
  function send(text: string): Promise<void> {
    return sendFrom(session.value?.head ?? null, text);
  }

  /** 编辑某条用户输入后从此处重新回答（产生分支）。 */
  function editAndResend(nodeId: string, text: string): Promise<void> {
    const node = session.value?.nodes.find((n) => n.id === nodeId);
    return sendFrom(node?.parent ?? null, text);
  }

  /** 停止兜底定时器：后端迟迟不回声时用 `agent_running` 对齐一次。 */
  let stopFallback: ReturnType<typeof setTimeout> | null = null;

  function clearStopFallback(): void {
    if (stopFallback !== null) {
      clearTimeout(stopFallback);
      stopFallback = null;
    }
  }

  /** 本地补收尾：`turn_finished` 可能因为面板卸载而错过了。 */
  function finalizeStopped(): void {
    clearStopFallback();
    running.value = false;
    stopping.value = false;
    stopped.value = true;
    resetStream();
    void reloadSession();
    void loadSessions();
  }

  /** 从后端回读当前会话（补齐面板卸载期间错过的节点）。 */
  async function reloadSession(): Promise<void> {
    const id = session.value?.id;
    if (!id) return;
    try {
      session.value = await agentSession(id);
    } catch {
      // 会话可能已被删除：保持现状即可。
    }
  }

  async function stop(): Promise<void> {
    if (!running.value || stopping.value) return;
    // 立刻给出「正在停止」反馈——后端可能正卡在一次不可打断的渲染里。
    stopping.value = true;
    const wasRunning = await agentCancel().catch(() => null);
    // 等待期间 `turn_finished` 可能已经先到，这里不要再动手。
    if (!stopping.value) return;
    if (wasRunning === false) {
      finalizeStopped();
      return;
    }
    clearStopFallback();
    stopFallback = setTimeout(() => {
      stopFallback = null;
      if (!stopping.value) return;
      void agentRunning()
        .catch(() => null)
        .then((actual) => {
          if (!stopping.value) return;
          if (actual === true) {
            // 后端还卡在不可打断的阻塞任务里：先放开按钮，允许用户再按一次。
            stopping.value = false;
            return;
          }
          finalizeStopped();
        });
    }, 4000);
  }

  /**
   * 面板每次挂载都跟后端对齐一次：卸载期间的事件已经丢了，`running` 会永远停在
   * `true`（停止按钮看起来彻底失灵）。这里回读会话与运行状态，把本地状态拉回来。
   */
  async function syncFromBackend(): Promise<void> {
    const actual = await agentRunning().catch(() => null);
    if (actual === null) return;
    await reloadSession();
    if (actual) {
      running.value = true;
      stopping.value = false;
    } else if (running.value) {
      running.value = false;
      stopping.value = false;
      stopped.value = true;
      resetStream();
      void loadSessions();
    }
  }

  // ---- 事件 ----

  function handleEvent(event: AgentEvent): void {
    // 除正文 / 思考增量外，其余事件都带结构变化：先把攒着的文字落地，保证顺序。
    if (event.type !== "delta" && event.type !== "reasoning") flushStreaming();
    switch (event.type) {
      case "turn_started": {
        if (session.value?.id !== event.session_id) {
          session.value = {
            id: event.session_id,
            title: "",
            created_at: Date.now(),
            updated_at: Date.now(),
            nodes: [],
            head: null,
          };
        }
        const s = session.value;
        if (s && !s.nodes.some((n) => n.id === event.user_node.id)) s.nodes.push(event.user_node);
        if (s) s.head = event.user_node.id;
        running.value = true;
        stopping.value = false;
        stopped.value = false;
        clearStopFallback();
        resetStream();
        break;
      }
      case "delta":
        queueDelta(event.node_id, event.text, "content");
        break;
      case "reasoning":
        queueDelta(event.node_id, event.text, "reasoning");
        break;
      case "tool_call": {
        const runs = (toolRuns.value[event.node_id] ??= []);
        runs.push({ callId: event.call_id, name: event.name, args: event.args, ok: null, output: "" });
        toolTick.value += 1;
        break;
      }
      case "tool_result": {
        const run = toolRuns.value[event.node_id]?.find((t) => t.callId === event.call_id);
        if (run) {
          run.ok = event.ok;
          run.output = event.output;
        }
        toolTick.value += 1;
        break;
      }
      case "node_added": {
        const s = session.value;
        if (s && !s.nodes.some((n) => n.id === event.node.id)) s.nodes.push(event.node);
        if (s) s.head = event.node.id;
        // 助手节点定稿：清掉流式缓冲，后续渲染走节点本身。
        // 注意**不要**清 toolRuns：这一轮的工具还没开始跑。
        if (event.node.role === "assistant") resetStreaming();
        break;
      }
      case "turn_finished": {
        const s = session.value;
        if (s) {
          s.head = event.head;
          s.title = event.title;
          s.updated_at = Date.now();
        }
        running.value = false;
        stopping.value = false;
        stopped.value = event.stopped;
        clearStopFallback();
        resetStream();
        void loadSessions();
        break;
      }
      case "error":
        running.value = false;
        stopping.value = false;
        clearStopFallback();
        layout.notify(event.message);
        break;
    }
  }

  /**
   * 订阅智能体事件。可重入：并发 / 卸载后重挂都只会留下一个监听。
   *
   * 面板在标签页切换时会被卸载再挂载；如果第一次 `start()` 还挂在 `await` 上就
   * 被卸载、随后又挂载，旧实现会订阅两次，每个 delta 被追加两遍——用户看到的就是
   * 「文字大量重复」。
   */
  async function start(): Promise<void> {
    wanted = true;
    if (unlisten) return;
    if (subscribing) return subscribing;
    subscribing = onAgentEvent(handleEvent)
      .then((off) => {
        subscribing = null;
        if (wanted) unlisten = off;
        else off();
      })
      .catch((err: unknown) => {
        subscribing = null;
        throw err;
      });
    return subscribing;
  }

  function dispose(): void {
    wanted = false;
    clearStopFallback();
    dropPending();
    unlisten?.();
    unlisten = null;
  }

  return {
    config,
    sessions,
    session,
    running,
    stopping,
    stopped,
    streamingNodeId,
    streamingContent,
    streamingReasoning,
    toolRuns,
    toolTick,
    configured,
    activePath,
    branchesOf,
    loadConfig,
    saveConfig,
    fetchModels,
    test,
    loadSessions,
    openSession,
    newSession,
    removeSession,
    setHead,
    send,
    editAndResend,
    stop,
    start,
    syncFromBackend,
    dispose,
  };
});
