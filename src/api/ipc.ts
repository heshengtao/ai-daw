/**
 * IPC 传输层：同一份前端代码，两种后端。
 *
 * - **桌面（Tauri）**：`invoke` / `listen` 直连 webview 与 Rust。
 * - **浏览器**：`POST /rpc/<cmd>` 与 `GET /events`（SSE），连的是同一台机器上的
 *   `aidawd`（或桌面进程开的那条 HTTP 服务）。
 *
 * 之所以做成"同名同签名的两个函数"而不是给 237 个调用点各加分支：命令名与参数名
 * 本来就与 Rust 一一对应（见 `src/api/tauri.ts` 的文件头约定），换传输层不该改动调用方。
 *
 * 服务端**不鉴权**：浏览器打开端口就能用；访问控制由部署侧（局域网 / 网关）负责。
 */
import { isTauri } from "@/utils/platform";

/** `invoke` 允许的参数形状（与 Tauri 的约定一致）。 */
export type InvokeArgs = Record<string, unknown>;

/** 取消监听。 */
export type UnlistenFn = () => void;

/** SSE 上的事件名（与后端 `Event::default().event("engine")` 对应）。 */
const SSE_EVENT = "engine";

async function httpInvoke<T>(cmd: string, args?: InvokeArgs): Promise<T> {
  const response = await fetch(`/rpc/${encodeURIComponent(cmd)}`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
    },
    body: JSON.stringify(args ?? {}),
  });
  if (!response.ok) {
    // 后端的错误形状是 `{ "error": "..." }`，与桌面端的字符串错误保持一致。
    let message = `${response.status} ${response.statusText}`;
    try {
      const body = (await response.json()) as { error?: string };
      if (body?.error) message = body.error;
    } catch {
      /* 非 JSON 错误体：保留状态码 */
    }
    throw message;
  }
  return (await response.json()) as T;
}

/** 调用一条后端命令。 */
export async function invoke<T>(cmd: string, args?: InvokeArgs): Promise<T> {
  if (isTauri()) {
    const { invoke: tauriInvoke } = await import("@tauri-apps/api/core");
    return tauriInvoke<T>(cmd, args);
  }
  return httpInvoke<T>(cmd, args);
}

/* -------------------------------------------------------------------------- */
/* 事件：桌面走 Tauri event，浏览器走一条共享的 SSE 连接                        */
/* -------------------------------------------------------------------------- */

type Handler = (payload: unknown) => void;

const handlers = new Map<string, Set<Handler>>();
let source: EventSource | null = null;

function ensureSource(): EventSource | null {
  if (source) return source;
  if (typeof window === "undefined" || typeof EventSource === "undefined") return null;
  // 服务不鉴权：直接连事件流。
  source = new EventSource("/events");
  source.addEventListener(SSE_EVENT, (event) => {
    try {
      const frame = JSON.parse((event as MessageEvent<string>).data) as {
        event: string;
        payload: unknown;
      };
      handlers.get(frame.event)?.forEach((handler) => handler(frame.payload));
    } catch {
      /* 坏帧只丢这一条 */
    }
  });
  return source;
}

/** 订阅一条事件通道；返回取消函数。 */
export async function listen<T>(
  event: string,
  handler: (event: { payload: T }) => void,
): Promise<UnlistenFn> {
  if (isTauri()) {
    const { listen: tauriListen } = await import("@tauri-apps/api/event");
    return tauriListen<T>(event, handler);
  }
  const wrapped: Handler = (payload) => handler({ payload: payload as T });
  const set = handlers.get(event) ?? new Set<Handler>();
  set.add(wrapped);
  handlers.set(event, set);
  ensureSource();
  return () => {
    set.delete(wrapped);
    if (set.size === 0) handlers.delete(event);
  };
}
