//! 인스턴스 저장소 — instance.json(PRD 7.2) 디스크 영속 + 프론트 뷰모델.
//!
//! 데이터 루트 레이아웃:
//! ```text
//! <data>/instances/<id>/instance.json      # PRD 7.2
//! <data>/instances/<id>/manifest.cached.json  # 마지막 적용 매니페스트 사본 (뷰/옵셔널 UI용)
//! <data>/instances/<id>/manifest.lock.json    # TD-02 §2
//! <data>/cache/                            # 공유 캐시 (PRD 7.1)
//! ```
use aqua_manifest::instance::{InstanceConfig, InstanceServer, InstallState};
use aqua_manifest::lockfile::{Lockfile, Origin};
use aqua_manifest::manifest::{LoaderKind, LoaderRef, Manifest};
use serde::Serialize;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const LOCKFILE_NAME: &str = "manifest.lock.json";
pub const INSTANCE_FILE: &str = "instance.json";
pub const CACHED_MANIFEST: &str = "manifest.cached.json";

#[derive(Debug, Clone)]
pub struct Paths {
    pub data_root: PathBuf,
}

impl Paths {
    pub fn instances_dir(&self) -> PathBuf {
        self.data_root.join("instances")
    }
    pub fn cache_dir(&self) -> PathBuf {
        self.data_root.join("cache")
    }
    pub fn instance_dir(&self, id: &str) -> PathBuf {
        self.instances_dir().join(id)
    }
}

fn now_unix() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp-write");
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(&tmp, path)
}

fn new_id() -> String {
    // 의존성 없는 유니크 id — 시각(나노) + 카운터
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}-{:x}", N.fetch_add(1, Ordering::Relaxed))
}

pub fn save_instance(paths: &Paths, cfg: &InstanceConfig) -> io::Result<()> {
    write_json_atomic(&paths.instance_dir(&cfg.id).join(INSTANCE_FILE), cfg)
}

pub fn load_lockfile(paths: &Paths, id: &str) -> Lockfile {
    fs::read_to_string(paths.instance_dir(id).join(LOCKFILE_NAME))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn load_cached_manifest(paths: &Paths, id: &str) -> Option<Manifest> {
    let raw = fs::read_to_string(paths.instance_dir(id).join(CACHED_MANIFEST)).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn save_cached_manifest(paths: &Paths, id: &str, manifest: &Manifest) -> io::Result<()> {
    write_json_atomic(&paths.instance_dir(id).join(CACHED_MANIFEST), manifest)
}

pub fn list_instances(paths: &Paths) -> Vec<InstanceConfig> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(paths.instances_dir()) else {
        return out;
    };
    for entry in entries.flatten() {
        let p = entry.path().join(INSTANCE_FILE);
        if let Ok(raw) = fs::read_to_string(&p) {
            match serde_json::from_str::<InstanceConfig>(&raw) {
                Ok(cfg) => out.push(cfg),
                Err(e) => tracing::warn!(path = %p.display(), %e, "skipping corrupt instance.json"),
            }
        }
    }
    // 최근 플레이 순 기본 정렬 (PRD 8.14)
    out.sort_by(|a, b| b.last_played.cmp(&a.last_played).then(a.name.cmp(&b.name)));
    out
}

pub fn get_instance(paths: &Paths, id: &str) -> Option<InstanceConfig> {
    let raw = fs::read_to_string(paths.instance_dir(id).join(INSTANCE_FILE)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// 수동 인스턴스 생성 — PRD 8.8. 모든 파일이 origin=user, 매니페스트 동기화 UI 없음.
pub fn create_manual(
    paths: &Paths,
    name: &str,
    mc_version: &str,
    loader: LoaderRef,
    icon_color: &str,
) -> io::Result<InstanceConfig> {
    let cfg = InstanceConfig {
        id: new_id(),
        name: name.to_string(),
        name_user_edited: true,
        minecraft_version: mc_version.to_string(),
        loader,
        manifest_source: None,
        manifest_hash: None,
        manifest_display_version: None,
        manifest_pinned: false,
        auto_update: true,
        last_synced_at: None,
        install_state: InstallState::Ready,
        server: None,
        jvm: None,
        jvm_args_override: None,
        java_version: None,
        java_path_override: None,
        last_account_id: None,
        optional_mods_selection: Default::default(),
        created_at: now_unix(),
        last_played: None,
        icon: Some(icon_color.to_string()),
    };
    fs::create_dir_all(paths.instance_dir(&cfg.id).join("mods"))?;
    save_instance(paths, &cfg)?;
    Ok(cfg)
}

/// 매니페스트 인스턴스 생성 — PRD 8.7/8.8. 파일 동기화는 이후 sync 단계가 수행.
pub fn create_from_manifest(
    paths: &Paths,
    manifest: &Manifest,
    manifest_url: &str,
) -> io::Result<InstanceConfig> {
    let cfg = InstanceConfig {
        id: new_id(),
        name: manifest.server_display_name.clone(),
        name_user_edited: false,
        minecraft_version: manifest.minecraft_version.clone(),
        loader: manifest.loader.clone(),
        manifest_source: Some(manifest_url.to_string()),
        manifest_hash: None, // 첫 동기화 성공 시 기록
        manifest_display_version: Some(manifest.display_version.clone()),
        manifest_pinned: false,
        auto_update: true,
        last_synced_at: None,
        install_state: InstallState::Installing,
        server: Some(InstanceServer {
            address: manifest.server.address.clone(),
            port: manifest.server.port,
            direct_connect_override: None,
        }),
        jvm: None,
        jvm_args_override: None,
        java_version: None,
        java_path_override: None,
        last_account_id: None,
        optional_mods_selection: manifest
            .mods
            .iter()
            .filter(|m| !m.required)
            .map(|m| (m.id.clone(), m.default_enabled))
            .collect(),
        created_at: now_unix(),
        last_played: None,
        icon: manifest.theme.as_ref().and_then(|t| t.primary_color.clone()),
    };
    fs::create_dir_all(paths.instance_dir(&cfg.id).join("mods"))?;
    save_instance(paths, &cfg)?;
    save_cached_manifest(paths, &cfg.id, manifest)?;
    Ok(cfg)
}

// ─────────────────── 프론트 뷰모델 (src/lib/types.ts 와 1:1) ───────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModItemVm {
    pub name: String,
    pub file: String,
    pub kind: &'static str, // "req" | "opt" | "user"
    pub enabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModGroupVm {
    pub key: &'static str, // "required" | "optional" | "user"
    pub items: Vec<ModItemVm>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceVm {
    pub id: String,
    pub name: String,
    pub initial: String,
    pub manual: bool,
    pub desc: String,
    pub loader_label: String,
    pub ver: String,
    pub clog: String,
    pub last: String,
    pub sync: String,
    pub disk: String,
    pub color: String,
    pub state: &'static str, // "ok" | "update" | "dirty" | "offline"
    pub play_size: Option<String>,
    pub domain: Option<String>,
    /// 딥링크 중복 감지용 (§8.7 — 동일 출처 인스턴스 존재 시 선택지 제공)
    pub manifest_source: Option<String>,
    pub order: i64,
    pub mods: Vec<ModGroupVm>,
}

fn loader_label(loader: &LoaderRef, mc: &str) -> String {
    let kind = match loader.kind {
        LoaderKind::Vanilla => "Vanilla",
        LoaderKind::Fabric => "Fabric",
        LoaderKind::Quilt => "Quilt",
        LoaderKind::Forge => "Forge",
        LoaderKind::Neoforge => "NeoForge",
    };
    format!("{kind} {mc}")
}

fn domain_of(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1)?;
    let host = rest.split(['/', '?', '#']).next()?;
    Some(host.to_string())
}

fn filename_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// InstanceConfig + lockfile + 캐시된 매니페스트 → 프론트 뷰모델.
pub fn build_vm(paths: &Paths, cfg: &InstanceConfig, order: i64) -> InstanceVm {
    let lock = load_lockfile(paths, &cfg.id);
    let manifest = load_cached_manifest(paths, &cfg.id);
    let manual = cfg.manifest_source.is_none();

    // 모드 그룹: 매니페스트 기준 required/optional 분류, lockfile 기준 user/enabled
    let mut required = Vec::new();
    let mut optional = Vec::new();
    let mut user = Vec::new();
    for mf in &lock.managed_files {
        if !mf.path.starts_with("mods/") {
            continue;
        }
        let file = filename_of(&mf.path).to_string();
        if mf.origin == Origin::User {
            user.push(ModItemVm { name: file.clone(), file, kind: "user", enabled: mf.enabled });
            continue;
        }
        let entry = manifest.as_ref().and_then(|m| {
            m.mods
                .iter()
                .find(|e| Some(&e.id) == mf.mod_id.as_ref() || e.filename == file)
        });
        let (name, is_required) = match entry {
            Some(e) => (e.id.clone(), e.required),
            None => (file.clone(), true),
        };
        let item = ModItemVm {
            name,
            file,
            kind: if is_required { "req" } else { "opt" },
            enabled: mf.enabled,
        };
        if is_required {
            required.push(item);
        } else {
            optional.push(item);
        }
    }

    // dirty: required 모드 비활성 존재 (PRD 8.2.8)
    let dirty = required.iter().any(|m| !m.enabled);
    let state = if dirty { "dirty" } else { "ok" };

    let desc = manifest
        .as_ref()
        .and_then(|m| m.server_description.clone())
        .unwrap_or_default();
    let clog = manifest
        .as_ref()
        .and_then(|m| m.changelog.clone())
        .unwrap_or_default();

    InstanceVm {
        id: cfg.id.clone(),
        initial: cfg.name.chars().next().map(|c| c.to_string()).unwrap_or_default(),
        name: cfg.name.clone(),
        manual,
        desc,
        loader_label: loader_label(&cfg.loader, &cfg.minecraft_version),
        ver: cfg.manifest_display_version.clone().unwrap_or_default(),
        clog,
        last: cfg.last_played.clone().unwrap_or_else(|| "—".into()),
        sync: cfg.last_synced_at.clone().unwrap_or_else(|| "—".into()),
        disk: "—".into(), // TODO(M4): 디렉토리 크기 계산 (설정 탭)
        color: cfg.icon.clone().unwrap_or_else(|| "#4a9b57".into()),
        state,
        play_size: None,
        domain: cfg.manifest_source.as_deref().and_then(domain_of),
        manifest_source: cfg.manifest_source.clone(),
        order,
        mods: vec![
            ModGroupVm { key: "required", items: required },
            ModGroupVm { key: "optional", items: optional },
            ModGroupVm { key: "user", items: user },
        ],
    }
}

/// 모드 토글 — PRD 8.2.8: `.jar` ↔ `.jar.disabled` rename + lockfile enabled 반영.
pub fn toggle_mod(paths: &Paths, id: &str, logical_path: &str, enabled: bool) -> io::Result<()> {
    let root = paths.instance_dir(id);
    let base = aqua_manifest::path::safe_join(&root, logical_path)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    let disabled_path = base.with_file_name(format!(
        "{}.disabled",
        base.file_name().unwrap_or_default().to_string_lossy()
    ));
    let (from, to) = if enabled { (disabled_path, base.clone()) } else { (base.clone(), disabled_path) };
    if from.is_file() {
        fs::rename(&from, &to)?;
    } else if !to.is_file() {
        return Err(io::Error::new(io::ErrorKind::NotFound, logical_path.to_string()));
    }

    let mut lock = load_lockfile(paths, id);
    for mf in &mut lock.managed_files {
        if mf.path == logical_path {
            mf.enabled = enabled;
        }
    }
    write_json_atomic(&root.join(LOCKFILE_NAME), &lock)
}

/// "매니페스트 상태로 초기화" — 관리(manifest) 모드 전부 활성화. 사용자 파일 무접촉 (PRD 8.2.8).
pub fn reset_to_manifest(paths: &Paths, id: &str) -> io::Result<()> {
    let lock = load_lockfile(paths, id);
    for mf in &lock.managed_files {
        if mf.origin == Origin::Manifest && !mf.enabled {
            toggle_mod(paths, id, &mf.path, true)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aqua_manifest::lockfile::ManagedFile;

    fn paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths { data_root: dir.path().to_path_buf() };
        (dir, p)
    }

    fn fabric() -> LoaderRef {
        LoaderRef { kind: LoaderKind::Fabric, version: "0.15.7".into() }
    }

    #[test]
    fn create_manual_roundtrips_via_list() {
        let (_d, p) = paths();
        let cfg = create_manual(&p, "내 야생", "1.20.4", fabric(), "#4d94c9").unwrap();
        let listed = list_instances(&p);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, cfg.id);
        assert!(listed[0].manifest_source.is_none());
        assert!(p.instance_dir(&cfg.id).join("mods").is_dir());
    }

    #[test]
    fn create_from_manifest_stores_cached_copy_and_selection() {
        let (_d, p) = paths();
        let raw = include_str!("../../../fixtures/data/manifest.json");
        let m: Manifest = serde_json::from_str(raw).unwrap();
        let cfg = create_from_manifest(&p, &m, "http://127.0.0.1:8750/manifest.json").unwrap();
        assert_eq!(cfg.name, m.server_display_name);
        assert_eq!(cfg.optional_mods_selection.get("dummy-optional"), Some(&true));
        assert!(load_cached_manifest(&p, &cfg.id).is_some());
        assert_eq!(cfg.install_state, InstallState::Installing);
    }

    #[test]
    fn vm_groups_mods_and_detects_dirty() {
        let (_d, p) = paths();
        let raw = include_str!("../../../fixtures/data/manifest.json");
        let m: Manifest = serde_json::from_str(raw).unwrap();
        let cfg = create_from_manifest(&p, &m, "http://127.0.0.1:8750/manifest.json").unwrap();
        let lock = Lockfile {
            applied_manifest_hash: Some("h".into()),
            applied_at: None,
            managed_files: vec![
                ManagedFile { path: "mods/dummy-required-1.0.jar".into(), mod_id: Some("dummy-required".into()), sha256: "x".into(), origin: Origin::Manifest, enabled: false },
                ManagedFile { path: "mods/dummy-optional-1.0.jar".into(), mod_id: Some("dummy-optional".into()), sha256: "x".into(), origin: Origin::Manifest, enabled: true },
                ManagedFile { path: "mods/mine.jar".into(), mod_id: None, sha256: "x".into(), origin: Origin::User, enabled: true },
                ManagedFile { path: "config/dummy.toml".into(), mod_id: None, sha256: "x".into(), origin: Origin::Manifest, enabled: true },
            ],
        };
        write_json_atomic(&p.instance_dir(&cfg.id).join(LOCKFILE_NAME), &lock).unwrap();

        let vm = build_vm(&p, &cfg, 0);
        assert!(!vm.manual);
        assert_eq!(vm.domain.as_deref(), Some("127.0.0.1:8750"));
        assert_eq!(vm.state, "dirty", "required 비활성 → dirty (PRD 8.2.8)");
        let group = |k: &str| vm.mods.iter().find(|g| g.key == k).unwrap();
        assert_eq!(group("required").items.len(), 1);
        assert!(!group("required").items[0].enabled);
        assert_eq!(group("optional").items.len(), 1);
        assert_eq!(group("user").items.len(), 1);
    }

    #[test]
    fn toggle_mod_renames_file_and_updates_lockfile() {
        let (_d, p) = paths();
        let cfg = create_manual(&p, "t", "1.20.4", fabric(), "#000000").unwrap();
        let jar = p.instance_dir(&cfg.id).join("mods/a.jar");
        fs::write(&jar, b"JAR").unwrap();
        let lock = Lockfile {
            applied_manifest_hash: None,
            applied_at: None,
            managed_files: vec![ManagedFile { path: "mods/a.jar".into(), mod_id: None, sha256: "x".into(), origin: Origin::User, enabled: true }],
        };
        write_json_atomic(&p.instance_dir(&cfg.id).join(LOCKFILE_NAME), &lock).unwrap();

        toggle_mod(&p, &cfg.id, "mods/a.jar", false).unwrap();
        assert!(!jar.exists());
        assert!(jar.with_file_name("a.jar.disabled").exists());
        assert!(!load_lockfile(&p, &cfg.id).managed_files[0].enabled);

        toggle_mod(&p, &cfg.id, "mods/a.jar", true).unwrap();
        assert!(jar.exists());
        assert!(load_lockfile(&p, &cfg.id).managed_files[0].enabled);
    }

    #[test]
    fn toggle_mod_rejects_path_escape() {
        let (_d, p) = paths();
        let cfg = create_manual(&p, "t", "1.20.4", fabric(), "#000000").unwrap();
        assert!(toggle_mod(&p, &cfg.id, "../evil.jar", false).is_err());
    }

    #[test]
    fn reset_reenables_only_manifest_mods() {
        let (_d, p) = paths();
        let cfg = create_manual(&p, "t", "1.20.4", fabric(), "#000000").unwrap();
        let dir = p.instance_dir(&cfg.id).join("mods");
        fs::write(dir.join("req.jar.disabled"), b"R").unwrap();
        fs::write(dir.join("user.jar.disabled"), b"U").unwrap();
        let lock = Lockfile {
            applied_manifest_hash: None,
            applied_at: None,
            managed_files: vec![
                ManagedFile { path: "mods/req.jar".into(), mod_id: Some("req".into()), sha256: "x".into(), origin: Origin::Manifest, enabled: false },
                ManagedFile { path: "mods/user.jar".into(), mod_id: None, sha256: "x".into(), origin: Origin::User, enabled: false },
            ],
        };
        write_json_atomic(&p.instance_dir(&cfg.id).join(LOCKFILE_NAME), &lock).unwrap();

        reset_to_manifest(&p, &cfg.id).unwrap();
        assert!(dir.join("req.jar").exists(), "manifest 모드는 재활성화");
        assert!(dir.join("user.jar.disabled").exists(), "사용자 모드는 무접촉 (PRD 8.2.8)");
    }
}
