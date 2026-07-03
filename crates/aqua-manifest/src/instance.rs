//! instance.json 스키마 — PRD 7.2
use crate::manifest::LoaderRef;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub name_user_edited: bool,
    pub minecraft_version: String,
    pub loader: LoaderRef,

    /// 수동 인스턴스(PRD 8.8 직접 만들기)는 None
    #[serde(default)]
    pub manifest_source: Option<String>,
    /// 마지막 적용 매니페스트 SHA-256 — 콘텐츠 변경 감지는 해시 비교 (PRD 7.2)
    #[serde(default)]
    pub manifest_hash: Option<String>,
    #[serde(default)]
    pub manifest_display_version: Option<String>,
    #[serde(default)]
    pub manifest_pinned: bool,
    #[serde(default = "yes")]
    pub auto_update: bool,
    #[serde(default)]
    pub last_synced_at: Option<String>,
    pub install_state: InstallState,

    #[serde(default)]
    pub server: Option<InstanceServer>,
    #[serde(default)]
    pub jvm: Option<JvmSettings>,
    #[serde(default)]
    pub jvm_args_override: Option<Vec<String>>,
    #[serde(default)]
    pub java_version: Option<u32>,
    #[serde(default)]
    pub java_path_override: Option<String>,
    #[serde(default)]
    pub last_account_id: Option<String>,
    /// 옵셔널 모드 선택 결과: mod id -> 설치 여부 (PRD 7.2)
    #[serde(default)]
    pub optional_mods_selection: std::collections::BTreeMap<String, bool>,
    pub created_at: String,
    #[serde(default)]
    pub last_played: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}
fn yes() -> bool { true }

/// 설치/동기화 중 크래시 복구 판단용 — PRD 7.2
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallState { Installing, Ready, Broken }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceServer {
    pub address: String,
    pub port: u16,
    #[serde(default)]
    pub direct_connect_override: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JvmSettings {
    pub min_memory_mb: u32,
    pub max_memory_mb: u32,
    #[serde(default)]
    pub extra_args: Vec<String>,
}
