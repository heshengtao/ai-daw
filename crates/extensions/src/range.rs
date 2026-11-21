//! 版本区间：把 **npm 风格**的 range 翻译成 `semver` crate 能吃的形式。
//!
//! 为什么需要它：市场的 `engines.aidaw` 是照 npm 写的（`">=1.9.0 <2.0.0"`、`"^1.9 || ^2"`
//! 这种写法到处都是），而 `semver::VersionReq` 只认逗号分隔、**不支持 `||`**。自己写一个
//! node-semver 不现实，但把常见写法翻译过来很便宜：
//!
//! - `a b` → `a, b`（空格分隔的比较子）；
//! - `>= 1.9.0` → `>=1.9.0`（操作符与版本号之间的空格）；
//! - `1.2.3 - 2.0.0` → `>=1.2.3, <=2.0.0`；
//! - `||` → 依次尝试每个分支；
//! - `*` / `x` / 空 → 任意版本。
//!
//! 预发布：对齐 node-semver 的 `includePrerelease: true`——先按原样匹配，不中再去掉预发布
//! 标签匹配一次。

use semver::{Prerelease, Version, VersionReq};

/// 判断版本是否满足 range。
pub fn matches(range: &str, version: &str) -> Result<bool, String> {
    let version = Version::parse(version.trim()).map_err(|err| format!("版本「{version}」不合法：{err}"))?;
    for alternative in alternatives(range)? {
        if matches_one(&alternative, &version) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// 只校验 range 能不能被理解（索引读取时用）。
pub fn validate(range: &str) -> Result<(), String> {
    let _ = alternatives(range)?;
    Ok(())
}

fn alternatives(range: &str) -> Result<Vec<VersionReq>, String> {
    let range = range.trim();
    if range.is_empty() {
        return Err("range 是空串".into());
    }
    let mut requirements = Vec::new();
    for alternative in range.split("||") {
        let normalized = normalize(alternative)?;
        if normalized == "*" {
            requirements.push(VersionReq::STAR);
            continue;
        }
        requirements.push(
            VersionReq::parse(&normalized)
                .map_err(|err| format!("「{alternative}」不是合法 range：{err}"))?,
        );
    }
    if requirements.is_empty() {
        return Err("range 是空串".into());
    }
    Ok(requirements)
}

fn normalize(alternative: &str) -> Result<String, String> {
    let alternative = alternative.trim();
    if alternative.is_empty() {
        return Err("range 里有空分支".into());
    }
    if matches!(alternative, "*" | "x" | "X") {
        return Ok("*".into());
    }

    // 连字符区间：`1.2.3 - 2.0.0`。
    if let Some((low, high)) = split_hyphen(alternative) {
        return Ok(format!(">={low}, <={high}"));
    }

    let mut comparators: Vec<String> = Vec::new();
    let tokens: Vec<&str> = alternative.split_whitespace().collect();
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index];
        // 操作符单独成词：`>= 1.9.0`。
        if is_bare_operator(token) {
            let next = tokens.get(index + 1).ok_or_else(|| format!("「{alternative}」操作符后面缺版本号"))?;
            comparators.push(format!("{token}{next}"));
            index += 2;
            continue;
        }
        comparators.push(token.to_string());
        index += 1;
    }
    if comparators.is_empty() {
        return Err(format!("「{alternative}」不是合法 range"));
    }
    Ok(comparators.join(", "))
}

fn is_bare_operator(token: &str) -> bool {
    matches!(token, ">" | ">=" | "<" | "<=" | "=" | "~" | "^")
}

fn split_hyphen(alternative: &str) -> Option<(&str, &str)> {
    let (low, high) = alternative.split_once(" - ")?;
    let low = low.trim();
    let high = high.trim();
    if low.is_empty() || high.is_empty() || low.contains(' ') || high.contains(' ') {
        return None;
    }
    Some((low, high))
}

fn matches_one(requirement: &VersionReq, version: &Version) -> bool {
    if requirement.matches(version) {
        return true;
    }
    // includePrerelease：去掉预发布标签再试一次。
    !version.pre.is_empty()
        && requirement.matches(&Version {
            pre: Prerelease::EMPTY,
            ..version.clone()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npm_style_space_separated_comparators() {
        assert!(matches(">=1.9.0 <2.0.0", "1.9.0").unwrap());
        assert!(!matches(">=1.9.0 <2.0.0", "2.1.0").unwrap());
        assert!(matches(">= 1.9.0", "1.9.3").unwrap(), "操作符后的空格要能吃");
    }

    #[test]
    fn or_branches() {
        assert!(matches("^1.9.0 || ^2.0.0", "2.2.0").unwrap());
        assert!(!matches("^1.9.0 || ^2.0.0", "3.0.0").unwrap());
    }

    #[test]
    fn hyphen_ranges() {
        assert!(matches("1.2.3 - 2.0.0", "1.9.9").unwrap());
        assert!(!matches("1.2.3 - 2.0.0", "2.0.1").unwrap());
    }

    #[test]
    fn wildcards() {
        assert!(matches("*", "0.0.1-alpha").unwrap());
        assert!(matches("^1.2.3", "1.4.0").unwrap());
        assert!(matches("~1.2.3", "1.2.9").unwrap());
        assert!(!matches("~1.2.3", "1.3.0").unwrap());
    }

    #[test]
    fn prerelease_participates() {
        assert!(matches(">=1.9.0 <2.0.0", "1.9.0-rc.1").unwrap());
        assert!(matches("^1.9.0", "1.9.0-beta.2").unwrap());
    }

    #[test]
    fn garbage_is_an_error_not_a_silent_no() {
        assert!(matches("not a range", "1.0.0").is_err());
        assert!(matches(">=1.0.0 ||", "1.0.0").is_err());
        assert!(matches("", "1.0.0").is_err());
    }

    #[test]
    fn validate_accepts_real_world_market_ranges() {
        for range in [">=1.9.0", "^1.9.0", "1.9.x", ">=1.9.0 <2.0.0", "^1 || ^2", "*"] {
            assert!(validate(range).is_ok(), "{range}");
        }
    }
}
