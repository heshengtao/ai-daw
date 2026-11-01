//! **SoulX-Singer-SVC 的 7 张 ONNX 图**：参考 mel / F0 编码 / whisper 前端 → 条件 →
//! flow-matching 扩散 → Vocos 声码器（backbone + ISTFT）。
//!
//! 数值契约与 ONNX 导出附带的参考实现 `tools/sxpipeline.py` 一致：
//! 24 kHz / hop 480 / 128 mel / 512 嵌入 / 1024 条件、Euler 32 步、CFG 3.0 + rescale 0.75。
//!
//! 与歌声合成（[`crate::singing::soulx`]）的区别：
//! - SVS 由「音符 + 歌词」生成 mel；SVC 把**参考音频的 mel** 当 prompt，把目标音频的 F0 /
//!   whisper 特征当条件，去重建目标那一段 mel。
//! - 这里每一步**没有**动态阈值（参考实现也没有）：SVC 的 CFG 后处理只有 rescale。
//!
//! 两个精度档（`fp16_hq` / `fp32`）的图**输入输出都是 fp32**（内部权重是 fp16），所以这里
//! 不需要在边界上换类型；`Graph::f16` 仍保留，万一将来导出成 fp16 I/O 也能直接跑。
//!
//! 会话在宿主的工作线程里建、在这条替换结束时就 drop：2.4 GB 的权重不该常驻内存。

use std::path::{Path, PathBuf};

use half::f16;
use ort::session::Session;
use ort::value::{DynTensor, Tensor, TensorElementType, ValueType};

use crate::backend::session::GraphRole;
use crate::rng::Rng;
use crate::Progress;

/// 模型采样率。
pub const SAMPLE_RATE: f64 = 24_000.0;
/// 一 mel 帧的采样数。
pub const HOP: usize = 480;
/// mel 维度。
pub const MEL_DIM: usize = 128;
/// 条件嵌入维度。
pub const COND_DIM: usize = 1024;
/// 内容 / F0 嵌入维度（whisper 与 f0 编码器共用）。
pub const EMB_DIM: usize = 512;
/// 声码器 backbone 输出的系数维度。
pub const COEF_DIM: usize = 1922;
/// 官方 F0 档位下限（与 `f0.encoder` 内部量化一致，仅作文档）。
pub const F0_MIN_HZ: f32 = 32.703_196;
/// 默认采样步数。
pub const DEFAULT_STEPS: u32 = 32;
/// CFG 强度。
pub const CFG_STRENGTH: f32 = 3.0;
/// CFG rescale 权重（`flow = r·rescale + (1−r)·pred`）。
pub const CFG_RESCALE: f32 = 0.75;

/// **重量精度**：同一份导出的两档。
///
/// - `Fp16Hq`（推荐，约 2.4 GB）：声码器 / 条件嵌入用 fp16，**扩散步保留 fp32**。
///   32 步欧拉积分 + CFG 会把单步舍入误差累积放大，纯 fp16 的 DiT 音质明显变差；
///   hq 档与原 PyTorch 的 SNR 约 30 dB。
/// - `Fp32`（约 3.0 GB）：全 fp32 参考版，SNR 约 33 dB；只在确实需要最高保真时下载。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    /// 推荐档（fp16 声码器 / 条件 + fp32 扩散）。
    Fp16Hq,
    /// 全 fp32。
    Fp32,
}

impl Precision {
    /// 全部档位（界面 / 清单的展示顺序：推荐档在前）。
    pub const ALL: [Precision; 2] = [Precision::Fp16Hq, Precision::Fp32];

    /// 清单条目 id（也是组目录名）。
    pub fn id(self) -> &'static str {
        match self {
            Precision::Fp16Hq => "soulx-svc-fp16hq",
            Precision::Fp32 => "soulx-svc-fp32",
        }
    }

    /// 展示名（界面 / 错误信息用）。
    pub fn label(self) -> &'static str {
        match self {
            Precision::Fp16Hq => "fp16_hq",
            Precision::Fp32 => "fp32",
        }
    }

    /// 组目录名（与清单里的 `dir` 一致）。
    pub fn dir(self) -> &'static str {
        self.id()
    }

    /// 从「模型」标识解析：清单 id，也接受 `fp16hq` / `fp16_hq` / `fp32` 简写。
    pub fn from_model(raw: &str) -> Result<Self, String> {
        match raw.trim() {
            "soulx-svc-fp16hq" | "fp16hq" | "fp16_hq" | "fp16" => Ok(Precision::Fp16Hq),
            "soulx-svc-fp32" | "fp32" => Ok(Precision::Fp32),
            other => Err(format!(
                "未知音色替换模型 `{other}`（可选 soulx-svc-fp16hq / soulx-svc-fp32）"
            )),
        }
    }
}

/// 默认档的组目录名；保留给「只看默认安装」的调用方。
pub const DIR: &str = "soulx-svc-fp16hq";

/// 组内必须齐备的文件（7 张自足图，没有外部权重）。
pub const FILES: &[&str] = &[
    "whisper.frontend.onnx",
    "mel.transform.onnx",
    "f0.encoder.onnx",
    "cond.emb.onnx",
    "dit.step.onnx",
    "vocoder.backbone.onnx",
    "vocoder.istft.onnx",
];

/// 某一档模型所在目录（整组齐备才算）。
pub fn model_dir_for(precision: Precision) -> Option<PathBuf> {
    let entry = crate::models::catalog::by_id(precision.id())?;
    crate::models::catalog::group_dir(entry)
}

/// 已安装的档位，按 [`Precision::ALL`] 的顺序（推荐档在前）。
pub fn installed_precisions() -> impl Iterator<Item = Precision> {
    Precision::ALL
        .into_iter()
        .filter(|p| model_dir_for(*p).is_some())
}

/// 没显式指定模型时用哪个：推荐档优先。
pub fn default_precision() -> Option<Precision> {
    installed_precisions().next()
}

/// 解析「要用的模型」：显式指定就必须已安装；没指定就取默认已安装档。
pub fn resolve(model: Option<&str>) -> Result<(Precision, PathBuf), String> {
    let p = match model {
        Some(raw) => Precision::from_model(raw)?,
        None => default_precision().ok_or_else(|| {
            "未安装音色替换模型。到「管理器 → 模型」下载 SoulX-Singer-SVC（fp16_hq 约 2.4 GB，fp32 约 3.0 GB）"
                .to_string()
        })?,
    };
    match model_dir_for(p) {
        Some(dir) => Ok((p, dir)),
        None => {
            let installed: Vec<&str> = installed_precisions().map(|x| x.label()).collect();
            let hint = if installed.is_empty() {
                "到「管理器 → 模型」下载".to_string()
            } else {
                format!(
                    "到「管理器 → 模型」下载，或改用已安装的 {}",
                    installed.join(" / ")
                )
            };
            Err(format!(
                "未安装模型 {}（{} 档）：{}",
                p.id(),
                p.label(),
                hint
            ))
        }
    }
}

/// 默认已安装档的目录。
pub fn model_dir() -> Option<PathBuf> {
    default_precision().and_then(model_dir_for)
}

/// 模型是否就位（任意一档）。
pub fn installed() -> bool {
    default_precision().is_some()
}

/// 某一档是否就位。
pub fn installed_for(precision: Precision) -> bool {
    model_dir_for(precision).is_some()
}

/// 一张图 + 它的输入精度（fp16 导出 or fp32 导出）。
struct Graph {
    session: Session,
    f16: bool,
}

impl Graph {
    fn open(path: &Path) -> Result<Self, String> {
        let session = crate::backend::session::build(path, GraphRole::Svc)?;
        let f16 = matches!(
            session.inputs().first().map(|i| i.dtype()),
            Some(ValueType::Tensor {
                ty: TensorElementType::Float16,
                ..
            })
        );
        Ok(Self { session, f16 })
    }

    /// 按图声明的元素类型包张量（f32 数据 → fp16 或 fp32）。
    fn tensor(&self, shape: &[usize], data: &[f32]) -> Result<DynTensor, String> {
        if self.f16 {
            let values: Vec<f16> = data.iter().map(|v| f16::from_f32(*v)).collect();
            Ok(Tensor::from_array((shape.to_vec(), values))
                .map_err(|e| format!("建 fp16 张量失败：{e}"))?
                .upcast())
        } else {
            Ok(Tensor::from_array((shape.to_vec(), data.to_vec()))
                .map_err(|e| format!("建 fp32 张量失败：{e}"))?
                .upcast())
        }
    }

    /// 跑一次；返回**第一个**输出（7 张图都只有一个输出）。
    fn run(&mut self, inputs: Vec<(&str, &[usize], &[f32])>) -> Result<Vec<f32>, String> {
        let mut feed = Vec::with_capacity(inputs.len());
        for (name, shape, data) in inputs {
            feed.push((name, self.tensor(shape, data)?));
        }
        let outputs = self
            .session
            .run(feed)
            .map_err(|e| format!("音色替换模型推理失败：{e}"))?;
        as_f32(&outputs[0])
    }
}

/// 输出张量 → f32（fp16 / fp32 都吃）。
fn as_f32(value: &ort::value::DynValue) -> Result<Vec<f32>, String> {
    match value.dtype() {
        ValueType::Tensor {
            ty: TensorElementType::Float16,
            ..
        } => {
            let (_, data) = value
                .try_extract_tensor::<f16>()
                .map_err(|e| format!("读 fp16 输出失败：{e}"))?;
            Ok(data.iter().map(|v| v.to_f32()).collect())
        }
        _ => {
            let (_, data) = value
                .try_extract_tensor::<f32>()
                .map_err(|e| format!("读 fp32 输出失败：{e}"))?;
            Ok(data.to_vec())
        }
    }
}

/// 加载后的推理器（一次替换用完即弃）。
pub struct Runner {
    mel: Graph,
    f0: Graph,
    whisper: Graph,
    cond: Graph,
    dit: Graph,
    voc_backbone: Graph,
    voc_istft: Graph,
}

impl Runner {
    /// 建全部会话。`progress` 的 0..1 覆盖整个加载阶段。
    pub fn load(dir: &Path, progress: Progress<'_>) -> Result<Self, String> {
        // 按体积给里程碑：diT 1.77 GB、声码器 510 MB 占大头。
        let milestones: [(&str, f32, f32); 7] = [
            ("dit.step.onnx", 0.02, 0.58),
            ("vocoder.backbone.onnx", 0.58, 0.78),
            ("whisper.frontend.onnx", 0.78, 0.88),
            ("mel.transform.onnx", 0.88, 0.92),
            ("vocoder.istft.onnx", 0.92, 0.95),
            ("cond.emb.onnx", 0.95, 0.97),
            ("f0.encoder.onnx", 0.97, 1.00),
        ];
        let mut sessions: Vec<Option<Graph>> = Vec::with_capacity(milestones.len());
        for (file, lo, hi) in milestones {
            if !progress(lo) {
                return Err("已取消".into());
            }
            let graph = Graph::open(&dir.join(file))
                .map_err(|e| format!("加载音色替换模型 {file} 失败：{e}"))?;
            sessions.push(Some(graph));
            if !progress(hi) {
                return Err("已取消".into());
            }
        }
        let mut take = |i: usize| sessions[i].take().expect("每个会话只取一次");
        Ok(Self {
            dit: take(0),
            voc_backbone: take(1),
            whisper: take(2),
            mel: take(3),
            voc_istft: take(4),
            cond: take(5),
            f0: take(6),
        })
    }

    /// 参考 / 目标音频 → 标准化 mel（`[frames, 128]`）。
    ///
    /// `mel_transform` 输出 `samples // 480` 帧，已经做过 `(x − mean) / std`，正是扩散模型
    /// 与声码器要的尺度。
    pub fn mel(&mut self, wav: &[f32]) -> Result<Vec<f32>, String> {
        if wav.is_empty() {
            return Ok(Vec::new());
        }
        let data = self.mel.run(vec![("audio", &[1, wav.len()], wav)])?;
        Ok(data)
    }

    /// F0（Hz，50 fps，清音 0）→ 512 维逐帧嵌入；`pitch_shift` 以半音为单位整体移调。
    pub fn f0_embed(&mut self, f0: &[f32], pitch_shift: i32) -> Result<Vec<f32>, String> {
        let f0_tensor = self.f0.tensor(&[1, f0.len()], f0)?;
        let shift = Tensor::from_array((Vec::<usize>::new(), vec![i64::from(pitch_shift)]))
            .map_err(|e| format!("建 pitch_shift 张量失败：{e}"))?
            .upcast();
        let outputs = self
            .f0
            .session
            .run(vec![("f0", f0_tensor), ("pitch_shift", shift)])
            .map_err(|e| format!("F0 编码推理失败：{e}"))?;
        as_f32(&outputs[0])
    }

    /// 24 kHz 波形 → whisper 编码器状态（`valid_frames × 512`，已按有效长度截断）。
    ///
    /// 前端把重采样 → log-mel → 6 层 encoder 合成一张图；`valid_frames` 用来把零填充区
    /// 精确置 0（HF 是 pad 特征而不是 pad 波形）。
    pub fn whisper(&mut self, wav: &[f32]) -> Result<Vec<f32>, String> {
        // ceil(n × 16000 / 24000)
        let n16 = (wav.len() * 2).div_ceil(3);
        let frames16 = n16 / 160;
        let wav_tensor = self.whisper.tensor(&[1, wav.len()], wav)?;
        let valid = Tensor::from_array((vec![1usize], vec![frames16 as i64]))
            .map_err(|e| format!("建 valid_frames 张量失败：{e}"))?
            .upcast();
        let outputs = self
            .whisper
            .session
            .run(vec![("wav24", wav_tensor), ("valid_frames", valid)])
            .map_err(|e| format!("whisper 前端推理失败：{e}"))?;
        let data = as_f32(&outputs[0])?;
        let total = data.len() / EMB_DIM;
        // 编码器每两个 10 ms 帧出一个状态（conv stride 2），与参考实现同一公式。
        let valid = ((frames16 + 2) / 2).min(total);
        Ok(data[..valid * EMB_DIM].to_vec())
    }

    /// 帧级特征（`[frames, 512]`）→ 条件（`[frames, 1024]`）。
    pub fn condition(&mut self, features: &[f32], frames: usize) -> Result<Vec<f32>, String> {
        let cond = self
            .cond
            .run(vec![("features", &[1, frames, EMB_DIM], features)])?;
        if cond.len() < frames * COND_DIM {
            return Err(format!(
                "条件嵌入长度不对：期望 {}，得到 {}",
                frames * COND_DIM,
                cond.len()
            ));
        }
        Ok(cond[..frames * COND_DIM].to_vec())
    }

    /// flow-matching 采样：从噪声积分到目标 mel（`[frames, 128]`）。
    ///
    /// `prompt` 是参考音色的 mel 前缀（`prompt_frames × 128`），`cond` 必须已经覆盖
    /// `prompt_frames + frames` 帧（prompt 段的条件填 0）。
    #[allow(clippy::too_many_arguments)]
    pub fn diffuse(
        &mut self,
        prompt: &[f32],
        prompt_frames: usize,
        cond: &[f32],
        frames: usize,
        steps: u32,
        seed: u64,
        progress: Progress<'_>,
    ) -> Result<Vec<f32>, String> {
        let steps = steps.clamp(4, 64);
        let total = prompt_frames + frames;
        if prompt.len() < prompt_frames * MEL_DIM {
            return Err("参考音色的 mel 长度不足".into());
        }
        if cond.len() < total * COND_DIM {
            return Err("条件嵌入没有覆盖 prompt + 目标".into());
        }
        let n = frames * MEL_DIM;
        let mut rng = Rng::new(seed);
        let mut xt = vec![0.0f32; n];
        for v in xt.iter_mut() {
            *v = rng.normal();
        }

        let mut cond_xt = vec![0.0f32; total * MEL_DIM];
        let mask_full = vec![1.0f32; total];
        let uncond_cond = vec![0.0f32; frames * COND_DIM];
        let mask_target = vec![1.0f32; frames];
        let mut flow = vec![0.0f32; n];
        let mut uncond = vec![0.0f32; n];
        let h = 1.0 / steps as f32;

        for step in 0..steps {
            if !progress((step as f32 + 0.5) / steps as f32) {
                return Err("已取消".into());
            }
            let t = [(step as f32 + 0.5) * h];

            // cond 分支：[prompt | xt]，mask 全 1。
            cond_xt[..prompt_frames * MEL_DIM].copy_from_slice(prompt);
            cond_xt[prompt_frames * MEL_DIM..].copy_from_slice(&xt);
            let pred = self.dit.run(vec![
                ("xt_input", &[1, total, MEL_DIM], &cond_xt),
                ("t", &[1], &t),
                ("cond", &[1, total, COND_DIM], cond),
                ("xt_mask", &[1, total], &mask_full),
            ])?;
            if pred.len() < total * MEL_DIM {
                return Err(format!(
                    "扩散模型输出长度不对：期望 {}，得到 {}",
                    total * MEL_DIM,
                    pred.len()
                ));
            }
            flow.copy_from_slice(&pred[prompt_frames * MEL_DIM..total * MEL_DIM]);

            // uncond 分支：xt 单独一段、条件全零、mask 只标目标段。
            let pred_u = self.dit.run(vec![
                ("xt_input", &[1, frames, MEL_DIM], &xt),
                ("t", &[1], &t),
                ("cond", &[1, frames, COND_DIM], &uncond_cond),
                ("xt_mask", &[1, frames], &mask_target),
            ])?;
            if pred_u.len() < n {
                return Err(format!(
                    "扩散模型（uncond）输出长度不对：期望 {n}，得到 {}",
                    pred_u.len()
                ));
            }
            uncond.copy_from_slice(&pred_u[..n]);

            // CFG + rescale（与 sxpipeline 一致：rescale 用 pred 自己的标准差）。
            let mean_flow = mean(&flow);
            let pos_std = std_dev(&flow, mean_flow);
            for i in 0..n {
                flow[i] += CFG_STRENGTH * (flow[i] - uncond[i]);
            }
            let mean_pred = mean(&flow);
            let pred_std = std_dev(&flow, mean_pred);
            let scale = if pred_std > 1e-12 {
                pos_std / pred_std
            } else {
                1.0
            };
            for v in flow.iter_mut() {
                *v = CFG_RESCALE * (*v * scale) + (1.0 - CFG_RESCALE) * *v;
            }
            for i in 0..n {
                xt[i] += flow[i] * h;
            }

            if !xt.iter().all(|x| x.is_finite()) {
                return Err("扩散采样出现 NaN / Inf（模型或权重有问题）".into());
            }
        }
        Ok(xt)
    }

    /// 标准化 mel → 24 kHz 波形（`frames × 480` 个采样）。
    pub fn vocode(&mut self, mel: &[f32], frames: usize) -> Result<Vec<f32>, String> {
        if frames == 0 {
            return Ok(Vec::new());
        }
        // backbone 要 `[1, 128, T]`：逐帧转置。
        let mut input = vec![0.0f32; MEL_DIM * frames];
        for t in 0..frames {
            for d in 0..MEL_DIM {
                input[d * frames + t] = mel[t * MEL_DIM + d];
            }
        }
        let coef = self
            .voc_backbone
            .run(vec![("mel", &[1, MEL_DIM, frames], &input)])?;
        if coef.len() < frames * COEF_DIM {
            return Err(format!(
                "声码器系数长度不对：期望 {}，得到 {}",
                frames * COEF_DIM,
                coef.len()
            ));
        }
        let wav = self
            .voc_istft
            .run(vec![("coef", &[1, frames, COEF_DIM], &coef)])?;
        let want = frames * HOP;
        let mut out = wav;
        out.resize(want, 0.0);
        Ok(out)
    }
}

/// 均值。
fn mean(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f32>() / values.len() as f32
}

/// 样本标准差（`ddof = 1`，与 numpy 的 `std(ddof=1)` 一致）。
fn std_dev(values: &[f32], mean: f32) -> f32 {
    if values.len() < 2 {
        return 0.0;
    }
    let var = values.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>();
    (var / (values.len() as f32 - 1.0)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_ids_map_to_tiers() {
        assert_eq!(
            Precision::from_model("soulx-svc-fp16hq").unwrap(),
            Precision::Fp16Hq
        );
        assert_eq!(Precision::from_model("fp16_hq").unwrap(), Precision::Fp16Hq);
        assert_eq!(Precision::from_model(" fp32 ").unwrap(), Precision::Fp32);
        assert!(Precision::from_model("fp64").is_err(), "未知模型要报错");
        // 两档各自对应清单里的一条，且 id 就是目录名。
        for p in Precision::ALL {
            assert!(
                crate::models::catalog::by_id(p.id()).is_some(),
                "{} 缺清单条目",
                p.label()
            );
            assert_eq!(p.dir(), p.id());
        }
        assert_eq!(Precision::ALL[0], Precision::Fp16Hq, "推荐档在前");
    }

    #[test]
    fn std_dev_matches_sample_standard_deviation() {
        let v = [1.0f32, 2.0, 3.0, 4.0];
        let m = mean(&v);
        assert!((m - 2.5).abs() < 1e-6);
        // numpy: np.std([1,2,3,4], ddof=1) = 1.2909944
        assert!((std_dev(&v, m) - 1.290_994_4).abs() < 1e-5);
        assert_eq!(std_dev(&[1.0], 1.0), 0.0);
    }

    #[test]
    fn cfg_rescale_restores_the_cond_branch_scale_before_blending() {
        // rescale 的作用：把 CFG 放大后的场拉回 cond 分支的标准差；随后 0.75/0.25 混合时，
        // 结果的标准差就是 `0.75·σ_flow + 0.25·σ_pred`（不是恰好 σ_flow）。
        let flow = [0.1f32, -0.2, 0.3, -0.4];
        let uncond = [0.05f32, -0.1, 0.15, -0.2];
        let mut pred = flow;
        for i in 0..4 {
            pred[i] += CFG_STRENGTH * (pred[i] - uncond[i]);
        }
        let flow_std = std_dev(&flow, mean(&flow));
        let pred_std = std_dev(&pred, mean(&pred));
        let scale = flow_std / pred_std;
        let rescaled: Vec<f32> = pred.iter().map(|v| v * scale).collect();
        assert!(
            (std_dev(&rescaled, mean(&rescaled)) - flow_std).abs() < 1e-5,
            "rescale 后应与 cond 分支同标准差"
        );
        let blended: Vec<f32> = rescaled
            .iter()
            .zip(pred.iter())
            .map(|(r, p)| CFG_RESCALE * r + (1.0 - CFG_RESCALE) * p)
            .collect();
        let expected = CFG_RESCALE * flow_std + (1.0 - CFG_RESCALE) * pred_std;
        assert!(
            (std_dev(&blended, mean(&blended)) - expected).abs() < 1e-5,
            "混合后的标准差应落在两条分支之间"
        );
    }
}
