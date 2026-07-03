//! Java 자동 관리 — PRD §8.4. Adoptium API에서 플랫폼별 JRE 확보.
//! 실패는 E-JV-01 (재시도 / 수동 경로 지정 안내).
use super::JavaRuntimeProvider;
use crate::error::AppError;
use crate::net::download::{download_verified, ExpectedHash};
use crate::net::Fetch;
use serde::Deserialize;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Adoptium assets API 응답 (필요 필드만)
#[derive(Debug, Deserialize)]
pub struct AdoptiumAsset {
    pub release_name: String,
    pub binary: AdoptiumBinary,
}

#[derive(Debug, Deserialize)]
pub struct AdoptiumBinary {
    pub package: AdoptiumPackage,
}

#[derive(Debug, Deserialize)]
pub struct AdoptiumPackage {
    pub name: String,
    pub link: String,
    /// sha256 hex
    pub checksum: String,
}

pub fn adoptium_api_url(major: u32, os: &str, arch: &str) -> String {
    format!(
        "https://api.adoptium.net/v3/assets/latest/{major}/hotspot?architecture={arch}&image_type=jre&os={os}&vendor=eclipse"
    )
}

/// dir 아래에서 `bin/java`(또는 java.exe)를 깊이 제한 탐색 (mac은 Contents/Home/bin/java).
pub fn find_java_exe(dir: &Path) -> Option<PathBuf> {
    fn walk(dir: &Path, depth: usize) -> Option<PathBuf> {
        if depth > 5 {
            return None;
        }
        let direct = dir.join("bin/java");
        if direct.is_file() {
            return Some(direct);
        }
        let direct_exe = dir.join("bin/java.exe");
        if direct_exe.is_file() {
            return Some(direct_exe);
        }
        for entry in fs::read_dir(dir).ok()? {
            let p = entry.ok()?.path();
            if p.is_dir() {
                if let Some(found) = walk(&p, depth + 1) {
                    return Some(found);
                }
            }
        }
        None
    }
    walk(dir, 0)
}

/// tar.gz 추출 — tar 크레이트의 unpack은 경로 탈출을 거부한다.
pub fn extract_tar_gz(archive: &Path, dest: &Path) -> io::Result<()> {
    let file = fs::File::open(archive)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);
    fs::create_dir_all(dest)?;
    tar.unpack(dest)
}

/// zip 추출 — Zip Slip 방지: enclosed_name만 허용 (PRD §11).
pub fn extract_zip(archive: &Path, dest: &Path) -> io::Result<()> {
    let file = fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(io::Error::other)?;
    fs::create_dir_all(dest)?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(io::Error::other)?;
        let Some(rel) = entry.enclosed_name() else {
            return Err(io::Error::other(format!(
                "zip entry escapes destination: {}",
                entry.name()
            )));
        };
        let out_path = dest.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out = fs::File::create(&out_path)?;
            io::copy(&mut entry, &mut out)?;
        }
    }
    Ok(())
}

fn extract_archive(archive: &Path, dest: &Path) -> io::Result<()> {
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        extract_tar_gz(archive, dest)
    } else if name.ends_with(".zip") {
        extract_zip(archive, dest)
    } else {
        Err(io::Error::other(format!("unsupported archive format: {name}")))
    }
}

/// Adoptium 기반 JRE 공급자 — 공유 캐시 `java/<major>-<os>-<arch>/`에 설치 (불변 경로).
pub struct AdoptiumProvider<'a> {
    pub fetch: &'a dyn Fetch,
    pub cache_root: PathBuf,
    /// "mac" | "windows"
    pub os: String,
    /// "aarch64" | "x64"
    pub arch: String,
}

impl AdoptiumProvider<'_> {
    fn install_dir(&self, major: u32) -> PathBuf {
        self.cache_root.join(format!("java/{major}-{}-{}", self.os, self.arch))
    }

    fn err(e: impl std::fmt::Display) -> AppError {
        AppError::JavaSetup(e.to_string())
    }
}

impl JavaRuntimeProvider for AdoptiumProvider<'_> {
    fn resolve(&self, major: u32) -> Result<PathBuf, AppError> {
        let install_dir = self.install_dir(major);
        if let Some(exe) = find_java_exe(&install_dir) {
            return Ok(exe);
        }
        let url = adoptium_api_url(major, &self.os, &self.arch);
        let raw = self.fetch.get_bytes(&url).map_err(Self::err)?;
        let assets: Vec<AdoptiumAsset> = serde_json::from_slice(&raw).map_err(Self::err)?;
        let asset = assets.first().ok_or_else(|| {
            Self::err(format!("no adoptium jre for major {major} on {}-{}", self.os, self.arch))
        })?;

        let dl_dir = self.cache_root.join("java/downloads");
        let archive = dl_dir.join(&asset.binary.package.name);
        download_verified(
            self.fetch,
            &archive,
            &asset.binary.package.link,
            ExpectedHash::Sha256(&asset.binary.package.checksum),
        )
        .map_err(Self::err)?;

        extract_archive(&archive, &install_dir).map_err(Self::err)?;
        let _ = fs::remove_file(&archive);
        find_java_exe(&install_dir)
            .ok_or_else(|| Self::err("archive extracted but bin/java not found"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::test_support::MockFetch;
    use sha2::{Digest, Sha256};
    use std::io::Write;

    fn sha256_hex(b: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(b);
        h.finalize().iter().map(|x| format!("{x:02x}")).collect()
    }

    /// jdk-21-jre/Contents/Home/bin/java 하나가 든 tar.gz (mac 레이아웃)
    fn fake_jre_targz() -> Vec<u8> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        {
            let mut tar = tar::Builder::new(&mut gz);
            let body = b"#!/bin/sh\necho fake java\n";
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(
                &mut header,
                "jdk-21.0.10+7-jre/Contents/Home/bin/java",
                body.as_slice(),
            )
            .unwrap();
            tar.finish().unwrap();
        }
        gz.finish().unwrap()
    }

    #[test]
    fn extract_tar_gz_and_find_java() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("jre.tar.gz");
        fs::write(&archive, fake_jre_targz()).unwrap();
        let dest = dir.path().join("out");
        extract_tar_gz(&archive, &dest).unwrap();
        let exe = find_java_exe(&dest).unwrap();
        assert!(exe.ends_with("Contents/Home/bin/java"));
    }

    #[test]
    fn zip_slip_entry_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("evil.zip");
        {
            let file = fs::File::create(&archive).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("../evil.txt", opts).unwrap();
            zip.write_all(b"pwn").unwrap();
            zip.finish().unwrap();
        }
        let dest = dir.path().join("out");
        assert!(extract_zip(&archive, &dest).is_err());
        assert!(!dir.path().join("evil.txt").exists());
    }

    #[test]
    fn resolve_downloads_extracts_and_caches() {
        let dir = tempfile::tempdir().unwrap();
        let package = fake_jre_targz();
        let api_url = adoptium_api_url(17, "mac", "aarch64");
        let assets_json = format!(
            r#"[{{"release_name": "jdk-21.0.10+7",
                 "binary": {{"package": {{"name": "jre.tar.gz",
                   "link": "https://github.com/adoptium/temurin/releases/jre.tar.gz",
                   "checksum": "{sum}"}}}}}}]"#,
            sum = sha256_hex(&package)
        );
        let mut f = MockFetch::with(&[]);
        f.responses = [
            (api_url, assets_json.into_bytes()),
            (
                "https://github.com/adoptium/temurin/releases/jre.tar.gz".to_string(),
                package,
            ),
        ]
        .into_iter()
        .collect();

        let provider = AdoptiumProvider {
            fetch: &f,
            cache_root: dir.path().to_path_buf(),
            os: "mac".into(),
            arch: "aarch64".into(),
        };
        use crate::launch::JavaRuntimeProvider;
        let exe = provider.resolve(17).unwrap();
        assert!(exe.is_file());
        assert!(exe.starts_with(dir.path().join("java/17-mac-aarch64")));

        // 캐시 적중: 추가 네트워크 호출 없음
        let calls = f.call_count();
        let exe2 = provider.resolve(17).unwrap();
        assert_eq!(exe2, exe);
        assert_eq!(f.call_count(), calls);
    }

    #[test]
    fn parses_adoptium_assets() {
        let raw = br#"[{"release_name": "jdk-17.0.10+7",
            "binary": {"package": {"name": "x.tar.gz", "link": "https://x/x.tar.gz", "checksum": "abc"}}}]"#;
        let assets: Vec<AdoptiumAsset> = serde_json::from_slice(raw).unwrap();
        assert_eq!(assets[0].release_name, "jdk-17.0.10+7");
        assert_eq!(assets[0].binary.package.checksum, "abc");
    }
}
