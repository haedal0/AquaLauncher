//! Forge/NeoForge 로더 — PRD §8.1 + docs/spike-forge-report.md 확정 규약.
//! 흐름: 작업 디렉토리 준비(launcher_profiles.json 더미 + client.jar 사전 스테이징)
//! → 인스톨러 headless 실행 → 산출물(versions/, libraries/)을 공유 캐시로 수확.
//!
//! `InstallContext.work_dir`는 호출자가 소유·정리하는 전용 임시 디렉토리여야 한다
//! (인스톨러가 CWD에 `<installer>.jar.log` 등 부산물을 남긴다 — 스파이크 §남은 리스크).
use super::{InstallContext, LaunchProfile, LoaderVersion, ModLoader};
use crate::error::AppError;
use crate::net::download::{download_verified, ExpectedHash};
use crate::net::Fetch;
use aqua_manifest::path::safe_join;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt::Display;
use std::fs;
use std::path::{Path, PathBuf};

pub const FORGE_PROMOTIONS_URL: &str =
    "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";
pub const FORGE_MAVEN: &str = "https://maven.minecraftforge.net/net/minecraftforge/forge";
pub const NEOFORGE_VERSIONS_URL: &str =
    "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge";
pub const NEOFORGE_MAVEN: &str = "https://maven.neoforged.net/releases/net/neoforged/neoforge";

pub struct InstallerOutput {
    pub exit_ok: bool,
    /// 실패 표면화용 로그 꼬리 (E-LD-01: 인스톨러 로그 표시, PRD §9)
    pub stdout_tail: String,
}

/// 인스톨러 프로세스 실행 추상화 — 실제는 `java -jar`, 테스트는 목 (AGENT.md 외부 의존 규칙).
pub trait InstallerRunner: Send + Sync {
    fn run(&self, installer_jar: &Path, work_dir: &Path) -> Result<InstallerOutput, AppError>;
}

/// `java -jar <installer> --installClient <work_dir>` — 스파이크 §프로세스 동작.
/// 종료 코드는 신뢰 가능(성공 0), 실패 사유는 stdout에 있으므로 꼬리를 보존한다.
pub struct JavaInstallerRunner {
    pub java: PathBuf,
}

impl InstallerRunner for JavaInstallerRunner {
    fn run(&self, installer_jar: &Path, work_dir: &Path) -> Result<InstallerOutput, AppError> {
        let out = std::process::Command::new(&self.java)
            .arg("-jar")
            .arg(installer_jar)
            .arg("--installClient")
            .arg(work_dir)
            .current_dir(work_dir)
            .output()
            .map_err(|e| AppError::LoaderInstall(format!("installer spawn failed: {e}")))?;
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        let tail_start = text.len().saturating_sub(4000);
        let tail = text[text.char_indices().map(|(i, _)| i).find(|&i| i >= tail_start).unwrap_or(0)..]
            .to_string();
        Ok(InstallerOutput { exit_ok: out.status.success(), stdout_tail: tail })
    }
}

pub struct ForgeLikeLoader<'a> {
    id: &'static str,
    fetch: &'a dyn Fetch,
    runner: &'a dyn InstallerRunner,
}

impl<'a> ForgeLikeLoader<'a> {
    pub fn forge(fetch: &'a dyn Fetch, runner: &'a dyn InstallerRunner) -> Self {
        ForgeLikeLoader { id: "forge", fetch, runner }
    }
    pub fn neoforge(fetch: &'a dyn Fetch, runner: &'a dyn InstallerRunner) -> Self {
        ForgeLikeLoader { id: "neoforge", fetch, runner }
    }

    fn err(e: impl Display) -> AppError {
        AppError::LoaderInstall(e.to_string())
    }

    /// 산출물 version id — 로더별 명명 규칙 차이를 여기서 흡수 (스파이크 §산출물 상세).
    fn version_id(&self, mc_version: &str, loader_version: &str) -> String {
        match self.id {
            "forge" => format!("{mc_version}-forge-{loader_version}"),
            _ => format!("neoforge-{loader_version}"),
        }
    }

    fn installer_url(&self, mc_version: &str, loader_version: &str) -> String {
        match self.id {
            "forge" => format!(
                "{FORGE_MAVEN}/{mc_version}-{loader_version}/forge-{mc_version}-{loader_version}-installer.jar"
            ),
            _ => format!("{NEOFORGE_MAVEN}/{loader_version}/neoforge-{loader_version}-installer.jar"),
        }
    }

    /// NeoForge 버전 체계: `<mc_minor>.<mc_patch>.<빌드>` — MC 1.20.4 → 접두 "20.4."
    fn neo_prefix(mc_version: &str) -> Option<String> {
        let mut it = mc_version.split('.');
        let _major = it.next()?;
        let minor = it.next()?;
        let patch = it.next().unwrap_or("0");
        Some(format!("{minor}.{patch}."))
    }
}

/// `src_root` 아래 전체 파일을 캐시의 `<rel_prefix>/...`로 수확.
/// 공유 캐시 불변 규칙(PRD §7.1): 이미 있는 경로는 건너뛴다. 쓰기는 tmp → rename.
fn harvest_dir(src_root: &Path, cache_root: &Path, rel_prefix: &str) -> Result<(), AppError> {
    if !src_root.is_dir() {
        return Ok(());
    }
    let mut stack = vec![src_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).map_err(ForgeLikeLoader::err)? {
            let path = entry.map_err(ForgeLikeLoader::err)?.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path.strip_prefix(src_root).map_err(ForgeLikeLoader::err)?;
            let rel_str = rel
                .to_str()
                .ok_or_else(|| ForgeLikeLoader::err(format!("non-utf8 path: {rel:?}")))?;
            // 인스톨러 산출물도 외부 입력(§11) — 캐시 경로 조합은 safe_join으로만
            let dest = safe_join(cache_root, &format!("{rel_prefix}/{rel_str}"))
                .map_err(ForgeLikeLoader::err)?;
            if dest.exists() {
                continue; // 불변 캐시 — 덮어쓰기 금지
            }
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(ForgeLikeLoader::err)?;
            }
            let tmp = dest.with_extension("tmp-harvest");
            fs::copy(&path, &tmp).map_err(ForgeLikeLoader::err)?;
            fs::rename(&tmp, &dest).map_err(ForgeLikeLoader::err)?;
        }
    }
    Ok(())
}

impl ModLoader for ForgeLikeLoader<'_> {
    fn id(&self) -> &'static str {
        self.id
    }

    fn list_versions(&self, mc_version: &str) -> Result<Vec<LoaderVersion>, AppError> {
        match self.id {
            "forge" => {
                #[derive(Deserialize)]
                struct Promotions {
                    promos: BTreeMap<String, String>,
                }
                let raw = self.fetch.get_bytes(FORGE_PROMOTIONS_URL).map_err(Self::err)?;
                let p: Promotions = serde_json::from_slice(&raw).map_err(Self::err)?;
                let recommended = p.promos.get(&format!("{mc_version}-recommended"));
                let latest = p.promos.get(&format!("{mc_version}-latest"));
                let mut out = Vec::new();
                if let Some(r) = recommended {
                    out.push(LoaderVersion { id: r.clone(), stable: true });
                }
                if let Some(l) = latest {
                    if recommended != Some(l) {
                        out.push(LoaderVersion { id: l.clone(), stable: false });
                    }
                }
                Ok(out)
            }
            _ => {
                #[derive(Deserialize)]
                struct Versions {
                    versions: Vec<String>,
                }
                let prefix = Self::neo_prefix(mc_version)
                    .ok_or_else(|| Self::err(format!("bad mc version: {mc_version}")))?;
                let raw = self.fetch.get_bytes(NEOFORGE_VERSIONS_URL).map_err(Self::err)?;
                let v: Versions = serde_json::from_slice(&raw).map_err(Self::err)?;
                Ok(v.versions
                    .into_iter()
                    .filter(|v| v.starts_with(&prefix))
                    .rev() // maven은 오름차순 — 최신 우선으로
                    .map(|v| LoaderVersion { stable: !v.contains("beta"), id: v })
                    .collect())
            }
        }
    }

    fn install(
        &self,
        mc_version: &str,
        loader_version: &str,
        ctx: &InstallContext,
    ) -> Result<LaunchProfile, AppError> {
        let vid = self.version_id(mc_version, loader_version);
        let dest = safe_join(ctx.shared_cache_root, &format!("versions/{vid}/{vid}.json"))
            .map_err(Self::err)?;
        if dest.is_file() {
            return Ok(LaunchProfile { version_json_path: dest }); // 불변 캐시 재사용
        }

        let work = ctx.work_dir;
        fs::create_dir_all(work).map_err(Self::err)?;
        // 함정 1 (§8.1): launcher_profiles.json 없으면 설치 거부 → 더미 사전 생성
        fs::write(work.join("launcher_profiles.json"), br#"{"profiles":{}}"#).map_err(Self::err)?;

        // 함정 2 (§8.1): client.jar 사전 스테이징 — 캐시에 있으면 재다운로드 방지 (하드링크, 실패 시 복사)
        let client_rel = format!("versions/{mc_version}/{mc_version}.jar");
        let cached_client = safe_join(ctx.shared_cache_root, &client_rel).map_err(Self::err)?;
        if cached_client.is_file() {
            let staged = safe_join(work, &client_rel).map_err(Self::err)?;
            if let Some(parent) = staged.parent() {
                fs::create_dir_all(parent).map_err(Self::err)?;
            }
            if !staged.is_file() && fs::hard_link(&cached_client, &staged).is_err() {
                fs::copy(&cached_client, &staged).map_err(Self::err)?;
            }
        }

        // 인스톨러 수급 — §8.2.5 패턴(part → 해시 검증 → 원자적 rename), maven .sha1 대조
        let url = self.installer_url(mc_version, loader_version);
        let sha1_raw = self.fetch.get_bytes(&format!("{url}.sha1")).map_err(Self::err)?;
        let sha1 = String::from_utf8_lossy(&sha1_raw).trim().to_string();
        let installer = work.join("installer.jar");
        download_verified(self.fetch, &installer, &url, ExpectedHash::Sha1(&sha1))
            .map_err(Self::err)?;

        let out = self.runner.run(&installer, work)?;
        if !out.exit_ok {
            // E-LD-01: 인스톨러 로그 표시 + 재시도/다른 버전 제안 (PRD §9)
            return Err(AppError::LoaderInstall(format!(
                "installer exited with failure:\n{}",
                out.stdout_tail
            )));
        }

        for top in ["versions", "libraries"] {
            harvest_dir(&work.join(top), ctx.shared_cache_root, top)?;
        }
        if !dest.is_file() {
            return Err(AppError::LoaderInstall(format!("installer did not produce {vid}.json")));
        }
        Ok(LaunchProfile { version_json_path: dest })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use crate::loaders::{InstallContext, ModLoader};
    use crate::net::test_support::MockFetch;
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn sha1_hex(bytes: &[u8]) -> String {
        use sha1::{Digest, Sha1};
        let mut out = String::new();
        for b in Sha1::digest(bytes) {
            use std::fmt::Write;
            let _ = write!(out, "{b:02x}");
        }
        out
    }

    /// 호출되면 안 되는 경로 검증용.
    struct PanicRunner;
    impl InstallerRunner for PanicRunner {
        fn run(&self, _: &Path, _: &Path) -> Result<InstallerOutput, AppError> {
            panic!("installer must not run in this scenario")
        }
    }

    /// 스파이크 보고서의 인스톨러 동작 모사 — 전제(함정 1) 검사 + 산출물 생성.
    struct FakeInstaller {
        version_id: &'static str,
        exit_ok: bool,
        called: AtomicBool,
    }
    impl InstallerRunner for FakeInstaller {
        fn run(&self, installer_jar: &Path, work_dir: &Path) -> Result<InstallerOutput, AppError> {
            self.called.store(true, Ordering::SeqCst);
            assert!(installer_jar.is_file(), "인스톨러 jar는 실행 전에 검증·저장돼 있어야 함");
            // 함정 1 (§8.1): launcher_profiles.json 더미 없이는 인스톨러가 거부한다
            assert!(
                work_dir.join("launcher_profiles.json").is_file(),
                "launcher_profiles.json 더미가 사전 생성돼야 함"
            );
            if !self.exit_ok {
                return Ok(InstallerOutput {
                    exit_ok: false,
                    stdout_tail: "There was an error during installation".into(),
                });
            }
            let vdir = work_dir.join("versions").join(self.version_id);
            fs::create_dir_all(&vdir).unwrap();
            fs::write(vdir.join(format!("{}.json", self.version_id)), br#"{"id": "X"}"#).unwrap();
            let lib = work_dir.join("libraries/net/minecraftforge/forge");
            fs::create_dir_all(&lib).unwrap();
            fs::write(lib.join("forge-universal.jar"), b"LIB").unwrap();
            Ok(InstallerOutput { exit_ok: true, stdout_tail: String::new() })
        }
    }

    #[test]
    fn forge_list_versions_uses_promotions() {
        let promos = br#"{"homepage": "h", "promos": {
            "1.20.1-recommended": "47.4.10", "1.20.1-latest": "47.4.11",
            "1.19.2-recommended": "43.3.0"}}"#;
        let f = MockFetch::with(&[(FORGE_PROMOTIONS_URL, promos.as_slice())]);
        let loader = ForgeLikeLoader::forge(&f, &PanicRunner);
        let vs = loader.list_versions("1.20.1").unwrap();
        let got: Vec<(&str, bool)> = vs.iter().map(|v| (v.id.as_str(), v.stable)).collect();
        assert_eq!(got, [("47.4.10", true), ("47.4.11", false)]);
    }

    #[test]
    fn neoforge_list_versions_filters_by_mc_and_marks_beta_unstable() {
        let body = br#"{"isSnapshot": false,
            "versions": ["20.4.100", "20.4.251", "20.4.300-beta", "21.1.1"]}"#;
        let f = MockFetch::with(&[(NEOFORGE_VERSIONS_URL, body.as_slice())]);
        let loader = ForgeLikeLoader::neoforge(&f, &PanicRunner);
        let vs = loader.list_versions("1.20.4").unwrap();
        let got: Vec<(&str, bool)> = vs.iter().map(|v| (v.id.as_str(), v.stable)).collect();
        // 최신 우선, 다른 MC(21.1.1) 제외, beta는 불안정
        assert_eq!(got, [("20.4.300-beta", false), ("20.4.251", true), ("20.4.100", true)]);
    }

    fn forge_installer_fetch(installer: &[u8]) -> (MockFetch, String) {
        let url = format!("{FORGE_MAVEN}/1.20.1-47.4.10/forge-1.20.1-47.4.10-installer.jar");
        let sha = sha1_hex(installer);
        let mut f = MockFetch::with(&[]);
        f.responses.insert(url.clone(), installer.to_vec());
        f.responses.insert(format!("{url}.sha1"), sha.into_bytes());
        (f, url)
    }

    #[test]
    fn install_stages_workdir_runs_installer_and_harvests_to_cache() {
        let cache = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        // 함정 2 (§8.1): 공유 캐시의 바닐라 client.jar를 작업 디렉토리에 사전 스테이징
        fs::create_dir_all(cache.path().join("versions/1.20.1")).unwrap();
        fs::write(cache.path().join("versions/1.20.1/1.20.1.jar"), b"CLIENT").unwrap();

        let (f, _) = forge_installer_fetch(b"INSTALLER BYTES");
        let runner = FakeInstaller {
            version_id: "1.20.1-forge-47.4.10",
            exit_ok: true,
            called: AtomicBool::new(false),
        };
        let loader = ForgeLikeLoader::forge(&f, &runner);
        let ctx = InstallContext { shared_cache_root: cache.path(), work_dir: work.path() };
        let profile = loader.install("1.20.1", "47.4.10", &ctx).unwrap();

        assert!(runner.called.load(Ordering::SeqCst));
        let expected = cache
            .path()
            .join("versions/1.20.1-forge-47.4.10/1.20.1-forge-47.4.10.json");
        assert_eq!(profile.version_json_path, expected);
        assert!(expected.is_file(), "version JSON 수확");
        assert_eq!(
            fs::read(cache.path().join("libraries/net/minecraftforge/forge/forge-universal.jar"))
                .unwrap(),
            b"LIB",
            "libraries 수확"
        );
        assert_eq!(
            fs::read(work.path().join("versions/1.20.1/1.20.1.jar")).unwrap(),
            b"CLIENT",
            "client.jar 사전 스테이징 (재다운로드 방지)"
        );
    }

    #[test]
    fn install_reuses_cached_version_json_without_network_or_installer() {
        let cache = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let vdir = cache.path().join("versions/neoforge-20.4.251");
        fs::create_dir_all(&vdir).unwrap();
        fs::write(vdir.join("neoforge-20.4.251.json"), b"{}").unwrap();

        let f = MockFetch::with(&[]); // 네트워크 응답 없음 — 접근하면 실패
        let loader = ForgeLikeLoader::neoforge(&f, &PanicRunner);
        let ctx = InstallContext { shared_cache_root: cache.path(), work_dir: work.path() };
        let profile = loader.install("1.20.4", "20.4.251", &ctx).unwrap();
        assert_eq!(profile.version_json_path, vdir.join("neoforge-20.4.251.json"));
        assert_eq!(f.call_count(), 0, "불변 캐시 재사용 — 네트워크 무접촉");
    }

    #[test]
    fn install_surfaces_installer_stdout_on_failure() {
        let cache = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let (f, _) = forge_installer_fetch(b"INSTALLER BYTES");
        let runner = FakeInstaller {
            version_id: "1.20.1-forge-47.4.10",
            exit_ok: false,
            called: AtomicBool::new(false),
        };
        let loader = ForgeLikeLoader::forge(&f, &runner);
        let ctx = InstallContext { shared_cache_root: cache.path(), work_dir: work.path() };
        let err = loader.install("1.20.1", "47.4.10", &ctx).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("error during installation"), "E-LD-01: 인스톨러 로그 첨부, got {msg}");
        assert!(
            !cache.path().join("versions/1.20.1-forge-47.4.10").exists(),
            "실패 시 캐시 미오염"
        );
    }

    #[test]
    fn harvest_never_overwrites_existing_cache_paths() {
        let cache = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let lib = cache.path().join("libraries/net/minecraftforge/forge");
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("forge-universal.jar"), b"ORIGINAL").unwrap();

        let (f, _) = forge_installer_fetch(b"INSTALLER BYTES");
        let runner = FakeInstaller {
            version_id: "1.20.1-forge-47.4.10",
            exit_ok: true,
            called: AtomicBool::new(false),
        };
        let loader = ForgeLikeLoader::forge(&f, &runner);
        let ctx = InstallContext { shared_cache_root: cache.path(), work_dir: work.path() };
        loader.install("1.20.1", "47.4.10", &ctx).unwrap();
        assert_eq!(
            fs::read(lib.join("forge-universal.jar")).unwrap(),
            b"ORIGINAL",
            "공유 캐시 불변 — 덮어쓰기 금지 (PRD §7.1)"
        );
    }

    /// M3 게이트 일부(mac): 실네트워크로 Forge + NeoForge headless 설치 재현 (스파이크 §결론 재검).
    /// 실행: cargo test -p aqua-launcher -- --ignored m3_gate --nocapture
    #[test]
    #[ignore = "실네트워크 + JVM 설치 실행 — M3 게이트 수동 검증용"]
    fn m3_gate_real_forge_and_neoforge_headless_install_on_mac() {
        use crate::launch::install::prepare_version;
        use crate::launch::java::AdoptiumProvider;
        use crate::launch::rules::RuleContext;
        use crate::launch::JavaRuntimeProvider;
        use crate::net::HttpFetcher;

        let fetch = HttpFetcher::new().unwrap();
        let base = std::env::temp_dir().join("aqua-m3-gate");
        let cache = base.join("cache");

        for (mc, family) in [("1.20.1", "forge"), ("1.20.4", "neoforge")] {
            // 바닐라 선준비 (client.jar 사전 스테이징 + java_major)
            let ctx = RuleContext::new("osx", "arm64");
            let prepared =
                prepare_version(&fetch, &cache, mc, None, &ctx, &mut |_, _| {}).unwrap();
            let java = AdoptiumProvider {
                fetch: &fetch,
                cache_root: cache.clone(),
                os: "mac".into(),
                arch: "aarch64".into(),
            }
            .resolve(prepared.merged.java_major)
            .unwrap();
            let runner = JavaInstallerRunner { java };
            let loader = if family == "forge" {
                ForgeLikeLoader::forge(&fetch, &runner)
            } else {
                ForgeLikeLoader::neoforge(&fetch, &runner)
            };
            let vs = loader.list_versions(mc).unwrap();
            let version = vs.iter().find(|v| v.stable).or(vs.first()).unwrap().id.clone();
            let work = base.join(format!("work-{}-{version}", loader.id()));
            let ictx = InstallContext { shared_cache_root: &cache, work_dir: &work };
            let t0 = std::time::Instant::now();
            let profile = loader.install(mc, &version, &ictx).expect("headless install");
            eprintln!(
                "[m3-gate] {} {version} ({mc}) installed in {:?} → {}",
                loader.id(), t0.elapsed(), profile.version_json_path.display()
            );
            assert!(profile.version_json_path.is_file());
            let _ = fs::remove_dir_all(&work);
        }
    }

    #[test]
    fn install_rejects_corrupt_installer_download() {
        let cache = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let url = format!("{FORGE_MAVEN}/1.20.1-47.4.10/forge-1.20.1-47.4.10-installer.jar");
        let mut f = MockFetch::with(&[]);
        f.responses.insert(url.clone(), b"TAMPERED".to_vec());
        f.responses.insert(format!("{url}.sha1"), sha1_hex(b"INSTALLER BYTES").into_bytes());
        let loader = ForgeLikeLoader::forge(&f, &PanicRunner);
        let ctx = InstallContext { shared_cache_root: cache.path(), work_dir: work.path() };
        assert!(loader.install("1.20.1", "47.4.10", &ctx).is_err(), "해시 불일치 → 실행 금지");
    }
}
