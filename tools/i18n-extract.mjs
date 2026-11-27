#!/usr/bin/env node
/**
 * 把界面词典切成翻译分片（JSON），供译者在仓库外批量翻译。
 *
 * 用法：
 *   node tools/i18n-extract.mjs                     # en → /tmp/ai-daw-i18n/en.chunk*.json
 *   node tools/i18n-extract.mjs --locale zh --out /tmp/ai-daw-i18n --chunks 4
 *
 * 为什么要切：一份词典 2500+ 条，一次性交给译者容易截断；按「同前缀尽量不拆开」切，
 * 每片保留原文、键名与占位符，翻译完再由 `i18n-merge.mjs` 合回 TS。
 */
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

function arg(name, fallback) {
  const i = process.argv.indexOf(`--${name}`);
  return i === -1 ? fallback : process.argv[i + 1];
}

/** 载入一个 TS 词典（模块里只有 `const x = {...}; export default x;`）。 */
export async function loadDict(file) {
  const source = readFileSync(file, "utf8");
  const js = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const url = `data:text/javascript;base64,${Buffer.from(js).toString("base64")}`;
  const mod = await import(url);
  return mod.default;
}

/** 只在直接执行时切分片；被 merge / check import 时不要产生副作用。 */
const isEntry = process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1];

if (isEntry) {
const locale = arg("locale", "en");
const outDir = arg("out", "/tmp/ai-daw-i18n");
const chunks = Number(arg("chunks", "4"));

const dict = await loadDict(join(root, "src/i18n", `${locale}.ts`));
const keys = Object.keys(dict);

/** 一组「同前缀」的键就是一个界面区域，切分尽量别把它劈开。 */
const prefixOf = (key) => key.split(".")[0];
const target = Math.ceil(keys.length / chunks);
const buckets = [];
let current = {};
for (let i = 0; i < keys.length; i += 1) {
  const key = keys[i];
  const nextPrefix = keys[i + 1] ? prefixOf(keys[i + 1]) : null;
  current[key] = dict[key];
  const full = Object.keys(current).length >= target;
  const boundary = nextPrefix !== prefixOf(key);
  if (full && (boundary || Object.keys(current).length >= target * 1.35)) {
    buckets.push(current);
    current = {};
  }
}
if (Object.keys(current).length) buckets.push(current);

mkdirSync(outDir, { recursive: true });
buckets.forEach((bucket, index) => {
  const file = join(outDir, `${locale}.chunk${index + 1}.json`);
  writeFileSync(file, `${JSON.stringify(bucket, null, 2)}\n`);
  console.log(`${file}  ${Object.keys(bucket).length} entries`);
});
console.log(`total ${keys.length} entries in ${buckets.length} chunks`);
}
