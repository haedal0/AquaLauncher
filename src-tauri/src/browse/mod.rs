//! 모드 브라우저 API 클라이언트 — PRD 8.17.
//! Modrinth 우선 구현. CurseForge는 API 키(PRD 0-4) 승인 전까지 MockCurseForge 사용.
//! CF 배포 opt-out 모드: 다운로드 URL 미제공 -> E-MB-01 폴백 (숨기지 말고 제한 표시).
pub mod modrinth;

#[allow(dead_code)]
pub struct SearchHit {
    pub name: String,
    pub project_id: String,
    /// None이면 런처 내 다운로드 불가 (opt-out) — 공식 페이지 링크 제공
    pub download_url: Option<String>,
}

pub trait ModPlatform: Send + Sync {
    fn id(&self) -> &'static str;
    /// 현재 인스턴스의 MC 버전 + 로더로 필터된 결과만 반환 (PRD 8.17)
    fn search(
        &self,
        query: &str,
        mc_version: &str,
        loader: &str,
    ) -> Result<Vec<SearchHit>, crate::error::AppError>;
}

pub struct MockCurseForge;

impl ModPlatform for MockCurseForge {
    fn id(&self) -> &'static str { "curseforge-mock" }
    fn search(
        &self,
        _query: &str,
        _mc_version: &str,
        _loader: &str,
    ) -> Result<Vec<SearchHit>, crate::error::AppError> {
        Ok(vec![])
    }
}
