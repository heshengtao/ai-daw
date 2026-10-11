# 插件管理器（E1）— 实施计划

- 日期：2026-10-10
- 状态：✅ **完成**（macOS 真机验证；Windows / Linux 逻辑就绪，待对应平台验证）
- 依据：`specs/2026-10-10-plugin-manager-design.md`；`ROADMAP.md` §5 **Stage E · E1**。
- **范围调整**：原 E1 叫「插件商店（前端搜索 + Rust 安装）」。核对后把功能扩成**插件管理器**：
  **安装 / 导入 / 更新 / 删除**四件事都在一个面板里；并**不引入** `@open-audio-stack/core`
  （Node 库，webview 跑不了），前端直接读注册表 JSON。

## 1 任务

1. ✅ **治理与安全边界**：只自动安装 `archive`（zip / tar.gz / 裸 `.clap`）——下载后校验 **sha256 + 大小**，
   解压挡 `..` / 绝对路径并设体积上限；`installer` 类型（dmg / pkg / exe / msi / deb / rpm / AppImage）
   **绝不自动执行**，只「打开下载页」。
2. ✅ **托管库**：装到应用数据目录 `plugins/{vst3,clap}`（非系统目录），配 `manifest.json` 记录
   「装了什么、什么版本、装在哪」。删除只删 manifest 里的东西。
3. ✅ **`src-tauri/src/manager.rs`**：注册表拉取 + 磁盘缓存（6h）/ 归一化；
   下载（`ureq` + `native-tls`）/ 校验（`sha2`）/ 解压（`zip` / `tar` + `flate2`）；
   安装 / 导入 / 删除 / 打开下载页；6 条 Tauri 命令。
4. ✅ **宿主发现托管库**：`crates/host-vst3/src/scan.rs` 新增 `scan_with` / `scan_catalog_with`
   （VST3 用 `add_scan_path`、CLAP 并入额外目录）；启动扫描与「重新扫描」都带上托管库。
5. ✅ **接线**：`lib.rs` 注册 `manager` 模块与 6 条命令；`spawn_plugin_scan` 改走
   `manager::rescan_with_managed`；`commands::rescan_plugins` 同样带上托管库。
6. ✅ **前端**：`api/types.ts` + `api/tauri.ts` + `api/mock.ts`（假注册表与假安装记录）；
   `api/dialogs.ts` 的 `pickPluginToImport()`；`stores/manager.ts`；
   `components/workspace/panes/ManagerPane.vue`——**三个标签**：已安装（本管理器装的）/ 外部
   （用户自己装的、扫描到的、只读）/ 可下载（注册表）。外部清单由 `plugin_manager_foreign`
   给出（路径不在托管库里的都算「外部」）。右上角是**图标 + 文字的「导入插件」**按钮（空态里也有一个）。
7. ✅ **面板接线**：`stores/layout.ts` 新增 pane `manager`；`PanelTabs.vue` / `RightPanel.vue`
   加图标与渲染分支；`SettingsView.vue`「插件」一节加入口。
8. ✅ **文案**：i18n zh / en 各一组 `manager.*`、`panes.manager*`、`settings.plugins.manager*`、
   `dialog.pluginFilter`。
9. ✅ **网络与镜像（国内可用性）**：注册表**多源回退**（配置的镜像 → 官方源 → 退回旧缓存），
   拉取 agent 连接级超时设短以便快速切源；下载**代理前缀**（只重写 GitHub 地址，可 `{url}` 占位）；
   设置页「网络与镜像」一组（注册表镜像 / 下载代理前缀 / 保存），落 `store.json`。
   **不提供任何默认镜像 / 代理地址**。新增命令 `plugin_manager_network` / `plugin_manager_set_network`；
   环境变量 `AIDAW_STORE_URLS`（逗号分隔）/ `AIDAW_STORE_URL` 仍可完全取代配置。
10. ✅ **文档**：本 plan + spec；`ROADMAP.md` 的 Stage E / 技术栈 / 文档索引同步。

## 2 落地结果

- **Rust**：`src-tauri/src/manager.rs`（新）；`crates/host-vst3/src/scan.rs` + `lib.rs`（额外扫描目录）；
  `src-tauri/{lib.rs,commands.rs,Cargo.toml}`（接线与依赖）。
- **前端**：`src/stores/manager.ts`、`src/components/workspace/panes/ManagerPane.vue`（新）；
  `src/api/{types,tauri,mock,dialogs}.ts`、`src/stores/layout.ts`、
  `src/components/workspace/{PanelTabs,RightPanel}.vue`、`src/views/SettingsView.vue`、
  `src/i18n/{zh,en}.ts`（改）。
- **新依赖**（均宽松许可，符合 `deny.toml`）：`ureq`（`native-tls`，复用已在树里的 TLS）、
  `sha2`、`zip`（deflate）、`tar`、`flate2`。
- **E2（崩溃隔离）本轮不做**：Roadmap 已明确「插件上量后再做」。附带信息：`vst3-host` 0.9
  已自带 `with_process_isolation(true)` + `vst3-host-helper` 的进程隔离开关，将来做 E2 时值得先看它。

## 3 验证

**本机已完成（macOS）**

| 项 | 结果 |
| --- | --- |
| `cargo check`（src-tauri） | ✅ 通过 |
| `cargo clippy --all-targets`（src-tauri） | ✅ `manager.rs` 零告警（其余为既有告警） |
| `cargo test --lib manager::`（src-tauri） | ✅ 7 通过 + 1 忽略（联网冒烟）；覆盖归一化过滤 / 兼容性 / 扩展名解析 / 安装器拒绝 / 归档内找插件 / 下载代理重写 / 注册表源顺序 |
| `cargo test --lib manager::tests::fetches_registry_over_https -- --ignored` | ✅ 真的走通 HTTPS 拉注册表：1.87 MB（gzip 解码后）**7 秒**（未开 gzip 时同样内容要 67 秒）→ 归一化 459 个插件、287 个兼容 mac/arm64 |
| `cargo fmt --check`（src-tauri） | ✅ `manager.rs` 无差异 |
| `cargo test -p ai-daw-host-vst3 --lib` | ✅ 通过 |
| 「外部」标签（真机） | ✅ 系统目录里自己装的 VST3/CLAP 出现在「外部」（只读）；本管理器装的出现在「已安装」 |
| `cargo check --workspace`（根） | ✅ 通过 |
| `npx vue-tsc --noEmit` | ✅ 无类型错误 |
| `cargo test --workspace` | ✅ 28 组全过 |
| 下载健壮性（`download_candidates_keep_the_original_as_a_fallback`） | ✅ 代理在前、原始地址在后；非 GitHub 只有一个候选 |
| 进度条（浏览器预览，mock 驱动） | ✅ 卡片下边缘进度条 12% → 96%，收尾切不确定态；`busy` 只标在那一张卡片上 |
| 复制下载链接（浏览器预览） | ✅ 每行文件一个链接按钮，复制的是后端解析后的地址 |
| 真机：下载中被掐断后**自动续传**而非从头开始 | ⬜ 需要一次真实的大文件下载来验证 |
| 真机：代理不可达时自动回退直连 | ⬜ 同上 |
| 代理：三模式解析（`proxy_modes_resolve_as_expected`） | ✅ 直连无代理、手动地址解析正确、坏地址退回直连并说明原因 |
| 代理：`scutil --proxy` 解析（`parses_scutil_proxy_output`） | ✅ 样例取自真实输出 |
| 代理：跟随系统（`follows_the_system_proxy`，`#[ignore]`） | ✅ 本机解析出「系统代理（操作系统设置）http://127.0.0.1:7892」；连接失败是因为**代理软件没在跑**（7892 无人监听，`curl` 同样失败） |
| 代理：「测试」按钮用输入框里的值（浏览器预览） | ✅ 填 `socks5://127.0.0.1:9999` → 结果里就是它，而非已保存的配置 |
| Windows 系统代理（注册表解析） | ⬜ 未在 Windows 上验证 |

**待对应平台完成**

| 项 | 说明 |
| --- | --- |
| Windows / Linux | 托管库能否被各自扫描器读到、解压后权限是否正确；`cfg` 分支本机不参与编译 |
| 真机端到端（需联网） | 从注册表装一个真实 `archive` 插件 → 「插件」面板出现 → 挂到轨道能出声 |
| 断网 / sha256 不符 | 用 `AIDAW_STORE_URL` 指向本地桩数据，验证报错且不安装 |
