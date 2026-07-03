//! `aqua-cli scan` — PRD 10.1: mods/, config/ 스캔해 해시/용량 자동 계산.
//! 원칙: 운영자는 해시/용량을 손으로 계산할 일이 없어야 한다.
use aqua_manifest::manifest::{FileEntry, Manifest, ModEntry, Source, SyncPolicy};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const PLACEHOLDER_BASE: &str = "https://CHANGE-ME.example.com";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub updated: usize,
    pub added: usize,
    pub removed: usize,
}

fn sha256_hex(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher)?;
    let mut out = String::with_capacity(64);
    for b in hasher.finalize() {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    Ok(out)
}

fn mod_source_url(base_url: &str, filename: &str) -> String {
    format!("{}/mods/{filename}", base_url.trim_end_matches('/'))
}

fn file_source_url(base_url: &str, rel_path: &str) -> String {
    format!("{}/{rel_path}", base_url.trim_end_matches('/'))
}

/// mods/*.jar 목록 (파일명 정렬, .disabled 등 비 jar는 무시)
fn jar_files(mods_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if mods_dir.is_dir() {
        for entry in fs::read_dir(mods_dir)? {
            let p = entry?.path();
            if p.is_file() && p.extension().is_some_and(|e| e == "jar") {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// config/ 이하 전체 파일의 인스턴스 루트 기준 상대경로 ('/' 구분자, 정렬)
fn config_files(root: &Path) -> io::Result<Vec<String>> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let p = entry?.path();
            if p.is_dir() {
                walk(&p, root, out)?;
            } else if p.is_file() {
                let rel = p
                    .strip_prefix(root)
                    .expect("under root")
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                out.push(rel);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    let config_dir = root.join("config");
    if config_dir.is_dir() {
        walk(&config_dir, root, &mut out)?;
    }
    out.sort();
    Ok(out)
}

/// instance_dir의 실물 기준으로 manifest의 mods/files 섹션을 갱신한다.
/// - 기존 항목: 해시/용량 갱신 (source/옵션 필드 유지)
/// - 새 파일: 항목 추가 (base_url 없으면 CHANGE-ME 플레이스홀더)
/// - 실물이 사라진 항목: 제거
pub fn scan(instance_dir: &Path, manifest: &mut Manifest, base_url: Option<&str>) -> io::Result<ScanReport> {
    let base = base_url.unwrap_or(PLACEHOLDER_BASE);
    let mut report = ScanReport::default();

    // ── mods ──
    let jars = jar_files(&instance_dir.join("mods"))?;
    let mut new_mods: Vec<ModEntry> = Vec::new();
    for jar in &jars {
        let filename = jar.file_name().unwrap().to_string_lossy().into_owned();
        let sha256 = sha256_hex(jar)?;
        let size_bytes = fs::metadata(jar)?.len();
        match manifest.mods.iter_mut().find(|m| m.filename == filename) {
            Some(entry) => {
                if entry.sha256 != sha256 || entry.size_bytes != size_bytes {
                    entry.sha256 = sha256;
                    entry.size_bytes = size_bytes;
                    report.updated += 1;
                }
            }
            None => {
                let stem = filename.trim_end_matches(".jar").to_string();
                new_mods.push(ModEntry {
                    id: stem,
                    filename: filename.clone(),
                    sha256,
                    size_bytes,
                    source: Source::Url { url: mod_source_url(base, &filename) },
                    required: true,
                    optional_group: None,
                    default_enabled: true,
                    description: None,
                });
                report.added += 1;
            }
        }
    }
    let before = manifest.mods.len();
    let present: Vec<String> = jars
        .iter()
        .map(|j| j.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    // 제거는 운영자 직접 호스팅(url) 항목만 — modrinth/curseforge 참조 모드는
    // 로컬에 실물이 없는 게 정상이므로 스캔이 건드리지 않는다.
    manifest.mods.retain(|m| {
        !matches!(m.source, Source::Url { .. }) || present.contains(&m.filename)
    });
    report.removed += before - manifest.mods.len();
    manifest.mods.extend(new_mods);

    // ── files (config/) ──
    let configs = config_files(instance_dir)?;
    let mut new_files: Vec<FileEntry> = Vec::new();
    for rel in &configs {
        let abs = aqua_manifest::path::safe_join(instance_dir, rel)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
        let sha256 = sha256_hex(&abs)?;
        let size_bytes = fs::metadata(&abs)?.len();
        match manifest.files.iter_mut().find(|f| &f.path == rel) {
            Some(entry) => {
                if entry.sha256 != sha256 || entry.size_bytes != size_bytes {
                    entry.sha256 = sha256;
                    entry.size_bytes = size_bytes;
                    report.updated += 1;
                }
            }
            None => {
                new_files.push(FileEntry {
                    path: rel.clone(),
                    sha256,
                    size_bytes,
                    source: Source::Url { url: file_source_url(base, rel) },
                    sync_policy: SyncPolicy::Always,
                });
                report.added += 1;
            }
        }
    }
    let before = manifest.files.len();
    manifest
        .files
        .retain(|f| !f.path.starts_with("config/") || configs.contains(&f.path));
    report.removed += before - manifest.files.len();
    manifest.files.extend(new_files);

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_manifest() -> Manifest {
        serde_json::from_str(
            r#"{
              "format_version": 1, "min_launcher_version": "1.0.0", "display_version": "1",
              "server_display_name": "t", "minecraft_version": "1.20.4",
              "loader": {"type": "fabric", "version": "0.15.7"},
              "server": {"address": "h", "port": 25565},
              "mods": [
                {"id": "keepme", "filename": "keepme-1.0.jar", "sha256": "OLD", "size_bytes": 0,
                 "source": {"type": "url", "url": "https://srv/mods/keepme-1.0.jar"}, "required": true},
                {"id": "gone", "filename": "gone-1.0.jar", "sha256": "x", "size_bytes": 0,
                 "source": {"type": "url", "url": "https://srv/mods/gone-1.0.jar"}, "required": true}
              ],
              "files": [
                {"path": "config/keep.toml", "sha256": "OLD", "size_bytes": 0,
                 "source": {"type": "url", "url": "https://srv/config/keep.toml"}, "sync_policy": "once"},
                {"path": "options.txt", "sha256": "not-config-untouched", "size_bytes": 0,
                 "source": {"type": "url", "url": "https://srv/options.txt"}, "sync_policy": "once"}
              ]
            }"#,
        )
        .unwrap()
    }

    fn setup_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("mods")).unwrap();
        fs::create_dir_all(dir.path().join("config/sub")).unwrap();
        fs::write(dir.path().join("mods/keepme-1.0.jar"), b"KEEP").unwrap();
        fs::write(dir.path().join("mods/newmod-2.0.jar"), b"NEW").unwrap();
        fs::write(dir.path().join("mods/notes.txt"), b"ignore me").unwrap();
        fs::write(dir.path().join("config/keep.toml"), b"CFG").unwrap();
        fs::write(dir.path().join("config/sub/extra.json"), b"{}").unwrap();
        dir
    }

    #[test]
    fn scan_updates_adds_and_removes() {
        let dir = setup_dir();
        let mut m = base_manifest();
        let report = scan(dir.path(), &mut m, Some("https://files.myserver.com/")).unwrap();

        // updated: keepme(해시 갱신) + keep.toml / added: newmod + sub/extra.json / removed: gone
        assert_eq!(report, ScanReport { updated: 2, added: 2, removed: 1 });

        let keep = m.mods.iter().find(|x| x.id == "keepme").unwrap();
        assert_eq!(keep.sha256.len(), 64);
        assert_eq!(keep.size_bytes, 4);
        assert!(m.mods.iter().all(|x| x.id != "gone"), "실물 없는 항목 제거");

        let new = m.mods.iter().find(|x| x.filename == "newmod-2.0.jar").unwrap();
        assert_eq!(new.id, "newmod-2.0");
        assert!(matches!(&new.source, Source::Url { url } if url == "https://files.myserver.com/mods/newmod-2.0.jar"));

        let cfg = m.files.iter().find(|f| f.path == "config/keep.toml").unwrap();
        assert_eq!(cfg.sync_policy, SyncPolicy::Once, "기존 sync_policy 유지");
        assert_eq!(cfg.size_bytes, 3);
        assert!(m.files.iter().any(|f| f.path == "config/sub/extra.json"));
        assert!(
            m.files.iter().any(|f| f.path == "options.txt"),
            "config/ 밖 항목은 스캔이 건드리지 않음"
        );
    }

    #[test]
    fn scan_without_base_url_uses_placeholder() {
        let dir = setup_dir();
        let mut m = base_manifest();
        scan(dir.path(), &mut m, None).unwrap();
        let new = m.mods.iter().find(|x| x.filename == "newmod-2.0.jar").unwrap();
        assert!(matches!(&new.source, Source::Url { url } if url.starts_with(PLACEHOLDER_BASE)));
    }

    #[test]
    fn scan_keeps_modrinth_entries_without_local_file() {
        let dir = setup_dir();
        let mut m = base_manifest();
        m.mods.push(
            serde_json::from_str(
                r#"{"id": "remote-only", "filename": "remote-only-1.0.jar", "sha256": "r", "size_bytes": 0,
                    "source": {"type": "modrinth", "project_id": "p", "version_id": "v"}, "required": false}"#,
            )
            .unwrap(),
        );
        scan(dir.path(), &mut m, None).unwrap();
        assert!(
            m.mods.iter().any(|x| x.id == "remote-only"),
            "modrinth 참조 모드는 로컬 실물이 없어도 유지"
        );
    }

    #[test]
    fn rescan_is_idempotent() {
        let dir = setup_dir();
        let mut m = base_manifest();
        scan(dir.path(), &mut m, None).unwrap();
        let report = scan(dir.path(), &mut m, None).unwrap();
        assert_eq!(report, ScanReport::default(), "변경 없으면 no-op");
    }
}
