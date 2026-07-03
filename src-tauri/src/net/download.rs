//! 검증 다운로드 — PRD 8.2.5: `<이름>.<랜덤>.part` → 해시 검증 → 원자적 rename.
//! 파일 단위 재시도 3회. 목적지가 이미 있으면 건너뜀 (불변 캐시 규칙과 정합).
use super::{Fetch, NetError};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const MAX_ATTEMPTS: u32 = 3;

#[derive(Debug, Error)]
pub enum DownloadError {
    #[error(transparent)]
    Net(#[from] NetError),
    #[error("hash mismatch for {url}: expected {expected}, got {actual}")]
    HashMismatch { url: String, expected: String, actual: String },
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// 기대 해시 — Mojang 계열은 sha1, 매니페스트 계열은 sha256 (PRD 7.3).
/// None은 해시를 제공하지 않는 소스(Fabric maven 등) 전용 — 경고 로그 후 무검증 저장.
#[derive(Debug, Clone, Copy)]
pub enum ExpectedHash<'a> {
    Sha256(&'a str),
    Sha1(&'a str),
    None,
}

impl ExpectedHash<'_> {
    fn expected_hex(&self) -> &str {
        match self {
            ExpectedHash::Sha256(h) | ExpectedHash::Sha1(h) => h,
            ExpectedHash::None => "",
        }
    }

    fn hex_of_file(&self, path: &Path) -> io::Result<String> {
        fn run<D: Digest + io::Write>(path: &Path) -> io::Result<String> {
            let mut file = fs::File::open(path)?;
            let mut hasher = D::new();
            io::copy(&mut file, &mut hasher)?;
            let mut out = String::new();
            for b in hasher.finalize() {
                use std::fmt::Write;
                let _ = write!(out, "{b:02x}");
            }
            Ok(out)
        }
        match self {
            ExpectedHash::Sha256(_) => run::<Sha256>(path),
            ExpectedHash::Sha1(_) => run::<Sha1>(path),
            ExpectedHash::None => Ok(String::new()),
        }
    }
}

fn temp_path_for(dest: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = dest
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".into());
    dest.with_file_name(format!("{name}.{nanos:08x}{n:04x}.part"))
}

/// dest가 이미 있으면 다운로드 없이 성공. 아니면 임시 파일 수신 → 해시 검증 → rename.
/// 일시 오류/해시 불일치는 총 3회까지 재시도 (PRD 8.2.5).
pub fn download_verified(
    fetch: &dyn Fetch,
    dest: &Path,
    url: &str,
    expected: ExpectedHash<'_>,
) -> Result<PathBuf, DownloadError> {
    if dest.is_file() {
        return Ok(dest.to_path_buf());
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut last_err: Option<DownloadError> = None;
    for _ in 0..MAX_ATTEMPTS {
        let temp = temp_path_for(dest);
        let result = (|| -> Result<(), DownloadError> {
            let mut file = fs::File::create(&temp)?;
            fetch.get_to_writer(url, &mut file)?;
            file.sync_all()?;
            drop(file);
            if matches!(expected, ExpectedHash::None) {
                tracing::warn!(url, "no hash provided by source; storing unverified");
                return Ok(());
            }
            let actual = expected.hex_of_file(&temp)?;
            if actual != expected.expected_hex() {
                return Err(DownloadError::HashMismatch {
                    url: url.to_string(),
                    expected: expected.expected_hex().to_string(),
                    actual,
                });
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                fs::rename(&temp, dest)?;
                return Ok(dest.to_path_buf());
            }
            Err(e) => {
                let _ = fs::remove_file(&temp);
                // 정책 위반(https 아님)은 재시도 무의미 — 즉시 반환
                if matches!(e, DownloadError::Net(NetError::Policy(_))) {
                    return Err(e);
                }
                last_err = Some(e);
            }
        }
    }
    Err(last_err.expect("at least one attempt"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::test_support::MockFetch;

    // "hello aqua" sha256 / sha1
    const SHA256: &str = "e8c83cdad46a35f7005e1bb85757108417155e932f48eac5ac7d87093cea0295";
    const SHA1: &str = "89a1bbe8026ab9dabb08dd28146ac98512b52674";
    const URL: &str = "https://example.com/hello.txt";

    fn fetch() -> MockFetch {
        MockFetch::with(&[(URL, b"hello aqua")])
    }

    #[test]
    fn downloads_and_verifies_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("libs/hello.txt");
        let out = download_verified(&fetch(), &dest, URL, ExpectedHash::Sha256(SHA256)).unwrap();
        assert_eq!(fs::read(out).unwrap(), b"hello aqua");
        // .part 잔여물 없음
        let leftovers: Vec<_> = fs::read_dir(dest.parent().unwrap())
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".part"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn downloads_and_verifies_sha1() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("hello.txt");
        download_verified(&fetch(), &dest, URL, ExpectedHash::Sha1(SHA1)).unwrap();
        assert!(dest.is_file());
    }

    #[test]
    fn existing_dest_skips_network() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("hello.txt");
        fs::write(&dest, b"already here").unwrap();
        let f = fetch();
        download_verified(&f, &dest, URL, ExpectedHash::Sha256(SHA256)).unwrap();
        assert_eq!(f.call_count(), 0, "immutable: no re-download");
        assert_eq!(fs::read(&dest).unwrap(), b"already here");
    }

    #[test]
    fn hash_mismatch_retries_then_fails_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("hello.txt");
        let f = fetch();
        let err = download_verified(&f, &dest, URL, ExpectedHash::Sha256(&"0".repeat(64)))
            .unwrap_err();
        assert!(matches!(err, DownloadError::HashMismatch { .. }));
        assert_eq!(f.call_count(), MAX_ATTEMPTS as usize);
        assert!(!dest.exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0, "no temp leftovers");
    }

    #[test]
    fn transient_failure_then_success() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("hello.txt");
        let mut f = fetch();
        f.fail_first = 2; // 2회 실패 후 성공 — 3회 한도 내
        download_verified(&f, &dest, URL, ExpectedHash::Sha256(SHA256)).unwrap();
        assert_eq!(f.call_count(), 3);
        assert!(dest.is_file());
    }
}
