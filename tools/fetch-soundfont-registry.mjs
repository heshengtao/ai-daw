#!/usr/bin/env node
/**
 * 更新随包的 SoundFont 音色库索引快照。
 *
 * 客户端运行时会先请求 `ai-daw-soundfont-registry` 的 `registry.json`（GitHub Pages → jsDelivr → raw
 * → 本地磁盘缓存），都拿不到时退回**随包快照**（`include_str!`），所以商店在任何网络下都能
 * 至少列出已知的几件库。
 *
 * 规则与 `fetch-plugin-registry.mjs` 一致：
 * - 拉取失败 / 内容不合法 → 保留仓库里已有的快照，构建继续（退出码 0）。
 * - 只有连旧快照都没有时才失败。
 *
 * 环境变量：
 * - `SOUNDFONT_REGISTRY_SOURCE`：换上游地址（测试 / 镜像）。
 * - `SOUNDFONT_REGISTRY_OUT`：换落点（默认 `src-tauri/resources/soundfont-registry.json`）。
 */

import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SOURCE =
  process.env.SOUNDFONT_REGISTRY_SOURCE?.trim() ||
  "https://heshengtao.github.io/ai-daw-soundfont-registry/registry.json";
const OUT =
  process.env.SOUNDFONT_REGISTRY_OUT?.trim() ||
  path.join(ROOT, "src-tauri", "resources", "soundfont-registry.json");
const TIMEOUT_MS = 120_000;

function fail(message) {
  console.error(`[soundfonts] ${message}`);
  if (existsSync(OUT)) {
    console.warn(`[soundfonts] 保留已有快照：${path.relative(ROOT, OUT)}`);
    process.exit(0);
  }
  console.error("[soundfonts] 没有可用的随包快照，构建无法继续。");
  process.exit(1);
}

function validate(text) {
  const data = JSON.parse(text);
  if (data?.schema_version !== 1) throw new Error("schema_version 不是 1");
  const banks = data?.soundfonts;
  if (!Array.isArray(banks) || banks.length === 0) throw new Error("soundfonts 为空");
  for (const bank of banks) {
    if (!bank?.slug || !bank?.name || !Array.isArray(bank?.files) || bank.files.length === 0) {
      throw new Error(`条目不完整：${bank?.slug ?? "(无 slug)"}`);
    }
  }
  return banks.length;
}

async function main() {
  let text;
  try {
    const response = await fetch(SOURCE, {
      redirect: "follow",
      headers: { Accept: "application/json" },
      signal: AbortSignal.timeout(TIMEOUT_MS),
    });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
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
    console.log(`[soundfonts] 未变化（${count} 件）：${path.relative(ROOT, OUT)}`);
    return;
  }

  const digest = createHash("sha256").update(text).digest("hex").slice(0, 12);
  await mkdir(path.dirname(OUT), { recursive: true });
  const tmp = `${OUT}.${digest}.tmp`;
  await writeFile(tmp, text, "utf8");
  await rename(tmp, OUT);
  console.log(
    `[soundfonts] 已更新 ${path.relative(ROOT, OUT)}：${count} 件，${(
      Buffer.byteLength(text) / 1024
    ).toFixed(0)} KB（sha256:${digest}）`,
  );
}

main().catch((error) => fail(`意外错误：${error?.stack ?? error}`));
