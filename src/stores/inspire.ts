/**
 * **灵感库**：生成过的歌 + 每首歌的试听播放器。
 *
 * 两个职责放一起是有意的——列表里每条记录的唯一动作就是「播放」，播放状态与列表是一份
 * 状态机（同时只有一首在播、换一首自动停上一首、拖动进度条 = 从这个位置开始试听）。
 * 拆成两个 store 只会让「谁在播」变成要跨 store 同步的东西。
 *
 * **列表只记「在灵感页点出来的」生成**：本地生成靠 `engine://yue-job` 的 `done` 事件收尾，
 * 云端靠 `generate()` 的返回值收尾。提交时抓的提示词 / 歌词 / 模型存在 `craft` 里，
 * 完成时与结果配对落库——所以即使生成途中切去编排页，回来照样看得到这一首。
 *
 * 音频播放**不走工程走带**：走带是「整首工程、所有轨道」，而这里要的是「单独听这一首」。
 * 用 `read_import_file` 把音频字节读进来塞给 `<audio>`，与工程的播放互不干扰（工程一开播
 * 就把试听停掉，免得两路声音叠在一起）。
 *
 * 但**灵感页里**底部栏那颗播放键要能当遥控器用：`focusedId` 记住当前选中的那张卡，
 * 页面在时底部播放键控制它、图标状态也跟着它变；切走由 `leaveView` 解除同步。
 */
import { computed, ref, watch } from "vue";
import { defineStore } from "pinia";
import { onYueJobEvent, readImportFile, yueImportAudio, yueRecord, yueSaveAudio } from "@/api/tauri";
import type { YueComposeArgs, YueJobStatus } from "@/api/types";
import { useCloudMusicStore } from "@/stores/cloudMusic";
import { useI18n } from "@/i18n";
import { useProjectStore } from "@/stores/project";
import { useTransportStore } from "@/stores/transport";
import { useYueStore } from "@/stores/yue";
import { pickAudioToSave } from "@/api/dialogs";
import { downloadFromServer } from "@/utils/download";
import { isTauri } from "@/utils/platform";
import { trackHue } from "@/utils/trackColor";

const KEY = "ai-daw-inspire";
/** 列表上限：再老的记录只占内存和 localStorage，用户不会翻到那么远。 */
const MAX_ITEMS = 50;
/** 一批最多几首：再多只是排队更久，用户通常也就挑 2–4 首里的一条。 */
export const MAX_BATCH = 8;
/** 同时缓存的音频 Blob URL 上限（每个可能几十 MB，不能无限攒）。 */
const MAX_URLS = 24;

/** 一条生成记录。 */
export interface InspireItem {
  id: string;
  /** 标题：歌词首句 → 风格描述前几个字。 */
  title: string;
  /** 风格 / 场景描述（提交时那份）。 */
  prompt: string;
  lyrics: string;
  /** 徽章文字：本地是权重档名，云端是「服务 · 模型」。 */
  model: string;
  source: "local" | "cloud";
  /** 落地的音频文件路径。 */
  path: string;
  /** 时长（秒）；`0` = 还不知道（播放时以 `<audio>` 的元数据为准）。 */
  seconds: number;
  createdAt: number;
  /** 生成参数存档（`.json`）：续写要读它的 style / model。 */
  record?: string | null;
  /** 语义码（`.semantic.json`）：续写的**唯一**输入（不是音频）。 */
  semantic?: string | null;
}

/** 提交一次生成要带的东西。 */
export interface CraftRequest {
  prompt: string;
  lyrics: string;
  /** 徽章文字（进列表用）。 */
  model: string;
  source: "local" | "cloud";
  /** 目标时长（秒）。 */
  duration: number;
  /** 本地：权重 id；云端：服务 id。 */
  target: string | null;
  /** 本地：流匹配步数。 */
  steps?: number;
  /** 本地：种子（-1 = 随机）。 */
  seed?: number;
  /** 纯音乐：云端是显式开关；本地 = 歌词留空 + 后端的两段式。 */
  instrumental?: boolean;
  /** 续写：拿这首的语义码当开头（`semantic_prefix_file`）。 */
  semanticPrefix?: string | null;
  /** 这是一次续写（只影响标题，别让人以为它是新写的一首）。 */
  continuation?: boolean;
}

/** 正在跑的那一次生成（骨架行与落库都靠它）。 */
interface Craft {
  token: number;
  request: CraftRequest;
  startedAt: number;
}

function loadItems(): InspireItem[] {
  try {
    const raw = JSON.parse(localStorage.getItem(KEY) ?? "[]") as unknown;
    if (!Array.isArray(raw)) return [];
    return raw
      .filter((x): x is InspireItem => {
        if (!x || typeof x !== "object") return false;
        const item = x as Partial<InspireItem>;
        return typeof item.id === "string" && typeof item.path === "string";
      })
      .slice(0, MAX_ITEMS)
      .map((item) => ({
        ...item,
        title: item.title || "",
        prompt: item.prompt || "",
        lyrics: item.lyrics || "",
        model: item.model || "",
        source: item.source === "cloud" ? "cloud" : "local",
        seconds: Number(item.seconds) || 0,
        createdAt: Number(item.createdAt) || Date.now(),
        record: typeof item.record === "string" ? item.record : null,
        semantic: typeof item.semantic === "string" ? item.semantic : null,
      }));
  } catch {
    return [];
  }
}

/**
 * 标题：歌词第一条**不是结构标签**的行 → 风格描述前 24 个字。
 *
 * 与 SUNO 一样拿首句当歌名，用户不用先想名字；`[Verse]` / `[Chorus]` 这种行要跳过，
 * 否则列表里会排出一串 `[Verse]`。
 */
function titleOf(request: CraftRequest, fallback: string): string {
  for (const line of request.lyrics.split("\n")) {
    const text = line.trim();
    if (!text || /^\[[^\]]+\]$/.test(text)) continue;
    return text.length > 24 ? `${text.slice(0, 24)}…` : text;
  }
  const prompt = request.prompt.trim();
  if (prompt) return prompt.length > 24 ? `${prompt.slice(0, 24)}…` : prompt;
  return fallback;
}

/** 音频文件的 MIME：显式给出类型，WKWebView / WebView2 才不会因为 sniff 失败而当成不可播。 */
function mimeOf(path: string): string {
  const ext = path.slice(path.lastIndexOf(".") + 1).toLowerCase();
  switch (ext) {
    case "mp3":
      return "audio/mpeg";
    case "flac":
      return "audio/flac";
    case "ogg":
    case "opus":
      return "audio/ogg";
    case "m4a":
    case "aac":
      return "audio/mp4";
    case "mp4":
      return "audio/mp4";
    default:
      return "audio/wav";
  }
}

function clamp01(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}

export const useInspireStore = defineStore("inspire", () => {
  const i18n = useI18n();
  const project = useProjectStore();
  const transport = useTransportStore();
  const yue = useYueStore();
  const cloudMusic = useCloudMusicStore();

  const items = ref<InspireItem[]>(loadItems());
  const error = ref<string | null>(null);

  // ---- 在跑的那一次生成 ----
  const craft = ref<Craft | null>(null);
  const elapsed = ref(0);
  let token = 0;
  let ticker: number | null = null;
  let stopJob: (() => void) | null = null;
  /** 这一首结束（成功 / 失败 / 取消）时兑现：批处理靠它一首一首往下走。 */
  let settleCraft: (() => void) | null = null;
  let craftDone: Promise<void> = Promise.resolve();

  /** 一批生成：总数与已完成数；单首生成时是 `null`。 */
  const batch = ref<{ total: number; done: number } | null>(null);
  /** 用户点了停止：当前这首照常取消，后面的不再开始。 */
  let batchStopped = false;

  const crafting = computed(() => craft.value !== null || batch.value !== null);

  /** 进度百分比：云端有真实进度，本地只知道「在跑」（返回 `null` = 不定长进度条）。 */
  const progressPct = computed(() =>
    craft.value?.request.source === "cloud" ? cloudMusic.progressPct : null,
  );

  /** 生成阶段的说明文字（骨架行与状态行共用）。 */
  const craftLine = computed(() => {
    if (!craft.value) return error.value ?? "";
    if (craft.value.request.source === "cloud") {
      return cloudMusic.progress?.message || i18n.t("inspire.cloudBusy");
    }
    const engine = yue.engine;
    if (engine?.state === "starting") return engine.message ?? i18n.t("inspire.engineLoading");
    return i18n.t("inspire.localBusy");
  });

  /** 骨架行的标题：和落库时同一套规则（歌词首句 → 描述），所以「出来以后」不会换名字。 */
  const craftTitle = computed(() =>
    craft.value ? titleOf(craft.value.request, i18n.t("inspire.untitled")) : "",
  );

  // ---- 播放（同一时刻只有一首） ----
  const playingId = ref<string | null>(null);
  const loadingId = ref<string | null>(null);
  /** 已经装进 `<audio>` 的那一首（与 playingId 分开：暂停后仍然「装着」，再点就接着放）。 */
  const loadedId = ref<string | null>(null);
  const position = ref(0);
  const duration = ref(0);
  const playError = ref<string | null>(null);

  // ---- 焦点（灵感页里底部播放键控制的那一首） ----
  /**
   * 当前选中（有焦点）的记录 id。
   *
   * 灵感页的卡片、以及底部栏那颗播放键，是**同一份状态的两个入口**：点卡片里的播放键
   * 会选中这张卡，底部播放键跟着变成它的播放 / 暂停；反过来点底部播放键也只动这一首。
   * 它只属于「此刻页面上的选择」，所以不持久化，切走时由 `leaveView` 清掉。
   */
  const focusedId = ref<string | null>(null);
  const focusedItem = computed(
    () => items.value.find((item) => item.id === focusedId.value) ?? null,
  );
  /** 焦点卡是不是正在响——底部播放键的图标 / 光晕直接读它。 */
  const focusedPlaying = computed(
    () => focusedId.value !== null && playingId.value === focusedId.value,
  );

  let el: HTMLAudioElement | null = null;
  let raf = 0;
  const urls = new Map<string, string>();

  function persist(): void {
    try {
      localStorage.setItem(KEY, JSON.stringify(items.value.slice(0, MAX_ITEMS)));
    } catch {
      // 存不下（配额 / 隐私模式）不影响本次会话。
    }
  }

  watch(items, persist, { deep: true });

  // ---- 列表 ----

  /** 新记录插到最前；超过上限的挤掉最老的（连它的 Blob URL 一起回收）。 */
  function push(item: InspireItem): void {
    items.value = [item, ...items.value].slice(0, MAX_ITEMS);
    // 被挤掉的正好是焦点卡时，焦点跟着失效，别让底部播放键指向一条不在列表里的记录。
    if (focusedId.value && !items.value.some((entry) => entry.id === focusedId.value)) {
      focusedId.value = null;
    }
    const keep = new Set(items.value.map((i) => i.path));
    for (const path of [...urls.keys()]) {
      if (!keep.has(path)) dropUrl(path);
    }
  }

  /** 把播放器从这条记录上摘下来（记录被删 / 列表被清空时用）。 */
  function forget(id: string): void {
    if (loadedId.value !== id) return;
    stopPlayback();
    el = null;
    loadedId.value = null;
    duration.value = 0;
  }

  /** 移除一条记录（**只删列表，不动工程里的轨道与音频文件**）。 */
  function remove(id: string): void {
    const item = items.value.find((i) => i.id === id);
    if (!item) return;
    forget(id);
    items.value = items.value.filter((i) => i.id !== id);
    // 删掉的正是焦点卡：焦点交给下一条，底部播放键不会悬空。
    if (focusedId.value === id) focusedId.value = items.value[0]?.id ?? null;
    if (!items.value.some((i) => i.path === item.path)) dropUrl(item.path);
  }

  /** 清空记录（同样只删列表）。 */
  function clear(): void {
    stopPlayback();
    el = null;
    loadedId.value = null;
    duration.value = 0;
    focusedId.value = null;
    items.value = [];
    for (const path of [...urls.keys()]) dropUrl(path);
  }

  /**
   * 这在工程里的第几条轨道（没有就是 -1）。
   *
   * 列表的轨道配色、以及「导入到工程」按钮的状态都靠它——真相在工程里，不在记录里：
   * 轨道被删掉，按钮就该自己变回「未导入」。
   */
  function trackIndexOf(item: InspireItem): number {
    const tracks = project.project?.tracks ?? [];
    return tracks.findIndex((track) =>
      track.clips.some((clip) => clip.audio?.path === item.path),
    );
  }

  /** 这首歌是否已经在工程里（决定卡片按钮显示「导入」还是「已导入」）。 */
  function placedOf(item: InspireItem): boolean {
    return trackIndexOf(item) >= 0;
  }

  /**
   * 这条记录该用哪个色相：优先跟**工程里那条轨道**一致（列表与编排页配色对得上），
   * 轨道已经被删 / 工程还没同步过来时，按 id 散列一个稳定色。
   */
  function hueOf(item: InspireItem): number {
    const index = trackIndexOf(item);
    if (index >= 0) return trackHue(index);
    let hash = 0;
    for (let i = 0; i < item.id.length; i += 1) hash = (hash * 31 + item.id.charCodeAt(i)) % 997;
    return trackHue(hash);
  }

  // ---- 播放 ----

  function dropUrl(path: string): void {
    const url = urls.get(path);
    if (!url) return;
    URL.revokeObjectURL(url);
    urls.delete(path);
  }

  function cancelRaf(): void {
    if (raf !== 0) {
      cancelAnimationFrame(raf);
      raf = 0;
    }
  }

  /** 按 `currentTime` 推进播放头：`timeupdate` 只有 4 Hz，进度条会一顿一顿。 */
  function startRaf(): void {
    cancelRaf();
    const tick = (): void => {
      if (!el) return;
      position.value = el.currentTime;
      if (Number.isFinite(el.duration) && el.duration > 0) duration.value = el.duration;
      if (!el.paused && !el.ended) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
  }

  /** 取（或造）这条记录的音频 URL：同一个文件只读一次盘。 */
  async function urlOf(item: InspireItem): Promise<string> {
    const cached = urls.get(item.path);
    if (cached) return cached;
    const bytes = await readImportFile(item.path);
    if (!bytes || bytes.length === 0) throw new Error(i18n.t("inspire.audioEmpty"));
    // 拷进独立的 `ArrayBuffer`：`read_import_file` 回来的是共享缓冲的视图，直接塞进
    // `Blob` 会被当成 `SharedArrayBuffer` 而类型不合法（并且播放器拿到的是可变的同一块内存）。
    const buffer = new ArrayBuffer(bytes.byteLength);
    new Uint8Array(buffer).set(bytes);
    const url = URL.createObjectURL(new Blob([buffer], { type: mimeOf(item.path) }));
    urls.set(item.path, url);
    // 超出上限就把最早的那几个放掉（`Map` 保持插入顺序）。
    // 正在播放的那一条不能放：URL 一撤销，声音当场断掉。
    const playingPath = items.value.find((entry) => entry.id === loadedId.value)?.path;
    while (urls.size > MAX_URLS) {
      const oldest = urls.keys().next().value as string | undefined;
      if (!oldest || oldest === item.path || oldest === playingPath) break;
      dropUrl(oldest);
    }
    return url;
  }

  /** 把某一首装进播放器；失败时返回 `null`（错误已写进 `playError`）。 */
  async function ensure(item: InspireItem): Promise<HTMLAudioElement | null> {
    if (loadedId.value === item.id && el) return el;
    loadingId.value = item.id;
    playError.value = null;
    try {
      const url = await urlOf(item);
      if (el) {
        el.pause();
        el.onended = null;
        el.onloadedmetadata = null;
        el.onerror = null;
      }
      const next = new Audio(url);
      next.preload = "auto";
      next.onended = () => {
        playingId.value = null;
        position.value = 0;
        if (next) next.currentTime = 0;
      };
      next.onloadedmetadata = () => {
        if (Number.isFinite(next.duration) && next.duration > 0) duration.value = next.duration;
      };
      // `timeupdate` 是**兜底**：rAF 在后台标签页 / 隐藏窗口里会被浏览器停掉，
      // 只靠它的话进度条会冻住而声音还在走。4 Hz 够用，rAF 负责顺滑。
      next.ontimeupdate = () => {
        if (Number.isFinite(next.currentTime)) position.value = next.currentTime;
        if (Number.isFinite(next.duration) && next.duration > 0) duration.value = next.duration;
      };
      next.onerror = () => {
        playError.value = i18n.t("inspire.audioFailed");
        playingId.value = null;
      };
      el = next;
      loadedId.value = item.id;
      duration.value = item.seconds;
      position.value = 0;
      return next;
    } catch (err) {
      playError.value = String(err instanceof Error ? err.message : err);
      loadedId.value = null;
      return null;
    } finally {
      loadingId.value = null;
    }
  }

  /**
   * `play()` 被浏览器拦下时给一句人话。
   *
   * 直接摊开 `NotAllowedError` 的原文（还带一个 google 链接）对用户毫无意义；
   * 真正会发生的情形只有两种：页面还没被点过（自动播放策略），或音频轨道被系统静音。
   */
  function describePlayError(err: unknown): string {
    const name = err instanceof Error ? err.name : "";
    if (name === "NotAllowedError") return i18n.t("inspire.audioBlocked");
    return String(err instanceof Error ? err.message : err);
  }

  /** 选中一张卡（点卡片、点播放、拖进度都会走到这里）。 */
  function focusItem(item: InspireItem): void {
    // 记录可能刚被删掉（删除按钮的 click 会先冒泡到卡片）：别把焦点钉在一条不存在的记录上。
    if (!items.value.some((entry) => entry.id === item.id)) return;
    if (focusedId.value === item.id) return;
    focusedId.value = item.id;
    // 换到另一张卡就停掉正在试听的那一首：焦点卡与底部播放键必须是同一首，
    // 否则按钮显示「暂停」而声音来自别的卡，状态就对不上了。
    if (playingId.value && playingId.value !== item.id) stopPlayback();
  }

  /** 进入灵感页 / 列表变化时：焦点为空或已失效就选第一条，底部播放键才有明确对象。 */
  function ensureFocused(): void {
    if (focusedId.value && items.value.some((item) => item.id === focusedId.value)) return;
    focusedId.value = items.value[0]?.id ?? null;
  }

  /** 底部播放键在灵感页的动作：控制焦点卡（没有焦点就落到第一条）。 */
  async function toggleFocused(): Promise<void> {
    const item = focusedItem.value ?? items.value[0];
    if (item) await toggle(item);
  }

  /** 离开灵感页：解除同步（停试听 + 清焦点），底部播放键回到工程走带。 */
  function leaveView(): void {
    stopPlayback();
    focusedId.value = null;
  }

  /**
   * 试听开始前停掉工程走带：两路声音叠在一起谁也听不清。
   *
   * 反向的那一半在文件末尾的 watch 里（工程一开播就停试听）——两个方向都要管，
   * 否则从编排页带着播放状态切到灵感页时，点一下卡片就会变成两首歌同时响。
   */
  async function quietTransport(): Promise<void> {
    if (!transport.playing) return;
    try {
      await transport.stop();
    } catch {
      // 停不下来也不该拦住试听：引擎的错误由传输层自己提示。
    }
  }

  /** 播放 / 暂停这一首。 */
  async function toggle(item: InspireItem): Promise<void> {
    focusItem(item);
    // 正在读盘：重复点击不再开第二条读取（封面播放键与底部播放键都可能被点到）。
    if (loadingId.value === item.id) return;
    if (playingId.value === item.id) {
      el?.pause();
      playingId.value = null;
      cancelRaf();
      return;
    }
    await quietTransport();
    const audio = await ensure(item);
    if (!audio) return;
    try {
      await audio.play();
      playingId.value = item.id;
      startRaf();
    } catch (err) {
      playError.value = describePlayError(err);
      playingId.value = null;
    }
  }

  /** 从某个比例处开始试听（拖动进度条）。没在播的先播起来，符合「拖着听」的直觉。 */
  async function seekTo(item: InspireItem, ratio: number): Promise<void> {
    focusItem(item);
    const audio = await ensure(item);
    if (!audio) return;
    const total = Number.isFinite(audio.duration) && audio.duration > 0 ? audio.duration : item.seconds;
    const target = clamp01(ratio) * (total || 0);
    if (playingId.value !== item.id) {
      try {
        await quietTransport();
        await audio.play();
        playingId.value = item.id;
        startRaf();
      } catch (err) {
        playError.value = describePlayError(err);
        return;
      }
    }
    audio.currentTime = Math.max(0, Math.min(total || target, target));
    position.value = audio.currentTime;
  }

  /** 停掉试听（换歌、删歌、离开页面、工程开播都走它）。 */
  function stopPlayback(): void {
    cancelRaf();
    if (el) {
      el.pause();
      el.currentTime = 0;
    }
    playingId.value = null;
    position.value = 0;
  }

  // 工程走带一开播就把试听停掉：两路声音叠在一起，谁也听不清。
  watch(
    () => transport.playing,
    (playing) => {
      if (playing && playingId.value) stopPlayback();
    },
  );

  // ---- 生成 ----

  function beginCraft(request: CraftRequest): number {
    token += 1;
    error.value = null;
    craft.value = { token, request, startedAt: Date.now() };
    elapsed.value = 0;
    settleCraft = null;
    craftDone = new Promise<void>((resolve) => {
      settleCraft = resolve;
    });
    if (ticker !== null) window.clearInterval(ticker);
    ticker = window.setInterval(() => {
      if (craft.value) elapsed.value = Math.round((Date.now() - craft.value.startedAt) / 1000);
    }, 500);
    return token;
  }

  function settle(): void {
    if (ticker !== null) {
      window.clearInterval(ticker);
      ticker = null;
    }
    if (stopJob) {
      stopJob();
      stopJob = null;
    }
    craft.value = null;
    elapsed.value = 0;
    // 兑现「这一首结束了」：批处理在这个 await 上接着跑下一首。
    const done = settleCraft;
    settleCraft = null;
    done?.();
  }

  function record(
    request: CraftRequest,
    path: string,
    seconds: number,
    model: string,
    sidecars?: { record?: string | null; semantic?: string | null },
  ): void {
    const title = titleOf(request, i18n.t("inspire.untitled"));
    push({
      id: `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      title: request.continuation ? i18n.t("inspire.continueTitle", { title }) : title,
      prompt: request.prompt.trim(),
      lyrics: request.instrumental ? "" : request.lyrics,
      model: model || request.model,
      source: request.source,
      path,
      seconds: Number(seconds) || 0,
      createdAt: Date.now(),
      record: sidecars?.record ?? null,
      semantic: sidecars?.semantic ?? null,
    });
  }

  /** 本地任务的事件收尾：完成 / 失败 / 取消都由后端推过来。 */
  function onLocalJob(status: YueJobStatus, mine: number): void {
    if (token !== mine) return;
    if (status.state === "running" || status.state === "idle") return;
    const request = craft.value?.request;
    if (status.state === "done") {
      settle();
      if (!request) return;
      if (!status.audio) {
        error.value = i18n.t("inspire.noAudio");
        return;
      }
      record(request, status.audio, status.seconds ?? request.duration, request.model, {
        record: status.record,
        semantic: status.semantic,
      });
      return;
    }
    if (status.state === "failed") {
      const message = status.message ?? yue.error ?? i18n.t("inspire.failed");
      settle();
      error.value = message;
      return;
    }
    // cancelled：用户自己停的，不报错。
    settle();
  }

  /**
   * 请求体里的风格串：纯音乐补一个 `instrumental`。
   *
   * 补在这里而不是提交时改 `prompt`，是为了让列表标题/描述保持用户原话，
   * 不被一个技术性后缀污染。
   *
   * **ACE-Step 不补**：它的器乐路径就是「歌词留空」，风格里再塞否定式提示会被规划器
   * 的 CoT 重写带偏（实测纯音乐开关打开时反而「乱弹」）。给它保持和「不勾纯音乐 + 歌词
   * 留空」完全一样的请求，才是它听感正常的那一版。
   */
  function styleOf(request: CraftRequest): string {
    const style = request.prompt.trim();
    if (!request.instrumental) return style;
    if (/^ace-step-/.test(request.target ?? "")) return style;
    // 纯器乐：风格里明确写「不要人声」。光把歌词清空不够——YuE2 会自己给人声部安排旋律，
    // 风格条件写清楚才稳（YuE2 Studio 的做法也是把器乐意图写进风格）。
    if (!style) return "instrumental, no vocals";
    return /\binstrumental\b/i.test(style) ? style : `${style}, instrumental, no vocals`;
  }

  /**
   * 本地生成：引擎与权重**必须已经在位**（由调用方先过门槛），这里只管跑。
   *
   * 返回时这一首**已经结束**（成功 / 失败 / 取消都算）：批处理要靠它一首一首串行。
   */
  async function craftLocal(request: CraftRequest): Promise<void> {
    // 只看「有没有这一首在跑」，不看 `crafting`：批处理期间 `crafting` 会一直为真。
    if (craft.value) return;
    const mine = beginCraft(request);
    const args: YueComposeArgs = {
      style: styleOf(request),
      lyrics: request.instrumental ? "" : request.lyrics,
      duration: request.duration,
      steps: request.steps,
      seed: request.seed,
      stop_after: "audio",
      export_semantic: true,
      model: request.target,
      after_track: null,
      // 灵感页只负责「先听歌」：渲染结果只落盘，**不碰工程**。
      // 要进编排页，得用户点卡片上的「导入」——那才是「我要它」的表达。
      place_in_project: false,
      // 纯音乐：后端会把整份歌词换成 `[instrumental]` 标签块。只把歌词清空是不够的——
      // 模型照样会给人声部安排旋律，标签块才压得住。
      instrumental: request.instrumental === true,
      // 续写：只吃语义码（不是音频），后端会强制走 cot=off 接着往下长。
      semantic_prefix_file: request.semanticPrefix ?? null,
    };
    try {
      stopJob = await onYueJobEvent((status) => onLocalJob(status, mine));
    } catch (err) {
      // 订阅失败（老版本桌面端没有这条命令 / IPC 异常）：别让按钮一直转。
      settle();
      error.value = String(err instanceof Error ? err.message : err);
      return;
    }
    // `compose()` 内部吞掉提交错误（写进 yue.error + job.state），所以提交完再看一眼。
    await yue.compose(args);
    if (token !== mine || !craft.value) return;
    if (yue.job.state === "failed") {
      const message = yue.job.message ?? yue.error ?? i18n.t("inspire.failed");
      settle();
      error.value = message;
      return;
    }
    // 提交就报错（引擎没起 / 权重不对）时任务不会真的跑：把骨架行收掉，别让它永远转。
    if (yue.job.state === "idle") {
      settle();
      error.value = yue.error ?? i18n.t("inspire.failed");
      return;
    }
    // 引擎在后台跑：等到这一首真的结束（done / failed / 取消）再放批处理往下走。
    await craftDone;
  }

  /** 云端生成：`generate()` 等到成品落盘才返回，所以结果直接落库。 */
  async function craftCloud(request: CraftRequest): Promise<void> {
    if (craft.value) return;
    const mine = beginCraft(request);
    const providerId = request.target;
    if (!providerId) {
      settle();
      error.value = i18n.t("ai.cloudMusicNeedsProvider");
      return;
    }
    const result = await cloudMusic.generate({
      provider_id: providerId,
      prompt: request.prompt,
      lyrics: request.instrumental ? "" : request.lyrics,
      duration_seconds: request.duration,
      instrumental: request.instrumental ?? false,
    });
    if (token !== mine) return;
    settle();
    if (!result) {
      error.value = cloudMusic.error ?? i18n.t("inspire.failed");
      return;
    }
    const model = request.model || `${result.provider} · ${result.model}`;
    record(request, result.path, result.duration, model);
  }

  /**
   * 一次提交 **1–N 首**（进阶选项里的「数量」）。
   *
   * 串行跑，不并发：本地是一次引擎请求（同一份权重、同一块显存），云端是一次计费调用，
   * 同时发 N 个只会互相抢资源 / 触发限流。每首之间换种子——固定种子往后加 1，
   * `-1`（随机）保持不变——所以同一条提示词能拿到 N 个不同的 take。
   *
   * 中途点「停止」：当前这首照常取消，后面的不再开始；已经生成好的留在列表里。
   */
  async function craftBatch(request: CraftRequest, count: number): Promise<void> {
    const total = Math.min(MAX_BATCH, Math.max(1, Math.trunc(Number(count) || 1)));
    if (crafting.value) return;
    if (total <= 1) {
      await (request.source === "cloud" ? craftCloud(request) : craftLocal(request));
      return;
    }
    batchStopped = false;
    batch.value = { total, done: 0 };
    try {
      for (let index = 0; index < total; index += 1) {
        if (batchStopped) break;
        const seed =
          typeof request.seed === "number" && request.seed >= 0 ? request.seed + index : request.seed;
        const one: CraftRequest = { ...request, seed };
        await (request.source === "cloud" ? craftCloud(one) : craftLocal(one));
        if (batch.value) batch.value.done = index + 1;
      }
    } finally {
      batch.value = null;
      batchStopped = false;
    }
  }

  /** 停掉正在跑的那次生成（本地 / 云端各走各的取消）。批处理里等于「这首停下、不再开下一首」。 */
  async function cancelCraft(): Promise<void> {
    batchStopped = true;
    const current = craft.value;
    if (!current) return;
    // 先抬 token：取消后即使后端还推一条 done 过来，也不会被当成这次的结果。
    token += 1;
    if (current.request.source === "cloud") await cloudMusic.cancel();
    else await yue.cancel();
    settle();
  }

  /** 正在导入的那一条（按钮转圈用）。 */
  const importingId = ref<string | null>(null);

  /**
   * 把这条记录导入工程（新建一条音频轨）。
   *
   * 失败时把错误写进 `error` 并返回 `null`——卡片状态不动，用户还能再点一次。
   */
  async function importToProject(item: InspireItem): Promise<string | null> {
    if (importingId.value) return null;
    importingId.value = item.id;
    try {
      return await yueImportAudio(item.path, item.title);
    } catch (err) {
      error.value = String(err);
      return null;
    } finally {
      importingId.value = null;
    }
  }

  /** 清掉上一次的错误（切走 / 重新提交时）。 */
  function clearError(): void {
    error.value = null;
  }

  // ---- 下载到本地 ----

  /** 文件名：标题去掉不能出现在路径里的字符，最多 60 字。 */
  function fileNameOf(item: InspireItem): string {
    const base = (item.title || "").replace(/[\\/:*?"<>|]/g, " ").replace(/\s+/g, " ").trim();
    return `${(base || "take").slice(0, 60)}.wav`;
  }

  /** 桌面端默认落到「下载」目录；浏览器端只给文件名（没有对话框）。 */
  async function defaultSavePath(name: string): Promise<string> {
    if (!isTauri()) return name;
    try {
      const { downloadDir, join } = await import("@tauri-apps/api/path");
      return await join(await downloadDir(), name);
    } catch {
      return name;
    }
  }

  /**
   * **下载到本地**：桌面端走系统保存对话框，浏览器端落到数据目录的 `exports/` 再下载。
   *
   * 返回最终落地的绝对路径；用户取消返回 `null`（不当成错误）。
   */
  async function saveToDisk(item: InspireItem): Promise<string | null> {
    error.value = null;
    try {
      const dest = await pickAudioToSave(await defaultSavePath(fileNameOf(item)));
      if (dest === null) return null;
      const saved = await yueSaveAudio(item.path, dest);
      // 浏览器客户端没有保存对话框（dest 为空）：后端落到 exports/，这里走 /download 拉下来。
      if (!dest) await downloadFromServer(saved);
      return saved;
    } catch (err) {
      error.value = String(err);
      return null;
    }
  }

  // ---- 续写 ----
  /**
   * 一条记录的旁挂文件。
   *
   * 新记录直接把路径存下来了；老记录（这次改动之前生成的）只能按同名规则推：
   * `xxx.wav` 旁边就是 `xxx.json` 与 `xxx.semantic.json`。云端结果没有这两样。
   */
  function sidecarsOf(item: InspireItem): { record: string | null; semantic: string | null } {
    if (item.source !== "local") return { record: null, semantic: null };
    const stem = item.path.replace(/\.[^./\\]+$/, "");
    return {
      record: item.record ?? `${stem}.json`,
      semantic: item.semantic ?? `${stem}.semantic.json`,
    };
  }

  /** 这首能不能续写（本地生成 + 有语义码）。 */
  function canContinue(item: InspireItem): boolean {
    return item.source === "local";
  }

  /** 引擎没起就先拉起来（续写同样要它），最多等 15 分钟。 */
  async function ensureEngine(model: string | null): Promise<boolean> {
    if (yue.ready) return true;
    try {
      await yue.start(model ?? undefined);
    } catch (err) {
      error.value = String(err);
      return false;
    }
    const started = Date.now();
    while (Date.now() - started < 15 * 60_000) {
      if (yue.ready) return true;
      if (yue.engine?.state === "error") {
        error.value = yue.engine.message ?? i18n.t("inspire.failed");
        return false;
      }
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
    error.value = i18n.t("inspire.engineTimeout");
    return false;
  }

  /**
   * **续写**：拿这首的语义码当开头，接着往下写（不碰工程）。
   *
   * 续写的条件（风格 / 模型 / 是否纯器乐）从落盘的生成记录里读——不沿用它们，接出来的
   * 段落会像另一首歌。`extend` 是「再加多长」（秒，界面上由 `ContinueDialog` 问），
   * 总长 = 原曲 + 加长，夹在引擎的单次上限 600 秒内。
   */
  async function craftContinuation(item: InspireItem, extend = 60): Promise<void> {
    if (crafting.value) return;
    error.value = null;
    const { record, semantic } = sidecarsOf(item);
    if (!record || !semantic) {
      error.value = i18n.t("inspire.continueNeedsCode");
      return;
    }
    let saved: { args?: Record<string, unknown> } | null = null;
    try {
      saved = JSON.parse(await yueRecord(record)) as { args?: Record<string, unknown> };
    } catch {
      error.value = i18n.t("inspire.continueNeedsCode");
      return;
    }
    const args = (saved?.args ?? {}) as {
      style?: string;
      steps?: number;
      model?: string;
      instrumental?: boolean;
    };
    const model = args.model ?? null;
    if (!(await ensureEngine(model))) return;
    const base = item.seconds > 0 ? item.seconds : 60;
    const total = Math.min(600, Math.max(15, Math.round(base + extend)));
    await craftLocal({
      prompt: String(args.style ?? item.prompt ?? ""),
      lyrics: "",
      model: item.model,
      source: "local",
      duration: total,
      target: model,
      steps: typeof args.steps === "number" ? args.steps : 32,
      seed: -1,
      instrumental: args.instrumental === true,
      semanticPrefix: semantic,
      continuation: true,
    });
  }

  return {
    // 列表
    items,
    remove,
    clear,
    hueOf,
    placedOf,
    // 生成
    craft,
    crafting,
    batch,
    craftTitle,
    elapsed,
    progressPct,
    craftLine,
    error,
    craftLocal,
    craftCloud,
    craftBatch,
    cancelCraft,
    clearError,
    // 续写
    canContinue,
    craftContinuation,
    // 下载到本地
    saveToDisk,
    // 导入工程
    importingId,
    importToProject,
    // 播放
    playingId,
    loadingId,
    loadedId,
    position,
    duration,
    playError,
    toggle,
    seekTo,
    stopPlayback,
    // 焦点（灵感页 ↔ 底部播放键）
    focusedId,
    focusedItem,
    focusedPlaying,
    focusItem,
    ensureFocused,
    toggleFocused,
    leaveView,
  };
});
