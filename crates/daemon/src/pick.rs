//! **只从远端 zip 里取清单列出的那几个文件**：HTTP `Range` + 中央目录 + 逐文件校验。
//!
//! 有些上游只发整包：ONNX Runtime 的官方 release 是个完整 SDK（解压 417 MB，我们只要两个
//! dll）。整包下下来再扔掉 90% 是浪费，所以这里直接按 zip 结构抓需要的那几段字节：
//!
//! 1. `Range: bytes=0-0` 拿总长（不用 HEAD，GitHub 的资产对 HEAD 会卡）；
//! 2. 抓文件尾 200 KB 找 EOCD → 中央目录偏移 / 长度；
//! 3. 抓中央目录，解析出每个条目的压缩方式 / 压缩长度 / 本地头偏移；
//! 4. 逐条目：抓本地头（name/extra 长度可能和中央目录不同）→ 抓压缩数据 → 解压 → 校验。
//!
//! **校验**：给了 `sha256` 就逐文件校验，没给就只对解压后的字节数——
//! ORT 的那几个文件目前只按大小 + HTTPS。
//!
//! 拿不到 `Range`（服务器回 200）时直接报错，由调用方退回整包下载。

use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// 要从归档里取出的一条。
pub struct PickTarget<'a> {
    /// 归档内路径（zip 里的名字，用 `/` 分隔）。
    pub inner: &'a str,
    /// 落地路径（相对 `dir`；可带子目录）。
    pub dest: &'a str,
    /// 期望的 sha256（小写十六进制）；`None` = 只校验大小。
    pub sha256: Option<&'a str>,
}

/// 一次「只取所需」的结果。
pub struct PickOutcome {
    /// 实际传输的字节（压缩后）。
    pub transferred: u64,
}

/// 中央目录里的一条。
struct ZipEntry {
    name: String,
    method: u16,
    compressed: u64,
    uncompressed: u64,
    offset: u64,
}

/// 用 Range 只抓 `targets` 列出的文件，落到 `dir` 下。
///
/// `progress` 收到的是**已传输字节 / 目标传输量**，由调用方决定怎么显示。
pub fn pick(
    agent: &ureq::Agent,
    url: &str,
    targets: &[PickTarget],
    plan: u64,
    dir: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<PickOutcome, String> {
    let total = remote_size(agent, url)?;

    // 中央目录在文件尾部：先取最后 200 KB 找 EOCD。
    let tail_len = total.min(200_000);
    let tail = http_range(agent, url, total - tail_len, total - 1)?;
    let eocd = find_eocd(&tail)?;
    let cd_size = u32_at(&tail, eocd + 12)? as u64;
    let cd_off = u32_at(&tail, eocd + 16)? as u64;
    if cd_off == 0xFFFF_FFFF || cd_size == 0xFFFF_FFFF {
        return Err("zip64，跳过".into());
    }
    let cd = http_range(agent, url, cd_off, cd_off + cd_size - 1)?;
    let entries = parse_directory(&cd)?;

    let mut received = 0u64;
    for target in targets {
        if cancel.load(Ordering::Relaxed) {
            return Err("已取消".into());
        }
        let entry = entries
            .iter()
            .find(|e| e.name == target.inner)
            .ok_or_else(|| format!("归档里没有 {}", target.inner))?;
        // 本地头的 extra 长度可能和中央目录不同，必须读一次。
        let header = http_range(agent, url, entry.offset, entry.offset + 29)?;
        if header.get(0..4) != Some(b"PK\x03\x04") {
            return Err(format!("{} 的本地头不合法", target.inner));
        }
        let name_len = u16_at(&header, 26)? as u64;
        let extra_len = u16_at(&header, 28)? as u64;
        let start = entry.offset + 30 + name_len + extra_len;
        let data = http_range(agent, url, start, start + entry.compressed - 1)?;
        let bytes = match entry.method {
            0 => data,
            8 => inflate(&data)?,
            other => return Err(format!("不支持的压缩方式 {other}")),
        };
        if bytes.len() as u64 != entry.uncompressed {
            return Err(format!(
                "{} 解压后 {} 字节，期望 {}",
                target.inner,
                bytes.len(),
                entry.uncompressed
            ));
        }
        if let Some(expected) = target.sha256 {
            let actual = sha256_hex(&bytes);
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(format!(
                    "{} 的 sha256 对不上（拿到 {actual}，清单 {expected}）",
                    target.inner
                ));
            }
        }
        write_atomic(&dir.join(target.dest), &bytes)?;
        received += entry.compressed;
        progress(received.min(plan), plan);
    }
    Ok(PickOutcome { transferred: received })
}

/// 只取所需文件时用的 HTTP agent。
///
/// 探测那套 3–6 秒超时会把几十 MB 的 `Range` 读超时掉；这里给的是「慢但大」的配置：
/// 连接 / 首个响应各 30 秒，读 120 秒，不设全局超时（一个 100 MB 的文件本来就要传一阵）。
pub fn agent(proxy: Option<&ureq::Proxy>) -> ureq::Agent {
    crate::manager::tls_agent(
        proxy,
        Some(std::time::Duration::from_secs(30)),
        Some(std::time::Duration::from_secs(30)),
        Some(std::time::Duration::from_secs(120)),
        None,
    )
}

/// 远端的文件大小：用 `Range: bytes=0-0` 的 `Content-Range` 拿。
///
/// 不用 HEAD：GitHub 的 release 资产对 HEAD 会走一条更慢 / 更不稳定的重定向链，
/// 实测在 ureq 下会卡到超时；GET + Range 反而稳。
pub fn remote_size(agent: &ureq::Agent, url: &str) -> Result<u64, String> {
    let resp = agent
        .get(url)
        .header("Range", "bytes=0-0")
        .call()
        .map_err(|e| format!("探测大小失败：{e}"))?;
    resp.headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('/').next())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .ok_or_else(|| "服务器没有给出文件大小".to_string())
}

fn http_range(
    agent: &ureq::Agent,
    url: &str,
    start: u64,
    end: u64,
) -> Result<Vec<u8>, String> {
    let resp = agent
        .get(url)
        .header("Range", &format!("bytes={start}-{end}"))
        .call()
        .map_err(|e| format!("Range 请求失败：{e}"))?;
    if resp.status().as_u16() != 206 {
        // 服务器忽略 Range（返回 200 整包）：这条路走不通，交给整包下载。
        return Err(format!("服务器不支持 Range（HTTP {}）", resp.status()));
    }
    let mut out = Vec::with_capacity((end - start + 1) as usize);
    resp.into_body()
        .into_reader()
        .read_to_end(&mut out)
        .map_err(|e| format!("读取响应失败：{e}"))?;
    Ok(out)
}

fn find_eocd(tail: &[u8]) -> Result<usize, String> {
    tail.windows(4)
        .rposition(|w| w == b"PK\x05\x06")
        .ok_or_else(|| "没找到 zip 目录（EOCD）".to_string())
}

fn parse_directory(cd: &[u8]) -> Result<Vec<ZipEntry>, String> {
    let mut out = Vec::new();
    let mut p = 0usize;
    while p + 46 <= cd.len() && cd.get(p..p + 4) == Some(b"PK\x01\x02") {
        let method = u16_at(cd, p + 10)?;
        let compressed = u32_at(cd, p + 20)? as u64;
        let uncompressed = u32_at(cd, p + 24)? as u64;
        let name_len = u16_at(cd, p + 28)? as usize;
        let extra_len = u16_at(cd, p + 30)? as usize;
        let comment_len = u16_at(cd, p + 32)? as usize;
        let offset = u32_at(cd, p + 42)? as u64;
        let name = cd
            .get(p + 46..p + 46 + name_len)
            .ok_or_else(|| "中央目录被截断".to_string())?;
        out.push(ZipEntry {
            // zip 规范要求 `/`，但 Windows 上打的包常常写 `\`，
            // 统一成 `/` 再和清单比对。
            name: String::from_utf8_lossy(name).replace('\\', "/"),
            method,
            compressed,
            uncompressed,
            offset,
        });
        p += 46 + name_len + extra_len + comment_len;
    }
    if out.is_empty() {
        return Err("中央目录为空".into());
    }
    Ok(out)
}

fn u16_at(buf: &[u8], at: usize) -> Result<u16, String> {
    buf.get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| "zip 结构被截断".to_string())
}

fn u32_at(buf: &[u8], at: usize) -> Result<u32, String> {
    buf.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| "zip 结构被截断".to_string())
}

fn inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    flate2::read::DeflateDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|e| format!("解压失败：{e}"))?;
    Ok(out)
}

/// 先写 `.part` 再改名：中途断掉不会留下一个「看起来装好了」的半文件。
fn write_atomic(dest: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = dest.with_extension("part");
    std::fs::write(&tmp, bytes).map_err(|e| format!("写入 {} 失败：{e}", dest.display()))?;
    std::fs::rename(&tmp, dest).map_err(|e| format!("落地 {} 失败：{e}", dest.display()))?;
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_digest() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn local_headers_are_short_enough_to_read_in_one_go() {
        // 固定读 30 字节头（签名 4 + 到 extra 长度 28），少一字节就解析不了。
        assert_eq!(u16_at(&[0xaa, 0xbb], 0).unwrap(), 0xbbaa);
        assert!(u16_at(&[0xaa], 0).is_err());
    }
}
