//! 通道电平表：音频线程写、界面线程读的无锁环形「峰值累加器」。
//!
//! 每个槽位只做两件事：把本块的峰值**取最大**写进原子量，把削波标记**或**进去。界面侧每次
//! 读取时用 `swap(0)` 取走并清零，于是「两次轮询之间的峰值」天然保留——轮询频率与音频块率
//! 完全解耦（旧实现每 120 ms 只抓 64 个采样，绝大部分峰值都漏了）。
//!
//! # 为什么可以按位取最大
//!
//! 峰值恒为非负数，而非负 IEEE754 `f32` 的位模式按 `u32` 比较是**单调**的（0 与正数同序），
//! 所以 `AtomicU32::fetch_max` 直接就是「取峰值」。负零、NaN 在入口就被夹成 0，不会污染比较。
//!
//! # 契约
//!
//! - 音频线程：只调 [`MeterBank::publish_track`] / [`MeterBank::publish_master`]，纯原子写。
//! - 界面线程：调 `read_*`，一次 `swap` 拿走。弹道、峰值保持、削波显示全在界面上做。
//! - 槽位号 = 该轨在 `project.tracks` 里的下标；超过 [`METER_SLOTS`] 的轨道不计量。

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use serde::{Deserialize, Serialize};

/// 可计量的轨道数上限。
///
/// 固定容量是为了让音频线程永远不必重新分配。真到了 256 轨以上，多出来的轨道只是没有表，
/// 不影响出声。
pub const METER_SLOTS: usize = 256;

/// 一帧电平读数（已从原子里取走）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MeterValue {
    /// 左声道峰值（线性，0..）。
    pub peak_l: f32,
    /// 右声道峰值（线性，0..）。
    pub peak_r: f32,
    /// 增益衰减量（dB，正数 = 压下去多少）；0 = 没压。
    pub gr_db: f32,
    /// 自上次清零以来出现过削波（峰值 ≥ 1.0）。
    pub clipped: bool,
}

impl Default for MeterValue {
    fn default() -> Self {
        MeterValue {
            peak_l: 0.0,
            peak_r: 0.0,
            gr_db: 0.0,
            clipped: false,
        }
    }
}

/// 一个通道的计量槽位。
#[derive(Debug, Default)]
pub struct MeterSlot {
    peak_l: AtomicU32,
    peak_r: AtomicU32,
    gr_db: AtomicU32,
    clipped: AtomicBool,
}

impl MeterSlot {
    /// 写入本块电平（音频线程）。
    ///
    /// `gr_db` 传正数（压下去多少 dB）；非有限值按 0 处理。
    #[inline]
    pub fn publish(&self, peak_l: f32, peak_r: f32, gr_db: f32) {
        let l = sanitize_peak(peak_l);
        let r = sanitize_peak(peak_r);
        self.peak_l.fetch_max(l.to_bits(), Ordering::Relaxed);
        self.peak_r.fetch_max(r.to_bits(), Ordering::Relaxed);
        self.gr_db
            .fetch_max(sanitize_gr(gr_db).to_bits(), Ordering::Relaxed);
        if l >= 1.0 || r >= 1.0 {
            self.clipped.store(true, Ordering::Relaxed);
        }
    }

    /// 取走读数并清零峰值（界面线程）。
    pub fn take(&self) -> MeterValue {
        MeterValue {
            peak_l: f32::from_bits(self.peak_l.swap(0, Ordering::Relaxed)),
            peak_r: f32::from_bits(self.peak_r.swap(0, Ordering::Relaxed)),
            gr_db: f32::from_bits(self.gr_db.swap(0, Ordering::Relaxed)),
            clipped: self.clipped.load(Ordering::Relaxed),
        }
    }

    /// 清掉削波锁存（不动峰值）。
    pub fn clear_clip(&self) {
        self.clipped.store(false, Ordering::Relaxed);
    }
}

/// 峰值夹紧：非有限值 / 负数 → 0。
#[inline]
fn sanitize_peak(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

/// 增益衰减夹紧：非有限值 / 负数 → 0。
#[inline]
fn sanitize_gr(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

/// 全工程的计量槽位：每轨一个 + 主输出一个。
#[derive(Debug)]
pub struct MeterBank {
    tracks: Box<[MeterSlot]>,
    master: MeterSlot,
}

impl Default for MeterBank {
    fn default() -> Self {
        Self::new()
    }
}

impl MeterBank {
    /// 建一张全空的表。
    pub fn new() -> Self {
        MeterBank {
            tracks: (0..METER_SLOTS).map(|_| MeterSlot::default()).collect(),
            master: MeterSlot::default(),
        }
    }

    /// 槽位数（界面遍历用）。
    pub fn slots(&self) -> usize {
        self.tracks.len()
    }

    /// 写入某轨本块的电平（音频线程）。越界槽位静默忽略。
    #[inline]
    pub fn publish_track(&self, slot: usize, peak_l: f32, peak_r: f32, gr_db: f32) {
        if let Some(s) = self.tracks.get(slot) {
            s.publish(peak_l, peak_r, gr_db);
        }
    }

    /// 写入主输出本块的电平（音频线程）。
    #[inline]
    pub fn publish_master(&self, peak_l: f32, peak_r: f32, gr_db: f32) {
        self.master.publish(peak_l, peak_r, gr_db);
    }

    /// 取走某轨的读数并清零。越界槽位返回全零。
    pub fn read_track(&self, slot: usize) -> MeterValue {
        self.tracks.get(slot).map(MeterSlot::take).unwrap_or_default()
    }

    /// 取走主输出的读数并清零。
    pub fn read_master(&self) -> MeterValue {
        self.master.take()
    }

    /// 清掉所有削波锁存。
    pub fn clear_clip(&self) {
        for slot in self.tracks.iter() {
            slot.clear_clip();
        }
        self.master.clear_clip();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_is_held_between_reads_and_cleared_after_take() {
        let bank = MeterBank::new();
        bank.publish_track(0, 0.25, 0.5, 0.0);
        // 第二块更小：峰值必须留在 0.5，而不是被覆盖成 0.1。
        bank.publish_track(0, 0.1, 0.1, 0.0);
        let v = bank.read_track(0);
        assert!((v.peak_l - 0.25).abs() < 1e-6, "{v:?}");
        assert!((v.peak_r - 0.5).abs() < 1e-6, "{v:?}");
        // 取走之后归零。
        let v = bank.read_track(0);
        assert_eq!(v.peak_l, 0.0);
        assert_eq!(v.peak_r, 0.0);
    }

    #[test]
    fn clipping_latches_until_cleared() {
        let bank = MeterBank::new();
        bank.publish_master(1.2, 0.0, 0.0);
        assert!(bank.read_master().clipped);
        // 峰值取走、削波标记仍在。
        assert!(bank.read_master().clipped);
        bank.clear_clip();
        assert!(!bank.read_master().clipped);
    }

    #[test]
    fn gain_reduction_takes_the_max_and_is_non_negative() {
        let bank = MeterBank::new();
        bank.publish_track(1, 0.0, 0.0, 3.0);
        bank.publish_track(1, 0.0, 0.0, 6.0);
        bank.publish_track(1, 0.0, 0.0, -2.0);
        assert!((bank.read_track(1).gr_db - 6.0).abs() < 1e-6);
    }

    #[test]
    fn out_of_range_slot_is_ignored_not_panicking() {
        let bank = MeterBank::new();
        bank.publish_track(METER_SLOTS + 3, 1.0, 1.0, 0.0);
        assert_eq!(bank.read_track(METER_SLOTS + 3).peak_l, 0.0);
    }

    #[test]
    fn nan_and_negative_peaks_do_not_poison_the_max() {
        let bank = MeterBank::new();
        bank.publish_track(2, f32::NAN, -1.0, f32::NAN);
        let v = bank.read_track(2);
        assert_eq!(v.peak_l, 0.0);
        assert_eq!(v.peak_r, 0.0);
        assert_eq!(v.gr_db, 0.0);
    }
}
