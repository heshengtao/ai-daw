//! 音频素材缓存：路径 → 已解码的 PCM。
//!
//! 解码是 IO + CPU 密集的操作，**绝不能发生在音频回调里**（见 `crates/engine/tests/realtime_safety.rs`
//! 的零分配门禁）。音频线程与离线渲染都从这里取素材：重建宿主之前先按工程引用到的路径取一份
//! 快照（`HashMap<String, Arc<DecodedAudio>>`），再交给 `ScheduledEvents::from_project_with_assets`。
//!
//! **失败也会被缓存**：缺失或损坏的文件记成 `None`，避免每次重建都重试一遍，也避免刷屏。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ai_daw_engine::audio::DecodedAudio;

/// 路径 → 素材（`None` = 已知加载失败）。
pub struct AssetCache {
    map: Mutex<HashMap<String, Option<Arc<DecodedAudio>>>>,
}

impl AssetCache {
    /// 空缓存。
    pub fn new() -> Self {
        Self { map: Mutex::new(HashMap::new()) }
    }

    /// 取素材（带缓存）。失败返回可读原因，且**失败结果也会被缓存**。
    pub fn load(&self, path: &str) -> Result<Arc<DecodedAudio>, String> {
        self.load_with_progress(path, |_| {})
    }

    /// 同 [`load`]，但解码过程中把进度回调透出去（供界面画进度条）。
    ///
    /// 命中缓存时**不回调**：素材早就解好了，没有进度可言。
    pub fn load_with_progress(
        &self,
        path: &str,
        mut on_progress: impl FnMut(ai_daw_asset::DecodeProgress),
    ) -> Result<Arc<DecodedAudio>, String> {
        {
            let map = self.map.lock().unwrap();
            match map.get(path) {
                Some(Some(audio)) => return Ok(Arc::clone(audio)),
                Some(None) => return Err(format!("音频素材加载失败：{path}")),
                None => {}
            }
        }
        // 解码在锁外做：它很慢，别把其它路径的查询一起堵住。
        let outcome = match ai_daw_asset::decode_with_progress(path, &mut on_progress) {
            Ok(audio) => Some(Arc::new(audio)),
            Err(err) => {
                eprintln!("[asset] 解码失败 {path}：{err}");
                None
            }
        };
        let mut map = self.map.lock().unwrap();
        let stored = map.entry(path.to_string()).or_insert_with(|| outcome.clone());
        match stored.as_ref() {
            Some(audio) => Ok(Arc::clone(audio)),
            None => Err(format!("音频素材加载失败：{path}")),
        }
    }

    /// 取工程引用到的全部素材（能拿到的；缺失的跳过 → 该片段静音）。
    ///
    /// 返回的表直接实现 `SampleProvider`，可以喂给调度与离线渲染。
    pub fn snapshot_for(&self, paths: &[String]) -> HashMap<String, Arc<DecodedAudio>> {
        let mut out = HashMap::with_capacity(paths.len());
        for path in paths {
            if let Ok(audio) = self.load(path) {
                out.insert(path.clone(), audio);
            }
        }
        out
    }

    /// 波形包络（每桶 min/max 各一个值），供界面绘制。
    pub fn waveform(&self, path: &str, buckets: usize) -> Result<Vec<f32>, String> {
        let audio = self.load(path)?;
        Ok(ai_daw_asset::peaks(&audio, buckets))
    }

    /// 波形包络，只取素材的 `[offset_frames, offset_frames + frames)` 这一段。
    ///
    /// 片段只引用素材的一个窗口；`frames` 为 `None` 表示「到结尾」（旧工程）。
    pub fn waveform_range(
        &self,
        path: &str,
        buckets: usize,
        offset_frames: u64,
        frames: Option<u64>,
    ) -> Result<Vec<f32>, String> {
        let audio = self.load(path)?;
        let window = frames.unwrap_or_else(|| (audio.frames() as u64).saturating_sub(offset_frames));
        Ok(ai_daw_asset::peaks_range(&audio, offset_frames, window, buckets))
    }

    /// 插入一份已解码素材（录音刚落盘时用：免去立刻重解码）。
    pub fn insert(&self, path: &str, audio: Arc<DecodedAudio>) {
        self.map.lock().unwrap().insert(path.to_string(), Some(audio));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_serves_a_predecoded_asset_without_touching_disk() {
        let cache = AssetCache::new();
        let audio = Arc::new(DecodedAudio {
            samples: vec![0.25f32, -0.25].into(),
            channels: 2,
            sample_rate: 48_000.0,
        });
        cache.insert("/nonexistent/rec.wav", Arc::clone(&audio));
        let got = cache.load("/nonexistent/rec.wav").expect("插入过的路径必须直接命中");
        assert_eq!(got.samples.len(), 2);
        assert_eq!(got.sample_rate, 48_000.0);
    }
}

impl Default for AssetCache {
    fn default() -> Self {
        Self::new()
    }
}
