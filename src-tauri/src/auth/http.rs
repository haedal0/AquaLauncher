//! 인증 체인용 HTTP 추상화 — **사람 리뷰 필수 구역.**
//! `net::Fetch`(GET 스트리밍)와 달리 POST + 상태코드가 필요하다:
//! XSTS는 401 본문의 XErr로, 프로필은 404로 분기한다 (PRD §8.3).
//! URL 정책(§11 https 강제)은 여기서도 동일하게 적용된다.
use crate::net::{HttpFetcher, NetError};
use aqua_manifest::urlpolicy::validate_distribution_url;

pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub trait AuthHttp: Send + Sync {
    fn post_json(&self, url: &str, body: &serde_json::Value) -> Result<HttpResponse, NetError>;
    /// application/x-www-form-urlencoded (MSA 토큰 엔드포인트)
    fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<HttpResponse, NetError>;
    fn get_bearer(&self, url: &str, token: &str) -> Result<HttpResponse, NetError>;
}

fn to_response(resp: reqwest::blocking::Response) -> Result<HttpResponse, NetError> {
    let status = resp.status().as_u16();
    let body = resp.bytes().map_err(|e| NetError::Http(e.to_string()))?.to_vec();
    Ok(HttpResponse { status, body })
}

impl AuthHttp for HttpFetcher {
    fn post_json(&self, url: &str, body: &serde_json::Value) -> Result<HttpResponse, NetError> {
        validate_distribution_url(url)?;
        let payload = serde_json::to_vec(body).map_err(|e| NetError::Http(e.to_string()))?;
        let resp = self
            .client()
            .post(url)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .body(payload)
            .send()
            .map_err(|e| NetError::Http(e.to_string()))?;
        to_response(resp)
    }

    fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<HttpResponse, NetError> {
        validate_distribution_url(url)?;
        let resp = self
            .client()
            .post(url)
            .form(form)
            .send()
            .map_err(|e| NetError::Http(e.to_string()))?;
        to_response(resp)
    }

    fn get_bearer(&self, url: &str, token: &str) -> Result<HttpResponse, NetError> {
        validate_distribution_url(url)?;
        let resp = self
            .client()
            .get(url)
            .bearer_auth(token)
            .send()
            .map_err(|e| NetError::Http(e.to_string()))?;
        to_response(resp)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// url → (status, body). 요청 본문을 기록해 체인이 보낸 내용을 검증한다.
    pub struct MockAuthHttp {
        pub responses: BTreeMap<String, (u16, Vec<u8>)>,
        pub requests: Mutex<Vec<(String, String)>>,
    }

    impl MockAuthHttp {
        pub fn with(pairs: &[(&str, u16, &[u8])]) -> Self {
            MockAuthHttp {
                responses: pairs
                    .iter()
                    .map(|(u, s, b)| (u.to_string(), (*s, b.to_vec())))
                    .collect(),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn respond(&self, url: &str, sent: String) -> Result<HttpResponse, NetError> {
            self.requests.lock().unwrap().push((url.to_string(), sent));
            match self.responses.get(url) {
                Some((status, body)) => {
                    Ok(HttpResponse { status: *status, body: body.clone() })
                }
                None => Err(NetError::Status { url: url.to_string(), status: 404 }),
            }
        }

        pub fn sent_to(&self, url: &str) -> Vec<String> {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .filter(|(u, _)| u == url)
                .map(|(_, b)| b.clone())
                .collect()
        }
    }

    impl AuthHttp for MockAuthHttp {
        fn post_json(&self, url: &str, body: &serde_json::Value) -> Result<HttpResponse, NetError> {
            self.respond(url, body.to_string())
        }
        fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<HttpResponse, NetError> {
            let encoded: Vec<String> =
                form.iter().map(|(k, v)| format!("{k}={v}")).collect();
            self.respond(url, encoded.join("&"))
        }
        fn get_bearer(&self, url: &str, token: &str) -> Result<HttpResponse, NetError> {
            self.respond(url, format!("bearer:{token}"))
        }
    }
}
