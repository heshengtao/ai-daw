/**
 * **「准备运行环境」计划**：一个功能要跑起来，还差哪些东西。
 *
 * 为什么需要这一层：以前「缺引擎」「缺权重」「缺 CUDA 组件」各弹各的框，用户点一次确认
 * 只解决一项；写歌这种「引擎 + 几 GB 权重」的场景要连点两三次，且中间没有任何地方能看到
 * 「总共还要下多少」。这里把依赖收成一份**可预算、可排序、可一次下完**的计划。
 *
 * 与 AI Cut 的 `types/setup.ts` 同一套结构（两边共用同一种体验），只是 kind 按本项目的
 * 三层依赖取名：audio.cpp 引擎 / ONNX 运行时 / CUDA 组件 / 权重。
 */

/** 依赖种类（下载顺序即数组顺序：先引擎 / 运行时，后权重）。 */
export type SetupDepKind = "engine" | "runtime" | "component" | "model";

/** 计划里的一行。 */
export interface SetupDep {
  kind: SetupDepKind;
  /** 引擎包 id / 运行时 id / 模型 id。 */
  id: string;
  /** 界面文案用的名字。 */
  label: string;
  /** **还差多少字节**（已装为 0）。 */
  bytes: number;
  /** 是否已经在位（在位也列出来，让用户看到「还差什么」）。 */
  installed: boolean;
  /** 这台机器能不能用；`false` 的行不渲染（例如 A 卡机器上的 CUDA）。 */
  usable: boolean;
  /** 副标题：运行时给「N 个组件」，模型给「N 个权重文件」。 */
  hint?: string;
}

/** 可切换的后端选项。 */
export interface SetupBackendChoice {
  kind: string;
  label: string;
}

/** 推荐后端 + 依据 + 可切换项。 */
export interface SetupBackend {
  kind: string;
  label: string;
  /** 「检测到 NVIDIA GeForce RTX 4060」这类依据文案。 */
  reason: string;
  recommended: boolean;
  choices: SetupBackendChoice[];
}

/** 一份「准备什么」的计划。 */
export interface SetupPlan {
  /** 记忆键（`yue` / `singing` / `svc` / `runtime`）。 */
  feature: string;
  title: string;
  /** 一句话说明：这台机器适合什么、为什么。 */
  lead: string;
  deps: SetupDep[];
  /** 合计还要下多少字节（已装的不计）。 */
  totalBytes: number;
  backend: SetupBackend | null;
  /** 装完是否必须重启才生效（ONNX 运行时是；audio.cpp 引擎不是）。 */
  restartAfter: boolean;
}

/** 计划里「真正要下」的那几项（已装的剔除，供队列使用）。 */
export function pendingDeps(plan: SetupPlan): SetupDep[] {
  return plan.deps.filter((dep) => !dep.installed && dep.bytes > 0);
}
