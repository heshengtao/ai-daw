# 应用扩展架构（App Extensions）— 设计

> **状态（2026-11-18）**：设计定稿，开始实施。
> 参考实现：DeepSeek Harness（`deepseek-harness`）的 bundle / plugin-manager / plugin-market /
> plugin-market-index。本文只抄**机制**，不抄 Node 特有的实现方式（详见 §12）。
> 逐任务记录见 `plans/2026-11-18-extensions.md`。

## 0 名词

| 词 | 含义 |
| --- | --- |
| **插件（audio plugin）** | 已有的 VST3 / CLAP / AU，DSP 层的东西。代码里叫 `plugin`。 |
| **应用扩展（app extension）** | 本文的东西：一个 npm 包，给工作室加工具、面板、供应商、内容。代码里叫 `extension`；界面上统一叫「应用扩展」。 |
| **扩展宿主（host runtime）** | daemon 里的 QuickJS 实例，跑扩展的 Host 侧代码。 |
| **市场（market）** | 一个 GitHub 仓库里的 `index.json`，列出可装的扩展。 |
| **注册表（registry）** | npm registry（可换国内镜像），扩展包的实际分发处。 |

界面上统一叫「应用扩展」，不叫「插件」，避免和 VST3 / CLAP 混在一起；插件面板与扩展面板是两个入口。

## 1 背景

AI DAW 现在的能力面全是**编译进二进制**的：144 个 MCP 工具写在 `crates/daemon/src/mcp.rs`，
云音乐供应商是一个 `enum`（`cloud_music.rs`），模型与运行时是四张手写表
（`models.rs` / `runtimes.rs` / `backends.rs` / `capabilities.rs`），右栏面板是 9 条静态
`PANES`（`src/stores/layout.ts`）。加一个能力要改 3–4 处、编译、发版。

同时这个项目的定位是「让模型在真实工程里干活」。模型能看见的世界 = **工具表 + 技能文档**。
所以「可扩展」对它比对普通 DAW 更重要：一个不能长出新工具的工作室，模型就只能永远用那 144 个动作。

DeepSeek Harness 已经把这件事做了一遍，并且是在「插件会很多」的前提下做的：npm 分发、
bundle + 补丁层、安装/更新/卸载、市场索引、热更新、按上下文回收注册。这套东西的**失败模式**
（命名冲突、注册泄漏、版本漂移、索引不可信）比它的成功模式更值得抄。

## 2 目标与非目标

**目标**

- 扩展是 **npm 包**：`npm pack` 产物即可分发；支持 `registry` + 国内镜像；也支持**本地目录 /
  `.tgz` 导入**。
- **不重启热更新**：装完、更新完、卸载完，工具表与面板立即变化；Host 侧重建 QuickJS 上下文，
  Client 侧重新 `import()` 新 blob。
- **互不打架**：一个扩展一个线程、一个 QuickJS 上下文、一份私有存储；注册名冲突**拒绝装载**
  而不是覆盖；卸载即回收全部注册。
- **市场**：一份可静态托管的 `index.json`（GitHub 仓库维护），应用启动后能实时发现、搜索、
  按兼容性过滤、安装/更新；索引源可在设置页增删。
- **AI 可见**：扩展注册的工具出现在 MCP `tools/list` 里，描述与风险等级齐全；市场搜索也有
  MCP 工具，agent 能自己发现并提议安装（安装必须用户确认）。
- **失败可解释**：装失败、跑崩、版本不兼容、注册冲突，都落到「扩展管理器」里的一句人话，
  而不是静默消失。

**非目标（v1 明确不做）**

- **不做扩展的网络与文件能力**：Host API 不暴露 `fetch` / `fs`。需要联网的能力走
  `aidaw.call(...)` 调宿主命令（宿主已有代理/镜像/鉴权），或等 v2 的显式权限。
- **不把供应商/内容包接进现有执行路径**：v1 只做「注册 + 展示 + MCP 可查」，把它们真正接进
  `cloud_music` / `models` / 'cue_sheet' 是下一阶段（否则是个大改且没有市场内容可验证）。
- **不执行 npm 的 install script / 不解析依赖树**：扩展包必须是**自包含**的（依赖在作者侧
  打包进产物）。QuickJS 里没有 `node_modules` 解析，这是硬约束，也顺便消掉了供应链的一大块。
- **不做代码签名/公证**：只校验 npm 的 `dist.integrity`（传输完整性）。索引与包都是**不可信输入**，
  安装 = 用户明确同意执行第三方代码。
- **不做 Git 源的版本**：索引格式保留 `source.type = "git"` 的位置，v1 读取时**整份索引拒绝**
  （与 DSH 一致：不认识的 source 是生成器 bug），生成器只产 npm 源。
- **不做插件 UI 的 iframe 沙箱**：Client 侧扩展与主界面同源同上下文（见 §9 的风险说明）。

## 3 关键决策

### D1 双侧运行时：Host = daemon 内嵌 QuickJS，Client = webview ESM

DSH 是 Host(TS/Node) + Client(React)。AI DAW 的对应物：

- **Host 侧**（`dist/host.cjs`）：跑在 daemon 的 QuickJS 上下文里。能给 MCP 加工具、能 `aidaw.call`
  调宿主命令、有私有存储、能订阅引擎事件。**无头 / Docker 模式一样工作**——这是选它的第一理由。
- **Client 侧**（`dist/client.js`）：跑在 webview 里，注册右栏面板。用 blob URL 动态 `import()`，
  所以 Tauri 与浏览器两种传输都能加载，不需要 CORS / asset 协议。

两者由**同一个包**提供，可各自缺省（只有 Host 或只有 Client 的扩展合法）。

**为什么不是「只做前端」**：前端扩展加不了 MCP 工具，无头模式直接失效，AI 侧扩展能力为零。
**为什么不是「独立 Node 子进程」**：打包 Node 至少 +40 MB，和 38 MB 的包体目标冲突；而且插件在
无头机上也得能跑。**QuickJS 的代价**是 1–2 MB 和一个 C 编译依赖，换来宿主内可控的沙箱
（内存上限、栈上限、中断钩子、上下文级热重建），这是划算的。

**API 是消息契约，不是 Rust trait**：`aidaw.call` / 工具调用 / 事件都走 JSON 字符串。所以将来把
宿主搬进独立 helper 进程（`vst3-host-helper` 已有先例）不需要改任何扩展代码。

### D2 Host 侧是同步 API（v1）

QuickJS 的 Promise/作业队列要和 tokio 拼起来，代价高、坑深。v1 全部同步：

- 工具 handler 是同步函数，返回 string / JSON 可序列化值。
- `aidaw.call(cmd, args)` 在**插件线程**上阻塞，等 daemon 的命令跑完（带超时）。
- 慢命令（导出、AI 生成）只阻塞那一个插件线程，不阻塞 daemon；代价是该插件的并发调用会排队。

这条以后可以升级成 async（API 形状不变，handler 返回 Promise 即可），所以不算单向门。

### D3 包形状：CJS/IIFE，不解析 `import`

入口契约（`aidaw.json` 里的 `host` 字段指向的文件）：

```js
// dist/host.cjs —— esbuild --format=cjs 的产物
module.exports = {
  activate(aidaw, config) { ... },   // 必须；加载时同步执行
  deactivate() { ... },              // 可选
};
```

也接受 IIFE 形式（顶层调 `aidaw.plugin(fn)`），但**推荐 CJS**：作者写 TS/ESM，打包时选 cjs，
运行时不需要模块解析器。理由：QuickJS 没有 node_modules 解析，允许 `import` 只会制造
「本地能跑、装进来炸」的落差。`require` 只提供白名单内建（v1 为空）。

### D4 命名冲突：拒绝，不覆盖

- 工具名全局唯一（含内建 144 条）。冲突 → **后加载者整包装载失败**，管理器里显示冲突对象；
  先加载者不受影响。
- 内置工具永远优先：扩展不能影子 `export_wav`、`set_tempo` 这类名字。
- 面板 id 同理（`layout.ts` 的 9 个内置 id 保留）。
- 加载顺序 = 已装清单按包名排序，确定性；所以冲突结果可复现。

不自动加前缀（`ext_<id>_<tool>`）是因为模型读到的是工具名本身，前缀会让 100 个扩展变成 100 个
难看的命名空间。约定优于强制：文档建议作者用 `<域>_<动作>` 且先查 `list_extensions` 看占用。

### D5 市场格式：直接沿用 DSH 的 `index.json`（format 1），只换引擎字段

字段、校验规则、图标内联上限、`dist-tags`、版本钉死、`compatibility` 语义全部照抄，唯一差异：

- `engines.dsh` → **`engines.aidaw`**（对 `AI DAW` 版本求值，`includePrerelease: true`）。
- `source.type = "git"` 保留在格式里，v1 读取即拒绝（生成器只产 npm）。
- 安装参数从 pnpm 风格改成 **`<name>@<exact version>` + registry 解析**（见 §10）。

理由：这套格式是「海量插件」前提下被推敲过的——列表页零额外请求、图标内联有上限、版本钉死
防「索引没变但装出来的东西变了」、未知字段向前兼容。自己设计一版精简格式，三个月后就会撞上
它已经解决的问题。

### D6 安装 = 执行第三方代码，必须显式同意

和现有 `install_plugin`（音频插件）一个级别：MCP 侧必须 `confirm=true` 且用户在当前对话里同意；
界面侧弹确认框，列出**包名、版本、来源 registry、索引 URL、声明的权限**。索引不可签名，
所以「官方」标签不加任何信任。

## 4 架构

```
┌──────────────────────────── daemon (Rust, tokio) ────────────────────────────┐
│                                                                              │
│  commands / IPC ──► extensions::Manager ──► ExtensionHost(plugin A)  [线程]   │
│                          │                       └─ rquickjs Runtime/Context  │
│                          │                               │ aidaw.* 同步调用     │
│                          ├─ Registry (tools/panes/…) ◄────┘                   │
│                          ├─ State (extensions.json)                          │
│                          ├─ Installer (npm registry / tarball / local dir)    │
│                          └─ Market (index 拉取 / 缓存 / 搜索)                  │
│                                                                              │
│  mcp.rs: definitions() = 内置 144 + Registry.tools()                          │
│          dispatch()    = 内置 match  → 未命中则 Registry.call_tool()          │
└──────────────────────────────────────────────────────────────────────────────┘
                                    │ engine 事件 / extensions://changed
┌──────────────────────────── webview (Vue) ───────────────────────────────────┐
│  extensions store ──► 动态 import(blob) ──► client.js ──► registerPane(...)   │
│  RightPanel: <component :is> 渲染内置面板或扩展面板                            │
└──────────────────────────────────────────────────────────────────────────────┘
```

**为什么扩展宿主是「一个扩展一个线程」**：QuickJS 的 `Runtime` / `Context` 是 `!Send` 的，必须
独占线程；顺带拿到隔离——一个扩展死循环只卡自己（有中断钩子兜底），不会卡音频线程或 UI。
线程按需创建（`enabled && has host entry`），卸载即 `join`。

**为什么注册表是 daemon 级的**：工具表要给 MCP 用、面板要给所有已连客户端用、无头模式没有
webview。注册表是唯一真相，Client 只是它的一个消费者。

## 5 插件包格式

```jsonc
// package.json
{
  "name": "@aidaw/chord-suggest",
  "version": "0.1.0",
  "description": "按当前和弦进行推荐声部",
  "type": "module",                       // 作者侧写法，不影响运行时
  "exports": {
    ".": "./dist/host.cjs",
    "./client": "./dist/client.js",
    "./locale/*": "./locale/*.json",
    "./icon": "./icon.svg"
  },
  "files": ["dist", "locale", "icon.svg", "aidaw.json"],
  "aidaw": { "manifest": "./aidaw.json" },  // 有它才是扩展；DSH 这里是 dsh.bundle.patch
  "engines": { "aidaw": ">=1.9.0 <2.0.0" }  // 也用 aidaw.json.minAppVersion 兜底
}
```

```jsonc
// aidaw.json
{
  "apiVersion": 1,
  "id": "chord-suggest",              // 展示与存储用；身份仍是包名
  "title": { "zh": "和弦建议", "en": "Chord Suggest" },
  "description": { "zh": "…", "en": "…" },
  "host": "./dist/host.cjs",          // 可缺省 = 只有界面
  "client": "./dist/client.js",       // 可缺省 = 只有工具
  "capabilities": ["tools", "ui", "storage", "events", "content"],
  "permissions": [],                  // v1 恒为空数组，留给将来（net/fs/audio）
  "icon": "./icon.svg"
}
```

规则：

- `package.json` 必须有 `aidaw.manifest`，`aidaw.json` 的 `apiVersion` 必须是 `1`。
- `host` / `client` 至少一个；两个都没有 = 空包装不上。
- `capabilities` 决定注册权限：没声明 `tools` 调 `registerTool` 直接抛错（**装载失败**，不是静默忽略）。
- 包名与版本以 `package.json` 为准；索引里的 `name`/`version` 必须与之一致，安装时**再校验一次**。

## 6 Host API（v1，全部同步）

```js
module.exports = {
  activate(aidaw, config) {
    aidaw.registerTool(
      { name: "suggest_voicing", description: "…", parameters: { /* JSON Schema */ }, risk: "read" },
      (args) => ({ voicing: [60, 64, 67] })      // 返回值会被 JSON 序列化
    );
    aidaw.registerContent({ genres: [ /* ... */ ] });   // capabilities: content
    aidaw.on("project_changed", (payload) => { /* ... */ });  // capabilities: events
    const t = aidaw.call("get_project_overview", {});   // 返回 JSON 字符串，同步阻塞
    aidaw.storage.set("last", "{}");
  },
  deactivate() {}
};
```

| API | 说明 | 需要 capability |
| --- | --- | --- |
| `aidaw.plugin(fn)` | IIFE 形式的入口（可选写法） | — |
| `aidaw.registerTool(spec, handler)` | 注册 MCP 工具；`risk` ∈ `read`/`write`/`dangerous`，默认 `write` | `tools` |
| `aidaw.call(cmd, argsJson)` | 调宿主：先查 `/rpc` 命令表，未命中落到 MCP 工具派发表（`get_project_overview` 这类不在 rpc 表里）；署名 `ext:<包名>`，30 s 超时；禁止同步调用自己的工具（死锁） | — |
| `aidaw.on(event, handler)` | 订阅引擎事件（`project_changed` / `transport` / `extensions://changed`） | `events` |
| `aidaw.storage.get/set/remove` | 扩展私有 JSON 存储，落在 `extensions/<name>/storage.json` | `storage` |
| `aidaw.registerContent(pack)` | 注册内容包（曲风/配器/配乐规则），MCP 可查 | `content` |
| `aidaw.log(level, msg)` | 写日志（落 daemon 日志 + 管理器诊断） | — |
| `aidaw.version` / `aidaw.pluginName` | 只读 | — |

**为什么 `risk` 由扩展自报**：宿主不知道一个工具做了什么；自报 + 管理器展示 + 内置工具的
`read_only_hint` 守卫测试（`mcp.rs` 已有）三件事一起，让「危险调用」至少是**可见**的。谎报不
构成安全边界，只是降低误用。

**v1 不提供**：网络、文件、音频线程、UI 打开对话框。要联网就 `aidaw.call` 一个宿主命令（比如
`compose_song`、`search_plugin_store`）——这也顺带让扩展的联网行为走宿主既有的代理与鉴权。

## 7 Client API（v1）

```js
// dist/client.js —— 产物是 ESM，顶层调用
aidawExt.registerPane({
  id: "chord-suggest",              // 与内置 9 个 pane id 不冲突
  title: { zh: "和弦建议", en: "Chord Suggest" },
  icon: "sparkles",                 // 取内置图标表里的键，避免 SVG 注入
  component: ChordPane,             // 用宿主提供的 Vue 运行时创建
  order: 100,
});
```

宿主注入（`window.__AIDAWC__`，import 前设置）：

| 成员 | 说明 |
| --- | --- |
| `vue` | 宿主的 Vue 运行时（`h` / `defineComponent` / `ref` / `computed`…），保证单实例 |
| `registerPane(spec)` | 注册右栏面板 |
| `invoke(cmd, args)` | 与界面同一套 IPC 传输（Tauri / HTTP 自动切换） |
| `on(event, handler)` | 应用事件（含 `extensions://changed`） |
| `i18n` | `t()` 与当前语言 |
| `theme` | 只读 token（`--md-sys-color-*`），面板直接写 CSS 即可 |
| `extension` | `{ name, version, id }` |

约束：

- **必须自包含**：不能 `import "vue"` 或任何裸模块；Vue 从 `__AIDAWC__.vue` 取。
- 不能改 DOM 之外的东西；样式必须用主题 token（与 DSH 同一条规则）。
- 面板组件崩溃不能拖垮整页：渲染包一层 `onErrorCaptured`，坏面板显示错误占位并允许关闭。

## 8 市场索引

默认源（常量，设置页可覆盖）：

```
https://heshengtao.github.io/ai-daw-plugins/index.json
```

仓库：<https://github.com/heshengtao/ai-daw-plugins>（GitHub Pages 从 `main` 发布；
raw 地址在部分网络下会被 TLS reset，所以默认取 Pages）。目前 `plugins: []`。

### 8.1 格式

照抄 DSH `format: 1`（见 `.agents/notes/.../plugin-market-index-format.md` 的字段表），差异只有
`engines.aidaw`。读取器规则也照抄：**整份拒绝**而不是丢条目（格式错误是生成器的 bug，静默丢
会让问题看不见）；未知成员忽略（向前兼容）；图标内联 ≤ 32 KiB base64，URL 图标按索引 URL 解析。

### 8.2 拉取与缓存

- `ureq` + 现有 `tls_agent` / 代理设置；15 s 超时、64 MiB 上限。
- 成功：写 `<cache_dir>/extensions/market/<sha256(url)>.json`，连同 `fetchedAt`；TTL 10 min。
- 失败：保留上一次成功的缓存并在 UI 标出「上次成功时间 / 本次错误」；没有缓存就只报错。
- 多个索引源按数组顺序展示；同名条目**先出现的源赢**（与 DSH 一致）。

### 8.3 搜索

规范化（NFKC + 小写）后按词匹配；中日韩文本切成重叠双字（不引分词库）；权重：包名/标题 ×3、
关键词 ×2、其余 ×1；所有词必须命中（子串或前缀）。**v1 不做编辑距离**（DSH 的四分之一长度
容错），文档记为本项目的已知差异。

排序：索引顺序 / 相关度 / 发布时间 / 市场声明的 sortKey；无值条目排在最后。

### 8.4 兼容性

`engines.aidaw` 用 `semver::VersionReq`（`includePrerelease` 语义按需处理）对当前版本求值；
不满足 → 列表里标灰 + 原因，安装按钮禁用。**缺声明 = 不兼容**（不猜、不回退 peerDeps）。

## 9 安装 / 更新 / 卸载 / 导入

### 9.1 从市场或 npm 安装

```
1. 解析：index(name) → defaultVersion（latest）或用户指定版本 → { name, version, registry? }
   直接从 npm 安装时：GET {registry}/{name} → 选满足 engines.aidaw 的最高稳定版本
2. 拉元数据：该 version 的 dist.tarball / dist.integrity / dist.shasum
3. 下载 tarball 到 cache 临时目录（复用 manager::download_verified 的思路，但校验 sha512）
   - 优先 dist.integrity（sha512-<base64>）；没有就 sha1 shasum；都没有 = 拒绝安装
4. 解压（tar.gz；现有 extract_archive 复用）→ 要求所有条目在 `package/` 下，去掉这层
5. 校验：package.json 的 name/version 与请求一致；有 aidaw.manifest；aidaw.json 合法；
   engines.aidaw 对当前版本成立
6. 落盘：<data_dir>/extensions/<name>/<version>/；原子改名（先解到 .tmp 再 rename）
7. 热加载：卸旧版本 → 载新版本 → 更新 extensions.json → 发 extensions://changed
```

失败在**任何一步**都回滚：临时目录清掉、旧版本继续用、错误写进管理器。

**权限确认**：安装对话框展示 `capabilities` 与 `permissions`；`permissions` 非空（将来）时要求
逐项同意。

### 9.2 更新

- 已装版本 vs 索引 `latest`（或用户选的频道）：不一致即「可更新」。
- 更新 = 装新版本 → 切换启用指针 → 删除旧目录（失败则保留旧版本并可回滚，保留最近 2 个版本）。
- 不做增量、不做后台静默更新。

### 9.3 卸载

停宿主线程 → 从注册表移除该扩展的全部注册 → 删目录与状态 → 发事件。只删本应用装的。

### 9.4 本地导入

- **目录**：校验 `package.json` → 复制进托管目录（`<name>/<version>/`）→ 记为 `source: { type: "local" }`。
  （v1 复制而不软链：用户随时可能删源目录。）
- **`.tgz`**：同 9.1 的第 3–7 步，跳过下载。

### 9.5 状态文件

`<config_dir>/extensions.json`：

```jsonc
{
  "indexes": ["https://raw.githubusercontent.com/<owner>/ai-daw-plugins/main/index.json"],
  "registry": "https://registry.npmjs.org",
  "mirrors": ["https://registry.npmmirror.com"],
  "installed": {
    "@aidaw/chord-suggest": {
      "version": "0.1.0",
      "enabled": true,
      "source": { "type": "npm", "registry": "https://registry.npmjs.org", "index": "…" },
      "installedAt": 1763500000,
      "config": {},
      "capabilities": ["tools", "ui"],
      "permissions": []
    }
  }
}
```

`enabled: false` 的扩展**不装载**（工具与面板都不出现），文件保留。

## 10 工具表接入

- `dispatch(name)`：内置 `match` 未命中 → 扩展宿主（`crate::extensions::try_call`）。
- `ServerHandler::list_tools` / `call_tool` **手写覆盖**（`#[tool_handler]` 只补缺失方法，可以盖掉）：
  `list_tools` = 内置 `tool_router.list_all()` + 扩展注册的工具；`call_tool` 先查内置路由，
  未命中再走扩展。这样扩展工具与内置工具出现在同一个 `tools/list` 里。
- `definitions()`（内置 agent 的 OpenAI 工具表）**不**追加扩展工具：agent 走的是 MCP 派发，
  工具表保持静态可测；扩展工具由 `list_extensions` 发现、经派发调用。
- `aidaw_call` 网关自动覆盖扩展工具（它走 `dispatch`）。
- 守卫测试：扩展工具不得与内置同名（`definitions()` 里查重），`read_only_hint` 按 `risk` 映射。
- 扩展工具的参数 schema 原样透传（扩展自报 JSON Schema，宿主不校验其正确性，只校验是对象）。

## 11 界面

- 新面板 `extensions`（右栏，图标 puzzle），三个标签：**发现 / 已安装 / 设置**。
  - 发现：市场列表（标题、作者、版本、体积未知、索引来源标记）、搜索框、分类/标签筛选、
    安装按钮（不兼容则禁用并给原因）。
  - 已安装：启用开关、版本、更新按钮、卸载、配置（JSON 编辑器）、诊断（加载错误/冲突/崩溃次数）。
  - 设置：索引源列表（增删）、npm registry 与镜像、刷新、上次成功时间。
- 设置页「扩展」一节给出入口 + 同样的索引源配置。
- i18n：`zh.ts` / `en.ts` 各加一组 `extensions.*`。
- 空索引是**正常状态**：发现页显示「还没有可用的扩展」+ 索引 URL，不报错。

## 12 与 DSH 的差异（为什么）

| DSH | 本项目 | 原因 |
| --- | --- | --- |
| Node + pnpm 解析 `node_modules` | QuickJS + 自包含 CJS | 没有 Node；自包含还消掉了依赖树与 install script 的供应链面 |
| `cordis.patch.yml` 补丁层 | `aidaw.json` + `extensions.json` 状态 | 补丁层的价值是 profile 多层覆盖；单用户桌面应用只需要「启用/配置」两件事 |
| Host 插件改动要重启 | Host 上下文热重建 | QuickJS 上下文销毁是 O(1)；这是选内嵌引擎的主要收益 |
| Client 插件走模块加载器 + HMR | blob URL 动态 import + 注册表回收 | 同源 blob 在 Tauri 与浏览器都成立，不依赖 dev server |
| 工具可被任意插件注册（全局名） | 冲突即拒绝装载 | 「插件互相不打架」是明确目标；拒绝比覆盖好排查 |
| 市场索引默认无源 | 默认指向本项目的 GitHub 索引 | 产品化：开箱能发现 |

## 13 存储布局

```
<config_dir>/extensions.json                 # 状态（上面 §9.5）
<data_dir>/extensions/<pkg>/<version>/...    # 包内容（只读）
<data_dir>/extensions/<pkg>/storage.json     # 扩展私有存储
<cache_dir>/extensions/market/<hash>.json    # 索引缓存
<cache_dir>/extensions/tmp/                  # 下载与解压临时区
```

## 14 安全边界

1. **安装即执行**：装载时就跑第三方代码。确认对话框 + MCP `confirm=true` 是唯一闸门。
2. **沙箱**：内存 64 MiB、栈 4 MiB、中断钩子（工具调用 30 s 超时，加载 10 s 超时）、无
   `eval` 之外的宿主能力注入。
3. **无 fs / net**：Host API 不提供；只能经 `aidaw.call` 用宿主命令（宿主有代理、镜像、鉴权、
   高风险确认）。
4. **索引不可信**：标题、作者、标签都是市场自述；UI 必须显示索引 URL 与来源 registry。
5. **Client 同上下文**：前端扩展与主应用同源，没有 iframe 隔离（DSH 明确否决 iframe：会丢主题
   与本地化）。用户装的是代码，不是数据——这一点在确认框里写清楚。
6. **崩溃隔离**：扩展线程 panic 不传播（`catch_unwind` + 线程 join），该扩展标红并停止装载，
   下次启动重试一次。

## 15 版本与演进

- `aidaw.json.apiVersion`：不兼容的 API 变更才升；读取器拒绝未知的值。
- `engines.aidaw`：包对应用版本的要求，安装与每次装载都校验。
- 索引 `format: 1`：只加字段不破坏；语义变了升 `format`。
- 状态文件带 `version: 1`，迁移函数写在 `extensions::state`。

## 16 验收

1. 本地放一个测试扩展（Host 注册 1 个工具、Client 注册 1 个面板），启动后 MCP `tools/list`
   里出现该工具，右栏出现该面板；调用工具返回正确 JSON，并带 `Actor::Extension` 署名进撤销栈。
2. 改扩展文件 → `重载` → 工具行为与面板内容变化，**不重启应用**；旧面板标签自动收起。
3. 装两个注册同名工具的扩展 → 第二个装载失败，管理器显示冲突，第一个照常工作。
4. 装一个 `engines.aidaw` 不兼容的包 → 拒绝安装并给原因；索引里该版本标灰。
5. 索引源指向一个空 `index.json`（本项目默认仓库）→ 发现页显示空态，不报错；指向坏 JSON →
   报「索引无效 + JSON 路径」，且不影响已装扩展。
6. 卸载扩展 → 工具与面板立即消失，进程内无残留注册（列表为空）。
7. `cargo test`：索引校验、搜索、版本兼容、tarball 路径剥离、状态迁移各有单测。
8. 无头模式（`aidawd`）：同样的扩展能装载、工具可调，说明没有任何一步依赖 webview。

## 17 已知取舍与缺口

- **同步 Host API**：扩展做不了「边下载边回调进度」，只能一次阻塞调用。v2 再上 async。
- **一个扩展一个线程**：100 个扩展 = 100 个线程（空转成本低，但内存按扩展累加，QuickJS 约
  几百 KB/上下文）。真到「海量」再改成共享宿主 + 轮转。
- **不做编辑距离搜索**：中文短词容易搜不到（比如打错一个字），先用精确双字匹配；等索引大了再补。
- **供应商/内容包只注册不接执行**：v1 的价值是「先让市场有东西可发现」，接线下一阶段。
- **没有扩展间的依赖**：一个扩展不能声明依赖另一个（DSH 用 npm 依赖解决）。v1 明确不支持；
  真要组合能力，让一个扩展在包内实现。
