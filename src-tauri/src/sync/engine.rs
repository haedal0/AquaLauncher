//! 동기화 오케스트레이터 — TD-02 §1 상태 흐름의 실행부.
//! Fetching(검증) → Planning(diff) → Staging(다운로드) → Committing(저널).
//!
//! 실패 지점별 보장: Staging까지의 실패는 스테이징 폐기로 기존 상태 무손상 (PRD 8.2.4).
use super::commit::{build_journal, commit, CommitError, Op, STAGING_DIR};
use super::{build_final_lockfile, plan_diff, DiffAction};
use crate::browse::modrinth::ModrinthClient;
use crate::net::download::{download_verified, DownloadError, ExpectedHash};
use crate::net::{Fetch, NetError};
use aqua_manifest::lockfile::Lockfile;
use aqua_manifest::manifest::{Manifest, Source, SUPPORTED_FORMAT_VERSION};
use aqua_manifest::path::safe_join;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SyncError {
    /// E-MF-01 — 호출측에서 오프라인 판정(§8.2.6: 마지막 정합 상태로 플레이 허용)
    #[error("manifest fetch failed: {0}")]
    Fetch(#[from] NetError),
    /// E-MF-02
    #[error("manifest parse failed: {0}")]
    Parse(String),
    /// E-MF-03
    #[error("unsupported manifest format_version {0}")]
    UnsupportedFormat(u32),
    /// E-MF-03 (min_launcher_version)
    #[error("launcher update required: manifest needs {0}")]
    LauncherTooOld(String),
    /// E-MF-05
    #[error("minecraft {0} is below the 1.13 floor")]
    McTooOld(String),
    /// modrinth/curseforge 소스 해석은 browse API 연동 후 지원 (M2 후속)
    #[error("source type not yet resolvable for {0}")]
    UnsupportedSource(String),
    /// E-MF-04 — 실패 파일 목록 (부분 재시도 UI용)
    #[error("{} file(s) failed to download", failed.len())]
    Download { failed: Vec<String> },
    #[error(transparent)]
    Commit(#[from] CommitError),
    #[error(transparent)]
    Path(#[from] aqua_manifest::path::PathError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub struct FetchedManifest {
    pub manifest: Manifest,
    /// 콘텐츠 변경 감지용 SHA-256 (PRD 7.2 — 해시 비교)
    pub hash_hex: String,
}

fn sha256_hex_of_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let mut out = String::with_capacity(64);
    for b in hasher.finalize() {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn parse_triple(v: &str) -> Option<(u32, u32, u32)> {
    let mut it = v.split('.');
    Some((
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next().unwrap_or("0").parse().ok()?,
    ))
}

fn mc_at_least_1_13(v: &str) -> bool {
    let mut it = v.split('.');
    match (
        it.next().and_then(|s| s.parse::<u32>().ok()),
        it.next().and_then(|s| s.parse::<u32>().ok()),
    ) {
        (Some(maj), Some(min)) => (maj, min) >= (1, 13),
        _ => false,
    }
}

/// Fetching 단계 — 수신 + 해시 + 스키마/버전 검증 (E-MF-01/02/03/05 분류).
pub fn fetch_manifest(fetch: &dyn Fetch, url: &str) -> Result<FetchedManifest, SyncError> {
    let raw = fetch.get_bytes(url)?;
    let hash_hex = sha256_hex_of_bytes(&raw);
    let manifest: Manifest =
        serde_json::from_slice(&raw).map_err(|e| SyncError::Parse(e.to_string()))?;

    if manifest.format_version != SUPPORTED_FORMAT_VERSION {
        return Err(SyncError::UnsupportedFormat(manifest.format_version));
    }
    let current = parse_triple(env!("CARGO_PKG_VERSION")).expect("valid pkg version");
    match parse_triple(&manifest.min_launcher_version) {
        Some(required) if required > current => {
            return Err(SyncError::LauncherTooOld(manifest.min_launcher_version.clone()));
        }
        _ => {}
    }
    if !mc_at_least_1_13(&manifest.minecraft_version) {
        return Err(SyncError::McTooOld(manifest.minecraft_version.clone()));
    }
    Ok(FetchedManifest { manifest, hash_hex })
}

#[derive(Debug)]
pub struct SyncOutcome {
    /// false = 이미 정합(Keep만) — 커밋 없이 종료
    pub changed: bool,
    pub lockfile: Lockfile,
}

/// 소스 → 다운로드 URL 해석 — PRD 7.3 소스 3종.
/// 어느 소스든 다운로드 후 검증은 매니페스트의 sha256으로 동일하게 수행한다.
pub trait SourceResolver: Send + Sync {
    fn resolve_url(&self, source: &Source) -> Result<String, String>;
}

/// url 소스만 해석 (modrinth/curseforge 미지원 환경용 폴백)
pub struct UrlOnlyResolver;

impl SourceResolver for UrlOnlyResolver {
    fn resolve_url(&self, source: &Source) -> Result<String, String> {
        match source {
            Source::Url { url } => Ok(url.clone()),
            other => Err(format!("unresolvable source: {other:?}")),
        }
    }
}

/// url 직통 + modrinth는 version API 경유 해석 (PRD 7.3).
/// curseforge는 API 키 승인(PRD 0-4) 전까지 미지원 — E-MF 계열로 표면화된다.
pub struct OnlineResolver<'a> {
    pub fetch: &'a dyn Fetch,
}

impl SourceResolver for OnlineResolver<'_> {
    fn resolve_url(&self, source: &Source) -> Result<String, String> {
        match source {
            Source::Url { url } => Ok(url.clone()),
            Source::Modrinth { version_id, .. } => {
                let client = ModrinthClient { fetch: self.fetch };
                let info = client.version(version_id).map_err(|e| e.to_string())?;
                let file = ModrinthClient::primary_file(&info).map_err(|e| e.to_string())?;
                Ok(file.url.clone())
            }
            Source::Curseforge { .. } => {
                Err("curseforge source requires an API key (PRD 0-4)".into())
            }
        }
    }
}

/// 대상 경로 → (소스, sha256) 매핑.
fn source_map(manifest: &Manifest) -> BTreeMap<String, (Source, String)> {
    let mut map = BTreeMap::new();
    for m in &manifest.mods {
        map.insert(format!("mods/{}", m.filename), (m.source.clone(), m.sha256.clone()));
    }
    for f in &manifest.files {
        map.insert(f.path.clone(), (f.source.clone(), f.sha256.clone()));
    }
    if let Some(rp) = &manifest.resourcepack {
        map.insert(
            format!("resourcepacks/{}", rp.filename),
            (rp.source.clone(), rp.sha256.clone()),
        );
    }
    map
}

/// 현재 lockfile 기준으로 논리 경로의 물리 경로(비활성 시 `.disabled`)를 구한다.
fn physical_path(lock: &Lockfile, logical: &str) -> String {
    let disabled = lock
        .managed_files
        .iter()
        .any(|mf| mf.path == logical && !mf.enabled);
    if disabled {
        format!("{logical}.disabled")
    } else {
        logical.to_string()
    }
}

/// Planning → Staging → Committing. 성공 시 새 lockfile 반환.
pub fn sync_instance(
    fetch: &dyn Fetch,
    resolver: &dyn SourceResolver,
    instance_root: &Path,
    fetched: &FetchedManifest,
    lock: &Lockfile,
    optional_selection: &BTreeMap<String, bool>,
    applied_at: &str,
) -> Result<SyncOutcome, SyncError> {
    let manifest = &fetched.manifest;
    let plan = plan_diff(manifest, lock, optional_selection);

    let needs_content = |a: &DiffAction| {
        matches!(
            a,
            DiffAction::Download { .. }
                | DiffAction::Update { .. }
                | DiffAction::UpdateKeepDisabled { .. }
        )
    };
    let has_removals = plan.iter().any(|a| matches!(a, DiffAction::Remove { .. }));
    if !plan.iter().any(needs_content) && !has_removals {
        return Ok(SyncOutcome {
            changed: false,
            lockfile: build_final_lockfile(
                manifest,
                lock,
                optional_selection,
                &fetched.hash_hex,
                applied_at,
            ),
        });
    }

    // ── Staging: 전부 .staging/<sync-id>/files/ 아래로 수신, 기존 파일 무접촉 ──
    let sync_id = &fetched.hash_hex[..12];
    let staging_rel = format!("{STAGING_DIR}/{sync_id}/files");
    let staging_root = safe_join(instance_root, &staging_rel)?;
    let sources = source_map(manifest);
    let mut failed: Vec<String> = Vec::new();
    let mut ops: Vec<Op> = Vec::new();

    let discard_staging = |instance_root: &Path, sync_id: &str| {
        let dir = instance_root.join(STAGING_DIR).join(sync_id);
        let _ = fs::remove_dir_all(dir);
    };

    for action in &plan {
        let (path, replaces, to_disabled) = match action {
            DiffAction::Download { path } => (path, &None, false),
            DiffAction::Update { path, replaces } => (path, replaces, false),
            DiffAction::UpdateKeepDisabled { path, replaces } => (path, replaces, true),
            DiffAction::Remove { path } => {
                ops.push(Op::Remove { path: physical_path(lock, path) });
                continue;
            }
            _ => continue,
        };
        let Some((source, sha256)) = sources.get(path) else {
            discard_staging(instance_root, sync_id);
            return Err(SyncError::UnsupportedSource(path.clone()));
        };
        let url = match resolver.resolve_url(source) {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!(path, %e, "source resolution failed");
                discard_staging(instance_root, sync_id);
                return Err(SyncError::UnsupportedSource(path.clone()));
            }
        };
        let dest = safe_join(&staging_root, path)?;
        match download_verified(fetch, &dest, &url, ExpectedHash::Sha256(sha256)) {
            Ok(_) => {
                if let Some(old) = replaces {
                    ops.push(Op::Remove { path: physical_path(lock, old) });
                }
                let to = if to_disabled { format!("{path}.disabled") } else { path.clone() };
                ops.push(Op::Move { from: format!("{staging_rel}/{path}"), to });
            }
            Err(DownloadError::Net(NetError::Policy(e))) => {
                discard_staging(instance_root, sync_id);
                return Err(SyncError::Fetch(NetError::Policy(e)));
            }
            Err(_) => failed.push(path.clone()),
        }
    }

    if !failed.is_empty() {
        // E-MF-04: 스테이징 폐기 — 기존 상태 그대로 (PRD 8.2.4-3)
        discard_staging(instance_root, sync_id);
        return Err(SyncError::Download { failed });
    }

    // ── Committing ──
    let final_lock = build_final_lockfile(
        manifest,
        lock,
        optional_selection,
        &fetched.hash_hex,
        applied_at,
    );
    let journal = build_journal(sync_id, &fetched.hash_hex, applied_at, ops, final_lock.clone(), lock)?;
    commit(instance_root, &journal)?;
    Ok(SyncOutcome { changed: true, lockfile: final_lock })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::test_support::MockFetch;
    use aqua_manifest::lockfile::{ManagedFile, Origin};

    const MANIFEST_URL: &str = "https://srv.example.com/manifest.json";

    fn manifest_json(mods: &[(&str, &str, &[u8])]) -> String {
        // (id, filename, body) — sha256은 본문에서 계산
        let mods_json: Vec<String> = mods
            .iter()
            .map(|(id, filename, body)| {
                format!(
                    r#"{{"id": "{id}", "filename": "{filename}", "sha256": "{sha}", "size_bytes": {n},
                        "source": {{"type": "url", "url": "https://srv.example.com/mods/{filename}"}}, "required": true}}"#,
                    sha = sha256_hex_of_bytes(body),
                    n = body.len()
                )
            })
            .collect();
        format!(
            r#"{{"format_version": 1, "min_launcher_version": "0.1.0", "display_version": "7",
                "server_display_name": "e2e", "minecraft_version": "1.20.4",
                "loader": {{"type": "fabric", "version": "0.15.7"}},
                "server": {{"address": "h", "port": 25565}},
                "mods": [{}]}}"#,
            mods_json.join(",")
        )
    }

    fn fetch_for(mods: &[(&str, &str, &[u8])]) -> MockFetch {
        let mjson = manifest_json(mods);
        let mut pairs: Vec<(String, Vec<u8>)> = vec![(MANIFEST_URL.into(), mjson.into_bytes())];
        for (_, filename, body) in mods {
            pairs.push((format!("https://srv.example.com/mods/{filename}"), body.to_vec()));
        }
        let mut f = MockFetch::with(&[]);
        f.responses = pairs.into_iter().collect();
        f
    }

    fn no_sel() -> BTreeMap<String, bool> {
        BTreeMap::new()
    }

    #[test]
    fn e2e_first_sync_then_update_then_noop() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // 사용자가 미리 넣어둔 파일 (lockfile 밖 — 이 테스트에서는 lockfile에 등록된 상태로 시작)
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::write(root.join("mods/mine.jar"), b"MINE").unwrap();
        let lock0 = Lockfile {
            applied_manifest_hash: None,
            applied_at: None,
            managed_files: vec![ManagedFile {
                path: "mods/mine.jar".into(),
                mod_id: None,
                sha256: sha256_hex_of_bytes(b"MINE"),
                origin: Origin::User,
                enabled: true,
            }],
        };

        // 1차 동기화: 모드 A 설치
        let f1 = fetch_for(&[("a", "a-1.0.jar", b"AAA v1")]);
        let m1 = fetch_manifest(&f1, MANIFEST_URL).unwrap();
        let out1 = sync_instance(&f1, &UrlOnlyResolver, root, &m1, &lock0, &no_sel(), "t1").unwrap();
        assert!(out1.changed);
        assert_eq!(fs::read(root.join("mods/a-1.0.jar")).unwrap(), b"AAA v1");
        assert!(root.join("mods/mine.jar").exists());
        assert!(!root.join(STAGING_DIR).exists() || fs::read_dir(root.join(STAGING_DIR)).unwrap().count() == 0);

        // 2차: 버전업(a-1.0 → a-2.0) — 구 파일 정리 + 신규 배치
        let f2 = fetch_for(&[("a", "a-2.0.jar", b"AAA v2")]);
        let m2 = fetch_manifest(&f2, MANIFEST_URL).unwrap();
        let out2 = sync_instance(&f2, &UrlOnlyResolver, root, &m2, &out1.lockfile, &no_sel(), "t2").unwrap();
        assert!(out2.changed);
        assert!(!root.join("mods/a-1.0.jar").exists(), "구 버전 정리");
        assert_eq!(fs::read(root.join("mods/a-2.0.jar")).unwrap(), b"AAA v2");
        assert!(root.join("mods/mine.jar").exists(), "사용자 파일 보존");

        // 3차: 동일 매니페스트 → no-op
        let out3 = sync_instance(&f2, &UrlOnlyResolver, root, &m2, &out2.lockfile, &no_sel(), "t3").unwrap();
        assert!(!out3.changed);

        // lockfile 파일도 기록되어 있어야 함 (커밋 산출물)
        let written: Lockfile = serde_json::from_str(
            &fs::read_to_string(root.join("manifest.lock.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(written.applied_manifest_hash.as_deref(), Some(m2.hash_hex.as_str()));
    }

    #[test]
    fn disabled_mod_updates_into_disabled_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("mods")).unwrap();
        fs::write(root.join("mods/a-1.0.jar.disabled"), b"AAA v1").unwrap();
        let lock = Lockfile {
            applied_manifest_hash: None,
            applied_at: None,
            managed_files: vec![ManagedFile {
                path: "mods/a-1.0.jar".into(),
                mod_id: Some("a".into()),
                sha256: sha256_hex_of_bytes(b"AAA v1"),
                origin: Origin::Manifest,
                enabled: false,
            }],
        };
        let f = fetch_for(&[("a", "a-2.0.jar", b"AAA v2")]);
        let m = fetch_manifest(&f, MANIFEST_URL).unwrap();
        let out = sync_instance(&f, &UrlOnlyResolver, root, &m, &lock, &no_sel(), "t").unwrap();
        assert!(out.changed);
        assert!(!root.join("mods/a-1.0.jar.disabled").exists(), "구 물리 경로 정리");
        assert!(root.join("mods/a-2.0.jar.disabled").exists(), "갱신 후에도 비활성 유지 (케이스4)");
        let entry = out.lockfile.managed_files.iter().find(|e| e.path == "mods/a-2.0.jar").unwrap();
        assert!(!entry.enabled);
    }

    #[test]
    fn download_failure_discards_staging_and_leaves_state_intact() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let f = {
            // 매니페스트만 서빙하고 모드 본문은 404
            let mjson = manifest_json(&[("a", "a-1.0.jar", b"AAA v1")]);
            MockFetch::with(&[(MANIFEST_URL, mjson.as_bytes())])
        };
        let m = fetch_manifest(&f, MANIFEST_URL).unwrap();
        let err = sync_instance(&f, &UrlOnlyResolver, root, &m, &Lockfile::default(), &no_sel(), "t").unwrap_err();
        match err {
            SyncError::Download { failed } => assert_eq!(failed, ["mods/a-1.0.jar"]),
            other => panic!("unexpected: {other:?}"),
        }
        assert!(!root.join("mods/a-1.0.jar").exists());
        assert!(!root.join(STAGING_DIR).join(&m.hash_hex[..12]).exists(), "스테이징 폐기");
        assert!(!root.join("manifest.lock.json").exists(), "커밋 미진입");
    }

    #[test]
    fn fetch_manifest_classifies_errors() {
        let bad_parse = MockFetch::with(&[(MANIFEST_URL, b"{not json".as_slice())]);
        assert!(matches!(fetch_manifest(&bad_parse, MANIFEST_URL), Err(SyncError::Parse(_))));

        let v99 = manifest_json(&[]).replace(r#""format_version": 1"#, r#""format_version": 99"#);
        let f = MockFetch::with(&[(MANIFEST_URL, v99.as_bytes())]);
        assert!(matches!(fetch_manifest(&f, MANIFEST_URL), Err(SyncError::UnsupportedFormat(99))));

        let old_mc = manifest_json(&[]).replace("1.20.4", "1.12.2");
        let f = MockFetch::with(&[(MANIFEST_URL, old_mc.as_bytes())]);
        assert!(matches!(fetch_manifest(&f, MANIFEST_URL), Err(SyncError::McTooOld(_))));

        let needs_new = manifest_json(&[]).replace(r#""min_launcher_version": "0.1.0""#, r#""min_launcher_version": "99.0.0""#);
        let f = MockFetch::with(&[(MANIFEST_URL, needs_new.as_bytes())]);
        assert!(matches!(fetch_manifest(&f, MANIFEST_URL), Err(SyncError::LauncherTooOld(_))));

        let gone = MockFetch::with(&[]);
        assert!(matches!(fetch_manifest(&gone, MANIFEST_URL), Err(SyncError::Fetch(_))));
    }

    #[test]
    fn online_resolver_passes_url_through_and_rejects_curseforge() {
        let f = MockFetch::with(&[]);
        let r = OnlineResolver { fetch: &f };
        let url = Source::Url { url: "https://srv.example.com/a.jar".into() };
        assert_eq!(r.resolve_url(&url).unwrap(), "https://srv.example.com/a.jar");
        let cf = Source::Curseforge { project_id: 1, file_id: 2 };
        assert!(r.resolve_url(&cf).is_err(), "CF는 API 키 승인(PRD 0-4) 전까지 미지원");
    }

    #[test]
    fn online_resolver_syncs_modrinth_source_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let body: &[u8] = b"MODRINTH MOD BODY";
        let mjson = format!(
            r#"{{"format_version": 1, "min_launcher_version": "0.1.0", "display_version": "1",
                "server_display_name": "t", "minecraft_version": "1.20.4",
                "loader": {{"type": "fabric", "version": "0.15.7"}},
                "server": {{"address": "h", "port": 25565}},
                "mods": [{{"id": "m", "filename": "m.jar", "sha256": "{sha}", "size_bytes": {n},
                          "source": {{"type": "modrinth", "project_id": "p", "version_id": "v1"}},
                          "required": true}}]}}"#,
            sha = sha256_hex_of_bytes(body),
            n = body.len()
        );
        let version_api = br#"{"id": "v1", "files": [
            {"url": "https://cdn.modrinth.com/data/p/m.jar", "filename": "m.jar",
             "primary": true, "hashes": {"sha1": "s"}, "size": 17}]}"#;
        let f = MockFetch::with(&[
            (MANIFEST_URL, mjson.as_bytes()),
            ("https://api.modrinth.com/v2/version/v1", version_api.as_slice()),
            ("https://cdn.modrinth.com/data/p/m.jar", body),
        ]);
        let m = fetch_manifest(&f, MANIFEST_URL).unwrap();
        let resolver = OnlineResolver { fetch: &f };
        let out = sync_instance(&f, &resolver, root, &m, &Lockfile::default(), &no_sel(), "t").unwrap();
        assert!(out.changed);
        assert_eq!(fs::read(root.join("mods/m.jar")).unwrap(), body);
    }

    /// M2 게이트 (PRD §13) — 픽스처 서버 상대 실 E2E 동기화.
    /// 사전: `cargo run -p aqua-fixtures`. 실행: cargo test -p aqua-launcher -- --ignored m2_gate --nocapture
    #[test]
    #[ignore = "픽스처 서버(127.0.0.1:8750) 필요 — M2 게이트 수동 검증용"]
    fn m2_gate_e2e_sync_against_fixture_server() {
        use crate::net::HttpFetcher;
        let fetch = HttpFetcher::new().unwrap();
        let m = fetch_manifest(&fetch, "http://127.0.0.1:8750/manifest.json").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // 옵셔널 모드는 가짜 modrinth ID(diff 테스트용 픽스처) → 선택 해제 경로로 검증
        let sel = BTreeMap::from([("dummy-optional".to_string(), false)]);
        let resolver = OnlineResolver { fetch: &fetch };
        let out = sync_instance(&fetch, &resolver, root, &m, &Lockfile::default(), &sel, "m2-gate").unwrap();
        assert!(out.changed);
        assert!(root.join("mods/dummy-required-1.0.jar").is_file());
        assert!(root.join("config/dummy.toml").is_file());
        assert!(!root.join("mods/dummy-optional-1.0.jar").exists(), "선택 해제 모드는 미설치");

        // 재동기화는 no-op — 커밋된 lockfile 기준
        let lock: Lockfile = serde_json::from_str(
            &fs::read_to_string(root.join("manifest.lock.json")).unwrap(),
        )
        .unwrap();
        let out2 = sync_instance(&fetch, &resolver, root, &m, &lock, &sel, "m2-gate-2").unwrap();
        assert!(!out2.changed);
    }

    #[test]
    fn modrinth_source_needing_download_is_rejected_for_now() {
        let dir = tempfile::tempdir().unwrap();
        let mjson = r#"{"format_version": 1, "min_launcher_version": "0.1.0", "display_version": "1",
            "server_display_name": "t", "minecraft_version": "1.20.4",
            "loader": {"type": "fabric", "version": "0.15.7"},
            "server": {"address": "h", "port": 25565},
            "mods": [{"id": "m", "filename": "m.jar", "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                      "size_bytes": 1, "source": {"type": "modrinth", "project_id": "p", "version_id": "v"}, "required": true}]}"#;
        let f = MockFetch::with(&[(MANIFEST_URL, mjson.as_bytes())]);
        let m = fetch_manifest(&f, MANIFEST_URL).unwrap();
        let err = sync_instance(&f, &UrlOnlyResolver, dir.path(), &m, &Lockfile::default(), &no_sel(), "t").unwrap_err();
        assert!(matches!(err, SyncError::UnsupportedSource(_)));
    }
}
