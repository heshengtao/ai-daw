//! **可复现的噪声**：SplitMix64 → 标准正态（Box–Muller）。
//!
//! 两条扩散管线（歌声合成 [`crate::singing`]、音色替换 [`crate::svc`]）都要从同一个种子
//! 生成同一段噪声：同一份输入 + 同一个种子必须得到同一条输出，否则「重新生成一次听听」
//! 就成了抽卡，A/B 也没法做。
//!
//! 不用 `rand` 是刻意的：这里只需要正态分布，自己写的 SplitMix64 是稳定实现，跨版本、
//! 跨平台都不会因为上游换了算法而变（`rand` 的 `StdRng` 不保证跨版本可复现）。

/// SplitMix64 + Box–Muller。
pub(crate) struct Rng {
    state: u64,
    /// Box–Muller 一次生成两个数，第二个先存着。
    spare: Option<f32>,
}

impl Rng {
    /// 从一个 u64 种子建流。
    pub(crate) fn new(seed: u64) -> Self {
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

    /// (0, 1] 均匀分布（避免 `ln(0)`）。
    fn uniform(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32 + 1.0) / 16_777_217.0
    }

    /// 标准正态。
    pub(crate) fn normal(&mut self) -> f32 {
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

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        assert_ne!(a.normal(), b.normal());
    }
}
