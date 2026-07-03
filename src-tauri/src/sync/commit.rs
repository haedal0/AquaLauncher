//! 스테이징 → 커밋 + 저널 + 크래시 복구 — PRD 8.2.4, TD-02 §4.
//! **사람 리뷰 필수 구역** (파일 삭제·커밋 저널·롤백 로직, AGENT.md).
//!
//! 규칙:
//! - remove 전부 → move 전부, 각 op는 멱등 (재개 안전).
//! - Remove는 저널 생성 시점에 현재 lockfile의 origin=manifest 확인 후에만 기록.
//! - 경로 조합은 전부 safe_join — 저널 파일도 신뢰하지 않는 입력이다.
use aqua_manifest::lockfile::{Lockfile, Origin};
use aqua_manifest::path::{safe_join, PathError};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;
use thiserror::Error;

pub const STAGING_DIR: &str = ".staging";
pub const JOURNAL_FILE: &str = "journal.json";
pub const LOCKFILE_NAME: &str = "manifest.lock.json";

#[derive(Debug, Error)]
pub enum CommitError {
    #[error("refusing to remove non-manifest file: {0}")]
    RemoveNotAllowed(String),
    #[error("move source and destination both missing: {0}")]
    MissingSource(String),
    #[error("unsafe path in journal: {0}")]
    UnsafePath(#[from] PathError),
    #[error("journal parse: {0}")]
    Parse(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// 커밋 저널 — TD-02 §4.2 포맷과 1:1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Journal {
    pub version: u32,
    pub sync_id: String,
    pub target_manifest_hash: String,
    pub created_at: String,
    pub ops: Vec<Op>,
    /// 커밋 완료 시점에 쓸 lockfile 전문
    pub final_lockfile: Lockfile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Op {
    Remove { path: String },
    Move { from: String, to: String },
}

/// 시작 시 복구 결과 — TD-02 §4.3.
#[derive(Debug, PartialEq, Eq)]
pub enum Recovery {
    /// 잔존 스테이징 없음
    Clean,
    /// 저널 발견 → 커밋 재개 완료
    Resumed { sync_id: String },
    /// 저널 없는/손상된 스테이징 폐기 (개수)
    DiscardedStaging(usize),
}

/// Remove 대상의 origin=manifest를 검증하며 저널을 만든다 (AGENT.md 파일 조작 규칙).
pub fn build_journal(
    sync_id: &str,
    target_manifest_hash: &str,
    created_at: &str,
    ops: Vec<Op>,
    final_lockfile: Lockfile,
    current_lock: &Lockfile,
) -> Result<Journal, CommitError> {
    for op in &ops {
        if let Op::Remove { path } = op {
            // lockfile은 논리 경로(".disabled" 제외)를 기록한다 (TD-02 §2)
            let logical = path.strip_suffix(".disabled").unwrap_or(path);
            let known_manifest_origin = current_lock
                .managed_files
                .iter()
                .any(|mf| mf.path == logical && mf.origin == Origin::Manifest);
            if !known_manifest_origin {
                return Err(CommitError::RemoveNotAllowed(path.clone()));
            }
        }
    }
    Ok(Journal {
        version: 1,
        sync_id: sync_id.into(),
        target_manifest_hash: target_manifest_hash.into(),
        created_at: created_at.into(),
        ops,
        final_lockfile,
    })
}

/// 파일을 원자적으로 쓴다: 임시 파일 → rename.
fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp-write");
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)
}

/// ops 적용 — remove 전부 → move 전부, 각 op 멱등 (TD-02 §4.2).
fn apply_ops(root: &Path, journal: &Journal) -> Result<(), CommitError> {
    for op in &journal.ops {
        if let Op::Remove { path } = op {
            let p = safe_join(root, path)?;
            match fs::remove_file(&p) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {} // 이미 없음 — 멱등
                Err(e) => return Err(e.into()),
            }
        }
    }
    for op in &journal.ops {
        if let Op::Move { from, to } = op {
            let src = safe_join(root, from)?;
            let dst = safe_join(root, to)?;
            if src.is_file() {
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::rename(&src, &dst)?;
            } else if dst.is_file() {
                // 이미 이동 완료 (크래시 후 재개) — 멱등 성공
            } else {
                return Err(CommitError::MissingSource(from.clone()));
            }
        }
    }
    Ok(())
}

/// 저널을 스테이징에 기록한 뒤 적용한다.
/// 성공 시: lockfile 원자적 갱신 → 저널·스테이징 정리. 실패 시 저널은 남는다(재시작 복구용).
pub fn commit(instance_root: &Path, journal: &Journal) -> Result<(), CommitError> {
    let staging = safe_join(instance_root, STAGING_DIR)?.join(&journal.sync_id);
    fs::create_dir_all(&staging)?;
    write_atomic(
        &staging.join(JOURNAL_FILE),
        serde_json::to_vec_pretty(journal)?.as_slice(),
    )?;

    apply_ops(instance_root, journal)?;

    write_atomic(
        &safe_join(instance_root, LOCKFILE_NAME)?,
        serde_json::to_vec_pretty(&journal.final_lockfile)?.as_slice(),
    )?;
    fs::remove_dir_all(&staging)?;
    Ok(())
}

/// 시작 시 잔존 `.staging/*` 처리: 저널이 있으면 커밋 재개, 없으면 폐기 — TD-02 §4.3.
pub fn recover(instance_root: &Path) -> Result<Recovery, CommitError> {
    let staging_root = safe_join(instance_root, STAGING_DIR)?;
    if !staging_root.is_dir() {
        return Ok(Recovery::Clean);
    }
    let mut discarded = 0usize;
    let mut resumed: Option<String> = None;
    for entry in fs::read_dir(&staging_root)? {
        let dir = entry?.path();
        if !dir.is_dir() {
            let _ = fs::remove_file(&dir);
            continue;
        }
        let journal_path = dir.join(JOURNAL_FILE);
        let journal = fs::read_to_string(&journal_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Journal>(&raw).ok());
        match journal {
            Some(j) if apply_resume(instance_root, &j).is_ok() => {
                resumed = Some(j.sync_id.clone());
            }
            _ => {
                // 저널 없음/손상/재개 실패 → 롤백: 스테이징 폐기, 기존 lockfile 유지 (TD-02 §4.3)
                fs::remove_dir_all(&dir)?;
                discarded += 1;
            }
        }
    }
    if let Some(sync_id) = resumed {
        return Ok(Recovery::Resumed { sync_id });
    }
    if discarded > 0 {
        return Ok(Recovery::DiscardedStaging(discarded));
    }
    Ok(Recovery::Clean)
}

/// 저널 기반 커밋 재개 — ops 멱등 재실행 → lockfile → 정리.
fn apply_resume(instance_root: &Path, journal: &Journal) -> Result<(), CommitError> {
    apply_ops(instance_root, journal)?;
    write_atomic(
        &safe_join(instance_root, LOCKFILE_NAME)?,
        serde_json::to_vec_pretty(&journal.final_lockfile)?.as_slice(),
    )?;
    let staging = safe_join(instance_root, STAGING_DIR)?.join(&journal.sync_id);
    fs::remove_dir_all(&staging)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aqua_manifest::lockfile::ManagedFile;

    fn managed(path: &str, origin: Origin) -> ManagedFile {
        ManagedFile {
            path: path.into(),
            mod_id: None,
            sha256: "x".into(),
            origin,
            enabled: true,
        }
    }

    fn lock(files: Vec<ManagedFile>) -> Lockfile {
        Lockfile {
            applied_manifest_hash: Some("sha256:new".into()),
            applied_at: Some("2026-07-03T00:00:00Z".into()),
            managed_files: files,
        }
    }

    /// 인스턴스 루트 + 스테이징 파일 세팅
    fn setup(root: &Path, staged: &[(&str, &str)], existing: &[(&str, &str)]) {
        for (rel, content) in existing {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, content).unwrap();
        }
        for (rel, content) in staged {
            let p = root.join(STAGING_DIR).join("abc123").join("files").join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, content).unwrap();
        }
    }

    fn journal_for(ops: Vec<Op>, final_lock: Lockfile) -> Journal {
        Journal {
            version: 1,
            sync_id: "abc123".into(),
            target_manifest_hash: "sha256:new".into(),
            created_at: "2026-07-03T00:00:00Z".into(),
            ops,
            final_lockfile: final_lock,
        }
    }

    fn mv(rel: &str) -> Op {
        Op::Move {
            from: format!("{STAGING_DIR}/abc123/files/{rel}"),
            to: rel.into(),
        }
    }

    #[test]
    fn commit_applies_ops_writes_lockfile_and_cleans_staging() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        setup(root, &[("mods/new-1.1.jar", "NEW")], &[("mods/old-1.0.jar", "OLD")]);
        let j = journal_for(
            vec![Op::Remove { path: "mods/old-1.0.jar".into() }, mv("mods/new-1.1.jar")],
            lock(vec![managed("mods/new-1.1.jar", Origin::Manifest)]),
        );
        commit(root, &j).unwrap();

        assert!(!root.join("mods/old-1.0.jar").exists(), "removed");
        assert_eq!(fs::read(root.join("mods/new-1.1.jar")).unwrap(), b"NEW");
        let written: Lockfile =
            serde_json::from_str(&fs::read_to_string(root.join(LOCKFILE_NAME)).unwrap()).unwrap();
        assert_eq!(written.applied_manifest_hash.as_deref(), Some("sha256:new"));
        assert!(!root.join(STAGING_DIR).join("abc123").exists(), "staging cleaned");
    }

    #[test]
    fn commit_is_idempotent_when_ops_partially_applied() {
        // 크래시 후 재개 시나리오: remove는 이미 수행, move 1개도 이미 수행된 상태
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        setup(root, &[("mods/b.jar", "B")], &[]);
        // a.jar는 이미 최종 위치에 있음 (move 완료 후 크래시 가정)
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::write(root.join("mods/a.jar"), "A").unwrap();
        let j = journal_for(
            vec![
                Op::Remove { path: "mods/gone.jar".into() }, // 이미 없음 — 멱등 성공
                mv("mods/a.jar"),                            // from 없음, to 있음 — 멱등 성공
                mv("mods/b.jar"),                            // 정상 move
            ],
            lock(vec![
                managed("mods/a.jar", Origin::Manifest),
                managed("mods/b.jar", Origin::Manifest),
            ]),
        );
        commit(root, &j).unwrap();
        assert_eq!(fs::read(root.join("mods/a.jar")).unwrap(), b"A");
        assert_eq!(fs::read(root.join("mods/b.jar")).unwrap(), b"B");
    }

    #[test]
    fn build_journal_rejects_remove_of_user_origin_file() {
        // origin=user 파일은 어떤 경로로도 자동 삭제 불가 (PRD 7.4 / AGENT.md)
        let current = lock(vec![managed("mods/my-mod.jar", Origin::User)]);
        let err = build_journal(
            "abc123",
            "sha256:new",
            "2026-07-03T00:00:00Z",
            vec![Op::Remove { path: "mods/my-mod.jar".into() }],
            lock(vec![]),
            &current,
        )
        .unwrap_err();
        assert!(matches!(err, CommitError::RemoveNotAllowed(_)));
    }

    #[test]
    fn build_journal_rejects_remove_of_unknown_file() {
        // lockfile에 없는 파일 삭제 시도 — origin=manifest 확인 불가 → 거부
        let err = build_journal(
            "abc123",
            "sha256:new",
            "2026-07-03T00:00:00Z",
            vec![Op::Remove { path: "mods/stranger.jar".into() }],
            lock(vec![]),
            &lock(vec![]),
        )
        .unwrap_err();
        assert!(matches!(err, CommitError::RemoveNotAllowed(_)));
    }

    #[test]
    fn build_journal_allows_remove_of_disabled_physical_path() {
        // 비활성 모드의 물리 경로는 "<논리>.disabled" — 논리 경로 기준으로 검증
        let current = lock(vec![managed("mods/old.jar", Origin::Manifest)]);
        assert!(build_journal(
            "abc123",
            "sha256:new",
            "t",
            vec![Op::Remove { path: "mods/old.jar.disabled".into() }],
            lock(vec![]),
            &current,
        )
        .is_ok());
    }

    #[test]
    fn commit_rejects_path_traversal_in_journal() {
        // 저널도 신뢰하지 않는 입력 — ".." 경로는 safe_join이 거부
        let dir = tempfile::tempdir().unwrap();
        let j = journal_for(
            vec![Op::Move { from: format!("{STAGING_DIR}/abc123/files/x"), to: "../escape".into() }],
            lock(vec![]),
        );
        let err = commit(dir.path(), &j).unwrap_err();
        assert!(matches!(err, CommitError::UnsafePath(_)));
    }

    #[test]
    fn recover_resumes_commit_from_persisted_journal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        setup(root, &[("mods/new.jar", "NEW")], &[]);
        let j = journal_for(
            vec![mv("mods/new.jar")],
            lock(vec![managed("mods/new.jar", Origin::Manifest)]),
        );
        let jpath = root.join(STAGING_DIR).join("abc123").join(JOURNAL_FILE);
        fs::write(&jpath, serde_json::to_string(&j).unwrap()).unwrap();

        let outcome = recover(root).unwrap();
        assert_eq!(outcome, Recovery::Resumed { sync_id: "abc123".into() });
        assert_eq!(fs::read(root.join("mods/new.jar")).unwrap(), b"NEW");
        assert!(root.join(LOCKFILE_NAME).exists());
        assert!(!root.join(STAGING_DIR).join("abc123").exists());
    }

    #[test]
    fn recover_discards_staging_without_journal() {
        // 저널 없음 = 커밋 미진입 — 통째로 폐기해도 안전 (TD-02 §4.1)
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        setup(root, &[("mods/partial.jar", "P")], &[]);
        let outcome = recover(root).unwrap();
        assert_eq!(outcome, Recovery::DiscardedStaging(1));
        assert!(!root.join(STAGING_DIR).join("abc123").exists());
    }

    #[test]
    fn recover_with_no_staging_is_clean() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(recover(dir.path()).unwrap(), Recovery::Clean);
    }
}
