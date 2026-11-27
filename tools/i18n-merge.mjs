#!/usr/bin/env node
/**
 * 把译好的分片 JSON 合回 `src/i18n/<locale>.ts`。
 *
 * 用法：`node tools/i18n-merge.mjs ja ko`（不给参数 = 合并 out 目录里所有语言）。
 *
 * 合并前会校验：键集合与英文完全一致、占位符一致、没有空值。任何一项不过就退出，
 * 不会写出半成品词典。
 */
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { loadDict } from "./i18n-extract.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = process.argv.includes("--out")
  ? process.argv[process.argv.indexOf("--out") + 1]
  : "/tmp/ai-daw-i18n/out";

/** 语言 → 词典变量名。 */
const VARS = {
  "zh-TW": "zhTW",
  ja: "ja",
  ko: "ko",
  es: "es",
  fr: "fr",
  de: "de",
  ru: "ru",
  ar: "ar",
};
const NAMES = {
  "zh-TW": "Traditional Chinese (Taiwan)",
  ja: "Japanese",
  ko: "Korean",
  es: "Spanish",
  fr: "French",
  de: "German",
  ru: "Russian",
  ar: "Arabic",
};

const placeholders = (value) =>
  [...String(value).matchAll(/\{[a-zA-Z0-9_]+\}/g)].map((m) => m[0]).sort().join(",");

const en = await loadDict(join(root, "src/i18n/en.ts"));
const enKeys = Object.keys(en);

const only = process.argv.slice(2).filter((a) => !a.startsWith("--") && a !== outDir);
const locales = only.length
  ? only
  : readdirSync(outDir)
      .filter((f) => f.endsWith(".chunk1.json"))
      .map((f) => f.replace(".chunk1.json", ""))
      .filter((id) => VARS[id]);

if (!locales.length) {
  console.error(`没有可合并的词典（${outDir}）`);
  process.exit(1);
}

for (const locale of locales) {
  const files = readdirSync(outDir)
    .filter((f) => f.startsWith(`${locale}.chunk`) && f.endsWith(".json"))
    .sort((a, b) => Number(a.match(/chunk(\d+)/)[1]) - Number(b.match(/chunk(\d+)/)[1]));
  if (!files.length) {
    console.error(`${locale}: 没有分片`);
    process.exitCode = 1;
    continue;
  }
  const merged = {};
  for (const file of files) {
    const part = JSON.parse(readFileSync(join(outDir, file), "utf8"));
    for (const [key, value] of Object.entries(part)) {
      if (key in merged) {
        console.error(`${locale}: 键重复 ${key}（${file}）`);
        process.exit(1);
      }
      merged[key] = value;
    }
  }

  const missing = enKeys.filter((k) => !(k in merged));
  const extra = Object.keys(merged).filter((k) => !(k in en));
  const bad = enKeys.filter((k) => (k in merged) && placeholders(en[k]) !== placeholders(merged[k]));
  const empty = enKeys.filter((k) => (k in merged) && !String(merged[k]).trim());
  if (missing.length || extra.length || bad.length || empty.length) {
    console.error(
      `${locale}: 校验失败 缺 ${missing.length} / 多 ${extra.length} / 占位符 ${bad.length} / 空 ${empty.length}`,
    );
    for (const k of [...missing.slice(0, 5), ...extra.slice(0, 5), ...bad.slice(0, 5), ...empty.slice(0, 5)])
      console.error(`  ${k}`);
    process.exitCode = 1;
    continue;
  }

  const lines = enKeys.map((key) => `  ${JSON.stringify(key)}: ${JSON.stringify(merged[key])},`);
  const body = `/**
 * ${NAMES[locale]} UI dictionary.
 *
 * 键与 \`en.ts\` 一一对应、顺序一致；缺词时运行时回退到英文（见 \`index.ts\`）。
 * 改文案请直接改这里，不要动键名；占位符 \`{name}\` 必须保留。
 */
const ${VARS[locale]}: Record<string, string> = {
${lines.join("\n")}
};

export default ${VARS[locale]};
`;
  writeFileSync(join(root, "src/i18n", `${locale}.ts`), body);
  console.log(`${locale}: ${enKeys.length} entries → src/i18n/${locale}.ts`);
}
