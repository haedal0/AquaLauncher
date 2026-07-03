//! 네트워크 계층 — PRD §11 (https 강제), §8.17 (식별 가능한 User-Agent).
//!
//! 모든 원격 접근은 `Fetch` trait 뒤에 둔다 (외부 API trait 추상화 + 목 동반, AGENT.md).
//! 블로킹 클라이언트다: Tauri 커맨드에서는 반드시 `spawn_blocking` 계열로 감싼다.

pub mod download;
pub mod mojang;

use aqua_manifest::urlpolicy::{validate_distribution_url, UrlPolicyError};
use std::io::Write;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NetError {
    #[error(transparent)]
    Policy(#[from] UrlPolicyError),
    #[error("http error: {0}")]
    Http(String),
    #[error("unexpected status {status} for {url}")]
    Status { url: String, status: u16 },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub trait Fetch: Send + Sync {
    /// URL 정책 검사 후 GET, 본문을 writer로 스트리밍. 반환값은 수신 바이트 수.
    fn get_to_writer(&self, url: &str, out: &mut dyn Write) -> Result<u64, NetError>;

    /// 소형 응답(메타 JSON 등) 편의 메서드.
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, NetError> {
        let mut buf = Vec::new();
        self.get_to_writer(url, &mut buf)?;
        Ok(buf)
    }
}

pub struct HttpFetcher {
    client: reqwest::blocking::Client,
}

impl HttpFetcher {
    pub fn new() -> Result<Self, NetError> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(concat!("AquaLauncher/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| NetError::Http(e.to_string()))?;
        Ok(HttpFetcher { client })
    }
}

impl Fetch for HttpFetcher {
    fn get_to_writer(&self, url: &str, out: &mut dyn Write) -> Result<u64, NetError> {
        validate_distribution_url(url)?;
        let mut resp = self
            .client
            .get(url)
            .send()
            .map_err(|e| NetError::Http(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(NetError::Status { url: url.to_string(), status: status.as_u16() });
        }
        let n = resp.copy_to(out).map_err(|e| NetError::Http(e.to_string()))?;
        Ok(n)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 테스트용 목 — url -> 본문. fail_first로 일시 오류 주입.
    pub struct MockFetch {
        pub responses: BTreeMap<String, Vec<u8>>,
        pub fail_first: usize,
        pub calls: AtomicUsize,
    }

    impl MockFetch {
        pub fn with(pairs: &[(&str, &[u8])]) -> Self {
            MockFetch {
                responses: pairs
                    .iter()
                    .map(|(u, b)| (u.to_string(), b.to_vec()))
                    .collect(),
                fail_first: 0,
                calls: AtomicUsize::new(0),
            }
        }
        pub fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl Fetch for MockFetch {
        fn get_to_writer(&self, url: &str, out: &mut dyn Write) -> Result<u64, NetError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n < self.fail_first {
                return Err(NetError::Http("injected transient failure".into()));
            }
            match self.responses.get(url) {
                Some(body) => {
                    out.write_all(body)?;
                    Ok(body.len() as u64)
                }
                None => Err(NetError::Status { url: url.to_string(), status: 404 }),
            }
        }
    }
}
