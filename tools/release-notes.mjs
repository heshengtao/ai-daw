#!/usr/bin/env node
/**
 * 从 git 历史生成发布页的「更新日志」。
 *
 * 用法：
 *   node tools/release-notes.mjs --tag v1.5.2                 # 打印到 stdout
 *   node tools/release-notes.mjs --tag v1.5.2 --out notes.md   # 写到文件
 *
 * 版本区间是「上一个 tag 到目标 tag」；没有上一个 tag 时取该 tag 的完整历史。
 * 按 Conventional Commits 的 type 分组（feat / fix / perf …），没写 type 的进「其他」。
 * 「chore: 版本升至 x.y.z」这类发版提交会被跳过——发布页上它们没有信息量。
 *
 * Release workflow 在 tag 推送后调用本脚本，生成的正文同时用于 Release 与 latest.json 的 notes。
 */
import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";

const args = process.argv.slice(2);

/** 读 `--name value` 形式的参数。 */
function arg(name, fallback = "") {
  const i = args.indexOf(`--${name}`);
  return i >= 0 && i + 1 < args.length ? args[i + 1] : fallback;
}

const tag = arg("tag", process.env.GITHUB_REF_NAME ?? "");
const repo = arg("repo", process.env.GITHUB_REPOSITORY ?? "heshengtao/ai-daw");
const out = arg("out", "");

if (!tag) {
  console.error("缺少 --tag（或环境变量 GITHUB_REF_NAME）");
  process.exit(1);
}

/** 跑一条 git 命令；失败返回空字符串。 */
function git(...argv) {
  try {
    return execFileSync("git", argv, { encoding: "utf8" }).trim();
  } catch {
    return "";
  }
}

/** 目标 tag 之前最近的一个 tag；找不到返回空串（= 从头开始）。 */
function previousTag(target) {
  const described = git("describe", "--tags", "--abbrev=0", "--match", "v*", `${target}^`);
  if (described && described !== target) return described;
  // 目标 tag 是仓库里的第一个版本时没有可比区间。
  return "";
}

const previous = previousTag(tag);
const range = previous ? `${previous}..${tag}` : tag;

/** `%x1f` 分隔字段、`%x1e` 分隔提交，避免提交正文里的换行干扰解析。 */
const raw = git("log", "--no-merges", "--pretty=format:%H%x1f%h%x1f%s%x1f%b%x1e", range);

/** 提交类型 → 分组（顺序即发布页上的顺序）。 */
const GROUPS = [
  { type: "feat", title: "✨ 新功能 · New features" },
  { type: "fix", title: "🐞 修复 · Fixes" },
  { type: "perf", title: "⚡ 性能 · Performance" },
  { type: "refactor", title: "♻️ 重构 · Refactoring" },
  { type: "docs", title: "📝 文档 · Documentation" },
  { type: "style", title: "💅 样式 · Style" },
  { type: "test", title: "✅ 测试 · Tests" },
  { type: "chore", title: "🧰 维护 · Maintenance" },
  { type: "build", title: "🧰 维护 · Maintenance" },
  { type: "ci", title: "🧰 维护 · Maintenance" },
  { type: "other", title: "📦 其他 · Other" },
];

const CONVENTIONAL = /^([a-zA-Z]+)(?:\(([^)]*)\))?!?:\s*(.+)$/;
/** 发版提交：发布页上只重复版本号，跳过。 */
const VERSION_BUMP = /^chore(?:\([^)]*\))?!?:\s*(?:版本升至|bump (?:the )?version)/i;

const groups = new Map(GROUPS.map((g) => [g.type, []]));
let total = 0;

for (const block of raw.split("\x1e")) {
  // git 会在 `%x1e` 之后插入一个换行再开始下一条提交，先把区块首尾的空白去掉。
  const [hash, short, subject, body = ""] = block.trim().split("\x1f");
  if (!hash || !subject) continue;
  if (VERSION_BUMP.test(subject)) continue;

  const match = CONVENTIONAL.exec(subject);
  const type = match ? match[1].toLowerCase() : "other";
  const scope = match?.[2] ?? "";
  const text = match ? match[3] : subject;
  const key = groups.has(type) ? type : "other";

  // 正文里保留作者写的补充说明（去掉各种 trailer），让「完整更新日志」不只是一行标题。
  const details = body
    .split("\n")
    .map((line) => line.trimEnd())
    .filter((line) => line && !/^(co-authored-by|signed-off-by|reviewed-by|refs?):/i.test(line));

  groups.get(key).push({ hash, short, scope, text, details });
  total += 1;
}

const lines = ["## 更新内容", ""];

for (const { type, title } of GROUPS) {
  const items = groups.get(type);
  if (items.length === 0) continue;
  lines.push(`### ${title}`, "");
  for (const item of items) {
    const scope = item.scope ? `**${item.scope}**: ` : "";
    const link = `https://github.com/${repo}/commit/${item.hash}`;
    lines.push(`- ${scope}${item.text} ([\`${item.short}\`](${link}))`);
    for (const detail of item.details) {
      lines.push(`  ${detail}`);
    }
  }
  lines.push("");
}

if (total === 0) {
  lines.push("本次发布没有可列出的提交。", "");
}

lines.push(
  previous
    ? `**完整变更**：https://github.com/${repo}/compare/${previous}...${tag}`
    : `**完整变更**：https://github.com/${repo}/commits/${tag}`,
  "",
);

const markdown = lines.join("\n");

if (out) {
  writeFileSync(out, markdown, "utf8");
  console.error(`已写入 ${out}（${total} 条提交，区间 ${previous || "(初始)"}..${tag}）`);
} else {
  process.stdout.write(markdown);
}
