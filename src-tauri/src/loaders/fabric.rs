//! Fabric 로더 — PRD 8.1 (메타 API 기반, 우선 구현으로 인터페이스 검증).
//! Quilt는 동일 구조의 메타 API를 쓰므로 이 구현을 파라미터화해 재사용한다.
use super::{InstallContext, LaunchProfile, LoaderVersion, ModLoader};
use crate::error::AppError;
use crate::launch::version_json::VersionJson;
use crate::net::Fetch;
use aqua_manifest::path::safe_join;
use serde::Deserialize;
use std::fs;

pub const FABRIC_META: &str = "https://meta.fabricmc.net/v2";
pub const QUILT_META: &str = "https://meta.quiltmc.org/v3";

#[derive(Debug, Deserialize)]
struct LoaderListEntry {
    loader: LoaderInfo,
}

#[derive(Debug, Deserialize)]
struct LoaderInfo {
    version: String,
    #[serde(default)]
    stable: bool,
}

pub struct FabricLikeLoader<'a> {
    id: &'static str,
    meta_base: &'a str,
    fetch: &'a dyn Fetch,
}

impl<'a> FabricLikeLoader<'a> {
    pub fn fabric(fetch: &'a dyn Fetch) -> Self {
        FabricLikeLoader { id: "fabric", meta_base: FABRIC_META, fetch }
    }
    pub fn quilt(fetch: &'a dyn Fetch) -> Self {
        FabricLikeLoader { id: "quilt", meta_base: QUILT_META, fetch }
    }

    fn err(e: impl std::fmt::Display) -> AppError {
        AppError::LoaderInstall(e.to_string())
    }
}

impl ModLoader for FabricLikeLoader<'_> {
    fn id(&self) -> &'static str {
        self.id
    }

    fn list_versions(&self, mc_version: &str) -> Result<Vec<LoaderVersion>, AppError> {
        let url = format!("{}/versions/loader/{mc_version}", self.meta_base);
        let raw = self.fetch.get_bytes(&url).map_err(Self::err)?;
        let entries: Vec<LoaderListEntry> = serde_json::from_slice(&raw).map_err(Self::err)?;
        Ok(entries
            .into_iter()
            .map(|e| LoaderVersion { id: e.loader.version, stable: e.loader.stable })
            .collect())
    }

    /// profile JSON을 받아 공유 캐시 `versions/<id>/<id>.json`에 저장 (불변: 있으면 재사용).
    fn install(
        &self,
        mc_version: &str,
        loader_version: &str,
        ctx: &InstallContext,
    ) -> Result<LaunchProfile, AppError> {
        let url = format!(
            "{}/versions/loader/{mc_version}/{loader_version}/profile/json",
            self.meta_base
        );
        let raw = self.fetch.get_bytes(&url).map_err(Self::err)?;
        // id 추출 겸 스키마 검증 — 원문 그대로 저장한다
        let parsed: VersionJson = serde_json::from_slice(&raw).map_err(Self::err)?;
        // 메타 응답의 id도 외부 입력(§11) — 캐시 경로 조합은 safe_join으로만 (§7.3)
        let dest = safe_join(
            ctx.shared_cache_root,
            &format!("versions/{id}/{id}.json", id = parsed.id),
        )
        .map_err(Self::err)?;
        if !dest.is_file() {
            let dir = dest.parent().expect("safe_join yields nested path");
            fs::create_dir_all(dir).map_err(Self::err)?;
            let tmp = dest.with_extension("json.tmp-write");
            fs::write(&tmp, &raw).map_err(Self::err)?;
            fs::rename(&tmp, &dest).map_err(Self::err)?;
        }
        Ok(LaunchProfile { version_json_path: dest })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::test_support::MockFetch;

    const LIST_URL: &str = "https://meta.fabricmc.net/v2/versions/loader/1.20.4";
    const PROFILE_URL: &str =
        "https://meta.fabricmc.net/v2/versions/loader/1.20.4/0.15.7/profile/json";

    fn fetch() -> MockFetch {
        MockFetch::with(&[
            (
                LIST_URL,
                br#"[{"loader": {"version": "0.15.7", "stable": true}},
                     {"loader": {"version": "0.16.0-beta.1", "stable": false}}]"#
                    .as_slice(),
            ),
            (
                PROFILE_URL,
                br#"{"id": "fabric-loader-0.15.7-1.20.4", "inheritsFrom": "1.20.4",
                     "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
                     "libraries": [{"name": "net.fabricmc:fabric-loader:0.15.7"}]}"#
                    .as_slice(),
            ),
        ])
    }

    #[test]
    fn lists_loader_versions_with_stability() {
        let f = fetch();
        let loader = FabricLikeLoader::fabric(&f);
        let versions = loader.list_versions("1.20.4").unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].id, "0.15.7");
        assert!(versions[0].stable);
        assert!(!versions[1].stable);
    }

    #[test]
    fn install_writes_profile_json_to_cache_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let f = fetch();
        let loader = FabricLikeLoader::fabric(&f);
        let ctx = InstallContext { shared_cache_root: dir.path(), work_dir: dir.path() };

        let profile = loader.install("1.20.4", "0.15.7", &ctx).unwrap();
        let expected = dir
            .path()
            .join("versions/fabric-loader-0.15.7-1.20.4/fabric-loader-0.15.7-1.20.4.json");
        assert_eq!(profile.version_json_path, expected);
        let raw = fs::read_to_string(&expected).unwrap();
        assert!(raw.contains("KnotClient"), "원문 그대로 저장");

        // 불변 캐시: 재설치는 재다운로드 없이 기존 파일 재사용... (fetch는 profile 1회만 추가 호출)
        let calls_before = f.call_count();
        loader.install("1.20.4", "0.15.7", &ctx).unwrap();
        assert_eq!(f.call_count(), calls_before + 1, "메타 재조회는 하되 파일은 덮어쓰지 않음");
    }

    #[test]
    fn install_rejects_traversal_in_profile_id() {
        // 메타 API 응답도 외부 입력(§11) — id로 캐시 밖 경로 조작 불가해야 함 (§7.3)
        let dir = tempfile::tempdir().unwrap();
        let evil = br#"{"id": "../../evil", "inheritsFrom": "1.20.4",
            "mainClass": "x", "libraries": []}"#;
        let f = MockFetch::with(&[(PROFILE_URL, evil.as_slice())]);
        let loader = FabricLikeLoader::fabric(&f);
        let cache = dir.path().join("cache");
        fs::create_dir_all(&cache).unwrap();
        let ctx = InstallContext { shared_cache_root: &cache, work_dir: dir.path() };
        assert!(loader.install("1.20.4", "0.15.7", &ctx).is_err());
        assert!(!dir.path().join("evil.json").exists(), "캐시 밖 쓰기 금지");
    }

    #[test]
    fn install_rejects_malformed_profile() {
        let dir = tempfile::tempdir().unwrap();
        let f = MockFetch::with(&[(PROFILE_URL, b"not json".as_slice())]);
        let loader = FabricLikeLoader::fabric(&f);
        let ctx = InstallContext { shared_cache_root: dir.path(), work_dir: dir.path() };
        assert!(loader.install("1.20.4", "0.15.7", &ctx).is_err());
    }
}
