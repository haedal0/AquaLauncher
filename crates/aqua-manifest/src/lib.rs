//! AquaLauncher 공유 데이터 모델 — PRD 7장 스키마 3종 + 경로 안전 유틸.
//!
//! 이 크레이트는 런처(src-tauri)와 운영자 CLI(cli)가 공유한다.
//! **스키마 변경은 사람 리뷰 필수 (AGENT.md 참고). 변경 시 PRD 7장 갱신 diff 동반.**

pub mod instance;
pub mod lockfile;
pub mod manifest;
pub mod path;
pub mod urlpolicy;
