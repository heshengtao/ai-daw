#!/usr/bin/env node
/**
 * 校验所有界面语言词典与英文源一致：键集合、占位符、空值、覆盖率。
 *
 * 用法：`node tools/i18n-check.mjs`（CI / 本地都能跑，失败退出码非 0）。
 */
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { loadDict } from "./i18n-extract.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const LOCALES = ["zh", "zh-TW", "en", "ja", "ko", "es", "fr", "de", "ru", "ar"];

const placeholders = (value) =>
  [...String(value).matchAll(/\{[a-zA-Z0-9_]+\}/g)].map((m) => m[0]).sort().join(",");

const en = await loadDict(join(root, "src/i18n/en.ts"));
const enKeys = Object.keys(en);
let failed = false;

for (const locale of LOCALES) {
  const dict = await loadDict(join(root, "src/i18n", `${locale}.ts`));
  const keys = Object.keys(dict);
  const missing = enKeys.filter((k) => !(k in dict));
  const extra = keys.filter((k) => !(k in en));
  const bad = enKeys.filter((k) => k in dict && placeholders(en[k]) !== placeholders(dict[k]));
  const empty = keys.filter((k) => !String(dict[k]).trim());
  const translated = enKeys.filter((k) => k in dict && String(dict[k]) !== String(en[k])).length;
  const coverage = ((translated / enKeys.length) * 100).toFixed(1);
  const ok = !missing.length && !extra.length && !bad.length && !empty.length;
  if (!ok) failed = true;
  console.log(
    `${ok ? "✓" : "✗"} ${locale.padEnd(6)} ${String(keys.length).padStart(4)} keys · ${coverage}% differs from en` +
      (missing.length ? ` · missing ${missing.length}` : "") +
      (extra.length ? ` · extra ${extra.length}` : "") +
      (bad.length ? ` · placeholder ${bad.length}` : "") +
      (empty.length ? ` · empty ${empty.length}` : ""),
  );
  if (!ok) {
    for (const k of [...missing.slice(0, 3), ...extra.slice(0, 3), ...bad.slice(0, 3), ...empty.slice(0, 3)]) {
      console.log(`    ${k}`);
    }
  }
}

if (failed) process.exit(1);
