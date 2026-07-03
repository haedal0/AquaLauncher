# TD-01 — 게임 실행 파이프라인 설계 (v1)

> PRD §8.15의 상세 설계 문서.
>
> 상태: **v1 승인됨 (2026-07-03)** — launch/ 구현 가능. 스파이크 실측(docs/spike-forge-report.md) 반영.

## 0. 전체 흐름

```
version JSON 로드 → inheritsFrom 병합 → rules 평가 → 클래스패스 조립
  → natives 추출 → 에셋 확인 → 인자 치환 → LaunchPlan(불변) → spawn → 감시
```

산출물 `LaunchPlan`은 순수 데이터(java 경로, jvm 인자, mainClass, game 인자, cwd, 마스킹 대상 인덱스)로, 조립 단계와 프로세스 실행을 분리해 조립부를 순수 함수로 단위 테스트한다.

## 1. version JSON 병합 — `inheritsFrom` 체인

- 체인 해석: 자식(로더 JSON, 예: `1.20.1-forge-47.4.10.json`)의 `inheritsFrom`을 따라 부모(바닐라 JSON)를 로드. 깊이 상한 5, 순환 감지 시 즉시 오류(신뢰하지 않는 입력).
- 병합 규칙 (자식 = 로더 측 우선):
  - 스칼라(`mainClass`, `assets`, `assetIndex`, `javaVersion` 등): **자식이 있으면 자식 값**.
  - `libraries`: **연결(append)** — 부모 목록 + 자식 목록. 중복 좌표는 §3에서 해소.
  - `arguments.jvm` / `arguments.game`: **부모 먼저, 자식 뒤에 연결** (Mojang 런처와 동일).
  - 레거시 `minecraftArguments`(1.13 미만)는 미지원 — E-MF-05 영역.
- 바닐라 JSON은 Mojang 버전 매니페스트(`piston-meta`)에서 받아 공유 캐시에 저장. 로더 JSON은 로더 설치 산출물(스파이크 보고서 참고).

## 2. `rules` 배열 평가

- 규칙: `rules`가 없으면 허용. 있으면 기본 거부로 시작해 각 rule을 순서대로 평가, **마지막으로 매치된 rule의 action**이 결정.
- `os.name`: `windows` / `osx`(macOS) / `linux`(2차). `os.arch`: `x86`, `arm64`. 대상 매트릭스:

| 플랫폼 | os.name | os.arch 매치 |
|---|---|---|
| win-x64 | windows | (x86 rule은 비매치) |
| mac-aarch64 | osx | arm64 |
| mac-x64 | osx | (arm64 rule은 비매치) |

- `features`(예: `is_demo_user`, `has_quick_plays_support`, `has_custom_resolution`): 런처가 명시적으로 켠 feature만 true, 미지의 feature는 false. quickPlay 접속(§7)에 `has_quick_plays_support` 사용 여부는 버전 분기와 함께 결정.
- 평가는 `RuleContext { os, arch, features }`를 받는 순수 함수로 구현, 3플랫폼 × 대표 rule 조합 테이블 테스트.

## 3. 클래스패스 조립

- 병합된 `libraries` → 각 항목의 maven 좌표(`group:artifact:version[:classifier]`)를 공유 캐시 경로로 해석.
- **중복 해소**: `group:artifact(:classifier)` 단위로 그룹핑, **로더 측(자식 JSON) 항목 우선**, 동순위면 뒤에 나온 것 우선. (PRD §8.15-2)
- 클래스패스 마지막에 client.jar(공유 캐시 경로) 추가. Forge 계열은 JVM 인자 `-p`(module-path)에 별도 라이브러리 목록이 오므로(스파이크 확인) 클래스패스와 module-path를 모두 인자 치환 결과로부터 그대로 사용 — 런처가 임의 재배열하지 않는다.
- 구분자: Windows `;`, macOS `:` → `${classpath_separator}` 치환에도 동일 적용.

## 4. natives 추출

- 대상: `library.natives`가 있는 항목(레거시) + `rules`로 플랫폼 한정된 native classifier 라이브러리(1.19+ 방식, 별도 추출 없이 클래스패스 포함되는 경우 있음).
- 추출 위치: `<instance>/natives/<version_id>/` — **인스턴스별** 임시 디렉토리 (공유 캐시 불변 원칙과 분리).
- 수명: 실행 직전 비우고 재추출 → 게임 종료 후 유지(다음 실행 시 재생성이므로 삭제 불필요, 디스크 사용량 표시에 포함). 추출 시 zip 엔트리 경로는 `safe_join` 검증(Zip Slip 방지, PRD §11).

## 5. 에셋 인덱스

- 병합 JSON의 `assetIndex` → `assets/indexes/<id>.json`을 공유 캐시에 저장, 각 오브젝트는 `assets/objects/<hash[0..2]>/<hash>`.
- 실행 전 인덱스 존재만 확인(전체 재검증은 §8.2.7 "파일 검증"의 영역). `${assets_root}`/`${assets_index_name}` 치환에 사용.

## 6. 인자 치환 테이블

| 플레이스홀더 | 값 | 비고 |
|---|---|---|
| `${auth_player_name}` | 프로필 gamertag | |
| `${auth_uuid}` | MC UUID | |
| `${auth_access_token}` | MC 액세스 토큰 | **로그 마스킹 필수** |
| `${auth_xuid}` | XUID | **마스킹** |
| `${clientid}` | 클라이언트 ID | **마스킹** |
| `${user_type}` | `msa` | |
| `${version_name}` | version id | |
| `${version_type}` | `release` 등 | |
| `${game_directory}` | **인스턴스 디렉토리** (격리, PRD §8.15-7) | |
| `${assets_root}` / `${assets_index_name}` | 공유 캐시 assets / 인덱스 id | |
| `${natives_directory}` | §4 추출 디렉토리 | |
| `${launcher_name}` / `${launcher_version}` | `AquaLauncher` / 앱 버전 | |
| `${classpath}` | §3 결과 | |
| `${library_directory}` | 공유 캐시 libraries 루트 | 로더 확장 (스파이크 확인) |
| `${classpath_separator}` | `;` 또는 `:` | 로더 확장 |
| `${resolution_width/height}` | 창 크기 (feature 켜진 경우만) | |

- 미지의 플레이스홀더: 오류가 아닌 **빈 문자열 치환 + 경고 로그** (로더 JSON 다양성 대응).
- 마스킹: LaunchPlan이 마스킹 대상 인자 인덱스를 함께 보유 → 로그/디버그 출력 경로가 하나의 마스킹 유틸을 강제 통과.

## 7. 서버 자동 접속 인자 (PRD §8.6 — 버전 분기 필수)

```rust
fn join_args(mc: &SemverLike, host: &str, port: u16) -> Vec<String> {
    if mc >= 1.20  { vec!["--quickPlayMultiplayer", format!("{host}:{port}")] }
    else /*1.13~1.19*/ { vec!["--server", host, "--port", port.to_string()] }
}
```

- 버전 비교는 릴리스 버전 파싱 기반(스냅샷은 대응 릴리스로 정규화 불가 시 접속 인자 생략 + 경고).
- servers.dat 등록은 이 함수와 별개로, 해당 인스턴스 게임 비실행 중 + 실행 직전에만 수행(§8.6).

## 8. 프로세스 수명

1. spawn: java 경로(§8.4 결정) + LaunchPlan 인자, cwd = 인스턴스 디렉토리. 환경변수는 최소 전달.
2. stdout/stderr 라인 스트림 캡처 → 링 버퍼(상한 있음) + 인앱 로그 뷰어 이벤트(100ms 스로틀, PRD §6).
3. 종료 코드: 0 정상. 비정상 → crash-reports 최신 파일 탐지 → E-GM-01 뷰어. 5분 내 3회 비정상 → E-GM-02 크래시 루프 차단 (PRD §8.12).
4. 실행 중 표시: 인스턴스 상태 `running` → 동기화/GC/servers.dat 쓰기 차단 (TD-02 §5와 동일 규칙).
5. 런처 종료 시 게임 프로세스는 **분리(detach) 유지** — 런처가 게임의 생존을 소유하지 않는다.

## 9. 테스트 전략

- 병합/rules/클래스패스/치환은 전부 순수 함수 → 실제 Forge/바닐라/NeoForge version JSON 픽스처(스파이크 산출물 축소판)로 골든 테스트.
- quickPlay 분기: 1.13/1.19.4/1.20.4 경계값 테스트 (AGENT.md 일반 규칙).
- 마스킹: 토큰 문자열이 로그 문자열에 등장하지 않음을 검증하는 네거티브 테스트.
