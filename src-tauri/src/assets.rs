//! 音频素材缓存：路径 → 已解码的 PCM。
//!
//! 解码是 IO + CPU 密集的操作，**绝不能发生在音频回调里**（见 `crates/engine/tests/realtime_safety.rs`
//! 的零分配门禁）。音频线程与离线渲染都从这里取素材：重建宿主之前先按工程引用到的路径取一份
//! 快照（`HashMap<String, Arc<DecodedAudio>>`），再交给 `ScheduledEvents::from_project_with_assets`。
//!
//! **失败也会被缓存**：缺失或损坏的文件记成 `None`，避免每次重建都重试一遍，也避免刷屏。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use daw_party_engine::audio::DecodedAudio;

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
        {
            let map = self.map.lock().unwrap();
            match map.get(path) {
                Some(Some(audio)) => return Ok(Arc::clone(audio)),
                Some(None) => return Err(format!("音频素材加载失败：{path}")),
                None => {}
            }
        }
        // 解码在锁外做：它很慢，别把其它路径的查询一起堵住。
        let outcome = match daw_party_asset::decode(path) {
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
        Ok(daw_party_asset::peaks(&audio, buckets))
    }
}

impl Default for AssetCache {
    fn default() -> Self {
        Self::new()
    }
}
