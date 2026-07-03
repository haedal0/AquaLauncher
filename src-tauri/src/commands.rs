//! Tauri 커맨드 계층 — 프론트(src/lib/api.ts)와 1:1.
//! 블로킹 작업(네트워크/파일)은 전부 spawn_blocking으로 감싼다 (net 모듈은 블로킹 클라이언트).
use crate::auth::{AuthProvider, MockAuthProvider};
use crate::error::AppError;
use crate::launch::install::{build_launch_plan, prepare_version, LaunchConfig};
use crate::launch::java::AdoptiumProvider;
use crate::launch::process::{spawn_and_wait, CrashTracker};
use crate::launch::rules::RuleContext;
use crate::launch::JavaRuntimeProvider;
use crate::loaders::{
    fabric::FabricLikeLoader,
    forge::{ForgeLikeLoader, JavaInstallerRunner},
    InstallContext, ModLoader,
};
use crate::net::HttpFetcher;
use crate::store::{self, InstanceVm, Paths};
use crate::sync::engine::{fetch_manifest, sync_instance, OnlineResolver, SyncError};
use aqua_manifest::instance::InstallState;
use aqua_manifest::manifest::LoaderKind;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

pub struct AppState {
    pub paths: Paths,
    /// 크래시 루프 가드 (§8.12, E-GM-02) — 인스턴스별, 런처 세션 동안 유지.
    pub crash_trackers: Arc<Mutex<HashMap<String, CrashTracker>>>,
}

fn vm_list(paths: &Paths) -> Vec<InstanceVm> {
    store::list_instances(paths)
        .iter()
        .enumerate()
        .map(|(i, cfg)| store::build_vm(paths, cfg, i as i64))
        .collect()
}

fn vm_one(paths: &Paths, id: &str) -> Result<InstanceVm, AppError> {
    let cfg = store::get_instance(paths, id)
        .ok_or_else(|| AppError::internal(format!("unknown instance {id}")))?;
    Ok(store::build_vm(paths, &cfg, 0))
}

#[tauri::command]
pub async fn list_instances(state: State<'_, AppState>) -> Result<Vec<InstanceVm>, AppError> {
    let paths = state.paths.clone();
    tauri::async_runtime::spawn_blocking(move || vm_list(&paths))
        .await
        .map_err(AppError::internal)
}

#[tauri::command]
pub async fn create_manual_instance(
    state: State<'_, AppState>,
    name: String,
    mc_version: String,
    loader_kind: String,
    color: String,
) -> Result<InstanceVm, AppError> {
    let paths = state.paths.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let kind = match loader_kind.to_lowercase().as_str() {
            "vanilla" => LoaderKind::Vanilla,
            "fabric" => LoaderKind::Fabric,
            "quilt" => LoaderKind::Quilt,
            "forge" => LoaderKind::Forge,
            "neoforge" => LoaderKind::Neoforge,
            other => return Err(AppError::internal(format!("unknown loader: {other}"))),
        };
        let loader = aqua_manifest::manifest::LoaderRef { kind, version: String::new() };
        let cfg = store::create_manual(&paths, &name, &mc_version, loader, &color)?;
        Ok(store::build_vm(&paths, &cfg, 0))
    })
    .await
    .map_err(AppError::internal)?
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestPreview {
    pub name: String,
    pub desc: Option<String>,
    pub domain: Option<String>,
    pub mod_count: usize,
    pub optional_count: usize,
    pub total_bytes: u64,
    pub loader_label: String,
    pub mc_version: String,
}

/// 딥링크/URL 추가 확인 모달용 미리보기 — PRD 8.7 (출처 도메인 표기는 스푸핑 방어).
#[tauri::command]
pub async fn preview_manifest(url: String) -> Result<ManifestPreview, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let fetch = HttpFetcher::new()?;
        let fetched = fetch_manifest(&fetch, &url)?;
        let m = &fetched.manifest;
        Ok(ManifestPreview {
            name: m.server_display_name.clone(),
            desc: m.server_description.clone(),
            domain: url.split("://").nth(1).and_then(|r| r.split('/').next()).map(String::from),
            mod_count: m.mods.len(),
            optional_count: m.mods.iter().filter(|e| !e.required).count(),
            total_bytes: m.mods.iter().map(|e| e.size_bytes).sum::<u64>()
                + m.files.iter().map(|f| f.size_bytes).sum::<u64>(),
            loader_label: format!("{:?}", m.loader.kind),
            mc_version: m.minecraft_version.clone(),
        })
    })
    .await
    .map_err(AppError::internal)?
}

#[tauri::command]
pub async fn create_instance_from_manifest(
    state: State<'_, AppState>,
    url: String,
) -> Result<InstanceVm, AppError> {
    let paths = state.paths.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let fetch = HttpFetcher::new()?;
        let fetched = fetch_manifest(&fetch, &url)?;
        let cfg = store::create_from_manifest(&paths, &fetched.manifest, &url)?;
        Ok(store::build_vm(&paths, &cfg, 0))
    })
    .await
    .map_err(AppError::internal)?
}

#[tauri::command]
pub async fn toggle_mod(
    state: State<'_, AppState>,
    id: String,
    path: String,
    enabled: bool,
) -> Result<InstanceVm, AppError> {
    let paths = state.paths.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store::toggle_mod(&paths, &id, &path, enabled)?;
        vm_one(&paths, &id)
    })
    .await
    .map_err(AppError::internal)?
}

#[tauri::command]
pub async fn reset_instance(state: State<'_, AppState>, id: String) -> Result<InstanceVm, AppError> {
    let paths = state.paths.clone();
    tauri::async_runtime::spawn_blocking(move || {
        store::reset_to_manifest(&paths, &id)?;
        vm_one(&paths, &id)
    })
    .await
    .map_err(AppError::internal)?
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent<'a> {
    id: &'a str,
    stage: &'a str,
    item: &'a str,
    done: usize,
}

/// 진행 이벤트 emit — 100ms 스로틀 (PRD 8.10 / §6 IPC 병목 방지).
struct Throttled {
    app: AppHandle,
    last: Instant,
    done: usize,
}

impl Throttled {
    fn new(app: AppHandle) -> Self {
        Throttled { app, last: Instant::now() - Duration::from_secs(1), done: 0 }
    }
    fn emit(&mut self, id: &str, stage: &str, item: &str) {
        self.done += 1;
        if self.last.elapsed() >= Duration::from_millis(100) {
            self.last = Instant::now();
            let _ = self
                .app
                .emit("progress", ProgressEvent { id, stage, item, done: self.done });
        }
    }
}

/// 동기화 수행 (내부 공용). 매니페스트 인스턴스가 아니면 no-op.
/// fetch 실패는 §8.2.6 오프라인 정책 — 에러 대신 offline 표시를 반환.
fn run_sync(paths: &Paths, id: &str) -> Result<bool, AppError> {
    let mut cfg = store::get_instance(paths, id)
        .ok_or_else(|| AppError::internal(format!("unknown instance {id}")))?;
    let Some(url) = cfg.manifest_source.clone() else {
        return Ok(false); // 수동 인스턴스 — 동기화 없음 (PRD 8.8)
    };
    let fetch = HttpFetcher::new()?;
    let fetched = match fetch_manifest(&fetch, &url) {
        Ok(f) => f,
        Err(SyncError::Fetch(e)) => {
            tracing::warn!(%e, "manifest fetch failed — offline policy (PRD 8.2.6)");
            return Ok(false);
        }
        Err(e) => return Err(e.into()),
    };
    if cfg.manifest_pinned || cfg.manifest_hash.as_deref() == Some(fetched.hash_hex.as_str()) {
        return Ok(false);
    }
    let lock = store::load_lockfile(paths, id);
    let root = paths.instance_dir(id);
    let outcome = sync_instance(
        &fetch,
        &OnlineResolver { fetch: &fetch }, // curseforge는 API 키 승인(PRD 0-4) 후 지원
        &root,
        &fetched,
        &lock,
        &cfg.optional_mods_selection,
        &now_label(),
    )
    ?;
    store::save_cached_manifest(paths, id, &fetched.manifest)?;
    cfg.manifest_hash = Some(fetched.hash_hex.clone());
    cfg.manifest_display_version = Some(fetched.manifest.display_version.clone());
    cfg.last_synced_at = Some(now_label());
    cfg.install_state = aqua_manifest::instance::InstallState::Ready;
    store::save_instance(paths, &cfg)?;
    Ok(outcome.changed)
}

/// 로더 버전 선택: 인스턴스에 지정된 버전 > 안정 최신 > 목록 첫 항목 (§8.1).
fn pick_loader_version(
    loader: &dyn ModLoader,
    mc_version: &str,
    configured: &str,
) -> Result<String, AppError> {
    if !configured.is_empty() {
        return Ok(configured.to_string());
    }
    let versions = loader.list_versions(mc_version)?;
    Ok(versions
        .iter()
        .find(|v| v.stable)
        .or(versions.first())
        .ok_or_else(|| AppError::LoaderInstall(format!("no loader versions for {mc_version}")))?
        .id
        .clone())
}

fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_label() -> String {
    format!("unix:{}", unix_secs())
}

#[tauri::command]
pub async fn sync_now(state: State<'_, AppState>, id: String) -> Result<InstanceVm, AppError> {
    let paths = state.paths.clone();
    tauri::async_runtime::spawn_blocking(move || {
        run_sync(&paths, &id)?;
        vm_one(&paths, &id)
    })
    .await
    .map_err(AppError::internal)?
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSummaryVm {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub total_bytes: u64,
    /// §8.2.2: 200MB 이상 — 다운로드 전 확인 다이얼로그 필요
    pub requires_confirmation: bool,
    pub display_version: String,
    pub changelog: Option<String>,
}

/// 업데이트 확인 — §8.2.1(시작 시 배지)·§8.2.2(대용량 확인) 공용.
/// **다운로드는 하지 않는다.** fetch 실패는 §8.2.6 오프라인 정책 — None.
#[tauri::command]
pub async fn check_update(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<UpdateSummaryVm>, AppError> {
    let paths = state.paths.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = store::get_instance(&paths, &id)
            .ok_or_else(|| AppError::internal(format!("unknown instance {id}")))?;
        let Some(url) = cfg.manifest_source.clone() else {
            return Ok(None); // 수동 인스턴스 — 동기화 없음 (PRD 8.8)
        };
        let fetch = HttpFetcher::new()?;
        let fetched = match fetch_manifest(&fetch, &url) {
            Ok(f) => f,
            Err(SyncError::Fetch(e)) => {
                tracing::warn!(%e, "update check fetch failed — offline policy (PRD 8.2.6)");
                return Ok(None);
            }
            Err(e) => return Err(e.into()),
        };
        if cfg.manifest_hash.as_deref() == Some(fetched.hash_hex.as_str()) {
            return Ok(None);
        }
        // manifest_pinned여도 배지는 표시하고 적용만 막는다 (§8.2.1) — 적용 차단은 run_sync 담당
        let lock = store::load_lockfile(&paths, &id);
        let actions =
            crate::sync::plan_diff(&fetched.manifest, &lock, &cfg.optional_mods_selection);
        let s = crate::sync::summarize_diff(&actions, &fetched.manifest);
        Ok(Some(UpdateSummaryVm {
            requires_confirmation: s.requires_confirmation(),
            added: s.added,
            updated: s.updated,
            removed: s.removed,
            total_bytes: s.total_bytes,
            display_version: fetched.manifest.display_version.clone(),
            changelog: fetched.manifest.changelog.clone(),
        }))
    })
    .await
    .map_err(AppError::internal)?
}

fn platform() -> (&'static str, &'static str, &'static str, &'static str) {
    // (rules os_name, rules arch, adoptium os, adoptium arch)
    let os = if cfg!(target_os = "windows") { ("windows", "windows") } else { ("osx", "mac") };
    let arch = if cfg!(target_arch = "aarch64") { ("arm64", "aarch64") } else { ("x86_64", "x64") };
    (os.0, arch.0, os.1, arch.1)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GameExited {
    id: String,
    code: Option<i32>,
    /// 5분 내 3회 비정상 종료 (§8.12) — 프론트가 E-GM-02 진단 안내로 표면화.
    crash_loop: bool,
}

/// 플레이 — PRD 8.2.1(플레이 시 동기화) + 8.15 파이프라인 전체.
#[tauri::command]
pub async fn play(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), AppError> {
    let paths = state.paths.clone();
    let crash_trackers = state.crash_trackers.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (rules_os, rules_arch, ad_os, ad_arch) = platform();
        let ctx = RuleContext::new(rules_os, rules_arch);
        let fetch = HttpFetcher::new()?;
        let mut progress = Throttled::new(app.clone());

        // 1) 동기화 (§8.2.1 플레이 버튼 트리거)
        progress.emit(&id, "sync", "");
        run_sync(&paths, &id)?;

        let mut cfg = store::get_instance(&paths, &id)
            .ok_or_else(|| AppError::internal(format!("unknown instance {id}")))?;
        let cache = paths.cache_dir();

        // 2) 로더 준비 (§8.1)
        let loader_profile = match cfg.loader.kind {
            LoaderKind::Vanilla => None,
            LoaderKind::Fabric | LoaderKind::Quilt => {
                let loader = if cfg.loader.kind == LoaderKind::Fabric {
                    FabricLikeLoader::fabric(&fetch)
                } else {
                    FabricLikeLoader::quilt(&fetch)
                };
                let version =
                    pick_loader_version(&loader, &cfg.minecraft_version, &cfg.loader.version)?;
                progress.emit(&id, "loader", &version);
                let ictx = InstallContext { shared_cache_root: &cache, work_dir: &cache };
                let profile = loader
                    .install(&cfg.minecraft_version, &version, &ictx)
                    ?;
                Some(profile.version_json_path)
            }
            LoaderKind::Forge | LoaderKind::Neoforge => {
                // 인스톨러 실행에 JVM + 바닐라 client.jar 필요 → 바닐라 체인 선준비
                // (client.jar가 캐시에 있으면 인스톨러의 재다운로드를 막는다 — 스파이크 함정 2)
                let id_p = id.clone();
                let vanilla = prepare_version(
                    &fetch,
                    &cache,
                    &cfg.minecraft_version,
                    None,
                    &ctx,
                    &mut |stage, item| progress_emit_shim(&mut progress, &id_p, stage, item),
                )
                ?;
                let java = match &cfg.java_path_override {
                    Some(p) => std::path::PathBuf::from(p),
                    None => AdoptiumProvider {
                        fetch: &fetch,
                        cache_root: cache.clone(),
                        os: ad_os.into(),
                        arch: ad_arch.into(),
                    }
                    .resolve(vanilla.merged.java_major)
                    ?,
                };
                let runner = JavaInstallerRunner { java };
                let loader = if cfg.loader.kind == LoaderKind::Forge {
                    ForgeLikeLoader::forge(&fetch, &runner)
                } else {
                    ForgeLikeLoader::neoforge(&fetch, &runner)
                };
                let version =
                    pick_loader_version(&loader, &cfg.minecraft_version, &cfg.loader.version)?;
                progress.emit(&id, "loader", &version);

                // 설치 중 크래시 대비 (§8.1): installing 기록 → 재시작 감지용
                let prev_state = cfg.install_state;
                cfg.install_state = InstallState::Installing;
                store::save_instance(&paths, &cfg)?;

                // 인스톨러 전용 작업 디렉토리 — 부산물(*.jar.log 등)째로 정리
                let work = cache
                    .join(".installer-work")
                    .join(format!("{}-{}", ModLoader::id(&loader), version));
                let ictx = InstallContext { shared_cache_root: &cache, work_dir: &work };
                let result = loader.install(&cfg.minecraft_version, &version, &ictx);
                let _ = std::fs::remove_dir_all(&work);
                let profile = match result {
                    Ok(p) => p,
                    Err(e) => {
                        cfg.install_state = prev_state;
                        let _ = store::save_instance(&paths, &cfg);
                        return Err(e); // E-LD-01 표면화
                    }
                };
                cfg.install_state = InstallState::Ready;
                store::save_instance(&paths, &cfg)?;
                Some(profile.version_json_path)
            }
        };

        // 3) 버전/아티팩트 준비 (TD-01 §0)
        let id_for_progress = id.clone();
        let prepared = prepare_version(
            &fetch,
            &cache,
            &cfg.minecraft_version,
            loader_profile.as_deref(),
            &ctx,
            &mut |stage, item| progress_emit_shim(&mut progress, &id_for_progress, stage, item),
        )
        ?;

        // 4) Java (§8.4)
        progress.emit(&id, "java", "");
        let java = match &cfg.java_path_override {
            Some(p) => std::path::PathBuf::from(p),
            None => AdoptiumProvider {
                fetch: &fetch,
                cache_root: cache.clone(),
                os: ad_os.into(),
                arch: ad_arch.into(),
            }
            .resolve(prepared.merged.java_major)
            ?,
        };

        // 5) LaunchPlan (§8.9 메모리 우선순위: 사용자 > 매니페스트 권장 > 기본)
        let manifest = store::load_cached_manifest(&paths, &id);
        let (min_mb, max_mb) = cfg
            .jvm
            .as_ref()
            .map(|j| (j.min_memory_mb, j.max_memory_mb))
            .or_else(|| {
                manifest
                    .as_ref()
                    .and_then(|m| m.recommended_jvm.as_ref())
                    .map(|r| (r.min_memory_mb, r.max_memory_mb))
            })
            .unwrap_or((2048, 4096));
        let account = MockAuthProvider.sign_in()?; // TODO(M3): 실계정
        let server = cfg.server.as_ref().and_then(|s| {
            let on = s.direct_connect_override.unwrap_or_else(|| {
                manifest
                    .as_ref()
                    .map(|m| m.server.direct_connect_default)
                    .unwrap_or(false)
            });
            on.then_some((s.address.as_str(), s.port))
        });
        let instance_dir = paths.instance_dir(&id);
        let extra = cfg.jvm.as_ref().map(|j| j.extra_args.clone()).unwrap_or_default();
        let plan = build_launch_plan(
            &prepared.merged,
            &cache,
            java,
            &LaunchConfig {
                instance_dir: &instance_dir,
                player_name: &account.gamertag,
                uuid: &account.mc_uuid,
                access_token: "mock-token", // TODO(M3): 실토큰 — 마스킹은 plan이 보장
                xuid: "0",
                client_id: "0",
                min_memory_mb: min_mb,
                max_memory_mb: max_mb,
                extra_jvm_args: &extra,
                server,
            },
            &ctx,
        )
        ?;

        // 6) 실행 + 종료 감시 (§8.15-6). 게임 수명은 별도 스레드 — 커맨드는 기동 후 반환.
        cfg.last_played = Some(now_label());
        store::save_instance(&paths, &cfg)?;
        progress.emit(&id, "launch", &prepared.merged.id);
        let app2 = app.clone();
        let id2 = id.clone();
        std::thread::spawn(move || {
            let mut args = plan.jvm_args.args.clone();
            args.push(plan.main_class.clone());
            args.extend(plan.game_args.args.clone());
            // §8.12: 게임 출력 tail 링 버퍼 — 비정상 종료 시 크래시 로그로 보존
            const RING_CAP: usize = 400;
            let mut ring: VecDeque<String> = VecDeque::with_capacity(RING_CAP);
            let code = spawn_and_wait(&plan.java_path, &args, &plan.cwd, |line| {
                if ring.len() == RING_CAP {
                    ring.pop_front();
                }
                ring.push_back(line);
            })
            .ok()
            .flatten();

            let abnormal = matches!(code, Some(c) if c != 0);
            let crash_loop = {
                let mut map = crash_trackers.lock().expect("crash tracker lock");
                let tracker = map.entry(id2.clone()).or_default();
                if abnormal {
                    tracker.record_abnormal_exit(unix_secs())
                } else {
                    tracker.reset();
                    false
                }
            };
            if abnormal {
                // 마스킹(§8.13) 후 저장 — 인앱 크래시 뷰어(후속)의 입력이 된다
                let log_dir = instance_dir.join("logs");
                let _ = std::fs::create_dir_all(&log_dir);
                let tail: String = ring
                    .iter()
                    .map(|l| crate::diag::mask(l))
                    .collect::<Vec<_>>()
                    .join("\n");
                let _ = std::fs::write(
                    log_dir.join(format!("aqua-crash-{}.log", unix_secs())),
                    tail,
                );
                tracing::warn!(instance = %id2, ?code, crash_loop, "game exited abnormally");
            }
            let _ = app2.emit("game-exited", GameExited { id: id2, code, crash_loop });
        });
        Ok(())
    })
    .await
    .map_err(AppError::internal)?
}

fn progress_emit_shim(progress: &mut Throttled, id: &str, stage: &str, item: &str) {
    progress.emit(id, stage, item);
}
