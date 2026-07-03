//! `aqua-cli validate` — PRD 10.1: 스키마/경로/URL 정책/중복 검증.
//! (URL HEAD 체크 + CF opt-out 검사는 네트워크 필요 — M2 후속, --check-urls로 추가 예정)
use aqua_manifest::manifest::{Manifest, Source, SUPPORTED_FORMAT_VERSION};
use aqua_manifest::path::safe_join;
use aqua_manifest::urlpolicy::validate_distribution_url;
use std::collections::BTreeSet;
use std::path::Path;

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_hex_color(s: &str) -> bool {
    s.len() == 7
        && s.starts_with('#')
        && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

fn mc_at_least_1_13(v: &str) -> bool {
    let mut it = v.split('.');
    let (Some(maj), Some(min)) = (it.next(), it.next()) else { return false };
    match (maj.parse::<u32>(), min.parse::<u32>()) {
        (Ok(maj), Ok(min)) => (maj, min) >= (1, 13),
        _ => false,
    }
}

fn check_source(issues: &mut Vec<String>, what: &str, source: &Source) {
    if let Source::Url { url } = source {
        if let Err(e) = validate_distribution_url(url) {
            issues.push(format!("{what}: {e}"));
        }
    }
}

/// 오프라인 검증 — 문제 목록을 반환한다 (비어 있으면 통과).
pub fn validate(manifest: &Manifest) -> Vec<String> {
    let mut issues = Vec::new();

    if manifest.format_version != SUPPORTED_FORMAT_VERSION {
        issues.push(format!(
            "format_version {}는 지원 대상({SUPPORTED_FORMAT_VERSION})이 아닙니다",
            manifest.format_version
        ));
    }
    if !mc_at_least_1_13(&manifest.minecraft_version) {
        issues.push(format!(
            "minecraft_version '{}' — 1.13 이상 릴리스만 지원 (PRD §5)",
            manifest.minecraft_version
        ));
    }

    let dummy_root = Path::new("/validate-root");
    let mut ids = BTreeSet::new();
    let mut filenames = BTreeSet::new();
    for m in &manifest.mods {
        if !ids.insert(&m.id) {
            issues.push(format!("mods: 중복 id '{}'", m.id));
        }
        if !filenames.insert(&m.filename) {
            issues.push(format!("mods: 중복 filename '{}'", m.filename));
        }
        if m.filename.contains('/') || m.filename.contains('\\') {
            issues.push(format!("mods '{}': filename에 경로 구분자 금지", m.id));
        }
        if !is_sha256_hex(&m.sha256) {
            issues.push(format!("mods '{}': sha256 형식 오류 (소문자 hex 64자)", m.id));
        }
        check_source(&mut issues, &format!("mods '{}'", m.id), &m.source);
        if m.required && m.optional_group.is_some() {
            issues.push(format!(
                "mods '{}': required=true인데 optional_group 지정 — 옵셔널 여부 확인 필요",
                m.id
            ));
        }
    }

    let mut file_paths = BTreeSet::new();
    for f in &manifest.files {
        if !file_paths.insert(&f.path) {
            issues.push(format!("files: 중복 path '{}'", f.path));
        }
        if safe_join(dummy_root, &f.path).is_err() {
            issues.push(format!("files '{}': 경로 탈출/절대경로 금지 (PRD §7.3)", f.path));
        }
        if !is_sha256_hex(&f.sha256) {
            issues.push(format!("files '{}': sha256 형식 오류", f.path));
        }
        check_source(&mut issues, &format!("files '{}'", f.path), &f.source);
    }

    if let Some(rp) = &manifest.resourcepack {
        if !is_sha256_hex(&rp.sha256) {
            issues.push("resourcepack: sha256 형식 오류".into());
        }
        check_source(&mut issues, "resourcepack", &rp.source);
    }

    if let Some(theme) = &manifest.theme {
        for (name, color) in [
            ("primary_color", &theme.primary_color),
            ("accent_color", &theme.accent_color),
        ] {
            if let Some(c) = color {
                if !is_hex_color(c) {
                    issues.push(format!("theme.{name}: '#rrggbb' 형식이어야 함 (PRD §8.5)"));
                }
            }
        }
        for (name, url) in [
            ("background_image", &theme.background_image),
            ("logo", &theme.logo),
        ] {
            if let Some(u) = url {
                if let Err(e) = validate_distribution_url(u) {
                    issues.push(format!("theme.{name}: {e}"));
                }
            }
        }
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(json: &str) -> Manifest {
        serde_json::from_str(json).unwrap()
    }

    const GOOD_SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn base(mods_json: &str, extra: &str) -> Manifest {
        manifest(&format!(
            r#"{{
              "format_version": 1, "min_launcher_version": "1.0.0", "display_version": "1",
              "server_display_name": "t", "minecraft_version": "1.20.4",
              "loader": {{"type": "fabric", "version": "0.15.7"}},
              "server": {{"address": "h", "port": 25565}},
              "mods": [{mods_json}]{extra}
            }}"#
        ))
    }

    fn mod_json(id: &str, sha: &str, url: &str) -> String {
        format!(
            r#"{{"id": "{id}", "filename": "{id}.jar", "sha256": "{sha}", "size_bytes": 1,
                "source": {{"type": "url", "url": "{url}"}}, "required": true}}"#
        )
    }

    #[test]
    fn clean_manifest_passes() {
        let m = base(&mod_json("a", GOOD_SHA, "https://srv/mods/a.jar"), "");
        assert!(validate(&m).is_empty());
    }

    #[test]
    fn fixture_manifest_passes() {
        // 저장소 픽스처는 항상 validate를 통과해야 한다 (루프백 http 예외 포함)
        let raw = include_str!("../../fixtures/data/manifest.json");
        let m: Manifest = serde_json::from_str(raw).unwrap();
        assert_eq!(validate(&m), Vec::<String>::new());
    }

    #[test]
    fn catches_duplicate_ids_and_bad_hash() {
        let mods = format!(
            "{},{}",
            mod_json("dup", GOOD_SHA, "https://srv/a.jar"),
            mod_json("dup", "NOT-A-HASH", "https://srv/b.jar")
        );
        let issues = validate(&base(&mods, ""));
        assert!(issues.iter().any(|i| i.contains("중복 id")));
        assert!(issues.iter().any(|i| i.contains("sha256 형식 오류")));
    }

    #[test]
    fn catches_http_url_and_path_escape() {
        let extra = format!(
            r#", "files": [{{"path": "../evil.toml", "sha256": "{GOOD_SHA}", "size_bytes": 1,
                "source": {{"type": "url", "url": "http://evil.com/x"}}, "sync_policy": "always"}}]"#
        );
        let issues = validate(&base(&mod_json("a", GOOD_SHA, "https://srv/a.jar"), &extra));
        assert!(issues.iter().any(|i| i.contains("경로 탈출")));
        assert!(issues.iter().any(|i| i.contains("only https")));
    }

    #[test]
    fn catches_unsupported_format_and_old_mc() {
        let mut m = base(&mod_json("a", GOOD_SHA, "https://srv/a.jar"), "");
        m.format_version = 99;
        m.minecraft_version = "1.12.2".into();
        let issues = validate(&m);
        assert!(issues.iter().any(|i| i.contains("format_version")));
        assert!(issues.iter().any(|i| i.contains("1.13 이상")));
    }

    #[test]
    fn catches_bad_theme_color() {
        let extra = r#", "theme": {"primary_color": "green"}"#;
        let issues = validate(&base(&mod_json("a", GOOD_SHA, "https://srv/a.jar"), extra));
        assert!(issues.iter().any(|i| i.contains("theme.primary_color")));
    }
}
