# 运行时后端（Execution Provider）下载管理 — 设计

- 日期：2026-10-16
- 状态：**最终形态：后端只是「三选一」，没有任何下载**。
  实测（§10.3）CoreML 在这几个小模型上**全面更慢**，所以**默认 CPU**；CoreML / DirectML 保留为
  设置页里的手动选项（按平台过滤）。原先设计的 CUDA 运行时下载 / `load-dynamic` / 下载管理器
  **已整体移除**——那条路本机无法验证，且对这套小模型的收益不成立。**实施记录与实测数据见 §10。**
- ⚠️ **注意**：本文 §0–§9 保留了**最初的设计动机与论证**（含 CUDA 下载方案），
  其中与实现不一致的部分以 §10 为准；ACE-Step 1.5 已整条移除（见 `../ROADMAP.md` 第十五次修订）。
- 关联：`crates/local-ai/src/backend/`（新模块）、`src-tauri/src/backends.rs`（下载器）、
  `crates/local-ai/src/models/catalog.rs`（清单模式）、`crates/local-ai/examples/backend_probe.rs`（新探针）
- 前置讨论：2026-10-16 晚「加速依赖不随包、按设备下载」方向确认（见 MEMORY.md）

## 0 背景

- 全部模型跑 CPU EP：`ort = "2.0.0-rc.13"` 裸依赖（无 EP 特性），全仓库 6 处建会话零
  `with_execution_providers`。ACE-Step 30 秒音乐在 M4 上 ~47 s（≈1.5 s CPU / 音频秒），
  是旗舰功能最大的体验短板。
- 方向（2026-10-16 定）：**加速依赖不随包，按设备在模型下载后另行下载**；CoreML 先行；
  下载管理复用模型下载器基建；自动回退 CPU；所有能加速的模型都走同一通道。

## 1 目标与非目标

**目标**

- 按设备硬件自动探测、**自动选择并推荐**后端（CoreML / DirectML / CUDA），用户可手动覆盖；
- 需要下载的后端（CUDA）走与模型下载同源的下载器（多源 / Range 续传 / sha256 / 取消 /
  磁盘预检 / 镜像），下载后重启生效；
- 两级自动回退：EP 注册/建会话失败 → 重试纯 CPU；ORT 内置的算子级回落（partitioning EP）
  照常工作，两者都不打断用户；
- 所有 `ort` 模型（Basic Pitch / LocalVQE / HTDemucs / ACE-Step 全部图）共享同一加速通道。

**非目标（v1 明确不做）**

- **TensorRT**：最重（~GB 级）、版本钉死最狠，CUDA 跑通后再评估；
- **ROCm / OpenVINO / oneDNN**：生态小、需求不明，清单结构留好口子即可；
- **Intel Mac（x86_64-apple-darwin）**：ort 不为该 target 发布预编译（dist.tsv 实查无条目），
  只能 CPU——文档明示，不做特殊处理；
- per-model 的 EP 细调（如只给 DiT 开 CoreML）：v1 全局一个选择，回落由 ORT 自己按节点决定。

## 2 已核实的事实（ort 2.0.0-rc.13 / ONNX Runtime 1.28，源码实查）

1. **macOS 二进制已含 CoreML**：`ort-sys/build/download/dist.tsv` 里 `aarch64-apple-darwin`
   只有 `coreml` 与 `coreml,webgpu` 两个构建。裸依赖（无特性）时 resolve 的回落逻辑
   （无精确匹配 → 取交集最佳候选）拿到的就是 coreml 构建——**当前包内的 framework 本身就
   带 CoreML EP**，缺的只有 API gate（`coreml` 特性解锁 `ort::ep::CoreML`）与注册代码。
2. **Windows 同理含 DirectML**：`x86_64-pc-windows-msvc` 的 dist 只有 `directml` /
   `webgpu` / `nvrtx,directml` / `cuda13,tensorrt,nvrtx,directml`，回落同样落到 directml
   构建。**Linux 才有纯 CPU（`none`）构建**。→ Windows 走 DirectML 也是零下载。
3. **特性全集匹配**：resolve 要求 dist 的特性集与请求特性集**完全一致**（symmetric
   difference == 0），否则构建失败。→ `coreml` / `directml` 特性必须**按 target cfg 条件
   启用**（`[target.'cfg(target_os = "macos")'.dependencies]`），不能无条件进 `[dependencies]`。
4. **load-dynamic 是纯 API gate**：`ort-sys/build/main.rs` 实查——`disable-linking`
   （= `load-dynamic` 特性）时 build 脚本**直接 return**：不下载、不链接、不解析 dist。
   → Phase 2（CUDA 下载 dylib）成立：crate 切 load-dynamic 后，`coreml`/`cuda`/`directml`
   特性可以全开（无 dist 约束），dylib 由 `ORT_DYLIB_PATH` 在运行时提供。
5. **ORT_DYLIB_PATH 生命周期**（`ort/src/lib.rs::setup_api`）：首次 `ort::api()` 时惰性加载，
   读环境变量（macOS 默认名 `libonnxruntime.dylib`），加载失败即 **panic**；一个进程一个
   dylib（`OnceLock`）。→ 启动时必须先校验路径有效再放行任何 ort 调用；下载新后端后
   **重启生效**。
6. **CoreML API**（`ort/src/ep/coreml.rs`）：`ep::CoreML::default().build()` →
   `Session::builder()?.with_execution_providers([ep])?`。旋钮：`ComputeUnits`
   （All / CPUAndNeuralEngine / CPUAndGPU / CPUOnly）、`ModelFormat`（MLProgram 需
   Core ML 5+ / NeuralNetwork 默认）、`with_subgraphs`。
7. **版本钉死**：ort rc.13 ↔ ONNX Runtime 1.28 ↔ C ABI `api-27`。后端 dylib 必须与
   `api-27` 匹配（ORT 1.28 提供）；pyke 的 dist URL 自带 sha256（dist.tsv 已公布）。

## 3 关键决策

| 决策 | 结论 | 理由 |
| --- | --- | --- |
| 分发模型 | **CoreML / DirectML 零下载**（现状二进制已含，只加特性+注册）；**CUDA 按需下载 dylib** | dist.tsv 实查：mac/win 默认构建已含；CUDA 构建数百 MB 且要驱动门槛，只能下载 |
| CUDA 加载方式 | `load-dynamic` + `ORT_DYLIB_PATH`，下载后重启生效 | 事实 4/5：build 脚本纯 API gate；一个进程一个 libonnxruntime，「替换」非「并存」 |
| 后端清单 | catalog 第四种条目 `BackendEntry`（平台 / GPU 厂商 / 文件组 / 体积 / 源 / sha256 / 驱动门槛 / 是否随包） | 复用 models.rs 下载器与 store 网络配置（`engine://backend` 事件同构） |
| 会话创建 | 6 处（`ace_step::session` × 7 调用、basic_pitch / localvqe / demucs / demucs6s）统一走 `backend::session::build(path)` | 「所有模型支持加速」+ 统一回落；错误分类：EP 注册失败回落，权重错误透传 |
| CoreML 配置 | `ComputeUnits::All`、`ModelFormat` 先 `NeuralNetwork`（默认） | 该导出谱系上游验收均为默认格式；MLProgram 留旋钮，probe 对比后再定 |
| 硬件探测 | macOS：arm64 → CoreML；Windows：显示适配器枚举（NVIDIA → CUDA 候选+驱动门槛，否则 DirectML）；Linux：`nvidia-smi` → CUDA，否则 CPU | 各平台最小实现；探测失败一律 CPU |
| CPU 后端 | **不下载化**（维持随包/现状链接） | Basic Pitch / LocalVQE 随包小模型开箱要用，下载化会打破「开箱可用」 |
| dylib 源 | pyke dist URL（`cdn.pyke.io/0/pyke:ort-rs/ms@1.28.0/...`，sha256 钉死）为首选，魔搭自建镜像兜底（与模型同策略，不写默认第三方源） | 与 ort rc.13 期望的构建布局精确匹配；hash 已公布 |

## 4 结构

**`crates/local-ai/src/backend/`（新模块）**

- `kind.rs`：`BackendKind { Cpu, CoreMl, DirectMl, Cuda }`（Display + 序列化给前端）。
- `detect.rs`：`HardwareProfile { os, arch, gpu: Option<GpuVendor>, driver: Option<String> }`。
  探测实现按 `cfg` 隔离（macOS `sysctl`；Windows 注册表/DXGI；Linux `nvidia-smi`），
  纯解析函数与 IO 分离 → 可单测（mock 输出喂解析器）。
- `catalog.rs`：`BackendEntry` + `BACKEND_CATALOG`。`cpu` 条目 `bundled: true`（无文件，
  仅声明「已随包」）；`cuda` / `directml` 条目带文件组（dylib 名 / 体积 / 源 / sha256 /
  驱动门槛 / 平台门槛）。
- `resolve.rs`：`available()`（已下载 + 随包）、`recommend(&HardwareProfile)`、
  `current()`（读 store 的强制选择，缺省 = 推荐）。
- `session.rs`：`build(path, backend)` —— 注册 EP → `Err` 则回落 CPU 重试并
  `tracing::warn!`。Phase 0 的 `ace_step::session`（`AIDAW_ACE_EP` 环境变量版）是它的前身。

**`src-tauri/src/backends.rs`（新，镜像 `models.rs`）**

- 下载：`download_candidates` / `download_verified`（多源 + 代理重写 + Range 续传 +
  sha256 + 取消）**直接复用 models.rs 的私有函数**（提为 `pub(crate)` 或抽公共模块）；
- 磁盘预检（CUDA 构建数百 MB，先查空间）；解压（pyke 的 tar.lzma2 → 纯 Rust lzma，
  候选 `lzma-rust2`，新增依赖需评估）；
- 命令：`backend_catalog` / `_installed` / `_download` / `_cancel` / `_remove` / `_reveal`；
  事件 `engine://backend`（`BackendProgress`：id / phase / received / total）；
- 网络配置复用 `store.json`（`StoreNetwork`：代理 + 镜像 + 区域偏好）。

**前端**

- `stores/backends.ts` + `panes/BackendPane.vue`（或并入管理器新标签）：每设备一张卡，
  显示「推荐 / 已启用 / 可下载（体积）/ 已下载（待重启）」；
- AI 面板/设置页显示当前生效后端；下载完成给明确的**重启提示**；
- 设置页「强制 CPU」开关（排障用），写 store.json。

## 5 数据流

```
启动 → backend::init()：校验当前后端 dylib 存在（ORT_DYLIB_PATH）→ 无效则回落随包 CPU
     → 建会话一律走 backend::session::build()（EP 失败自动回落 CPU）
面板 → backend_catalog → 清单按本机探测结果过滤（「你的设备可用」）
推荐 → 「Apple M4 → CoreML（已随包，已启用）」/「NVIDIA RTX … → CUDA（待下载 312 MB）」
点下载 → backend_download → 多源+续传+sha256+磁盘预检 → engine://backend 进度
       → 完成 → 「重启后生效」提示 → 下次启动 ORT_DYLIB_PATH 指向新后端
```

## 6 版本钉死与升级

- 清单锁定 `ms@1.28.0` 的对应 tar.lzma2 + sha256（值来自 dist.tsv，随 ort 版本升级
  一并换源、重测、重验哈希）；
- ort crate 升级（如 rc.14）→ 后端条目同步换 URL + hash，**CI/冒烟测试**验证加载。

## 7 风险与未决项

1. **MatMulNBits 覆盖（生死假设）**：CoreML/DirectML 对 INT4 图的支持未知。CoreML 是
   partitioning EP——不支持的节点留在 CPU、会话照常建成，所以「会话建成」≠「加速生效」。
   **Phase 0 probe 测的就是这个**：同 prompt 同 seed，CPU vs CoreML 的 `--generate`
   端到端耗时对比（CPU 基线 10 s → 18.2 s、30 s → 46.8 s，M4）。若 CoreML 下 DiT 全回落
   CPU，则 q8/fp16 档 + GPU EP 的讨论合并、整个加速排序重议。
2. **ORT_DYLIB_PATH 指向无效文件时 ort 直接 panic**（`setup_api` 的 expect）：
   `init()` 必须在任何 ort 调用前校验（存在 + 大小 + sha256），宁可回落 CPU。
3. **重启生效**：dylib 惰性加载 → 下载后必须重启；UX 明确提示，设置页显示「待重启」。
4. **CUDA 驱动门槛**：CUDA 13 dist 的驱动最低版本需按 ORT 1.28 文档核实（预计 ≥ 580），
   探测不达标 → 推荐 DirectML 而非 CUDA；CUDA 运行时依赖是否随 gpu 构建提供需按平台核实
   （Windows ORT gpu 通常自带，Linux 要用户装 toolkit）——写入推荐逻辑的前置条件。
5. **pyke CDN 国内可达性**：加魔搭自建镜像（同模型策略）；lzma2 解压依赖评估。
6. **load-dynamic 切换期的构建回归**：Phase 2 切 load-dynamic 时全平台构建+现有测试
   必须全绿（Phase 1 先把 backend 模块与回落逻辑在 download-binaries 下跑通，隔离变量）。
7. **x86_64-apple-darwin 无预编译**（事实 1）：Intel Mac 只能 CPU，文档明示即可。

## 8 验收

1. macOS M4（Phase 0）：`AIDAW_ACE_EP=coreml` 下五个图全部建会话成功；`--generate`
   10 s 音乐端到端耗时 vs CPU 基线有可测差异（正反向都算结论，数值记入本文件）。
2. 回落（Phase 1）：坏 EP 路径 / EP 注册失败 → 模型照常跑 CPU，`tracing` 可见回落原因。
3. 探测（Phase 1）：mock 硬件输出 → 推荐正确（单测不依赖真实 GPU）。
4. 下载（Phase 2）：CUDA/DML 条目下载、sha256 校验、解压、安装成功；坏文件被拒且不落地。
5. 回归（每阶段）：`cargo test -p ai-daw-local-ai`（现 74 条）全绿；CPU 路径行为不变
   （同 seed 同 WAV，字节级）。
6. 跨平台构建（Phase 2 后）：Windows / Linux 构建不回归（target-cfg 特性）。

## 9 阶段与推进顺序

| 阶段 | 内容 | 前置 |
| --- | --- | --- |
| **Phase 0**（已完成） | target-cfg coreml 特性 + `ace_step::session` 的 `AIDAW_ACE_EP` 开关 + 本机 probe 实测 | — |
| Phase 1（已完成） | `backend` 模块（detect / catalog / resolve / session 统一注册+回落）；6 处会话点改造；EP 选择从环境变量升级为 store | Phase 0 通过 |
| Phase 2（骨架已落地） | 下载管理（`backends.rs` + 命令 + 事件 + 磁盘预检 + lzma 解压）；CUDA dylib 源；load-dynamic 切换（CUDA 需要） | Phase 1 |
| Phase 3（已完成） | 前端：后端卡片 / 推荐理由 / 重启提示 / 强制后端开关 | Phase 2 |
| Phase 4 | 打磨（文档、听感回归、跨平台验证） | Phase 3 |

> Phase 0 是生死闸门：CoreML 接不住 MatMulNBits 的话，加速的收益模型要重算
> （可能只剩 FP32 组件与分离/转 MIDI 那几路），后续阶段排序重议。

---

## 10 实施记录（2026-10-16 落地）与实测数据

### 10.1 落地的代码

**`crates/local-ai/src/backend/`（新模块）**

| 文件 | 内容 |
| --- | --- |
| `kind.rs` | `BackendKind { Cpu, CoreMl, DirectMl }`（`as_str`/`parse`/`label`） |
| `detect.rs` | `HardwareProfile` + `GpuVendor`；解析函数（`vendor_of_adapter` / `parse_nvidia_smi` / `parse_windows_adapters` / `parse_macos_chip`）与 IO（`detect`）分离，解析可单测 |
| `catalog.rs` | `BackendEntry`；3 个条目（`cpu` / `coreml` / `directml`），各带**平台 / 架构门槛**与一句说明；`supported(profile)` 给出本机可用项 |
| `session.rs` | `GraphRole`（4 个角色）+ `build(path, role)`；EP 注册失败**自动回落 CPU**；per-graph 排除（运行时学习）；CoreML 编译缓存 |
| `mod.rs` | `set_backend` / `backend`（`AIDAW_BACKEND` > 已设值 > **CPU**）/ `init`（平台不符时回落 CPU） |

**会话点**：`basic_pitch` / `localvqe` / `demucs` / `demucs6s` —— **全部**改走
`backend::session::build`（原 ACE-Step 的 6 张图与 LM 已随该功能一并移除，见 ROADMAP 第十五次修订）。

**`src-tauri/src/backends.rs`（小）**：只有「选哪个」——`ai_backend_info` / `ai_set_backend`
两个命令 + 启动时的 `init_runtime`（设 CoreML 缓存目录、把偏好落到 `local-ai`、校验并打印一行）。
选择存在 `store.json` 的 `backend_preference`（`cpu` / `coreml` / `directml`，**默认 cpu**）。
**没有下载 / 进度 / 事件 / 删除**。

**前端**：设置页新增「AI」分区（执行后端分段控件，只列出本机 `available` 的项）+
`api/{types,tauri,mock}.ts`；zh/en i18n。**下载管理器里不再有后端页签**。

**其他**：修了一个**已存在的跨平台 bug**——`rustfft` / `beat-this` / `tokenizers` 原本被写在
`[target.'cfg(target_os = "macos")'.dependencies]` 下，非 macOS 根本编不过；已移回 `[dependencies]`。

### 10.2 实测（macOS M4，本机权重齐全；`examples/backend_probe`）

自动探测结果：`macos / aarch64 · GPU=Apple M4 → CoreML`。全部图**都建得上会话**。

| 图 | CoreML | 备注 |
| --- | --- | --- |
| Basic Pitch | ✅ | |
| LocalVQE | ✅ | |
| 6 分轨（`htdemucs_6s.onnx`） | ❌ 学习后 CPU | CoreML 编译期失败：`Espresso … generic_general_slice: Invalid values 2048 in begin_ids` |

**两个新发现（都已处理）**：

1. **CoreML 默认每次建会话都重编译**（官方文档：caching 默认关闭）。本仓库的模型层是「一次推理
   建一次会话」，不缓存的话 LocalVQE 一次 **36 s**。加 `ModelCacheDirectory`（按 `GraphRole`
   分桶）后：**36.4 s → 1.7 s**（第二次起）。缓存目录由宿主设成应用配置目录下的
   `runtime/coreml/`（跨重启复用）。
2. **有些图 CoreML 编译期就失败**（6 分轨）。与其写死一张会过时的名单，改成**运行时学习**：
   某张图第一次失败后记进位图，后续直接 CPU（省掉每次十几秒的白等）。
   → 编译期失败的靠学习；将来若出现「建会话成功、推理中崩溃」的图，必须**写死排除**，因为兜不住。

### 10.3 **实测结论：CoreML 在本机全面更慢 → 默认关闭**（2026-10-16 补测）

跑 `examples/ep_bench`（release，M4），同一输入 CPU vs CoreML：

| 模型 | CPU | CoreML | 倍率 |
| --- | --- | --- | --- |
| Basic Pitch（3 s 音频） | 63 ms | 412 ms | **6.5× 慢** |
| LocalVQE（3 s 音频） | 0.48 s | 68.1 s（几乎全是建会话/编译） | **142× 慢** |
| HTDemucs 6s（3 s 音频） | 2.61 s | 31.9 s（CoreML 编不过 → 回落 CPU + 白等） | **12× 慢** |

**没有一个模型变快。** 原因（都不是配置问题）：

1. **CoreML 建会话 = 编译模型**，而本仓库是「一次推理建一次会话」→ 每次调用都付编译费，
   模型越大越离谱（LocalVQE 单次调用 68 s 里 68.08 s 是建会话）。
   加 `ModelCacheDirectory` 能缓解（LocalVQE 冷 36 s → 热 1.7 s），但**救不了根本**：
2. 就算把建会话时间扣掉，**单次推理本身也更慢**（如 Basic Pitch：扣掉建会话后 CoreML 83 ms
   vs CPU 32 ms，仍慢 2.6×）。CoreML EP 要按节点分区、有固定开销，而 M4 的 CPU 配上
   ORT 的 ARM 优化内核本来就很快；ANE 对这套图（INT4 `MatMulNBits` + 小卷积/注意力）没有优势。
3. `ComputeUnits` 三种都试过：`ane` < `all` < `gpu`（`gpu` 慢到跑不完）。

→ **决定**：**默认 CPU**；CoreML / DirectML 只作为设置页里的手动选项（按平台过滤）。
`backends.rs` 里的 `available` 字段就是「本机平台支不支持」，界面只渲染 `available` 的项。

**重新评估的前提**（任一满足就再测一轮）：
- **进程内缓存 Session**（现在每次调用重建）——能把建会话这一项从计时代价里彻底去掉；
- ORT / CoreML EP 版本升级（rc.14+）；
- 模型换成 FP16/FP32 导出（INT4 `MatMulNBits` 也许正是 CoreML 的短板）。

### 10.4 未完成 / 待办

- **进程内缓存 Session**：**没做**（只在这里列为建议）。它不是无脑赚：本仓库的模型层是
  「一次推理建一次会话」，缓存能帮「同进程内的重复调用」（逐片段增强 / 逐次分离），
  代价是图要常驻内存（6 分轨 258 MB、4 分轨 210 MB）。合理的做法是**只缓存小模型**
  （Basic Pitch 0.2 MB / LocalVQE 6 MB / mel），大图仍按需建。
- **`htdemucs.ort`（4 分轨）本机没有权重**，CoreML 兼容性未知（6 分轨同族且编译失败）。
- **DirectML 未实测**（本机没有 Windows）：它不需要 CoreML 那种「编译到磁盘」的步骤，
  建会话开销小，所以保留为选项；拿到 Windows 数据后再决定要不要标注「更慢」。
- **听感回归**：加速只应改数值噪声，不应改听感；未做正式的 A/B 听感对比。


