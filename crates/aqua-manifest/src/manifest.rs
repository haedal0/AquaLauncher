//! 서버 매니페스트 스키마 — PRD 7.3
use serde::{Deserialize, Serialize};

/// 런처가 이해하는 매니페스트 포맷 버전. 이보다 크면 E-MF-03 처리 (PRD 9장).
pub const SUPPORTED_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub format_version: u32,
    pub min_launcher_version: String,
    pub display_version: String,
    #[serde(default)]
    pub changelog: Option<String>,
    pub server_display_name: String,
    #[serde(default)]
    pub server_description: Option<String>,
    pub minecraft_version: String,
    pub loader: LoaderRef,
    pub server: ServerInfo,
    #[serde(default)]
    pub recommended_jvm: Option<RecommendedJvm>,
    #[serde(default)]
    pub theme: Option<Theme>,
    #[serde(default)]
    pub mods: Vec<ModEntry>,
    /// config/shaderpack 등 일반 파일 배포 — PRD 7.3 files
    #[serde(default)]
    pub files: Vec<FileEntry>,
    #[serde(default)]
    pub resourcepack: Option<ResourcePack>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoaderRef {
    #[serde(rename = "type")]
    pub kind: LoaderKind,
    pub version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoaderKind { Vanilla, Fabric, Quilt, Forge, Neoforge }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub address: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub direct_connect_default: bool,
}
fn default_port() -> u16 { 25565 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendedJvm {
    pub min_memory_mb: u32,
    pub max_memory_mb: u32,
}

/// 테마 — 화이트리스트 필드만 (PRD 8.5). font는 번들 폰트 키만 허용.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    #[serde(default)]
    pub primary_color: Option<String>,
    #[serde(default)]
    pub accent_color: Option<String>,
    #[serde(default)]
    pub background_image: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub font: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModEntry {
    pub id: String,
    pub filename: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub source: Source,
    #[serde(default = "yes")]
    pub required: bool,
    /// 옵셔널 모드 UI 그룹 — PRD 7.3
    #[serde(default)]
    pub optional_group: Option<String>,
    #[serde(default = "yes")]
    pub default_enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}
fn yes() -> bool { true }

/// 모드/파일 소스 3종 — PRD 7.3 [v2.3]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Source {
    Url { url: String },
    Modrinth { project_id: String, version_id: String },
    Curseforge { project_id: u64, file_id: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// 인스턴스 루트 기준 상대경로. 반드시 path::safe_join 으로만 사용.
    pub path: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub source: Source,
    pub sync_policy: SyncPolicy,
}

/// config 동기화 정책 — PRD 7.3 [확정: 운영자 파일별 지정]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncPolicy {
    /// 해시 불일치 시 항상 서버 버전으로 덮어씀
    Always,
    /// 최초 설치 1회만 배포, 이후 사용자 소유
    Once,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourcePack {
    pub filename: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub source: Source,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_manifest_deserializes() {
        let raw = include_str!("../../../fixtures/data/manifest.json");
        let m: Manifest = serde_json::from_str(raw).expect("fixture must match schema");
        assert_eq!(m.format_version, SUPPORTED_FORMAT_VERSION);
        assert!(!m.mods.is_empty());
    }
}
