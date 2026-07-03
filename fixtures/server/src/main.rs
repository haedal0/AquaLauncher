//! 테스트용 매니페스트 픽스처 서버 — fixtures/data/ 를 정적 서빙.
//! 동기화 엔진(M2) 개발 내내 사용. 프로덕션 코드 아님.
use std::path::Path;

fn main() {
    let addr = "127.0.0.1:8750";
    let server = tiny_http::Server::http(addr).expect("bind failed");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../data");
    println!("aqua-fixtures serving {} on http://{addr}", root.display());

    for req in server.incoming_requests() {
        let url = req.url().trim_start_matches('/');
        // 픽스처 전용 최소 방어 — 프로덕션 코드는 aqua_manifest::path::safe_join 사용
        if url.contains("..") || url.is_empty() {
            let _ = req.respond(tiny_http::Response::empty(400));
            continue;
        }
        match std::fs::read(root.join(url)) {
            Ok(bytes) => { let _ = req.respond(tiny_http::Response::from_data(bytes)); }
            Err(_) => { let _ = req.respond(tiny_http::Response::empty(404)); }
        }
    }
}
