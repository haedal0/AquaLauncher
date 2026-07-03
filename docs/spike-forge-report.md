# M0 스파이크 보고서 — Forge/NeoForge headless 설치 검증

- 실행일: 2026-07-03, macOS (Apple Silicon), OpenJDK 21 (Homebrew), 유선급 네트워크
- 검증 대상: **Forge 1.20.1-47.4.10** (1.20.1-recommended), **NeoForge 20.4.251** (MC 1.20.4)
- 방법: `java -jar <installer>.jar --installClient <dir>` 를 빈 디렉토리에 직접 실행하고 산출물·로그·종료 코드 분석. PRD §0-3, §8.1 함정 목록 항목별 확인.

## 결론

**성공 — 두 로더 모두 headless 설치가 재현 가능하게 동작한다. §8.1의 `ModLoader` trait 인터페이스를 그대로 확정해도 된다.** (게이트 통과: "스파이크 성공 = 로더 인터페이스 확정", PRD §13)

## 항목별 검증 결과

| # | PRD §8.1 함정 | 결과 |
|---|---|---|
| 1 | `launcher_profiles.json` 없으면 설치 거부 | **확인됨.** `There is no minecraft launcher profile in ".", you need to run the launcher first!` 출력 후 실패. 더미 `{"profiles":{}}` 생성으로 통과 (Forge/Neo 동일) |
| 2 | 바닐라 client.jar 기대 위치 | `versions/<mc_version>/<mc_version>.jar`. **없으면 인스톨러가 Mojang(piston-data)에서 직접 다운로드**(체크섬 검증 포함), **미리 배치하면 재다운로드 없이 재사용**함을 로그로 확인 → 공유 캐시에서 하드링크/복사로 스테이징하는 §8.1 전략 유효 |
| 3 | 설치 프로세서 소요 시간 | Forge 18.0초 / NeoForge 17.7초 / client.jar 사전 배치 시 Forge 15.3초 (M-시리즈 맥 + 고속망 기준. 저사양·저속망에서는 수 분 가능 → UI는 불확정 진행바 유지) |
| 4 | 산출물 version JSON | 두 로더 모두 `inheritsFrom` 체인 확인 (아래 상세) |

## 산출물 상세 (실행 파이프라인 설계 입력 — TD-01 반영)

**디렉토리 레이아웃** (두 로더 공통, maven 스타일):

```
<work-dir>/
  launcher_profiles.json            # 사전 생성 더미 (설치 후 인스톨러가 프로필 주입)
  <installer>.jar.log               # 인스톨러가 CWD에 로그 파일 생성 (부산물 정리 필요)
  libraries/...                     # maven 좌표 구조. Forge 66개 / Neo 86개 jar
  versions/1.20.1/1.20.1.jar        # 바닐라 client.jar (Forge는 .json 미생성)
  versions/1.20.4/{1.20.4.jar,1.20.4.json}   # Neo는 바닐라 version JSON도 생성
  versions/1.20.1-forge-47.4.10/1.20.1-forge-47.4.10.json   # Forge 명명 규칙
  versions/neoforge-20.4.251/neoforge-20.4.251.json          # Neo 명명 규칙 (상이!)
```

**version JSON 요점**:

- `inheritsFrom`: `1.20.1` / `1.20.4` — 바닐라 JSON과의 병합은 런처(우리) 책임.
- `mainClass`: 둘 다 `cpw.mods.bootstraplauncher.BootstrapLauncher`.
- JVM 인자에 **바닐라에 없는 플레이스홀더** 사용: `${library_directory}`, `${classpath_separator}`, `${version_name}` — TD-01 인자 치환 테이블에 반드시 포함해야 함.
- game 인자는 `--launchTarget forgeclient` 등 고정 문자열 위주. 라이브러리는 Forge 29 / Neo 49개 엔트리가 바닐라 목록에 **추가**된다 (중복 시 로더 우선, PRD §8.15-2).
- 프로세서가 변환한 마인크래프트 아티팩트는 `libraries/net/minecraft/client/<mcp-version>/` 아래에 생성됨 (client-extra, slim 등) — 이 산출물은 인스턴스가 아닌 **로더 설치 캐시**로 취급 가능.

**프로세스 동작**:

- 종료 코드 신뢰 가능: 성공 0, 실패 1. 단, 실패 사유는 stdout 파싱 필요 (`There was an error during installation`).
- stdout에 단계별 로그가 나옴 (`Considering library ...`, `Patching ...`) → 인앱 로그 캡처에 그대로 사용 가능. 진행률 퍼센트는 제공되지 않음 → 불확정 바 확정.
- NeoForge 인스톨러는 `--install-client`(kebab-case)도 수용. Forge와 옵션 표기가 다를 수 있으므로 로더별 인자 구성은 구현체 내부에 캡슐화한다 (trait 시그니처에 영향 없음).

## 로더 인터페이스 확정안

§8.1의 trait을 그대로 확정하되, 스파이크 결과를 반영해 구현 규약을 추가한다:

```rust
trait ModLoader {
    fn id(&self) -> &str;
    fn list_versions(&self, mc_version: &str) -> Result<Vec<LoaderVersion>>;
    fn install(&self, mc_version: &str, loader_version: &str, ctx: &InstallContext) -> Result<LaunchProfile>;
}
```

- `InstallContext`에 **인스톨러 작업 디렉토리** 개념 유지: `{launcher_profiles.json 더미, versions/<mc>/<mc>.jar 사전 스테이징(공유 캐시에서 하드링크/복사)}`를 만든 뒤 인스톨러 실행 → 산출 `versions/*/*.json`과 `libraries/`를 공유 캐시로 수확(harvest)하는 흐름. (캐시 불변 규칙 준수: 이미 있는 경로는 건너뜀)
- `LaunchProfile.version_json_path`는 로더별 명명 규칙 차이(`1.20.1-forge-47.4.10` vs `neoforge-20.4.251`)를 흡수한다 — 호출자는 JSON 경로만 받는다.
- 실패 판정: 종료 코드 ≠ 0 → `E-LD-01` + 인스톨러 stdout/`.jar.log` 첨부.
- 크래시 대비: 실행 전 `install_state=installing` 기록 (PRD §8.1).

## 남은 리스크 / 후속

- 저속 환경 실측치 없음 → M3에서 Windows 저사양 VM으로 스모크 시 재측정.
- 인스톨러가 CWD에 `<installer>.jar.log`를 남김 → 작업 디렉토리 정리 루틴에 포함.
- 구버전 Forge(1.13~1.16대)는 인스톨러 세부가 다를 수 있음 → M3 매트릭스 테스트에서 1.13대 대표 버전으로 재검증 (PRD §13).
- M0의 나머지 항목(MS 서드파티 승인 신청, 코드서명 조달, CF API 키 신청)은 **외부 절차**로 이 스파이크와 무관하게 진행 필요.
