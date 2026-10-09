// 从 src/data/plugins.ts 里抠出出厂预设（补丁 + 默认值），摊平成完整参数集，写成 JSON。
// 用法: node tools/extract-presets.mjs <plugins.ts> <out.json>
import { readFileSync, writeFileSync } from "node:fs";

const [src, out] = process.argv.slice(2);
if (!src || !out) {
  console.error("用法: node tools/extract-presets.mjs <plugins.ts> <out.json>");
  process.exit(2);
}
const text = readFileSync(src, "utf8");

/**
 * 抠出 `export const NAME ... = <字面量>;` 里的字面量。
 *
 * 必须从 `=` **之后**开始找开括号：声明上带着 `: FactoryPreset[]` 这样的类型标注，
 * 从名字后面直接找会被类型里的 `[]` 骗到。
 */
function literalAfter(name, open, close) {
  const decl = text.indexOf(`export const ${name}`);
  if (decl < 0) throw new Error(`找不到 ${name}`);
  const eq = text.indexOf("=", decl);
  if (eq < 0) throw new Error(`${name} 没有 =`);
  const start = text.indexOf(open, eq);
  if (start < 0) throw new Error(`${name} 没有 ${open}`);
  let depth = 0;
  for (let i = start; i < text.length; i += 1) {
    const c = text[i];
    if (c === open) depth += 1;
    else if (c === close) {
      depth -= 1;
      if (depth === 0) return text.slice(start, i + 1);
    }
  }
  throw new Error(`${name} 没闭合`);
}

/** 参数表的 id → default。 */
function defaults(name) {
  const body = literalAfter(name, "[", "]");
  const out = {};
  for (const m of body.matchAll(/id:\s*"([^"]+)"[^}]*?default:\s*(-?[\d.]+)/gs)) {
    out[m[1]] = Number(m[2]);
  }
  if (Object.keys(out).length === 0) throw new Error(`${name} 没解析出默认值`);
  return out;
}

/** 数据文件里的数组/对象字面量就是纯 JS；带 `[DELAY_ID]:` 这种计算键，所以要把常量喂进作用域。 */
const constants = {};
for (const m of text.matchAll(/export const (SYNTH_ID|DELAY_ID|REVERB_ID)\s*=\s*"([^"]+)"/g)) {
  constants[m[1]] = m[2];
}
const names = Object.keys(constants);
const evaluate = (literal) =>
  new Function(...names, `return (${literal})`)(...names.map((n) => constants[n]));

const synthDefaults = defaults("SYNTH_PARAMS");
const delayDefaults = defaults("DELAY_PARAMS");
const reverbDefaults = defaults("REVERB_PARAMS");

const synth = evaluate(literalAfter("SYNTH_FACTORY_PRESETS", "[", "]")).map((p) => ({
  id: p.id,
  category: p.category,
  params: { ...synthDefaults, ...p.params },
}));

const effectTable = evaluate(literalAfter("EFFECT_FACTORY_PRESETS", "{", "}"));
const effects = [];
for (const [pluginId, list] of Object.entries(effectTable)) {
  const base = pluginId.includes("delay") ? delayDefaults : reverbDefaults;
  for (const p of list) {
    effects.push({ id: p.id, category: pluginId, params: { ...base, ...p.params } });
  }
}

writeFileSync(out, JSON.stringify([...synth, ...effects], null, 2));
console.log(`写出 ${out}: ${synth.length} 条合成器预设 + ${effects.length} 条效果器预设`);

// ---- 一致性地校验：注册表里那 5 条在 Rust 与 TS 两侧必须逐值相同 ----
// 引擎注册表的补丁只能带参数（带不了效果链），默认分轨直接挂它；两边一旦漂移，
// 「新建工程时的听感」和「点一下那个预设的听感」就对不上，而且不会有任何报错。
const REGISTERED = [
  "builtin.synth.lead",
  "builtin.synth.bass",
  "builtin.synth.drums",
  "builtin.synth.strings",
  "builtin.synth.pad",
];

if (process.argv.includes("--check-registry")) {
  const rs = readFileSync(new URL("../crates/engine/src/registry.rs", import.meta.url), "utf8");
  const rustById = {};
  for (const m of rs.matchAll(/id: "([^"]+)",\s*\n\s*name: "[^"]*",\s*\n\s*patch: &\[([\s\S]*?)\]/g)) {
    const params = {};
    for (const pv of m[2].matchAll(/\(\s*"([^"]+)"\s*,\s*(-?[\d.]+)\s*\)/g)) params[pv[1]] = Number(pv[2]);
    rustById[m[1]] = params;
  }
  const tsById = Object.fromEntries(synth.map((p) => [p.id, p.params]));
  let bad = 0;
  for (const id of REGISTERED) {
    const rust = rustById[id];
    const ts = tsById[id];
    if (!rust) { console.error(`❌ Rust 注册表里没有 ${id}`); bad += 1; continue; }
    if (!ts) { console.error(`❌ TS 预设表里没有 ${id}`); bad += 1; continue; }
    for (const [k, v] of Object.entries(rust)) {
      // 补丁只写出改过的参数，未列出的取该合成器的默认值。
      const expected = ts[k] ?? synthDefaults[k];
      if (Math.abs(expected - v) > 1e-6) {
        console.error(`❌ ${id}.${k}: Rust=${v} TS=${expected}`);
        bad += 1;
      }
    }
    if (tsById[id].effects && tsById[id].effects.length) {
      console.error(`❌ ${id} 带有效果链，但注册表补丁带不了效果链`);
      bad += 1;
    }
  }
  console.log(bad === 0 ? "✅ 注册表 5 条与 TS 预设表逐值一致" : `发现 ${bad} 处不一致`);
  process.exit(bad === 0 ? 0 : 1);
}
