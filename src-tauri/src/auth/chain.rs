//! MSA→XBL→XSTS→MC 인증 체인 — PRD §8.3. **사람 리뷰 필수 구역.**
//! 규칙: 이 모듈의 어떤 토큰 값도 로그/에러 메시지에 싣지 않는다 (§8.3, §8.13).
use super::http::{AuthHttp, HttpResponse};
use super::pkce::Pkce;
use super::store::TokenStore;
use super::{AccountProfile, AuthProvider};
use crate::error::AppError;
use serde::Deserialize;
use serde_json::json;

pub const MSA_TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
pub const XBL_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
pub const XSTS_AUTH_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
pub const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
pub const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

/// 브라우저 인터랙션 추상화 — 실구현(시스템 브라우저 + localhost 리스너)은
/// MS 승인(PRD 0-1)으로 client_id 확보 후 작성한다. 구현은 state 대조(CSRF) 책임.
pub trait AuthCodeSource: Send + Sync {
    fn redirect_uri(&self) -> String;
    fn obtain_code(&self, authorize_url: &str, expected_state: &str) -> Result<String, AppError>;
}

#[derive(Deserialize)]
struct MsaTokens {
    access_token: String,
    refresh_token: String,
}

#[derive(Deserialize)]
struct XboxToken {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: DisplayClaims,
}

#[derive(Deserialize)]
struct DisplayClaims {
    xui: Vec<Xui>,
}

#[derive(Deserialize)]
struct Xui {
    uhs: String,
}

#[derive(Deserialize)]
struct XstsErrorBody {
    #[serde(rename = "XErr", default)]
    xerr: u64,
}

#[derive(Deserialize)]
struct McTokens {
    access_token: String,
}

#[derive(Deserialize)]
struct McProfile {
    id: String,
    name: String,
}

/// E-AU-01 — 토큰 값이 섞이지 않도록 단계 이름만 기록한다.
fn step_failed(step: &str, detail: impl std::fmt::Display) -> AppError {
    AppError::AuthFailed(format!("{step}: {detail}"))
}

fn parse_ok<T: serde::de::DeserializeOwned>(step: &str, resp: HttpResponse) -> Result<T, AppError> {
    if !(200..300).contains(&resp.status) {
        return Err(step_failed(step, format!("status {}", resp.status)));
    }
    serde_json::from_slice(&resp.body).map_err(|e| step_failed(step, e))
}

/// XSTS XErr → PRD §9 에러 코드 (§8.3 [v2 필수] 분기).
fn classify_xsts_error(xerr: u64) -> AppError {
    match xerr {
        2148916233 => AppError::XstsNoProfile,         // E-AU-02: Xbox 프로필 없음
        2148916238 => AppError::XstsChildAccount,      // E-AU-03: 자녀 계정
        2148916236 | 2148916237 => AppError::XstsAdultVerification, // E-AU-04: 성인 인증(한국)
        other => step_failed("xsts", format!("XErr {other}")),
    }
}

pub struct PkceAuthProvider<'a> {
    pub http: &'a dyn AuthHttp,
    pub client_id: &'a str,
    pub code_source: &'a dyn AuthCodeSource,
    pub store: &'a dyn TokenStore,
}

impl PkceAuthProvider<'_> {
    fn msa_exchange(&self, form: &[(&str, &str)]) -> Result<HttpResponse, AppError> {
        self.http.post_form(MSA_TOKEN_URL, form).map_err(|e| step_failed("msa-token", e))
    }

    /// MSA access token 이후의 공통 구간: XBL → XSTS → MC 로그인 → 프로필.
    fn chain_from_msa(&self, tokens: &MsaTokens) -> Result<AccountProfile, AppError> {
        // 1) XBL authenticate
        let xbl_body = json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": format!("d={}", tokens.access_token),
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT",
        });
        let resp = self.http.post_json(XBL_AUTH_URL, &xbl_body).map_err(|e| step_failed("xbl", e))?;
        let xbl: XboxToken = parse_ok("xbl", resp)?;

        // 2) XSTS authorize — 401 본문의 XErr로 E-AU-02/03/04 분류
        let xsts_body = json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl.token] },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT",
        });
        let resp =
            self.http.post_json(XSTS_AUTH_URL, &xsts_body).map_err(|e| step_failed("xsts", e))?;
        if resp.status == 401 {
            let body: XstsErrorBody =
                serde_json::from_slice(&resp.body).unwrap_or(XstsErrorBody { xerr: 0 });
            return Err(classify_xsts_error(body.xerr));
        }
        let xsts: XboxToken = parse_ok("xsts", resp)?;
        let uhs = xsts
            .display_claims
            .xui
            .first()
            .map(|x| x.uhs.clone())
            .ok_or_else(|| step_failed("xsts", "missing uhs claim"))?;

        // 3) Minecraft 로그인
        let login_body =
            json!({ "identityToken": format!("XBL3.0 x={uhs};{}", xsts.token) });
        let resp = self
            .http
            .post_json(MC_LOGIN_URL, &login_body)
            .map_err(|e| step_failed("mc-login", e))?;
        let mc: McTokens = parse_ok("mc-login", resp)?;

        // 4) 소유권 판정 = 프로필 조회 성공 여부 (§8.3 [v2 필수] Game Pass 규칙 — E-AU-05)
        let resp = self
            .http
            .get_bearer(MC_PROFILE_URL, &mc.access_token)
            .map_err(|e| step_failed("mc-profile", e))?;
        if !(200..300).contains(&resp.status) {
            return Err(AppError::NotEntitled);
        }
        let profile: McProfile = serde_json::from_slice(&resp.body)
            .map_err(|e| step_failed("mc-profile", e))?;

        Ok(AccountProfile {
            account_id: profile.id.clone(),
            gamertag: profile.name,
            mc_uuid: profile.id,
        })
    }
}

impl AuthProvider for PkceAuthProvider<'_> {
    fn sign_in(&self) -> Result<AccountProfile, AppError> {
        let pkce = Pkce::generate()?;
        let redirect_uri = self.code_source.redirect_uri();
        let url = pkce.authorize_url(self.client_id, &redirect_uri);
        let code = self.code_source.obtain_code(&url, &pkce.state)?;

        let resp = self.msa_exchange(&[
            ("client_id", self.client_id),
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect_uri),
            ("code_verifier", &pkce.verifier),
        ])?;
        let tokens: MsaTokens = parse_ok("msa-token", resp)?;

        let account = self.chain_from_msa(&tokens)?;
        self.store.save_refresh(&account.account_id, &tokens.refresh_token)?;
        Ok(account)
    }

    fn refresh(&self, account_id: &str) -> Result<AccountProfile, AppError> {
        // E-AU-06: 저장된 refresh_token 없음/만료 → 재로그인 유도
        let Some(stored) = self.store.load_refresh(account_id)? else {
            return Err(AppError::TokenExpired);
        };
        let resp = self.msa_exchange(&[
            ("client_id", self.client_id),
            ("grant_type", "refresh_token"),
            ("refresh_token", &stored),
        ])?;
        if !(200..300).contains(&resp.status) {
            return Err(AppError::TokenExpired); // invalid_grant 등 — 재로그인 유도
        }
        let tokens: MsaTokens =
            serde_json::from_slice(&resp.body).map_err(|e| step_failed("msa-token", e))?;

        let account = self.chain_from_msa(&tokens)?;
        self.store.save_refresh(&account.account_id, &tokens.refresh_token)?;
        Ok(account)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::http::test_support::MockAuthHttp;
    use crate::auth::store::test_support::MemoryStore;
    use crate::auth::store::TokenStore;
    use crate::auth::AuthProvider;
    use crate::error::AppError;

    /// 테스트용 코드 소스 — 브라우저 없이 고정 code 반환.
    struct FixedCode(&'static str);
    impl AuthCodeSource for FixedCode {
        fn redirect_uri(&self) -> String {
            "http://127.0.0.1:43210/cb".into()
        }
        fn obtain_code(&self, authorize_url: &str, expected_state: &str) -> Result<String, AppError> {
            assert!(authorize_url.contains(&format!("state={expected_state}")));
            Ok(self.0.to_string())
        }
    }

    fn happy_http() -> MockAuthHttp {
        MockAuthHttp::with(&[
            (
                MSA_TOKEN_URL,
                200,
                br#"{"access_token": "MSA_ACCESS", "refresh_token": "MSA_REFRESH", "expires_in": 3600}"#,
            ),
            (
                XBL_AUTH_URL,
                200,
                br#"{"Token": "XBL_TOKEN", "DisplayClaims": {"xui": [{"uhs": "UHS1"}]}}"#,
            ),
            (
                XSTS_AUTH_URL,
                200,
                br#"{"Token": "XSTS_TOKEN", "DisplayClaims": {"xui": [{"uhs": "UHS1"}]}}"#,
            ),
            (
                MC_LOGIN_URL,
                200,
                br#"{"access_token": "MC_ACCESS", "expires_in": 86400}"#,
            ),
            (
                MC_PROFILE_URL,
                200,
                br#"{"id": "b876ec32e396476ba1158438d83c67d4", "name": "Player1"}"#,
            ),
        ])
    }

    fn provider<'a>(
        http: &'a MockAuthHttp,
        code: &'a FixedCode,
        store: &'a MemoryStore,
    ) -> PkceAuthProvider<'a> {
        PkceAuthProvider { http, client_id: "client-abc", code_source: code, store }
    }

    #[test]
    fn sign_in_runs_full_chain_and_stores_refresh_token() {
        let http = happy_http();
        let code = FixedCode("AUTHCODE9");
        let store = MemoryStore::default();
        let account = provider(&http, &code, &store).sign_in().unwrap();

        assert_eq!(account.account_id, "b876ec32e396476ba1158438d83c67d4");
        assert_eq!(account.gamertag, "Player1");
        assert_eq!(account.mc_uuid, "b876ec32e396476ba1158438d83c67d4");

        // MSA 교환: authorization_code + PKCE verifier + 동일 redirect_uri
        let token_req = &http.sent_to(MSA_TOKEN_URL)[0];
        for needle in [
            "grant_type=authorization_code",
            "code=AUTHCODE9",
            "client_id=client-abc",
            "code_verifier=",
            "redirect_uri=http://127.0.0.1:43210/cb",
        ] {
            assert!(token_req.contains(needle), "missing {needle} in {token_req}");
        }
        // XBL: RpsTicket에 MSA access token ("d=" 접두)
        assert!(http.sent_to(XBL_AUTH_URL)[0].contains(r#""RpsTicket":"d=MSA_ACCESS""#));
        // XSTS: XBL 토큰 + minecraftservices RP
        let xsts_req = &http.sent_to(XSTS_AUTH_URL)[0];
        assert!(xsts_req.contains("XBL_TOKEN"));
        assert!(xsts_req.contains("rp://api.minecraftservices.com/"));
        // MC 로그인: XBL3.0 x=<uhs>;<xsts>
        assert!(http.sent_to(MC_LOGIN_URL)[0].contains("XBL3.0 x=UHS1;XSTS_TOKEN"));
        // 프로필: MC access token bearer
        assert_eq!(http.sent_to(MC_PROFILE_URL)[0], "bearer:MC_ACCESS");
        // refresh_token은 계정 id 키로 저장 (§8.3)
        assert_eq!(
            store.load_refresh("b876ec32e396476ba1158438d83c67d4").unwrap().as_deref(),
            Some("MSA_REFRESH")
        );
    }

    #[test]
    fn xsts_xerr_maps_to_prd_error_codes() {
        // PRD §8.3 XSTS 에러 처리 [v2 필수] — §9 E-AU-02/03/04
        let cases: &[(u64, fn(&AppError) -> bool)] = &[
            (2148916233, |e| matches!(e, AppError::XstsNoProfile)),
            (2148916238, |e| matches!(e, AppError::XstsChildAccount)),
            (2148916236, |e| matches!(e, AppError::XstsAdultVerification)),
            (2148916237, |e| matches!(e, AppError::XstsAdultVerification)),
            (999, |e| matches!(e, AppError::AuthFailed(_))),
        ];
        for (xerr, check) in cases {
            let mut http = happy_http();
            http.responses.insert(
                XSTS_AUTH_URL.to_string(),
                (401, format!(r#"{{"XErr": {xerr}, "Message": "denied"}}"#).into_bytes()),
            );
            let code = FixedCode("C");
            let store = MemoryStore::default();
            let err = provider(&http, &code, &store).sign_in().unwrap_err();
            assert!(check(&err), "XErr {xerr} → {err:?}");
        }
    }

    #[test]
    fn profile_failure_means_not_entitled_game_pass_rule() {
        // PRD §8.3 [v2 필수]: 소유권 판정은 프로필 조회 성공 여부 — entitlement로 차단 금지
        let mut http = happy_http();
        http.responses.insert(MC_PROFILE_URL.to_string(), (404, b"{}".to_vec()));
        let code = FixedCode("C");
        let store = MemoryStore::default();
        let err = provider(&http, &code, &store).sign_in().unwrap_err();
        assert!(matches!(err, AppError::NotEntitled), "got {err:?}");
    }

    #[test]
    fn refresh_uses_stored_token_and_rotates_it() {
        let mut http = happy_http();
        http.responses.insert(
            MSA_TOKEN_URL.to_string(),
            (
                200,
                br#"{"access_token": "MSA_ACCESS2", "refresh_token": "ROTATED", "expires_in": 3600}"#
                    .to_vec(),
            ),
        );
        let code = FixedCode("MUST-NOT-BE-USED");
        let store = MemoryStore::default();
        store.save_refresh("b876ec32e396476ba1158438d83c67d4", "OLD_REFRESH").unwrap();

        let account = provider(&http, &code, &store)
            .refresh("b876ec32e396476ba1158438d83c67d4")
            .unwrap();
        assert_eq!(account.gamertag, "Player1");

        let token_req = &http.sent_to(MSA_TOKEN_URL)[0];
        assert!(token_req.contains("grant_type=refresh_token"));
        assert!(token_req.contains("refresh_token=OLD_REFRESH"));
        // 회전된 refresh_token 저장 (§8.3)
        assert_eq!(
            store.load_refresh("b876ec32e396476ba1158438d83c67d4").unwrap().as_deref(),
            Some("ROTATED")
        );
    }

    #[test]
    fn refresh_without_stored_token_is_token_expired() {
        let http = happy_http();
        let code = FixedCode("C");
        let store = MemoryStore::default();
        let err = provider(&http, &code, &store).refresh("nobody").unwrap_err();
        assert!(matches!(err, AppError::TokenExpired), "E-AU-06, got {err:?}");
    }

    #[test]
    fn expired_refresh_grant_is_token_expired() {
        let mut http = happy_http();
        http.responses.insert(
            MSA_TOKEN_URL.to_string(),
            (400, br#"{"error": "invalid_grant"}"#.to_vec()),
        );
        let code = FixedCode("C");
        let store = MemoryStore::default();
        store.save_refresh("acc", "STALE").unwrap();
        let err = provider(&http, &code, &store).refresh("acc").unwrap_err();
        assert!(matches!(err, AppError::TokenExpired), "E-AU-06, got {err:?}");
    }
}
