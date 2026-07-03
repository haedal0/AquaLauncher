//! 모드로더 추상화 — PRD 8.1. 구현 순서: Fabric -> Quilt -> (스파이크 후) Forge/NeoForge.
//! 인터페이스는 스파이크로 확정됨 (docs/spike-forge-report.md).
pub mod fabric;
pub mod forge;

use std::path::Path;

#[allow(dead_code)]
pub struct LoaderVersion {
    pub id: String,
    pub stable: bool,
}

/// 설치 결과 — version JSON 위치 등. TD-01 확정 시 구체화.
#[allow(dead_code)]
#[derive(Debug)]
pub struct LaunchProfile {
    pub version_json_path: std::path::PathBuf,
}

#[allow(dead_code)]
pub struct InstallContext<'a> {
    pub shared_cache_root: &'a Path,
    pub work_dir: &'a Path,
}

pub trait ModLoader: Send + Sync {
    fn id(&self) -> &'static str;
    fn list_versions(&self, mc_version: &str) -> Result<Vec<LoaderVersion>, crate::error::AppError>;
    /// Forge/NeoForge 함정 목록은 PRD 8.1 참고 (launcher_profiles.json 더미, 인스톨러 작업 디렉토리 등)
    fn install(
        &self,
        mc_version: &str,
        loader_version: &str,
        ctx: &InstallContext,
    ) -> Result<LaunchProfile, crate::error::AppError>;
}
