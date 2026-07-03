# TD-02 — 동기화 엔진 상태 머신 설계 (v1)

> PRD §7.4, §8.2의 상세 설계 문서.
>
> 상태: **v1 승인됨 (2026-07-03)** — sync/ 본 구현 가능.

## 1. 상태 전이도

```
                    ┌──────────────────────────────────────────────┐
                    │  (모든 실패 전이는 Failed(reason)로 수렴,      │
                    │   Staging까지의 실패는 기존 상태 무손상)        │
                    └──────────────────────────────────────────────┘

 Idle ──fetch 요청──▶ Fetching ──매니페스트 수신+검증──▶ Planning
   ▲                     │ E-MF-01/02/03/05                 │
   │                     ▼                                  ▼
   │                  Failed ◀──다운로드 실패(E-MF-04)── Staging
   │                     ▲                                  │ 전체 해시 검증 성공
   │                     │ 저널 복구 불가                    ▼
   └──── Done ◀──lockfile 갱신─── Committing ◀──저널 기록──┘
```

| 상태 | 진입 조건 | 하는 일 | 실패 시 |
|---|---|---|---|
| `Idle` | 초기/완료 후 | 대기 | — |
| `Fetching` | 시작 시 배지 체크 / 플레이 버튼 / 수동 동기화 | manifest.json GET(https 강제), sha256 계산, `format_version`·`min_launcher_version`·MC 1.13+ 검증 | E-MF-01(오프라인 → §6 판정), E-MF-02/03/05 |
| `Planning` | fetch 성공 & 해시 ≠ `instance.manifest_hash` | §3 diff 계산 → `SyncPlan` 산출. 디스크 공간 사전검증(§8.16). 200MB 이상이면 확인 다이얼로그 대기 | E-DL-01 |
| `Staging` | plan 승인(자동/사용자) | 모든 Download/Update 대상을 `.staging/`에 수신+검증. 일시정지/취소 가능. 취소 → `.staging/` 폐기 → Idle | E-MF-04 (기존 상태 무손상) |
| `Committing` | 스테이징 전체 성공 | 저널 기록 → 삭제/rename 일괄 적용 → lockfile·`manifest_hash` 갱신 → 저널 삭제 | 크래시 → §5 복구 |
| `Done/Failed` | — | UI 통지, Idle 복귀 | — |

- 트리거별 차이(PRD §8.2.1): 런처 시작 시에는 `Fetching`까지만 수행하고 해시 불일치면 배지만 표시(Planning 진입 안 함). `manifest_pinned=true`도 동일.
- 상태는 인스턴스 단위로 독립. 동일 인스턴스에 동시 동기화 금지(인스턴스별 mutex).

## 2. lockfile 스키마

`crates/aqua-manifest/src/lockfile.rs`와 1:1 — 현행 스키마(`applied_manifest_hash`, `applied_at`, `managed_files[{path, mod_id, sha256, origin, enabled}]`)를 그대로 확정한다. 추가 필드 없음.

- 직렬화 파일명: 인스턴스 루트의 `manifest.lock.json`.
- 쓰기는 Committing 단계의 마지막에 원자적으로(임시 파일 → rename) 1회만 수행.
- `enabled=false`인 파일의 실제 경로는 `path + ".disabled"` (PRD §8.2.8). lockfile의 `path`는 항상 원래 경로로 기록한다.

## 3. diff 알고리즘 (PRD §8.2.3 표 7케이스)

### 3.1 매칭 키 우선순위

매니페스트 항목 ↔ lockfile 항목 매칭은 순서대로:

1. `mod_id` — 매니페스트 유래(`origin=manifest`) 파일. 동일 `mod_id`의 파일명 변경(버전업)은 "삭제+신규"가 아닌 **Update**로 처리.
2. 파일명(`path`의 파일명 성분)
3. `sha256`

### 3.2 의사코드

```
fn plan_diff(manifest, lockfile, optional_selection) -> Vec<DiffAction>:
    # 0. 스캔: mods/ 등 실물에 있으나 lockfile에 없는 파일 → origin=user로 lockfile 등록
    # 1. 매니페스트 항목 순회
    for entry in manifest.mods + manifest.files + [resourcepack]:
        if entry.optional and not optional_selection[entry.id]:
            emit SkipOptionalUnselected                    # 케이스 5
            continue
        local = match(entry, lockfile)                     # §3.1 우선순위
        if local is None:
            emit Download                                  # 케이스 1
        elif entry는 files[]이고 sync_policy == once and local 존재:
            emit Keep                                      # once는 최초 1회만
        elif local.sha256 == entry.sha256:
            emit Keep                                      # 케이스 2
        elif not local.enabled:
            emit UpdateKeepDisabled                        # 케이스 4 (파일 최신화, 비활성 유지)
        else:
            emit Update                                    # 케이스 3
    # 2. lockfile 항목 중 매니페스트에 없는 것
    for local in lockfile.managed_files not matched above:
        if local.origin == user:
            emit PreserveUserFile                          # 케이스 6 — 어떤 경로로도 삭제 금지
        else:
            emit Remove                                    # 케이스 7
```

- **엣지케이스(PRD §7.4)**: 사용자가 넣은 파일과 동일 해시의 모드가 매니페스트에 추가되면 매칭은 되지만 origin은 `user` 유지 → 이후 매니페스트에서 사라져도 PreserveUserFile.
- `files[]`의 `path`는 `safe_join`으로 검증 실패 시 매니페스트 전체 거부(E-MF-02 상세 사유).
- required 모드가 disabled인 개수를 plan 메타데이터로 산출(실행 직전 리마인더용, §8.2.8).

## 4. 스테이징 레이아웃과 커밋 저널

### 4.1 `.staging/` 레이아웃

```
<instance>/.staging/<sync-id>/          # sync-id = 적용할 manifest hash 앞 12자
  files/<원래 상대경로>                  # 최종 위치와 동일한 상대 트리로 수신
  journal.json                          # 커밋 직전 기록
```

- 다운로드는 공유 캐시 규칙과 동일: `<이름>.<랜덤>.part` → sha256 검증 → rename (PRD §8.2.5).
- Range 이어받기 지원, 동시 다운로드 기본 4, 파일당 재시도 3회.
- 시작 시 잔존 `.staging/*` 발견하면: journal.json 없으면 통째로 폐기(커밋 미진입이므로 안전).

### 4.2 커밋 저널 `journal.json` 포맷

```json
{
  "version": 1,
  "sync_id": "a1b2c3d4e5f6",
  "target_manifest_hash": "sha256:...",
  "created_at": "2026-07-03T09:00:00Z",
  "ops": [
    { "op": "remove", "path": "mods/old-1.0.jar" },
    { "op": "move",   "from": ".staging/a1b2c3d4e5f6/files/mods/new-1.1.jar", "to": "mods/new-1.1.jar" }
  ],
  "final_lockfile": { "...": "커밋 완료 시점에 쓸 lockfile 전문" }
}
```

- ops 순서: **remove 전부 → move 전부**. 각 op는 멱등(이미 없으면 remove 성공 취급, 목적지에 올바른 해시가 있으면 move 성공 취급).
- remove 대상은 기록 직전에 lockfile `origin=manifest` 재확인 (AGENT.md 파일 조작 규칙).

### 4.3 크래시 복구 절차

시작 시 `journal.json` 발견 →
1. `final_lockfile`과 ops가 파싱되면 **커밋 재개**: 각 op를 멱등 재실행 → lockfile 쓰기 → 저널·스테이징 삭제.
2. 파싱 불가/스테이징 파일 해시 불일치면 **롤백**: 스테이징 폐기, lockfile은 기존 것 유지, `install_state=broken` 대신 "파일 검증"(§6) 1회 자동 수행 후 결과에 따라 ready/broken.

## 5. 동시성 규칙

- **인스턴스 단위 직렬화**: 인스턴스당 동기화 1개(프로세스 내 mutex + 인스턴스 디렉토리 lock 파일).
- **공유 캐시**: 내용 주소 + 불변이므로 잠금 불필요 — 동시 다운로드는 `.part` 랜덤 접미사로 충돌 없고, rename 경쟁은 결과가 멱등(같은 해시 = 같은 내용).
- **실행 중 인스턴스**: 게임 실행 중에는 해당 인스턴스 동기화 금지(배지만 갱신). servers.dat 쓰기도 동일 규칙(§8.6).

## 6. 오프라인 판정(§8.2.6)과 검증/복구(§8.2.7)

- Fetching 실패가 네트워크 계층 오류(연결/DNS/타임아웃)면 **오프라인 모드**: lockfile이 정합(마지막 Done)이면 플레이 허용 + "동기화 확인 실패 — 마지막 동기화 {last_synced_at}" 배지. HTTP 4xx/5xx는 E-MF-01로 동일 UX.
- **파일 검증**: managed_files 전체 해시 재검사 → 불일치/누락만 `SyncPlan{Update|Download}`으로 만들어 §1 파이프라인 재사용(Planning부터 진입). 사용자 파일(origin=user)은 검사·복구 대상 아님.

## 7. 구현 체크리스트 (승인 후)

- `sync/mod.rs`의 `plan_diff` 구현 + ignored 테스트 8개 활성화 (케이스당 1개 이상, AGENT.md DoD 1)
- diff는 property 기반 테스트 추가 (PRD §13 테스트 전략)
- 스테이징-커밋은 강제 실패 주입 테스트 (다운로드 중단, 커밋 중 kill)
