//! **SoulX-Singer 的 9 张 ONNX 图**：编码 → 条件 → flow-matching 采样 → 声码器。
//!
//! 数值契约与 SXSEditor 的 `inference/pipeline/` 一致（那是这套 ONNX 导出的参考实现）：
//! 24 kHz / hop 480 / 128 mel / 512 embed / 1024 cond / 361 个 F0 档、Euler 32 步、
//! CFG 3.0 + rescale 0.7、Vocos 1024 帧一块、32 帧重叠。
//!
//! **fp16 与 fp32 都能喂**：图到底是 fp16 还是 fp32 由导出决定（魔搭上的 fp16 仓库整组
//! 都是 fp16 输入/输出），这里在加载时读一次输入类型，之后在边界上转换，内部一律用 f32
//! 计算——同一套代码既能跑 fp16 小模型，也能跑 fp32 高精度模型，不必分叉。
//!
//! 会话在宿主的工作线程里建、在这条合成结束时就 drop：1.4 GB 的权重不该常驻内存
//! （这个应用的前提是「不用的东西不占内存」）。代价是每次合成多几秒加载时间。

use std::path::{Path, PathBuf};

use half::f16;
use ort::session::Session;
use ort::value::{DynTensor, Tensor, TensorElementType, ValueType};

use crate::backend::session::GraphRole;
use crate::Progress;

use super::seq::Sequence;

/// 模型组目录名（与 `models::catalog` 里的条目一致）。
pub const DIR: &str = "soulx-singer-fp16";

/// 组内必须齐备的文件（9 张图 + 外部权重）。
pub const FILES: &[&str] = &[
    "note_text_encoder.onnx",
    "note_text_encoder.onnx.data",
    "note_pitch_encoder.onnx",
    "note_pitch_encoder.onnx.data",
    "note_type_encoder.onnx",
    "note_type_encoder.onnx.data",
    "f0_encoder.onnx",
    "f0_encoder.onnx.data",
    "preflow.onnx",
    "preflow.onnx.data",
    "cond_emb.onnx",
    "cond_emb.onnx.data",
    "diff_step_dml.onnx",
    "vocoder_dml.onnx",
    "mel_transform.onnx",
    "mel_transform.onnx.data",
];

/// 模型采样率。
pub const SAMPLE_RATE: f64 = 24_000.0;
/// 一 mel 帧的采样数。
pub const HOP: usize = 480;
/// mel 维度。
pub const MEL_DIM: usize = 128;
/// 条件嵌入维度。
pub const COND_DIM: usize = 1024;
/// 声码器一块的帧数（1024 帧 ≈ 20.5 秒）。
pub const VOC_CHUNK: usize = 1024;
/// 声码器块间重叠帧数。
pub const VOC_OVERLAP: usize = 32;
/// 参考音色 prompt 最多取多少帧（10 秒）。
pub const PROMPT_MAX_FRAMES: usize = 500;
/// 默认采样步数。
pub const DEFAULT_STEPS: u32 = 32;
/// CFG 强度。
pub const CFG_STRENGTH: f32 = 3.0;
/// CFG rescale 权重。
pub const CFG_RESCALE: f32 = 0.7;

/// 模型所在目录（整组齐备才算）。
pub fn model_dir() -> Option<PathBuf> {
    let entry = crate::models::catalog::by_id(DIR)?;
    crate::models::catalog::group_dir(entry)
}

/// 模型是否就位。
pub fn installed() -> bool {
    model_dir().is_some()
}

/// 一张图 + 它的输入精度（fp16 导出 or fp32 导出）。
struct Graph {
    session: Session,
    f16: bool,
}

impl Graph {
    fn open(path: &Path) -> Result<Self, String> {
        let session = crate::backend::session::build(path, GraphRole::Svs)?;
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

    /// 跑一次；返回**第一个**输出（所有 9 张图都只有一个输出）。
    fn run(&mut self, inputs: Vec<(&str, &[usize], &[f32])>) -> Result<Vec<f32>, String> {
        let mut feed = Vec::with_capacity(inputs.len());
        for (name, shape, data) in inputs {
            feed.push((name, self.tensor(shape, data)?));
        }
        let outputs = self
            .session
            .run(feed)
            .map_err(|e| format!("歌声合成模型推理失败：{e}"))?;
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

/// 加载后的推理器（一次合成用完即弃）。
pub struct Runner {
    text: Graph,
    pitch: Graph,
    kind: Graph,
    f0: Graph,
    preflow: Graph,
    cond: Graph,
    mel: Graph,
    diff: Graph,
    vocoder: Graph,
}

impl Runner {
    /// 建全部会话。`progress` 的 0..1 覆盖整个加载阶段。
    pub fn load(dir: &Path, progress: Progress<'_>) -> Result<Self, String> {
        // 按「加载耗时大致与体积成正比」给里程碑（diff 887 MB、vocoder 519 MB 占大头）。
        let milestones: [(&str, f32, f32); 9] = [
            ("note_text_encoder.onnx", 0.02, 0.08),
            ("note_pitch_encoder.onnx", 0.08, 0.11),
            ("note_type_encoder.onnx", 0.11, 0.14),
            ("f0_encoder.onnx", 0.14, 0.17),
            ("preflow.onnx", 0.17, 0.25),
            ("cond_emb.onnx", 0.25, 0.30),
            ("mel_transform.onnx", 0.30, 0.33),
            ("diff_step_dml.onnx", 0.33, 0.78),
            ("vocoder_dml.onnx", 0.78, 1.00),
        ];
        let mut sessions: Vec<Option<Graph>> = Vec::with_capacity(milestones.len());
        for (file, lo, hi) in milestones {
            if !progress(lo) {
                return Err("已取消".into());
            }
            let graph = Graph::open(&dir.join(file))
                .map_err(|e| format!("加载歌声合成模型 {file} 失败：{e}"))?;
            sessions.push(Some(graph));
            if !progress(hi) {
                return Err("已取消".into());
            }
        }
        let mut take = |i: usize| sessions[i].take().expect("每个会话只取一次");
        Ok(Self {
            text: take(0),
            pitch: take(1),
            kind: take(2),
            f0: take(3),
            preflow: take(4),
            cond: take(5),
            mel: take(6),
            diff: take(7),
            vocoder: take(8),
        })
    }

    /// 参考音频 → 标准化 mel（`[frames, 128]`，最多 `max_frames` 帧）。
    ///
    /// 输入是 24 kHz 单声道。`mel_transform` 输出 `num_samples // 480` 帧，已经做过
    /// `(x - mean) / std`（mean -4.92 / std 2.85），正是扩散模型和声码器要的尺度。
    pub fn reference_mel(&mut self, mono: &[f32], max_frames: usize) -> Result<Vec<f32>, String> {
        if mono.is_empty() {
            return Ok(Vec::new());
        }
        let usable = mono.len().min(max_frames.max(1) * HOP).max(HOP);
        let data = self
            .mel
            .run(vec![("waveform", &[1, usable], &mono[..usable])])?;
        let frames = (data.len() / MEL_DIM).min(max_frames);
        Ok(data[..frames * MEL_DIM].to_vec())
    }

    /// 音符序列 → 帧级条件（`[prompt_frames + frames, 1024]`）。
    ///
    /// prompt 段的条件填 0——参考实现也是这么做的：参考音色只通过 xt 前缀进入模型。
    pub fn condition(&mut self, seq: &Sequence, prompt_frames: usize) -> Result<Vec<f32>, String> {
        let tokens = &seq.tokens;
        let pitches = &seq.pitches;
        let kinds = &seq.types;
        let frames = seq.frames;
        let len = tokens.len();

        // 四个编码器：int64 输入 → 512 维嵌入（int64 张量必须用 i64 数据，不能走 f32 包装）。
        let text = self.run_i64(GraphRef::Text, tokens, &[1, len])?;
        let pitch = self.run_i64(GraphRef::Pitch, pitches, &[1, len])?;
        let kind = self.run_i64(GraphRef::Kind, kinds, &[1, len])?;
        let f0 = self.run_i64(GraphRef::F0, &seq.f0_ids, &[1, frames])?;

        // 三者逐元素相加，再过 preflow。
        let mut token_emb = vec![0.0f32; len * 512];
        for i in 0..token_emb.len() {
            token_emb[i] = text[i] + pitch[i] + kind[i];
        }
        let processed = self
            .preflow
            .run(vec![("features", &[1, len, 512], &token_emb)])?;

        // 按 mel2token 展开到帧，再叠加 F0 嵌入。
        let mut features = vec![0.0f32; frames * 512];
        for f in 0..frames {
            let token = (seq.mel2token[f].max(0) as usize).min(len - 1);
            features[f * 512..(f + 1) * 512]
                .copy_from_slice(&processed[token * 512..(token + 1) * 512]);
        }
        for i in 0..features.len() {
            features[i] += f0[i];
        }

        let total = prompt_frames + frames;
        let mut code = vec![0.0f32; total * 512];
        code[prompt_frames * 512..].copy_from_slice(&features);
        let cond = self
            .cond
            .run(vec![("cond_code", &[1, total, 512], &code)])?;
        if cond.len() < total * COND_DIM {
            return Err(format!(
                "条件嵌入长度不对：期望 {}，得到 {}",
                total * COND_DIM,
                cond.len()
            ));
        }
        Ok(cond[..total * COND_DIM].to_vec())
    }

    /// 跑一个 int64 输入的编码器。
    fn run_i64(
        &mut self,
        which: GraphRef,
        ids: &[i64],
        shape: &[usize],
    ) -> Result<Vec<f32>, String> {
        let graph = match which {
            GraphRef::Text => &mut self.text,
            GraphRef::Pitch => &mut self.pitch,
            GraphRef::Kind => &mut self.kind,
            GraphRef::F0 => &mut self.f0,
        };
        let tensor = Tensor::from_array((shape.to_vec(), ids.to_vec()))
            .map_err(|e| format!("建 int64 张量失败：{e}"))?
            .upcast();
        let outputs = graph
            .session
            .run(vec![("input_ids", tensor)])
            .map_err(|e| format!("歌声合成编码器推理失败：{e}"))?;
        as_f32(&outputs[0])
    }

    /// flow-matching 采样：从噪声积分到标准化 mel（`[frames, 128]`）。
    ///
    /// `prompt` 是参考音色的 mel 前缀（可为空），`cond` 是 [`Self::condition`] 的输出。
    // 参数就是采样器的全部输入（prompt / cond / 形状 / 步数 / 种子 / 进度），拆成结构体反而
    // 让调用点更难读——和参考实现的 `runDiffusionLoop` 一样保持扁平。
    #[allow(clippy::too_many_arguments)]
    pub fn sample(
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
        let mut rng = Rng::new(seed);
        let mut xt = vec![0.0f32; frames * MEL_DIM];
        for v in xt.iter_mut() {
            *v = rng.normal();
        }

        let mut cond_xt = vec![0.0f32; total * MEL_DIM];
        let mask = vec![1.0f32; total];
        let mut uncond_xt = vec![0.0f32; total * MEL_DIM];
        let mut uncond_mask = vec![0.0f32; total];
        uncond_mask[..frames].fill(1.0);
        let uncond_cond = vec![0.0f32; total * COND_DIM];
        let mut v = vec![0.0f32; frames * MEL_DIM];
        let dt = 1.0 / steps as f32;

        for step in 0..steps {
            if !progress((step as f32 + 0.5) / steps as f32) {
                return Err("已取消".into());
            }
            let tval = (step as f32 + 0.5) / steps as f32;
            let t = [tval];
            let expected = total * MEL_DIM;

            // cond 分支：[prompt | xt]，mask 全 1。
            cond_xt[..prompt_frames * MEL_DIM].copy_from_slice(prompt);
            cond_xt[prompt_frames * MEL_DIM..].copy_from_slice(&xt);
            let pred_c = self.diff.run(vec![
                ("xt_input", &[1, total, MEL_DIM], &cond_xt),
                ("t", &[1], &t),
                ("cond", &[1, total, COND_DIM], cond),
                ("xt_mask", &[1, total], &mask),
            ])?;

            // uncond 分支：xt 放在最前面、后面补零；cond 全零、mask 只标目标段。
            uncond_xt[..frames * MEL_DIM].copy_from_slice(&xt);
            let pred_u = self.diff.run(vec![
                ("xt_input", &[1, total, MEL_DIM], &uncond_xt),
                ("t", &[1], &t),
                ("cond", &[1, total, COND_DIM], &uncond_cond),
                ("xt_mask", &[1, total], &uncond_mask),
            ])?;

            if pred_c.len() < expected || pred_u.len() < expected {
                return Err(format!(
                    "扩散模型输出长度不对：期望 {expected}（{total} 帧），得到 {} / {}",
                    pred_c.len(),
                    pred_u.len()
                ));
            }
            // CFG + rescale，只作用在目标段。
            let c = &pred_c[prompt_frames * MEL_DIM..prompt_frames * MEL_DIM + frames * MEL_DIM];
            let u = &pred_u[..frames * MEL_DIM];
            let mut mean_c = 0.0f32;
            let mut mean_v = 0.0f32;
            for i in 0..v.len() {
                v[i] = c[i] + CFG_STRENGTH * (c[i] - u[i]);
                mean_c += c[i];
                mean_v += v[i];
            }
            let n = v.len() as f32;
            mean_c /= n;
            mean_v /= n;
            let (mut var_c, mut var_v) = (0.0f32, 0.0f32);
            for i in 0..v.len() {
                let dc = c[i] - mean_c;
                let dv = v[i] - mean_v;
                var_c += dc * dc;
                var_v += dv * dv;
            }
            let std_c = (var_c / (n - 1.0).max(1.0)).sqrt();
            let std_v = (var_v / (n - 1.0).max(1.0)).sqrt();
            let rescale = std_c / (std_v + 1e-8);
            let scale = CFG_RESCALE * rescale + 1.0 - CFG_RESCALE;
            for i in 0..xt.len() {
                xt[i] += v[i] * scale * dt;
            }

            if !xt.iter().all(|x| x.is_finite()) {
                return Err("扩散采样出现 NaN / Inf（模型或权重有问题）".into());
            }
        }
        Ok(xt)
    }

    /// 标准化 mel → 24 kHz 波形。长音频按 1024 帧分块、32 帧重叠交叉淡化。
    ///
    /// Vocos 的输出是 `(seq_len - 1) * 480`（"same" padding 少一帧），所以每块多喂一帧
    /// （重复最后一帧）把长度补满，避免每块尾巴少 10 ms。
    pub fn vocode(&mut self, mel: &[f32], frames: usize) -> Result<Vec<f32>, String> {
        if frames == 0 {
            return Ok(Vec::new());
        }
        let mut out = vec![0.0f32; frames * HOP];
        let mut buffer = Vec::with_capacity((VOC_CHUNK + 1) * MEL_DIM);
        let fade = VOC_OVERLAP * HOP;
        let window: Vec<f32> = (0..fade)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / fade as f32).cos())
            .collect();

        let mut start = 0usize;
        let mut first = true;
        while start < frames {
            let end = (start + VOC_CHUNK).min(frames);
            let count = end - start;
            buffer.clear();
            buffer.extend_from_slice(&mel[start * MEL_DIM..end * MEL_DIM]);
            // 多补一帧，抵消 Vocos "same" padding 少一帧。
            buffer.extend_from_slice(&mel[(end - 1) * MEL_DIM..end * MEL_DIM]);
            let wav = self
                .vocoder
                .run(vec![("mel", &[1, count + 1, MEL_DIM], &buffer)])?;
            let base = start * HOP;
            if first {
                let n = wav.len().min(out.len() - base);
                out[base..base + n].copy_from_slice(&wav[..n]);
                first = false;
            } else {
                // 与前一块的尾巴交叉淡化（前一块已经写进 out）。
                let ov = fade.min(wav.len()).min(out.len() - base);
                for i in 0..ov {
                    let w = window[i];
                    out[base + i] = out[base + i] * (1.0 - w) + wav[i] * w;
                }
                let rest = wav.len().saturating_sub(ov);
                let limit = rest.min(out.len() - base - ov);
                out[base + ov..base + ov + limit].copy_from_slice(&wav[ov..ov + limit]);
            }
            if end >= frames {
                break;
            }
            start = end - VOC_OVERLAP.min(count.saturating_sub(1));
            if start >= end {
                break;
            }
        }
        Ok(out)
    }
}

/// 编码器选择（避免九个字段各写一遍同样的 int64 调用）。
enum GraphRef {
    Text,
    Pitch,
    Kind,
    F0,
}

/// 可复现的噪声（SplitMix64 → 标准正态，Box-Muller）。
struct Rng {
    state: u64,
    spare: Option<f32>,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Self {
            state: seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1),
            spare: None,
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn uniform(&mut self) -> f32 {
        // (0, 1]：避免 log(0)。
        ((self.next_u64() >> 40) as f32 + 1.0) / 16_777_217.0
    }

    fn normal(&mut self) -> f32 {
        if let Some(v) = self.spare.take() {
            return v;
        }
        let u1 = self.uniform();
        let u2 = self.uniform();
        let r = (-2.0 * u1.ln()).sqrt();
        let theta = std::f32::consts::TAU * u2;
        self.spare = Some(r * theta.sin());
        r * theta.cos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_and_reasonably_gaussian() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        let xs: Vec<f32> = (0..20_000).map(|_| a.normal()).collect();
        let ys: Vec<f32> = (0..20_000).map(|_| b.normal()).collect();
        assert_eq!(xs, ys, "同一个种子必须给出同一段噪声");
        let mean = xs.iter().sum::<f32>() / xs.len() as f32;
        let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / xs.len() as f32;
        assert!(mean.abs() < 0.05, "均值应接近 0：{mean}");
        assert!((var - 1.0).abs() < 0.1, "方差应接近 1：{var}");
    }
}
