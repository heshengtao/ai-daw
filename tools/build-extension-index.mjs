#!/usr/bin/env node
/**
 * 扩展市场索引用具：读一份 npm 包名清单，产出 `index.json`（format 1）。
 *
 * 用法（在仓库根目录）：
 *
 *   node tools/build-extension-index.mjs \
 *     --input plugins.txt \
 *     --output ../ai-daw-plugins/index.json \
 *     --title "AI DAW Extensions"
 *
 * 清单格式：一行一个 npm 包名，`#` 开头与空行忽略。生成器**只读** registry 元数据，
 * 从不下载或执行包的代码。
 *
 * 只收录声明了 `engines.aidaw`、且版本可解析的包（与读取器的硬要求一致：缺声明 = 不兼容）。
 */

import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";

const args = process.argv.slice(2);
function arg(name, fallback) {
  const index = args.indexOf(`--${name}`);
  if (index === -1) return fallback;
  const value = args[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`--${name} 缺少值`);
  return value;
}

const input = resolve(arg("input", "plugins.txt"));
const output = resolve(arg("output", "index.json"));
const title = arg("title", "AI DAW Extensions");
const description = arg("description", "Community extensions for AI DAW");
const registry = arg("registry", "https://registry.npmjs.org").replace(/\/$/, "");

const lines = readFileSync(input, "utf8")
  .split("\n")
  .map((line) => line.trim())
  .filter((line) => line.length > 0 && !line.startsWith("#"));

const plugins = [];
let failures = 0;

for (const name of lines) {
  try {
    const response = await fetch(`${registry}/${encodeURIComponent(name).replace("%2F", "/")}`, {
      headers: { accept: "application/json" },
    });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const doc = await response.json();
    const time = doc.time ?? {};
    const versions = [];

    for (const [version, meta] of Object.entries(doc.versions ?? {})) {
      if (meta.deprecated) continue;
      const range = meta.engines?.aidaw;
      if (typeof range !== "string" || range.trim() === "") {
        console.warn(`  ! ${name}@${version} 没有 engines.aidaw，跳过`);
        continue;
      }
      const published = Date.parse(time[version] ?? "");
      versions.push({
        version,
        publishTimestamp: Number.isFinite(published) ? Math.floor(published / 1000) : 0,
        source: { type: "npm" },
        engines: { aidaw: range.trim() },
      });
    }

    if (versions.length === 0) throw new Error("没有任何声明 engines.aidaw 的版本");

    const listed = new Set(versions.map((item) => item.version));
    const distTags = {};
    for (const [tag, target] of Object.entries(doc["dist-tags"] ?? {})) {
      if (listed.has(target)) distTags[tag] = target;
    }

    const latest = doc.versions?.[doc["dist-tags"]?.latest] ?? {};
    const entry = {
      name: doc.name ?? name,
      title: doc.displayName ?? doc.name ?? name,
      description: latest.description ?? doc.description ?? undefined,
      author: typeof doc.author === "string" ? doc.author.replace(/\s*<[^>]*>/g, "").trim() : doc.author?.name,
      homepage: /^https?:/.test(latest.homepage ?? doc.homepage ?? "") ? latest.homepage ?? doc.homepage : undefined,
      metadata: {
        keywords: Array.isArray(latest.keywords ?? doc.keywords) ? latest.keywords ?? doc.keywords : [],
      },
      ...(Object.keys(distTags).length > 0 ? { "dist-tags": distTags } : {}),
      versions,
    };
    plugins.push(clean(entry));
    console.log(`  ✓ ${name}（${versions.length} 个版本）`);
  } catch (error) {
    failures += 1;
    console.error(`  ✗ ${name}: ${error.message}`);
  }
}

function clean(value) {
  if (Array.isArray(value)) return value.map(clean);
  if (value && typeof value === "object") {
    const out = {};
    for (const [key, item] of Object.entries(value)) {
      if (item === undefined) continue;
      out[key] = clean(item);
    }
    return out;
  }
  return value;
}

if (plugins.length === 0 && failures > 0) {
  console.error("没有生成任何条目：所有源都失败了。");
  process.exit(1);
}

mkdirSync(dirname(output), { recursive: true });
writeFileSync(
  output,
  `${JSON.stringify({ format: 1, title, description, plugins }, null, 2)}\n`,
  "utf8",
);
console.log(`写入 ${output}：${plugins.length} 个扩展${failures ? `，${failures} 个源失败` : ""}`);
process.exit(failures > 0 ? 2 : 0);
