//! 사용자 노출 에러 코드 체계 — PRD 9장과 1:1.
//! 새 에러 시나리오는 PRD 9장에 행 추가를 제안한 뒤 여기에 반영한다.
//!
//! 직렬화 형태는 `{"code":"E-MF-01","detail":…}` — Tauri invoke 거부 페이로드로
//! 프론트(src/lib/errors.ts)에 전달되고, 사용자 문구·복구 액션은 프론트가
//! code 기준으로 i18n 매핑한다 (§8.13 하드코딩 금지). `#[error]` 문구는 로그 전용.
use crate::launch::install::PrepareError;
use crate::net::NetError;
use crate::sync::engine::SyncError;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "code", content = "detail")]
pub enum AppError {
    // ── 매니페스트 (E-MF-*) ──
    #[error("서버 구성을 가져올 수 없습니다: {0}")]
    #[serde(rename = "E-MF-01")]
    ManifestFetch(String),
    #[error("구성 파일이 손상되었습니다: {0}")]
    #[serde(rename = "E-MF-02")]
    ManifestParse(String),
    #[error("런처 업데이트가 필요합니다: {0}")]
    #[serde(rename = "E-MF-03")]
    ManifestUnsupported(String),
    #[error("{}개 파일을 받지 못했습니다", files.len())]
    #[serde(rename = "E-MF-04")]
    DownloadFailed { files: Vec<String> },
    #[error("이 런처는 MC 1.13 이상만 지원합니다 ({0})")]
    #[serde(rename = "E-MF-05")]
    McVersionTooOld(String),

    // ── 인증 (E-AU-*) — PRD 8.3 XSTS 분기 필수. 구현은 auth/(사람 리뷰 구역) ──
    #[error("로그인에 실패했습니다: {0}")]
    #[serde(rename = "E-AU-01")]
    #[allow(dead_code)]
    AuthFailed(String),
    #[error("Xbox 프로필이 필요합니다")]
    #[serde(rename = "E-AU-02")]
    #[allow(dead_code)]
    XstsNoProfile,
    #[error("자녀 계정은 가족 등록이 필요합니다")]
    #[serde(rename = "E-AU-03")]
    #[allow(dead_code)]
    XstsChildAccount,
    #[error("Xbox 성인 인증이 필요합니다 (한국 계정)")]
    #[serde(rename = "E-AU-04")]
    #[allow(dead_code)]
    XstsAdultVerification,
    #[error("마인크래프트를 소유하고 있지 않습니다")]
    #[serde(rename = "E-AU-05")]
    #[allow(dead_code)]
    NotEntitled, // Game Pass는 프로필 조회로 판정
    #[error("다시 로그인해 주세요")]
    #[serde(rename = "E-AU-06")]
    #[allow(dead_code)]
    TokenExpired,

    // ── 실행 준비 ──
    #[error("Java 런타임을 준비하지 못했습니다: {0}")]
    #[serde(rename = "E-JV-01")]
    JavaSetup(String),
    #[error("모드로더 설치에 실패했습니다: {0}")]
    #[serde(rename = "E-LD-01")]
    LoaderInstall(String),
    #[error("디스크 공간이 부족합니다")]
    #[serde(rename = "E-DL-01")]
    #[allow(dead_code)] // §8.2.2 대용량 업데이트 사전 확인에서 구성 예정
    DiskFull { needed_bytes: u64, available_bytes: u64 },
    #[error("제작자가 런처 내 다운로드를 제한한 모드입니다")]
    #[serde(rename = "E-MB-01")]
    #[allow(dead_code)] // CurseForge API 키 승인(PRD 0-4) 후 실경로 연결
    CurseforgeOptOut { page_url: String },

    // ── 분류 불가 (§9 표 밖 — PRD에 E-IN-01 행 추가 제안됨) ──
    #[error("내부 오류: {0}")]
    #[serde(rename = "E-IN-01")]
    Internal(String),
}

impl AppError {
    /// 분류 불가 에러의 표준 생성자 — 세부는 detail로 프론트 "상세 복사"에 노출.
    pub fn internal(e: impl std::fmt::Display) -> Self {
        AppError::Internal(e.to_string())
    }

    /// PRD 9장 코드 문자열 — 로그/테스트용 (직렬화 tag와 동일해야 함).
    pub fn code(&self) -> &'static str {
        match self {
            AppError::ManifestFetch(_) => "E-MF-01",
            AppError::ManifestParse(_) => "E-MF-02",
            AppError::ManifestUnsupported(_) => "E-MF-03",
            AppError::DownloadFailed { .. } => "E-MF-04",
            AppError::McVersionTooOld(_) => "E-MF-05",
            AppError::AuthFailed(_) => "E-AU-01",
            AppError::XstsNoProfile => "E-AU-02",
            AppError::XstsChildAccount => "E-AU-03",
            AppError::XstsAdultVerification => "E-AU-04",
            AppError::NotEntitled => "E-AU-05",
            AppError::TokenExpired => "E-AU-06",
            AppError::JavaSetup(_) => "E-JV-01",
            AppError::LoaderInstall(_) => "E-LD-01",
            AppError::DiskFull { .. } => "E-DL-01",
            AppError::CurseforgeOptOut { .. } => "E-MB-01",
            AppError::Internal(_) => "E-IN-01",
        }
    }
}

/// 동기화 엔진 에러 → §9 코드 (SyncError 정의부의 코드 주석과 1:1).
impl From<SyncError> for AppError {
    fn from(e: SyncError) -> Self {
        match e {
            SyncError::Fetch(inner) => AppError::ManifestFetch(inner.to_string()),
            SyncError::Parse(s) => AppError::ManifestParse(s),
            SyncError::UnsupportedFormat(v) => {
                AppError::ManifestUnsupported(format!("format_version {v}"))
            }
            SyncError::LauncherTooOld(v) => {
                AppError::ManifestUnsupported(format!("min_launcher_version {v}"))
            }
            SyncError::McTooOld(v) => AppError::McVersionTooOld(v),
            SyncError::Download { failed } => AppError::DownloadFailed { files: failed },
            other => AppError::internal(other),
        }
    }
}

/// 실행 준비(버전/아티팩트) 에러 — 다운로드 실패만 E-MF-04로 표면화, 나머지는 내부.
impl From<PrepareError> for AppError {
    fn from(e: PrepareError) -> Self {
        match e {
            PrepareError::Download { failed } => AppError::DownloadFailed { files: failed },
            other => AppError::internal(other),
        }
    }
}

impl From<NetError> for AppError {
    fn from(e: NetError) -> Self {
        AppError::internal(e)
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::internal(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §9: 모든 에러는 코드를 갖는다 — 직렬화 tag가 code()와 일치.
    #[test]
    fn serializes_with_prd9_code_tag() {
        let cases: Vec<AppError> = vec![
            AppError::ManifestFetch("404".into()),
            AppError::ManifestParse("bad json".into()),
            AppError::ManifestUnsupported("format_version 9".into()),
            AppError::DownloadFailed { files: vec!["mods/a.jar".into()] },
            AppError::McVersionTooOld("1.12.2".into()),
            AppError::AuthFailed("cancelled".into()),
            AppError::XstsNoProfile,
            AppError::XstsChildAccount,
            AppError::XstsAdultVerification,
            AppError::NotEntitled,
            AppError::TokenExpired,
            AppError::JavaSetup("adoptium 503".into()),
            AppError::LoaderInstall("installer exit 1".into()),
            AppError::DiskFull { needed_bytes: 10, available_bytes: 1 },
            AppError::CurseforgeOptOut { page_url: "https://cf.example/mod".into() },
            AppError::Internal("boom".into()),
        ];
        for e in cases {
            let json = serde_json::to_value(&e).unwrap();
            assert_eq!(json["code"], e.code(), "직렬화 tag ≠ code(): {e:?}");
        }
    }

    /// E-MF-04 detail은 부분 재시도 UI용 실패 파일 목록을 담는다.
    #[test]
    fn download_failed_detail_carries_file_list() {
        let e = AppError::DownloadFailed { files: vec!["mods/a.jar".into(), "cfg/b.toml".into()] };
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["detail"]["files"][0], "mods/a.jar");
        assert_eq!(json["detail"]["files"][1], "cfg/b.toml");
    }

    /// §9 표: SyncError 각 변형이 올바른 코드로 매핑되는지.
    #[test]
    fn sync_error_maps_to_prd9_codes() {
        use crate::net::NetError;
        let cases: Vec<(SyncError, &str)> = vec![
            (SyncError::Fetch(NetError::Http("timeout".into())), "E-MF-01"),
            (SyncError::Parse("eof".into()), "E-MF-02"),
            (SyncError::UnsupportedFormat(9), "E-MF-03"),
            (SyncError::LauncherTooOld("2.0".into()), "E-MF-03"),
            (SyncError::McTooOld("1.12.2".into()), "E-MF-05"),
            (SyncError::Download { failed: vec!["a".into()] }, "E-MF-04"),
            (SyncError::UnsupportedSource("curseforge".into()), "E-IN-01"),
            (SyncError::Io(std::io::Error::other("io")), "E-IN-01"),
        ];
        for (e, code) in cases {
            assert_eq!(AppError::from(e).code(), code);
        }
    }

    #[test]
    fn prepare_error_download_maps_to_emf04() {
        let e = PrepareError::Download { failed: vec!["lib.jar".into()] };
        assert_eq!(AppError::from(e).code(), "E-MF-04");
        let e = PrepareError::Coord("bad:coord".into());
        assert_eq!(AppError::from(e).code(), "E-IN-01");
    }
}
