//! 给界面画实时波形用的环形缓冲。
//!
//! 音频线程只写、界面线程只读，两边都**不加锁**：音频线程在任何情况下都不能等锁。
//! 界面可能读到正好被覆盖的位置，但那只是波形上一个点的抖动，肉眼看不出来。

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// 单声道采样的环形缓冲（容量在构造时定死，之后不再分配）。
#[derive(Debug)]
pub struct ScopeBuffer {
    /// 采样以 `f32` 的位模式存放，才能塞进原子里。
    samples: Box<[AtomicU32]>,
    /// 单调递增的写入计数；对容量取模得到真实下标。
    written: AtomicUsize,
}

impl ScopeBuffer {
    /// 建一个能装 `capacity` 个采样的环形缓冲。
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(2);
        let samples = (0..capacity)
            .map(|_| AtomicU32::new(0))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        ScopeBuffer { samples, written: AtomicUsize::new(0) }
    }

    /// 推入一块交错音频（多声道下混成单声道）。
    ///
    /// **音频线程调用**：只有原子写，没有分配、没有锁。
    pub fn push_interleaved(&self, interleaved: &[f32], frames: usize) {
        let cap = self.samples.len();
        let channels = if frames == 0 { 0 } else { interleaved.len() / frames };
        if channels == 0 {
            return;
        }
        let mut head = self.written.load(Ordering::Relaxed);
        for frame in 0..frames {
            let base = frame * channels;
            let mono = if channels >= 2 {
                (interleaved[base] + interleaved[base + 1]) * 0.5
            } else {
                interleaved[base]
            };
            self.samples[head % cap].store(mono.to_bits(), Ordering::Relaxed);
            head += 1;
        }
        // Release：界面线程看到 `written` 更新时，前面的采样一定已经写好了。
        self.written.store(head, Ordering::Release);
    }

    /// 取最近 `out.len()` 个采样（写入顺序）。
    ///
    /// **界面线程调用**。`out` 比容量长时，多出来的部分会被填 0。
    pub fn read_last(&self, out: &mut [f32]) {
        let cap = self.samples.len();
        let head = self.written.load(Ordering::Acquire);
        let want = out.len();
        let count = want.min(cap);
        let pad = want - count;
        let start = head.saturating_sub(count);

        for slot in out.iter_mut().take(pad) {
            *slot = 0.0;
        }
        for i in 0..count {
            let idx = (start + i) % cap;
            out[pad + i] = f32::from_bits(self.samples[idx].load(Ordering::Relaxed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_back_what_was_pushed() {
        let scope = ScopeBuffer::new(8);
        scope.push_interleaved(&[0.0, 0.0, 0.5, 0.5, -0.5, -0.5], 3);
        let mut out = [9.0f32; 3];
        scope.read_last(&mut out);
        assert_eq!(out, [0.0, 0.5, -0.5]);
    }

    #[test]
    fn stereo_is_downmixed_and_only_the_newest_kept() {
        let scope = ScopeBuffer::new(4);
        // 左 1.0 / 右 0.0 → 0.5；写 6 个点，环容量 4 → 只留最后 4 个。
        let block: Vec<f32> = (0..6).flat_map(|_| [1.0f32, 0.0]).collect();
        scope.push_interleaved(&block, 6);
        let mut out = [0.0f32; 4];
        scope.read_last(&mut out);
        assert!(out.iter().all(|v| (*v - 0.5).abs() < 1e-6), "{out:?}");
    }

    #[test]
    fn asking_for_more_than_capacity_pads_with_zeros() {
        let scope = ScopeBuffer::new(2);
        scope.push_interleaved(&[0.25, 0.25], 1);
        let mut out = [9.0f32; 4];
        scope.read_last(&mut out);
        assert_eq!(out, [0.0, 0.0, 0.25, 0.0]);
    }
}
