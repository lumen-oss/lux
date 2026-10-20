use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

use bzip2::read::BzDecoder;
use ssri::{Algorithm, Integrity, IntegrityOpts};
use tar::Archive;

use crate::DynError;

const VERSION: &str = "1.61";
const SHA256: &str = "sha256-eoVBPyvDVPT4qoMrcYrxIuSJZeng65AS7mWcE8Y4XJM=";

pub fn is_required() -> bool {
    cfg!(target_os = "macos") && env::var("SYSTEM_DEPS_LINK").is_ok_and(|value| value == "static")
}

pub fn apply(command: &mut Command) -> Result<(), DynError> {
    if !is_required() {
        return Ok(());
    }
    let prefix = ensure_installed()?;
    command.env("PKG_CONFIG_PATH", pkg_config_path(&prefix)?);
    Ok(())
}

fn pkg_config_path(prefix: &Path) -> Result<OsString, DynError> {
    let mut paths = vec![prefix.join("lib/pkgconfig")];
    if let Some(existing) = env::var_os("PKG_CONFIG_PATH") {
        paths.extend(env::split_paths(&existing));
    }
    Ok(env::join_paths(paths)?)
}

pub fn ensure_installed() -> Result<PathBuf, DynError> {
    if !cfg!(target_os = "macos") {
        Err("build-static-libgpg-error is only supported on macOS")?;
    }

    let prefix = prefix();
    if prefix.join("lib/pkgconfig/gpg-error.pc").is_file() {
        return Ok(prefix);
    }

    let cache_dir = cache_dir();
    fs::create_dir_all(&cache_dir)?;
    let tarball = ensure_tarball(&cache_dir)?;
    let src_dir = cache_dir.join("src");
    extract_sources(&tarball, &src_dir)?;
    build(&src_dir, &prefix)?;
    install(&src_dir)?;
    Ok(prefix)
}

fn ensure_tarball(cache_dir: &Path) -> Result<PathBuf, DynError> {
    let tarball = cache_dir.join(format!("libgpg-error-{VERSION}.tar.bz2"));
    if verify_integrity(&tarball).is_err() {
        let _ = fs::remove_file(&tarball);
        download(&tarball)?;
        verify_integrity(&tarball)?;
    }
    Ok(tarball)
}

fn extract_sources(tarball: &Path, src_dir: &Path) -> Result<(), DynError> {
    let tarball = tarball.canonicalize()?;
    let cache_dir = cache_dir().canonicalize()?;
    if tarball.starts_with(cache_dir) {
        let extract_dir = src_dir.with_file_name("extract");
        if extract_dir.exists() {
            fs::remove_dir_all(&extract_dir)?;
        }
        if src_dir.exists() {
            fs::remove_dir_all(src_dir)?;
        }
        fs::create_dir_all(&extract_dir)?;
        let file = fs::File::open(&tarball)?;
        Archive::new(BzDecoder::new(file)).unpack(&extract_dir)?;
        let mut entries = fs::read_dir(&extract_dir)?;
        let top_level = entries.next().ok_or("libgpg-error archive is empty")??;
        fs::rename(top_level.path(), src_dir)?;
        fs::remove_dir_all(&extract_dir)?;
        Ok(())
    } else {
        Err("libgpg-error tarball is outside the cache directory")?
    }
}

fn build(src_dir: &Path, prefix: &Path) -> Result<(), DynError> {
    let jobs = std::thread::available_parallelism()
        .map(|jobs| jobs.get())
        .unwrap_or(1)
        .to_string();
    run(Command::new(src_dir.join("configure"))
        .current_dir(src_dir)
        .args([
            "--disable-nls",
            "--disable-doc",
            "--disable-tests",
            "--enable-static",
            "--disable-shared",
        ])
        .arg("--prefix")
        .arg(prefix))?;
    run(Command::new("make")
        .current_dir(src_dir)
        .arg("-j")
        .arg(&jobs))
}

fn install(src_dir: &Path) -> Result<(), DynError> {
    run(Command::new("make").current_dir(src_dir).arg("install"))
}

fn download(tarball: &Path) -> Result<(), DynError> {
    println!("downloading libgpg-error {VERSION}");
    let url = format!("https://gnupg.org/ftp/gcrypt/libgpg-error/libgpg-error-{VERSION}.tar.bz2");
    let mut response = reqwest::blocking::get(url)?.error_for_status()?;
    let mut file = fs::File::create(tarball)?;
    io::copy(&mut response, &mut file)?;
    Ok(())
}

fn verify_integrity(tarball: &Path) -> Result<(), DynError> {
    let expected = SHA256.parse::<Integrity>()?;
    let mut file = fs::File::open(tarball)?;
    let mut hasher = IntegrityOpts::new().algorithm(Algorithm::Sha256);
    io::copy(&mut file, &mut hasher)?;
    let actual = hasher.result();
    if expected.matches(&actual).is_none() {
        Err(format!(
            "libgpg-error checksum mismatch: expected {expected}, got {actual}"
        ))?;
    }
    Ok(())
}

fn run(command: &mut Command) -> Result<(), DynError> {
    let status = command.status()?;
    if !status.success() {
        Err(format!("command failed: {command:?}"))?;
    }
    Ok(())
}

fn cache_dir() -> PathBuf {
    crate::project_root().join("target/static-libgpg-error")
}

fn prefix() -> PathBuf {
    cache_dir().join("prefix")
}
