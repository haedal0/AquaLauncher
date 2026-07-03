//! Mojang/로더 version JSON 모델 + `inheritsFrom` 체인 병합 — TD-01 §1.
//!
//! 병합 규칙: 스칼라는 자식(로더) 우선, libraries·arguments는 부모 먼저 연결(append).
//! 체인 깊이 상한 5, 순환 감지 — version JSON도 신뢰하지 않는 입력이다.
use super::rules::Rule;
use serde::Deserialize;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MergeError {
    #[error("inheritsFrom chain too deep (max {0})")]
    ChainTooDeep(usize),
    #[error("inheritsFrom cycle detected at {0}")]
    Cycle(String),
    #[error("missing parent version json: {0}")]
    MissingParent(String),
    #[error("merged version has no mainClass")]
    NoMainClass,
}

pub const MAX_CHAIN_DEPTH: usize = 5;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionJson {
    pub id: String,
    #[serde(default)]
    pub inherits_from: Option<String>,
    #[serde(default)]
    pub main_class: Option<String>,
    #[serde(default)]
    pub arguments: Option<Arguments>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default)]
    pub asset_index: Option<AssetIndexRef>,
    #[serde(default)]
    pub assets: Option<String>,
    #[serde(default)]
    pub java_version: Option<JavaVersionReq>,
    #[serde(default)]
    pub downloads: Option<VersionDownloads>,
    #[serde(default, rename = "type")]
    pub release_type: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Arguments {
    #[serde(default)]
    pub jvm: Vec<ArgumentEntry>,
    #[serde(default)]
    pub game: Vec<ArgumentEntry>,
}

/// 인자 항목: 문자열 또는 rules 조건부 값.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ArgumentEntry {
    Plain(String),
    Conditional {
        rules: Vec<Rule>,
        value: ValueOrList,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ValueOrList {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct Library {
    /// maven 좌표 `group:artifact:version[:classifier]`
    pub name: String,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub natives: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub downloads: Option<LibraryDownloads>,
    /// Fabric/Quilt profile 방식: maven base URL만 제공 (downloads 없음, 해시 없음)
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LibraryDownloads {
    #[serde(default)]
    pub artifact: Option<Artifact>,
    /// 레거시(1.18 이전) natives classifier — natives 맵의 키로 조회
    #[serde(default)]
    pub classifiers: Option<BTreeMap<String, Artifact>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Artifact {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub url: Option<String>,
}

/// 바닐라 version JSON 최상위 downloads (client.jar 등)
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VersionDownloads {
    #[serde(default)]
    pub client: Option<Artifact>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetIndexRef {
    pub id: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub sha1: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaVersionReq {
    pub major_version: u32,
}

/// 병합 결과 — 실행 파이프라인의 입력.
#[derive(Debug, Clone)]
pub struct MergedVersion {
    /// 최종(자식) version id — natives 디렉토리 등에 사용
    pub id: String,
    /// 체인 최상위(바닐라) version id — client.jar 경로에 사용
    pub root_id: String,
    /// 바닐라 client.jar 다운로드 정보
    pub client_download: Option<Artifact>,
    pub main_class: String,
    pub jvm_args: Vec<ArgumentEntry>,
    pub game_args: Vec<ArgumentEntry>,
    /// 부모(바닐라) → 자식(로더) 순서 그대로 연결. 중복 해소는 classpath 단계.
    pub libraries: Vec<Library>,
    pub asset_index: Option<AssetIndexRef>,
    pub assets: Option<String>,
    /// 요구 Java 메이저 버전. 명시 없으면 8 (레거시 기본, PRD §8.4)
    pub java_major: u32,
    pub release_type: Option<String>,
}

/// leaf부터 `inheritsFrom`을 따라 [최상위 부모, ..., leaf] 순서의 체인을 만든다.
pub fn resolve_chain<F>(leaf: VersionJson, lookup: F) -> Result<Vec<VersionJson>, MergeError>
where
    F: Fn(&str) -> Option<VersionJson>,
{
    let mut seen = vec![leaf.id.clone()];
    let mut chain = vec![leaf];
    while let Some(parent_id) = chain.last().and_then(|v| v.inherits_from.clone()) {
        if seen.contains(&parent_id) {
            return Err(MergeError::Cycle(parent_id));
        }
        if chain.len() >= MAX_CHAIN_DEPTH {
            return Err(MergeError::ChainTooDeep(MAX_CHAIN_DEPTH));
        }
        let parent = lookup(&parent_id).ok_or(MergeError::MissingParent(parent_id.clone()))?;
        seen.push(parent_id);
        chain.push(parent);
    }
    chain.reverse(); // [부모, ..., 자식]
    Ok(chain)
}

/// 체인([부모, ..., 자식])을 하나로 병합한다.
/// 스칼라는 자식 우선, libraries·arguments는 부모 먼저 연결 (TD-01 §1).
pub fn merge_chain(chain: Vec<VersionJson>) -> Result<MergedVersion, MergeError> {
    let mut id = None;
    let mut root_id = None;
    let mut client_download = None;
    let mut main_class = None;
    let mut jvm_args = Vec::new();
    let mut game_args = Vec::new();
    let mut libraries = Vec::new();
    let mut asset_index = None;
    let mut assets = None;
    let mut java_major = None;
    let mut release_type = None;

    for v in chain {
        if root_id.is_none() {
            root_id = Some(v.id.clone());
        }
        id = Some(v.id);
        if let Some(d) = v.downloads {
            if d.client.is_some() {
                client_download = d.client;
            }
        }
        if v.main_class.is_some() {
            main_class = v.main_class;
        }
        if let Some(args) = v.arguments {
            jvm_args.extend(args.jvm);
            game_args.extend(args.game);
        }
        libraries.extend(v.libraries);
        if v.asset_index.is_some() {
            asset_index = v.asset_index;
        }
        if v.assets.is_some() {
            assets = v.assets;
        }
        if let Some(j) = v.java_version {
            java_major = Some(j.major_version);
        }
        if v.release_type.is_some() {
            release_type = v.release_type;
        }
    }

    Ok(MergedVersion {
        id: id.ok_or(MergeError::NoMainClass)?,
        root_id: root_id.ok_or(MergeError::NoMainClass)?,
        client_download,
        main_class: main_class.ok_or(MergeError::NoMainClass)?,
        jvm_args,
        game_args,
        libraries,
        asset_index,
        assets,
        java_major: java_major.unwrap_or(8),
        release_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vanilla() -> VersionJson {
        serde_json::from_str(
            r#"{
              "id": "1.20.1",
              "mainClass": "net.minecraft.client.main.Main",
              "type": "release",
              "assets": "5",
              "assetIndex": {"id": "5", "url": "https://example/5.json"},
              "javaVersion": {"majorVersion": 17},
              "arguments": {
                "jvm": ["-Djava.library.path=${natives_directory}", "-cp", "${classpath}"],
                "game": ["--username", "${auth_player_name}"]
              },
              "libraries": [
                {"name": "org.ow2.asm:asm:9.3"},
                {"name": "com.mojang:logging:1.1.1"}
              ]
            }"#,
        )
        .unwrap()
    }

    fn forge() -> VersionJson {
        serde_json::from_str(
            r#"{
              "id": "1.20.1-forge-47.4.10",
              "inheritsFrom": "1.20.1",
              "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
              "arguments": {
                "jvm": ["-DlibraryDirectory=${library_directory}"],
                "game": ["--launchTarget", "forgeclient"]
              },
              "libraries": [
                {"name": "org.ow2.asm:asm:9.8"},
                {"name": "cpw.mods:bootstraplauncher:1.1.2"}
              ]
            }"#,
        )
        .unwrap()
    }

    fn lookup(id: &str) -> Option<VersionJson> {
        (id == "1.20.1").then(vanilla)
    }

    #[test]
    fn chain_resolves_parent_then_leaf() {
        let chain = resolve_chain(forge(), lookup).unwrap();
        let ids: Vec<_> = chain.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["1.20.1", "1.20.1-forge-47.4.10"]);
    }

    #[test]
    fn chain_missing_parent_errors() {
        let mut orphan = forge();
        orphan.inherits_from = Some("9.9.9".into());
        assert!(matches!(
            resolve_chain(orphan, lookup),
            Err(MergeError::MissingParent(_))
        ));
    }

    #[test]
    fn chain_cycle_detected() {
        let mut a = vanilla();
        a.inherits_from = Some("1.20.1".into()); // 자기 자신 참조
        assert!(matches!(
            resolve_chain(a, lookup),
            Err(MergeError::Cycle(_) | MergeError::ChainTooDeep(_))
        ));
    }

    #[test]
    fn merge_child_overrides_scalars_and_appends_lists() {
        let merged = merge_chain(vec![vanilla(), forge()]).unwrap();
        assert_eq!(merged.id, "1.20.1-forge-47.4.10");
        assert_eq!(merged.main_class, "cpw.mods.bootstraplauncher.BootstrapLauncher");
        // 부모 값 유지 (자식에 없음)
        assert_eq!(merged.assets.as_deref(), Some("5"));
        assert_eq!(merged.java_major, 17);
        // arguments: 부모 먼저, 자식 뒤 (TD-01 §1)
        assert_eq!(merged.jvm_args.len(), 4);
        assert_eq!(merged.game_args.len(), 4);
        // libraries: append — 중복 asm 좌표도 그대로 (해소는 classpath 단계)
        let names: Vec<_> = merged.libraries.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "org.ow2.asm:asm:9.3",
                "com.mojang:logging:1.1.1",
                "org.ow2.asm:asm:9.8",
                "cpw.mods:bootstraplauncher:1.1.2"
            ]
        );
    }

    #[test]
    fn merge_without_main_class_errors() {
        let mut v = vanilla();
        v.main_class = None;
        assert!(matches!(merge_chain(vec![v]), Err(MergeError::NoMainClass)));
    }

    #[test]
    fn vanilla_only_merge_defaults_java_to_8_when_absent() {
        let mut v = vanilla();
        v.java_version = None;
        let merged = merge_chain(vec![v]).unwrap();
        assert_eq!(merged.java_major, 8);
    }
}
