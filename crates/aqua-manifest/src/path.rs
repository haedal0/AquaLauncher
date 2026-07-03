//! 경로 안전 유틸 — PRD 7.3 / 11장
//!
//! 매니페스트 files.path 등 모든 신뢰하지 않는 상대경로는 이 함수로만 조합한다.
//! **직접 Path::join 금지 (AGENT.md 코딩 규칙).**
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PathError {
    #[error("absolute paths are not allowed")]
    Absolute,
    #[error("path traversal or disallowed component")]
    Traversal,
    #[error("empty path")]
    Empty,
}

/// `root` 아래로만 향하는 안전한 경로를 만든다.
/// TODO(PRD 11장): 최종 커밋 직전 심볼릭 링크 실체 검사(canonicalize)를 sync 엔진에서 추가할 것.
pub fn safe_join(root: &Path, untrusted_relative: &str) -> Result<PathBuf, PathError> {
    if untrusted_relative.starts_with('/') || untrusted_relative.starts_with('\\') {
        return Err(PathError::Absolute);
    }
    let rel = Path::new(untrusted_relative);
    if rel.is_absolute() || rel.components().any(|c| matches!(c, Component::Prefix(_))) {
        return Err(PathError::Absolute);
    }
    let mut clean = PathBuf::new();
    for c in rel.components() {
        match c {
            Component::Normal(seg) => clean.push(seg),
            Component::CurDir => {}
            _ => return Err(PathError::Traversal), // ParentDir, RootDir
        }
    }
    if clean.as_os_str().is_empty() {
        return Err(PathError::Empty);
    }
    Ok(root.join(clean))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn root() -> &'static Path { Path::new("/inst") }

    #[test]
    fn accepts_normal_relative() {
        assert!(safe_join(root(), "config/servermod.toml").is_ok());
        assert!(safe_join(root(), "./options.txt").is_ok());
    }
    #[test]
    fn rejects_parent_traversal() {
        assert_eq!(safe_join(root(), "../evil").unwrap_err(), PathError::Traversal);
        assert_eq!(safe_join(root(), "a/../../b").unwrap_err(), PathError::Traversal);
    }
    #[test]
    fn rejects_absolute() {
        assert_eq!(safe_join(root(), "/etc/passwd").unwrap_err(), PathError::Absolute);
        assert_eq!(safe_join(root(), "\\windows\\x").unwrap_err(), PathError::Absolute);
    }
    #[test]
    fn rejects_empty() {
        assert_eq!(safe_join(root(), "").unwrap_err(), PathError::Empty);
    }
}
