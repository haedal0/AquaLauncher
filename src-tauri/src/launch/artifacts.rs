//! 병합된 version JSON → 다운로드 계획 — TD-01 §3~§5.
//! 순수 함수: 네트워크/파일시스템 접근 없음. 실행은 net::download가 담당.
use super::classpath::maven_to_rel_path;
use super::rules::{evaluate, RuleContext};
use super::version_json::{Artifact, Library, MergedVersion};
use crate::net::mojang::{asset_url, AssetIndex};

/// 공유 캐시 루트 기준 상대 경로로 표현된 다운로드 1건.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedDownload {
    pub url: String,
    pub dest_rel: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

fn plan_artifact(prefix: &str, a: &Artifact) -> Option<PlannedDownload> {
    let url = a.url.clone().filter(|u| !u.is_empty())?;
    let path = a.path.clone().filter(|p| !p.is_empty())?;
    Some(PlannedDownload {
        url,
        dest_rel: format!("{prefix}/{path}"),
        sha1: a.sha1.clone(),
        size: a.size,
    })
}

/// rules를 평가해 이 플랫폼에 필요한 라이브러리(+레거시 natives classifier)만 계획.
/// url이 없는 항목(로더 인스톨러가 로컬 생성한 라이브러리)은 건너뜀 — 이미 캐시에 있다.
pub fn plan_library_downloads(libs: &[Library], ctx: &RuleContext) -> Vec<PlannedDownload> {
    let mut out = Vec::new();
    for lib in libs {
        if !evaluate(&lib.rules, ctx) {
            continue;
        }
        match &lib.downloads {
            Some(downloads) => {
                if let Some(a) = &downloads.artifact {
                    out.extend(plan_artifact("libraries", a));
                }
                // 레거시(1.18 이전) natives: natives 맵의 os 키 → classifiers 조회
                if let (Some(natives), Some(classifiers)) = (&lib.natives, &downloads.classifiers) {
                    if let Some(key) = natives.get(&ctx.os_name) {
                        if let Some(a) = classifiers.get(key) {
                            out.extend(plan_artifact("libraries", a));
                        }
                    }
                }
            }
            // Fabric/Quilt profile 방식: maven base URL + 좌표로 경로 유도 (해시 미제공)
            None => {
                if let Some(base) = &lib.url {
                    if let Ok(rel) = maven_to_rel_path(&lib.name) {
                        out.push(PlannedDownload {
                            url: format!("{}/{rel}", base.trim_end_matches('/')),
                            dest_rel: format!("libraries/{rel}"),
                            sha1: None,
                            size: None,
                        });
                    }
                }
            }
        }
    }
    out
}

/// 바닐라 client.jar — `versions/<root_id>/<root_id>.jar` (스파이크 확인 레이아웃).
pub fn plan_client_download(merged: &MergedVersion) -> Option<PlannedDownload> {
    let client = merged.client_download.as_ref()?;
    let url = client.url.clone().filter(|u| !u.is_empty())?;
    Some(PlannedDownload {
        url,
        dest_rel: format!("versions/{id}/{id}.jar", id = merged.root_id),
        sha1: client.sha1.clone(),
        size: client.size,
    })
}

/// 에셋 오브젝트 전체 — `assets/objects/<h2>/<hash>` (TD-01 §5).
pub fn plan_asset_downloads(index: &AssetIndex) -> Vec<PlannedDownload> {
    index
        .objects
        .values()
        .map(|obj| PlannedDownload {
            url: asset_url(&obj.hash),
            dest_rel: format!("assets/objects/{}/{}", &obj.hash[..2], obj.hash),
            sha1: Some(obj.hash.clone()),
            size: Some(obj.size),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch::version_json::{merge_chain, VersionJson};

    fn libs() -> Vec<Library> {
        serde_json::from_str(
            r#"[
              {"name": "com.mojang:logging:1.1.1",
               "downloads": {"artifact": {"path": "com/mojang/logging/1.1.1/logging-1.1.1.jar",
                 "sha1": "s1", "size": 10, "url": "https://libraries.minecraft.net/com/mojang/logging/1.1.1/logging-1.1.1.jar"}}},
              {"name": "org.lwjgl:lwjgl:3.3.2:natives-windows",
               "rules": [{"action": "allow", "os": {"name": "windows"}}],
               "downloads": {"artifact": {"path": "org/lwjgl/lwjgl/3.3.2/lwjgl-3.3.2-natives-windows.jar",
                 "sha1": "s2", "size": 20, "url": "https://libraries.minecraft.net/w.jar"}}},
              {"name": "org.lwjgl:lwjgl:3.3.2:natives-macos-arm64",
               "rules": [{"action": "allow", "os": {"name": "osx", "arch": "arm64"}}],
               "downloads": {"artifact": {"path": "org/lwjgl/lwjgl/3.3.2/lwjgl-3.3.2-natives-macos-arm64.jar",
                 "sha1": "s3", "size": 30, "url": "https://libraries.minecraft.net/m.jar"}}},
              {"name": "net.minecraftforge:forge:1.20.1-47.4.10:client",
               "downloads": {"artifact": {"path": "net/minecraftforge/forge/x/client.jar", "url": ""}}},
              {"name": "legacy.natives:lib:1.0",
               "natives": {"osx": "natives-osx"},
               "downloads": {"classifiers": {"natives-osx": {"path": "legacy/natives-osx.jar",
                 "sha1": "s4", "size": 40, "url": "https://libraries.minecraft.net/l.jar"}}}}
            ]"#,
        )
        .unwrap()
    }

    #[test]
    fn plans_only_platform_matching_libraries() {
        let ctx = RuleContext::new("osx", "arm64");
        let plan = plan_library_downloads(&libs(), &ctx);
        let dests: Vec<_> = plan.iter().map(|p| p.dest_rel.as_str()).collect();
        assert_eq!(
            dests,
            [
                "libraries/com/mojang/logging/1.1.1/logging-1.1.1.jar",
                "libraries/org/lwjgl/lwjgl/3.3.2/lwjgl-3.3.2-natives-macos-arm64.jar",
                "libraries/legacy/natives-osx.jar"
            ]
        );
    }

    #[test]
    fn windows_gets_windows_natives_but_not_legacy_osx() {
        let ctx = RuleContext::new("windows", "x86_64");
        let plan = plan_library_downloads(&libs(), &ctx);
        let dests: Vec<_> = plan.iter().map(|p| p.dest_rel.as_str()).collect();
        assert_eq!(
            dests,
            [
                "libraries/com/mojang/logging/1.1.1/logging-1.1.1.jar",
                "libraries/org/lwjgl/lwjgl/3.3.2/lwjgl-3.3.2-natives-windows.jar"
            ]
        );
    }

    #[test]
    fn fabric_maven_base_library_derives_url_and_path() {
        let libs: Vec<Library> = serde_json::from_str(
            r#"[{"name": "net.fabricmc:fabric-loader:0.15.7", "url": "https://maven.fabricmc.net/"}]"#,
        )
        .unwrap();
        let plan = plan_library_downloads(&libs, &RuleContext::new("osx", "arm64"));
        assert_eq!(plan.len(), 1);
        assert_eq!(
            plan[0].url,
            "https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.15.7/fabric-loader-0.15.7.jar"
        );
        assert_eq!(
            plan[0].dest_rel,
            "libraries/net/fabricmc/fabric-loader/0.15.7/fabric-loader-0.15.7.jar"
        );
        assert!(plan[0].sha1.is_none());
    }

    #[test]
    fn client_jar_uses_root_version_id() {
        let vanilla: VersionJson = serde_json::from_str(
            r#"{"id": "1.20.1", "mainClass": "m",
                "downloads": {"client": {"sha1": "c1", "size": 5, "url": "https://piston-data.mojang.com/client.jar"}}}"#,
        )
        .unwrap();
        let forge: VersionJson = serde_json::from_str(
            r#"{"id": "1.20.1-forge-47.4.10", "inheritsFrom": "1.20.1", "mainClass": "f"}"#,
        )
        .unwrap();
        let merged = merge_chain(vec![vanilla, forge]).unwrap();
        let plan = plan_client_download(&merged).unwrap();
        assert_eq!(plan.dest_rel, "versions/1.20.1/1.20.1.jar");
        assert_eq!(plan.sha1.as_deref(), Some("c1"));
    }

    #[test]
    fn asset_plan_uses_hash_sharding() {
        let index: AssetIndex = serde_json::from_str(
            r#"{"objects": {"a": {"hash": "ff00112233445566778899aabbccddeeff001122", "size": 7}}}"#,
        )
        .unwrap();
        let plan = plan_asset_downloads(&index);
        assert_eq!(plan.len(), 1);
        assert_eq!(
            plan[0].dest_rel,
            "assets/objects/ff/ff00112233445566778899aabbccddeeff001122"
        );
        assert!(plan[0].url.starts_with("https://resources.download.minecraft.net/ff/"));
    }
}
