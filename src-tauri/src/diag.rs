//! 진단 — PRD 8.12: 로그/크래시 리포트는 외부 전송 없이 로컬 보관.
//! - tracing 일자별 로테이션 파일 로거 + 보관 기간 초과 로그 정리
//! - 런처 크래시: panic hook → 로컬 JSON
//! - 로그 출력 전 민감 정보(토큰/이메일) 마스킹 유틸 (§8.3, §8.13)
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// 로그 보관 기간 기본값 — 설정 UI(§8.12 "보관 기간 설정") 연결 전까지 고정.
pub const DEFAULT_RETENTION_DAYS: u64 = 14;

/// non_blocking writer의 flush 수명 — Tauri managed state로 보관해 앱 종료까지 유지.
pub struct LogGuard(#[allow(dead_code)] pub tracing_appender::non_blocking::WorkerGuard);

/// 파일 로거 초기화. 실패해도 앱은 계속 뜬다(로깅은 부가 기능) — None 반환.
pub fn init_logging(logs_dir: &Path) -> Option<LogGuard> {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    fs::create_dir_all(logs_dir).ok()?;
    cleanup_old_logs(logs_dir, DEFAULT_RETENTION_DAYS);
    let (writer, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::daily(logs_dir, "aqua.log"));
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(tracing_subscriber::fmt::layer().with_ansi(false).with_writer(writer))
        .try_init()
        .ok()?;
    Some(LogGuard(guard))
}

/// 보관 기간을 넘긴 로그 파일 삭제 (수정 시각 기준).
pub fn cleanup_old_logs(logs_dir: &Path, retention_days: u64) {
    let cutoff = Duration::from_secs(retention_days * 24 * 3600);
    let Ok(entries) = fs::read_dir(logs_dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_log = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("aqua.log"));
        if !is_log {
            continue; // 크래시 JSON 등 다른 산출물은 건드리지 않는다
        }
        let expired = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > cutoff);
        if expired {
            let _ = fs::remove_file(&path);
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 런처 panic → 로컬 JSON (PRD 8.12). 기존 훅(콘솔 출력)은 이어서 호출.
pub fn install_panic_hook(crash_dir: PathBuf) {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic payload".into());
        let report = serde_json::json!({
            "at_unix": unix_now(),
            "message": mask(&msg),
            "location": info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())),
            "thread": std::thread::current().name().unwrap_or("unnamed").to_string(),
            "launcher_version": env!("CARGO_PKG_VERSION"),
        });
        let _ = fs::create_dir_all(&crash_dir);
        let _ = fs::write(
            crash_dir.join(format!("launcher-crash-{}.json", unix_now())),
            serde_json::to_vec_pretty(&report).unwrap_or_default(),
        );
        tracing::error!(message = %mask(&msg), "launcher panic");
        prev(info);
    }));
}

/// 토큰으로 간주해 마스킹하는 최소 길이 — MSA/MC 토큰은 수백 자, 해시는 40~64자.
const TOKEN_MIN_LEN: usize = 40;

/// 로그 출력 전 민감 정보 마스킹 (§8.3, §8.13 — 파일/로그 평문 저장 금지).
/// - 이메일: local part를 첫 글자만 남기고 마스킹
/// - 40자 이상 토큰형 문자열(Base64/JWT/해시): 앞 6자만 남기고 마스킹
pub fn mask(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut run = String::new();
    let is_run_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '+' | '@');
    for c in s.chars() {
        if is_run_char(c) {
            run.push(c);
        } else {
            flush_run(&mut out, &run);
            run.clear();
            out.push(c);
        }
    }
    flush_run(&mut out, &run);
    out
}

fn flush_run(out: &mut String, run: &str) {
    if run.is_empty() {
        return;
    }
    // 이메일: a@b.c 형태 — local part 마스킹, 도메인은 유지(진단 가치)
    if let Some(at) = run.find('@') {
        let (local, rest) = run.split_at(at);
        if !local.is_empty() && rest.len() > 1 && rest[1..].contains('.') {
            let first = local.chars().next().unwrap();
            out.push(first);
            out.push_str("***");
            out.push_str(rest);
            return;
        }
    }
    if run.len() >= TOKEN_MIN_LEN {
        out.push_str(&run[..6]);
        out.push_str("…[masked]");
        return;
    }
    out.push_str(run);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_email_local_part() {
        assert_eq!(mask("user login: steve.kr@example.com ok"), "user login: s***@example.com ok");
    }

    #[test]
    fn masks_long_token_like_strings() {
        let jwt = format!("eyJhbG{}", "x".repeat(200));
        let masked = mask(&format!("Authorization: Bearer {jwt}"));
        assert!(masked.contains("eyJhbG…[masked]"), "{masked}");
        assert!(!masked.contains("xxxx"), "토큰 본문이 남으면 안 된다");
    }

    #[test]
    fn masks_token_in_json_value() {
        let token = "A".repeat(64);
        let masked = mask(&format!("{{\"access_token\":\"{token}\"}}"));
        assert!(!masked.contains(&token));
        assert!(masked.contains("[masked]"));
    }

    #[test]
    fn leaves_normal_text_untouched() {
        let s = "sync ok: mods/sodium-0.5.8.jar (1.2 MB) — 초록 마을";
        assert_eq!(mask(s), s);
    }

    #[test]
    fn cleanup_removes_only_expired_aqua_logs() {
        let dir = tempfile::tempdir().unwrap();
        let old_log = dir.path().join("aqua.log.2020-01-01");
        let crash = dir.path().join("launcher-crash-1.json");
        fs::write(&old_log, "x").unwrap();
        fs::write(&crash, "x").unwrap();
        // 수정 시각을 과거로 되돌릴 표준 API가 없어 retention 0일로 만료를 재현
        cleanup_old_logs(dir.path(), 0);
        std::thread::sleep(Duration::from_millis(1100)); // mtime 해상도(1s) 초과 대기
        cleanup_old_logs(dir.path(), 0);
        assert!(!old_log.exists(), "만료된 aqua.log.*는 삭제");
        assert!(crash.exists(), "크래시 리포트는 보존");
    }
}
