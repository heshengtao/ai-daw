#!/usr/bin/env node
/**
 * 更新随包的奏法映射快照（ArtiCluster Universal Definition，MIT）。
 *
 * 客户端只读 `src-tauri/resources/articulation-maps/`，不联网；上游只在打包 / 手动更新时拉取。
 *
 * 规则：
 * - 下载 / 解压 / 校验失败 → 保留已有快照，构建继续（退出码 0）。
 * - 只有旧快照也不存在时才失败。
 *
 * 环境变量：
 * - `ARTICULATION_MAPS_SOURCE`：换上游地址（测试 / 镜像）。
 * - `ARTICULATION_MAPS_OUT`：换落点（默认 `src-tauri/resources/articulation-maps`）。
 */

import { spawnSync } from "node:child_process";
import { createWriteStream, existsSync } from "node:fs";
import { mkdir, mkdtemp, readdir, rename, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SOURCE =
  process.env.ARTICULATION_MAPS_SOURCE?.trim() ||
  "https://codeload.github.com/r-koubou/ArticulationMappingFiles/tar.gz/refs/heads/main";
const OUT =
  process.env.ARTICULATION_MAPS_OUT?.trim() ||
  path.join(ROOT, "src-tauri", "resources", "articulation-maps");
const TIMEOUT_MS = 180_000;
const MIN_MAPS = 100;

function fail(message) {
  if (existsSync(OUT)) {
    console.warn(`[articulation-maps] ${message}`);
    console.warn(`[articulation-maps] 保留已有快照：${path.relative(ROOT, OUT)}`);
    process.exit(0);
  }
  console.error(`[articulation-maps] ${message}`);
  console.error("[articulation-maps] 没有可用的随包快照，构建无法继续。");
  process.exit(1);
}

async function countYaml(dir) {
  let count = 0;
  const entries = await readdir(dir, { withFileTypes: true }).catch(() => []);
  for (const entry of entries) {
    if (entry.isSymbolicLink()) continue;
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) count += await countYaml(full);
    else if (/\.ya?ml$/i.test(entry.name)) count += 1;
  }
  return count;
}

async function main() {
  const work = await mkdtemp(path.join(tmpdir(), "ai-daw-maps-"));
  const archive = path.join(work, "maps.tar.gz");
  const stage = path.join(work, "stage");
  await mkdir(stage, { recursive: true });

  try {
    const response = await fetch(SOURCE, {
      redirect: "follow",
      headers: { Accept: "application/gzip, application/octet-stream" },
      signal: AbortSignal.timeout(TIMEOUT_MS),
    });
    if (!response.ok || !response.body) {
      throw new Error(`HTTP ${response.status}`);
    }
    await pipeline(Readable.fromWeb(response.body), createWriteStream(archive));
  } catch (error) {
    fail(`拉取上游失败（${SOURCE}）：${error?.message ?? error}`);
  }

  const extract = spawnSync(
    "tar",
    ["-xzf", archive, "-C", stage, "--strip-components=2", "ArticulationMappingFiles-main/articluster-files"],
    { stdio: "inherit" },
  );
  if (extract.status !== 0) {
    fail(`解压映射失败（tar 退出码 ${extract.status ?? "?"}）`);
  }
  spawnSync(
    "tar",
    ["-xzf", archive, "-C", stage, "--strip-components=1", "ArticulationMappingFiles-main/LICENSE"],
    { stdio: "inherit" },
  );

  const count = await countYaml(stage);
  if (count < MIN_MAPS) {
    fail(`上游只解析到 ${count} 份映射（少于 ${MIN_MAPS}），视为异常`);
  }

  const note = [
    "ArtiCluster Universal Definition articulation maps",
    "Source: https://github.com/r-koubou/ArticulationMappingFiles",
    `Snapshot: ${new Date().toISOString()}`,
    `Maps: ${count}`,
    "",
  ].join("\n");
  await writeFile(path.join(stage, "SNAPSHOT.txt"), note, "utf8");

  const backup = `${OUT}.old`;
  await rm(backup, { recursive: true, force: true });
  const hadOld = existsSync(OUT);
  if (hadOld) await rename(OUT, backup);
  try {
    await rename(stage, OUT);
  } catch (error) {
    if (hadOld) await rename(backup, OUT).catch(() => {});
    fail(`替换快照失败：${error?.message ?? error}`);
  }
  await rm(backup, { recursive: true, force: true });

  const fileCount = (await readdir(OUT, { recursive: true })).length;
  console.log(
    `[articulation-maps] 已更新 ${path.relative(ROOT, OUT)}：${count} 份映射，${fileCount} 个文件`,
  );
}

main().catch((error) => fail(`意外错误：${error?.stack ?? error}`));
