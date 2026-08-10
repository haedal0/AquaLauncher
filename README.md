# AquaLauncher

서버 운영자가 배포한 manifest.json 하나로 모드/리소스팩/설정이 자동 동기화되는 멀티 인스턴스 마인크래프트 런처.

- 제품 명세: [docs/PRD_v2.md](docs/PRD_v2.md)
- UI 레퍼런스: [docs/mockup.html](docs/mockup.html)
- 에이전트 작업 규칙: [AGENT.md](AGENT.md)

## 개발 시작

```bash
pnpm install
cargo test --workspace          # Rust 테스트
pnpm check                      # 프론트 타입체크
cargo run -p aqua-fixtures &    # 픽스처 매니페스트 서버 (127.0.0.1:8750)
pnpm tauri dev                  # 앱 실행
```

## 현재 상태

**M4 진행 중** — 마일스톤 정의는 PRD §13 참고.

| 마일스톤 | 내용 | 상태 |
|---|---|---|
| M0 | MS 승인 신청 / CF 키 신청 / Forge headless 스파이크 | 완료 — [스파이크 보고서](docs/spike-forge-report.md). 외부 신청 2건은 회신 대기 |
| M1 | 실행 파이프라인(바닐라) + 공유 캐시 + Java 관리 | mac 게이트 통과 (바닐라 1.20.4 콜드부팅 38.9s, `m1_gate` ignored 테스트). Windows 게이트 남음 |
| M2 | Fabric/Quilt + 동기화 엔진 + CLI | 엔진·로더·CLI scan/validate/landing 완료. 남음: modrinth 소스 해석, CLI init/diff/add-modrinth, `validate --check-urls` |
| M3 | 인증(PKCE) + Forge/NeoForge | 로더 완료. 인증은 `feat/auth-pkce` 브랜치 리뷰 대기, 실계정 접속은 MS 승인 후 |
| M4 | 딥링크, 테마, 옵셔널 모드, 에러 처리, i18n | 진행 중 — 프론트 전체 + 커맨드 결선, 딥링크, §9 에러 코드, 진단 로거, 옵셔널 모드 UI 완료. 게이트(파일럿 서버 클로즈드 베타) 남음 |
| M5 | 자동 업데이트(minisign), 배포 파이프라인 | 대기 |
