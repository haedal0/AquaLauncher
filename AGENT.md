# AGENT.md — AquaLauncher

에이전트 작업 규칙. 모든 코딩 에이전트 세션에서 이 파일이 우선 적용된다.

## 프로젝트 한 줄 요약

서버 운영자가 배포한 manifest.json 하나로 모드/설정이 자동 동기화되는 멀티 인스턴스 마인크래프트 런처 **AquaLauncher**. Tauri(Rust) + Svelte. 지원: Windows/macOS, MC 1.13+.

## 필수 문서 — 작업 전 반드시 읽기

- `docs/PRD_v2.md` — 제품 명세의 단일 진실 공급원(SSOT). **모든 작업은 해당 PRD 섹션을 먼저 읽고 시작한다.**
- `docs/TD-01-launch.md` — 실행 파이프라인 설계 (승인 전 launch/ 구현 금지, 초안 작성부터)
- `docs/TD-02-sync.md` — 동기화 엔진 설계 (승인 전 sync/ 본 구현 금지)
- `docs/mockup.html` — UI 레퍼런스. 프론트 작업 시 레이아웃/컴포넌트 구조는 이 목업을 따른다.

PRD와 코드가 충돌하면 PRD가 옳다. PRD 자체에 문제가 있다고 판단되면 **임의로 다르게 구현하지 말고** 작업을 멈추고 문제점과 대안을 보고한다.

## 저장소 구조

```
crates/aqua-manifest/   스키마 3종(manifest/instance/lockfile) + safe_join — PRD §7
src-tauri/src/
  auth/                 MS 인증 체인 (MSA→XBL→XSTS→MC)   — §8.3
  sync/                 동기화 엔진, diff, lockfile        — §7.4, §8.2, TD-02
  launch/               실행 파이프라인                     — §8.15, TD-01
  loaders/              ModLoader trait 구현체             — §8.1
  cache/                내용 주소 공유 캐시                 — §7.1
  browse/               Modrinth/CurseForge 클라이언트     — §8.17
src/                    Svelte 프론트엔드                   — §8.14, mockup.html
cli/                    운영자용 aqua-cli                   — §10.1
fixtures/               테스트용 매니페스트 픽스처 서버
docs/                   PRD, TD, 목업
```

## 명령어

```bash
cargo test --workspace                     # Rust 테스트 (완료 조건)
cargo clippy --workspace -- -D warnings    # 경고 0 필수
pnpm check                                 # Svelte/TS 타입체크
cargo run -p aqua-fixtures                 # 픽스처 매니페스트 서버 (http://127.0.0.1:8750)
pnpm tauri dev                             # 앱 실행 (pnpm install 선행)
```

## 코딩 규칙 (위반 시 리뷰 반려)

**보안 — 예외 없음**
- 모든 외부 입력(매니페스트, 딥링크, 다운로드 파일, API 응답)은 신뢰하지 않는다. (§11)
- 파일 경로 조합은 `aqua_manifest::path::safe_join()`만 사용. 직접 `Path::join` 금지. `..`/절대경로/심볼릭 링크 거부. (§7.3)
- 토큰/refresh_token: OS 자격증명 저장소만 사용, 파일/로그 평문 저장 금지. 로그 출력 전 마스킹 유틸 필수. (§8.3, §8.13)
- 네트워크는 https만. 딥링크 URL은 스킴 검증 후 사용. (§8.7)

**파일 조작 — 예외 없음**
- 다운로드: `<이름>.<랜덤>.part` 임시 파일 → SHA-256 검증 → 원자적 rename. 이 패턴 외 금지. (§8.2.5)
- 공유 캐시는 불변: 덮어쓰기 금지, 새 경로 추가만. (§7.1)
- `origin=user` 파일은 어떤 경로로도 자동 삭제 불가. 삭제는 lockfile의 `origin=manifest` 확인 후에만. (§7.4)
- 동기화는 `.staging/` → 전체 성공 시에만 커밋. 부분 적용 상태를 만드는 코드 금지. (§8.2.4)

**에러 처리**
- 사용자 노출 에러는 PRD §9 에러 코드 체계(`src-tauri/src/error.rs`)를 따른다. 새 시나리오는 §9에 행 추가를 제안한다.

**일반**
- 프론트 문자열 하드코딩 금지 — `src/lib/i18n/` 리소스 사용. (§8.13)
- `--server` 인자는 MC 1.20 미만 전용. 1.20+는 `--quickPlayMultiplayer`. 버전 분기 없는 접속 코드 금지. (§8.6)
- 외부 API(Mojang, Modrinth, CurseForge, Adoptium)는 trait으로 추상화 + 목 구현 동반. (§0 외부 대기 대응)

## 완료 조건 (Definition of Done)

1. 해당 PRD 섹션의 요구사항이 테스트로 증명됨 (특히 §8.2.3 diff 표는 케이스당 테스트 1개 이상 — `sync/mod.rs`의 ignored 테스트 체크리스트를 활성화하는 방식)
2. `cargo test` / `cargo clippy -D warnings` / `pnpm check` 전부 통과
3. 스키마(manifest/instance/lockfile) 변경 시 PRD 스키마 섹션 갱신 diff 동반 제출
4. 커밋 메시지에 PRD 섹션 번호 명시 (예: `feat(sync): staging commit pipeline (§8.2.4)`)

## 사람 리뷰 필수 구역 — 자율 머지 금지

- `crates/aqua-manifest/**` (스키마·safe_join), `src-tauri/src/auth/**`, 파일 삭제 로직, 커밋 저널/롤백 로직, 배포·서명 파이프라인

## 외부 대기 항목 (블로킹 금지 — 목으로 우회)

- MS/Mojang 서드파티 런처 승인 (§0-1): 승인 전까지 `auth::MockAuthProvider`로 개발
- CurseForge API 키 (§0-4): 승인 전까지 `browse::MockCurseForge` + Modrinth 우선 구현
- 코드서명 (§0-2): 예산 제약으로 미서명 배포 허용(우회 안내 동봉). 업데이터 minisign 서명만 필수. SignPath(OSS 무료) 확보 시 Windows 서명 도입

## 작업 순서 (마일스톤 — PRD §13)

현재 단계: **M4** (M0~M3 완료 — M0 스파이크 산출물은 `docs/spike-forge-report.md`, M3 PKCE 인증은 `feat/auth-pkce` 브랜치에서 사람 리뷰 대기).
M4 본체(딥링크, 테마, 옵셔널 모드 UI, §9 에러 코드, §8.12 진단, §10.1 랜딩)는 끝났고 게이트(파일럿 서버 클로즈드 베타)가 남았다.
다음 작업이 명시되지 않았다면 M2 잔여 건(modrinth 소스 해석, CLI `init`/`diff`/`add-modrinth`, `validate --check-urls`)을 우선한다.
