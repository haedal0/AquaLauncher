//! MS 인증 체인 — PRD 8.3. **사람 리뷰 필수 구역.**
//! 플로우: Authorization Code + PKCE 기본, 디바이스 코드 폴백.
//! MS/Mojang 서드파티 런처 승인(PRD 0-1) 전까지 MockAuthProvider로 개발한다.

#[derive(Debug, Clone)]
pub struct AccountProfile {
    pub account_id: String,
    pub gamertag: String,
    pub mc_uuid: String,
}

pub trait AuthProvider: Send + Sync {
    /// 전체 체인(MSA→XBL→XSTS→MC) 수행. 실패는 error.rs의 E-AU-* 로 분류.
    fn sign_in(&self) -> Result<AccountProfile, crate::error::AppError>;
    fn refresh(&self, account_id: &str) -> Result<AccountProfile, crate::error::AppError>;
}

/// 승인 대기 중 개발용 목 — 실제 네트워크 호출 없음.
pub struct MockAuthProvider;

impl AuthProvider for MockAuthProvider {
    fn sign_in(&self) -> Result<AccountProfile, crate::error::AppError> {
        Ok(AccountProfile {
            account_id: "mock-account".into(),
            gamertag: "Steve_KR".into(),
            mc_uuid: "00000000-0000-0000-0000-000000000000".into(),
        })
    }
    fn refresh(&self, _account_id: &str) -> Result<AccountProfile, crate::error::AppError> {
        self.sign_in()
    }
}
