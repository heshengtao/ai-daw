#!/usr/bin/env node
/**
 * 扩展 **界面冒烟**：用 headless Chrome（CDP）打开浏览器版 AI DAW，验证
 *
 *   1. 「扩展」面板能打开，已安装的扩展出现在「已安装」里；
 *   2. 扩展贡献的 Client 面板能作为标签打开并渲染（blob ESM → registerPane → Vue 渲染）；
 *   3. 面板里 `api.invoke("get_project_overview")` 的返回值真的显示出来了。
 *
 * 它不替代人工验收，但能证明「Client 插件链路」在真浏览器里是通的。
 *
 * 前置：
 *   - `npx vite build`（产出 `dist/`）；
 *   - 起一个带前端的无头后端：
 *     `AIDAW_NO_AUDIO=1 ./target/debug/aidawd --data-dir /tmp/aidaw-ui \
 *        --port 28437 --web-dir "$PWD/dist"`
 *   - 装一个扩展（例如示例）：
 *     `curl -X POST http://127.0.0.1:28437/rpc/extensions_rpc -H 'content-type: application/json' \
 *        -d '{"op":"install_local","args":{"path":"'"$PWD"'/examples/extensions/hello"}}'`
 *   - 本机有 Chrome：默认 `/Applications/Google Chrome.app/...`，可用 `--chrome <path>` 覆盖。
 *
 * 用法：`node tools/ui-smoke-extensions.mjs [--url http://127.0.0.1:28437/] [--cdp 9222]`
 */

import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const argv = process.argv.slice(2);
const arg = (name, fallback) => {
  const index = argv.indexOf(`--${name}`);
  return index === -1 ? fallback : argv[index + 1];
};

const BASE = arg("url", "http://127.0.0.1:28437/");
const CDP_PORT = Number(arg("cdp", "9222"));
const CHROME = arg("chrome", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const profile = mkdtempSync(join(tmpdir(), "aidaw-ui-smoke-"));
const chrome = spawn(
  CHROME,
  [
    "--headless=new",
    "--disable-gpu",
    "--no-first-run",
    "--no-default-browser-check",
    `--user-data-dir=${profile}`,
    `--remote-debugging-port=${CDP_PORT}`,
    "--window-size=1440,900",
    "about:blank",
  ],
  { stdio: "ignore" },
);

async function cdpVersion() {
  for (let attempt = 0; attempt < 40; attempt += 1) {
    try {
      const response = await fetch(`http://127.0.0.1:${CDP_PORT}/json/version`);
      if (response.ok) return response.json();
    } catch {
      /* 还没起来 */
    }
    await sleep(250);
  }
  throw new Error("Chrome 的调试端口没起来");
}

function connect(wsUrl) {
  const ws = new WebSocket(wsUrl);
  let id = 0;
  const pending = new Map();
  ws.onmessage = (event) => {
    const message = JSON.parse(event.data);
    const entry = message.id && pending.get(message.id);
    if (!entry) return;
    pending.delete(message.id);
    message.error ? entry.reject(new Error(JSON.stringify(message.error))) : entry.resolve(message.result);
  };
  const ready = new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = reject;
  });
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const messageId = ++id;
      pending.set(messageId, { resolve, reject });
      ws.send(JSON.stringify({ id: messageId, method, params }));
    });
  return { ready, send, close: () => ws.close() };
}

try {
  await cdpVersion();
  const target = await (
    await fetch(`http://127.0.0.1:${CDP_PORT}/json/new?${encodeURIComponent(BASE)}`, { method: "PUT" })
  ).json();
  const client = connect(target.webSocketDebuggerUrl);
  await client.ready;
  await client.send("Page.enable");
  await client.send("Runtime.enable");
  await sleep(2500);

  const evaluate = async (expression) => {
    const result = await client.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
  };
  const reload = async (ms = 4500) => {
    await client.send("Page.reload", { ignoreCache: true });
    await sleep(ms);
  };
  const waitFor = async (expression, timeoutMs = 8000) => {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      if (await evaluate(expression)) return true;
      await sleep(200);
    }
    return false;
  };
  const screenshot = async (path) => {
    const shot = await client.send("Page.captureScreenshot", { format: "png" });
    writeFileSync(path, Buffer.from(shot.data, "base64"));
  };

  // 关掉欢迎页，并预置布局：打开「扩展」面板。
  await evaluate(`localStorage.setItem("ai-daw-settings", JSON.stringify({ defaultTempo: 120, welcomeOnStart: false }))`);
  await evaluate(`localStorage.setItem("ai-daw-layout", JSON.stringify({ tabs: ["extensions"], activeTab: "extensions", version: 2 }))`);
  await reload();
  if (!(await waitFor(`!!document.querySelector(".right-panel .pane")`))) {
    throw new Error("「扩展」面板没有渲染出来");
  }

  await evaluate(`[...document.querySelectorAll("button")].find((b) => b.textContent.trim() === "已安装")?.click()`);
  const found = await waitFor(
    `(() => { const text = document.body.innerText; return text.includes("@aidaw/") && text.includes("运行中"); })()`,
  );
  if (!found) {
    const text = await evaluate("document.querySelector('.right-panel .pane')?.innerText ?? '(no pane)'");
    throw new Error(`「已安装」里没有看到运行中的扩展：\n${text.slice(0, 400)}`);
  }
  await screenshot(join(tmpdir(), "aidaw-ui-extensions.png"));

  // 发现标签：索引状态胶囊 + 搜索框
  await evaluate(`[...document.querySelectorAll(".tab")].find((b) => b.textContent.trim().startsWith("发现"))?.click()`);
  await sleep(1500);
  const discoverText = await evaluate("document.querySelector('.right-panel .pane')?.innerText ?? ''");
  if (!discoverText.includes("索引") && !discoverText.includes("Index")) {
    throw new Error(`「发现」里没有索引状态：${discoverText.slice(0, 200)}`);
  }
  await screenshot(join(tmpdir(), "aidaw-ui-discover.png"));

  // 设置 → 应用扩展：索引源与 registry 真的在设置页里（不在面板里）
  await evaluate(`location.href = "${BASE.replace(/\/$/, "")}/#/settings?section=extensions"`);
  await sleep(3000);
  const settingsText = await evaluate("document.body.innerText");
  if (!settingsText.includes("npm registry")) {
    throw new Error(`设置页没有「应用扩展」一节：${settingsText.slice(0, 300)}`);
  }
  await screenshot(join(tmpdir(), "aidaw-ui-settings-extensions.png"));

  // 打开扩展贡献的面板。
  await evaluate(`location.href = "${BASE}"`);
  await sleep(2500);
  await evaluate(`localStorage.setItem("ai-daw-layout", JSON.stringify({ tabs: ["hello-pane"], activeTab: "hello-pane", version: 2 }))`);
  await evaluate(`localStorage.setItem("ai-daw-layout", JSON.stringify({ tabs: ["hello-pane"], activeTab: "hello-pane", version: 2 }))`);
  await reload();
  const paneText = await evaluate("document.querySelector('.right-panel')?.innerText ?? ''");
  if (!paneText.includes("Hello 扩展")) throw new Error(`扩展面板没渲染：${paneText.slice(0, 200)}`);
  if (paneText.includes("读取中")) throw new Error("扩展面板卡在加载态（invoke 没有返回）");
  await screenshot(join(tmpdir(), "aidaw-ui-hello-pane.png"));

  console.log("✓ 界面冒烟通过：扩展面板与扩展贡献的面板都渲染了");
  console.log(`  截图：${join(tmpdir(), "aidaw-ui-extensions.png")} / ${join(tmpdir(), "aidaw-ui-hello-pane.png")}`);
  client.close();
} finally {
  chrome.kill("SIGTERM");
}
