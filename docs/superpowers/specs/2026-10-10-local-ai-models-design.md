# 本地 AI 模型：分发基建 + G4 选型设计

- 日期：2026-10-10
- 状态：**基建已落地（2026-10-11）；母带处理已选方案 A 并完成**（见 §6）
- 关联：`ROADMAP.md` Stage G（G0 进程内小模型 / G4 其余四项）、`crates/local-ai`、`tools/fetch-models.sh`

> **落地记录（2026-10-11）**
> - **清单**：`crates/local-ai/src/models/catalog.rs`（纯 Rust 内嵌，不引 serde），三条：
>   `basic-pitch` / `localvqe`（`bundled = true`）、`htdemucs-ort-v1`（`feature = separate`，209 MB）。
> - **落点与查找**：§3.3 改为——`$AIDAW_MODELS` → **宿主登记的额外目录 `models::set_extra_dirs`**
>   （应用配置目录 `models/`＝下载落点；打包资源目录 `models/`＝随包小模型）→ 可执行文件同级 →
>   `../Resources/models` → 仓库根 → cwd。
> - **下载器**：`src-tauri/src/models.rs`；复用 `manager.rs::download_verified`（多源 + `Range` 续传 +
>   sha256 + 代理/镜像），新增**取消**与 **`.part` → 原子 rename**；事件 `engine://model`；
>   命令 `model_catalog` / `model_download` / `model_cancel` / `model_remove` / `model_reveal`。
> - **镜像**：§3.7 的 `model_mirrors` 落在 `StoreNetwork`（`store.json`），设置页「网络」一节可配，
>   **不给默认**；环境变量 `AIDAW_MODEL_SOURCES` 整体覆盖。
> - **小模型随包**：Basic Pitch / LocalVQE 提交进 `src-tauri/resources/onnx/`，`bundle.resources`
>   映射到 `models/` → 开箱可用；**大模型（htdemucs）不随包**。
> - **魔搭（ModelScope）源 ✅ 已接入**：自建仓库 `ailm32442/htdemucs-ort`、
>   `ailm32442/beat_this.onnx` 的 `resolve/master/*`（sha256 与 HF / GitHub 完全一致）。
> - **区域感知源排序 ✅（2026-10-15）**：`ModelSource` 增 `region`（`intl` / `cn`）；
>   `StoreNetwork::model_source_preference`（`auto` / `international` / `domestic`，默认 `auto`，
>   设置页「网络」可切）决定优先区域，`auto` 时**探测首字节延迟**定序，另一区域兜底（详 §3.2）。
>   清单补齐：`htdemucs-6s` + 魔搭、`beat-this` + 魔搭，至此三个大模型均国内外双源。
> - **运行方式原则（2026-10-11 定，不引 Python、保轻量）**：Python **只**用于**开发期**把
>   `.pt` 导成 ONNX（同 `tools/export-localvqe-onnx.py` 先例），产出 ONNX 后即退场，随包/运行时
>   **不带 Python**；运行时统一 `ort`（③④ SoulX 拿到 ONNX 后**进程内**、无需 sidecar）；只有需要
>   **第二套运行时**（⑤ ACE-Step 的 `candle`）或要崩溃隔离时才用 **Rust sidecar**（不是 Python），
>   主程序保持只依赖 `ort`。
> - **未做**：磁盘预检；
>   下载完成后的 AI 面板热更新为**事件驱动**（`engine://model` → 重拉 `ai_models`）。
> - **第一个真实模型 = 音源分离 HTDemucs v4**（§4 第 2 项）✅；**不依赖 `stem-splitter-core`**
>   （它精确锁 `ort =2.0.0-rc.11`，与本仓库 rc.13 冲突），改为手写移植其 DSP 到
>   `crates/local-ai/src/models/demucs.rs`。

## 0. 背景与问题

应用本体约 **32 MB**。Stage G 的目标功能全部依赖神经网络权重，而权重的体积从几十 MB 到
**6 GB** 不等，**不可能进安装包**（`bundle.resources` 也放不下、也不该放）。

现状：`crates/local-ai/src/models/` 已经能加载 ONNX 权重（`ort` 2.0-rc.13，Basic Pitch +
LocalVQE 两个模型），但**权重的获取只有开发路径**——`tools/fetch-models.sh` 手工跑，把文件丢进
仓库根 `models/`。装进包里的 App 没有任何下载途径，用户只能看到「内置 DSP」兜底。

**本轮目标**：给「**用户主动安装模型**」做一套**应用内下载基建**，并且要求**国内外都能下到**
（HuggingFace + 魔搭社区双源）。

## 1. 选型核查（2026-10-10 实查）

外部给的技术选型表里有三处与事实不符，**已逐条核实**：

| 功能 | 表里说的 | 实查结果 |
| --- | --- | --- |
| 音源分离 | `stem-splitter-core`，MIT，纯 Rust `ort`，~200 MB | ✅ **属实**。crates.io `1.2.0`（2026-04），`MIT OR Apache-2.0`，用 `ort`（features: `cuda`/`coreml`/`directml`/`onednn`/`xnnpack`）。模型 `htdemucs_ort_v1`（209 MB）从 HF 下载、**自带 sha256 校验**，且提供 `manifest_url_override` 可指向自建镜像 |
| 音乐生成 | ACE-Step v1.5 + `ace-step-rs` crate，Apache 2.0，~6 GB | ⚠️ **一半错**。ACE-Step v1.5 是真的；但 `ace-step-rs`（`Marenz/ace-step-rs`）**没有发布到 crates.io**（sparse index 404）——只能走 git 依赖；Candle 加载 safetensors，~6 GB |
| 歌声替换（SVC） | SoulX-Singer-SVC，Apache 2.0，ONNX 已验证 | ⚠️ **模型真、ONNX 假**。`Soul-AILab/SoulX-Singer` 确实 Apache-2.0（HF 上 5.6 GB，en/zh），`SoulX-Singer-SVC` 是它的 SVC 微调；但 **HF 仓库里只有 `.pt`**（`model.pt` / `model-svc.pt`），**没有 ONNX**——要自己在本地导出（和我们做 LocalVQE 一样） |
| MIDI 转歌声（SVS） | SoulX-Singer，Apache 2.0，~2.82 GB | ✅ 同上（Apache-2.0 属实）。同样**只有 `.pt`，需自行导出 ONNX** |
| 母带处理 | `denoize`，MIT/Apache，10–50 MB | ⚠️ **功能定位错了**。`denoize`（crates.io `0.90.2`，MIT，147k 行）真的是降噪 + 去混响 + 去嗡嗡 + LUFS 响度标准化——那是**修复**，不是母带处理；且它用 **`tract-onnx`**（不是我们的 `ort`），并**要求 Rust 1.96** |

**三条结论**：

1. `ace-step-rs` 得走 git 依赖（且是 6 GB 级 + Candle，另一套运行时）。
2. SoulX 两个模型**要自建 ONNX 导出**（现成的只有 `.pt`），导出脚本 + 数值自检是必须的前置工作。
3. **`denoize` 不该放在「母带处理」这一格**——见 §6。

## 2. 运行时策略：统一走 `ort`

上表隐含了**三套 ML 运行时**：`ort`（我们已有 + stem-splitter-core）、`tract-onnx`（denoize）、
`candle`（ace-step-rs）。三套同时进一个二进制是不可接受的（体积、编译时间、CUDA/GPU 提供者
互相打架、以及 `ort` 的全局环境单例问题）。

**决策**：

- **主运行时 = `ort`**，与现有 `crates/local-ai/src/models/` 一致。
- 引入新组件前先确认它用的 `ort` 版本能与我们共存（我们锁 **2.0-rc.13**）；
  `stem-splitter-core` 的 `ort` 版本需在接入前核对，**冲突时优先降级为我们自己的实现**。
- `tract`（纯 Rust、无 C 依赖）作为**例外**：只有在「某个小模型非它不可，且换成 `ort` 成本过高」
  时才考虑，且必须显式记录理由。
- `candle` + git 依赖（音乐生成）**不参与本轮**，等单模型体积 > 1 GB 的下载基建跑通、且确有需求再评估。

## 3. 模型分发基建（本轮核心，与具体功能无关）

### 3.1 模型清单（清单即代码）

新增一份**编译期内嵌**的清单（`crates/local-ai/src/models/catalog.rs`，数据结构 + `include_str!`
一份 JSON），每个条目：

```
id            稳定标识（如 "htdemucs-ort-v1"）
name          展示名
feature       所属功能（separate / svc / svs / mastering / …）
license       SPDX（"MIT OR Apache-2.0" / "Apache-2.0" …）
approx_bytes  预估体积（下载前告知用户）
filename      落地文件名
sources[]     候选源：{ kind: "hf" | "modelscope" | "custom", region: "intl" | "cn", url, sha256 }
```

同一个模型的各源是**同一份字节**：`sha256` 必须完全一致，否则换源就校验失败。
`region` 标出该源是**国外直连友好**（`intl`：HF / GitHub）还是**国内友好**（`cn`：魔搭），
下载前据此排序（见 3.2）。

**sha256 是硬门禁**，来自第三方源的一律先校验再落地——和插件管理器 E1 同一套安全立场
（「用第三方加速站不会降低完整性保证」）。

### 3.2 双源下载（国内外都能下）

**区域感知排序 + 自动回退**（2026-10-15 修订，取代早先「魔搭固定靠后 / 记住成功源」的做法）。下载前先决定「优先哪个区域的源」，把该区域的源排到前面，另一个区域留在候选里兜底——下载器**逐个尝试、第一个成功即止**，所以排错序也只是慢一步、不会失败。

优先区域由 `StoreNetwork::model_source_preference` 决定：

- `auto`（默认）：分别对清单里 `intl` / `cn` 各取一个源，发一个 `Range: bytes=0-0` 探**首字节延迟**（连接/解析/首字节全 3–4 秒超时，避免被墙的源把用户卡住）；谁先通 / 更快就优先谁；两边都探不出结果 → 按清单原序。
- `international` / `domestic`：用户在设置页显式指定，不联网探测。

候选地址的完整顺序：

1. **偏好区域的源**（清单原序）→ 2. **另一区域的源**（清单原序）；
  **清单里只放官方源**（HF / GitHub / 魔搭）：第三方镜像与加速站一律不写进源码——
开源项目不该替用户选第三方服务，要镜像由用户自己在设置页填（默认留空）。

每个源内部再依次是：**用户配置的镜像**（设置页；为空则跳过，见 3.7）→ **官方直链**（GitHub 地址会先过 `download_prefix` 重写）。

各源的官方直链形态：
- **HuggingFace**：`https://huggingface.co/{repo}/resolve/main/{file}`（`intl`）；
- **魔搭社区**：`https://modelscope.cn/models/{ns}/{model}/resolve/master/{file}`（`cn`）——
  对 LFS 文件返回 **302 → CDN**（`ureq` 默认跟随；CDN 的 `X-Linked-Etag` 即该文件的 sha256），
  `Range` 断点续传照常可用；
- **GitHub Releases** 等其它直链记为 `custom` + `intl`。

**前置工作（每模型一次，需要人做）**：模型必须**真的存在于**魔搭。分两种：
- 魔搭上已有（或 `AI-ModelScope` 等组织已镜像）；
- 没有 → 我们自己上传一份到魔搭，或在魔搭建一个只放权重的仓库。

这条**必须逐模型核实**，不能假设「HF 有就等于魔搭有」。清单里**没有的源就不写进去**，
缺源时该地区用户会看到「此源不可用」而不是一个死链。目前双源齐备的是 `htdemucs-ort-v1`
（魔搭 + HF）、`htdemucs-6s`（魔搭 + HF）、`beat-this`（魔搭 + GitHub Releases）。

`AIDAW_MODEL_SOURCES`（逗号分隔）仍是最高优先级的整体覆盖，此时不参与区域排序（测试 / 无头用）。

### 3.3 安装位置与查找顺序

下载到**应用配置目录**的 `models/`（`com.aidaw.app/models/`），并把它插进
`models::search_dirs()` 的**最前面**（`AIDAW_MODELS` 之后、可执行文件同级之前）：

1. `$AIDAW_MODELS`（测试/无头覆盖）
2. **应用配置目录 `models/`（新，用户装的）**
3. 可执行文件同级 `models/`
4. macOS 包内 `../Resources/models/`
5. 仓库根 `models/`（开发）
6. cwd `models/`

这样开发用 `tools/fetch-models.sh` 灌的权重、用户用界面下载的权重，走**同一条查找路径**。

### 3.4 下载器

复用 `src-tauri/src/ai.rs` 已有的「后台线程 + 进度 + 取消」模式，新增：

- **进度事件** `engine://model`（`id` / `state`（`downloading`/`verifying`/`done`/`error`）/ `received` / `total`）；
- **断点续传**：HTTP `Range` + `<file>.part` 临时文件，完成后校验 sha256 → **原子 rename**；
- **取消**：与 `ai_cancel` 一致的语义；
- **磁盘预检**：`approx_bytes` 先算可用空间，不够直接拒绝（6 GB 的模型必须先拦）。

### 3.5 用户可见流程（一步到位）

按你的要求：**小模型不搞分阶段**，「用户主动安装就一步到位」。

- `approx_bytes ≤ 300 MB`（如音源分离 209 MB、母带类 10–50 MB）：点「下载」**直接下完整版**，
  只有一条进度条，没有「基础版/完整版」的分叉；
- 大模型（GB 级）：点「下载」先弹一个**体积确认**（「将下载 2.8 GB」），确认后同样一次性下完；
- 已装 → 卡片显示 `模型` 标签；未装但有内置兜底 → 显示 `内置 DSP` 标签（沿用 G3 的现有 UI）。

### 3.6 前端

`panes/AiPanel.vue` 每个功能一张卡，卡片底部状态区三种形态：未安装（`下载模型 · 209 MB`）、
下载中（进度条 + 取消）、已安装（`模型` 徽章 + 删除/重装）。

### 3.7 与 E1 的关系：共用一套「镜像」设置

E1 已经落了「设置页 → 插件 → 网络与镜像」（注册表镜像 + 下载代理，**不给默认地址**）。
模型下载**复用同一组配置**，不新开一节：注册表镜像用于**插件注册表**，下载代理前缀用于**插件二进制**；
模型再加 **`model_mirrors: Vec<String>`**（模型权重镜像/前置代理，仍遵循「**不给默认地址**」）与
**`model_source_preference`**（区域偏好，默认 `auto`，见 §3.2）。

环境变量 `AIDAW_MODEL_SOURCES`（逗号分隔）可**整体覆盖**配置，供测试与无头场景使用
（对应 E1 的 `AIDAW_STORE_URLS`）。

## 4. G4 的路线与优先序

| 序 | 功能 | 组件 | 运行时 | 许可证 | 体积 | 前置工作 |
| --- | --- | --- | --- | --- | --- | --- |
| 0 | **基建**（本轮） | 下载器 + 清单 + 设置 | — | — | — | 魔搭侧仓库 |
| 1 | **母带处理** | 见 §6 | 见 §6 | 见 §6 | 10 MB–50 MB | 见 §6 |
| 2 | 音源分离 | `stem-splitter-core` 1.2 | `ort`（需核对版本） | MIT / Apache-2.0 | 209 MB | 魔搭侧镜像 + 模型下载改指向 |
| 3 | 歌声替换 SVC | `SoulX-Singer-SVC` | `ort` | Apache-2.0 | ~2.8 GB | **自建 ONNX 导出** + 数值自检 |
| 4 | MIDI 转歌声 SVS | `SoulX-Singer` | `ort` | Apache-2.0 | ~2.8 GB | 同 3（共用推理管线） |
| 5 | 音乐生成 | ACE-Step v1.5 | `candle` + git 依赖 | Apache-2.0 | ~6 GB | 运行时决策推迟 |

**为什么把「音源分离」放第 2**：它顺带解锁 ROADMAP 里记的
「**更好的乐器降噪 = 先分离再处理**」那条路（只对人声过 LocalVQE 再混回），一份模型两处收益。

## 5. 与既有代码的关系

| 位置 | 改动 |
| --- | --- |
| `crates/local-ai/src/models/mod.rs` | `search_dirs()` 插入应用配置目录；`availability()` 扩展为清单驱动 |
| `crates/local-ai/src/models/catalog.rs` | **新**：模型清单 + sha256 |
| `src-tauri/src/models.rs` | **新**：下载器（多源、Range 续传、进度、取消、磁盘预检），6 条命令 |
| `src-tauri/src/ai.rs` | `ai_models` 改为清单驱动（不再硬编码两个字段） |
| `src-tauri/src/store.rs` | 网络配置加 `model_mirrors` |
| `panes/AiPanel.vue` | 卡片三种状态 + 下载/取消/删除 |
| `SettingsView.vue` | 网络与镜像加「模型镜像」一项 |
| `tools/fetch-models.sh` | 保留为**开发**路径，增加「从魔搭拉」的选项 |

## 6. 「母带处理」的三条岔路

> **已拍板（2026-10-10）：选方案 A —— 纯 DSP 母带链，不下载任何模型。**
>
> 实现落在新的 `crates/mastering`（响度归一 + 真峰值限幅 + 立体声宽度），宿主侧作为
> `ai.rs` 的一条任务（`kind = "master"`）接到 AI 面板，另有 MCP 工具 `master_project`。
> 于是「带模型下载」的首次验证交给**音源分离**（第 2 项）——那是确定的 209 MB 单文件 +
> sha256 + 双源，正好把下载基建完整跑通。

外部的表把这一格填成 `denoize`，但 `denoize` 做的是**降噪 / 去混响 / 去嗡嗡 / LUFS 响度标准化**
——那是**修复**，不是母带处理；而且它和 G2 的降噪在功能上已经重叠（我们已有 LocalVQE + 内置
宽带扩展器两条降噪），还会引入 `tract` 这第二套运行时。所以这一格要重新定。

**方案 A — 规则化母带链（纯 DSP，零模型）**
- 内容：目标 LUFS 归一 + 真峰值限幅 + 轻微总线上色（可选 EQ / 立体声宽度）。
- 优点：**零下载、零许可风险、立刻可用**；这是「母带」里最常用、最关键的一步；也天然给
  方案 B 当好「底层」。
- 缺点：**不是「AI 模型」**，和 G4「模型驱动」的定位有偏差。

**方案 B — 参考匹配（Matchering 风格）**
- 内容：给一首参考曲，把音色 / 动态 / 响度匹配到目标曲上。效果是最接近「真母带」的。
- 问题：Matchering 是 **copyleft（GPL 系）**，按 ROADMAP §护栏 只能 **sidecar-only**——
  要么带一个 Python sidecar（重），要么找/训练一个 permissive 的替代实现（不确定有没有）。
- 结论：**工程量最大**，且与 F5/E2 的 sidecar 基建绑在一起。

**方案 C — 就用 `denoize`（按表里原样）**
- 优点：纯 Rust、MIT、10–50 MB、小模型一步到位，完全符合你说的「50 MB 很小、直接下完整版」。
- 缺点：**功能定位是修复不是母带**；引入 `tract` 第二运行时；要求 **Rust 1.96**；
  与已有降噪重叠。若要收，建议**改名叫「音频修复」**而不是「母带处理」。

**我的建议**：先做 **A**（纯 DSP 母带链，零下载，立刻能用），把「AI 模型下载基建」留给
**音源分离（第 2 项）**去首验——那是**确定的 209 MB 单文件 + sha256 + 双源**，正好把基建完整跑通；
之后 **B（参考匹配）** 等 sidecar 基建再说。**C 不建议**，除非把它的定位改成「修复」。

## 7. 风险与未决项

1. **魔搭是否存在对应权重**——逐模型核实，不能假设（§3.2）。
2. **`ort` 版本共存**——`stem-splitter-core` 的 `ort` 版本待核对。
3. **SoulX 只有 `.pt`**——自建导出 + 数值自检是一条独立的工作线（参考 `export-localvqe-onnx.py` 的先例）。
4. **磁盘**——GB 级模型要预检空间；应用配置目录在 macOS 默认不会被 Time Machine 排除策略照顾，
   是否放到 `~/Library/Caches` 或允许用户改路径，待定。
5. **CI 下载不了**——CI 里不可能下 209 MB+ 权重，测试必须**只覆盖清单/校验/续传逻辑**，
   真实推理测试保持「有权重才跑、否则跳过」的既有约定。

## 8. 验收（基建部分）

1. 设置页配置魔搭镜像 → 断网 HF 也能下载成功（sha256 通过）。
2. 配置错误的镜像 → 自动回退下一源，不失败。
3. 下载中断 → 重开后**从断点继续**（`Range`），最终 sha256 一致。
4. 篡改文件 → sha256 不匹配，**拒绝落地**且不留下 `models/` 里的坏文件。
5. 磁盘不足 → 明确报错，不进入下载。
6. 下载完成后**无需重启**即可在 AI 面板看到 `模型` 标签（或提示重启一次，二选一，待实现时定）。
7. 单元测试：清单解析、源排序、URL 拼接、sha256 失败路径（**不依赖网络**）。
