#!/usr/bin/env node
/**
 * 把模型下载源的 revision **钉到 commit**，不再跟随 `master` / `main` 浮动分支。
 *
 * 为什么：清单里的 `sha256` 是硬门禁（见 `crates/local-ai/src/models/catalog.rs` 头注释），
 * 而 `resolve/master/<file>` 的内容随上游每次 push 变化。上游换一版权重，用户的下载会在
 * 100% 处报「sha256 校验失败」，而且换源也救不了——两个源都会变。所以 revision 必须是
 * **不可变的 commit sha**。
 *
 *   node tools/pin-models.mjs            # 只检查：报告还有哪些浮动 revision、钉住的 commit 是否还能解析
 *   node tools/pin-models.mjs --write    # 把 `master` / `main` 改写成当前分支头 commit
 *   node tools/pin-models.mjs --bump     # 连已经钉住的也一起升到当前头（升级模型 = 人肉决定）
 *
 * HTTP 读不到的仓库（HF 直连在部分网络下不通）会退到 `git ls-remote`；HF 走 `HF_MIRROR`
 * 镜像做同样的解析，镜像只是通道，不改变仓库内容。
 */
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const HF_MIRROR = (process.env.HF_MIRROR ?? "https://hf-mirror.com").replace(/\/+$/, "");

/** 需要钉 revision 的文件（清单 + 开发期下载脚本，同一个 bug 两边都有）。 */
const TARGETS = [
  "crates/local-ai/src/models/catalog.rs",
  "tools/fetch-models.sh",
  "tools/mirror-soulx-hf.sh",
];

const MODE = process.argv.includes("--write")
  ? "write"
  : process.argv.includes("--bump")
    ? "bump"
    : "check";

/** `https://host/(models/)?ns/repo/resolve/<rev>/<path>` */
const URL_RE =
  /https:\/\/(?:www\.)?(modelscope\.cn|huggingface\.co)\/(models\/)?([\w.-]+)\/([\w.-]+)\/resolve\/([^/"'\s]+)\/([^"'\s]+)/g;

const SHA_REV = /^[0-9a-f]{40}$/;

/**
 * URL 里直接写死的浮动分支（含没有文件名的 BASE 前缀，例如 `…/resolve/master`）。
 * 这一路是兜底：`URL_RE` 只认「带文件名的完整 URL」，前缀式写法会漏。
 */
function floatingLines(text) {
  const hits = [];
  text.split("\n").forEach((line, index) => {
    if (/https:\/\/[^\s"']*\/resolve\/(master|main)\b/.test(line)) {
      hits.push(`${index + 1}: ${line.trim()}`);
    }
  });
  return hits;
}

/** `git ls-remote <url> <ref>` → commit sha；失败返回 null。 */
function lsRemote(url, ref) {
  try {
    const out = execFileSync("git", ["ls-remote", url, ref], {
      encoding: "utf8",
      timeout: 60_000,
      stdio: ["ignore", "pipe", "ignore"],
    });
    const line = out.split("\n").find((l) => l.trim().endsWith(ref));
    return line ? line.split("\t")[0].trim() : null;
  } catch {
    return null;
  }
}

/** 解出某个仓库默认分支的头 commit（= 今天 `master` / `main` 指向的东西）。 */
/**
 * 解出某个仓库默认分支的头 commit。
 *
 * HF 走 `/api/models/<repo>` 而不是 `git ls-remote`：仓库被重命名时会 307 到新名字（旧名字
 * 上的 `resolve/<sha>` 是 404，只有 `main` 能被重定向救——所以重命名必须连仓库名一起改），
 * 而 gated / auto-gated 的仓库 git 匿名读不到、API 元数据却是公开的。
 */
async function headCommit(key) {
  const [host, repo] = key.split("\u0000");
  if (host === "modelscope.cn") {
    const sha = lsRemote(`https://modelscope.cn/models/${repo}.git`, "refs/heads/master");
    return sha ? { sha, repo } : null;
  }
  try {
    const response = await fetch(`${HF_MIRROR}/api/models/${repo}`, {
      signal: AbortSignal.timeout(30_000),
    });
    if (response.ok) {
      const data = await response.json();
      if (data?.sha) return { sha: data.sha, repo: data.id ?? repo };
    }
  } catch {
    // 掉到 git 兜底
  }
  const sha = lsRemote(`${HF_MIRROR}/${repo}.git`, "refs/heads/main");
  return sha ? { sha, repo } : null;
}

/** 逐个（host, repo）解 commit，带缓存与并发上限。 */
async function headCommits(pairs, concurrency = 4) {
  const unique = [...pairs].sort();
  const out = new Map();
  let index = 0;
  async function worker() {
    while (index < unique.length) {
      const key = unique[index++];
      const [host, repo] = key.split("\u0000");
      const resolved = await headCommit(key);
      out.set(key, resolved);
      process.stderr.write(
        resolved
          ? `  ✓ ${host} ${repo} @ ${resolved.sha.slice(0, 8)}${resolved.repo !== repo ? `（重命名为 ${resolved.repo}）` : ""}\n`
          : `  ✗ ${host} ${repo}（解析不出 commit）\n`,
      );
    }
  }
  await Promise.all(Array.from({ length: concurrency }, worker));
  return out;
}

/** 扫一遍目标文件，收集所有 resolve 链接。 */
function scan() {
  const found = [];
  for (const rel of TARGETS) {
    const abs = join(ROOT, rel);
    let text;
    try {
      text = readFileSync(abs, "utf8");
    } catch {
      continue; // 这个仓可能没有某些脚本
    }
    for (const match of text.matchAll(URL_RE)) {
      const [full, host, modelsPrefix, ns, repo, rev, path] = match;
      found.push({
        file: rel,
        abs,
        text,
        index: match.index,
        full,
        host,
        modelsPrefix,
        repo: `${ns}/${repo}`,
        rev,
        path,
      });
    }
  }
  return found;
}

const links = scan();
const floating = links.filter((l) => !SHA_REV.test(l.rev));
const pinned = links.filter((l) => SHA_REV.test(l.rev));

console.log(
  `扫描 ${TARGETS.length} 个文件：${links.length} 条 resolve 链接，` +
    `${pinned.length} 条已钉 commit，${floating.length} 条跟随浮动分支`,
);

// check 模式下浮动链接就是失败：这正是要修的那个 bug。
const literalFloats = TARGETS.flatMap((rel) => {
  try {
    return floatingLines(readFileSync(join(ROOT, rel), "utf8")).map((line) => `${rel} → ${line}`);
  } catch {
    return [];
  }
});

if (MODE === "check") {
  for (const l of floating) {
    console.log(`  浮动：${relative(ROOT, l.abs)} ← ${l.full}`);
  }
  const bad = pinned.filter((l) => !l.rev.startsWith("0000"));
  if (pinned.length) {
    console.log(`  （已钉住的 ${bad.length} 条不在本模式下校验字节，用 tools/check-models.mjs）`);
  }
  for (const line of literalFloats) {
    console.log(`  浮动（字面 URL）：${line}`);
  }
  process.exit(floating.length || literalFloats.length ? 1 : 0);
}

// write / bump：先把要动的链接挑出来，只解这些仓库的 commit。
const todo = MODE === "bump" ? links.slice() : floating.slice();
if (!todo.length) {
  console.log("没有需要改写的链接。");
  process.exit(0);
}
console.log(`解析 ${new Set(todo.map((l) => `${l.host}\u0000${l.repo}`)).size} 个仓库的头 commit…`);
const heads = await headCommits(new Set(todo.map((l) => `${l.host}\u0000${l.repo}`)));

// 按文件聚合改写：从后往前替换，避免下标错位。
const byFile = new Map();
for (const l of todo) {
  const sha = heads.get(`${l.host}\u0000${l.repo}`);
  if (!sha) {
    console.error(`  跳过 ${l.repo}：解不出 commit，保留原样`);
    continue;
  }
  const list = byFile.get(l.abs) ?? [];
  list.push({ ...l, sha });
  byFile.set(l.abs, list);
}

const unresolved = [];
let changed = 0;
for (const [abs, list] of byFile) {
  let text = readFileSync(abs, "utf8");
  list.sort((a, b) => b.index - a.index);
  for (const l of list) {
    const before = text.slice(l.index, l.index + l.full.length);
    if (!before.startsWith(l.full)) continue; // 同名 URL 已被前一条替换过
    // 用 parts 重建：HF 仓库可能被重命名（旧名 + 新 commit = 404，必须连仓库名一起换）。
    const after =
      `https://${l.host}/${l.modelsPrefix ?? ""}${l.sha.repo}/resolve/${l.sha.sha}/${l.path}`;
    text = text.slice(0, l.index) + after + text.slice(l.index + l.full.length);
    changed += 1;
  }
  writeFileSync(abs, text);
  console.log(`  ✓ ${relative(ROOT, abs)}（${list.length} 条）`);
}
if (unresolved.length) {
  console.error(`\n有 ${unresolved.length} 条链接没能钉住：${[...new Set(unresolved)].join(", ")}`);
  process.exit(1);
}
console.log(`已钉住 ${changed} 条链接。跑一遍 node tools/check-models.mjs 校验字节。`);
