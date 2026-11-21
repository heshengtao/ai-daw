# 域：应用扩展（app extensions）

**应用扩展不是音频插件**。VST3 / CLAP / AU 是 DSP（声音）；应用扩展是 **npm 分发的应用级插件**，
给工作室加 **MCP 工具**、**右栏面板**、**内容包**。两者在界面上分属「插件」与「扩展」两个面板，
不要混着说。

## 先看现状

- `list_extensions`：已安装了哪些扩展、各自注册了什么工具 / 面板、有没有装载失败。
- 扩展工具和内建工具出现在**同一个 `tools/list`** 里；不认识的工具名也可以经
  `aidaw_call(op="…")` 调用（效果一样）。
- 用户装了扩展但没启用（`enabled: false`）时，它的工具与面板都不存在。

## 安装 / 卸载（高风险，先问）

扩展一装上，Host 侧代码就会在 daemon 里执行。所以：

1. 先 `search_extension_market(query=…)` 看市场里有什么（只读，不下载）。
2. 把候选、来源索引、版本与它声明的能力**摆给用户**，等明确同意。
3. 用户同意后才 `install_extension(name, confirm=true)`；不确定版本时用市场给的默认版本。
4. 卸载 / 停用用 `uninstall_extension` / `set_extension_enabled`；停用不会删文件。

**不要**为了「省事」自动安装，也不要说「已经装好」——没调用工具就是没装。

## 写一个扩展

一个扩展就是一个 npm 包，至少要有 `package.json` 里的 `aidaw.manifest` 指向的 `aidaw.json`：

```jsonc
// package.json
{
  "name": "@you/plugin-name",          // 身份就是包名
  "version": "0.1.0",
  "exports": { ".": "./dist/host.cjs", "./client": "./dist/client.js" },
  "files": ["dist", "aidaw.json"],
  "aidaw": { "manifest": "./aidaw.json" },
  "engines": { "aidaw": ">=1.9.0 <2.0.0" }   // 缺了就是不兼容（不猜）
}
```

```jsonc
// aidaw.json
{
  "apiVersion": 1,
  "id": "plugin-name",
  "title": { "zh": "中文名", "en": "English name" },
  "host": "./dist/host.cjs",     // 可选：没有就是纯界面扩展
  "client": "./dist/client.js",  // 可选：没有就是纯工具扩展
  "capabilities": ["tools", "ui"],
  "panes": [{ "id": "my-pane", "title": { "zh": "我的面板" }, "icon": "puzzle", "order": 200 }]
}
```

`capabilities` 决定能注册什么：`tools` / `ui` / `storage` / `events` / `content`。
没声明就注册 → **整包装载失败**（fail-loud，不是静默忽略）。

### Host 侧契约

**必须是自包含的单文件 CJS**（用 esbuild `--format=cjs` 打，`require` 只用来报错；
QuickJS 里没有 `node_modules` 解析）：

```js
module.exports = {
  activate(aidaw, config) {
    aidaw.registerTool(
      { name: "my_tool", description: "说明（模型靠它决定什么时候用）",
        parameters: { type: "object", properties: {} }, risk: "read" },
      (args) => ({ ok: true }),           // 返回值会 JSON 序列化
    );
    const overview = aidaw.call("get_project_overview", {});  // 同步调宿主命令
  },
  deactivate() {},
};
```

| API | 说明 | 需要 |
| --- | --- | --- |
| `aidaw.registerTool(spec, fn)` | 注册 MCP 工具；`risk` = `read` / `write` / `dangerous` | `tools` |
| `aidaw.call(cmd, args)` | 调宿主：**MCP 工具名**（`get_project_overview` / `add_notes`…）或 `/rpc` 命令都行，同一条管线、署名 `ext:<包名>`，30 s 超时；不能调自己的工具（会死锁，被宿主拒绝） | — |
| `aidaw.storage.get/set/remove` | 私有 JSON 存储 | `storage` |
| `aidaw.on(event, fn)` | 订阅引擎事件（`project_changed` 等） | `events` |
| `aidaw.registerContent(pack)` | 内容包（曲风 / 配器 / 配乐规则） | `content` |
| `aidaw.log(level, msg)` | 写日志 | — |

限制（v1）：**没有 fetch / fs / require**；工具 handler 是同步的；内存 64 MiB、activate 10 s、
单次调用 30 s。要联网就 `aidaw.call` 一个宿主命令。

### Client 侧契约

**自包含 ESM**，Vue 从 `window.aidawExt.vue` 取（不能 `import "vue"`）：

```js
export default function install(api) {
  const { h, ref, onMounted } = api.vue;
  const Pane = { setup() { return () => h("div", {}, "hello"); } };
  api.registerPane({ id: "my-pane", component: Pane });   // id 必须与清单里的 panes 一致
}
```

面板 id 全局唯一（含内置的 9 个）；冲突会让这个扩展**装载失败**，不会覆盖别人。

## 打包与发布

```sh
esbuild src/host.ts --bundle --platform=node --format=cjs --outfile=dist/host.cjs
esbuild src/client.ts --bundle --format=esm --external:vue --outfile=dist/client.js
npm publish
```

Client 侧 `api.invoke(name, args)` 与 Host 侧 `aidaw.call` 一样：**MCP 工具名或 /rpc 命令都行**
（例如 `api.invoke("get_project_overview")`）。

发布后由插件商店索引（`index.json`）负责「被发现」：索引里写 `name`、精确 `version`、
`engines.aidaw`（与 package.json 一致）和 `source: { type: "npm" }`。生成索引用
`tools/build-extension-index.mjs`（见 `docs/extensions-index.md`）。

## 调试

- 装完不生效：先 `list_extensions` 看 `state` 与 `error`；
- 改了文件想即时生效：`reload_extension(name)`（Host 侧重建上下文，Client 侧重新 import）；
- 工具调用报错：错误会带着 JS 栈回到调用方，先看栈第一行。
