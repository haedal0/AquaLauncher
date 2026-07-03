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

**M0 (스파이크 단계)** — PRD §13 마일스톤 참고.

| 마일스톤 | 내용 | 상태 |
|---|---|---|
| M0 | MS 승인 신청 / CF 키 신청 / Forge headless 스파이크 | 스파이크 완료([보고서](docs/spike-forge-report.md)) — 외부 신청 3건 대기 |
| M1 | 실행 파이프라인(바닐라) + 공유 캐시 + Java 관리 | **mac 게이트 통과** — 바닐라 1.20.4 실기동 검증(38.9s 콜드부팅, `m1_gate` ignored 테스트). Windows 게이트 남음 |
| M2 | Fabric/Quilt + 동기화 엔진 + CLI | 진행 중 — 엔진 E2E(목)·Fabric/Quilt·CLI scan/validate 완료. 남음: modrinth 소스 해석, CLI init/diff/add-modrinth, 픽스처 서버 E2E |
| M4 일부 | 프론트엔드 1차 + Tauri 결선 | UI 전체(mockup 기준) + 커맨드 8종 결선(list/create/preview/toggle/reset/sync/play), 진행 이벤트 100ms 스로틀, 브라우저 dev는 목 폴백. 실앱 기동 검증 완료 |
| M3 | 인증 + Forge/NeoForge | 대기 |
| M4 | 딥링크, 테마, 옵셔널 모드, 에러 처리, i18n | 대기 |
| M5 | 자동 업데이트, 서명 배포 | 대기 |
