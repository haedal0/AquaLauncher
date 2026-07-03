//! 동기화 엔진 — PRD 7.4 / 8.2, TD-02 (v1 승인 2026-07-03).
//!
//! diff 규칙 (PRD 8.2.3 표 + 7.4):
//! - Origin::User 파일은 어떤 경로로도 자동 삭제하지 않는다.
//! - 매칭 키 우선순위: mod_id -> 파일명 -> sha256.
//! - 동일 mod_id의 파일명 변경(버전업)은 "삭제+신규"가 아닌 Update.

pub mod commit;
pub mod engine;

use aqua_manifest::lockfile::{Lockfile, ManagedFile, Origin};
use aqua_manifest::manifest::{Manifest, SyncPolicy};
use std::collections::BTreeMap;

/// diff 계산 결과 액션 — PRD 8.2.3
///
/// `replaces`: 동일 mod_id 매칭으로 파일명이 바뀐 경우(버전업) 정리해야 할 구 논리 경로.
/// 매칭 상대가 origin=user면 항상 None — 사용자 파일은 어떤 경로로도 삭제하지 않는다.
#[derive(Debug, PartialEq, Eq)]
pub enum DiffAction {
    Download { path: String },
    Keep { path: String },
    Update { path: String, replaces: Option<String> },
    /// 파일은 최신으로 갱신하되 disabled 상태 유지 (v2 규칙 확정)
    UpdateKeepDisabled { path: String, replaces: Option<String> },
    SkipOptionalUnselected { mod_id: String },
    PreserveUserFile { path: String },
    Remove { path: String },
}

/// 매니페스트가 관리하는 대상 하나 (mods/files/resourcepack 공통 표현)
struct Target {
    path: String,
    mod_id: Option<String>,
    sha256: String,
    /// files[]의 sync_policy=once — 존재하면 해시가 달라도 Keep
    keep_if_present: bool,
}

/// 옵셔널 모드 설치 여부: 사용자가 선택했으면 그 값, 아니면 default_enabled (PRD 7.3).
fn optional_selected(
    selection: &BTreeMap<String, bool>,
    mod_id: &str,
    default_enabled: bool,
) -> bool {
    selection.get(mod_id).copied().unwrap_or(default_enabled)
}

fn targets_of(manifest: &Manifest, selection: &BTreeMap<String, bool>) -> (Vec<Target>, Vec<DiffAction>) {
    let mut targets = Vec::new();
    let mut skipped = Vec::new();
    for m in &manifest.mods {
        if !m.required && !optional_selected(selection, &m.id, m.default_enabled) {
            skipped.push(DiffAction::SkipOptionalUnselected { mod_id: m.id.clone() });
            continue;
        }
        targets.push(Target {
            path: format!("mods/{}", m.filename),
            mod_id: Some(m.id.clone()),
            sha256: m.sha256.clone(),
            keep_if_present: false,
        });
    }
    for f in &manifest.files {
        targets.push(Target {
            path: f.path.clone(),
            mod_id: None,
            sha256: f.sha256.clone(),
            keep_if_present: f.sync_policy == SyncPolicy::Once,
        });
    }
    if let Some(rp) = &manifest.resourcepack {
        targets.push(Target {
            path: format!("resourcepacks/{}", rp.filename),
            mod_id: None,
            sha256: rp.sha256.clone(),
            keep_if_present: false,
        });
    }
    (targets, skipped)
}

fn filename_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// 매칭 키 우선순위 mod_id -> 파일명 -> sha256 (PRD 7.4)
fn find_match(target: &Target, lock: &Lockfile, taken: &[bool]) -> Option<usize> {
    let by = |pred: &dyn Fn(&ManagedFile) -> bool| {
        lock.managed_files
            .iter()
            .enumerate()
            .find(|(i, mf)| !taken[*i] && pred(mf))
            .map(|(i, _)| i)
    };
    if let Some(id) = &target.mod_id {
        if let Some(i) = by(&|mf| mf.mod_id.as_deref() == Some(id)) {
            return Some(i);
        }
    }
    let fname = filename_of(&target.path);
    if let Some(i) = by(&|mf| filename_of(&mf.path) == fname) {
        return Some(i);
    }
    by(&|mf| mf.sha256 == target.sha256)
}

/// PRD 8.2.3 diff 표 구현. 순수 함수 — 파일시스템 접근 없음.
/// (lockfile에 없는 미지 파일의 origin=user 등록은 fs 스캔 계층에서 lockfile에 반영 후 호출)
pub fn plan_diff(
    manifest: &Manifest,
    lock: &Lockfile,
    optional_selection: &BTreeMap<String, bool>,
) -> Vec<DiffAction> {
    let (targets, mut actions) = targets_of(manifest, optional_selection);
    let mut taken = vec![false; lock.managed_files.len()];

    for t in &targets {
        match find_match(t, lock, &taken) {
            None => actions.push(DiffAction::Download { path: t.path.clone() }),
            Some(i) => {
                taken[i] = true;
                let local = &lock.managed_files[i];
                let replaces = (local.origin == Origin::Manifest && local.path != t.path)
                    .then(|| local.path.clone());
                if t.keep_if_present || local.sha256 == t.sha256 {
                    actions.push(DiffAction::Keep { path: t.path.clone() });
                } else if !local.enabled {
                    actions.push(DiffAction::UpdateKeepDisabled { path: t.path.clone(), replaces });
                } else {
                    actions.push(DiffAction::Update { path: t.path.clone(), replaces });
                }
            }
        }
    }

    for (i, local) in lock.managed_files.iter().enumerate() {
        if taken[i] {
            continue;
        }
        match local.origin {
            Origin::User => actions.push(DiffAction::PreserveUserFile { path: local.path.clone() }),
            Origin::Manifest => actions.push(DiffAction::Remove { path: local.path.clone() }),
        }
    }
    actions
}

/// §8.2.2 대용량 업데이트 확인 임계값 — 총 다운로드가 이 값 이상이면
/// 다운로드 시작 전 diff 요약 확인 다이얼로그, 미만이면 자동 진행.
pub const LARGE_UPDATE_BYTES: u64 = 200 * 1024 * 1024;

/// diff 요약 (§8.2.2): 추가 N / 갱신 M / 삭제 K + 총 다운로드 용량.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct DiffSummary {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub total_bytes: u64,
}

impl DiffSummary {
    pub fn requires_confirmation(&self) -> bool {
        self.total_bytes >= LARGE_UPDATE_BYTES
    }
}

/// plan_diff 결과를 사용자 확인용으로 요약. 용량은 매니페스트 선언값(size_bytes) 합산.
pub fn summarize_diff(actions: &[DiffAction], manifest: &Manifest) -> DiffSummary {
    let mut size_of: BTreeMap<String, u64> = BTreeMap::new();
    for m in &manifest.mods {
        size_of.insert(format!("mods/{}", m.filename), m.size_bytes);
    }
    for f in &manifest.files {
        size_of.insert(f.path.clone(), f.size_bytes);
    }
    if let Some(rp) = &manifest.resourcepack {
        size_of.insert(format!("resourcepacks/{}", rp.filename), rp.size_bytes);
    }

    let mut sum = DiffSummary { added: 0, updated: 0, removed: 0, total_bytes: 0 };
    for a in actions {
        match a {
            DiffAction::Download { path } => {
                sum.added += 1;
                sum.total_bytes += size_of.get(path.as_str()).copied().unwrap_or(0);
            }
            DiffAction::Update { path, .. } | DiffAction::UpdateKeepDisabled { path, .. } => {
                sum.updated += 1;
                sum.total_bytes += size_of.get(path.as_str()).copied().unwrap_or(0);
            }
            DiffAction::Remove { .. } => sum.removed += 1,
            DiffAction::Keep { .. }
            | DiffAction::SkipOptionalUnselected { .. }
            | DiffAction::PreserveUserFile { .. } => {}
        }
    }
    sum
}

/// 커밋 성공 시 기록할 lockfile 전문 — PRD 7.4 origin 규칙 구현.
///
/// 매칭된 기존 항목의 origin 승계:
/// - 해시 동일 → **origin 유지** (사용자 파일과 동일 모드 추가 엣지케이스: user 유지)
/// - 해시 상이 + 동일 경로 → 매니페스트 내용으로 덮어쓰므로 origin=manifest
/// - 해시 상이 + 경로 변경 + origin=user → 구 파일은 사용자 소유로 보존(별도 항목 유지),
///   새 경로는 origin=manifest 신규 항목
pub fn build_final_lockfile(
    manifest: &Manifest,
    lock: &Lockfile,
    optional_selection: &BTreeMap<String, bool>,
    applied_manifest_hash: &str,
    applied_at: &str,
) -> Lockfile {
    let (targets, _skipped) = targets_of(manifest, optional_selection);
    let mut taken = vec![false; lock.managed_files.len()];
    let mut managed = Vec::new();

    for t in &targets {
        let matched = find_match(t, lock, &taken);
        let entry = match matched {
            Some(i) => {
                taken[i] = true;
                let local = &lock.managed_files[i];
                let same_hash = local.sha256 == t.sha256 || t.keep_if_present;
                if local.origin == Origin::User {
                    if same_hash {
                        // 엣지케이스: origin=user 그대로 유지
                        local.clone()
                    } else if local.path == t.path {
                        ManagedFile {
                            path: t.path.clone(),
                            mod_id: t.mod_id.clone(),
                            sha256: t.sha256.clone(),
                            origin: Origin::Manifest,
                            enabled: local.enabled,
                        }
                    } else {
                        // 사용자 파일은 보존하고 새 경로를 별도 관리 항목으로 추가
                        managed.push(local.clone());
                        ManagedFile {
                            path: t.path.clone(),
                            mod_id: t.mod_id.clone(),
                            sha256: t.sha256.clone(),
                            origin: Origin::Manifest,
                            enabled: true,
                        }
                    }
                } else {
                    ManagedFile {
                        path: t.path.clone(),
                        mod_id: t.mod_id.clone(),
                        sha256: if t.keep_if_present { local.sha256.clone() } else { t.sha256.clone() },
                        origin: Origin::Manifest,
                        enabled: local.enabled,
                    }
                }
            }
            None => ManagedFile {
                path: t.path.clone(),
                mod_id: t.mod_id.clone(),
                sha256: t.sha256.clone(),
                origin: Origin::Manifest,
                enabled: true,
            },
        };
        managed.push(entry);
    }

    // 매니페스트에 없는 사용자 파일은 그대로 승계 (manifest 항목은 Remove 대상이므로 제외)
    for (i, local) in lock.managed_files.iter().enumerate() {
        if !taken[i] && local.origin == Origin::User {
            managed.push(local.clone());
        }
    }

    Lockfile {
        applied_manifest_hash: Some(applied_manifest_hash.to_string()),
        applied_at: Some(applied_at.to_string()),
        managed_files: managed,
    }
}

#[cfg(test)]
mod diff_checklist {
    //! PRD 8.2.3 표 — 케이스당 테스트 1개 (AGENT.md 완료 조건 1).
    use super::*;
    use aqua_manifest::manifest::*;

    fn manifest_with(mods: Vec<ModEntry>, files: Vec<FileEntry>) -> Manifest {
        Manifest {
            format_version: 1,
            min_launcher_version: "1.0.0".into(),
            display_version: "1".into(),
            changelog: None,
            server_display_name: "test".into(),
            server_description: None,
            minecraft_version: "1.20.4".into(),
            loader: LoaderRef { kind: LoaderKind::Fabric, version: "0.15.7".into() },
            server: ServerInfo { address: "play.example.com".into(), port: 25565, direct_connect_default: false },
            recommended_jvm: None,
            theme: None,
            mods,
            files,
            resourcepack: None,
        }
    }

    fn mod_entry(id: &str, filename: &str, sha: &str) -> ModEntry {
        ModEntry {
            id: id.into(),
            filename: filename.into(),
            sha256: sha.into(),
            size_bytes: 1,
            source: Source::Url { url: "https://example.com/m.jar".into() },
            required: true,
            optional_group: None,
            default_enabled: true,
            description: None,
        }
    }

    fn optional(mut m: ModEntry) -> ModEntry {
        m.required = false;
        m
    }

    fn lock_with(files: Vec<ManagedFile>) -> Lockfile {
        Lockfile { applied_manifest_hash: None, applied_at: None, managed_files: files }
    }

    fn managed(path: &str, mod_id: Option<&str>, sha: &str, origin: Origin, enabled: bool) -> ManagedFile {
        ManagedFile {
            path: path.into(),
            mod_id: mod_id.map(Into::into),
            sha256: sha.into(),
            origin,
            enabled,
        }
    }

    fn no_selection() -> BTreeMap<String, bool> {
        BTreeMap::new()
    }

    #[test]
    fn case1_manifest_has_local_missing_downloads() {
        let m = manifest_with(vec![mod_entry("sodium", "sodium-0.5.8.jar", "aaa")], vec![]);
        let acts = plan_diff(&m, &lock_with(vec![]), &no_selection());
        assert_eq!(acts, [DiffAction::Download { path: "mods/sodium-0.5.8.jar".into() }]);
    }

    #[test]
    fn case2_hash_match_keeps() {
        let m = manifest_with(vec![mod_entry("sodium", "sodium-0.5.8.jar", "aaa")], vec![]);
        let l = lock_with(vec![managed("mods/sodium-0.5.8.jar", Some("sodium"), "aaa", Origin::Manifest, true)]);
        let acts = plan_diff(&m, &l, &no_selection());
        assert_eq!(acts, [DiffAction::Keep { path: "mods/sodium-0.5.8.jar".into() }]);
    }

    #[test]
    fn case3_hash_mismatch_updates() {
        // 동일 mod_id, 파일명 변경(버전업) — 삭제+신규가 아닌 Update (PRD 7.4 매칭 키)
        let m = manifest_with(vec![mod_entry("sodium", "sodium-0.5.9.jar", "bbb")], vec![]);
        let l = lock_with(vec![managed("mods/sodium-0.5.8.jar", Some("sodium"), "aaa", Origin::Manifest, true)]);
        let acts = plan_diff(&m, &l, &no_selection());
        assert_eq!(
            acts,
            [DiffAction::Update {
                path: "mods/sodium-0.5.9.jar".into(),
                replaces: Some("mods/sodium-0.5.8.jar".into()),
            }]
        );
    }

    #[test]
    fn case4_disabled_updates_but_stays_disabled() {
        let m = manifest_with(vec![mod_entry("sodium", "sodium-0.5.9.jar", "bbb")], vec![]);
        let l = lock_with(vec![managed("mods/sodium-0.5.8.jar", Some("sodium"), "aaa", Origin::Manifest, false)]);
        let acts = plan_diff(&m, &l, &no_selection());
        assert_eq!(
            acts,
            [DiffAction::UpdateKeepDisabled {
                path: "mods/sodium-0.5.9.jar".into(),
                replaces: Some("mods/sodium-0.5.8.jar".into()),
            }]
        );
    }

    #[test]
    fn case5_optional_unselected_skips() {
        let mut sel = BTreeMap::new();
        sel.insert("minimap".to_string(), false);
        let m = manifest_with(vec![optional(mod_entry("minimap", "map-1.jar", "ccc"))], vec![]);
        let acts = plan_diff(&m, &lock_with(vec![]), &sel);
        assert_eq!(acts, [DiffAction::SkipOptionalUnselected { mod_id: "minimap".into() }]);
    }

    #[test]
    fn case6_user_origin_never_removed() {
        let m = manifest_with(vec![], vec![]);
        let l = lock_with(vec![managed("mods/my-custom.jar", None, "ddd", Origin::User, true)]);
        let acts = plan_diff(&m, &l, &no_selection());
        assert_eq!(acts, [DiffAction::PreserveUserFile { path: "mods/my-custom.jar".into() }]);
    }

    #[test]
    fn case7_manifest_origin_removed_when_gone() {
        let m = manifest_with(vec![], vec![]);
        let l = lock_with(vec![managed("mods/old-mod.jar", Some("old"), "eee", Origin::Manifest, true)]);
        let acts = plan_diff(&m, &l, &no_selection());
        assert_eq!(acts, [DiffAction::Remove { path: "mods/old-mod.jar".into() }]);
    }

    #[test]
    fn edge_user_file_stays_user_when_manifest_adds_same_hash() {
        // 사용자가 넣은 파일과 동일 해시의 모드가 매니페스트에 추가된 경우:
        // 매치되어 Keep (재다운로드 없음), origin은 user 유지 →
        // 이후 매니페스트에서 제거돼도 PreserveUserFile (PRD 7.4 엣지케이스)
        let user_file = managed("mods/fun.jar", None, "fff", Origin::User, true);

        let with_mod = manifest_with(vec![mod_entry("fun", "fun.jar", "fff")], vec![]);
        let acts = plan_diff(&with_mod, &lock_with(vec![user_file.clone()]), &no_selection());
        assert_eq!(acts, [DiffAction::Keep { path: "mods/fun.jar".into() }]);

        let without_mod = manifest_with(vec![], vec![]);
        let acts = plan_diff(&without_mod, &lock_with(vec![user_file]), &no_selection());
        assert_eq!(acts, [DiffAction::PreserveUserFile { path: "mods/fun.jar".into() }]);
    }

    #[test]
    fn edge_user_matched_update_never_replaces_user_file() {
        // 사용자 파일이 mod_id 없이 파일명/해시로 매칭돼 갱신 대상이 되어도,
        // 경로가 달라지는 경우 구 사용자 파일은 삭제 대상(replaces)에 오르지 않는다.
        let m = manifest_with(vec![mod_entry("fun", "fun-2.0.jar", "new")], vec![]);
        let l = lock_with(vec![managed("mods/fun-1.0.jar", None, "new", Origin::User, true)]);
        // 해시 매칭 → Keep (동일 해시)
        let acts = plan_diff(&m, &l, &no_selection());
        assert_eq!(acts, [DiffAction::Keep { path: "mods/fun-2.0.jar".into() }]);

        let l2 = lock_with(vec![managed("mods/fun-1.0.jar", None, "old", Origin::User, true)]);
        let m2 = manifest_with(vec![mod_entry("fun", "fun-1.0.jar", "new")], vec![]);
        // 파일명 매칭 + 해시 불일치 → 같은 경로 덮어쓰기 Update, replaces 없음
        let acts = plan_diff(&m2, &l2, &no_selection());
        assert_eq!(
            acts,
            [DiffAction::Update { path: "mods/fun-1.0.jar".into(), replaces: None }]
        );
    }

    #[test]
    fn final_lockfile_applies_origin_rules() {
        // user + 동일 해시 → user 유지 / manifest 항목 갱신 / 미매칭 user 승계 / 제거 대상 제외
        let m = manifest_with(
            vec![mod_entry("same", "same.jar", "aaa"), mod_entry("upd", "upd-2.jar", "new")],
            vec![],
        );
        let l = lock_with(vec![
            managed("mods/same.jar", None, "aaa", Origin::User, true),
            managed("mods/upd-1.jar", Some("upd"), "old", Origin::Manifest, false),
            managed("mods/mine.jar", None, "zzz", Origin::User, true),
            managed("mods/gone.jar", Some("gone"), "ggg", Origin::Manifest, true),
        ]);
        let out = build_final_lockfile(&m, &l, &no_selection(), "sha256:h", "2026-07-03T00:00:00Z");
        assert_eq!(out.applied_manifest_hash.as_deref(), Some("sha256:h"));

        let by_path = |p: &str| out.managed_files.iter().find(|f| f.path == p);
        let same = by_path("mods/same.jar").unwrap();
        assert_eq!(same.origin, Origin::User, "동일 해시 매칭은 user 유지");
        let upd = by_path("mods/upd-2.jar").unwrap();
        assert_eq!(upd.origin, Origin::Manifest);
        assert!(!upd.enabled, "disabled 상태 승계 (PRD 8.2.3 케이스4)");
        assert_eq!(upd.sha256, "new");
        assert!(by_path("mods/mine.jar").is_some(), "미매칭 user 승계");
        assert!(by_path("mods/gone.jar").is_none(), "제거 대상은 lockfile에서 제외");
        assert!(by_path("mods/upd-1.jar").is_none(), "구 경로 제외");
    }

    #[test]
    fn files_sync_policy_once_keeps_existing_even_on_hash_mismatch() {
        // PRD 7.3: once는 최초 설치 후 사용자 소유 — 해시가 달라도 덮어쓰지 않는다
        let f = FileEntry {
            path: "options.txt".into(),
            sha256: "new-hash".into(),
            size_bytes: 1,
            source: Source::Url { url: "https://example.com/options.txt".into() },
            sync_policy: SyncPolicy::Once,
        };
        let m = manifest_with(vec![], vec![f]);
        let l = lock_with(vec![managed("options.txt", None, "old-hash", Origin::Manifest, true)]);
        let acts = plan_diff(&m, &l, &no_selection());
        assert_eq!(acts, [DiffAction::Keep { path: "options.txt".into() }]);
    }

    #[test]
    fn optional_default_enabled_installs_without_explicit_selection() {
        let m = manifest_with(vec![optional(mod_entry("minimap", "map-1.jar", "ccc"))], vec![]);
        let acts = plan_diff(&m, &lock_with(vec![]), &no_selection());
        assert_eq!(acts, [DiffAction::Download { path: "mods/map-1.jar".into() }]);
    }

    /// §8.2.2: 확인 다이얼로그용 요약 — 추가/갱신/삭제 카운트 + 다운로드 대상만 용량 합산.
    #[test]
    fn summary_counts_and_bytes_for_confirmation_dialog() {
        let mut new_mod = mod_entry("create", "create-0.6.jar", "new1");
        new_mod.size_bytes = 100;
        let mut upd_mod = mod_entry("sodium", "sodium-0.6.jar", "new2");
        upd_mod.size_bytes = 50;
        let mut kept = mod_entry("jei", "jei-15.jar", "same");
        kept.size_bytes = 999; // Keep은 다운로드 없음 — 합산 제외 검증
        let m = manifest_with(vec![new_mod, upd_mod, kept], vec![]);
        let l = lock_with(vec![
            managed("mods/sodium-0.5.jar", Some("sodium"), "old2", Origin::Manifest, true),
            managed("mods/jei-15.jar", Some("jei"), "same", Origin::Manifest, true),
            managed("mods/gone.jar", Some("gone"), "zzz", Origin::Manifest, true),
        ]);
        let acts = plan_diff(&m, &l, &no_selection());
        let s = summarize_diff(&acts, &m);
        assert_eq!(s, DiffSummary { added: 1, updated: 1, removed: 1, total_bytes: 150 });
    }

    #[test]
    fn large_update_threshold_is_200mb() {
        let at = DiffSummary { added: 0, updated: 0, removed: 0, total_bytes: LARGE_UPDATE_BYTES };
        let below =
            DiffSummary { added: 0, updated: 0, removed: 0, total_bytes: LARGE_UPDATE_BYTES - 1 };
        assert!(at.requires_confirmation());
        assert!(!below.requires_confirmation());
    }
}
