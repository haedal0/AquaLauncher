//! 클래스패스 조립 — TD-01 §3.
//! maven 좌표 → 공유 캐시 상대 경로 해석, 중복 좌표는 뒤(로더 측) 우선.
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CoordError {
    #[error("invalid maven coordinate: {0}")]
    Invalid(String),
}

struct Coord<'a> {
    group: &'a str,
    artifact: &'a str,
    version: &'a str,
    classifier: Option<&'a str>,
    ext: &'a str,
}

fn parse(coord: &str) -> Result<Coord<'_>, CoordError> {
    let (body, ext) = match coord.split_once('@') {
        Some((b, e)) => (b, e),
        None => (coord, "jar"),
    };
    let parts: Vec<&str> = body.split(':').collect();
    let (group, artifact, version, classifier) = match parts.as_slice() {
        [g, a, v] => (*g, *a, *v, None),
        [g, a, v, c] => (*g, *a, *v, Some(*c)),
        _ => return Err(CoordError::Invalid(coord.to_string())),
    };
    if group.is_empty() || artifact.is_empty() || version.is_empty() || ext.is_empty() {
        return Err(CoordError::Invalid(coord.to_string()));
    }
    Ok(Coord { group, artifact, version, classifier, ext })
}

/// `group:artifact:version[:classifier][@ext]` → `group/…/artifact/version/artifact-version[-classifier].ext`
pub fn maven_to_rel_path(coord: &str) -> Result<String, CoordError> {
    let c = parse(coord)?;
    let group_path = c.group.replace('.', "/");
    let classifier = c.classifier.map(|cl| format!("-{cl}")).unwrap_or_default();
    Ok(format!(
        "{group_path}/{a}/{v}/{a}-{v}{classifier}.{ext}",
        a = c.artifact,
        v = c.version,
        ext = c.ext
    ))
}

/// 중복 라이브러리 해소: `group:artifact[:classifier]` 단위로 **뒤에 온 항목(로더 측)** 이 이긴다.
/// 순서는 최초 등장 위치를 유지한다 (PRD §8.15-2).
pub fn dedupe_coords(coords: &[&str]) -> Result<Vec<String>, CoordError> {
    let mut out: Vec<String> = Vec::new();
    let mut index_of: std::collections::BTreeMap<String, usize> = Default::default();
    for &coord in coords {
        let c = parse(coord)?;
        let key = format!("{}:{}:{}", c.group, c.artifact, c.classifier.unwrap_or(""));
        match index_of.get(&key) {
            Some(&i) => out[i] = coord.to_string(),
            None => {
                index_of.insert(key, out.len());
                out.push(coord.to_string());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinate_maps_to_maven_layout() {
        assert_eq!(
            maven_to_rel_path("org.ow2.asm:asm:9.8").unwrap(),
            "org/ow2/asm/asm/9.8/asm-9.8.jar"
        );
    }

    #[test]
    fn coordinate_with_classifier() {
        assert_eq!(
            maven_to_rel_path("org.lwjgl:lwjgl:3.3.1:natives-macos-arm64").unwrap(),
            "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-macos-arm64.jar"
        );
    }

    #[test]
    fn coordinate_with_extension() {
        assert_eq!(
            maven_to_rel_path("de.oceanlabs.mcp:mcp_config:1.20.1@zip").unwrap(),
            "de/oceanlabs/mcp/mcp_config/1.20.1/mcp_config-1.20.1.zip"
        );
    }

    #[test]
    fn malformed_coordinate_rejected() {
        assert!(maven_to_rel_path("no-colons").is_err());
        assert!(maven_to_rel_path("a:b").is_err());
        assert!(maven_to_rel_path("").is_err());
    }

    #[test]
    fn duplicate_artifact_loader_side_wins_in_place() {
        // 바닐라 asm 9.3 뒤에 로더 asm 9.8 → 9.8이 원래 위치에서 승리
        let out = dedupe_coords(&[
            "org.ow2.asm:asm:9.3",
            "com.mojang:logging:1.1.1",
            "org.ow2.asm:asm:9.8",
        ])
        .unwrap();
        assert_eq!(
            out,
            ["org.ow2.asm:asm:9.8", "com.mojang:logging:1.1.1"]
        );
    }

    #[test]
    fn different_classifiers_are_distinct() {
        let out = dedupe_coords(&[
            "org.lwjgl:lwjgl:3.3.1",
            "org.lwjgl:lwjgl:3.3.1:natives-macos-arm64",
        ])
        .unwrap();
        assert_eq!(out.len(), 2);
    }
}
