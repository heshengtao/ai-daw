# 应用扩展架构 — 实施计划

- 日期：2026-11-18
- 状态：🚧 实施中（P0 完成：`crates/extensions` 76 个单测全绿；daemon 已接桥并可在启动时装载）
- 依据：`specs/2026-11-18-extensions-architecture-design.md`
- 参考：DeepSeek Harness 的 `packages/boot/plugin-manager`、`plugin-market`、
  `packages/util/plugin-market-index`、`apps/plugin-market-generator`

## 0 里程碑

| 阶段 | 交付 | 验收 |
| --- | --- | --- |
| P0 | `crates/extensions`：包格式、状态、注册表、QuickJS 宿主、工具桥 | `cargo test -p ai-daw-extensions` 全绿 |
| P1 | daemon 接线：宿主生命周期、IPC 命令、MCP 工具合并与派发 | 测试扩展的工具出现在 `tools/list`，能调通 |
| P2 | Client 加载：模块读取、blob import、面板注册、右栏动态化 | 测试扩展的面板出现在右栏 |
| P3 | 市场：索引校验/缓存/搜索；npm 解析/下载/校验/解压/安装/更新/卸载/本地导入 | 单测覆盖 §16.5，空索引不报错 |
| P4 | 界面：扩展面板（发现/已安装/设置）、设置页入口、i18n | 手动走通发现→安装→启用→卸载 |
| P5 | 文档与分发：扩展开发指南、测试扩展样例、商店生成器、默认索引常量 | 新扩展能照文档 30 分钟内跑起来 |

## P0 — 核心 crate

1. [x] `crates/extensions` 骨架（workspace 成员、`#![forbid(unsafe_code)]` 需评估：rquickjs 绑定在
       依赖里，本 crate 自身仍可 forbid）。
2. [x] `manifest.rs`：解析 `package.json` 的 `aidaw` / `engines.aidaw` / `exports`；解析
       `aidaw.json`；`capabilities` 校验；`Host/Client` 至少一个。
3. [x] `state.rs`：`extensions.json`（indexes / registry / mirrors / installed / enabled / config），
       带 `version: 1` 与迁移入口。
4. [x] `registry.rs`：工具注册表（name → {plugin, desc, schema, risk, handler}），冲突检测
       （内置名单注入），面板注册表，内容包注册表；`snapshot()` 给 MCP/UI。
5. [x] `host.rs`：一个扩展一个线程；rquickjs Runtime（内存/栈/中断）；CJS 契约；
       `aidaw` 对象注入（registerTool / call / storage / on / log / registerContent）；
       `activate` 异常 → 装载失败并带栈；`deactivate` + 上下文销毁；panic 捕获。
6. [x] `bridge.rs`：JS 同步调用 → tokio 命令的阻塞桥（`Handle::block_on` + 超时）；
       事件从 daemon 派发到插件线程（`aidaw.on`）。
7. [x] 单测：manifest 校验、冲突拒绝、注册回收、工具调用、超时、坏脚本。

## P1 — daemon 接线

8. [x] `AppState` 加 `extensions: Arc<ExtensionsManager>`；`boot.rs` 里启动时装载已启用扩展。
9. [x] `commands.rs`（一个统一入口 `extensions_rpc(op, args)`：list / market / install / install_local / uninstall / set_enabled / reload / set_indexes / set_registry / client_manifest / client_source）：`ext_list`、`ext_reload`、`ext_set_enabled`、`ext_diagnostics`、
       `ext_read_client_module`、`ext_client_manifest`。
10. [x] `mcp.rs`：`definitions()` 追加扩展工具；`dispatch()` 兜底走注册表；
        `read_only_hint` 按 risk 映射；守卫测试覆盖「扩展不得影子内置」。
11. [x] MCP 新工具（`list_extensions` / `search_extension_market` / `install_extension`(confirm) / `uninstall_extension` / `set_extension_enabled` / `reload_extension`）（`list_extensions` 已上；search / install / uninstall 留到下一轮）：`list_extensions`、`search_extension_market`、`install_extension`
        （`confirm=true` 高风险）、`uninstall_extension`、`set_extension_enabled`。
12. [x] 署名（`Agent("ext:<包名>")`：不改事件 schema）：扩展发起的写操作在撤销栈/事件流里署名。
13. [x] `server/rpc.rs` 注册新命令（浏览器/无头模式同路径）。

## P2 — Client 侧

14. [x] `src/api/tauri.ts` + `types.ts` + `mock.ts`：扩展相关命令与类型。
15. [x] `src/stores/extensions.ts`：已装清单、加载状态、Client 模块加载器
        （`blob:` 动态 import，`@vite-ignore`）、面板组件表、错误隔离。
16. [x] 启动时（`App.vue` onMounted）加载 Client 扩展；监听 `extensions://changed` 重载。
17. [x] `layout.ts`：`PANES` 变成「内置 + 动态面板」；`openTab` / 校验接受扩展面板 id。
18. [x] `RightPanel.vue` / `PanelTabs.vue`：按 id 渲染 `<component :is>`，图标键表合并。
19. [x] `window.__AIDAWC__` 注入（Vue 运行时、invoke、事件、i18n、主题）。

## P3 — 市场与安装

20. [x] `market.rs`：索引拉取（ureq + 代理 + 超时 + 上限）、JSON 校验（整份拒绝 + 路径诊断）、
        磁盘缓存（TTL / 保留上次成功）、多源合并（先到先得）。
21. [x] `index_format.rs`：`format: 1` 的完整读取器（LocalizedText、icon 内联/URL、dist-tags、
        versions、engines.aidaw、sortKeys）；单测含「每种违规都拒绝」。
22. [x] `search.rs`：NFKC + 小写 + 中日韩双字；权重 3/2/1；排序与分页；单测。
23. [x] `npm.rs`：registry 元数据、版本选择（engines.aidaw）、tarball 下载、sha512 integrity 校验、
        tar.gz 解压（剥离 `package/`）、路径安全。
24. [x] `installer.rs`：安装/更新/卸载/本地目录导入/`.tgz` 导入；原子落盘与回滚；
        版本兼容复查；状态更新；热加载。
25. [x] 单测：坏 integrity、缺 `package/` 前缀、路径穿越、版本不匹配、空索引、坏索引。

## P4 — 界面

26. [x] `components/workspace/panes/ExtensionsPane.vue`：发现 / 已安装 / 设置 三标签。
27. [x] `SettingsView.vue`「扩展」一节（索引源、registry/镜像、刷新）。
28. [x] i18n `extensions.*`（zh / en）。
29. [x] 安装确认（`window.confirm` 列出包名 / 版本 / 索引来源；MCP 侧要求 `confirm=true`）（包名、版本、来源、索引 URL、capabilities/permissions）。

## P5 — 文档与分发

30. [x] `skills/aidaw/references/extensions.md`：扩展开发指南（契约、打包、调试、发布），
        并挂进 `SKILL_DOCS`（`aidaw_help("extensions")`）。
31. [x] `examples/extensions/hello/`：测试扩展（Host 工具 + Client 面板），随仓库跑测试用。
32. [x] `tools/build-extension-index.mjs`：从 `plugins.txt` 生成 `index.json`（商店仓库用）。
33. [x] `docs/extensions-index.md`：商店仓库怎么维护、字段说明、发布流程。
34. [x] 默认索引常量（`DEFAULT_INDEX_URL` 指向 `heshengtao/ai-daw-plugins`；商店仓库已建）（`DEFAULT_INDEX_URL`）留占位并标注替换点。

## 验证

- `cargo test -p ai-daw-extensions` / `cargo test -p ai-daw-daemon`（相关模块）
- `npm run typecheck`（或仓库既有前端检查脚本）
- 手动：装 `examples/extensions/hello` → 工具与面板出现 → 重载生效 → 卸载干净
- 无头：`cargo run --bin aidawd` 下同一扩展可装载、工具可调

## 已知后置项

- 供应商/内容包接进 `cloud_music` / `models` / `cue_sheet` 执行路径（v2）
- 异步 Host API（Promise handler）
- 共享宿主进程 / 独立 helper 进程隔离（对齐 roadmap E2）
- 编辑距离搜索、扩展间依赖

## 本轮验证（2026-11-18 夜）

- `cargo test -p ai-daw-extensions`：**76 passed**（包格式 / 注册表冲突 / 宿主超时与崩溃 / 市场解析与搜索 / integrity 与解包 / 安装回滚 / range 适配）。
- `cargo check -p ai-daw-daemon`（桌面）与 `--no-default-features`（无头）：通过。
- `npx vue-tsc --noEmit` + `npx vite build`：通过。
- **无头真机端到端**（`aidawd --data-dir /tmp/aidaw-ext-test --port 28431`）：
  1. `extensions_rpc install_local` 安装 `examples/extensions/hello` → `{name, version}`；
  2. `list` → `state=loaded`、`tools=[hello_summary]`、`panes=[hello-pane]`（带 zh/en 标题）；
  3. `client_manifest` / `client_source` → 前端能拿到 Client ESM 源码；
  4. 本地 `index.json` → `market` 返回 `update-available`（已装 0.1.0 vs 0.2.0）与 `incompatible`
     （`>=2.0.0` 带原因）；中文「示例」搜索命中；坏索引报 `$.plugins[0].name`。
- daemon 集成测试 `extension_tools_are_dispatched_and_listed`：装载 → `mcp::tools::dispatch`
  派发 → 扩展内 `aidaw.call` 经桥执行 → 停用后注册回收。
- 过程中发现并修掉一个真 bug：宿主命令返回的 JSON 文本在信封里被当成**字符串**再包一次，
  导致扩展拿到的是字符串而不是对象（`host.rs` 的 `call` 现在先解回值再嵌入）。

**未验证**：真实 GUI 里 blob ESM 面板的渲染（要跑起 Tauri/浏览器界面，本轮没有浏览器自动化）；
面板组件的错误隔离（`onErrorCaptured`）也还没加。

## 下一轮

1. MCP 工具补齐：`search_extension_market` / `install_extension`（`confirm=true`）/ `uninstall_extension` / `set_extension_enabled`。
2. 设置页「扩展」一节入口 + 安装确认对话框（包名 / 版本 / 来源 / 索引 URL / 能力）。
3. 面板错误隔离：坏组件显示错误占位而不是拖垮整页。
4. 文档与分发：`skills/aidaw/references/extensions.md`、`tools/build-extension-index.mjs`、
   `docs/extensions-index.md`、替换默认索引地址。

## 第二轮验证（2026-11-18 深夜）

- **占位索引**：`DEFAULT_INDEX_URL` 里还有 `<owner>` 时视为「未配置」，默认索引源为空，
  「发现」页显示空态而不是每次刷新都报 `invalid uri`。
- **Client 契约自检**（`npm run selftest:extensions`，不需要界面）：
  `examples/extensions/hello/dist/client.js` 能 `export default install(api)`、注册的面板 id 与
  `aidaw.json` 一致、组件能被 Vue SSR 渲染出内容。
- **真 registry 网络测试**（`cargo test -p ai-daw-extensions -- --ignored real_registry`）：
  解析 → 下载 → sha512 integrity 校验 → 解包全通；主源不可达时回退到 `registry.npmmirror.com`。
- **MCP 真机**（`aidawd` + curl 走完整 MCP 握手）：
  - `tools/list` 返回 **151** 个工具，含扩展的 `hello_summary`（schema 原样透传）；
  - `tools/call hello_summary` 成功：扩展里 `aidaw.call("get_project_overview")` 落到 MCP
    派发表（不在 `/rpc` 命令表里），返回轨道数；
  - 扩展自调用被宿主拒绝（不再可能死锁），0.01 s 返回明确错误。
- **热更新与更新**（同一台无头 daemon）：
  - 改已装包的 `host.cjs` → `reload` → `list` 里的工具描述立即变化（未重启进程）；
  - `install_local` 0.2.0 覆盖 0.1.0 → 版本与状态更新，磁盘只留一个版本目录。

## 第三轮验证（真浏览器，2026-11-18 深夜）

用 headless Chrome + CDP 跑了真实界面（`tools/ui-smoke-extensions.mjs`，浏览器版连 `aidawd`）：

- 「扩展」面板能作为右栏标签打开，显示 发现 / 已安装 / 设置；「已安装」里能看到示例扩展的版本、
  来源与「运行中」状态；
- **扩展贡献的面板**（`hello-pane`）能作为标签打开并渲染：说明 webview 里的 **blob ESM 动态
  import → registerPane → Vue 渲染**整条链路是通的；
- 面板里 `api.invoke("get_project_overview")` 的返回值（工程概览 JSON）真的显示出来了
  ——**daemon → webview 的数据通路**也通了。

过程中发现并修掉两个真问题：

1. **Client 侧 `invoke` 走错了通道**：原先映射到 `extensions_rpc(op=cmd)`，而 `get_project_overview`
   之类其实是 MCP 工具、不是扩展操作 → 面板显示「未知的扩展操作」。现在统一走
   `extensions_rpc("call_tool", {name, args})`，daemon 侧先查 `/rpc` 命令表、未命中落到 MCP 工具
   （与 Host 侧 `aidaw.call` 同一条兜底路径）。
2. **重启后扩展标签丢失**：`sanitizeTabs` 启动时按「是否已注册」过滤，而扩展面板要等装载后才注册。
   现在布局保留未知 id，标签栏只显示「此刻能渲染」的（`visibleTabs`），面板体给出
   「这个面板现在打不开」的兜底文案。

## 交付状态（2026-11-18）

已完成并验证：npm 分发 + 本地导入、镜像回退、Host/Client 双侧装载与热更新、冲突拒绝与隔离、
市场索引（format 1）读取/校验/缓存/多源/搜索、安装 / 更新 / 卸载、MCP 工具面（6 个扩展管理工具 +
扩展工具并入 `tools/list`）、扩展面板与设置页入口、示例扩展、索引生成器与文档。

**商店仓库**：<https://github.com/heshengtao/ai-daw-plugins>（public，`index.json` 目前
`plugins: []`；GitHub Pages 已开启）。`DEFAULT_INDEX_URL` 指向
`https://heshengtao.github.io/ai-daw-plugins/index.json`（实测 raw 地址在部分网络下会被
TLS reset、Pages 正常）。用 `tools/build-extension-index.mjs` 生成，流程见仓库 README。

**未覆盖**：Tauri 桌面壳里的真机目视验证（本轮用 headless Chrome 跑的是浏览器版，两者共用同一份
Vue 包与同一套 IPC；blob ESM import 在 WKWebView 上未实测）。

## 商店仓库（2026-11-18 晚）

已建并上线：<https://github.com/heshengtao/ai-daw-plugins>（public）

- `index.json`：`format: 1`，`plugins: []`（空索引是合法状态）
- `plugins.txt`：维护清单（暂无条目）
- `README.md`：怎么发布扩展、怎么重新生成索引、安全说明
- `LICENSE`（MIT）、`.gitignore`
- `.github/workflows/validate.yml`：push / PR 改 `index.json` 时校验 format / title / versions /
  `source.type` / `engines.aidaw`（本地用同一段脚本验过：`✓ index.json 合法（0 个扩展）`）
- **GitHub Pages 已开启**（从 `main` 根目录发布），默认索引地址因此是
  `https://heshengtao.github.io/ai-daw-plugins/index.json`

实测：`raw.githubusercontent.com` 在本机网络下对 Rust 客户端（native-tls）会被 reset
（curl 能过），而 `*.github.io` 正常——所以默认改用 Pages，raw 作为备选写在文档里。
新起的 `aidawd` 用默认配置就拉到了它：`title=AI DAW 扩展`、`error=None`、`0 entries`。

## 第四轮：UI 对齐 AI CUT + 改名「应用扩展」（2026-11-18 夜）

参考 `heshengtao/ai-cut` 的 `ExtensionsPane`（下载管理器视觉基线）重做了 AI DAW 的扩展面板：

- **两标签**：已安装 / 发现（带计数），头部「标题 + 数量」+ 刷新；「导入本地目录」放在标签栏右侧。
- **卡片**：名称 + `vX` + 运行状态 + 来源（托管安装 / 本地目录）徽章、来源路径、错误行、
  工具/面板计数、操作（更新到 x / 重新装载 / 启用|停用 / 卸载）。
- **发现**：搜索框 + 索引状态胶囊（在线 / 缓存 / 不可达 / 未配置）+ 「索引源设置」胶囊，
  卡片带已安装 / 有新版本徽章、作者、说明、安装/更新与主页入口。
- **设置移出面板**：索引源、npm registry / 镜像改为「设置 → 应用扩展」一节（`settings.section.extensions`），
  面板只留跳转胶囊；`SettingsView` 深链 `?section=extensions` 已验证。
- **改名**：面板与设置统一叫「应用扩展」（en: App extensions），与 VST3/CLAP 的「插件」区分；
  skill 文档、spec、plan 同步。

顺带修掉一个真 bug：`/rpc` 静态回退把 `/settings` 这类无扩展名路径按请求路径猜 MIME，
得到 `application/octet-stream`，浏览器当下载处理（页面白着不动）。现在按**实际服务的文件**
（index.html）算 MIME。

验证：headless Chrome 走了「已安装 → 发现 → 设置 → 扩展贡献的面板」四个界面并截图；
`vue-tsc` + `vite build`、`cargo test -p ai-daw-extensions`（78）与 daemon（254）全绿。

## 商店仓库（两个）

| 应用 | 仓库 | 默认索引 |
| --- | --- | --- |
| AI DAW | <https://github.com/heshengtao/ai-daw-plugins> | `https://heshengtao.github.io/ai-daw-plugins/index.json` |
| AI CUT | <https://github.com/heshengtao/ai-cut-plugin> | `https://heshengtao.github.io/ai-cut-plugin/index.json` |

两个仓库都开了 GitHub Pages，索引为空（`plugins: []`），都带 README / 清单 / validate workflow。
AI CUT 也同步把界面里的「插件扩展」改名为「应用扩展」（zh）／「App extensions」（en），
`docs/plugins.md` 与三个 examples README 一并更新；`crates/daemon` 的 22 个 market 测试通过。

## 第五轮：两个静态索引仓库也上 Pages（2026-11-18 深夜）

同样的网络问题（raw.githubusercontent.com 对 Rust 客户端会被 TLS reset）也影响另外两个索引仓库，
所以一起改成 GitHub Pages 静态托管：

| 仓库 | Pages 索引 | 消费者 |
| --- | --- | --- |
| `heshengtao/ai-daw-soundfont-registry` | `https://heshengtao.github.io/ai-daw-soundfont-registry/registry.json` | AI DAW「下载管理器 → 音色库」 |
| `heshengtao/ai-cut-ofx-registry` | `https://heshengtao.github.io/ai-cut-ofx-registry/registry.json` | AI CUT「OFX 插件」下载管理器 |

- 两个仓库都加了 `.nojekyll`（**通过 GitHub API 提交**，本地克隆需要 `git pull` 一次）、开启 Pages
  （`main` 根目录），实测 `registry.json` 200 且 JSON 合法（70 个音色库 / 112 个 OFX 插件）。
- AI DAW：`soundfont_store.rs` 的索引源顺序改为 **Pages → jsDelivr → raw**；
  `tools/fetch-soundfont-registry.mjs` 的抓取地址同步。
- AI CUT：`plugins.rs` 的 `DEFAULT_REGISTRY_URL` 改为 Pages（镜像/自定义地址仍然优先）；
  `scripts/sync-ofx-registry.mjs` 同步。
- 两个索引仓库的 README「加载顺序」也改成 Pages 优先。

验证（AI DAW，真机）：`aidawd` 全新数据目录 → `soundfont_store_search` 返回结果并写出
`soundfont-registry.json`（70 件），说明命中的是 Pages 源；再故意把环境变量
`AIDAW_SOUNDFONT_REGISTRY` 指到一个不可达地址，仍然通过 Pages 回退成功并落缓存。
AI CUT 侧 `crates/daemon` 的 `plugins` 14 个测试通过；其 HTTP 栈是 reqwest(rustls)，未做联网实测。

## 第六轮：音频插件注册表也上 Pages + 索引仓库 README 提交（2026-11-18 深夜）

新仓库 **<https://github.com/heshengtao/ai-daw-plugin-registry>**（public，GitHub Pages 已开）：

- `plugin-registry.json`：当前 Open Audio Registry 快照（563 个插件 / 24 个 app，1.8 MB）
- `sync.mjs` + `.github/workflows/sync.yml`：每周一 + 手动触发，从上游拉一次、有变化才提交
- `.nojekyll`、README、LICENSE

默认索引地址（`crates/daemon/src/manager.rs` 的 `REGISTRY_URL`）改为

```
https://heshengtao.github.io/ai-daw-plugin-registry/plugin-registry.json
```

`tools/fetch-plugin-registry.mjs` 仍然只生成**随包兜底快照**（注释已说明运行时不读它）。

真机验证（AI DAW，全新数据目录）：`plugin_manager_registry(force=true)` 返回
`from_cache=false`、460 个本平台可装插件，耗时约 3 s，并写出缓存
`<数据目录>/cache/plugin-manager/registry.json`。

> 注意：未压缩的 1.8 MB 直接下载在这条网络下要 40–50 s；应用侧 ureq 开着 `gzip`，
> Pages 会返回 284 KB 的压缩体（约 3–5 s），所以体验没问题。用 curl 测速记得加 `--compressed`。

顺带：两个索引仓库（soundfont / OFX）的 README 已提交并推送（`.nojekyll` 与 README 各一条提交）。
这台机器上 git 走 HTTP/2 会挂，加 `-c http.version=HTTP/1.1` 就正常——以后推送都可以这样。
