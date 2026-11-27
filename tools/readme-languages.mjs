#!/usr/bin/env node
/**
 * 统一维护 README 的语言切换条（10 种语言）。
 *
 * 用法：
 *   node tools/readme-languages.mjs          # 写入（缺文件会报出来）
 *   node tools/readme-languages.mjs --check  # 只检查，不改文件
 *
 * 切换条里当前语言加粗、其余是普通链接；每个 README 的切换条由这个脚本统一生成，
 * 免得手改 10 份时漏掉一门语言。
 */
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const checkOnly = process.argv.includes("--check");

/** 顺序即切换条顺序。 */
const LANGS = [
  { id: "zh", label: "简体中文", file: "README_zh.md" },
  { id: "zh-TW", label: "繁體中文", file: "README_zh-TW.md" },
  { id: "en", label: "English", file: "README.md" },
  { id: "ja", label: "日本語", file: "README_ja.md" },
  { id: "ko", label: "한국어", file: "README_ko.md" },
  { id: "es", label: "Español", file: "README_es.md" },
  { id: "fr", label: "Français", file: "README_fr.md" },
  { id: "de", label: "Deutsch", file: "README_de.md" },
  { id: "ru", label: "Русский", file: "README_ru.md" },
  { id: "ar", label: "العربية", file: "README_ar.md" },
];

/** 生成某一门语言页面里的切换条。 */
function block(currentId) {
  const parts = LANGS.map((lang) => {
    const label = lang.id === currentId ? `<b>${lang.label}</b>` : lang.label;
    return `<a href="${lang.file}">${label}</a>`;
  });
  const lines = [];
  for (let i = 0; i < parts.length; i += 4) lines.push(parts.slice(i, i + 4).join(" · "));
  return `<p align="center">\n  ${lines.join(" ·\n  ")}\n</p>`;
}

/** 找到已有的切换条：第一个包含 README_zh.md 的 `<p align="center">` 块。 */
function findBlock(text) {
  let from = 0;
  while (true) {
    const start = text.indexOf('<p align="center">', from);
    if (start === -1) return null;
    const end = text.indexOf("</p>", start);
    if (end === -1) return null;
    const chunk = text.slice(start, end + 4);
    if (chunk.includes("README_zh.md") && chunk.includes("README.md")) return { start, end: end + 4 };
    from = end + 4;
  }
}

let problems = 0;
for (const lang of LANGS) {
  const path = join(root, lang.file);
  if (!existsSync(path)) {
    console.log(`missing ${lang.file}`);
    problems += 1;
    continue;
  }
  const text = readFileSync(path, "utf8");
  const spot = findBlock(text);
  if (!spot) {
    console.log(`${lang.file}: 没有找到语言切换条`);
    problems += 1;
    continue;
  }
  const next = `${text.slice(0, spot.start)}${block(lang.id)}${text.slice(spot.end)}`;
  const links = [...next.matchAll(/href="(README[^"]*\.md)"/g)].map((m) => m[1]);
  const missing = LANGS.map((l) => l.file).filter((f) => !links.includes(f));
  if (missing.length) {
    console.log(`${lang.file}: 缺链接 ${missing.join(", ")}`);
    problems += 1;
  }
  if (checkOnly) continue;
  if (next !== text) {
    writeFileSync(path, next);
    console.log(`updated ${lang.file}`);
  }
}
if (checkOnly && !problems) console.log("✓ 10 份 README 的语言切换条一致");
if (problems) process.exit(1);
