//! 사용자 노출 에러 코드 체계 — PRD 9장과 1:1.
//! 새 에러 시나리오는 PRD 9장에 행 추가를 제안한 뒤 여기에 반영한다.
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "code", content = "detail")]
#[allow(dead_code)] // 스켈레톤 단계 — 구현 진행에 따라 제거
pub enum AppError {
    // ── 매니페스트 (E-MF-*) ──
    #[error("서버 구성을 가져올 수 없습니다")]
    ManifestFetch(String),                    // E-MF-01
    #[error("구성 파일이 손상되었습니다")]
    ManifestParse(String),                    // E-MF-02
    #[error("런처 업데이트가 필요합니다")]
    ManifestUnsupported { format_version: u32 }, // E-MF-03
    #[error("일부 파일을 받지 못했습니다")]
    DownloadFailed { files: Vec<String> },    // E-MF-04
    #[error("이 런처는 MC 1.13 이상만 지원합니다")]
    McVersionTooOld(String),                  // E-MF-05

    // ── 인증 (E-AU-*) — PRD 8.3 XSTS 분기 필수 ──
    #[error("로그인에 실패했습니다")]
    AuthFailed(String),                       // E-AU-01
    #[error("Xbox 프로필이 필요합니다")]
    XstsNoProfile,                            // E-AU-02
    #[error("자녀 계정은 가족 등록이 필요합니다")]
    XstsChildAccount,                         // E-AU-03
    #[error("Xbox 성인 인증이 필요합니다 (한국 계정)")]
    XstsAdultVerification,                    // E-AU-04
    #[error("마인크래프트를 소유하고 있지 않습니다")]
    NotEntitled,                              // E-AU-05 (Game Pass는 프로필 조회로 판정)
    #[error("다시 로그인해 주세요")]
    TokenExpired,                             // E-AU-06

    // ── 기타 ──
    #[error("Java 런타임을 준비하지 못했습니다")]
    JavaSetup(String),                        // E-JV-01
    #[error("모드로더 설치에 실패했습니다")]
    LoaderInstall(String),                    // E-LD-01
    #[error("디스크 공간이 부족합니다")]
    DiskFull { needed_bytes: u64, available_bytes: u64 }, // E-DL-01
    #[error("제작자가 런처 내 다운로드를 제한한 모드입니다")]
    CurseforgeOptOut { page_url: String },    // E-MB-01
}
