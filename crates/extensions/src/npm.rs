//! npm registry 客户端：版本解析、tarball 下载、完整性校验、解包。
//!
//! 只做「装一个精确版本」这一件事，不做依赖解析——扩展包必须自包含（设计 §2 非目标）。
//! 完整性优先用 `dist.integrity`（sha512 base64），退而求其次用旧的 `dist.shasum`（sha1）；
//! 两者都没有就拒绝安装。

use std::fmt::Write as _;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use serde_json::Value;
use sha2::{Digest, Sha256, Sha512};

use crate::error::{ExtError, ExtResult};

/// 单个 tarball 的下载上限（解压前）。
pub const MAX_TARBALL_BYTES: u64 = 64 * 1024 * 1024;
/// 解包后的总字节上限。
pub const MAX_UNPACKED_BYTES: u64 = 256 * 1024 * 1024;
/// 解包后的文件数上限。
pub const MAX_FILES: usize = 20_000;

/// 要装的版本。
#[derive(Debug, Clone)]
pub enum VersionRequest {
    /// 精确版本。
    Exact(String),
    /// 最新兼容版本（按 `engines.aidaw` 过滤，优先稳定版）。
    Latest,
}

/// 解析好的一个可下载版本。
#[derive(Debug, Clone)]
pub struct Resolved {
    /// 包名。
    pub name: String,
    /// 版本。
    pub version: String,
    /// tarball 地址。
    pub tarball: String,
    /// `dist.integrity`（`sha512-...`）。
    pub integrity: Option<String>,
    /// `dist.shasum`（sha1 hex）。
    pub shasum: Option<String>,
    /// 实际用了哪个 registry。
    pub registry: String,
}

impl Resolved {
    /// 有没有可用于校验的摘要。
    pub fn has_digest(&self) -> bool {
        self.integrity.is_some() || self.shasum.is_some()
    }
}

/// npm 客户端。
pub struct NpmClient {
    agent: ureq::Agent,
    /// 依次尝试的 registry。
    registries: Vec<String>,
}

impl NpmClient {
    /// 用一组 registry 建客户端（第一个是主源，其余是镜像）。
    pub fn new(registries: Vec<String>) -> Self {
        Self {
            agent: agent(),
            registries: if registries.is_empty() {
                vec!["https://registry.npmjs.org".to_string()]
            } else {
                registries
            },
        }
    }

    /// registry 列表。
    pub fn registries(&self) -> &[String] {
        &self.registries
    }

    /// 拉一个包的 registry 文档（`GET <registry>/<name>`）。
    pub fn document(&self, name: &str) -> ExtResult<(Value, String)> {
        let encoded = encode_package_name(name);
        let mut last_error = String::new();
        for registry in &self.registries {
            let url = format!("{registry}/{encoded}");
            match self.get_json(&url) {
                Ok(value) => return Ok((value, registry.clone())),
                Err(err) => last_error = err.to_string(),
            }
        }
        Err(ExtError::Network(format!("所有 registry 都拿不到「{name}」：{last_error}")))
    }

    /// 解析版本（含兼容性过滤）。
    pub fn resolve(&self, name: &str, request: &VersionRequest, app_version: &str) -> ExtResult<Resolved> {
        let (document, registry) = self.document(name)?;
        let versions = document
            .get("versions")
            .and_then(Value::as_object)
            .ok_or_else(|| ExtError::Network(format!("「{name}」的 registry 文档里没有 versions")))?;

        let chosen: Option<(String, &Value)> = match request {
            VersionRequest::Exact(version) => versions
                .get(version)
                .map(|meta| (version.clone(), meta))
                .or_else(|| {
                    // `name@1.2` 这类不完整版本：退化成前缀匹配里最高的一个。
                    let mut candidates: Vec<(&String, &Value)> = versions
                        .iter()
                        .filter(|(candidate, _)| candidate.starts_with(&format!("{version}.")))
                        .collect();
                    candidates.sort_by(|a, b| compare_versions(a.0, b.0));
                    candidates.last().map(|(candidate, meta)| ((*candidate).clone(), *meta))
                }),
            VersionRequest::Latest => {
                let mut candidates: Vec<(&String, &Value)> = versions
                    .iter()
                    .filter(|(version, meta)| {
                        compatible(meta, app_version) && semver::Version::parse(version).is_ok()
                    })
                    .collect();
                candidates.sort_by(|a, b| compare_versions(a.0, b.0));
                // 优先稳定版：最高稳定版排在最高预发布版之前。
                let stable = candidates
                    .iter()
                    .rev()
                    .find(|(version, _)| semver::Prerelease::EMPTY == semver::Version::parse(version).unwrap().pre);
                stable
                    .or_else(|| candidates.last())
                    .map(|(version, meta)| ((*version).clone(), *meta))
            }
        };

        let (version, meta) = chosen.ok_or_else(|| {
            ExtError::Install(format!("「{name}」没有满足要求的版本（请求：{request:?}，当前 AI DAW {app_version}）"))
        })?;

        let dist = meta
            .get("dist")
            .and_then(Value::as_object)
            .ok_or_else(|| ExtError::Install(format!("「{name}@{version}」缺少 dist 信息")))?;
        let tarball = dist
            .get("tarball")
            .and_then(Value::as_str)
            .ok_or_else(|| ExtError::Install(format!("「{name}@{version}」缺少 dist.tarball")))?
            .to_string();
        let integrity = dist.get("integrity").and_then(Value::as_str).map(str::to_string);
        let shasum = dist.get("shasum").and_then(Value::as_str).map(str::to_string);

        Ok(Resolved {
            name: name.to_string(),
            version,
            tarball,
            integrity,
            shasum,
            registry,
        })
    }

    /// 下载 tarball 到 `dest`，边下边校验大小，下完校验摘要。
    pub fn download(&self, resolved: &Resolved, dest: &Path) -> ExtResult<()> {
        if !resolved.has_digest() {
            return Err(ExtError::Install(format!(
                "「{}@{}」的 registry 既没有 integrity 也没有 shasum，拒绝安装（无法校验完整性）",
                resolved.name, resolved.version
            )));
        }
        let response = self
            .agent
            .get(&resolved.tarball)
            .call()
            .map_err(|err| ExtError::Network(format!("下载 {} 失败：{err}", resolved.tarball)))?;
        if response.status().as_u16() != 200 {
            return Err(ExtError::Network(format!(
                "下载 {} 返回 HTTP {}",
                resolved.tarball,
                response.status()
            )));
        }
        let mut reader = response.into_body().into_reader();
        let mut bytes = Vec::new();
        reader
            .by_ref()
            .take(MAX_TARBALL_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|err| ExtError::Network(format!("读取 tarball 失败：{err}")))?;
        if bytes.len() as u64 > MAX_TARBALL_BYTES {
            return Err(ExtError::Install(format!(
                "tarball 超过 {} MiB 上限",
                MAX_TARBALL_BYTES / 1024 / 1024
            )));
        }
        verify_digest(&bytes, resolved.integrity.as_deref(), resolved.shasum.as_deref())?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(dest, bytes)?;
        Ok(())
    }

    fn get_json(&self, url: &str) -> ExtResult<Value> {
        let response = self
            .agent
            .get(url)
            .header("accept", "application/json")
            .call()
            .map_err(|err| ExtError::Network(format!("GET {url} 失败：{err}")))?;
        if response.status().as_u16() != 200 {
            return Err(ExtError::Network(format!("GET {url} 返回 HTTP {}", response.status())));
        }
        let mut text = String::new();
        response
            .into_body()
            .into_reader()
            .take(8 * 1024 * 1024)
            .read_to_string(&mut text)
            .map_err(|err| ExtError::Network(format!("读取 {url} 失败：{err}")))?;
        serde_json::from_str(&text).map_err(|err| ExtError::Network(format!("{url} 返回的不是 JSON：{err}")))
    }
}

/// 校验 tarball 的完整性。
pub fn verify_digest(bytes: &[u8], integrity: Option<&str>, shasum: Option<&str>) -> ExtResult<()> {
    if let Some(integrity) = integrity {
        for token in integrity.split_whitespace() {
            let Some((algorithm, encoded)) = token.split_once('-') else {
                continue;
            };
            match algorithm {
                "sha512" => {
                    let expected = base64::engine::general_purpose::STANDARD
                        .decode(encoded)
                        .map_err(|err| ExtError::Install(format!("integrity 不是合法 base64：{err}")))?;
                    let actual = Sha512::digest(bytes);
                    if actual.as_slice() == expected.as_slice() {
                        return Ok(());
                    }
                    return Err(ExtError::Install("sha512 校验失败：下载内容被改过".into()));
                }
                "sha256" | "sha384" | "sha1" => {
                    // 只认 sha512；其它算法退到 shasum。
                }
                _ => {}
            }
        }
    }
    if let Some(shasum) = shasum {
        let mut hasher = Sha1::new();
        hasher.update(bytes);
        let actual = hasher.finalize();
        let mut hex = String::with_capacity(40);
        for byte in actual {
            let _ = write!(hex, "{byte:02x}");
        }
        if hex.eq_ignore_ascii_case(shasum.trim()) {
            return Ok(());
        }
        return Err(ExtError::Install("sha1 校验失败：下载内容被改过".into()));
    }
    Err(ExtError::Install("没有可用的完整性摘要".into()))
}

use sha1::Sha1;

/// 解 npm tarball 到 `dest`（剥掉顶层 `package/`，挡住路径穿越）。
pub fn extract_tarball(tar_path: &Path, dest: &Path) -> ExtResult<()> {
    let file = std::fs::File::open(tar_path)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    std::fs::create_dir_all(dest)?;

    let mut unpacked: u64 = 0;
    let mut files = 0usize;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        let mut components = path.components();
        // npm tarball 一律以 `package/` 开头；不是就拒绝（说明这不是标准 npm 包）。
        match components.next() {
            Some(std::path::Component::Normal(first)) if first == "package" => {}
            _ => {
                return Err(ExtError::Install(format!(
                    "tarball 里没有 `package/` 顶层目录：{}",
                    path.display()
                )));
            }
        }
        let relative: PathBuf = components.collect();
        if relative.as_os_str().is_empty() {
            continue;
        }
        if relative.is_absolute() || relative.components().any(|part| matches!(part, std::path::Component::ParentDir)) {
            return Err(ExtError::Install(format!("tarball 里有非法路径：{}", path.display())));
        }
        let target = dest.join(&relative);
        if !target.starts_with(dest) {
            return Err(ExtError::Install(format!("tarball 里的路径逃出了目标目录：{}", path.display())));
        }

        let header = entry.header().clone();
        match header.entry_type() {
            tar::EntryType::Directory => {
                std::fs::create_dir_all(&target)?;
            }
            tar::EntryType::Regular | tar::EntryType::Continuous => {
                unpacked += entry.size();
                files += 1;
                if unpacked > MAX_UNPACKED_BYTES {
                    return Err(ExtError::Install("解包后超过上限".into()));
                }
                if files > MAX_FILES {
                    return Err(ExtError::Install("文件数超过上限".into()));
                }
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut buffer = Vec::with_capacity(entry.size() as usize);
                entry.read_to_end(&mut buffer)?;
                std::fs::write(&target, buffer)?;
            }
            // 软链 / 硬链 / 设备文件：一律跳过（npm 包不需要，且是攻击面）。
            _ => {}
        }
    }
    Ok(())
}

fn compatible(meta: &Value, app_version: &str) -> bool {
    let declared = meta
        .get("engines")
        .and_then(Value::as_object)
        .and_then(|engines| engines.get("aidaw"))
        .and_then(Value::as_str);
    crate::manifest::check_engine(declared, app_version).is_ok()
}

fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    match (semver::Version::parse(a), semver::Version::parse(b)) {
        (Ok(a), Ok(b)) => a.cmp(&b),
        _ => a.cmp(b),
    }
}

/// 包名 → registry 路径（`@scope/name` 保留 `/`）。
fn encode_package_name(name: &str) -> String {
    name.replace('/', "%2F")
}

fn agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::NativeTls)
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(20)))
        .timeout_global(Some(Duration::from_secs(600)))
        // 4xx/5xx 自己判，别让 ureq 当成错误抛出来（要读 body 里的错误）。
        .http_status_as_error(false)
        .user_agent("AI-DAW-Extensions/0.1")
        .build()
        .into()
}

#[allow(dead_code)]
fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let mut hex = String::with_capacity(64);
    for byte in hasher.finalize() {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn extracts_a_npm_shaped_tarball() {
        let dir = tempfile::tempdir().unwrap();
        let tar_path = dir.path().join("pkg.tgz");
        {
            let file = std::fs::File::create(&tar_path).unwrap();
            let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
            let mut builder = tar::Builder::new(encoder);
            let payload = br#"{"name":"@aidaw/x","version":"1.0.0"}"#;
            let mut header = tar::Header::new_gnu();
            header.set_size(payload.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "package/package.json", &payload[..])
                .unwrap();
            builder.finish().unwrap();
        }
        let dest = dir.path().join("out");
        extract_tarball(&tar_path, &dest).unwrap();
        assert!(dest.join("package.json").is_file());
    }

    #[test]
    fn rejects_tarballs_without_package_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let tar_path = dir.path().join("bad.tgz");
        {
            let file = std::fs::File::create(&tar_path).unwrap();
            let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
            let mut builder = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(3);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, "evil.js", &b"bad"[..]).unwrap();
            builder.finish().unwrap();
        }
        let err = extract_tarball(&tar_path, &dir.path().join("out")).unwrap_err();
        assert!(err.to_string().contains("package/"), "{err}");
    }

    #[test]
    fn rejects_entries_that_are_not_under_package() {
        let dir = tempfile::tempdir().unwrap();
        let tar_path = dir.path().join("bad2.tgz");
        {
            let file = std::fs::File::create(&tar_path).unwrap();
            let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
            let mut builder = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(3);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, "evil/escape.txt", &b"bad"[..]).unwrap();
            builder.finish().unwrap();
        }
        assert!(extract_tarball(&tar_path, &dir.path().join("out")).is_err());
    }

    #[test]
    fn integrity_and_shasum_are_checked() {
        let bytes = b"hello";
        let digest = Sha512::digest(bytes);
        let encoded = base64::engine::general_purpose::STANDARD.encode(digest);
        assert!(verify_digest(bytes, Some(&format!("sha512-{encoded}")), None).is_ok());
        assert!(verify_digest(bytes, Some("sha512-AAAA"), None).is_err());
        assert!(verify_digest(b"tampered", Some(&format!("sha512-{encoded}")), None).is_err());

        let sha1 = {
            let mut hasher = Sha1::new();
            hasher.update(bytes);
            let mut hex = String::new();
            for byte in hasher.finalize() {
                let _ = write!(hex, "{byte:02x}");
            }
            hex
        };
        assert!(verify_digest(bytes, None, Some(&sha1)).is_ok());
        assert!(verify_digest(bytes, None, None).is_err());
    }

    #[test]
    fn encode_keeps_scope_slash_encoded() {
        assert_eq!(encode_package_name("@aidaw/x"), "@aidaw%2Fx");
        assert_eq!(encode_package_name("plain"), "plain");
    }

    #[test]
    fn sha256_helper_is_stable() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn tar_builder_smoke() {
        // 保证测试里的写路径真的用到了 Write trait（避免 import 被优化掉）。
        let mut buffer = Vec::new();
        buffer.write_all(b"x").unwrap();
        assert_eq!(buffer.len(), 1);
    }
}

#[cfg(test)]
mod network_tests {
    use super::*;

    /// 真网络：解析 → 下载 → 校验 integrity → 解包。
    ///
    /// 默认 `#[ignore]`：CI 与离线开发不该依赖 npm registry。手动跑：
    /// `cargo test -p ai-daw-extensions -- --ignored real_registry`
    #[test]
    #[ignore = "需要网络：访问 registry.npmjs.org"]
    fn real_registry_resolve_download_and_extract() {
        let dir = tempfile::tempdir().unwrap();
        let client = NpmClient::new(vec!["https://registry.npmjs.org".into()]);
        let resolved = client
            .resolve("is-number", &VersionRequest::Exact("7.0.0".into()), "1.9.0")
            .unwrap();
        assert_eq!(resolved.version, "7.0.0");
        assert!(resolved.has_digest(), "npm 应该给出 integrity 或 shasum");

        let tarball = dir.path().join("package.tgz");
        client.download(&resolved, &tarball).unwrap();
        let out = dir.path().join("out");
        extract_tarball(&tarball, &out).unwrap();
        let package: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(out.join("package.json")).unwrap()).unwrap();
        assert_eq!(package["name"], "is-number");
    }

    /// 真网络：主源不可达时按顺序回退到镜像（国内 npm 镜像）。
    #[test]
    #[ignore = "需要网络：访问 registry.npmmirror.com"]
    fn real_registry_falls_back_to_a_mirror() {
        let client = NpmClient::new(vec![
            "https://registry.invalid".to_string(),
            "https://registry.npmmirror.com".to_string(),
        ]);
        let (document, registry) = client.document("is-number").unwrap();
        assert!(document.get("versions").is_some(), "镜像应返回 versions");
        assert!(registry.contains("npmmirror"), "实际命中的源：{registry}");
    }
}
