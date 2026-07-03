//! Modrinth API 클라이언트 — PRD 8.17 (모드 브라우저) + 7.3 (매니페스트 modrinth 소스).
//! API 예절: 식별 가능한 UA는 HttpFetcher가 전송, 검색 디바운스는 프론트(300ms).
use crate::net::{Fetch, NetError};
use serde::Deserialize;
use thiserror::Error;

pub const MODRINTH_API: &str = "https://api.modrinth.com/v2";

#[derive(Debug, Error)]
pub enum ModrinthError {
    #[error(transparent)]
    Net(#[from] NetError),
    #[error("modrinth response parse: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("no downloadable file for {0}")]
    NoFile(String),
    #[error("no compatible version of {project} for {mc}/{loader}")]
    NoCompatibleVersion { project: String, mc: String, loader: String },
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionInfo {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub files: Vec<VersionFile>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionFile {
    pub url: String,
    pub filename: String,
    #[serde(default)]
    pub primary: bool,
    pub hashes: FileHashes,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileHashes {
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub sha512: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Dependency {
    #[serde(default)]
    pub project_id: Option<String>,
    /// "required" | "optional" | ...
    #[serde(default)]
    pub dependency_type: String,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    hits: Vec<SearchHitRaw>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchHitRaw {
    pub project_id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub downloads: u64,
}

pub struct ModrinthClient<'a> {
    pub fetch: &'a dyn Fetch,
}

impl ModrinthClient<'_> {
    /// 매니페스트 modrinth 소스 해석용 — version_id로 파일 정보 조회.
    pub fn version(&self, version_id: &str) -> Result<VersionInfo, ModrinthError> {
        let raw = self.fetch.get_bytes(&format!("{MODRINTH_API}/version/{version_id}"))?;
        Ok(serde_json::from_slice(&raw)?)
    }

    /// primary 파일(없으면 첫 파일) — 다운로드 대상.
    pub fn primary_file(info: &VersionInfo) -> Result<&VersionFile, ModrinthError> {
        info.files
            .iter()
            .find(|f| f.primary)
            .or_else(|| info.files.first())
            .ok_or_else(|| ModrinthError::NoFile(info.id.clone()))
    }

    /// 검색 — 현재 인스턴스 MC 버전 + 로더로 자동 필터 (PRD 8.17).
    pub fn search(
        &self,
        query: &str,
        mc_version: &str,
        loader: &str,
        limit: usize,
    ) -> Result<Vec<SearchHitRaw>, ModrinthError> {
        let facets = format!(
            r#"[["versions:{mc_version}"],["categories:{loader}"],["project_type:mod"]]"#
        );
        let url = format!(
            "{MODRINTH_API}/search?query={}&limit={limit}&facets={}",
            percent_encode(query),
            percent_encode(&facets),
        );
        let raw = self.fetch.get_bytes(&url)?;
        let resp: SearchResponse = serde_json::from_slice(&raw)?;
        Ok(resp.hits)
    }

    /// 최신 호환 버전 — 브라우저 설치 기본값 (PRD 8.17).
    pub fn latest_compatible(
        &self,
        project_id: &str,
        mc_version: &str,
        loader: &str,
    ) -> Result<VersionInfo, ModrinthError> {
        let url = format!(
            "{MODRINTH_API}/project/{project_id}/version?game_versions={}&loaders={}",
            percent_encode(&format!(r#"["{mc_version}"]"#)),
            percent_encode(&format!(r#"["{loader}"]"#)),
        );
        let raw = self.fetch.get_bytes(&url)?;
        let versions: Vec<VersionInfo> = serde_json::from_slice(&raw)?;
        versions.into_iter().next().ok_or_else(|| ModrinthError::NoCompatibleVersion {
            project: project_id.to_string(),
            mc: mc_version.to_string(),
            loader: loader.to_string(),
        })
    }
}

/// 쿼리 성분 퍼센트 인코딩 (RFC 3986 unreserved 외 전부)
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::test_support::MockFetch;

    #[test]
    fn version_lookup_and_primary_file() {
        let body = br#"{"id": "abc123", "name": "v24.1", "files": [
            {"url": "https://cdn.modrinth.com/data/x/map.jar", "filename": "map.jar",
             "primary": true, "hashes": {"sha1": "s1"}, "size": 10},
            {"url": "https://cdn.modrinth.com/data/x/map-sources.jar", "filename": "map-sources.jar",
             "primary": false, "hashes": {}, "size": 5}
        ]}"#;
        let f = MockFetch::with(&[("https://api.modrinth.com/v2/version/abc123", body.as_slice())]);
        let client = ModrinthClient { fetch: &f };
        let info = client.version("abc123").unwrap();
        let file = ModrinthClient::primary_file(&info).unwrap();
        assert_eq!(file.filename, "map.jar");
        assert_eq!(file.hashes.sha1.as_deref(), Some("s1"));
    }

    #[test]
    fn search_builds_facet_filtered_request() {
        let expected_url = format!(
            "{MODRINTH_API}/search?query=sodium&limit=10&facets={}",
            percent_encode(r#"[["versions:1.20.4"],["categories:fabric"],["project_type:mod"]]"#)
        );
        let body = br#"{"hits": [{"project_id": "AANobbMI", "title": "Sodium",
            "description": "rendering", "downloads": 4200000}]}"#;
        let f = MockFetch::with(&[(expected_url.as_str(), body.as_slice())]);
        let client = ModrinthClient { fetch: &f };
        let hits = client.search("sodium", "1.20.4", "fabric", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Sodium");
    }

    #[test]
    fn latest_compatible_takes_first_and_errors_when_empty() {
        let url = format!(
            "{MODRINTH_API}/project/AANobbMI/version?game_versions={}&loaders={}",
            percent_encode(r#"["1.20.4"]"#),
            percent_encode(r#"["fabric"]"#)
        );
        let body = br#"[{"id": "new", "files": [{"url": "https://cdn/x.jar", "filename": "x.jar",
            "primary": true, "hashes": {"sha1": "s"}, "size": 1}]},
            {"id": "old", "files": []}]"#;
        let f = MockFetch::with(&[(url.as_str(), body.as_slice())]);
        let client = ModrinthClient { fetch: &f };
        assert_eq!(client.latest_compatible("AANobbMI", "1.20.4", "fabric").unwrap().id, "new");

        let empty = MockFetch::with(&[(url.as_str(), b"[]".as_slice())]);
        let client = ModrinthClient { fetch: &empty };
        assert!(matches!(
            client.latest_compatible("AANobbMI", "1.20.4", "fabric"),
            Err(ModrinthError::NoCompatibleVersion { .. })
        ));
    }
}
