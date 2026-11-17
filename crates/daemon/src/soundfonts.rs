//! 用户导入的 SoundFont（`.sf2` / `.sf3`）托管库。
//!
//! 内置采样乐器是编译进二进制的 18 件；用户自己的库不能这么办（体积从几 MB 到几百 MB，
//! 而且要能删），所以这里做三件事：
//!
//! 1. **托管目录**：`<数据目录>/soundfonts/`，文件按 `sha256 前 8 位_原名.扩展名` 落地。
//!    复制而不是引用：工程换机器、原文件被移动都不会让音色失效。
//! 2. **清单**：`<数据目录>/soundfonts/soundfonts.json`，存哈希、体积、预设表、导入时间。
//!    清单损坏只会丢「已导入列表」，不会让应用起不来。
//! 3. **插件 id**：`sf2.user.<hash8>.<bank>.<program>`，由**内容哈希**决定，所以重命名文件、
//!    重装、跨机器同步工程都仍然指向同一个音色（见 [`plugin_id`]）。
//!
//! 解析（解码 SF3 的 Ogg）发生在导入时和首次创建实例时，不在音频回调里。

use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 清单文件名（与库文件同一个目录，方便用户整个文件夹拷走）。
const MANIFEST: &str = "soundfonts.json";
/// 单文件上限：SF2 大库能到几百 MB，给到 2 GB 足够，超过的多半不是 SoundFont。
const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// 一个预设（SF2 的 bank/program 组合）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundFontPreset {
    pub bank: i32,
    pub program: i32,
    pub name: String,
}

/// 一件已导入的库。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundFontEntry {
    /// 内容哈希前 8 位，同时也是插件 id 的中段。
    pub id: String,
    /// 显示名（用户导入时给的名字，默认文件名）。
    pub name: String,
    /// 托管目录里的绝对路径。
    pub file: String,
    /// `sf2` / `sf3`。
    pub format: String,
    pub size: u64,
    pub sha256: String,
    pub presets: Vec<SoundFontPreset>,
    /// 导入时间（Unix 秒）。
    pub imported_at: i64,
    /// 来源（从注册表商店装的是 slug，本地导入的是原路径），便于「这个库哪来的」。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// 注册表要把每个预设注册成一条插件：这里是装配所需的最小信息。
#[derive(Debug, Clone)]
pub struct Registration {
    pub id: String,
    pub name: String,
    pub file: PathBuf,
    pub bank: i32,
    pub program: i32,
}

struct Store {
    dir: PathBuf,
    entries: Vec<SoundFontEntry>,
}

fn store() -> &'static Mutex<Option<Store>> {
    static STORE: OnceLock<Mutex<Option<Store>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(None))
}

fn dir_of() -> Option<PathBuf> {
    store().lock().unwrap().as_ref().map(|s| s.dir.clone())
}

/// 启动时钉死托管目录并读清单。重复调用是幂等的（只换目录重新读一次）。
///
/// 拿不到目录（例如单元测试里没起应用）时 `catalog()` 返回空表，插件注册表就只有内置音色。
pub fn init(dir: PathBuf) {
    let entries = read_manifest(&dir);
    let mut guard = store().lock().unwrap();
    *guard = Some(Store { dir, entries });
}

/// 已导入的库（按导入时间倒序，最近导入的排前面）。
pub fn catalog() -> Vec<SoundFontEntry> {
    store()
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| {
            let mut entries = s.entries.clone();
            entries.sort_by(|a, b| b.imported_at.cmp(&a.imported_at).then(a.name.cmp(&b.name)));
            entries
        })
        .unwrap_or_default()
}

/// 按 id 取一条。
pub fn entry(id: &str) -> Option<SoundFontEntry> {
    store()
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|s| s.entries.iter().find(|e| e.id == id).cloned())
}

/// 全部预设展开成注册表条目。
pub fn registrations() -> Vec<Registration> {
    let mut out = Vec::new();
    for entry in catalog() {
        for preset in &entry.presets {
            out.push(Registration {
                id: plugin_id(&entry.id, preset.bank, preset.program),
                // 显示名带上库名：同一个工程里挂两个钢琴库时，界面才分得清。
                name: if preset.name.is_empty() {
                    entry.name.clone()
                } else {
                    format!("{} · {}", entry.name, preset.name)
                },
                file: PathBuf::from(&entry.file),
                bank: preset.bank,
                program: preset.program,
            });
        }
    }
    out
}

/// 插件 id：`sf2.user.<hash8>.<bank>.<program>`。
pub fn plugin_id(file_id: &str, bank: i32, program: i32) -> String {
    ai_daw_instruments::plugin_id(file_id, bank, program)
}

/// 导入一个 `.sf2` / `.sf3`。
///
/// - 按内容去重：同一个库再导入一次直接返回已有条目（不会复制第二份）；
/// - 先校验再落地：解析失败的文件不会进托管目录；
/// - `name` 缺省用文件名；`source` 记录来源（注册表 slug / 原路径）。
pub fn import(path: &Path, name: Option<String>, source: Option<String>) -> Result<SoundFontEntry, String> {
    let dir = dir_of().ok_or_else(|| "SoundFont 库还没初始化（应用未就绪）".to_string())?;
    fs::create_dir_all(&dir).map_err(|e| format!("创建 {} 失败：{e}", dir.display()))?;

    let format = format_of(path).ok_or_else(|| "只支持 .sf2 / .sf3".to_string())?;
    let meta = fs::metadata(path).map_err(|e| format!("读不到 {}：{e}", path.display()))?;
    if !meta.is_file() {
        return Err("请选择一个文件，不是文件夹".into());
    }
    if meta.len() > MAX_BYTES {
        return Err(format!(
            "文件太大（{} GB），超过 2 GB 上限",
            meta.len() as f64 / 1024.0 / 1024.0 / 1024.0
        ));
    }

    // 1. 复制到托管目录的临时文件，同时算 sha256（一次读盘）。
    let tmp = dir.join(format!(".import-{}.tmp", std::process::id()));
    let (sha256, size) = copy_and_hash(path, &tmp)?;
    if size == 0 {
        let _ = fs::remove_file(&tmp);
        return Err("文件是空的".into());
    }

    // 2. 同一份内容已经导入过 → 丢掉临时文件，直接返回老条目。
    let existing = store()
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|s| s.entries.iter().find(|e| e.sha256 == sha256).cloned());
    if let Some(entry) = existing {
        let _ = fs::remove_file(&tmp);
        return Ok(entry);
    }

    // 3. 落盘：id 用 8 位，撞了就加长（不同内容同前缀，概率极低但要有确定性处理）。
    let id = unique_id(&sha256);
    let stem = sanitize_stem(path.file_stem().and_then(|s| s.to_str()).unwrap_or("soundfont"));
    let dest = dir.join(format!("{id}_{stem}.{format}"));
    fs::rename(&tmp, &dest).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("保存到托管目录失败：{e}")
    })?;

    // 4. 校验并枚举预设。失败就把文件删掉，托管目录里不留半成品。
    let presets = match ai_daw_instruments::user::inspect(&dest) {
        Ok(presets) if !presets.is_empty() => presets,
        Ok(_) => {
            ai_daw_instruments::user::evict(&dest);
            let _ = fs::remove_file(&dest);
            return Err("这个 SoundFont 里没有任何预设".into());
        }
        Err(err) => {
            ai_daw_instruments::user::evict(&dest);
            let _ = fs::remove_file(&dest);
            return Err(err);
        }
    };

    let entry = SoundFontEntry {
        id,
        name: name
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("SoundFont")
                    .to_string()
            }),
        file: dest.to_string_lossy().into_owned(),
        format: format.to_string(),
        size,
        sha256,
        presets: presets
            .into_iter()
            .map(|p| SoundFontPreset {
                bank: p.bank,
                program: p.program,
                name: p.name,
            })
            .collect(),
        imported_at: now_secs(),
        source,
    };

    {
        let mut guard = store().lock().unwrap();
        let state = guard.as_mut().ok_or_else(|| "SoundFont 库还没初始化（应用未就绪）".to_string())?;
        state.entries.push(entry.clone());
        save_manifest(&state.dir, &state.entries)?;
    }
    Ok(entry)
}

/// 删除一件已导入的库（文件 + 清单条目 + 进程内字体缓存）。
pub fn remove(id: &str) -> Result<(), String> {
    let mut guard = store().lock().unwrap();
    let state = guard.as_mut().ok_or_else(|| "SoundFont 库还没初始化（应用未就绪）".to_string())?;
    let index = state
        .entries
        .iter()
        .position(|e| e.id == id)
        .ok_or_else(|| "没有这件音色库".to_string())?;
    let entry = state.entries.remove(index);
    let path = PathBuf::from(&entry.file);
    ai_daw_instruments::user::evict(&path);
    match fs::remove_file(&path) {
        Ok(()) => {}
        // 文件已经不在了不算失败：用户可能手动删过，清单要能自愈。
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            // 文件删不掉就把条目放回去，别让清单指向一个「已经消失但还显示可用」的库。
            state.entries.insert(index, entry);
            return Err(format!("删除 {} 失败：{err}", path.display()));
        }
    }
    save_manifest(&state.dir, &state.entries)
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn format_of(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "sf2" => Some("sf2"),
        "sf3" => Some("sf3"),
        _ => None,
    }
}

/// 复制文件并同时算 sha256。返回 (hex, 字节数)。
fn copy_and_hash(from: &Path, to: &Path) -> Result<(String, u64), String> {
    let mut input = fs::File::open(from).map_err(|e| format!("打不开 {}：{e}", from.display()))?;
    let mut output = fs::File::create(to).map_err(|e| format!("写入 {} 失败：{e}", to.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    let mut total = 0u64;
    loop {
        let read = input.read(&mut buffer).map_err(|e| format!("读取失败：{e}"))?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > MAX_BYTES {
            let _ = fs::remove_file(to);
            return Err("文件超过 2 GB 上限".into());
        }
        hasher.update(&buffer[..read]);
        output.write_all(&buffer[..read]).map_err(|e| format!("写入失败：{e}"))?;
    }
    output.flush().ok();
    Ok((hex(hasher.finalize().as_slice()), total))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// 选一个不和现有条目冲突的 id 前缀（默认 8 位，冲突时加长到 12 / 16…）。
fn unique_id(sha256: &str) -> String {
    let taken: HashSet<String> = store()
        .lock()
        .unwrap()
        .as_ref()
        .map(|s| s.entries.iter().map(|e| e.id.clone()).collect())
        .unwrap_or_default();
    let mut length = 8;
    while length < sha256.len() {
        let candidate = &sha256[..length];
        if !taken.contains(candidate) {
            return candidate.to_string();
        }
        length += 4;
    }
    sha256.to_string()
}

/// 文件名只留安全字符，避免路径穿越与跨平台非法字符。
fn sanitize_stem(stem: &str) -> String {
    let mut out = String::new();
    for ch in stem.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else if ch.is_alphanumeric() {
            // 中文等非 ASCII 名字保留（文件系统都能存），只是去掉分隔符与符号。
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-').trim_matches('.');
    let clipped: String = trimmed.chars().take(60).collect();
    if clipped.is_empty() {
        "soundfont".to_string()
    } else {
        clipped
    }
}

fn manifest_path(dir: &Path) -> PathBuf {
    dir.join(MANIFEST)
}

fn read_manifest(dir: &Path) -> Vec<SoundFontEntry> {
    let Ok(text) = fs::read_to_string(manifest_path(dir)) else {
        return Vec::new();
    };
    match serde_json::from_str::<Vec<SoundFontEntry>>(&text) {
        Ok(entries) => entries
            .into_iter()
            // 清单可能比磁盘旧（用户手删了文件）：过滤掉，别在界面上留幽灵条目。
            .filter(|entry| Path::new(&entry.file).is_file())
            .collect(),
        Err(err) => {
            eprintln!("soundfonts.json 无法解析，按空列表处理：{err}");
            Vec::new()
        }
    }
}

fn save_manifest(dir: &Path, entries: &[SoundFontEntry]) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建 {} 失败：{e}", dir.display()))?;
    let text = serde_json::to_string_pretty(entries).map_err(|e| e.to_string())?;
    let target = manifest_path(dir);
    let tmp = target.with_extension("json.tmp");
    fs::write(&tmp, format!("{text}\n")).map_err(|e| format!("写入清单失败：{e}"))?;
    // Windows 的 rename 不覆盖已存在的目标。
    let _ = fs::remove_file(&target);
    fs::rename(&tmp, &target).map_err(|e| format!("保存清单失败：{e}"))
}

/// 库是**进程级单例**（真实应用里本来就只有一份），所以所有碰它的测试必须串行。
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 库是**进程级单例**（真实应用里本来就只有一份），所以测试必须串行，否则一个用例的
    /// `init` 会把另一个用例正在写的目录换掉。
    /// 拿一份真实可解析的 SF3 当「用户文件」：内置钢琴资产。
    fn builtin_sf3() -> &'static [u8] {
        include_bytes!("../../instruments/assets/piano.sf3")
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ai-daw-soundfonts-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn import_copies_hashes_and_lists_presets() {
        let _guard = crate::soundfonts::test_lock();
        let dir = temp_dir("import");
        init(dir.clone());
        let source = dir.join("Piano Sampler.sf3");
        // 用内置钢琴资产当「用户文件」：真实可解析的 SF3。
        fs::write(&source, builtin_sf3()).expect("write sf3");

        let entry = import(&source, None, Some("test".into())).expect("import");
        assert_eq!(entry.format, "sf3");
        assert_eq!(entry.name, "Piano Sampler");
        assert!(!entry.presets.is_empty(), "预设要枚举出来");
        assert!(Path::new(&entry.file).is_file(), "文件要落到托管目录");
        assert_eq!(entry.id.len(), 8);
        assert!(entry.file.contains(&entry.id));

        // 同一个文件再导入一次 = 去重，不复制第二份。
        let again = import(&source, Some("改名了".into()), None).expect("re-import");
        assert_eq!(again.id, entry.id);
        assert_eq!(catalog().len(), 1);

        // 清单能落盘并重新读回。
        let text = fs::read_to_string(manifest_path(&dir)).expect("manifest");
        let parsed: Vec<SoundFontEntry> = serde_json::from_str(&text).expect("parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].sha256, entry.sha256);

        remove(&entry.id).expect("remove");
        assert!(catalog().is_empty());
        assert!(!Path::new(&entry.file).exists(), "文件也要删掉");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn import_rejects_garbage_and_leaves_nothing_behind() {
        let _guard = crate::soundfonts::test_lock();
        let dir = temp_dir("garbage");
        init(dir.clone());
        // 源文件放在子目录里：托管目录本身只应该出现「导入成功的库 + 清单」。
        let src = dir.join("src");
        fs::create_dir_all(&src).expect("create src");
        let source = src.join("fake.sf2");
        fs::write(&source, b"definitely not a soundfont").expect("write");

        let err = import(&source, None, None).unwrap_err();
        assert!(err.contains("SoundFont"), "错误要说人话：{err}");
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .map(|e| e.file_name())
            .filter(|name| name != MANIFEST)
            .collect();
        assert!(leftovers.is_empty(), "坏文件不该留在托管目录：{leftovers:?}");
        assert!(catalog().is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn manifest_drops_entries_whose_file_disappeared() {
        let _guard = crate::soundfonts::test_lock();
        let dir = temp_dir("ghost");
        init(dir.clone());
        let source = dir.join("a.sf3");
        fs::write(&source, builtin_sf3()).expect("write");
        let entry = import(&source, None, None).expect("import");
        fs::remove_file(&entry.file).expect("delete file behind the store's back");
        init(dir.clone());
        assert!(catalog().is_empty(), "文件没了就不能在清单里显示");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn registrations_expose_one_plugin_per_preset() {
        let _guard = crate::soundfonts::test_lock();
        let dir = temp_dir("regs");
        init(dir.clone());
        let source = dir.join("a.sf3");
        fs::write(&source, builtin_sf3()).expect("write");
        let entry = import(&source, Some("钢琴库".into()), None).expect("import");

        let regs = registrations();
        assert_eq!(regs.len(), entry.presets.len(), "每个预设一条注册项");
        assert!(
            regs.iter().all(|reg| reg.id.starts_with(&format!("sf2.user.{}.", entry.id))),
            "id 必须以库 id 为中段：{:?}",
            regs.iter().map(|r| &r.id).collect::<Vec<_>>()
        );
        assert!(regs.iter().all(|reg| reg.name.contains("钢琴库")), "显示名带上库名");
        assert_eq!(regs[0].file, std::path::PathBuf::from(&entry.file));

        remove(&entry.id).expect("remove");
        assert!(registrations().is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn plugin_id_shape_is_documented() {
        let _guard = crate::soundfonts::test_lock();
        assert_eq!(plugin_id("0123abcd", 0, 0), "sf2.user.0123abcd.0.0");
    }
}
