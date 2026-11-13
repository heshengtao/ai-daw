/**
 * 浏览器端的实时监听：把 `aidawd` 推来的 PCM 排进 WebAudio 播放。
 *
 * 协议（见 `crates/daemon/src/server/mod.rs`）：
 * `[epoch u32][start_frame u64][frames u16][channels u16][f32 × frames × channels]`，小端。
 *
 * 排播策略：每收到一块就 `AudioBufferSourceNode.start(nextTime)` 接着上一块，
 * 靠 `nextTime` 维持一条连续的播放队列（而不是每次 `currentTime`），这样块与块之间
 * 没有缝。欠载时把队列重置到「现在 + 一小段预滚」，宁可跳一下也不要卡住。
 */
import { webToken } from "./ipc";

/** 预滚：第一块开始播放前先攒这么久，吸收网络抖动。 */
const PREROLL_SECONDS = 0.08;

/** 队列落后这么多就认为断流了，重新对齐。 */
const REANCHOR_SECONDS = 0.25;

export interface MonitorStats {
  /** 已经排进去的秒数（队列深度）。 */
  bufferedSeconds: number;
  /** 当前流代数。 */
  epoch: number;
  /** 估算的端到端延迟（毫秒）。 */
  latencyMs: number;
}

/** 一块解码后的 PCM。 */
interface Block {
  epoch: number;
  startFrame: number;
  channels: number;
  samples: Float32Array;
}

/** 解析一个二进制帧；格式不对返回 `null`（只丢这一块）。 */
export function parseBlock(data: ArrayBuffer): Block | null {
  if (data.byteLength < 16) return null;
  const view = new DataView(data);
  const epoch = view.getUint32(0, true);
  const startFrame = Number(view.getBigUint64(4, true));
  const frames = view.getUint16(12, true);
  const channels = view.getUint16(14, true);
  const expected = 16 + frames * channels * 4;
  if (data.byteLength < expected || frames === 0 || channels === 0) return null;
  return {
    epoch,
    startFrame,
    channels,
    samples: new Float32Array(data.slice(16, expected)),
  };
}

/** 一条到 daemon 的监听连接。 */
export class WebMonitor {
  private ctx: AudioContext | null = null;
  private socket: WebSocket | null = null;
  private nextTime = 0;
  private epoch = -1;
  /**
   * 队列的锚点：第一块在时间轴上的帧号 + 它在 AudioContext 时钟里的播放时刻。
   *
   * 播放头由「锚点 + 上下文时钟已走过的秒数」推算，而不是数已收帧数——
   * 后者会把网络抖动算进播放位置，界面上的播放头就会一跳一跳。
   */
  private anchorFrame = 0;
  private anchorTime = 0;
  private consumed = 0;
  private queued = 0;
  private active = false;

  /** 当前播放头（帧）。 */
  get positionFrames(): number {
    const ctx = this.ctx;
    if (!ctx || this.anchorTime === 0) return this.anchorFrame;
    const elapsed = Math.max(0, ctx.currentTime - this.anchorTime);
    return this.anchorFrame + Math.round(elapsed * ctx.sampleRate);
  }

  /** 队列深度与延迟估计，供界面显示缓冲状态。 */
  get stats(): MonitorStats {
    const ctx = this.ctx;
    const latency = ctx ? Math.max(0, (this.nextTime - ctx.currentTime) * 1000) : 0;
    return { bufferedSeconds: this.queued, epoch: this.epoch, latencyMs: latency };
  }

  /** 开始监听。重复调用是幂等的。 */
  async start(): Promise<void> {
    if (this.active) return;
    this.active = true;
    const ctx = new AudioContext({ latencyHint: "interactive" });
    this.ctx = ctx;
    // iOS Safari 会挂起 AudioContext（后台 / 锁屏 / 来电），必须显式恢复。
    if (ctx.state === "suspended") await ctx.resume();
    const url = new URL(window.location.href);
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    url.pathname = "/audio";
    url.search = `?token=${encodeURIComponent(webToken())}`;
    url.hash = "";
    const socket = new WebSocket(url.toString());
    socket.binaryType = "arraybuffer";
    socket.onmessage = (event) => {
      if (event.data instanceof ArrayBuffer) this.enqueue(event.data);
    };
    this.socket = socket;
    this.nextTime = 0;
    this.epoch = -1;
    this.consumed = 0;
    this.anchorFrame = 0;
    this.anchorTime = 0;
    this.queued = 0;
  }

  /** 停止监听并释放音频上下文。 */
  stop(): void {
    this.active = false;
    this.socket?.close();
    this.socket = null;
    void this.ctx?.close();
    this.ctx = null;
    this.nextTime = 0;
    this.queued = 0;
  }

  /** 页面回到前台时恢复上下文（Safari 会把它挂起）。 */
  async resume(): Promise<void> {
    if (this.ctx && this.ctx.state === "suspended") await this.ctx.resume();
  }

  private enqueue(data: ArrayBuffer): void {
    const ctx = this.ctx;
    if (!ctx) return;
    const block = parseBlock(data);
    if (!block) return;
    // 流代数变了（播放器重建）：旧队列作废，重新对齐。
    if (block.epoch !== this.epoch) {
      this.epoch = block.epoch;
      this.nextTime = 0;
      this.consumed = 0;
      this.anchorTime = 0;
    }
    const frames = block.samples.length / block.channels;
    if (frames === 0) return;
    const buffer = ctx.createBuffer(2, frames, ctx.sampleRate);
    const left = buffer.getChannelData(0);
    const right = buffer.getChannelData(1);
    for (let i = 0; i < frames; i += 1) {
      left[i] = block.samples[i * block.channels];
      right[i] = block.channels > 1 ? block.samples[i * block.channels + 1] : left[i];
    }
    const at = this.nextTime === 0 ? ctx.currentTime + PREROLL_SECONDS : this.nextTime;
    if (at < ctx.currentTime - REANCHOR_SECONDS) {
      // 队列落后太多（切后台 / 卡顿）：丢掉旧的，从「现在 + 预滚」重新开始。
      this.nextTime = ctx.currentTime + PREROLL_SECONDS;
    }
    const source = ctx.createBufferSource();
    source.buffer = buffer;
    source.connect(ctx.destination);
    const startAt = Math.max(at, ctx.currentTime);
    source.start(startAt);
    this.nextTime = startAt + buffer.duration;
    this.queued = Math.max(0, this.nextTime - ctx.currentTime);
    this.consumed += frames;
    if (this.anchorTime === 0) {
      // 队列刚开始：这一刻就是「流里的 startFrame」对应的时钟时刻。
      this.anchorFrame = block.startFrame;
      this.anchorTime = startAt;
    }
  }
}
