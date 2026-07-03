//! 게임 실행 파이프라인 — PRD 8.15, TD-01 (v1 승인 2026-07-03).
//!
//! 조립(순수 함수: version_json/rules/classpath/args)과 프로세스 실행을 분리한다.
//! 주의(PRD 8.6): 접속 인자는 버전 분기 필수 — args::join_args만 사용.
//! 주의(PRD 8.15-5): auth 토큰은 SubstitutedArgs::display_masked()로만 로그에 출력.

pub mod args;
pub mod artifacts;
pub mod classpath;
pub mod install;
pub mod java;
pub mod process;
pub mod rules;
pub mod version_json;

use std::path::PathBuf;

/// 실행 계획 — 조립 결과의 불변 스냅샷 (TD-01 §0).
#[allow(dead_code)]
pub struct LaunchPlan {
    pub java_path: PathBuf,
    pub jvm_args: args::SubstitutedArgs,
    pub main_class: String,
    pub game_args: args::SubstitutedArgs,
    /// 인스턴스 디렉토리 (game_directory 격리, PRD 8.15-7)
    pub cwd: PathBuf,
}

/// Java 런타임 확보 — PRD 8.4. 외부 API(Adoptium)는 trait 추상화 + 목 동반 (AGENT.md).
pub trait JavaRuntimeProvider: Send + Sync {
    /// 메이저 버전에 맞는 java 실행 파일 경로. 실패는 E-JV-01.
    fn resolve(&self, major: u32) -> Result<PathBuf, crate::error::AppError>;
}

/// 테스트/개발용 목 — 항상 고정 경로 반환.
pub struct MockJavaProvider(pub PathBuf);

impl JavaRuntimeProvider for MockJavaProvider {
    fn resolve(&self, _major: u32) -> Result<PathBuf, crate::error::AppError> {
        Ok(self.0.clone())
    }
}
