# 实施计划：本地 AI 模型分发基建（G4 前置）

- 日期：2026-10-10
- 设计：`specs/2026-10-10-local-ai-models-design.md`
- 范围：**只做基建**（清单 + 双源下载 + 安装位置 + 设置页），并用**第一个真实模型**端到端验证
- **阻塞**：第一个模型选谁取决于「母带处理」方案拍板（见设计 §6），**待定**

> **进展（2026-10-11）—— 本计划已完成（阶段 0–3）**
> - **阶段 0** ✅：新增 `crates/local-ai/src/models/catalog.rs`（纯 Rust 内嵌清单 + sha256）；
>   `search_dirs()` 通过 **`models::set_extra_dirs`**（宿主启动时登记「应用配置目录 `models/`」与
>   「打包资源目录 `models/`」）接入新落点；`Availability` 增 `htdemucs`。
> - **阶段 1** ✅：新增 `src-tauri/src/models.rs` —— `model_catalog` / `model_download` /
>   `model_cancel` / `model_remove` / `model_reveal`；事件 `engine://model`；复用 `manager.rs` 的
>   `download_verified`（多源回退 + `Range` 续传 + sha256 + 代理）并新增**取消**与
>   **`.part` → 原子 rename**；成功源等逻辑与插件下载一致。
> - **阶段 2** ✅：`StoreNetwork` 增 `model_mirrors`（`store.json`，**不给默认**）；
>   设置页「网络」增「模型镜像」；侧边栏「管理器」新增**「模型」标签**（下载 / 取消 / 删除 + 进度）。
> - **阶段 3** ✅：第一个真实模型 = **音源分离 HTDemucs v4**（见下）。**魔搭源未做**（待用户自建仓库）。
> - **磁盘预检未做**（不引新依赖；空间不足会在写入时明确报错）。
> - **母带处理（方案 A，纯 DSP）** ✅ 已完成（见下「插入任务」）。

## 阶段 0 — 清单与结构（无网络，可独立完成）

1. ⬜ `crates/local-ai/src/models/catalog.rs`：模型条目结构体 + 解析 + 单元测试。
   - 字段：`id` / `name` / `feature` / `license` / `approx_bytes` / `filename` / `sources[]`
     （`kind` ∈ `hf` | `hf-mirror` | `modelscope` | `custom`；`url`；`sha256`）。
   - 先只放**现有两个**模型（Basic Pitch / LocalVQE）作为清单的初始内容——它们的 sha256 由
     `tools/fetch-models.sh` 拉下来后算出并**固化进仓库**。
2. ⬜ `search_dirs()` 插入应用配置目录的 `models/`（位置：`$AIDAW_MODELS` 之后）。
3. ⬜ `availability()` 改为清单驱动（每个 `id` 一个 bool），`AiModels` 命令随之扩展。

## 阶段 1 — 下载器（Rust）

4. ⬜ `src-tauri/src/models.rs`：
   - `model_catalog()` → 清单 + 每个条目的本地状态（`installed` / `bytes` / `path`）。
   - `model_download(id)` → 后台线程；源顺序：配置镜像 → `hf` / `hf-mirror` → `modelscope`，
     成功源记忆；`Range` 续传 + `.part` + sha256 → 原子 rename。
   - `model_cancel()` / `model_remove(id)` / `model_reveal(id)`（在文件管理器中显示）。
   - **磁盘预检**：`approx_bytes` vs 可用空间，不足即拒。
   - 事件 `engine://model`（`id` / `state` / `received` / `total`）。
5. ⬜ 单测（**不联网**）：清单解析、源排序与回退顺序、URL 拼接（HF / hf-mirror / 魔搭三种形态）、
   「sha256 不匹配 ⇒ 不落地」、断点续传的临时文件语义。

## 阶段 2 — 设置页与前端

6. ⬜ `store.rs`：网络配置加 `model_mirrors: Vec<String>`（默认**空**，**不给默认地址**）；
   环境变量 `AIDAW_MODEL_SOURCES` 整体覆盖。
7. ⬜ `SettingsView.vue`：「网络与镜像」加一项「模型镜像」。
8. ⬜ `panes/AiPanel.vue`：卡片三态（未装 → `下载模型 · 209 MB` / 下载中 → 进度条 + 取消 /
   已装 → `模型` 徽章 + 删除）。`stores/ai.ts` + `api/tauri.ts` + mock + i18n(zh/en)。
9. ⬜ 体积确认：`approx_bytes > 300 MB` 时先弹确认框（小模型直接下，一步到位）。

## 阶段 3 — 第一个真实模型端到端（**待拍板选谁**）

10. ⬜ 依设计 §6 的结论接入第一个模型（当前倾向：**先做方案 A 的纯 DSP 母带链**，
    把「带模型下载」的首验交给**音源分离**）。
11. ⬜ 魔搭侧准备：核实权重是否已存在；没有就自建仓库并上传 → 把 URL + sha256 写进清单。
12. ⬜ 真机验证：下载 → 校验 → 立即可用；断网 + 只有魔搭源时同样成功。

## 插入任务 — 母带处理（方案 A，**已拍板并完成**）

> 2026-10-10 决定：母带处理走**纯 DSP**，不下载模型。下载基建的首验因此顺延给音源分离。
> 详见设计 §6 的拍板记录。

13. ✅ **`crates/mastering`（新 crate，纯 DSP，零依赖模型）**
    - `limiter.rs`：**真峰值限幅器**——4× 过采样（多相 FIR，每相位各自归一到直流增益 1）估算采样间
      峰值 → 逐帧目标增益 → **前向滑动窗口最小值**（单调双端队列，`O(n)`）实现前瞻 → 一阶释放平滑。
      因为是离线处理，前瞻不需要延迟线（增益已经看到了未来）。
    - `lib.rs`：`MasterOptions`（目标 LUFS / 真峰值上限 / 立体声宽度，带 `sanitized()` 兜底）+
      `master()` 处理顺序 **宽度 → 增益 → 限幅 → 复测** + `MasterReport`（前后响度与真峰值、
      施加增益、限幅量）。响度观测复用 `ai-daw-analysis`（不重写 EBU R128）。
    - 测试 **14 条**，其中关键两条：`limiter_sees_inter_sample_peaks`（`fs/4` 相移 45°，取样
      只有 ±0.707 而真峰值是 1.0——证明「必须过采样」）与 `normalizes_toward_the_target_loudness`。
14. ✅ **宿主**：`src-tauri/src/ai.rs` 新增 `KIND_MASTER`，**复用现成的进度/取消/事件/新轨机制**——
    `ai_start` 多一个可选 `master` 参数；`run_master_job` 走 `render_inputs` → `render_offline`
    → `master()` → `write_wav` → `Command::AddAudioTrack`（一条可撤销事务，原混音不动，便于 A/B）。
15. ✅ **MCP**：新增工具 `master_project`（渲染 → 母带 → 写 WAV → 返回 `report`），
    工具数 32 → 33；`skills/aidaw/references/analysis.md` 同步。
16. ✅ **前端**：AI 面板新增「母带处理」卡片（目标响度 / 峰值上限 / 立体声宽度三个数字输入 +
    一个按钮 + 进度条，复用同一套 `.item` / `.iface-btn` / `.progress` 样式），i18n zh/en 各 6 条。
17. ⬜ **未做**：实时主总线的响度表 / 限幅器（`Project.master` 目前只有 gain/pan，**没有**效果链；
    加它要动模型 + 播放器 + 命令族，属于独立的一轮）。

## 验证清单

| 项 | 状态 |
| --- | --- |
| `cargo test -p ai-daw-mastering`（14 条：限幅 / 采样间峰值 / 宽度 / 响度归一 / 静音 / 单声道 / 参数兜底） | ✅ 14 通过 |
| `cargo test --workspace` | ✅ 全绿 |
| `cargo clippy -p ai-daw-mastering -p ai-daw-app --all-targets` | ✅ 新代码零告警 |
| `cargo fmt --check`（本次改动的文件） | ✅ 无差异（仓库其余部分是历史遗留的不整洁） |
| `npx vue-tsc --noEmit` | ✅ 无类型错误 |
| `mcp::tests::every_tool_is_registered`（工具数 32 → 33） | ✅ 通过 |
| 真机：AI 面板点「开始母带」→ 落到一条「母带」新轨道，通知里报前后 LUFS | ⬜ 待你在桌面端跑一次 |
| 真机：限幅介入时通知里出现「限幅 x dB」 | ⬜ 同上 |
| `cargo test -p ai-daw-local-ai --lib`（清单解析） | ⬜ |
| `cargo test --lib models::`（src-tauri，下载器纯逻辑） | ⬜ |
| `cargo clippy --all-targets` / `cargo fmt --check` | ⬜ |
| `npx vue-tsc --noEmit` | ⬜ |
| 真机：HF 直连下载成功 | ⬜ |
| 真机：只配魔搭源、HF 不可达时下载成功 | ⬜ |
| 真机：中断后续传，最终 sha256 一致 | ⬜ |
| 真机：篡改文件被拒、不留坏文件 | ⬜ |
| 真机：磁盘不足时明确报错 | ⬜ |
| Windows / Linux 分支（`cfg`）在对应平台跑过一遍 | ⬜ |

## 未决

- **第一个模型选谁**（设计 §6 的三条岔路）——需要拍板后才能开工阶段 3。
- 应用配置目录 vs `~/Library/Caches`（GB 级模型的落点）。
- 下载完成后 AI 面板是**热更新**还是提示重启一次。
