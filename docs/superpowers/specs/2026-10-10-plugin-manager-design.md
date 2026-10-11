# 插件管理器（E1）— 设计

> **状态（2026-10-10）**：**已完成**（macOS 真机验证；Windows / Linux 逻辑就绪，待对应平台验证）。
> 实现：`src-tauri/src/manager.rs`、`crates/host-vst3/src/scan.rs`（额外扫描目录）、
> `src/stores/manager.ts`、`src/components/workspace/panes/ManagerPane.vue`。
> 逐任务记录见 `plans/2026-10-10-plugin-manager.md`。

## 1 背景

Roadmap **Stage E · E1** 原计划「前端用 `@open-audio-stack/core` 搜索，Rust 侧安装」。
核对后有两个现实约束：

1. **`@open-audio-stack/core` 是 Node CLI 库**（依赖 `fs-extra` / `adm-zip` / `7zip-min` /
   `sudo-prompt` / `inquirer`），在 Tauri 的 webview 里跑不起来。它的浏览器构建只提供类型与
   少量纯函数，真正干活的「下载 / 解压 / 安装」仍是 Node API。
2. 所以**照 Roadmap 的分工来**：前端只管**浏览与筛选**，Rust 侧管**下载 / 校验 / 解压 / 安装**。
   注册表是一份**静态 JSON**（`plugins/apps/presets/projects`），前端/Rust 直接读即可，不需要那个库。

同时把功能从「商店」扩成**插件管理器**：**安装 / 导入 / 更新 / 删除**四件事，都在一个面板里。

## 2 目标与非目标

**目标**

- 浏览开放注册表里**本 DAW 能加载**的插件（只保留含 `vst3` / `clap` 的文件），可按名称 /
  作者 / 关键词搜索，按乐器 / 效果与「是否兼容本机」筛选。
- **安装**：下载 → 校验 sha256 → 解压归档 → 放进托管库 → 自动重扫。
- **导入**：选本地 `.vst3` / `.clap`，复制进托管库。
- **更新**：同一 slug 有更新版本时一键替换（先删旧文件再装）。
- **删除**：只删**本管理器装的**东西，不碰系统目录里的插件。
- 设置页「插件」一节给出入口。

**非目标（v1 明确不做）**

- **不自动执行 `installer` 类型的安装器**（dmg / pkg / exe / msi / deb / rpm / AppImage）：
  只提供「打开下载页」，由用户在浏览器里下载并自己装。这是安全边界，不商量。
- **不做签名 / 公证校验**：只校验注册表给出的 sha256（数据完整性），不做代码签名。
- **不做断点续传 / 增量更新**：整包重下。
- **不支持 AU / LV2 / VST2**：本宿主只载 VST3 与 CLAP（改了也没用）。
- 不做只读的「全网插件百科」（那是 PluginDB 那类发现层的活）。

## 3 关键决策

| 决策 | 结论 | 理由 |
| --- | --- | --- |
| 是否引入 `@open-audio-stack/core` | **不引入** | Node 库，webview 跑不了；我们只需要读它的 JSON 数据源 |
| 注册表数据源 | 静态 JSON（`open-audio-stack-registry`），可用 `AIDAW_STORE_URL` 覆盖 | 公开、稳定、无需自建后端 |
| 缓存 | 磁盘缓存到应用缓存目录，TTL 6 小时；面板可「刷新」强制联网 | 那份 JSON ~1.9 MB，不必每次打开都重下 |
| 下载客户端 | `ureq` + `native-tls` + `gzip`（**复用依赖树里已有的 TLS**，不引 rustls） | 只发 GET；已在 `Cargo.lock` 里，不新增 TLS 依赖树。`gzip` 让 1.9 MB 的注册表按约 280 KB 传 |
| 首次加载体验 | 启动时**后台预热**注册表缓存（`warm_registry_cache`，失败静默） | 注册表首次拉取在慢网络下要十几秒；预热后点开「可下载」通常直接命中缓存、秒开 |
| 装到哪 | **托管库**：应用数据目录 `plugins/vst3`、`plugins/clap` | ① 用户级可写，不要管理员权限；② 卸载干净；③ 不污染系统目录 |
| 宿主怎么发现 | `host-vst3` 新增 `scan_with` / `scan_catalog_with`（额外扫描目录）；启动扫描与「重新扫描」都带上托管库 | 托管库不在系统标准路径下，必须显式传入 |
| 解压 | `zip`（deflate）+ `tar` + `flate2`（tar.gz）；防目录穿越 + 体积上限 | 纯 Rust、跨平台、许可宽松 |
| 安全边界 | 下载后**校验 sha256 与大小**；解压时挡 `..` / 绝对路径；`installer` 类型拒绝自动安装 | 装第三方二进制是高风险动作，能挡的都挡 |
| 更新语义 | 同一 key `slug`：先删旧记录里的文件，再装新的 | 避免旧版本文件残留 |
| 删除范围 | **只删 manifest 里有记录的**（本管理器装的） | 系统目录 / 用户自己装的插件不归我们管 |
| 「可更新」怎么判 | 已装记录的 `version` ≠ 注册表最新 `version` | 版本号来自注册表，字符串比较即可 |
| 安装/导入后 | 自动**重扫**插件库并广播 `engine://plugins` | 「插件」面板立刻看到新插件，不用手动重扫 |
| 打开下载页 | Rust `open` / `xdg-open` / `cmd /C start`，**仅 http/https** | 沿用 `cli.rs` 的做法，零 capability 变更 |

## 4 结构

**Rust（`src-tauri/src/manager.rs`）**

- 注册表：`RawRegistry`（只 `plugins`）→ `normalize()` → 精简的 `RegistryPlugin[]`
  （只留含 `vst3`/`clap` 的文件，并算好 `compatible`）。
- 托管库：`ManagedDirs`（`plugins/{vst3,clap,downloads,staging}` + `manifest.json`）。
- 安装：`download_verified()`（流式下载 + 边下边算 sha256 + 校验大小）→ `extract_archive()`
  （zip / tar.gz / 裸文件）→ `collect_plugin_paths()`（找 `.vst3` bundle 与 `.clap`）→
  `move_path()` 进托管库 → 写 manifest → `rescan_with_managed()`。
- 导入 / 删除 / 打开下载页：`import_local()` / `remove_installed()` / `open_external()`。
- 命令：`plugin_manager_registry` / `_installed` / `_install` / `_import` / `_remove` / `_open`。
- 平台：`platform()`（`mac`/`win`/`linux`）、`arch()`（`arm64`/`x64`/…）——兼容性判定的依据。

**宿主（`crates/host-vst3/src/scan.rs`）**

- 新增 `scan_with(vst3_extra, clap_extra)`、`scan_catalog_with(...)`；`scan_vst3` 用
  `Vst3Host::builder().add_scan_path(...)` 接收额外目录，`scan_clap` 把额外目录并入。

**前端**

- `api/types.ts` + `api/tauri.ts` + `api/mock.ts`：新增 `manager*` 六个调用与对应类型。
- `stores/manager.ts`：注册表 + 已装清单 + 外部清单 + 安装/导入/删除动作。
- `components/workspace/panes/ManagerPane.vue`：三个标签（**已安装** = 本管理器装的 / **外部** =
  你自己装的、只读 / **可下载** = 注册表），右上角一个**图标 + 文字的「导入插件」**按钮。
- `stores/layout.ts` 新增 pane `manager`；`PanelTabs` / `RightPanel` 加图标与渲染分支；
  `SettingsView` 的「插件」一节加入口。
- `api/dialogs.ts` 新增 `pickPluginToImport()`。

## 5 数据流

```
面板「可下载」→ managerRegistry() → Rust 拉/缓存注册表 → normalize（只留 vst3/clap）
面板「安装」   → managerInstall()  → Rust 下载 → 校验 sha256 → 解压 → 进托管库 → 写 manifest
                                                        └→ rescan_with_managed() → engine://plugins
「插件」面板（stores/plugins）← engine://plugins ← 扫描（含托管库）
```

装了插件后它是否出现在轨道可选项里，取决于宿主扫描是否认得它——这正是把托管库接进
`scan_catalog_with` 的原因。

## 6 网络与镜像（国内可用性）

注册表（GitHub Pages 的 `*.github.io`）与插件二进制（GitHub Releases）在国内都可能**很慢或不通**。
基础和「能搜到 / 能下下来」是两件事，所以分开处理，**都在设置页配置，且不提供任何默认镜像/代理地址**
（国内可用的镜像与加速站五花八门、稳定性各异，写死一个默认值早晚会失效）。

配置落在应用配置目录的 `store.json`（`StoreNetwork`）：

| 字段 | 作用 |
| --- | --- |
| `registry_mirrors: string[]` | 注册表镜像，**按顺序在官方源之前**尝试；空 = 只用官方源 |
| `download_prefix: string` | 下载代理前缀，**只重写 GitHub 域名**的地址；空 = 直连。可含 `{url}` 占位符，否则直接拼在前缀之后 |

**注册表多源回退**（`load_registry`）：新鲜缓存 → 依次尝试各源 → **全部失败时退回旧缓存**（过期也比没有强）。
拉取用的 agent 连接级超时设短（resolve 5s / connect 10s / recv 20s），遇到被墙的源能**快速失败、切下一个**；
下载用的 agent 只留整体超时（大文件、慢链路）。

**下载走代理也安全**：无论走不走代理都**校验 sha256**，字节被改会被拒——所以用第三方加速站不会降低完整性保证。

`registry_url` 旧的环境变量 `AIDAW_STORE_URL` 仍保留，并新增 `AIDAW_STORE_URLS`（逗号分隔）；
两者**完全取代**配置列表，供测试 / 无界面场景使用。

**典型用法**：把官方注册表镜像到 Gitee（raw 地址），填进「注册表镜像」；再填一个 GitHub 加速站作「下载代理前缀」。
注意：镜像**注册表 JSON 只解决「能搜到」**，插件二进制仍来自 GitHub Releases，要「能下下来」必须配下载代理。

## 7 已知取舍与缺口

- **没有默认镜像 / 代理**：见上，交给用户填。代价是「不配就没有加速」。
- **`ureq` 的 TLS provider 必须显式指定**：`ureq::tls::TlsConfig` 的 provider **默认是 Rustls**；
  本项目只编进了 `native-tls`（不引 rustls 那棵树），所以 agent 构造里必须显式设
  `TlsProvider::NativeTls`，否则首次 HTTPS 请求会 panic：
  `uri scheme is https, provider is Rustls but feature is not enabled: rustls`。
  已用 `#[ignore]` 的联网冒烟测试 `manager::tests::fetches_registry_over_https` 守住这条。
- **Windows / Linux 未验证**：`cfg` 分支在 macOS 上不参与编译（仓库无 CI）。托管库路径是跨平台的，
  但 VST3 / CLAP 的落盘位置是否都能被各自的扫描器读到，需要对应平台各跑一次。
- **只按扩展名判安装器**：`kind_is_installer()` 看 URL 扩展名。注册表还有 `type: installer`
  字段，但按扩展名判断对「URL 是 `.dmg` 却标 archive」这种脏数据更稳。
- **注册表全量拉取**：一次 1.9 MB（gzip 后约 280 KB）。将来若注册表继续膨胀，再考虑增量 / 只拉 `plugins` 子集。
- **归档内的选择**：只挑 `.vst3` / `.clap`。若归档把它们埋得很深（例如某层目录名就以 `.vst3` 结尾），
  `collect` 会把它当插件收下——绝大多数归档结构都直接可判。
- **不校验签名**：sha256 来自注册表本身；注册表被篡改则无法防。要更强保证需要引入签名 / 公证。
- **版本比较是字符串**：`"1.10" != "1.9"` 会判为「可更新」，但注册表的 `version` 是顶层「最新」，
  实际语义正确。

## 8 验收

1. 打开「插件管理器」→「可下载」，能列出注册表里含 VST3/CLAP 的插件，搜索与筛选正常。
2. 对一个 `archive` 且兼容本机的条目点「安装」→ 下载、校验、解压、进托管库 → 「插件」面板出现该插件。
3. 对一个 `installer` 条目点「打开下载页」→ 系统浏览器打开官方下载页；**不下载、不执行**。
4. 「已安装」里能看到记录；有更新版本时点「更新到 x.y.z」→ 替换成功，版本号更新。
5. 「导入插件…」选一个本地 `.vst3` → 出现在「已安装」与「插件」面板。
6. 「已安装」里点删除（二次确认）→ 文件删除、记录消失、「插件」面板同步消失。
7. 断网打开面板 → 有缓存则照常；无缓存给出错误横幅，不崩溃。
8. `sha256` 对不上的下载 → 报错且**不安装**（可用 `AIDAW_STORE_URL` 指向本地桩数据验证）。
9. 设置页填一个注册表镜像 → 刷新后走镜像；填错地址 → 回退官方源（或旧缓存），列表照常。
10. 「外部」标签列出**你自己装**的插件（系统目录扫描到的，只读、无删除按钮）；本管理器装的出现在「已安装」。

## 9 下载健壮性（2026-10-10 补，源自真机反馈）

真机上出现过两次下载失败：`io: Peer disconnected`（传输中被对端掐断）与 `timeout: connect`
（30 s 内连不上）。**两者都是传输层的，不是逻辑 bug**——但它们暴露了当时下载路径的脆弱：
只有一个地址、只试一次、没有续传，而且报错**不带地址**，用户根本无从判断该改设置还是稍后再试。

现在的下载路径（`download_candidates` / `download_verified` / `download_from`）：

- **候选地址**：配了代理前缀就是 **先代理、再原始 GitHub 地址**；没配（或非 GitHub）只有原始地址。
  「代理站抖动」与「GitHub 直连被墙」是国内最常见的两种失败，两条路都留着才不会一条不通就整体失败。
- **重试**：单个地址最多 3 次，退避 0.6 s / 1.2 s。
- **续传**：`Range: bytes=N-`。本地已有残缺文件且注册表给了总大小时，从断点继续——
  大插件下到 80% 被掐断不必从头再来。服务器忽略 Range（回 200）就自动退回从头下载。
- **错误分类**：`Transient`（连不上 / 被断开 / 5xx）→ 重试同一地址；`Permanent`（4xx、
  大小或 sha256 对不上）→ 不重试同一地址，直接换下一个源（内容不会因为重试而改变）。
  大小 / 哈希不符时**删掉残缺文件**，否则下一轮会把它当断点拼出一个已经错了的文件。
- **错误信息带地址**：`https://… → io: Peer disconnected`，并汇总「N 个源 × 最多 3 次都试过」。

**进度**：`engine://manager` 事件（`ManagerProgress`：`slug` / `phase` / `received` / `total`），
节流到约 120 ms 一次；阶段为 `downloading` → `extracting` → `installing`，失败补一条 `error`。
界面在**卡片下边缘**画进度条（与 AI 面板同一套观感）；`total` 未知或处于解压 / 安装阶段时
画**不确定态**长条。

**复制下载链接**：每行文件一个链接按钮，复制的地址由后端 `plugin_manager_resolve_url`
按当前配置解析——保证「复制到的」与「真去请求的」是同一个 URL，而不是在前端重写一遍规则。

**仍未做**：取消下载（要加取消标志与循环内的检查）；进度只在单个安装上（界面用 `busy` 串行化）。

## 10 网络设置（2026-10-10 补，独立成一节）

真机反馈：「我开了代理，但软件似乎没走代理下载」。**这是一个真 bug，而且有两层**：

1. **只认环境变量不够**。原来的 agent 从不设代理，靠 ureq 的隐式行为读 `HTTP_PROXY` 之类。
   可 macOS 上的「系统代理」（Clash / Surge 那类）写在 SystemConfiguration 里，而**从 Dock 启动的
   GUI 应用不继承 shell 的环境变量**——实测这台机器 `scutil --proxy` 里 HTTP/HTTPS/SOCKS 都指向
   `127.0.0.1:7892`，而 shell 里一个代理环境变量都没有。于是 App 直连，撞上被墙的 github.com。
2. **「下载代理前缀」是 URL 重写，不是 HTTP 代理**。它是把 `github.com/...` 换成加速站地址，
   和「让流量经过 127.0.0.1:7892」是两回事。文档里以前没说清，容易被误当成代理开关。

**现在的设计**（设置页新增独立的「网络」一节，原「网络与镜像」从「插件」一节搬了过来）：

- **三种代理方式**（`ProxyMode`，默认 `follow system`）：
  - `direct` 不使用 —— 显式 `.proxy(None)`，哪怕系统开着也不走；
  - `system` 跟随系统 —— 先 `Proxy::try_from_env()`，再问操作系统；
  - `manual` 手动 —— 用户填的地址，`http://` / `https://` / `socks5://` 都支持
    （为此给 ureq 打开了 `socks-proxy` 特性）。
- **系统代理探测**：macOS = `scutil --proxy`（优先 HTTPS → HTTP → SOCKS，都按 CONNECT 代理用）；
  Windows = 注册表 `HKCU\...\Internet Settings` 的 `ProxyEnable` / `ProxyServer`；
  Linux = 只认环境变量（桌面环境各写各的 gsettings，暂不处理）。
- **显式设置代理**，不依赖 ureq 的隐式默认值：「不走代理」就是真的不走。
- **设置页显示「当前实际走」**（`plugin_manager_proxy_status`，只解析、不联网）——一眼看出
  到底解析到了什么；没解析到就是「未检测到系统代理 → 直连」。
- **「测试」按钮**（`plugin_manager_probe_proxy`）真的发一次请求，把「解析到哪个代理」和
  「这个代理能不能用」分开报——代理软件退出后经常把系统代理设置留在原地，这时解析得到、
  但连不上，正是最容易被误判成「代码坏了」的情形。
- **测试用的是输入框里的值**（不写盘）；「当前实际走」显示的是**已保存**的配置，两者语义不同。
- **不静默绕过代理**：配了代理但代理不通就报错，而不是偷偷直连——绕过代理可能违背用户的
  隐私/合规意图。错误信息里带上「解析出的代理来源」，便于定位。
- 代理设置**同时服务于插件与（将来的）模型下载**——两者共用同一份 `store.json`。

**平台验证状态**：macOS 的 `scutil` 解析有单测（样例取自真实输出），端到端那条
（`follows_the_system_proxy`，`#[ignore]`）在本机验证过**解析正确**；当时代理软件没在跑
（7892 无人监听），所以连接失败——这恰好演示了「解析得到但连不上」这个分支。
Windows 的注册表解析**未在 Windows 上验证**。

