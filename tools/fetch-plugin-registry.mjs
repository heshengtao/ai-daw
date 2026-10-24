#!/usr/bin/env node
/**
 * 打包前更新插件注册表快照。
 *
 * 客户端只请求 ai-daw 仓库里的这份 JSON（raw 链接），不再每次直连上游
 * Open Audio Stack。上游只在这里被请求一次——打包时。
 *
 * 规则：
 * - 拉取失败 / 内容不合法 → 保留仓库里已有的快照，构建继续（退出码 0）。
 * - 只有连旧快照都没有时才失败（否则 `include_str!` 编不过）。
 *
 * 环境变量：
 * - `PLUGIN_REGISTRY_SOURCE`：换上游地址（测试 / 镜像）。
 * - `PLUGIN_REGISTRY_OUT`：换落点（默认 `src-tauri/resources/plugin-registry.json`）。
 */

import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SOURCE =
  process.env.PLUGIN_REGISTRY_SOURCE?.trim() ||
  "https://open-audio-stack.github.io/open-audio-stack-registry/";
const OUT =
  process.env.PLUGIN_REGISTRY_OUT?.trim() ||
  path.join(ROOT, "src-tauri", "resources", "plugin-registry.json");
const TIMEOUT_MS = 180_000;

function fail(message) {
  console.error(`[registry] ${message}`);
  if (existsSync(OUT)) {
    console.warn(`[registry] 保留已有快照：${path.relative(ROOT, OUT)}`);
    process.exit(0);
  }
  console.error("[registry] 没有可用的随包快照，构建无法继续。");
  process.exit(1);
}

function validate(text) {
  const data = JSON.parse(text);
  const plugins = data?.plugins;
  if (!plugins || typeof plugins !== "object" || Array.isArray(plugins)) {
    throw new Error("JSON 里没有 plugins 对象");
  }
  const count = Object.keys(plugins).length;
  if (count === 0) {
    throw new Error("plugins 为空");
  }
  return count;
}

async function main() {
  let text;
  try {
    const response = await fetch(SOURCE, {
      redirect: "follow",
      headers: { Accept: "application/json" },
      signal: AbortSignal.timeout(TIMEOUT_MS),
    });
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}`);
    }
    text = await response.text();
  } catch (error) {
    fail(`拉取上游失败（${SOURCE}）：${error?.message ?? error}`);
  }

  let count;
  try {
    count = validate(text);
  } catch (error) {
    fail(`上游内容不合法（${SOURCE}）：${error.message}`);
  }

  const previous = existsSync(OUT) ? await readFile(OUT, "utf8").catch(() => null) : null;
  if (previous === text) {
    console.log(`[registry] 未变化（${count} 个插件）：${path.relative(ROOT, OUT)}`);
    return;
  }

  const digest = createHash("sha256").update(text).digest("hex").slice(0, 12);
  await mkdir(path.dirname(OUT), { recursive: true });
  const tmp = `${OUT}.${digest}.tmp`;
  await writeFile(tmp, text, "utf8");
  await rename(tmp, OUT);
  console.log(
    `[registry] 已更新 ${path.relative(ROOT, OUT)}：${count} 个插件，${(
      Buffer.byteLength(text) / 1024
    ).toFixed(0)} KB（sha256:${digest}）`,
  );
}

main().catch((error) => fail(`意外错误：${error?.stack ?? error}`));
