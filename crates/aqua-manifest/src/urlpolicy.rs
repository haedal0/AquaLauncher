//! 배포 URL 정책 — PRD 11장 "네트워크는 https만".
//!
//! 예외: 루프백 호스트(127.0.0.1 / localhost / [::1])는 http 허용.
//! 근거: 저장소 픽스처 서버(fixtures/, http://127.0.0.1:8750)로 M2 E2E 게이트를
//! 검증해야 하며, 루프백은 네트워크 경계를 벗어나지 않는다. 외부 호스트는 예외 없음.
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum UrlPolicyError {
    #[error("only https urls are allowed (got {0})")]
    NotHttps(String),
    #[error("malformed url: {0}")]
    Malformed(String),
}

fn host_of(rest: &str) -> &str {
    // scheme:// 이후 첫 '/' 전까지에서 userinfo/포트 제거
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let after_userinfo = authority.rsplit('@').next().unwrap_or(authority);
    if let Some(v6) = after_userinfo.strip_prefix('[') {
        // "[::1]:8750" → "::1"
        return v6.split(']').next().unwrap_or("");
    }
    after_userinfo.split(':').next().unwrap_or("")
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

/// 매니페스트/모드/테마 등 모든 배포 URL에 적용하는 정책 검사.
pub fn validate_distribution_url(url: &str) -> Result<(), UrlPolicyError> {
    let lower = url.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("https://") {
        if host_of(rest).is_empty() {
            return Err(UrlPolicyError::Malformed(url.to_string()));
        }
        return Ok(());
    }
    if let Some(rest) = lower.strip_prefix("http://") {
        let host = host_of(rest);
        if host.is_empty() {
            return Err(UrlPolicyError::Malformed(url.to_string()));
        }
        if is_loopback_host(host) {
            return Ok(()); // 개발 픽스처 예외
        }
        return Err(UrlPolicyError::NotHttps(url.to_string()));
    }
    Err(UrlPolicyError::Malformed(url.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_is_allowed() {
        assert!(validate_distribution_url("https://example.com/mods/a.jar").is_ok());
        assert!(validate_distribution_url("HTTPS://EXAMPLE.COM/x").is_ok());
    }

    #[test]
    fn loopback_http_is_allowed_for_fixtures() {
        assert!(validate_distribution_url("http://127.0.0.1:8750/manifest.json").is_ok());
        assert!(validate_distribution_url("http://localhost:8750/x").is_ok());
        assert!(validate_distribution_url("http://[::1]:8750/x").is_ok());
    }

    #[test]
    fn external_http_is_rejected() {
        assert_eq!(
            validate_distribution_url("http://example.com/a.jar"),
            Err(UrlPolicyError::NotHttps("http://example.com/a.jar".into()))
        );
        // 루프백으로 위장 시도
        assert!(validate_distribution_url("http://localhost.evil.com/x").is_err());
        assert!(validate_distribution_url("http://evil.com@localhost.evil.com/x").is_err());
    }

    #[test]
    fn other_schemes_and_garbage_rejected() {
        assert!(validate_distribution_url("file:///etc/passwd").is_err());
        assert!(validate_distribution_url("ftp://x/y").is_err());
        assert!(validate_distribution_url("not a url").is_err());
        assert!(validate_distribution_url("http://").is_err());
    }
}
