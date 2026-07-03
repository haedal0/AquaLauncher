//! 프로세스 수명 — TD-01 §8, PRD §8.12 (크래시 루프 방지), §8.15-6.
//!
//! 로그로 나가는 인자는 반드시 SubstitutedArgs::display_masked() 경유 —
//! 이 모듈은 인자를 그대로 로그에 남기지 않는다.
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;

/// 게임 프로세스를 spawn하고 stdout/stderr 라인을 on_line으로 전달, 종료 코드를 반환.
/// 런처 종료 시 게임을 죽이지 않는 정책(TD-01 §8-5)은 호출 계층에서 detach로 처리.
pub fn spawn_and_wait(
    program: &Path,
    args: &[String],
    cwd: &Path,
    mut on_line: impl FnMut(String),
) -> std::io::Result<Option<i32>> {
    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let (tx, rx) = mpsc::channel::<String>();
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        }));
    }
    if let Some(err) = child.stderr.take() {
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        }));
    }
    drop(child.stdout.take());
    for line in rx {
        on_line(line);
    }
    for r in readers {
        let _ = r.join();
    }
    let status = child.wait()?;
    Ok(status.code())
}

/// 크래시 루프 가드 — 동일 인스턴스가 5분 내 3회 비정상 종료하면 자동 재실행 중단 (E-GM-02).
/// 시계는 초 단위로 주입받아 테스트 가능하게 한다.
pub struct CrashTracker {
    window_secs: u64,
    threshold: usize,
    events: Vec<u64>,
}

impl Default for CrashTracker {
    fn default() -> Self {
        CrashTracker { window_secs: 300, threshold: 3, events: Vec::new() }
    }
}

impl CrashTracker {
    /// 비정상 종료를 기록하고, 크래시 루프(차단 필요)면 true.
    pub fn record_abnormal_exit(&mut self, now_secs: u64) -> bool {
        self.events.push(now_secs);
        self.events
            .retain(|&t| now_secs.saturating_sub(t) < self.window_secs);
        self.events.len() >= self.threshold
    }

    /// 정상 실행 성공 시 카운터 초기화.
    pub fn reset(&mut self) {
        self.events.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_output_and_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let mut lines = Vec::new();
        let code = spawn_and_wait(
            Path::new("/bin/sh"),
            &["-c".into(), "echo out-line; echo err-line 1>&2; exit 3".into()],
            dir.path(),
            |l| lines.push(l),
        )
        .unwrap();
        assert_eq!(code, Some(3));
        assert!(lines.contains(&"out-line".to_string()));
        assert!(lines.contains(&"err-line".to_string()));
    }

    #[test]
    fn crash_loop_blocks_after_3_in_5_minutes() {
        let mut t = CrashTracker::default();
        assert!(!t.record_abnormal_exit(1000));
        assert!(!t.record_abnormal_exit(1060));
        assert!(t.record_abnormal_exit(1120), "3rd crash within 5min -> block");
    }

    #[test]
    fn spread_out_crashes_do_not_block() {
        let mut t = CrashTracker::default();
        assert!(!t.record_abnormal_exit(0));
        assert!(!t.record_abnormal_exit(301));
        assert!(!t.record_abnormal_exit(700), "old events fall out of window");
    }

    #[test]
    fn reset_clears_history() {
        let mut t = CrashTracker::default();
        t.record_abnormal_exit(10);
        t.record_abnormal_exit(20);
        t.reset();
        assert!(!t.record_abnormal_exit(30));
    }
}
