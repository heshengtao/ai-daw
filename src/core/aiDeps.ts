/**
 * **依赖解析**：某个 AI 功能要跑起来，还差哪些东西、一共多少字节。
 *
 * 三层依赖，按「先引擎 / 运行时、后权重」排序（下载顺序即数组顺序）：
 * 1. `engine`   —— audio.cpp 的引擎包（写歌 / 歌声合成 / 音色替换）；
 * 2. `runtime`  —— ONNX Runtime 本体（CUDA 12 / 13）；
 * 3. `component`—— CUDA 的四件套（cudart / cuBLAS / cuFFT / cuDNN）；
 * 4. `model`    —— 权重。
 *
 * 体积必须算**真实要下的量**：CUDA 本体一条命令会顺带补齐同版本的全部组件
 * （后端 `install_parts`），所以「本体没装」时只报本体 + 组件合计的**一个**数字，
 * 不能再把四个组件重复列一遍，否则总量会翻倍。
 */
import type { ModelStatus, RuntimeStatus } from "@/api/types";
import type { SetupBackend, SetupDep, SetupPlan } from "@/types/setup";
import { humanSize, missingBytes } from "@/utils/models";

/** 一个 audio.cpp 引擎包 → 一条依赖。 */
export function engineDep(pack: RuntimeStatus, label: string): SetupDep {
  return {
    kind: "engine",
    id: pack.id,
    label,
    bytes: pack.installed ? 0 : pack.bytes,
    installed: pack.installed,
    usable: pack.usable,
  };
}

/** 一个 ONNX 运行时本体 → 一条依赖（组件跟着本体一起下）。 */
export function runtimeDep(item: RuntimeStatus, label: string, parts: RuntimeStatus[]): SetupDep {
  const missing = parts.filter((part) => !part.installed);
  return {
    kind: "runtime",
    id: item.id,
    label,
    // 真下多少：本体 + 缺的组件。装好了就报 0，别把「以后可能补」的算进本次预算。
    bytes: item.installed ? 0 : item.bytes + missing.reduce((sum, part) => sum + part.bytes, 0),
    installed: item.installed,
    usable: true,
    hint: missing.length > 0 ? String(missing.length) : undefined,
  };
}

/** 一个模型 → 一条依赖。 */
export function modelDep(model: ModelStatus): SetupDep {
  return {
    kind: "model",
    id: model.id,
    label: model.name,
    bytes: missingBytes(model),
    installed: model.installed,
    usable: true,
    hint: model.total_files > 1 ? String(model.total_files) : undefined,
  };
}

/** 一组依赖里总共还差多少字节。 */
export function totalBytes(deps: SetupDep[]): number {
  return deps.reduce((sum, dep) => (dep.installed ? sum : sum + dep.bytes), 0);
}

/**
 * 挑「本机最该装的那一份」引擎。
 *
 * 顺序很关键：**推荐但没装**优于「已装但不是最优」——否则全新 N 卡机器会默认落到
 * CPU 兜底那份，用户以为软件不支持显卡（AI Cut 的 `pickDefault` 踩过这个坑）。
 */
export function pickRecommendedEngine(packs: RuntimeStatus[]): RuntimeStatus | null {
  const usable = packs.filter((pack) => pack.usable);
  return (
    usable.find((pack) => pack.recommended) ??
    usable.find((pack) => pack.installed && pack.selected) ??
    usable.find((pack) => pack.installed) ??
    usable.find((pack) => pack.core) ??
    usable[0] ??
    null
  );
}

/** 引擎「为什么推荐它」的一句话（文案走 i18n，缺词条时退回后端那句 note）。 */
export function engineReason(
  t: (key: string, params?: Record<string, string | number>) => string,
  pack: RuntimeStatus | null,
): string {
  if (!pack) return t("setup.reason.generic");
  const device = pack.id.startsWith("audiocpp-") ? pack.id.slice("audiocpp-".length) : "";
  const key = `setup.reason.${device || "generic"}`;
  const text = t(key);
  return text === key ? pack.note || t("setup.reason.generic") : text;
}

/** 把一个引擎包 + 一批模型收成一份计划（`feature` 决定文案与记忆键）。 */
export function buildPlan(options: {
  feature: string;
  title: string;
  lead: (backendLabel: string, reason: string) => string;
  engine: RuntimeStatus | null;
  engineLabel: string;
  reason: string;
  choices: { kind: string; label: string }[];
  models: ModelStatus[];
  restartAfter?: boolean;
}): SetupPlan | null {
  const deps: SetupDep[] = [];
  if (options.engine && !options.engine.installed) {
    deps.push(engineDep(options.engine, options.engineLabel));
  }
  for (const model of options.models) {
    if (!model.installed) deps.push(modelDep(model));
  }
  if (deps.length === 0) return null;

  const backend: SetupBackend | null = options.engine
    ? {
        kind: options.engine.id,
        label: options.engineLabel,
        reason: options.reason,
        recommended: options.engine.recommended,
        choices: options.choices,
      }
    : null;

  return {
    feature: options.feature,
    title: options.title,
    lead: options.lead(options.engineLabel, options.reason),
    deps,
    totalBytes: totalBytes(deps),
    backend,
    restartAfter: options.restartAfter ?? false,
  };
}

/** 「合计需要下载 7.09 GB」这类摘要（0 字节时给空串，调用方自行决定要不要显示）。 */
export function sizeSummary(bytes: number): string {
  return bytes > 0 ? humanSize(bytes) : "";
}
