//! PKCE 프리미티브 — RFC 7636 (S256만 지원). **사람 리뷰 필수 구역.**
//! verifier는 반드시 OS 암호학적 난수(getrandom)로 생성한다.
use crate::error::AppError;
use sha2::{Digest, Sha256};

pub const MSA_AUTHORIZE_URL: &str =
    "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize";

/// base64url 무패딩 인코딩 (RFC 4648 §5) — 의존성 없이 구현.
pub fn b64url_no_pad(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = u32::from_be_bytes([0, b[0], b[1], b[2]]);
        let chars = [(n >> 18) & 63, (n >> 12) & 63, (n >> 6) & 63, n & 63];
        for (i, c) in chars.iter().enumerate() {
            if i <= chunk.len() {
                out.push(TABLE[*c as usize] as char);
            }
        }
    }
    out
}

/// S256 챌린지 = BASE64URL(SHA256(verifier)) (RFC 7636 §4.2)
pub fn code_challenge_s256(verifier: &str) -> String {
    b64url_no_pad(&Sha256::digest(verifier.as_bytes()))
}

fn random_bytes<const N: usize>() -> Result<[u8; N], AppError> {
    let mut buf = [0u8; N];
    getrandom::getrandom(&mut buf)
        .map_err(|e| AppError::AuthFailed(format!("os rng unavailable: {e}")))?;
    Ok(buf)
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
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

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
    /// CSRF 방지용 — 리다이렉트 수신 측에서 반드시 대조한다.
    pub state: String,
}

impl Pkce {
    /// 32바이트 OS 난수 → 43자 verifier (RFC 7636 §4.1 권장 엔트로피)
    pub fn generate() -> Result<Self, AppError> {
        let verifier = b64url_no_pad(&random_bytes::<32>()?);
        let challenge = code_challenge_s256(&verifier);
        let state = b64url_no_pad(&random_bytes::<16>()?);
        Ok(Pkce { verifier, challenge, state })
    }

    /// 시스템 브라우저로 열 authorize URL (§8.3 확정 플로우).
    pub fn authorize_url(&self, client_id: &str, redirect_uri: &str) -> String {
        format!(
            "{MSA_AUTHORIZE_URL}?client_id={}&response_type=code&redirect_uri={}\
             &scope={}&code_challenge={}&code_challenge_method=S256&state={}",
            percent_encode(client_id),
            percent_encode(redirect_uri),
            percent_encode("XboxLive.signin offline_access"),
            percent_encode(&self.challenge),
            percent_encode(&self.state),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_matches_rfc7636_appendix_b_vector() {
        // RFC 7636 부록 B 공식 테스트 벡터
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(code_challenge_s256(verifier), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn generated_verifier_is_unreserved_and_within_rfc_length() {
        let p = Pkce::generate().unwrap();
        // RFC 7636 §4.1: 43~128자, unreserved 문자만
        assert!(p.verifier.len() >= 43 && p.verifier.len() <= 128, "len {}", p.verifier.len());
        assert!(p
            .verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~'));
        assert_eq!(p.challenge, code_challenge_s256(&p.verifier));
    }

    #[test]
    fn two_generations_differ() {
        let a = Pkce::generate().unwrap();
        let b = Pkce::generate().unwrap();
        assert_ne!(a.verifier, b.verifier);
        assert_ne!(a.state, b.state);
    }

    #[test]
    fn authorize_url_carries_pkce_and_state() {
        let p = Pkce {
            verifier: "v".repeat(43),
            challenge: "CHAL".into(),
            state: "STATE123".into(),
        };
        let url = p.authorize_url("client-abc", "http://127.0.0.1:43210/cb");
        assert!(url.starts_with(super::MSA_AUTHORIZE_URL));
        for needle in [
            "client_id=client-abc",
            "response_type=code",
            "code_challenge=CHAL",
            "code_challenge_method=S256",
            "state=STATE123",
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A43210%2Fcb",
            "scope=XboxLive.signin%20offline_access",
        ] {
            assert!(url.contains(needle), "missing {needle} in {url}");
        }
    }
}
