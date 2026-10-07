//! URL 规范化与确定性 key（zk 血统；cito `src/key.rs` 的收编上层）。
//!
//! 规则：小写 scheme/host、去 `www.`、去 userinfo、去 fragment、
//! 去 `utm_*`/`fbclid`/`gclid`、query 按 key 排序、去路径尾 `/`、
//! 剥 scheme 默认端口（http:80 / https:443，显式非默认端口保留）、
//! 缺 scheme 补 https。

use crate::error::{DataError, Result};
use sha1::{Digest, Sha1};

/// 小写 scheme/host、去 www.、去 userinfo、去 fragment、
/// 去 utm_*/fbclid/gclid、query 按 key 排序、去路径尾 /、去默认端口、缺 scheme 补 https。
pub fn normalize_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let mut u = url::Url::parse(&with_scheme)
        .map_err(|_| DataError::Url(format!("unparseable url: {raw}")))?;

    u.set_fragment(None);
    if u.host_str().is_none() {
        return Err(DataError::Url(format!("url has no host: {raw}")));
    }
    // 去 userinfo（user:pw@ 会残留在序列化中，必须显式清除）
    let _ = u.set_password(None);
    let _ = u.set_username("");
    // 小写 host、去 www.（url crate 已小写 host，这里防御性处理）
    let host = u.host_str().unwrap().to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    u.set_host(Some(&host))
        .map_err(|e| DataError::Url(format!("bad host: {e}")))?;
    // 只剥 scheme 默认端口（http:80 / https:443）；显式非默认端口（如 :8080）保留
    if let Some(port) = u.port() {
        let is_default = match u.scheme() {
            "http" => port == 80,
            "https" => port == 443,
            _ => false,
        };
        if is_default {
            let _ = u.set_port(None);
        }
    }

    // 去 tracking 参数并按 key 排序
    let pairs: Vec<(String, String)> = u
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .filter(|(k, _)| !is_tracking_key(k))
        .collect();
    let query = if pairs.is_empty() {
        None
    } else {
        let mut sorted = pairs;
        sorted.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        // key 与 value 一并保守编码：query_pairs 是解码后的形态，
        // 不编码 key 会让含保留字的 key（如 a%3Db）在二次规范化时变形，破坏幂等
        Some(
            sorted
                .iter()
                .map(|(k, v)| format!("{}={}", urlencode(k), urlencode(v)))
                .collect::<Vec<_>>()
                .join("&"),
        )
    };
    u.set_query(query.as_deref());

    // 去路径尾 /
    let path = u.path().trim_end_matches('/').to_string();
    u.set_path(if path.is_empty() { "/" } else { &path });

    let mut out = u.to_string();
    // url crate 对空路径序列化为 scheme://host/；规范形态保持裸 host
    if query.is_none() && u.path() == "/" && out.ends_with('/') {
        out.pop();
    }
    Ok(out)
}

fn is_tracking_key(k: &str) -> bool {
    k == "fbclid" || k == "gclid" || k.starts_with("utm_")
}

fn urlencode(v: &str) -> String {
    // query 值的保守百分号编码（与 url crate query_pairs 输出风格一致）
    let mut out = String::new();
    for b in v.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn h8(s: &str) -> String {
    let mut h = Sha1::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())[..8].to_string()
}

/// 规范化 URL 的 host → domain slug：小写、点转横线（www. 已在规范化时去除）
pub fn domain_slug(normalized: &str) -> String {
    let rest = normalized
        .split_once("://")
        .map(|(_, r)| r)
        .unwrap_or(normalized);
    let host = rest.split('/').next().unwrap_or(rest);
    host.split('@')
        .next_back()
        .unwrap_or(host)
        .replace('.', "-")
}

pub fn derive_key(raw_url: &str) -> Result<String> {
    let normalized = normalize_url(raw_url)?;
    Ok(format!("{}--{}", domain_slug(&normalized), h8(&normalized)))
}

pub fn derive_pub_key(title: &str, first_author: &str, published: &str) -> String {
    let year = published
        .split(|c: char| !c.is_ascii_digit())
        .find(|s| s.len() == 4)
        .unwrap_or("");
    format!("pub--{}", h8(&format!("{title}|{first_author}|{year}")))
}

/// key = base 或 base-kN（N≥2）
pub fn key_matches(key: &str, recomputed_base: &str) -> bool {
    if key == recomputed_base {
        return true;
    }
    match key.strip_prefix(&format!("{recomputed_base}-k")) {
        Some(n) => !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) && n != "0" && n != "1",
        None => false,
    }
}

pub fn strip_suffix(key: &str) -> &str {
    match key.rsplit_once("-k") {
        // 只有当 -k 后全为数字时才视为后缀（域名含 -k 的正常情况不受影响）
        Some((base, n)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => base,
        _ => key,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_case_www_fragment_tracking() {
        let n = normalize_url("HTTPS://WWW.Example.com/A/B/?utm_source=x&id=2#frag").unwrap();
        assert_eq!(n, "https://example.com/A/B?id=2");
    }

    #[test]
    fn sorts_query_and_strips_trailing_slash_and_default_port() {
        let n = normalize_url("https://example.com/path/?b=2&a=1").unwrap();
        assert_eq!(n, "https://example.com/path?a=1&b=2");
        let p = normalize_url("http://example.com:80/x").unwrap();
        assert_eq!(p, "http://example.com/x");
        let s = normalize_url("https://example.com:443/").unwrap();
        assert_eq!(s, "https://example.com");
    }

    #[test]
    fn strips_userinfo_and_adds_default_scheme() {
        let n = normalize_url("https://user:pw@example.com/doc").unwrap();
        assert_eq!(n, "https://example.com/doc");
        let m = normalize_url("example.com/doc").unwrap();
        assert_eq!(m, "https://example.com/doc");
    }

    #[test]
    fn removes_tracking_params_only() {
        let n = normalize_url("https://example.com/p?fbclid=zz&keep=1&gclid=yy&utm_campaign=a").unwrap();
        assert_eq!(n, "https://example.com/p?keep=1");
    }

    #[test]
    fn keeps_explicit_non_default_port() {
        let n = normalize_url("https://example.com:8080/x").unwrap();
        assert_eq!(n, "https://example.com:8080/x");
        let s = normalize_url("https://example.com:443/x").unwrap();
        assert_eq!(s, "https://example.com/x");
    }

    #[test]
    fn derive_key_is_deterministic_domain_h8() {
        let k1 = derive_key("https://a2a-protocol.org/A2A/latest/specification/").unwrap();
        let k2 = derive_key("HTTPS://A2A-PROTOCOL.ORG/A2A/latest/specification#x").unwrap();
        assert_eq!(k1, k2, "规范化后同源必同 key");
        assert_eq!(k1, "a2a-protocol-org--cf95d4df"); // M1-followups 字面钉
    }

    #[test]
    fn pub_key_uses_title_author_year() {
        let k = derive_pub_key("Some Title", "Alice", "2026-05-07");
        assert!(k.starts_with("pub--"));
        assert_eq!(k.len(), 5 + 8);
        let k2 = derive_pub_key("T", "", "");
        assert!(k2.starts_with("pub--"));
    }

    #[test]
    fn key_matching_accepts_suffix() {
        assert!(key_matches("ab-cd--12345678", "ab-cd--12345678"));
        assert!(key_matches("ab-cd--12345678-k2", "ab-cd--12345678"));
        assert!(!key_matches("ab-cd--12345678", "zz-zz--00000000"));
        assert!(!key_matches("ab-cd--12345678-k1", "ab-cd--12345678"), "后缀从 k2 起");
        assert!(!key_matches("ab-cd--12345678k2", "ab-cd--12345678"));
        assert_eq!(strip_suffix("ab-cd--12345678-k2"), "ab-cd--12345678");
    }

    #[test]
    fn invalid_url_rejected() {
        assert!(normalize_url("::bad::").is_err());
    }
}
