//! 实时混音的**抽头**：音频线程把最终立体声混音推出去，控制侧按自己的节奏读。
//!
//! 与 [`crate::scope::ScopeBuffer`] 同一套思路：音频线程只做原子写，读者只做原子读，
//! **不加锁、不分配**。写满一圈就覆盖最旧的样本——对监听来说丢几十毫秒远好过让
//! 音频线程等一秒。

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// 立体声交错采样的环形缓冲。
///
/// 容量按**帧**算（一帧 = 左右两样本）；实际数组长度是 `frames * 2`。
pub struct AudioTap {
    /// 左/右交错的样本，以 `f32` 的位模式存放在原子里。
    samples: Box<[AtomicU32]>,
    /// 单调递增的**帧**写入计数。
    written: AtomicUsize,
    /// 设备采样率（Hz）。
    rate: AtomicU32,
    /// 流代数：重建宿主时递增，客户端据此丢弃旧流的数据。
    epoch: AtomicU32,
}

impl std::fmt::Debug for AudioTap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioTap")
            .field("frames", &(self.samples.len() / 2))
            .field("rate", &self.rate.load(Ordering::Relaxed))
            .field("epoch", &self.epoch.load(Ordering::Relaxed))
            .finish()
    }
}

impl AudioTap {
    /// 建一个能装 `frames` 帧的抽头（建议 1–2 秒）。
    pub fn new(frames: usize, rate: u32, epoch: u32) -> Self {
        let frames = frames.max(2);
        let samples = (0..frames * 2)
            .map(|_| AtomicU32::new(0))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            samples,
            written: AtomicUsize::new(0),
            rate: AtomicU32::new(rate),
            epoch: AtomicU32::new(epoch),
        }
    }

    /// 设备采样率。
    pub fn rate(&self) -> u32 {
        self.rate.load(Ordering::Relaxed)
    }

    /// 流代数。
    pub fn epoch(&self) -> u32 {
        self.epoch.load(Ordering::Relaxed)
    }

    /// 当前写入位置（帧）。
    pub fn cursor(&self) -> usize {
        self.written.load(Ordering::Acquire)
    }

    /// **音频线程调用**：推入一块交错立体声。
    pub fn push_interleaved(&self, interleaved: &[f32]) {
        let cap_frames = self.samples.len() / 2;
        let frames = interleaved.len() / 2;
        if frames == 0 {
            return;
        }
        let mut head = self.written.load(Ordering::Relaxed);
        for frame in 0..frames {
            let idx = (head % cap_frames) * 2;
            self.samples[idx].store(interleaved[frame * 2].to_bits(), Ordering::Relaxed);
            self.samples[idx + 1].store(interleaved[frame * 2 + 1].to_bits(), Ordering::Relaxed);
            head = head.wrapping_add(1);
        }
        // Release：读者看到 `written` 更新时，样本一定已经写好了。
        self.written.store(head, Ordering::Release);
    }

    /// **控制侧调用**：把 `cursor` 之后的新样本读进 `out`（交错立体声）。
    ///
    /// 返回实际读到的帧数；`cursor` 会被推进。落后超过一圈时只读最后一圈。
    pub fn read_since(&self, cursor: &mut usize, out: &mut Vec<f32>) -> usize {
        let cap_frames = self.samples.len() / 2;
        let head = self.written.load(Ordering::Acquire);
        if head <= *cursor {
            return 0;
        }
        let mut start = *cursor;
        if head - start > cap_frames {
            // 已经绕圈了：丢掉过旧的部分。
            start = head - cap_frames;
        }
        let frames = head - start;
        out.clear();
        out.reserve(frames * 2);
        for frame in start..head {
            let idx = (frame % cap_frames) * 2;
            let left = f32::from_bits(self.samples[idx].load(Ordering::Relaxed));
            let right = f32::from_bits(self.samples[idx + 1].load(Ordering::Relaxed));
            out.push(left);
            out.push(right);
        }
        *cursor = head;
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_back_what_was_pushed() {
        let tap = AudioTap::new(8, 48_000, 0);
        tap.push_interleaved(&[0.1, -0.1, 0.2, -0.2]);
        let mut cursor = 0;
        let mut out = Vec::new();
        assert_eq!(tap.read_since(&mut cursor, &mut out), 2);
        assert!((out[0] - 0.1).abs() < 1e-6);
        assert!((out[3] + 0.2).abs() < 1e-6);
        // 再读一次没有新数据。
        assert_eq!(tap.read_since(&mut cursor, &mut out), 0);
    }

    #[test]
    fn overrun_keeps_the_newest_window() {
        let tap = AudioTap::new(2, 48_000, 0);
        tap.push_interleaved(&[1.0, 1.0, 2.0, 2.0, 3.0, 3.0]);
        let mut cursor = 0;
        let mut out = Vec::new();
        assert_eq!(tap.read_since(&mut cursor, &mut out), 2);
        assert!((out[0] - 2.0).abs() < 1e-6, "只保留最后两帧");
    }
}
