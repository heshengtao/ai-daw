#!/usr/bin/env node
/**
 * 扩展 **Client 侧契约**自检（不需要跑界面、不需要浏览器）。
 *
 * 做的事：把 `examples/extensions/hello/dist/client.js` 当 ESM 装载，注入一份最小的
 * `aidawExt`（Vue 用真实的 `vue` 包，`invoke` 用假实现），断言：
 *
 *   1. 模块导出 `install(api)` 并调用了 `registerPane`，面板 id 与 `aidaw.json` 声明一致；
 *   2. 拿到的组件能被 Vue 渲染（SSR 渲染一次，至少证明 setup/render 不炸）。
 *
 * 用法：`npm run selftest:extensions`
 *
 * 这是「Client 插件能被加载」的最轻验证；真实 webview 里的 blob import 仍要人看一眼。
 */

import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { createSSRApp, h, ref, computed, shallowRef, markRaw, defineComponent, onMounted, onUnmounted, watch } from "vue";
import { renderToString } from "vue/server-renderer";

const root = resolve(import.meta.dirname ?? new URL(".", import.meta.url).pathname, "..");
const packageDir = resolve(root, "examples/extensions/hello");

const manifest = JSON.parse(await readFile(resolve(packageDir, "aidaw.json"), "utf8"));
const clientPath = resolve(packageDir, manifest.client);
const code = await readFile(clientPath, "utf8");

const url = `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
const module = await import(url);

if (typeof module.default !== "function") {
  throw new Error("Client 入口必须 `export default function install(api)`");
}

const panes = [];
const api = {
  extension: { name: "@aidaw/hello-extension", version: "0.1.0" },
  registerPane(spec) {
    panes.push(spec);
  },
  async invoke(cmd) {
    if (cmd !== "get_project_overview") throw new Error(`unexpected invoke ${cmd}`);
    return { tempo: 120, tracks: [{}, {}] };
  },
  on() {
    return () => {};
  },
  vue: { h, ref, computed, shallowRef, markRaw, defineComponent, onMounted, onUnmounted, watch },
  i18n: { t: (key) => key, locale: "zh" },
};

await module.default(api);

if (panes.length !== 1) throw new Error(`期望注册 1 个面板，实际 ${panes.length}`);
const [pane] = panes;
if (pane.id !== manifest.panes[0].id) {
  throw new Error(`面板 id 不一致：client=${pane.id} manifest=${manifest.panes[0].id}`);
}
if (!pane.component) throw new Error("registerPane 没有给 component");

const html = await renderToString(createSSRApp(pane.component));
if (!html || html.length < 10) throw new Error(`面板渲染结果为空：${JSON.stringify(html)}`);

console.log(`✓ Client 契约：${panes[0].id}（渲染 ${html.length} 字符）`);
console.log(html.replace(/\s+/g, " ").slice(0, 120));
