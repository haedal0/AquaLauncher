//! aqua-cli — 서버 운영자용 매니페스트 도구 (PRD 10.1)
//! 원칙: 운영자는 해시/용량을 손으로 계산할 일이 없어야 한다.
mod landing;
mod scan;
mod validate;

use aqua_manifest::manifest::Manifest;
use clap::{Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "aqua-cli", about = "AquaLauncher 서버 운영자 도구")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 대화형으로 서버 정보/테마 입력 -> manifest.json 골격 생성
    Init,
    /// mods/, config/ 스캔 -> 해시/용량 자동 계산, mods/files 섹션 갱신
    Scan {
        instance_dir: PathBuf,
        /// 갱신할 manifest.json (기본: <instance_dir>/manifest.json)
        #[arg(long)]
        manifest: Option<PathBuf>,
        /// 신규 항목의 다운로드 URL 프리픽스 (없으면 CHANGE-ME 플레이스홀더)
        #[arg(long)]
        base_url: Option<String>,
    },
    /// Modrinth API에서 project/version id, 해시 자동 채움
    AddModrinth { slug: String },
    /// 스키마 검증 + 경로/URL 정책 + 중복 검사 (URL HEAD 체크는 후속)
    Validate { manifest: PathBuf },
    /// 사용자에게 배포될 변경 요약 미리보기
    Diff { old: PathBuf, new: PathBuf },
    /// 딥링크 랜딩 페이지 정적 HTML 생성 (PRD 8.7)
    Landing {
        /// 배포된 manifest.json의 https URL (딥링크에 포함)
        manifest_url: String,
        /// 서버 이름/설명을 읽어올 로컬 manifest.json (생략 시 URL 표기만)
        #[arg(long)]
        manifest: Option<PathBuf>,
        /// 런처 다운로드 페이지 URL (미설치 사용자 안내 버튼)
        #[arg(long)]
        download_url: Option<String>,
        /// 출력 파일
        #[arg(long, default_value = "landing.html")]
        out: PathBuf,
    },
}

fn load_manifest(path: &PathBuf) -> Result<Manifest, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("{}: 파싱 실패 — {e}", path.display()))
}

fn not_implemented(what: &str) -> ExitCode {
    eprintln!("aqua-cli {what}: 아직 구현되지 않았습니다 (PRD 10.1 후속 작업)");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Init => not_implemented("init"),
        Cmd::Scan { instance_dir, manifest, base_url } => {
            let manifest_path = manifest.unwrap_or_else(|| instance_dir.join("manifest.json"));
            let mut m = match load_manifest(&manifest_path) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("오류: {e}");
                    eprintln!("힌트: manifest.json이 없으면 먼저 골격을 만들어 두세요 (init은 후속 구현).");
                    return ExitCode::FAILURE;
                }
            };
            match scan::scan(&instance_dir, &mut m, base_url.as_deref()) {
                Ok(report) => {
                    let pretty = serde_json::to_string_pretty(&m).expect("serialize");
                    if let Err(e) = fs::write(&manifest_path, pretty + "\n") {
                        eprintln!("쓰기 실패: {e}");
                        return ExitCode::FAILURE;
                    }
                    println!(
                        "scan 완료: 갱신 {}건 / 추가 {}건 / 제거 {}건 → {}",
                        report.updated, report.added, report.removed,
                        manifest_path.display()
                    );
                    if report.added > 0 && base_url.is_none() {
                        println!("주의: 신규 항목의 URL이 {} 플레이스홀더입니다. 실제 주소로 바꾸세요.", scan::PLACEHOLDER_BASE);
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("scan 실패: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Cmd::AddModrinth { .. } => not_implemented("add-modrinth"),
        Cmd::Validate { manifest } => {
            let m = match load_manifest(&manifest) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("오류(E-MF-02 상당): {e}");
                    return ExitCode::FAILURE;
                }
            };
            let issues = validate::validate(&m);
            if issues.is_empty() {
                println!("validate 통과: {}", manifest.display());
                ExitCode::SUCCESS
            } else {
                eprintln!("validate 실패 — {}건:", issues.len());
                for issue in &issues {
                    eprintln!("  - {issue}");
                }
                ExitCode::FAILURE
            }
        }
        Cmd::Diff { .. } => not_implemented("diff"),
        Cmd::Landing { manifest_url, manifest, download_url, out } => {
            let (name, desc) = match manifest.as_ref().map(load_manifest) {
                Some(Ok(m)) => (m.server_display_name, m.server_description),
                Some(Err(e)) => {
                    eprintln!("오류: {e}");
                    return ExitCode::FAILURE;
                }
                None => ("마인크래프트 서버".to_string(), None),
            };
            match landing::render(&name, desc.as_deref(), &manifest_url, download_url.as_deref())
            {
                Ok(html) => {
                    if let Err(e) = fs::write(&out, html) {
                        eprintln!("쓰기 실패: {e}");
                        return ExitCode::FAILURE;
                    }
                    println!("landing 생성 완료: {} (https로 호스팅해 배포하세요)", out.display());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("landing 실패: {e}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}
