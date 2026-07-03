//! 내용 주소 공유 캐시 — PRD 7.1.
//! 불변(immutable): 덮어쓰기 금지, 새 경로 추가만. Windows 파일 잠금 회피의 근거.
//! 임시 파일은 `<이름>.<랜덤>.part` -> 해시 검증 -> 원자적 rename (PRD 8.2.5).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("invalid sha256 hex")]
    InvalidHash,
    #[error("hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// 내용 주소 저장소. 오브젝트 경로는 `objects/<hh>/<sha256-hex>`.
pub struct ContentStore {
    root: PathBuf,
}

/// `begin_insert`로 연 임시 수신 슬롯 — `<이름>.<랜덤>.part`.
/// commit 없이 drop되면 임시 파일은 정리된다.
pub struct PendingObject {
    temp_path: PathBuf,
    committed: bool,
}

fn is_valid_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn sha256_hex_of_file(path: &Path) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    Ok(out)
}

impl ContentStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        ContentStore { root: root.into() }
    }

    /// 검증된 sha256 hex의 최종 오브젝트 경로. 신뢰하지 않는 입력이므로 형식 검증.
    pub fn object_path(&self, sha256_hex: &str) -> Result<PathBuf, CacheError> {
        if !is_valid_sha256_hex(sha256_hex) {
            return Err(CacheError::InvalidHash);
        }
        Ok(self
            .root
            .join("objects")
            .join(&sha256_hex[..2])
            .join(sha256_hex))
    }

    pub fn contains(&self, sha256_hex: &str) -> bool {
        self.object_path(sha256_hex)
            .map(|p| p.is_file())
            .unwrap_or(false)
    }

    /// 임시 `.part` 파일 슬롯 생성. 호출자가 내용을 쓴 뒤 commit한다.
    /// 최종 경로와 같은 파일시스템에 두기 위해 저장소 루트 아래 tmp/를 쓴다 (원자적 rename 보장).
    pub fn begin_insert(&self, name_hint: &str) -> Result<PendingObject, CacheError> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let tmp_dir = self.root.join("tmp");
        fs::create_dir_all(&tmp_dir)?;
        // name_hint는 파일명 성분만 사용 (경로 성분 유입 차단)
        let base = Path::new(name_hint)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "download".to_string());
        loop {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let candidate = tmp_dir.join(format!("{base}.{:08x}{n:04x}.part", nanos));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(_) => {
                    return Ok(PendingObject {
                        temp_path: candidate,
                        committed: false,
                    })
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// 해시 검증 후 원자적 rename으로 확정. 이미 존재하면 기존 파일을 건드리지 않고 성공(멱등).
    /// 불변 캐시: 기존 오브젝트는 어떤 경우에도 다시 쓰지 않는다 (PRD 7.1).
    pub fn commit(
        &self,
        mut pending: PendingObject,
        expected_sha256: &str,
    ) -> Result<PathBuf, CacheError> {
        let dest = self.object_path(expected_sha256)?;
        let actual = sha256_hex_of_file(&pending.temp_path)?;
        if actual != expected_sha256 {
            return Err(CacheError::HashMismatch {
                expected: expected_sha256.to_string(),
                actual,
            }); // pending drop이 임시 파일을 정리한다
        }
        if dest.is_file() {
            return Ok(dest); // 멱등 — 기존 오브젝트 유지, 임시 파일은 drop에서 정리
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        match fs::rename(&pending.temp_path, &dest) {
            Ok(()) => {
                pending.committed = true;
                Ok(dest)
            }
            // 동시 커밋 경쟁: 다른 쪽이 먼저 확정했으면 성공 취급 (내용 동일 = 멱등)
            Err(_) if dest.is_file() => Ok(dest),
            Err(e) => Err(e.into()),
        }
    }
}

impl PendingObject {
    pub fn path(&self) -> &Path {
        &self.temp_path
    }
}

impl Drop for PendingObject {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.temp_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // "hello aqua"의 sha256
    const HELLO_SHA: &str = "e8c83cdad46a35f7005e1bb85757108417155e932f48eac5ac7d87093cea0295";

    fn store() -> (tempfile::TempDir, ContentStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::new(dir.path().join("cache"));
        (dir, store)
    }

    fn insert_hello(store: &ContentStore) -> Result<PathBuf, CacheError> {
        let pending = store.begin_insert("hello.txt")?;
        fs::write(pending.path(), b"hello aqua")?;
        store.commit(pending, HELLO_SHA)
    }

    #[test]
    fn commit_stores_object_at_content_addressed_path() {
        let (_d, store) = store();
        let path = insert_hello(&store).unwrap();
        assert_eq!(path, store.object_path(HELLO_SHA).unwrap());
        assert_eq!(fs::read(&path).unwrap(), b"hello aqua");
        assert!(store.contains(HELLO_SHA));
    }

    #[test]
    fn commit_rejects_hash_mismatch_and_cleans_temp() {
        let (_d, store) = store();
        let pending = store.begin_insert("evil.jar").unwrap();
        fs::write(pending.path(), b"tampered content").unwrap();
        let temp = pending.path().to_path_buf();
        let err = store.commit(pending, HELLO_SHA).unwrap_err();
        assert!(matches!(err, CacheError::HashMismatch { .. }));
        assert!(!temp.exists(), "temp .part must be removed on mismatch");
        assert!(!store.contains(HELLO_SHA));
    }

    #[test]
    fn existing_object_is_never_overwritten() {
        let (_d, store) = store();
        let path = insert_hello(&store).unwrap();
        // 불변성: 동일 해시 재삽입은 성공(멱등)하되 기존 파일을 교체하지 않는다.
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        let again = insert_hello(&store).unwrap();
        assert_eq!(again, path);
        let after = fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(before, after, "existing object must not be rewritten");
    }

    #[test]
    fn temp_slot_uses_part_suffix_with_random_component_under_root() {
        let (_d, store) = store();
        let a = store.begin_insert("mod.jar").unwrap();
        let b = store.begin_insert("mod.jar").unwrap();
        for p in [a.path(), b.path()] {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            assert!(name.starts_with("mod.jar."), "got {name}");
            assert!(name.ends_with(".part"), "got {name}");
        }
        assert_ne!(a.path(), b.path(), "random suffix must avoid collisions");
    }

    #[test]
    fn uncommitted_pending_is_cleaned_on_drop() {
        let (_d, store) = store();
        let pending = store.begin_insert("dropped.jar").unwrap();
        fs::write(pending.path(), b"abandoned").unwrap();
        let temp = pending.path().to_path_buf();
        drop(pending);
        assert!(!temp.exists());
    }

    #[test]
    fn object_path_rejects_invalid_hash_input() {
        let (_d, store) = store();
        // 신뢰하지 않는 입력: 길이/문자셋 검증 (경로 조작 차단)
        assert!(matches!(store.object_path("abc"), Err(CacheError::InvalidHash)));
        assert!(matches!(
            store.object_path("../../etc/passwd"),
            Err(CacheError::InvalidHash)
        ));
        let upper = HELLO_SHA.to_uppercase();
        assert!(matches!(store.object_path(&upper), Err(CacheError::InvalidHash)));
    }
}
