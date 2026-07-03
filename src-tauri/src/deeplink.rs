//! 딥링크 파서 — PRD §8.7. `aqualauncher://add?manifest=<url-encoded-https-url>`.
//! 외부에서 진입하는 입력(§11): 스킴/액션/매니페스트 URL 전부 검증 통과 후에만 모달로 전달.
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DeepLinkError {
    #[error("unsupported deep link: {0}")]
    Unsupported(String),
    #[error("manifest param missing or malformed")]
    BadManifestParam,
    #[error("manifest url must be https")]
    NotHttps,
}

/// 엄격한 퍼센트 디코딩 — 잘못된 시퀀스(%zz, 잘린 %)는 통째로 거부한다.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = bytes.get(i + 1..i + 3)?;
                let hi = (hex[0] as char).to_digit(16)?;
                let lo = (hex[1] as char).to_digit(16)?;
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// `aqualauncher://add?...`에서 검증된 https 매니페스트 URL을 꺼낸다.
/// 통과한 URL만 확인 모달(§8.7)로 전달할 것 — 자동 생성 금지.
pub fn parse_add_link(raw: &str) -> Result<String, DeepLinkError> {
    let lower = raw.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("aqualauncher://")
        .ok_or_else(|| DeepLinkError::Unsupported(short(raw)))?;
    // 액션은 add만 지원 (스킴 검증 §8.7) — 대소문자 무시
    if rest != "add" && !rest.starts_with("add?") {
        return Err(DeepLinkError::Unsupported(short(raw)));
    }
    // 쿼리는 원본에서 추출 (대소문자 보존 — URL 값 훼손 금지)
    let query = raw.split_once('?').map(|x| x.1).unwrap_or("");
    let manifest = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("manifest="))
        .filter(|v| !v.is_empty())
        .ok_or(DeepLinkError::BadManifestParam)?;
    let url = percent_decode(manifest).ok_or(DeepLinkError::BadManifestParam)?;
    // §8.7 확정: https만 — 픽스처용 루프백 http 예외(§11 urlpolicy)도 딥링크에는 적용하지 않는다
    if !url.starts_with("https://") || url.len() <= "https://".len() {
        return Err(DeepLinkError::NotHttps);
    }
    Ok(url)
}

/// 에러 메시지용 절단 — 임의 길이 외부 입력을 로그에 그대로 싣지 않는다 (§8.13).
fn short(raw: &str) -> String {
    raw.chars().take(64).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_add_link() {
        let url = parse_add_link(
            "aqualauncher://add?manifest=https%3A%2F%2Fsrv.example.com%2Fpack%2Fmanifest.json",
        )
        .unwrap();
        assert_eq!(url, "https://srv.example.com/pack/manifest.json");
    }

    #[test]
    fn scheme_is_case_insensitive() {
        let url =
            parse_add_link("AquaLauncher://ADD?manifest=https%3A%2F%2Fs.kr%2Fm.json").unwrap();
        assert_eq!(url, "https://s.kr/m.json");
    }

    #[test]
    fn ignores_extra_params_and_takes_manifest() {
        let url = parse_add_link(
            "aqualauncher://add?utm=x&manifest=https%3A%2F%2Fs.kr%2Fm.json&y=2",
        )
        .unwrap();
        assert_eq!(url, "https://s.kr/m.json");
    }

    #[test]
    fn rejects_wrong_scheme_or_action() {
        // 스킴 검증 (§8.7) — 다른 스킴/액션은 전부 거부
        for raw in [
            "https://add?manifest=https%3A%2F%2Fs.kr%2Fm.json",
            "aqualauncher2://add?manifest=https%3A%2F%2Fs.kr%2Fm.json",
            "aqualauncher://delete?manifest=https%3A%2F%2Fs.kr%2Fm.json",
            "aqualauncher://",
            "",
        ] {
            assert!(parse_add_link(raw).is_err(), "should reject {raw:?}");
        }
    }

    #[test]
    fn rejects_missing_or_empty_manifest_param() {
        assert!(parse_add_link("aqualauncher://add").is_err());
        assert!(parse_add_link("aqualauncher://add?manifest=").is_err());
        assert!(parse_add_link("aqualauncher://add?other=1").is_err());
    }

    #[test]
    fn rejects_non_https_manifest_urls() {
        // §8.7: 매니페스트 URL은 https만 — 픽스처용 루프백 http도 딥링크에서는 불허
        for inner in [
            "http://srv.example.com/m.json",
            "http://127.0.0.1:8750/manifest.json",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "aqualauncher://add",
            "not a url",
        ] {
            let raw = format!("aqualauncher://add?manifest={}", percent_encode(inner));
            assert!(parse_add_link(&raw).is_err(), "should reject inner {inner:?}");
        }
    }

    #[test]
    fn decodes_percent_encoding_strictly() {
        // 잘못된 인코딩(%zz, 잘린 %)은 거부
        assert!(parse_add_link("aqualauncher://add?manifest=https%3A%2F%2Fs.kr%2Fm%zz").is_err());
        assert!(parse_add_link("aqualauncher://add?manifest=https%3A%2F%2Fs.kr%2Fm%2").is_err());
    }

    fn percent_encode(s: &str) -> String {
        let mut out = String::new();
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char)
                }
                _ => {
                    use std::fmt::Write;
                    let _ = write!(out, "%{b:02X}");
                }
            }
        }
        out
    }
}
