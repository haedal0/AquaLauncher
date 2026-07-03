//! manifest.lock.json — 동기화 엔진의 핵심 상태 (PRD 7.4)
//!
//! 규칙 (위반 = 사용자 데이터 파괴):
//! - Origin::User 파일은 어떤 경로로도 자동 삭제하지 않는다.
//! - 사용자가 직접 넣은 파일과 동일 해시의 모드가 매니페스트에 추가돼도 origin은 User 유지.
//! - 매칭 키 우선순위: mod_id -> filename -> sha256.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Lockfile {
    #[serde(default)]
    pub applied_manifest_hash: Option<String>,
    #[serde(default)]
    pub applied_at: Option<String>,
    #[serde(default)]
    pub managed_files: Vec<ManagedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedFile {
    /// 인스턴스 루트 기준 상대경로
    pub path: String,
    #[serde(default)]
    pub mod_id: Option<String>,
    pub sha256: String,
    pub origin: Origin,
    #[serde(default = "yes")]
    pub enabled: bool,
}
fn yes() -> bool { true }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// 매니페스트가 설치 — 매니페스트에서 사라지면 삭제 대상
    Manifest,
    /// 사용자가 추가 — 절대 자동 삭제 금지
    User,
}
