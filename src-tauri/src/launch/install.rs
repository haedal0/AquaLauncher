//! 실행 준비 오케스트레이터 — TD-01 §0 흐름의 결선.
//! version JSON 확보 → 병합 → 아티팩트 다운로드 → LaunchPlan 조립.
use super::args::{join_args, substitute, SubstContext};
use super::artifacts::{plan_asset_downloads, plan_client_download, plan_library_downloads};
use super::classpath::{dedupe_coords, maven_to_rel_path};
use super::rules::{evaluate, RuleContext};
use super::version_json::{
    merge_chain, resolve_chain, ArgumentEntry, MergeError, MergedVersion, ValueOrList, VersionJson,
};
use super::LaunchPlan;
use crate::net::download::{download_verified, ExpectedHash};
use crate::net::mojang::{MetaError, MojangMeta};
use crate::net::Fetch;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PrepareError {
    #[error(transparent)]
    Meta(#[from] MetaError),
    #[error(transparent)]
    Merge(#[from] MergeError),
    #[error("version json parse: {0}")]
    Parse(#[from] serde_json::Error),
    /// E-MF-04 상당 — 실패 파일 목록 (부분 재시도용)
    #[error("{} artifact(s) failed to download", failed.len())]
    Download { failed: Vec<String> },
    #[error("invalid classpath coordinate: {0}")]
    Coord(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn read_cached_version_json(cache_root: &Path, id: &str) -> Option<VersionJson> {
    let path = cache_root.join("versions").join(id).join(format!("{id}.json"));
    let raw = fs::read(path).ok()?;
    serde_json::from_slice(&raw).ok()
}

fn save_version_json(cache_root: &Path, id: &str, raw: &[u8]) -> std::io::Result<()> {
    let dir = cache_root.join("versions").join(id);
    fs::create_dir_all(&dir)?;
    let dest = dir.join(format!("{id}.json"));
    if !dest.is_file() {
        let tmp = dest.with_extension("json.tmp-write");
        fs::write(&tmp, raw)?;
        fs::rename(&tmp, &dest)?;
    }
    Ok(())
}

/// 바닐라 version JSON을 캐시에 확보(없으면 Mojang 메타에서 수신)하고 파싱한다.
pub fn ensure_vanilla_json(
    fetch: &dyn Fetch,
    cache_root: &Path,
    mc_version: &str,
) -> Result<VersionJson, PrepareError> {
    if let Some(v) = read_cached_version_json(cache_root, mc_version) {
        return Ok(v);
    }
    let meta = MojangMeta { fetch };
    let manifest = meta.version_manifest()?;
    let vref = MojangMeta::find_version(&manifest, mc_version)?;
    let raw = fetch.get_bytes(&vref.url).map_err(MetaError::from)?;
    let parsed: VersionJson = serde_json::from_slice(&raw)?;
    save_version_json(cache_root, &parsed.id, &raw)?;
    Ok(parsed)
}

/// 준비 완료 상태: 병합 결과 + 다운로드 수행 내역.
pub struct PreparedVersion {
    pub merged: MergedVersion,
    pub downloaded: usize,
}

/// 버전 준비: leaf(로더 profile 또는 바닐라)에서 체인 병합 후 전체 아티팩트 확보.
/// on_item(단계, 항목) — PRD 8.10 진행 표시용 콜백 (스로틀은 UI 계층).
pub fn prepare_version(
    fetch: &dyn Fetch,
    cache_root: &Path,
    mc_version: &str,
    loader_profile_path: Option<&Path>,
    ctx: &RuleContext,
    on_item: &mut (dyn FnMut(&str, &str) + Send),
) -> Result<PreparedVersion, PrepareError> {
    // 1) 바닐라 JSON 확보 (체인 lookup 대상)
    ensure_vanilla_json(fetch, cache_root, mc_version)?;

    // 2) leaf 결정 및 체인 병합
    let leaf: VersionJson = match loader_profile_path {
        Some(p) => serde_json::from_slice(&fs::read(p)?)?,
        None => read_cached_version_json(cache_root, mc_version)
            .expect("vanilla json ensured above"),
    };
    let chain = resolve_chain(leaf, |id| read_cached_version_json(cache_root, id))?;
    let merged = merge_chain(chain)?;

    // 3) 다운로드 계획 수립
    let mut planned = plan_library_downloads(&merged.libraries, ctx);
    planned.extend(plan_client_download(&merged));
    if let Some(index_ref) = &merged.asset_index {
        if let Some(url) = &index_ref.url {
            let meta = MojangMeta { fetch };
            let index = meta.asset_index(url)?;
            let dir = cache_root.join("assets/indexes");
            fs::create_dir_all(&dir)?;
            let dest = dir.join(format!("{}.json", index_ref.id));
            if !dest.is_file() {
                fs::write(&dest, serde_json::to_vec(&serde_json::json!({}))?)?; // 자리표시 후 원문 저장
                let raw = fetch.get_bytes(url).map_err(MetaError::from)?;
                fs::write(&dest, raw)?;
            }
            planned.extend(plan_asset_downloads(&index));
        }
    }

    // 4) 다운로드 실행 — 동시 4 (PRD 8.2.5), 실패는 모아서 E-MF-04 상당으로 보고
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    let next = AtomicUsize::new(0);
    let downloaded = AtomicUsize::new(0);
    let failed: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let progress: Mutex<ProgressFn<'_>> = Mutex::new(on_item);

    std::thread::scope(|s| {
        for _ in 0..DOWNLOAD_CONCURRENCY {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some(item) = planned.get(i) else { break };
                (progress.lock().expect("progress lock"))("download", &item.dest_rel);
                let dest = cache_root.join(&item.dest_rel);
                let existed = dest.is_file();
                let expected = match &item.sha1 {
                    Some(h) => ExpectedHash::Sha1(h),
                    None => ExpectedHash::None,
                };
                match download_verified(fetch, &dest, &item.url, expected) {
                    Ok(_) => {
                        if !existed {
                            downloaded.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                    Err(_) => failed.lock().expect("failed lock").push(item.dest_rel.clone()),
                }
            });
        }
    });

    let failed = failed.into_inner().expect("failed lock");
    if !failed.is_empty() {
        return Err(PrepareError::Download { failed });
    }
    Ok(PreparedVersion { merged, downloaded: downloaded.into_inner() })
}

/// 동시 다운로드 수 기본값 (PRD 8.2.5 — 전역 설정 연동은 후속)
pub const DOWNLOAD_CONCURRENCY: usize = 4;

/// 진행 콜백 — (단계, 항목). 다운로드 워커 스레드에서 공유 호출.
pub type ProgressFn<'a> = &'a mut (dyn FnMut(&str, &str) + Send);

/// ArgumentEntry 목록을 rules 평가해 평탄화 (TD-01 §2).
fn flatten_args(entries: &[ArgumentEntry], ctx: &RuleContext) -> Vec<String> {
    let mut out = Vec::new();
    for e in entries {
        match e {
            ArgumentEntry::Plain(s) => out.push(s.clone()),
            ArgumentEntry::Conditional { rules, value } => {
                if evaluate(rules, ctx) {
                    match value {
                        ValueOrList::One(s) => out.push(s.clone()),
                        ValueOrList::Many(v) => out.extend(v.iter().cloned()),
                    }
                }
            }
        }
    }
    out
}

/// 실행 설정 — 인증/메모리/접속 정보.
pub struct LaunchConfig<'a> {
    pub instance_dir: &'a Path,
    pub player_name: &'a str,
    pub uuid: &'a str,
    /// 마스킹 대상 (PRD 8.15-5)
    pub access_token: &'a str,
    pub xuid: &'a str,
    pub client_id: &'a str,
    pub min_memory_mb: u32,
    pub max_memory_mb: u32,
    pub extra_jvm_args: &'a [String],
    /// 자동 접속 대상 (§8.6 버전 분기는 join_args가 처리)
    pub server: Option<(&'a str, u16)>,
}

/// LaunchPlan 조립 — 순수 함수 (TD-01 §0, §3, §6).
pub fn build_launch_plan(
    merged: &MergedVersion,
    cache_root: &Path,
    java_path: PathBuf,
    cfg: &LaunchConfig<'_>,
    ctx: &RuleContext,
) -> Result<LaunchPlan, PrepareError> {
    // ── 클래스패스 (TD-01 §3): rules 필터 → 중복 해소(로더 우선) → 캐시 경로 ──
    let coords: Vec<&str> = merged
        .libraries
        .iter()
        .filter(|l| evaluate(&l.rules, ctx))
        .map(|l| l.name.as_str())
        .collect();
    let deduped = dedupe_coords(&coords).map_err(|e| PrepareError::Coord(e.to_string()))?;
    let sep = if ctx.os_name == "windows" { ';' } else { ':' };
    let mut cp_parts: Vec<String> = Vec::new();
    for coord in &deduped {
        let rel = maven_to_rel_path(coord).map_err(|e| PrepareError::Coord(e.to_string()))?;
        cp_parts.push(cache_root.join("libraries").join(rel).to_string_lossy().into_owned());
    }
    cp_parts.push(
        cache_root
            .join(format!("versions/{id}/{id}.jar", id = merged.root_id))
            .to_string_lossy()
            .into_owned(),
    );
    let classpath = cp_parts.join(&sep.to_string());

    // ── 치환 문맥 (TD-01 §6 테이블) ──
    let mut subst = SubstContext::default();
    subst.set("auth_player_name", cfg.player_name);
    subst.set("auth_uuid", cfg.uuid);
    subst.set_secret("auth_access_token", cfg.access_token);
    subst.set_secret("auth_xuid", cfg.xuid);
    subst.set_secret("clientid", cfg.client_id);
    subst.set("user_type", "msa");
    subst.set("version_name", &merged.id);
    subst.set("version_type", merged.release_type.as_deref().unwrap_or("release"));
    subst.set("game_directory", cfg.instance_dir.to_string_lossy());
    subst.set("assets_root", cache_root.join("assets").to_string_lossy());
    subst.set("assets_index_name", merged.assets.as_deref().unwrap_or(""));
    subst.set(
        "natives_directory",
        cfg.instance_dir.join("natives").join(&merged.id).to_string_lossy(),
    );
    subst.set("launcher_name", "AquaLauncher");
    subst.set("launcher_version", env!("CARGO_PKG_VERSION"));
    subst.set("classpath", &classpath);
    subst.set("library_directory", cache_root.join("libraries").to_string_lossy());
    subst.set("classpath_separator", sep.to_string());

    // ── JVM 인자: version JSON + 메모리 + 사용자 추가 (§8.9 우선순위는 호출측 결정) ──
    let mut jvm_raw = flatten_args(&merged.jvm_args, ctx);
    jvm_raw.push(format!("-Xms{}m", cfg.min_memory_mb));
    jvm_raw.push(format!("-Xmx{}m", cfg.max_memory_mb));
    jvm_raw.extend(cfg.extra_jvm_args.iter().cloned());

    // ── game 인자 + 자동 접속 (§8.6 버전 분기) ──
    let mut game_raw = flatten_args(&merged.game_args, ctx);
    if let Some((host, port)) = cfg.server {
        game_raw.extend(join_args(&merged.root_id, host, port));
    }

    Ok(LaunchPlan {
        java_path,
        jvm_args: substitute(&jvm_raw, &subst),
        main_class: merged.main_class.clone(),
        game_args: substitute(&game_raw, &subst),
        cwd: cfg.instance_dir.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::test_support::MockFetch;
    use crate::net::mojang::VERSION_MANIFEST_URL;
    use sha1::{Digest, Sha1};

    fn sha1_hex(b: &[u8]) -> String {
        let mut h = Sha1::new();
        h.update(b);
        h.finalize().iter().map(|x| format!("{x:02x}")).collect()
    }

    const LIB_BODY: &[u8] = b"LIB BYTES";
    const CLIENT_BODY: &[u8] = b"CLIENT BYTES";
    const ASSET_BODY: &[u8] = b"ASSET BYTES";

    fn vanilla_json() -> String {
        format!(
            r#"{{
              "id": "1.20.4", "type": "release", "mainClass": "net.minecraft.client.main.Main",
              "assets": "12", "javaVersion": {{"majorVersion": 17}},
              "assetIndex": {{"id": "12", "url": "https://piston-meta.mojang.com/assets/12.json"}},
              "downloads": {{"client": {{"sha1": "{client_sha}", "size": 12, "url": "https://piston-data.mojang.com/client.jar"}}}},
              "arguments": {{
                "jvm": ["-Djava.library.path=${{natives_directory}}", "-cp", "${{classpath}}"],
                "game": ["--username", "${{auth_player_name}}", "--accessToken", "${{auth_access_token}}",
                         "--assetIndex", "${{assets_index_name}}"]
              }},
              "libraries": [{{"name": "com.mojang:logging:1.1.1",
                "downloads": {{"artifact": {{"path": "com/mojang/logging/1.1.1/logging-1.1.1.jar",
                  "sha1": "{lib_sha}", "size": 9, "url": "https://libraries.minecraft.net/logging.jar"}}}}}}]
            }}"#,
            client_sha = sha1_hex(CLIENT_BODY),
            lib_sha = sha1_hex(LIB_BODY),
        )
    }

    fn asset_hash() -> String {
        sha1_hex(ASSET_BODY)
    }

    fn full_fetch() -> MockFetch {
        let manifest = format!(
            r#"{{"latest": {{"release": "1.20.4", "snapshot": "x"}},
                "versions": [{{"id": "1.20.4", "type": "release", "url": "https://piston-meta.mojang.com/v1/1.20.4.json"}}]}}"#
        );
        let asset_index = format!(
            r#"{{"objects": {{"icons/icon.png": {{"hash": "{h}", "size": {n}}}}}}}"#,
            h = asset_hash(),
            n = ASSET_BODY.len()
        );
        let asset_url = format!(
            "https://resources.download.minecraft.net/{}/{}",
            &asset_hash()[..2],
            asset_hash()
        );
        let mut f = MockFetch::with(&[]);
        f.responses = [
            (VERSION_MANIFEST_URL.to_string(), manifest.into_bytes()),
            ("https://piston-meta.mojang.com/v1/1.20.4.json".to_string(), vanilla_json().into_bytes()),
            ("https://piston-meta.mojang.com/assets/12.json".to_string(), asset_index.into_bytes()),
            ("https://libraries.minecraft.net/logging.jar".to_string(), LIB_BODY.to_vec()),
            ("https://piston-data.mojang.com/client.jar".to_string(), CLIENT_BODY.to_vec()),
            (asset_url, ASSET_BODY.to_vec()),
        ]
        .into_iter()
        .collect();
        f
    }

    #[test]
    fn prepare_vanilla_downloads_everything_into_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path();
        let f = full_fetch();
        let mut seen = Vec::new();
        let prepared = prepare_version(
            &f,
            cache,
            "1.20.4",
            None,
            &RuleContext::new("osx", "arm64"),
            &mut |_, item| seen.push(item.to_string()),
        )
        .unwrap();
        assert_eq!(prepared.downloaded, 3, "lib + client + asset");
        assert_eq!(prepared.merged.java_major, 17);
        assert!(cache.join("versions/1.20.4/1.20.4.json").is_file());
        assert!(cache.join("versions/1.20.4/1.20.4.jar").is_file());
        assert!(cache.join("libraries/com/mojang/logging/1.1.1/logging-1.1.1.jar").is_file());
        assert!(cache.join("assets/indexes/12.json").is_file());
        assert!(cache
            .join(format!("assets/objects/{}/{}", &asset_hash()[..2], asset_hash()))
            .is_file());
        assert_eq!(seen.len(), 3);

        // 재실행: 전부 캐시 적중 → 신규 다운로드 0
        let again = prepare_version(
            &f, cache, "1.20.4", None,
            &RuleContext::new("osx", "arm64"), &mut |_, _| {},
        )
        .unwrap();
        assert_eq!(again.downloaded, 0);
    }

    #[test]
    fn prepare_with_fabric_profile_merges_chain() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path();
        let f = full_fetch();
        // Fabric profile을 캐시에 배치 (FabricLikeLoader::install 산출물 형태)
        let profile_dir = cache.join("versions/fabric-loader-0.15.7-1.20.4");
        fs::create_dir_all(&profile_dir).unwrap();
        let profile_path = profile_dir.join("fabric-loader-0.15.7-1.20.4.json");
        fs::write(
            &profile_path,
            br#"{"id": "fabric-loader-0.15.7-1.20.4", "inheritsFrom": "1.20.4",
                "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
                "libraries": []}"#,
        )
        .unwrap();
        let prepared = prepare_version(
            &f, cache, "1.20.4", Some(&profile_path),
            &RuleContext::new("osx", "arm64"), &mut |_, _| {},
        )
        .unwrap();
        assert_eq!(prepared.merged.id, "fabric-loader-0.15.7-1.20.4");
        assert_eq!(prepared.merged.root_id, "1.20.4");
        assert_eq!(prepared.merged.main_class, "net.fabricmc.loader.impl.launch.knot.KnotClient");
    }

    /// M1 게이트(mac): 실네트워크로 바닐라 1.20.4 전체 준비 + 실제 JVM 기동 검증.
    /// 게임 창이 잠깐 열렸다 닫힌다. 실행: cargo test -p aqua-launcher -- --ignored m1_gate --nocapture
    #[test]
    #[ignore = "실네트워크 + 게임 프로세스 기동 — M1 게이트 수동 검증용"]
    fn m1_gate_real_vanilla_1_20_4_launch_on_mac() {
        use crate::launch::java::AdoptiumProvider;
        use crate::launch::JavaRuntimeProvider;
        use crate::net::HttpFetcher;
        use std::io::BufRead;
        use std::time::{Duration, Instant};

        let fetch = HttpFetcher::new().unwrap();
        let base = std::env::temp_dir().join("aqua-m1-gate");
        let cache = base.join("cache");
        let instance = base.join("instance");
        fs::create_dir_all(&instance).unwrap();
        let ctx = RuleContext::new("osx", "arm64");

        let t0 = Instant::now();
        let prepared = prepare_version(&fetch, &cache, "1.20.4", None, &ctx, &mut |_, _| {})
            .expect("prepare_version");
        eprintln!(
            "[m1-gate] prepared: {} new downloads in {:?} (java major {})",
            prepared.downloaded, t0.elapsed(), prepared.merged.java_major
        );

        let provider = AdoptiumProvider {
            fetch: &fetch,
            cache_root: cache.clone(),
            os: "mac".into(),
            arch: "aarch64".into(),
        };
        let java = provider.resolve(prepared.merged.java_major).expect("jre");
        eprintln!("[m1-gate] java: {}", java.display());

        let cfg = LaunchConfig {
            instance_dir: &instance,
            player_name: "AquaSmoke",
            uuid: "00000000-0000-0000-0000-000000000000",
            access_token: "smoke-token",
            xuid: "0",
            client_id: "0",
            min_memory_mb: 1024,
            max_memory_mb: 2048,
            extra_jvm_args: &[],
            server: None,
        };
        let plan = build_launch_plan(&prepared.merged, &cache, java, &cfg, &ctx).unwrap();

        let mut args = plan.jvm_args.args.clone();
        args.push(plan.main_class.clone());
        args.extend(plan.game_args.args.clone());
        let mut child = std::process::Command::new(&plan.java_path)
            .args(&args)
            .current_dir(&plan.cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn game");

        // "Setting user:" 로그 = 게임 부팅 성공 판정. 이후 즉시 종료.
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut booted = false;
        let mut tail: Vec<String> = Vec::new();
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_secs(5)) {
                Ok(line) => {
                    if tail.len() > 50 { tail.remove(0); }
                    tail.push(line.clone());
                    if line.contains("Setting user: AquaSmoke") {
                        booted = true;
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if let Ok(Some(status)) = child.try_wait() {
                        panic!("game exited early: {status:?}\n--- tail ---\n{}", tail.join("\n"));
                    }
                }
                Err(_) => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        assert!(booted, "did not see 'Setting user' — tail:\n{}", tail.join("\n"));
        eprintln!("[m1-gate] SUCCESS: vanilla 1.20.4 booted in {:?} total", t0.elapsed());
    }

    #[test]
    fn build_plan_masks_token_and_assembles_classpath() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path();
        let f = full_fetch();
        let ctx = RuleContext::new("osx", "arm64");
        let prepared =
            prepare_version(&f, cache, "1.20.4", None, &ctx, &mut |_, _| {}).unwrap();
        let inst = dir.path().join("instance");
        let cfg = LaunchConfig {
            instance_dir: &inst,
            player_name: "Steve_KR",
            uuid: "u-u-i-d",
            access_token: "SECRET-TOKEN",
            xuid: "XUID-1",
            client_id: "CID",
            min_memory_mb: 2048,
            max_memory_mb: 4096,
            extra_jvm_args: &[],
            server: Some(("play.example.com", 25565)),
        };
        let plan =
            build_launch_plan(&prepared.merged, cache, PathBuf::from("/usr/bin/java"), &cfg, &ctx)
                .unwrap();

        assert_eq!(plan.main_class, "net.minecraft.client.main.Main");
        let cp = &plan.jvm_args.args[plan.jvm_args.args.iter().position(|a| a == "-cp").unwrap() + 1];
        assert!(cp.contains("logging-1.1.1.jar"));
        assert!(cp.contains("versions/1.20.4/1.20.4.jar"));
        assert!(cp.contains(':'), "mac 구분자");
        assert!(plan.jvm_args.args.iter().any(|a| a == "-Xmx4096m"));

        // 토큰: 실제 인자에는 있으나 마스킹 출력에는 없어야 함
        assert!(plan.game_args.args.iter().any(|a| a == "SECRET-TOKEN"));
        let shown = plan.game_args.display_masked().join(" ");
        assert!(!shown.contains("SECRET-TOKEN"));

        // 1.20.4 → quickPlay 분기 (§8.6)
        assert!(plan.game_args.args.iter().any(|a| a == "--quickPlayMultiplayer"));
        assert!(plan.game_args.args.iter().any(|a| a == "play.example.com:25565"));
        // assetIndex 치환
        let ai = plan.game_args.args.iter().position(|a| a == "--assetIndex").unwrap();
        assert_eq!(plan.game_args.args[ai + 1], "12");
    }
}
