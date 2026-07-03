//! refresh_token 보관 — **사람 리뷰 필수 구역.**
//! PRD §8.3 확정: OS 자격증명 저장소(Windows Credential Manager / macOS Keychain)만 사용.
//! 파일/로그 평문 저장은 어떤 경로로도 금지 (§8.13). 값은 Debug/Display 구현체에 싣지 않는다.
use crate::error::AppError;

pub trait TokenStore: Send + Sync {
    fn save_refresh(&self, account_id: &str, refresh_token: &str) -> Result<(), AppError>;
    /// 없으면 Ok(None) — 만료/미로그인 판단은 호출측(E-AU-06).
    fn load_refresh(&self, account_id: &str) -> Result<Option<String>, AppError>;
    fn delete_refresh(&self, account_id: &str) -> Result<(), AppError>;
}

const SERVICE: &str = "app.aqualauncher.msa-refresh";

/// OS 자격증명 저장소 구현 (keyring).
pub struct KeyringStore;

impl KeyringStore {
    fn entry(account_id: &str) -> Result<keyring::Entry, AppError> {
        keyring::Entry::new(SERVICE, account_id)
            .map_err(|e| AppError::AuthFailed(format!("credential store unavailable: {e}")))
    }
}

impl TokenStore for KeyringStore {
    fn save_refresh(&self, account_id: &str, refresh_token: &str) -> Result<(), AppError> {
        Self::entry(account_id)?
            .set_password(refresh_token)
            .map_err(|e| AppError::AuthFailed(format!("credential store write: {e}")))
    }

    fn load_refresh(&self, account_id: &str) -> Result<Option<String>, AppError> {
        match Self::entry(account_id)?.get_password() {
            Ok(t) => Ok(Some(t)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AppError::AuthFailed(format!("credential store read: {e}"))),
        }
    }

    fn delete_refresh(&self, account_id: &str) -> Result<(), AppError> {
        match Self::entry(account_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AppError::AuthFailed(format!("credential store delete: {e}"))),
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// 테스트용 인메모리 저장소 — 프로덕션 사용 금지 (§8.3).
    #[derive(Default)]
    pub struct MemoryStore(pub Mutex<BTreeMap<String, String>>);

    impl TokenStore for MemoryStore {
        fn save_refresh(&self, account_id: &str, refresh_token: &str) -> Result<(), AppError> {
            self.0.lock().unwrap().insert(account_id.into(), refresh_token.into());
            Ok(())
        }
        fn load_refresh(&self, account_id: &str) -> Result<Option<String>, AppError> {
            Ok(self.0.lock().unwrap().get(account_id).cloned())
        }
        fn delete_refresh(&self, account_id: &str) -> Result<(), AppError> {
            self.0.lock().unwrap().remove(account_id);
            Ok(())
        }
    }
}
