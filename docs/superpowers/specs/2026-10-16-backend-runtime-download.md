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


### 10.4 歌声合成（SVS）实测（2026-10-29，M4）

同一句 6 秒 / 8 步（`examples/svs_demo`，release，seed 7，fp16 档）：

| 后端 | 建 9 个会话 | 端到端 | 摊掉建会话后的推理 |
| --- | --- | --- | --- |
| CPU | 10.1 s | 82.9 s | ≈72.8 s |
| CoreML | 3.1 s | 76.6 s | ≈73.5 s |

**CoreML 没有真正接住大图。** 如果 CoreML 编译了 846 MB 的 `diff_step` / 519 MB 的 `vocoder`
供 ANE 使用，建会话应是分钟级、`runtime/coreml/` 下会落编译缓存；实测建会话最长 1.4 s、
缓存目录没有生成，而摊掉建会话后的推理耗时与 CPU 几乎相同（73.5 vs 72.8 s）——重的算子仍在
CPU 上，只有部分小图 / 算子落到 CoreML（两次输出因此有细微差异，最大差约 −17.5 dB 相对峰值）。
对 SVS 来说切 CoreML 不会更快，**CPU（或 CPU + fp32）仍是正确选择**。

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



---

## 11 重新引入：运行时下载 + 切换（2026-10-29）

§10 的结论是「小模型上加速不成立，所以不做下载」。**这一幕是为了大模型重开的**：SoulX 的
SVS/SVC 是 1–2 GB 的图，CPU 上一次调用几十秒，值得单独下载一份 CUDA 运行时。

### 11.1 换成了什么

| 项 | 以前（§10） | 现在 |
| --- | --- | --- |
| `ort` 链接方式 | `download-binaries` 静态链接 | **`load-dynamic`**（`ort-sys` build 脚本直接 return，不下载不链接） |
| 运行时来源 | 随构建产物 | **应用配置目录 `runtime/ort/<id>/`，下载而来** |
| C API 版本 | `api-27` | **`api-24`**（1.24 起的 dylib 都能加载，含 1.30；DirectML 只发布到 1.24） |
| 后端 | CPU / CoreML / DirectML | 再加 **CUDA**（Windows / Linux x86_64） |
| 切换生效 | 立即（每次建会话现选 EP） | **重启**（一个进程只能加载一份 dylib） |

### 11.2 落地的代码

| 文件 | 内容 |
| --- | --- |
| `crates/local-ai/src/backend/runtime.rs`（新） | `RuntimeId` / 清单（URL、字节数、归档内路径、marker、国内外源）/ 安装目录 / **一次性加载**（`ort::init_from`）/ CUDA 依赖预加载（`ep::cuda::preload_dylibs`）；加载失败返回 `Err`，绝不触发 `ort` 自己的 `expect` |
| `crates/local-ai/src/backend/session.rs` | 建会话前 `runtime::ensure_loaded()`；新增 `register_cuda`；CUDA 建会话失败与 CoreML 一样记入「学到的失败」并回落 CPU |
| `crates/local-ai/src/backend/kind.rs` `catalog.rs` | `BackendKind::Cuda`、清单条目、`runtime_for(kind)` |
| `src-tauri/src/runtimes.rs`（新） | 下载（多源 / 代理 / 断点续传 / 体积校验 / 取消）+ zip/nupkg/tgz 取单个文件 + `engine://runtime` 进度 + 推荐运行时；**一份都没装时后台自动补 CPU** |
| `src-tauri/src/backends.rs` | `BackendOption` 带运行时状态与体积；`BackendInfo.restart_required` / `loaded_runtime` / `recommended` |
| 前端 | 设置 → AI 新增「运行库」卡片（下载 / 删除 / 已加载标记 / 重启提示），`stores/runtimes.ts`，zh/en i18n |

### 11.3 实测（Windows 11 / RTX 4080 / 驱动 610.88 / CUDA 12.8 + 13.4）

CUDA 运行时：ORT 1.30 `microsoft.ml.onnxruntime.gpu.windows` 的 nupkg（153 MB）取 3 个 dll，
cuDNN 9.9 取 `nvidia_cudnn_cu12` wheel 的 8 个 dll，CUDA 12 的 4 个库从本机 v12.8 toolkit 预加载。
`[runtime] CUDA EP 可用` 验证通过，会话全部建成、无回落。

SoulX **SVC**（fp16_hq，release）：

| 场景 | CPU | CUDA |
| --- | --- | --- |
| 6 s / 16 步 | 22.5 / 23.4 s | 23.9 / 23.7 s |
| 3 s / 64 步 | 48.4 s | **38.8 s**（快 20 %） |

**结论：CUDA 确实在跑，但当前管线里收益被两件事吃掉了**

1. **一次调用建一次会话**（§10.4 的老账）：7 张图里 `dit.step` 1.7 GB，光加载就 ~2.5 s；
2. 3–6 s 的片段上，每步计算太小，CUDA 的节点调度 + H2D/D2H 拷贝把收益抹平。

→ 下一个该做的是 **进程内缓存 Session**（只对 GPU 后端 / 大图开），把 §10.4 那条待办真正做掉，
再复测。另一个待查项是 `dit.step` 的算子覆盖率（ORT 的 CUDA EP 会静默把不支持的节点留在 CPU，
「建会话成功」不等于「全在 GPU 上」）。

### 11.4 还缺的

- CUDA 的**依赖**没有自动下载：cuDNN 9 要手动放进 `<运行时目录>/cuda/cudnn/`（或设 `AIDAW_CUDNN_DIR`）；
  CUDA 12 的 4 个库会自动去 `AIDAW_CUDA_DIR` / `CUDA_PATH\bin` / 本机 `v12.*` toolkit 里找。
  下一步可以把 cuDNN 也做成一个 asset（PyPI wheel 768 MB，实测大小）。
- 所有运行时的 `sha256` 目前是 `None`（只校验体积）。清单结构的字段已经留好，拿到值就填。
### 11.5 借用本机已有的 CUDA 版运行时（2026-10-29 补）

用户机器上往往已经有一份带 CUDA EP 的 ONNX Runtime，没必要让他再下 153 MB。

**能借的形态（都自带完整「core + provider」一对）**

| 来源 | 目录 | 版本从哪读 |
| --- | --- | --- |
| pip `onnxruntime-gpu` | `<site-packages>/onnxruntime/capi/` | 同级 `onnxruntime_gpu-*.dist-info` |
| conda 环境 | 同上 | 同上 |
| 官方 release 解压 | `onnxruntime-win-x64-gpu-*/lib/` | `VERSION_NUMBER` |
| 手动指定 | `AIDAW_ORT_SEARCH`（`;` 分隔） | 目录名末尾的 `x.y.z` |

已核实：`onnxruntime-gpu 1.31.1` 的 Windows wheel 里同时含 `onnxruntime.dll`、
`onnxruntime_providers_cuda.dll`、`onnxruntime_providers_shared.dll`（按 HTTP Range 只读
wheel 中央目录得到，未整包下载）。

**三条硬约束**

1. **必须整对借**：core 与 provider 必须出自同一次构建。拿「我们的 1.30 core + 别人 1.17 的
   provider」必崩——所以 `discover` 只认「同一目录里两样都在」。
2. **API ≥ 1.24**：本 crate 按 `api-24` 编译，1.24 起的 core 都能用（实测 1.31 也行）。
3. **别放进 PATH**：ORT 用裸文件名 `LoadLibrary("onnxruntime_providers_cuda.dll")`，Windows
   搜索顺序里 exe 目录优先、其次 PATH——PATH 上有一份陌生版本会被误加载。借用只走
   `init_from(那一份 core)` 这一个入口。

**复制而不是引用**：`import_cuda()` 把 3 个 dll 复制进应用的运行时目录（实测 16921400 +
186986848 + 21856 字节）。对方 `pip uninstall` 或升级版本，我们不受影响。

**顺带把依赖也借了**（768 MB 的大头）：`preload_cuda` 的搜索列表加了
`<site-packages>/nvidia/{cublas,cuda_runtime,cufft,cudnn}/bin`、`<site-packages>/torch/lib`、
`C:\Program Files\NVIDIA\CUDNN\v9*\bin`、`CUDNN_PATH`/`CUDNN_HOME`、conda 的 `Library\bin`。
这与 ORT 自己 Python 版 `preload_dlls()` 的默认顺序一致。

实测：只放 ORT 的三个 dll、不建 `cuda/` 子目录，CUDA 12 的库自动从本机 `v12.8` toolkit 找到，
`[runtime] CUDA EP 可用`。
---

## 12 再调整：CPU / DirectML / CoreML 随包，只有 CUDA 下载（2026-10-30）

§11 把**所有**运行时都做成了下载，代价是「装了程序却跑不了本地 AI，得先下 79 MB」。
按产品决定改回来：

| 运行时 | 交付方式 | 说明 |
| --- | --- | --- |
| CPU | **随包** | Windows 用 DirectML 那份构建代替（它内置 CPU EP，一份顶两个后端，省 16.5 MB） |
| DirectML | **随包** | Windows：`onnxruntime.dll` + `DirectML.dll`，共 34.2 MB |
| CoreML | **随包** | macOS arm64：`libonnxruntime.dylib`，41.8 MB |
| CUDA | **可选下载** | 只出现在**下载管理器**（设置页只留一个入口），153 MB → 装完 204 MB |

- 随包的三份不可删除；界面不出现「删除」。
- `src-tauri/resources/ort/` 由 `npm run fetch:ort` 生成（`tools/fetch-ort-runtime.mjs`，按当前平台
  只取需要的文件），已接进 `beforeDevCommand` / `beforeBuildCommand`，并写进
  `bundle.resources`；目录本身 `.gitignore`（34 MB 二进制不入库）。
- 打包体积：Windows 安装包 **+约 17 MB**（压缩后）；**macOS 不变**（见 §13）。
- 设置 → AI 只剩一行「运行库 · 打开下载管理器」；CUDA 的下载 / 删除 / 「借用本机已有的」
  全在管理器里（`ManagerPane` 顶部新块）。

### 12.1 为什么以前没有这些下载项

以前 `ort` 是 `download-binaries` **静态链接**：ONNX Runtime 的核心在 `ai-daw.exe` 里，
所以既没有单独文件、也不用下载；DirectML.dll 由 `ort` 的 `copy-dylibs` 放到 target 目录
（但 `bundle.resources` 里没有它，装出来的包其实缺这个文件——顺手修了）。

现在为了「CUDA 可按需下载」必须 `load-dynamic`，运行时不再进 exe，于是 CPU / DML / CoreML
要显式随包。用户在界面上看到的「34 MB」就是以前藏在 exe 里的那一坨。

### 12.2 实测

- `AIDAW_ORT_BUNDLED=src-tauri/resources/ort`：CPU 与 DirectML 都从随包目录加载成功，
  `[runtime] 已加载 …\resources\ort\onnxruntime.dll（directml）`，DirectML EP 注册无回落。
- 只取所需文件的下载器（§11）保留：CPU 5.7 MiB / DirectML 14.7 MiB（整包 78.8 / 204.8 MiB）。
  CUDA 改为整包下载（145/146 MiB，取文件没有收益，还多几次往返）。
### 13 macOS 不变：它走静态链接（2026-10-30 修正）

§12 里「macOS 从 38 MB 涨到约 80 MB」是**错的**，两个数不能比：

| 数 | 是什么 |
| --- | --- |
| 35.3 MB | 发布出来的 **dmg**（压缩后的下载体积，v1.6.2 实测） |
| 41.8 MB | 官方 `libonnxruntime.dylib` **解压后**的磁盘体积 |

而且 macOS 的 ONNX Runtime **本来就是静态链接进二进制的**：pyke 给
`aarch64-apple-darwin+coreml` 只发 `libonnxruntime.a`（实测 76.7 MB），链接 + strip/LTO 之后
就是 app 里那一段代码。所以「38 MB 的 app」里早就含了 ORT，只是看不见。

**结论**：macOS 没有 CUDA、不需要运行期换 dylib，于是它在 `Cargo.toml` 里保持
`download-binaries`（静态），只有 Windows / Linux 用 `load-dynamic`：

```toml
# macOS
ort = { ..., features = ["std", "half", "api-24", "coreml", "download-binaries", "tls-native"] }
# Windows / Linux
ort = { ..., features = ["std", "half", "load-dynamic", "api-24", "cuda", ...] }
```

`backend::runtime` 里对应地：

- `statically_linked()` = `cfg!(target_os = "macos")`；
- `ensure_loaded()` / `loaded()` 在 macOS 直接成功（没有 dylib 要加载）；
- `load` / `locate` / CUDA 依赖预加载只在非 macOS 编译；
- macOS 的运行时清单为空（管理器里不出现运行时块），后端选项一律算「已装好」，
  `restart_required` 恒为 false。

Windows / Linux 仍然：CPU + DirectML（或 CPU）随包，CUDA 可选下载（下载管理器里）。
### 14 运行时和模型一样：可下载 → 重新安装，已安装 → 删除（2026-10-30）

最后一版，运行时被当成**和模型同类的条目**：

| 位置 | 动作 |
| --- | --- |
| 可下载（chip：全部 / 乐器 / 效果 / 模型 / **运行时**） | 未装 → `下载 146 MB`；已装 → `重新安装`；下载中 → `取消` |
| 已安装（chip：全部 / 乐器 / 效果 / 模型 / **运行时**） | 在文件夹中显示、`删除`（点一次确认，与模型同一套） |

- 「运行时」chip 只在有东西可管时出现（可下载：CUDA 未装或有可借用的；已安装：CUDA 已装）；
- 选中「运行时」时，模型与插件都不显示，计数也切到运行时；
- 设 置 → AI →「打开下载管理器」= `requestFocus()` → `tab = "available"` + `kindFilter = "runtime"`；
- 顶部不再有独立的「运行时」标签，也不再显示说明文字（`manager.runtimeHint` 已删）。

实现上运行时复用了模型卡片的两套排版：可下载用 `card col`（信息 + 文件行 + 进度），已安装用
`card`（信息在左、图标按钮在右）。`RuntimeStatus.bundled` 的条目不进这两个列表（CPU / DirectML
/ CoreML 随包，既不用下载也不能删）。

### 15 YuE2：引擎与权重都进下载管理器（2026-10-30）

YuE2 不走 ONNX Runtime（ggml 图 + 自定义算子），所以它的「运行时」是**另一套东西**：
一个 `yue-server.exe` 加一组 `ggml-*.dll`。但**管理方式与模型完全一致**（可下载 → 下载 /
重新安装；已安装 → 显示位置 / 删除），并且**安装包一个字节都不涨**——全部按需下载。

#### 15.1 引擎清单（`crates/local-ai/src/engine.rs`）

上游没有单独发布引擎二进制，它只随 YuE2-Studio 的 **335 MB Windows 便携包**发布。清单
钉死这个归档（sha256 取自发布页 `SHA256SUMS.txt`，本地实测一致），再按 zip 中央目录
**逐文件**取我们需要的那几个：

| 组 | 传输量 | 落地 | 说明 |
| --- | --- | --- | --- |
| `yue2-core`「YuE2 引擎」 | 5.25 MiB | 14.4 MiB | `yue-server.exe` + `ggml.dll` + `ggml-base.dll` + 9 个 CPU 变体 + cudart12 + VC 运行库 |
| `yue2-vulkan`「YuE2 · Vulkan」 | 12.2 MiB | 39.6 MiB | `ggml-vulkan.dll`，落在根目录（ggml 启动时扫 exe 同目录） |
| `yue2-cuda13`「YuE2 · CUDA 13」 | 102.9 MiB | 116.6 MiB | `cuda13/ggml-cuda.dll` |
| `yue2-cuda12`「YuE2 · CUDA 12」 | 148.5 MiB | 162.6 MiB | `cuda12/ggml-cuda.dll`（老卡 / 老驱动） |

- 每个文件都钉了 **sha256**（[`crate::pick`] 逐文件校验；整包回退时再校验整包）；
- CUDA 的两份放在**各自的子目录**里：启动时用 `YUE_CUDA_BACKEND` 指过去，两份才能并存
  （ggml 只会在 exe 同目录里自动加载 `ggml-*.dll`）；
- 目录：`<应用配置目录>/runtime/yue2/`，`preferred` 文件记录用户挑的那一份。

`crate::pick`（原 `runtimes.rs` 里的「只取所需文件」）被抽成独立模块，ONNX Runtime 与
YuE2 共用：`Range: bytes=0-0` 拿总长 → 尾部找 EOCD → 抓中央目录 → 逐条目抓本地头 +
压缩数据 → 解压 → 校验 → 原子落地。

#### 15.2 权重清单（`crates/local-ai/src/models/catalog.rs`，feature = `compose`）

权重来自 `Serveurperso/YuE2-GGUF` 的**一个 commit**（`e630f2b8`），四档量化各自是一个条目，
共用同一个目录 `models/yue2/`（换档时 VAE 已在位就不重下）：

| 条目 | 文件 | 体积 |
| --- | --- | --- |
| `yue2-q5k-m` | `YuE2-3B-Q5_K_M.gguf` + `YuE2-Vae-F32.gguf` | 2.94 GiB |
| `yue2-q6-k` | `YuE2-3B-Q6_K.gguf` + VAE | 3.24 GiB |
| `yue2-q8-0` | `YuE2-3B-Q8_0.gguf` + VAE | 4.04 GiB |
| `yue2-bf16` | `YuE2-3B-BF16.gguf` + VAE | 7.17 GiB |
| `yue2-companion` | `nar_lora_joint_v9.safetensors`（Mothersuperior，可选） | 134 MiB |

- 许可 **CC BY-NC 4.0**：不自带，用户自下；
- **魔搭源暂无**：用户后续自建，在对应 `sources` 里补一条 `kind: "modelscope", region: "cn"`
  即可，界面按网络偏好自动排序；
- 共用文件的删除保护：`model_remove` 发现同目录里还有**已装好**的条目要用这个文件就跳过
  （删 q5 不会把 q6 的 VAE 一起删掉）。

#### 15.3 界面

引擎后端与 ONNX Runtime 共用「运行时」那张列表（`RuntimeStatus.group = "ort" | "yue2"`）：

- 可下载：未装 → `下载 · 5.2 MB`；已装 → `重新安装`；下载中 → `取消`；
- 已安装：`显示位置`、`删除`（点一次确认）；引擎行多一个 `设为默认`（`ai_yue_select`），
  不设就按显卡自动挑：N 卡且有对应包 → CUDA 13 → CUDA 12 → Vulkan → CPU；
- 徽标按组显示「运行时」/「音乐引擎」，默认那份带「默认」；
- 进度事件同时发 `engine://yue` 与 `engine://runtime`，下载管理器只订阅后者即可。

#### 15.4 还没做的

- **生成链路**（`POST /synth`、进度 `/logs`、结果 ABC 谱 + 48 kHz 音频导入工程）留到下一步；
  引擎现在只能下载 / 安装 / 选后端，还不能出声；
- macOS / Linux 的引擎：分别在 `.dmg` / `.deb` 里，要额外解包链（`hdiutil` / `ar` + `xz`），
  `engine::packs()` 目前对非 Windows 返回空表（界面上自然不出现）；
- 权重没有国内源（等自建魔搭）；
- 引擎的第三方许可（yue2.cpp MIT、ggml MIT、便携包版权归 YuE2-Studio）需要在「关于」里补署名。

### 16 切后端的两个对话框（2026-10-30）

设置页的「运行库」原先是一行裸 `row`（没有卡片底、和上面那张后端卡不像），现在是**和上面
同一种卡片**（`card` + `row` + `row-text` + `row-actions`），「重启后生效」贴着按钮显示。

执行后端的按钮不再因为「运行时没下载」而置灰——点了以后分两种情况：

| 情况 | 行为 |
| --- | --- |
| 运行时不存在 | **不改设定**（旧后端继续生效，没有「切过去再回退」这一步），弹对话框：「运行时还没下载 / 「{backend}」需要先下载运行时（{size}）。现在打开下载管理器并开始下载吗？」→「去下载」 |
| 运行时存在 | 正常切换；`restart_required` 为真时弹对话框：「运行库已切换 / 新的运行库会在下次启动时加载，现在重启吗？」→「立即重启」（`relaunchApp`） |

- 「去下载」= `runtimes.requestFocus()` → 打开管理器 → `runtimes.install(id)`，**连点击都不用再点一次**；
- 体积取 `RuntimeStatus.bytes`（清单里的传输量），拿不到时退化成不带体积的说法；
- 对话框复用 `ConfirmDialog`（MD3 外壳、Escape / 点遮罩关闭），不另外造轮子。

### 17 写歌跑起来：引擎进程 + 生成任务（2026-10-30）

引擎（`yue-server`）是独立进程、独立 HTTP 协议，宿主只当它是**外部服务**。全部协议细节
都在 `src-tauri/src/yue_engine.rs` 里，界面拿到的只有「状态 + 一段音频 + 一份 ABC 谱」。

#### 17.1 起停

```text
yue-server --host 127.0.0.1 --port <空闲端口>
           --model <backbone.gguf> --vae <vae.gguf> [--companion <nar_lora_v9.safetensors>]
           --device CUDA0|Vulkan0|CPU
YUE_CUDA_BACKEND=<runtime/yue2/cudaXX/ggml-cuda.dll>   （CUDA 时）
```

- 后端由「运行时」里挑的那份决定（用户选的优先，否则 CUDA 13 → CUDA 12 → Vulkan → CPU）；
- 权重按 `q6_k → q8_0 → q5_k_m → bf16` 取第一个装好的，音色解码器装了才传 `--companion`；
- 端口用 `bind(127.0.0.1:0)` 现挑；`CREATE_NO_WINDOW` 免得弹黑框；
- 启动要加载几 GB 权重，宿主最多等 **5 分钟**，每 10 秒把日志尾巴推给界面；
- 引擎日志落在 `runtime/yue2/server.log`，退出时报错的那几行直接显示在对话框里；
- 应用退出（`RunEvent::ExitRequested`）与 `ai_yue_engine_stop` 都会 `kill` 子进程，
  不会留一个占着显存的孤儿。

#### 17.2 生成（分钟级）

| 步骤 | 调用 |
| --- | --- |
| 提交 | `POST /synth`（style / lyrics / duration / steps / seed / `output_format: wav24`）→ `{"id"}` |
| 轮询 | `GET /job?id=N` 每秒一次，界面按秒显示「生成中…已 N 秒」 |
| 取消 | `POST /job?id=N&cancel=1`（引擎在下一个采样边界停下） |
| 取结果 | `GET /job?id=N&result=1` → `multipart/mixed`：一段请求 JSON + 一段音频 |

multipart 自己拆（boundary 默认 `yue2-batch-boundary`，也认响应头里给的）：
- 音频 → `local-ai/yue2-<时间戳>.wav`（48 kHz 立体声），解码后走统一的
  `AddAudioTrack` 进工程，**一次生成 = 一条撤销记录**；
- 请求 JSON 存成同名 `.json`；里面有 `abc` 就再写一份 `.abc`（谱面），路径随结果回给界面；
- 新轨道起点 0，长度按**当前速度**把秒数换算成拍数（音频本身不跟速度走）。

#### 17.3 界面

「AI 功能」面板里多一张 **YuE2 写歌** 卡片：

- 徽标 = 引擎状态：`未启动` / `加载中` / `CUDA 13`·`Vulkan`·`CPU` / `出错`；
- 条目副标题 = 引擎在做什么（加载日志 / 「生成中…已 N 秒」/ 就绪的设备与权重）；
- 「写歌」打开模态框（`ModalShell`，540px）：风格、歌词（可写 `[Verse]`/`[Chorus]`）、
  时长、步数；引擎没就绪时页脚按钮变成「**启动引擎**」，就绪后才是「生成」；
- 生成中卡片底部是**不确定进度条**（引擎只报 running，不给百分比——不编造进度），
  卡片与对话框的按钮都变成「停止」；
- 完成时弹一次通知、关掉对话框，工程里多一条音频轨；失败把引擎的话原样弹出来。

#### 17.4 还没做的

- 引擎日志没有实时流（`/logs` 是 SSE，目前只在启动阶段与出错时读文件尾巴）；
- 界面还不能选权重档 / 种子（后端已支持 `model` / `seed` 字段，等表单需要时再接）；
- ABC 谱只落盘，还没进乐谱视图（要 ABC → MusicXML 的转换）；
- MCP 工具还没暴露「写歌」（下一步与 SongAgent 合并考虑）。

### 18. AI 三个功能的配置页在**侧栏里占整页**（2026-10-30）

「AI 功能」列表不再承载长表单。三张卡片（**AI 写歌（YuE2）**、**歌声合成**、**音色替换**）
只留四样东西：

| 卡片里的东西 | 说明 |
| --- | --- |
| 徽标 | 权重是否就位（`模型` / `未下载`） |
| 一句话 | 在跑什么 / 缺什么 |
| 模型下拉 | 全部档位，未下载的标出来 |
| 一个按钮 | 已装 →「开始 / 生成歌声 / 替换音色」；缺件 →「下载并…」（先问，确认后去管理器） |

点按钮不是跳到工作区中间，而是把**右栏整页换成配置页**（`aiPage` 状态：`yue` / `singing` / `svc`），
列表与标签条让位，左上角返回列表。理由：这三个功能是右栏的延续，左边的时间线要一直看得见；
把它们做成整页路由会把时间线挤掉，改一句歌词还得再点回去。

- 三个子页放在 `src/components/workspace/panes/ai/`（`AiYuePage` / `AiSingingPage` / `AiSvcPage`），
  共用 `aiPage.css`：返回 + 标题 + 状态徽标 + 一张卡片 + 底部主动作 + 进度条。侧栏只有 ~300 px，
  所以字段默认单列，数值类控件才用 `.sub-pair` / `.sub-trio` 挤一行；按钮一律 `nowrap`。
- 参考声线（工程轨道 / 人声音色库）抽成 `src/composables/useTonePicker.ts`：两个子页各持一份状态、
  互不干扰，而「存入音色库 / 导入人声 / 管理音色库」的行为完全一致。
- 子页自己会在跑完时弹通知，列表那边用 `inAiPage` 让路——两边同时挂载，否则会叠两条一样的 toast。
- 卡片按钮在**清单里没有这一档**（后端还没下发 catalog）时也照样进页：页里能直接去下载管理器，
  而不是点了没反应。

> 早先一版把这三页做成了 `/write/*` 路由（占据工作区中间），已按「侧栏整页」的形态改回并删除路由。

改动：`src/components/workspace/panes/ai/{AiYuePage,AiSingingPage,AiSvcPage}.vue`、`aiPage.css`、
`src/composables/useTonePicker.ts`、`src/components/workspace/panes/AiPanel.vue`、`src/router/index.ts`。

> 同一套「运行后端」行也补进了 `D:\ai-cut` 的 AI 面板（在「命令行与 Skills」下方，切完提示重启），
> 那边沿用它的文案「执行后端」，不新造词。

### 19. CUDA 下「进度条不动 / 显存不涨」的根因：`preload_dylibs` 在错误构造里 panic

**现象**（2026-10-30 实测）：后端切到 CUDA 后跑音色替换，显存不涨、进度条停在 0，
stderr 出现：

```
thread 'ai-daw-svc' panicked at ort-2.0.0-rc.13/src/lib.rs:234:
Failed to load ONNX Runtime dylib: BadVersion { version_str: "1.17.1", path: "onnxruntime.dll" }
```

**根因链条**（`crates/local-ai/tests/runtime_probe.rs` 可复现）：

1. `load()` 先 `preload_cuda()`，它调 `ort::ep::cuda::preload_dylibs(..)`；
2. 那个 helper 逐个 `preload_dylib(..)?`，缺一个就**构造 `ort::Error`**；
3. `ort::Error::new_internal` 会顺手去拿 `ort::api()`（`error.rs`）——此刻 dylib 还没初始化；
4. `setup_api()` 于是退回裸文件名 `onnxruntime.dll`，在系统目录里抓到 Windows 自带的
   **1.17.1**（`C:\WINDOWS\system32\onnxruntime.dll`），版本不达标 → `expect` panic；
5. 本机 `CUDA_DYLIBS` 里的 `cufft64_11.dll` 不在我们下载的组件里 → 第 2 步**必然**失败
   → 必然 panic → 后台线程死掉、`finish()` 不执行 → 界面永远「运行中」。

**修法**：

- `load()` **第一件事**就 `set_var("ORT_DYLIB_PATH", 即将加载的那份)`：任何「没走 init 就摸到
  API」的路径也只会拿到对的 dll（1.30），而不是系统目录里的旧货；
- 不再用 `preload_dylibs`：自己按 `CUDA_DYLIBS` / `CUDNN_DYLIBS` 逐个 `preload_dylib`，
  **缺哪个跳过哪个**（真正缺的库 ORT 在用的时候会自己报错，那时运行时已经加载好了）；
- 加载失败要留痕（以前是静默 `Err`）：`[runtime] 加载 … 失败：…`；
- 后台任务线程一律套 `crate::job::guard(..)`（`catch_unwind`）：任何第三方库 panic 都变成
  「后台任务崩溃：…」的失败消息，而不是让进度条永远停在原地。

**验证**（`cargo test -p ai-daw-local-ai --test runtime_probe -- --nocapture`，
`AIDAW_TEST_RUNTIME` 指到 `%APPDATA%\com.aidaw.app\runtime\ort\cuda`）：

```
[runtime] 已加载 …\ort\cuda\onnxruntime.dll（cuda）
`Session::builder()` 正常
CUDA EP：Ok(true)
```

修之前同一段代码在 `ensure_loaded()` 里就 panic；修之后 CUDA EP 可用，会话能真正建在显卡上。

### 20. 「CUDA 模式下 GPU 利用率不涨、CPU 拉满」：组件目录是分开的，预加载只挑了一个目录

上一条修完 panic 之后，CUDA 能跑了，但实测仍然**全部落在 CPU**（GPU 利用率 0、CPU 70%）。

**根因**：运行时按组件分开装（`runtime/ort/cudart`、`../cublas`、`../cudnn`），
而 `preload_cuda` 的旧逻辑是「先挑**一个**含 `cublas64_12.dll` 的目录，再从这个目录里
拿 `CUDA_DYLIBS` 的全部文件」。挑中的是 `cublas/`，于是：

- `cublas64_12` / `cublasLt64_12` 加载了；
- `cudart64_12.dll`、`cufft64_11.dll` 在**别的目录**，全被当成「缺失」跳过；
- ORT 的 CUDA EP 初始化时找不到 `cudart64_12`（系统 PATH 里只有 CUDA 13 的 `cudart64_13`），
  注册失败 → `build_for` **静默回落 CPU**（只在 stderr 留一行，容易被刷过去）。

**修法**：改成**按文件找目录**——对 `CUDA_DYLIBS` / `CUDNN_DYLIBS` 里的每个文件名，
依次在所有候选目录（组件目录 → `AIDAW_CUDA_DIR` → `CUDA_PATH` → pip wheel → `torch/lib`）
里找第一个存在的再 `preload_dylib`，缺的跳过并（`AIDAW_ORT_DEBUG=1` 时）列出来。

**可观测性**（这次排查花了太久，补上）：

- `session::build` 现在统计「建在加速器上 / 回落 CPU」的会话数（`take_session_stats()`）；
- 音色替换 / 歌声合成结束时的摘要会带上 `，GPU 7 / CPU 0 张图`，stderr 另有一行
  `[backend] 音色替换：GPU 7 张图 / CPU 0 张`；
- 回落时日志带**文件名**与原因：`[backend] dit.step.onnx（音色替换）：CUDA 不可用（…），
  这张图回落 CPU EP（本任务后续同类图也走 CPU）`；
- `backend::init()` 若发现运行时在 init 之前就被加载过（一个进程只能有一份 dylib），
  会打印明确的「需要重启」告警。

> 诚实备注：测试进程里即使没预加载 `cudart64_12`，ORT 也可能从系统 PATH 找到它
> （本机 CUDA 12.4 的 `bin` 在 PATH 上），所以「预加载不完整」不一定是每台机器上的唯一原因。
> 真正的判据是任务摘要里的 `GPU n / CPU m 张图`：`m > 0` 时 stderr 会指名是哪张图、为什么。

**验证**：`crates/local-ai/tests/cuda_session.rs` 把 SVC 那一档 7 张图依次建一遍并断言
`CPU == 0`：

```
[runtime] CUDA 预加载 12 个
[runtime] 已加载 …\ort\cuda\onnxruntime.dll（cuda）
[runtime] CUDA EP 可用
OK dit.step.onnx …
会话统计：GPU 7 / CPU 0
```

---

## 21. 三个「看得见」的问题（2026-10-30 修）

### 21.1 卸载了还留在「已安装」里

三个原因叠在一起，都不在下载逻辑里：

1. **列表不过滤**：`ManagerPane` 的 `visibleComponents` 不检查 `installed`。CUDA 组件卡片在
   「已安装」页签里**无条件**列出，删掉之后它只是把按钮变回「下载」，行本身还在。
2. **Windows 删不掉已加载的 dll**：只要这个进程跑过一次 CUDA 推理，`onnxruntime.dll` /
   `cudart` / `cublas` / `cudnn` 就被映射住了，`fs::remove_dir_all` 必然报拒绝访问。
   （实测这台机器上 `runtime/ort/cudart/` 只剩一个空目录，就是上一次删到一半留下的。）
3. **失败被静默吃掉**：`stores/runtimes.ts` 的 `remove()` 把错误塞进 `error` 后立刻调
   `load()`，而 `load()` 第一行就是 `error = null`——界面既没有提示，条目也原样回来。

修法：

- 组件按页签过滤：「已安装」只列 `installed`，其余在「可下载」里（与运行时本体同一规则）；
- 删不掉的目录记进 `<配置目录>/runtime/ort/pending-remove.json`：`statuses()` 立刻按
  「未安装」返回（用户点了删除，列表马上变），下次启动 `runtimes::init`（**任何 dylib 加载
  之前**）再真正删除；安装成功后撤销这条记录，避免把刚装的当残留删掉；
- 删除失败若连记录都写不下，才把错误抛给界面；`runtimes.error` 现在有 watcher 弹 toast。

### 21.2 不挂代理时卡在境外源上

`candidates_for` 原本只处理 `Domestic` 偏好，而默认偏好是 **`Auto`（自动探测）**——
对运行时下载而言 `Auto` 等于「按清单原序」：永远先试 `api.nuget.org` /
`developer.download.nvidia.com`。这两家在境内多半连不上，而下载器的连接超时是 **30 秒**
（`pick` 也是 30 秒），五个组件逐个卡过去就是好几分钟。模型下载器早就有探测
（`probe_agent` + `Range: bytes=0-0`），只是运行时这条路径没接上。

修法：`install_now` 先按 `model_source_preference` 定区域——显式偏好直接用；`auto` 用
`probe_agent` 各探一次国内外源（3 秒连接 / 4 秒首响应），谁通 / 谁快就排前面，
失败仍回退另一个区域。**一次安装只探一次**（几个组件共用同一个网络结论）。

### 21.3 打开应用黑屏很久

日志（`%APPDATA%\com.aidaw.app\logs\aidaw.log`）里能直接读到三段：

```
[startup] setup 完成，用时 527 ms
[startup] 音频线程收到工程装载，开始重建宿主（+3536 ms）   ← 3 秒兜底定时器抢跑
[startup] webview 开始加载（+18663 ms）                    ← 页面还没开始加载
[startup] 音频宿主：… 素材 20848 ms …（+24432 ms）
```

- 窗口建出来就**可见**，而 WebView 的内容还没开始加载——用户看到的是只有底色的黑块；
- 音频宿主装载要**解码工程里的全部素材**（实测 4–21 秒）。它和 WebView 的首次绘制抢
  CPU / 磁盘，`webview 开始加载` 因此被推到 +11 s / +18 s；
- 更早的兜底定时器是 **3 秒**就装载音频：WebView 慢于 3 秒时它几乎每次都抢跑，
  正好把上面那条变成常态（日志里 setup 之后 3.5 s 必然出现「音频线程收到工程装载」）。

修法：

- 主窗口 `visible: false` + `backgroundColor: "#1c1b1f"`；`PageLoadEvent::Finished` 时才
  `show()`——建窗到内容就绪之间根本不出现；
- 音频宿主改由前端触发：`main.ts` 挂载后 250 ms 调新命令 `app_ready`
  （`commands::app_ready`，只认主窗口），它先 `show()` 再让后台线程延迟 120 ms 装载；
- 兜底定时器 3 s → 6 s，且超时先显示窗口再装载；`index.html` 早就在首帧前铺好主题底色，
  所以窗口出现时不会闪白。

---

## 22. 双源补齐：audio.cpp 引擎与 Linux ORT CUDA（2026-10-31）

### 22.1 audio.cpp 引擎 6 个归档

魔搭 `ailm32442/audiocpp-assets` 已镜像 v0.9.3 的全部发布产物（34 个文件、6.54 GB，
`LATEST` / `SHA256SUMS` 齐全）。Windows 的 6 个归档与清单**逐包 sha256 一致**（CPU 包实测
下载后哈希对得上）：

| 归档 | 国际 | 国内 |
| --- | --- | --- |
| CPU / Vulkan / CUDA 12 bin+cudart / CUDA 13 bin+cudart | GitHub Releases | 魔搭 `resolve/master/<同名文件>` |

`engine.rs` 每个 `EngineArchive` 现在是 `intl + cn` 两条源；`yue.rs` 的 `resolve_region` 在
`auto` 下用 `probe_agent` 各探一次（`Range: bytes=0-0`，3s 连接 / 4s 响应），国内网络直接
把魔搭排前面，不再先等 GitHub 的 30 秒超时。显式 `Domestic` / `International` 仍直接生效，
下载失败会回退另一边。

### 22.2 Linux ORT CUDA 12 / 13 本体

以前 Linux 的 CUDA 本体钉的是 GitHub 的 `onnxruntime-linux-x64-gpu_cuda*.tgz`，所以只有
国际源；Windows 用的 NuGet 包与它**不是同一个文件**，不能直接照搬 URL。现在 Linux 也改走
NuGet（`Microsoft.ML.OnnxRuntime.GPU.Linux`）：

| 运行时 | 版本 | 包内路径 | 国际 / 国内 |
| --- | --- | --- | --- |
| CUDA 12 | **1.26.0** | `runtimes/linux-x64/native/libonnxruntime.so` | nuget.org / 华为镜像 |
| CUDA 13 | **1.31.1** | 同上 | nuget.org / 华为镜像 |

**版本是按 CUDA 大版本钉的，不是按系统**：1.30+ 的 Linux GPU 包已经是 CUDA 13 构建
（`libonnxruntime_providers_cuda.so` 的 `DT_NEEDED` 是 `libcublas.so.13` /
`libcudart.so.13`）；CUDA 12 必须用 1.26（`libcublas.so.12` / `libcudart.so.12` /
`libcufft.so.11` / `libcudnn.so.9`），与 Windows 的 1.26 对齐。两个包都在华为镜像上取了
同版本、同大小（225,183,750 / 244,926,281 字节），sha256 已钉进清单。

包里的核心库是**不带版本后缀**的 `libonnxruntime.so`（ELF，已验），`library_path` 仍按
清单的 `required` 首项取，不硬拼文件名。

### 22.3 跨平台引擎包（2026-10-31）

`engine::packs` 现在按平台返回：

| 平台 | 包 | 归档 |
| --- | --- | --- |
| Windows x64 | CPU / Vulkan / CUDA 12 / CUDA 13 | zip（原有） |
| macOS arm64 | Metal | `bin-macos-arm64-metal.tar.gz` |
| Linux x64 | CPU / Vulkan / CUDA 12 | `tar.gz`（CUDA 12 = colab bin + cudart） |

- `EngineArchive` 增加 `format: Zip | TarGz`；`yue.rs::extract` 分两条路，tar 走 `entry.unpack()`
  （保留 `audiocpp_server` 的 0755），zip 仍流式写盘；
- 新包全部国内外双源（GitHub + 魔搭同名文件），sha256 / 体积钉死；
- macOS 没有 CPU 包，Metal 就是默认；Linux 没有 CUDA 13 上游 bin（只有 lib），所以不列；
- 已用本机 `.tmp/audiocpp-v093` 的真实归档跑过 `extract`（mac Metal / Linux CPU / Linux Vulkan），
  文件清单、可执行位、`missing()` 全过；**没有在 macOS / Linux 真机上启动过引擎**。

### 22.4 这一轮补的边角

- **进度秒数**：`ai_yue_compose` 起一个 500ms 的 ticker，持续发 `running + elapsed`；以前只在
  开始发 0、结束发总时长，真机上看起来像卡死。
- **`/health` 不等于权重已加载**：0.9.3 的 `/health` 在权重上传前就 200（`/v1/models` 也会先报
  `loaded:true`）；UI 现在明确写「首次生成才加载权重」，并由 ticker 显示真实等待时间。
- **CUDA 13 驱动门槛**：`auto_device` / `usable` 读 `HardwareProfile.driver`，<580 或读不到时
  自动落到 CUDA 12；显式选 CUDA 13 也会被拒绝。
- **`cuda_perf` 开关**：`store.json` 新增 `yue_cuda_perf`（默认 true），AI 写歌页可关；下次启动
  引擎生效。
- **重装 / 删除先停引擎**：覆盖运行中的 `audiocpp_server` / dylib 在 Windows 上会共享冲突。
- **本地归档安装**：管理器 → 运行时 → audio.cpp 引擎多一个「从本地归档…」，选目录后逐包校验
  sha256 / 体积再解压，不联网。
- **`job::guard`**：安装线程与生成线程都套上；panic 变成 failed 事件，不会卡住 busy。
- **日志**：`ai_yue_engine_log` + AI 写歌页轮询，实时显示 `server.log` 尾巴。
- **MCP**：新增 `compose_song` 工具（阻塞到生成完成，结果入轨）。
- **ABC 谱**：生成完成后**自动**把音频和 ABC 解析出的 MIDI 声部放进**同一个 `run_agent_all`
  事务**：音频插在「生成时选中的轨道」下面，MIDI 谱轨紧跟在音频下面；没有选中轨道时按旧行为
  追加到末尾。一次撤销即可整体回退。`abc.rs` 解析 YuE2 写出的声部/小节/休止/和弦标记/八度/
  时值；复杂 ABC（装饰音 / 连音线 / 反复）只覆盖子集。
- **NOTICE**：补 audio.cpp（Apache-2.0）/ ggml（MIT）/ YuE2 权重（CC BY-NC 4.0，不随包）署名。

### 22.5 还没做

- **Linux / macOS 引擎包只做了结构与解包验证**，没有真机启动 / 生成。
- `/health` 仍不等权重；没有做「启动即预热」的短请求（CPU 上代价太大），只做了文案与进度。
- `ai_yue_import`（本地导入）仍是同步命令，未套 `job::guard`。
- ABC 解析只覆盖 YuE2 会写出的子集（装饰音 / 连音线 / 反复记号会被忽略），遇到复杂 ABC 仍可能丢音。

---

## 23 引擎后端切换：没装就先确认再下载（2026-11-01）

设置页 / AI 面板的「audio.cpp 引擎后端」以前点了没装的后端只弹一句提示并切侧栏标签：
设置页是独立路由，标签根本看不见；而且从不真正开始下载——看起来像引擎后端没实现。

- `EngineBackendPicker` 改成和 ONNX Runtime 后端同一套：没装 → `ConfirmDialog` 报体积 →
  「去下载」→ 打开管理器 → `runtimes.installAndWait(id)` → 装完自动 `ai_yue_select`；
- 设置页里跳转前先 `router.push(lastWorkspace)`，否则只切了侧栏标签；
- `installAndWait` 自己订阅 `engine://runtime`，不依赖管理器是否已挂载（预览模式同步返回）；
- `runtimes.select()` 返回是否成功，失败不再谎报「已切换」。

---

## 24 写歌页重做 + 引擎安装向导（2026-11-01）

写歌页原先用一整套 `.sub-input` / `.sub-ghost` / 原生 `<details>` 自绘控件，和检查器 / 音符
对话框那套 MD3 表单不一致；而且默认没装引擎时只显示一个「启动引擎」，点下去才报错。

- 页面改用 `main.css` 的 MD3 原语：`m3-card` / `m3-row` / `m3-field` / `m3-input` /
  `m3-textarea`（新增）/ `m3-btn`，以及共享控件 `Md3Select` / `Md3Segmented` / `Md3Switch` /
  `NumberField`；
- 底部常驻动作条按「缺什么」给下一步：选择引擎 / 下载权重 / 启动引擎 / 生成；
- 新增 `EngineSetupDialog`：列出本机平台的引擎包（体积、适用显卡、是否已装、当前使用），
  选定 → 跳下载管理器 → `installAndWait` → 自动 `ai_yue_select`；已装时作为「更换」对话框；
- **一份引擎都没有时**，进入写歌页强制弹出安装向导（`ModalShell.dismissible = false`，
  没有 X、点遮罩 / Escape 都不关），只能选一份去下载或返回列表；「启动引擎」也会打开它；
- `ModalShell` 新增 `dismissible` 参数；`main.css` 新增 `.m3-textarea`。

实测（浏览器预览 / mock）：无引擎 → 进页强制弹向导 → 选 CPU 下载 → 管理器里出现「已安装」
并自动选中 → 再进页面不再弹窗。

### 24.1 第二轮收敛（2026-11-01）

- **引擎日志整块移除**：日志是排障信息，不该占写歌页的主视线；`ai_yue_engine_log` / MCP
  工具仍在，页面不再轮询 `server.log`；
- **数字控件统一**：时长 / 步数 / CFG / 种子都是同一种 `NumberField`、同宽 64 px，并且都按
  「标签在左、控件在右」排列；步数 / CFG / 种子收进默认折叠的「高级参数」；
- **作曲方式的选项靠右**，并改写成普通用户能懂的文案（参考 YuE2-Studio 的模型语义：
  full / melody-only / no score）：完整 / 只要旋律 / 不写谱，下面一行解释三者差别；
- **曲谱**不再直接写「ABC」：标签是「曲谱（可选）」，占位与说明写清「留空由模型作曲，
  ABC 是文本谱，示例 X:1 / K:C / C D E F」；
- **CUDA 性能预设只在 CUDA 后端出现**：CPU / Vulkan 下根本不显示这一行；
- **下载管理器移除「从本地归档…」**：引擎后端的本地 zip 导入入口不再暴露，避免用户拿到
  一份不完整归档后卡在 sha256 校验上；
- **底部常驻动作条去掉**：主动作回到「创作」卡片底部（`m3-card-actions`），和歌声合成 /
  音色替换两个子页一致，不再悬浮遮挡内容；
- **切换后弹「现在重启引擎吗？」**：`EngineBackendPicker` / `EngineSetupDialog` / 管理器的
  「设为默认」共用一个 `ConfirmDialog`；确认后 `yue.start(null)`（按默认权重档启动），
  稍后则保持停止，下次生成时按新后端拉起。

### 24.2 CUDA 13「本机不可用」的根因（2026-11-01）

Windows 上 `windows_nvidia_driver()` 读的是 WMI `Win32_VideoController.DriverVersion`
（RTX 4080 / 610.88 报成 `32.0.16.1088`），`cuda13_driver_ok()` 取 major 得到 32，和 580
比较永远是 false——CUDA 13 引擎装好了也选不上，界面只回一句「本机不可用」。

修法：Windows 优先用 `nvidia-smi --query-gpu=driver_version`（本机 `610.88`），PATH 里没有
时试 `C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe`，最后才退回 WMI。实测
`backend_probe`：`驱动 Some("610.88")`、`本机可用 [Cpu, DirectMl, Cuda12, Cuda13]`。

顺带：设置页 / AI 面板里 `EngineBackendPicker` 切换失败时不再静默——把后端给的
「本机不可用」原因直接弹出来；`engine.backendHint` 也写明「audio.cpp 是独立进程，切换
不用重启应用，下次启动引擎时生效」（它不是 ONNX Runtime 那种必须重启的进程内 dylib）。







