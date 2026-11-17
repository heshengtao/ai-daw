//! 在线 SoundFont 商店：读 [ai-daw-soundfont-registry] 的静态索引。
//!
//! 与插件商店（[`crate::manager`]）是**两套东西**：插件是「装一个可执行包，然后扫描目录」，
//! SoundFont 是「下一个数据文件，解析预设，落进托管目录」。所以这里只借用下载/解压/校验
//! 那几条经过验证的通道，条目模型是自己的。
//!
//! [ai-daw-soundfont-registry]: https://github.com/heshengtao/ai-daw-soundfont-registry
//!
//! 索引里每条库可以带多个 `files`（SF3 压缩版 / SF2 原版 / 压缩包）。选哪一个由
//! [`choose_file`] 决定：**能一键装、体积最小、格式优先 SF3**。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::manager::{self, StoreRuntime};

/// 索引地址：GitHub raw 为主，jsDelivr 作为镜像（国内直连 raw 常常不通）。
const SOURCES: &[&str] = &[
    "https://raw.githubusercontent.com/heshengtao/ai-daw-soundfont-registry/main/registry.json",
    "https://cdn.jsdelivr.net/gh/heshengtao/ai-daw-soundfont-registry@main/registry.json",
];
/// 索引缓存有效期：与插件商店一致（6 小时）。
const TTL: Duration = Duration::from_secs(6 * 3600);
/// 一次最多导入一个压缩包里的多少个库（Sonatina 那种全集有几十个 sf2）。
const MAX_BANKS_PER_ARCHIVE: usize = 60;
/// 随包快照：网络与磁盘缓存都不可用时的最后兜底。
///
/// 保证「连不上 GitHub」时商店仍能列出已知的几件库（与插件商店的随包注册表同一套思路）。
const BUNDLED: &str = include_str!("../../../src-tauri/resources/soundfont-registry.json");

/// 索引里的一个下载文件。
#[derive(Debug, Clone, Deserialize)]
pub struct StoreFile {
    pub url: String,
    /// `sf2` / `sf3`。
    pub format: String,
    /// `file` / `archive` / `external`。
    pub install: String,
    /// `install = archive` 时的压缩格式（zip / tar.gz / tar.bz2 / tar.xz / 7z）。
    #[serde(default)]
    pub archive: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

/// 索引里的一件库（原始结构，字段与注册表 JSON 对齐）。
#[derive(Debug, Clone, Deserialize)]
pub struct StoreBank {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    pub license: String,
    #[serde(default)]
    pub license_url: Option<String>,
    #[serde(default)]
    pub formats: Vec<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub presets: Option<u32>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub files: Vec<StoreFile>,
}

#[derive(Debug, Deserialize)]
struct Registry {
    schema_version: u32,
    soundfonts: Vec<StoreBank>,
}

/// 搜索/列表结果（给界面用；比原始结构多了「本机装没装」「能不能一键装」）。
#[derive(Debug, Clone, Serialize)]
pub struct StoreMatch {
    pub slug: String,
    pub name: String,
    pub vendor: String,
    pub description: String,
    pub license: String,
    pub license_url: Option<String>,
    pub homepage: Option<String>,
    pub formats: Vec<String>,
    pub kind: Option<String>,
    pub presets: Option<u32>,
    pub tags: Vec<String>,
    /// 本机是否已经装过（按 `source = slug` 认）。
    pub installed: bool,
    /// 能否一键安装。
    pub installable: bool,
    /// 不能一键安装的原因（界面直接显示，不弹错误框）。
    pub blocked: Option<String>,
    /// 首选下载的体积 / 格式 / 归档类型。
    pub size: Option<u64>,
    pub format: Option<String>,
    pub archive: Option<String>,
}

/// 一次安装的结果。
#[derive(Debug, Clone, Serialize)]
pub struct StoreInstall {
    pub slug: String,
    pub name: String,
    /// 真正进托管目录的库（压缩包里可能有多个）。
    pub imported: Vec<crate::soundfonts::SoundFontEntry>,
    /// 没导入的文件及原因（例如压缩包里的 `.sfz`）。
    pub skipped: Vec<String>,
}

/// 一次拉取的结果：索引 + 它该缓存多久。
///
/// 正常命中是 6 小时；退回**随包快照**时只缓存 60 秒——网络恢复了要能很快重试，
/// 否则用户会以为商店永远停在旧数据上。
struct Cached {
    at: Instant,
    banks: Vec<StoreBank>,
    ttl: Duration,
}

fn cache() -> &'static Mutex<Option<Cached>> {
    static CACHE: OnceLock<Mutex<Option<Cached>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn remember(banks: Vec<StoreBank>, ttl: Duration) -> Vec<StoreBank> {
    *cache().lock().unwrap() = Some(Cached { at: Instant::now(), banks: banks.clone(), ttl });
    banks
}

/// 测试注入口：直接用一份 JSON 顶掉网络（单元测试不联网）。
fn override_registry() -> &'static Mutex<Option<String>> {
    static OVERRIDE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    OVERRIDE.get_or_init(|| Mutex::new(None))
}

/// 读索引（带 6 小时内存缓存与磁盘兜底）。
pub fn catalog(app: &crate::tauri_api::AppHandle, force: bool) -> Result<Vec<StoreBank>, String> {
    if !force {
        if let Some(cached) = cache().lock().unwrap().as_ref() {
            if cached.at.elapsed() < cached.ttl {
                return Ok(cached.banks.clone());
            }
        }
    }

    if let Some(text) = override_registry().lock().unwrap().clone() {
        return parse_registry(&text);
    }

    let disk = disk_cache_path(app);
    let network = manager::StoreRuntime::load(app).network();
    let proxy = manager::resolve_proxy(&network).proxy;
    let mut errors = Vec::new();
    for url in SOURCES {
        match fetch(url, proxy.as_ref()) {
            Ok(text) => match parse_registry(&text) {
                Ok(banks) => {
                    if let Some(path) = &disk {
                        let _ = fs::write(path, &text);
                    }
                    return Ok(remember(banks, TTL));
                }
                Err(err) => errors.push(format!("{url} → {err}")),
            },
            Err(err) => errors.push(format!("{url} → {err}")),
        }
    }

    // 网络全挂：用上一次落盘的索引（过期也胜过「商店打不开」）。
    if let Some(path) = &disk {
        if let Ok(text) = fs::read_to_string(path) {
            if let Ok(banks) = parse_registry(&text) {
                return Ok(remember(banks, TTL));
            }
        }
    }

    // 最后一层：随包快照（`include_str!` 保证一定有）。此时错误不抛给界面——
    // 用旧数据总比「商店整块打不开」好，但要留一条日志说明为什么没更新。
    if let Ok(banks) = parse_registry(BUNDLED) {
        eprintln!(
            "[soundfonts] 索引拉取失败，改用随包快照（{} 件）：{}",
            banks.len(),
            errors.join("；")
        );
        return Ok(remember(banks, Duration::from_secs(60)));
    }

    Err(format!("音色库索引拉取失败：{}", errors.join("；")))
}

/// 按关键词搜索；空关键词返回全部（按名字排序）。
pub fn search(
    app: &crate::tauri_api::AppHandle,
    query: &str,
) -> Result<Vec<StoreMatch>, String> {
    let banks = catalog(app, false)?;
    Ok(search_in(&banks, &installed_slugs(), query))
}

/// 搜索的纯函数部分（不碰网络 / 磁盘，单测直接用）。
fn search_in(
    banks: &[StoreBank],
    installed: &std::collections::HashSet<String>,
    query: &str,
) -> Vec<StoreMatch> {
    let needle = query.trim().to_lowercase();
    let mut out: Vec<(i64, StoreMatch)> = Vec::new();
    for bank in banks {
        let score = if needle.is_empty() {
            0
        } else {
            match score_bank(bank, &needle) {
                Some(score) => score,
                None => continue,
            }
        };
        out.push((score, to_match(bank, installed)));
    }
    out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.name.cmp(&b.1.name)));
    out.into_iter().map(|(_, item)| item).collect()
}

/// 安装一件库，返回真正导入的条目。
pub fn install(
    app: &crate::tauri_api::AppHandle,
    slug: &str,
) -> Result<StoreInstall, String> {
    let banks = catalog(app, false)?;
    let bank = banks
        .iter()
        .find(|bank| bank.slug == slug)
        .ok_or_else(|| format!("索引里没有 slug「{slug}」"))?;
    let dir = crate::paths::data_dir(app)?.join("soundfonts");
    let network = StoreRuntime::load(app).network();
    let proxy = manager::resolve_proxy(&network).proxy;
    install_into(&dir, bank, proxy.as_ref())
}

/// [`install`] 的核心：目录与代理由调用方给（单测因此可以不碰 Tauri）。
pub(crate) fn install_into(
    dir: &Path,
    bank: &StoreBank,
    proxy: Option<&ureq::Proxy>,
) -> Result<StoreInstall, String> {
    let (file, blocked) = choose_file(bank);
    let file = file.ok_or_else(|| {
        format!(
            "「{}」不能一键安装：{}",
            bank.name,
            blocked.unwrap_or_else(|| "没有可用的下载".into())
        )
    })?;
    fs::create_dir_all(dir).map_err(|e| format!("创建 {} 失败：{e}", dir.display()))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let work = dir.join(format!(".store-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&work).map_err(|e| format!("创建临时目录失败：{e}"))?;

    let result = (|| -> Result<StoreInstall, String> {
        let download = work.join("download.bin");
        manager::download_verified(
            &[file.url.clone()],
            &download,
            file.sha256.as_deref().unwrap_or(""),
            file.size.unwrap_or(0),
            proxy,
            None,
            &mut |_, _| {},
        )?;

        let (paths, mut skipped) = if file.install == "archive" {
            let extract = work.join("extract");
            fs::create_dir_all(&extract).map_err(|e| format!("创建解压目录失败：{e}"))?;
            manager::extract_archive(&download, &extract, &file.url)?;
            collect_banks(&extract)
        } else {
            (vec![download.clone()], Vec::new())
        };

        let mut imported = Vec::new();
        for path in &paths {
            let name = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(|stem| stem.replace(['_', '-'], " "))
                .unwrap_or_else(|| bank.name.clone());
            match crate::soundfonts::import(path, Some(name), Some(bank.slug.clone())) {
                Ok(entry) => imported.push(entry),
                Err(err) => skipped.push(format!(
                    "{}：{err}",
                    path.file_name().and_then(|n| n.to_str()).unwrap_or("?")
                )),
            }
        }
        if imported.is_empty() {
            return Err(if skipped.is_empty() {
                "下载成功，但里面没有可用的 .sf2 / .sf3".to_string()
            } else {
                format!("没有可导入的音色库：{}", skipped.join("；"))
            });
        }
        Ok(StoreInstall {
            slug: bank.slug.clone(),
            name: bank.name.clone(),
            imported,
            skipped,
        })
    })();

    let _ = fs::remove_dir_all(&work);
    result
}

/// 本机已装的商店 slug。
fn installed_slugs() -> std::collections::HashSet<String> {
    crate::soundfonts::catalog()
        .into_iter()
        .filter_map(|entry| entry.source)
        .collect()
}

fn to_match(bank: &StoreBank, installed: &std::collections::HashSet<String>) -> StoreMatch {
    let (file, blocked) = choose_file(bank);
    StoreMatch {
        slug: bank.slug.clone(),
        name: bank.name.clone(),
        vendor: bank.vendor.clone(),
        description: bank.description.clone(),
        license: bank.license.clone(),
        license_url: bank.license_url.clone(),
        homepage: bank.homepage.clone(),
        formats: bank.formats.clone(),
        kind: bank.kind.clone(),
        presets: bank.presets,
        tags: bank.tags.clone(),
        installed: installed.contains(&bank.slug),
        installable: file.is_some(),
        blocked,
        size: file.and_then(|f| f.size),
        format: file.map(|f| f.format.clone()),
        archive: file.and_then(|f| f.archive.clone()),
    }
}

/// 从一条索引里挑一个「本机真能一键装」的文件。
///
/// 排序：文件直链优先于压缩包；同类型里 SF3 优先（体积小）；再按体积从小到大。
/// 返回 `(Some(file), _)` 表示可装；`(None, Some(reason))` 说明为什么不行。
fn choose_file(bank: &StoreBank) -> (Option<&StoreFile>, Option<String>) {
    let mut external = false;
    let mut unsupported: Vec<String> = Vec::new();
    let mut candidates: Vec<&StoreFile> = Vec::new();

    for file in &bank.files {
        match file.install.as_str() {
            "file" => candidates.push(file),
            "archive" => match file.archive.as_deref() {
                Some(kind) if manager::archive_supported(kind) => candidates.push(file),
                Some(kind) => unsupported.push(kind.to_string()),
                None => unsupported.push("未知归档".into()),
            },
            _ => external = true,
        }
    }

    candidates.sort_by_key(|file| {
        let kind = if file.install == "file" { 0 } else { 1 };
        let format = if file.format == "sf3" { 0 } else { 1 };
        (kind, format, file.size.unwrap_or(u64::MAX))
    });

    if let Some(file) = candidates.first() {
        return (Some(*file), None);
    }
    let reason = if external {
        "只提供下载页，请在浏览器打开后再导入".to_string()
    } else if !unsupported.is_empty() {
        format!(
            "压缩包是 {} 格式，客户端还不支持自动解压；请在浏览器打开后手动导入",
            unsupported.join(" / ")
        )
    } else {
        "索引里没有可下载的文件".to_string()
    };
    (None, Some(reason))
}

/// 收集目录里的 `.sf2` / `.sf3`（大的优先；Sonatina 那种全集只取前 N 个）。
fn collect_banks(root: &Path) -> (Vec<PathBuf>, Vec<String>) {
    let mut found: Vec<PathBuf> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    collect_banks_into(root, &mut found, &mut skipped);
    found.sort_by_key(|path| {
        std::cmp::Reverse(fs::metadata(path).map(|meta| meta.len()).unwrap_or(0))
    });
    if found.len() > MAX_BANKS_PER_ARCHIVE {
        for extra in found.split_off(MAX_BANKS_PER_ARCHIVE) {
            skipped.push(format!(
                "{}（超过单次 {} 个的上限）",
                extra.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                MAX_BANKS_PER_ARCHIVE
            ));
        }
    }
    (found, skipped)
}

fn collect_banks_into(dir: &Path, found: &mut Vec<PathBuf>, skipped: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_banks_into(&path, found, skipped);
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match ext.as_str() {
            "sf2" | "sf3" => found.push(path),
            // 包里常见的读我/授权文件不算失败，其余音频资产说明它没被用上。
            "sfz" => skipped.push(format!(
                "{}（SFZ 暂不支持）",
                path.file_name().and_then(|n| n.to_str()).unwrap_or("?")
            )),
            _ => {}
        }
    }
}

fn score_bank(bank: &StoreBank, needle: &str) -> Option<i64> {
    let name = bank.name.to_lowercase();
    let slug = bank.slug.to_lowercase();
    let mut score = 0;
    if name == needle || slug == needle {
        score += 100;
    }
    if name.contains(needle) {
        score += 40;
    }
    if slug.contains(needle) {
        score += 20;
    }
    if bank.vendor.to_lowercase().contains(needle) {
        score += 15;
    }
    if bank.tags.iter().any(|tag| tag.to_lowercase().contains(needle)) {
        score += 10;
    }
    if bank.description.to_lowercase().contains(needle) {
        score += 5;
    }
    (score > 0).then_some(score)
}

fn parse_registry(text: &str) -> Result<Vec<StoreBank>, String> {
    let registry: Registry =
        serde_json::from_str(text).map_err(|e| format!("索引 JSON 无法解析：{e}"))?;
    if registry.schema_version != 1 {
        return Err(format!(
            "索引版本 {} 不认识（客户端只认 1）",
            registry.schema_version
        ));
    }
    Ok(registry.soundfonts)
}

fn fetch(url: &str, proxy: Option<&ureq::Proxy>) -> Result<String, String> {
    let agent = manager::tls_agent(
        proxy,
        Some(Duration::from_secs(3)),
        Some(Duration::from_secs(6)),
        Some(Duration::from_secs(8)),
        Some(Duration::from_secs(20)),
    );
    let mut resp = agent.get(url).call().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    let text = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    Ok(text)
}

fn disk_cache_path(app: &crate::tauri_api::AppHandle) -> Option<PathBuf> {
    crate::paths::data_dir(app)
        .ok()
        .map(|dir| dir.join("soundfont-registry.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "schema_version": 1,
      "soundfonts": [
        {
          "slug": "musescore/general",
          "name": "MuseScore General",
          "vendor": "MuseScore",
          "description": "GM bank",
          "license": "MIT",
          "formats": ["sf2", "sf3"],
          "kind": "gm",
          "presets": 128,
          "tags": ["gm"],
          "files": [
            {"url": "https://example.com/a.sf3", "format": "sf3", "install": "file", "size": 39900972},
            {"url": "https://example.com/a.sf2", "format": "sf2", "install": "file", "size": 215614036}
          ]
        },
        {
          "slug": "freepats/upright-piano-kw",
          "name": "Upright Piano KW",
          "license": "CC0-1.0",
          "formats": ["sf2"],
          "files": [
            {"url": "https://example.com/p.7z", "format": "sf2", "install": "archive", "archive": "7z", "size": 6093984}
          ]
        },
        {
          "slug": "generaluser/gs",
          "name": "GeneralUser GS",
          "license": "LicenseRef-generaluser-gs",
          "formats": ["sf2"],
          "files": [
            {"url": "https://example.com/page", "format": "sf2", "install": "external"}
          ]
        }
      ]
    }"#;

    #[test]
    fn parses_and_ranks_downloads() {
        let banks = parse_registry(SAMPLE).expect("parse");
        assert_eq!(banks.len(), 3);

        // 直链 + SF3 优先，且体积小。
        let (file, blocked) = choose_file(&banks[0]);
        assert!(blocked.is_none());
        let file = file.expect("installable");
        assert_eq!(file.format, "sf3");

        // 7z 目前不支持：给出可读原因，而不是「不可用」。
        let (file, blocked) = choose_file(&banks[1]);
        assert!(file.is_none());
        assert!(blocked.unwrap().contains("7z"));

        // 纯外链同样不能一键装。
        let (file, blocked) = choose_file(&banks[2]);
        assert!(file.is_none());
        assert!(blocked.unwrap().contains("下载页"));
    }

    #[test]
    fn search_scores_name_over_description() {
        let banks = parse_registry(SAMPLE).expect("parse");
        assert!(score_bank(&banks[0], "musescore").unwrap() > 0);
        assert!(score_bank(&banks[0], "zzz").is_none());
        // 大小写不敏感。
        assert!(score_bank(&banks[0], "general").unwrap() > 0);
    }

    /// 端到端：下载 → 校验 sha256 → 解压 zip → 导入托管目录。
    ///
    /// 起一个只回一次请求的本地 HTTP 服务，避免测试依赖外网（索引注入走 override 槽）。
    #[test]
    fn install_downloads_extracts_and_imports() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        use sha2::{Digest, Sha256};

        let _guard = crate::soundfonts::test_lock();

        // 1. 造一个只含一个真实 SF3 的 zip。
        let sf3 = include_bytes!("../../instruments/assets/piano.sf3");
        let mut zip_bytes = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut zip_bytes));
            writer
                .start_file("Piano.sf3", zip::write::SimpleFileOptions::default())
                .expect("zip entry");
            writer.write_all(sf3).expect("zip body");
            writer.finish().expect("zip finish");
        }
        let sha256 = format!("{:x}", Sha256::digest(&zip_bytes));
        let size = zip_bytes.len() as u64;

        // 2. 本地服务：只服务一次 GET。
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let body = zip_bytes.clone();
        let server = std::thread::spawn(move || {
            if let Ok((mut socket, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/zip\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes());
                let _ = socket.write_all(&body);
            }
        });

        // 3. 索引注入 + 托管目录。
        let json = format!(
            r#"{{"schema_version":1,"soundfonts":[{{"slug":"test/bank","name":"Test Bank","license":"CC0-1.0","formats":["sf3"],
                "files":[{{"url":"http://127.0.0.1:{port}/bank.zip","format":"sf3","install":"archive","archive":"zip","size":{size},"sha256":"{sha256}"}}]}}]}}"#
        );
        let banks = parse_registry(&json).expect("fixture parses");
        assert!(choose_file(&banks[0]).0.is_some(), "fixture 应当可装");

        let dir = std::env::temp_dir().join(format!("ai-daw-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        crate::soundfonts::init(dir.clone());

        let outcome = install_into(&dir, &banks[0], None).expect("install");
        assert_eq!(outcome.imported.len(), 1, "zip 里一个 sf3 应导入一条");
        assert_eq!(outcome.slug, "test/bank");
        let entry = &outcome.imported[0];
        assert!(std::path::Path::new(&entry.file).is_file(), "文件要落到托管目录");
        assert_eq!(entry.source.as_deref(), Some("test/bank"), "来源要记成 slug");
        assert!(!entry.presets.is_empty(), "预设要解析出来");
        assert_eq!(crate::soundfonts::catalog().len(), 1, "清单里要能看到它");

        server.join().expect("server thread");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 下载内容与索引里的 sha256 不符时必须拒绝，且不留半成品。
    #[test]
    fn install_rejects_a_hash_mismatch() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let _guard = crate::soundfonts::test_lock();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = std::thread::spawn(move || {
            if let Ok((mut socket, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf);
                let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope");
            }
        });

        let dir = std::env::temp_dir().join(format!("ai-daw-store-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        crate::soundfonts::init(dir.clone());

        let json = format!(
            r#"{{"schema_version":1,"soundfonts":[{{"slug":"test/bad","name":"Bad","license":"CC0-1.0","formats":["sf3"],
                "files":[{{"url":"http://127.0.0.1:{port}/bank.sf3","format":"sf3","install":"file","size":4,"sha256":"{}"}}]}}]}}"#,
            "0".repeat(64)
        );
        let banks = parse_registry(&json).expect("fixture parses");
        let err = install_into(&dir, &banks[0], None).unwrap_err();
        assert!(err.contains("sha256") || err.contains("校验"), "错误要说明校验失败：{err}");
        assert!(crate::soundfonts::catalog().is_empty(), "失败不该留下条目");
        server.join().expect("server thread");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bundled_snapshot_parses_and_has_installable_banks() {
        let banks = parse_registry(BUNDLED).expect("随包索引必须可解析");
        assert!(banks.len() >= 5, "随包索引太少：{}", banks.len());
        assert!(banks.iter().any(|bank| bank.slug == "musescore/general"));
        assert!(
            banks.iter().any(|bank| choose_file(bank).0.is_some()),
            "至少要有一条能一键安装，否则商店是空壳"
        );
    }

    #[test]
    fn search_matches_names_and_reports_installability() {
        let banks = parse_registry(SAMPLE).expect("parse");
        let installed = std::collections::HashSet::new();
        let found = search_in(&banks, &installed, "piano");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].slug, "freepats/upright-piano-kw");
        assert!(!found[0].installable, "7z 还不能一键装");
        assert!(found[0].blocked.is_some());

        // 空关键词 = 全部，带 installed 标记。
        let mut installed = std::collections::HashSet::new();
        installed.insert("generaluser/gs".to_string());
        let all = search_in(&banks, &installed, "");
        assert_eq!(all.len(), 3);
        assert!(all.iter().find(|b| b.slug == "generaluser/gs").unwrap().installed);
        assert!(all.iter().find(|b| b.slug == "musescore/general").unwrap().installable);
    }
}
