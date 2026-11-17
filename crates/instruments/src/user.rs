//! 用户导入的 SoundFont（`.sf2` / `.sf3`）。
//!
//! 内置乐器是**编译期内嵌**的固定 18 件；这里处理的是用户自己拖进来的库：
//!
//! - 同一个文件在进程内只解析一次（`Arc<SoundFont>` 缓存），一首曲子里挂同一个库的多个
//!   预设不会把几百 MB 的采样解码多遍；
//! - 解析出的预设坐标是 `(bank, program)`，也就是 SF2 的 preset 编号——它不是 DAW 的
//!   「轨道」概念，一条库可能展开成几十上百个可选音色；
//! - 插件 id 由**文件内容哈希 + 坐标**组成（见 [`plugin_id`]），所以工程里存的 id 在重命名
//!   文件、换机器之后仍然指向同一个音色。

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use rustysynth_ext::SoundFont;

/// 一个 SoundFont 文件里的一个预设。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankPreset {
    /// SF2 的 bank 编号（`wBank`），128 以上是「bank MSB/LSB」拼出来的，不能当成通用 MIDI 的 bank。
    pub bank: i32,
    /// SF2 的 preset 编号（`wPreset`）。
    pub program: i32,
    /// 预设名，取自 phdr chunk，通常是作者写的音色名（如 `Grand Piano`）。
    pub name: String,
}

/// 已解析字体缓存：路径 → 共享字体。
///
/// 用路径而不是内容哈希做键：导入时文件已经落到托管目录，路径是稳定的；换库（同路径不同内容）
/// 由导入流程显式 [`evict`] 掉旧条目。
fn cache() -> &'static Mutex<HashMap<PathBuf, Arc<SoundFont>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<SoundFont>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 从磁盘加载（或命中缓存）一个 SoundFont。
///
/// 失败返回可读的原因：文件不存在、不是 RIFF/sfbk、采样区损坏（rustysynth 的 sanity check）。
/// 调用方负责决定这是「导入失败」还是「工程里的音色暂时缺失」。
pub fn load_font(path: &Path) -> Result<Arc<SoundFont>, String> {
    if let Some(font) = cache().lock().unwrap().get(path).cloned() {
        return Ok(font);
    }
    let file = File::open(path).map_err(|e| format!("打不开 {}：{e}", path.display()))?;
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let font = SoundFont::new(&mut reader).map_err(|e| format!("不是可用的 SoundFont：{e}"))?;
    let font = Arc::new(font);
    cache().lock().unwrap().insert(path.to_path_buf(), Arc::clone(&font));
    Ok(font)
}

/// 解析一个文件里的全部预设。顺序与文件里的 phdr 一致（不做排序，界面自己排）。
pub fn inspect(path: &Path) -> Result<Vec<BankPreset>, String> {
    let font = load_font(path)?;
    Ok(presets_of(&font))
}

/// 从内存字节解析（导入时先校验再落盘，避免把坏文件拷进托管目录）。
pub fn inspect_bytes(bytes: &[u8]) -> Result<Vec<BankPreset>, String> {
    let mut cursor = std::io::Cursor::new(bytes);
    let font = SoundFont::new(&mut cursor).map_err(|e| format!("不是可用的 SoundFont：{e}"))?;
    Ok(presets_of(&font))
}

fn presets_of(font: &SoundFont) -> Vec<BankPreset> {
    font.get_presets()
        .iter()
        .map(|preset| BankPreset {
            bank: preset.get_bank_number(),
            program: preset.get_patch_number(),
            name: preset.get_name().trim().to_string(),
        })
        .collect()
}

/// 插件 id：`sf2.user.<文件 id>.<bank>.<program>`。
///
/// `file_id` 是文件 sha256 的前 8 位（由导入流程给出）。用内容哈希而不是文件名：用户改了
/// 文件名、重装、或从另一台机器同步工程，id 依旧命中同一份采样。
pub fn plugin_id(file_id: &str, bank: i32, program: i32) -> String {
    format!("sf2.user.{file_id}.{bank}.{program}")
}

/// 丢掉某个路径的缓存（重新导入同路径文件、或删除库时调用）。
pub fn evict(path: &Path) {
    cache().lock().unwrap().remove(path);
}

/// 缓存里的字体数量（诊断 / 测试用）。
pub fn cached_fonts() -> usize {
    cache().lock().unwrap().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtin_bytes(kind: crate::Kind) -> &'static [u8] {
        crate::asset_bytes(kind)
    }

    #[test]
    fn inspect_reads_presets_from_a_real_soundfont() {
        let presets = inspect_bytes(builtin_bytes(crate::Kind::Piano)).expect("内置钢琴应当可解析");
        assert!(!presets.is_empty(), "至少有一个预设");
        assert!(
            presets.iter().any(|p| p.name.to_lowercase().contains("piano")),
            "预设名应当来自 phdr：{:?}",
            presets.iter().map(|p| &p.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn inspect_rejects_non_soundfont_bytes() {
        let err = inspect_bytes(b"not a riff file").unwrap_err();
        assert!(err.contains("SoundFont"), "错误信息要能看懂：{err}");
    }

    #[test]
    fn user_plugin_renders_sound_from_a_file() {
        use crate::SoundFontPlugin;
        use ai_daw_plugin_api::{MidiEvent, Plugin, ProcessContext, TimedMidiEvent};

        // 把内置钢琴写成一个真实文件，走和用户导入完全一样的路径。
        let dir = std::env::temp_dir().join(format!("ai-daw-user-sf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("piano.sf3");
        std::fs::write(&path, builtin_bytes(crate::Kind::Piano)).expect("write");

        let font = load_font(&path).expect("load");
        let presets = inspect(&path).expect("inspect");
        let preset = presets.first().expect("a preset").clone();
        let mut plugin = SoundFontPlugin::user(
            plugin_id("deadbeef", preset.bank, preset.program),
            "用户钢琴",
            font,
            preset.bank,
            preset.program,
        );
        assert_eq!(plugin.descriptor().id, "sf2.user.deadbeef.0.0");
        plugin.prepare(48_000.0, 256);

        let ctx = ProcessContext { sample_rate: 48_000.0, block_size: 256, tempo: 120.0, playing: true };
        let mut left = vec![0.0f32; 2048];
        let mut right = vec![0.0f32; 2048];
        let events = [TimedMidiEvent { frame: 0, channel: 0, event: MidiEvent::NoteOn { note: 60, velocity: 100 } }];
        plugin.process(&ctx, &mut [&mut left, &mut right], &events);
        assert!(left.iter().any(|v| v.abs() > 1e-4), "用户音色要能出声");

        // 缓存按路径共享：同一个文件不会解析两遍。
        assert_eq!(cached_fonts(), 1);
        evict(&path);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn plugin_id_is_stable_and_scoped() {
        assert_eq!(plugin_id("9f3a1c7e", 0, 0), "sf2.user.9f3a1c7e.0.0");
        assert_ne!(plugin_id("9f3a1c7e", 0, 0), plugin_id("9f3a1c7e", 0, 1));
    }
}
