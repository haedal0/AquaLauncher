//! Mojang 메타 API 클라이언트 — PRD §8.15-1, §8.4 (javaVersion).
//! 외부 API는 Fetch trait 뒤에 있으므로 테스트는 MockFetch로 수행.
use super::{Fetch, NetError};
use crate::launch::version_json::VersionJson;
use serde::Deserialize;
use std::collections::BTreeMap;
use thiserror::Error;

pub const VERSION_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
pub const ASSET_BASE_URL: &str = "https://resources.download.minecraft.net";

#[derive(Debug, Error)]
pub enum MetaError {
    #[error(transparent)]
    Net(#[from] NetError),
    #[error("meta json parse: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("unknown minecraft version: {0}")]
    UnknownVersion(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionManifest {
    pub latest: LatestRefs,
    pub versions: Vec<VersionRef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LatestRefs {
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionRef {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
    /// ISO-8601 — 1.13 지원 하한(§5) 필터에 사용
    #[serde(rename = "releaseTime", default)]
    pub release_time: String,
}

/// MC 1.13 정식 릴리스 시각 — 이보다 오래된 버전은 지원 범위 밖 (§5, E-MF-05와 동일 하한).
/// ISO-8601 문자열은 사전순 비교가 시간순 비교와 일치한다.
pub const MC_113_RELEASE_TIME: &str = "2018-07-18";

impl VersionManifest {
    /// 수동 인스턴스 생성 드롭다운용 버전 id 목록 (최신순 유지).
    /// release는 항상, snapshot은 옵션, old_beta/old_alpha는 항상 제외.
    pub fn selectable_ids(&self, include_snapshots: bool) -> Vec<String> {
        self.versions
            .iter()
            .filter(|v| match v.kind.as_str() {
                "release" => true,
                "snapshot" => include_snapshots,
                _ => false,
            })
            .filter(|v| v.release_time.as_str() >= MC_113_RELEASE_TIME)
            .map(|v| v.id.clone())
            .collect()
    }
}

/// asset index 본문 — objects: "경로" -> {hash, size}
#[derive(Debug, Clone, Deserialize)]
pub struct AssetIndex {
    pub objects: BTreeMap<String, AssetObject>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

/// 에셋 오브젝트의 원격 URL (`<base>/<hash 앞2>/<hash>`)
pub fn asset_url(hash: &str) -> String {
    format!("{ASSET_BASE_URL}/{}/{hash}", &hash[..2])
}

pub struct MojangMeta<'a> {
    pub fetch: &'a dyn Fetch,
}

impl MojangMeta<'_> {
    pub fn version_manifest(&self) -> Result<VersionManifest, MetaError> {
        let raw = self.fetch.get_bytes(VERSION_MANIFEST_URL)?;
        Ok(serde_json::from_slice(&raw)?)
    }

    pub fn find_version<'m>(
        manifest: &'m VersionManifest,
        id: &str,
    ) -> Result<&'m VersionRef, MetaError> {
        manifest
            .versions
            .iter()
            .find(|v| v.id == id)
            .ok_or_else(|| MetaError::UnknownVersion(id.to_string()))
    }

    pub fn version_json(&self, vref: &VersionRef) -> Result<VersionJson, MetaError> {
        let raw = self.fetch.get_bytes(&vref.url)?;
        Ok(serde_json::from_slice(&raw)?)
    }

    pub fn asset_index(&self, url: &str) -> Result<AssetIndex, MetaError> {
        let raw = self.fetch.get_bytes(url)?;
        Ok(serde_json::from_slice(&raw)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::test_support::MockFetch;

    #[test]
    fn parses_version_manifest_and_finds_version() {
        let body = br#"{
          "latest": {"release": "1.20.4", "snapshot": "24w10a"},
          "versions": [
            {"id": "1.20.4", "type": "release", "url": "https://piston-meta.mojang.com/v1/1.20.4.json", "sha1": "abc"},
            {"id": "24w10a", "type": "snapshot", "url": "https://piston-meta.mojang.com/v1/24w10a.json"}
          ]
        }"#;
        let fetch = MockFetch::with(&[(VERSION_MANIFEST_URL, body.as_slice())]);
        let meta = MojangMeta { fetch: &fetch };
        let manifest = meta.version_manifest().unwrap();
        assert_eq!(manifest.latest.release, "1.20.4");
        let v = MojangMeta::find_version(&manifest, "1.20.4").unwrap();
        assert_eq!(v.kind, "release");
        assert!(matches!(
            MojangMeta::find_version(&manifest, "0.0.0"),
            Err(MetaError::UnknownVersion(_))
        ));
    }

    /// §5 지원 범위: 1.13 미만·old_beta/alpha 제외, snapshot은 옵션.
    #[test]
    fn selectable_ids_respects_113_floor_and_snapshot_toggle() {
        let manifest: VersionManifest = serde_json::from_str(
            r#"{
              "latest": {"release": "1.21.1", "snapshot": "24w33a"},
              "versions": [
                {"id": "24w33a", "type": "snapshot", "url": "https://m/s.json", "releaseTime": "2024-08-15T12:00:00+00:00"},
                {"id": "1.21.1", "type": "release", "url": "https://m/r.json", "releaseTime": "2024-08-08T12:24:45+00:00"},
                {"id": "1.13", "type": "release", "url": "https://m/13.json", "releaseTime": "2018-07-18T15:11:46+00:00"},
                {"id": "1.12.2", "type": "release", "url": "https://m/12.json", "releaseTime": "2017-09-18T08:39:46+00:00"},
                {"id": "b1.8.1", "type": "old_beta", "url": "https://m/b.json", "releaseTime": "2011-09-19T22:00:00+00:00"}
              ]
            }"#,
        )
        .unwrap();
        assert_eq!(manifest.selectable_ids(false), ["1.21.1", "1.13"]);
        assert_eq!(manifest.selectable_ids(true), ["24w33a", "1.21.1", "1.13"]);
    }

    #[test]
    fn parses_version_json_via_ref() {
        let vjson = br#"{"id": "1.20.4", "mainClass": "net.minecraft.client.main.Main",
            "downloads": {"client": {"sha1": "c1", "size": 1, "url": "https://piston-data.mojang.com/client.jar"}}}"#;
        let fetch = MockFetch::with(&[("https://x/1.20.4.json", vjson.as_slice())]);
        let meta = MojangMeta { fetch: &fetch };
        let vref = VersionRef {
            id: "1.20.4".into(),
            kind: "release".into(),
            url: "https://x/1.20.4.json".into(),
            sha1: None,
            release_time: String::new(),
        };
        let vj = meta.version_json(&vref).unwrap();
        assert_eq!(vj.downloads.unwrap().client.unwrap().sha1.as_deref(), Some("c1"));
    }

    #[test]
    fn parses_asset_index_and_builds_urls() {
        let body = br#"{"objects": {"minecraft/lang/ko_kr.json": {"hash": "aabbcc00112233445566778899aabbccdd001122", "size": 42}}}"#;
        let fetch = MockFetch::with(&[("https://x/5.json", body.as_slice())]);
        let meta = MojangMeta { fetch: &fetch };
        let index = meta.asset_index("https://x/5.json").unwrap();
        let obj = &index.objects["minecraft/lang/ko_kr.json"];
        assert_eq!(obj.size, 42);
        assert_eq!(
            asset_url(&obj.hash),
            "https://resources.download.minecraft.net/aa/aabbcc00112233445566778899aabbccdd001122"
        );
    }
}
