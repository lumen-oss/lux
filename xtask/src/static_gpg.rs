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
const SRI: &str = "sha256-eoVBPyvDVPT4qoMrcYrxIuSJZeng65AS7mWcE8Y4XJM=";
const ASSUAN_VERSION: &str = "2.5.7";
const ASSUAN_SRI: &str = "sha256-AQMIH/wng4ouUEeRU8oQXoc9PWXYqVkygunJTH5q+3Y=";
const GPGME_VERSION: &str = "1.24.3";
const GPGME_SRI: &str = "sha256-v8F/W9GxeMhkn92RiVbSdwgPM98Aai3ECs3s3OaMUN0=";

struct Dependency {
    name: &'static str,
    library: &'static str,
    version: &'static str,
    sha256: &'static str,
    configure_args: &'static [&'static str],
}

const LIBGPG_ERROR: Dependency = Dependency {
    name: "libgpg-error",
    library: "gpg-error",
    version: VERSION,
    sha256: SRI,
    configure_args: &[
        "--disable-nls",
        "--disable-doc",
        "--disable-tests",
        "--disable-languages",
        "--enable-install-gpg-error-config",
    ],
};

const LIBASSUAN: Dependency = Dependency {
    name: "libassuan",
    library: "assuan",
    version: ASSUAN_VERSION,
    sha256: ASSUAN_SRI,
    configure_args: &["--disable-doc"],
};

const GPGME: Dependency = Dependency {
    name: "gpgme",
    library: "gpgme",
    version: GPGME_VERSION,
    sha256: GPGME_SRI,
    configure_args: &["--disable-languages"],
};

fn dependencies() -> &'static [Dependency] {
    if cfg!(target_os = "macos") {
        &[LIBGPG_ERROR]
    } else {
        &[LIBGPG_ERROR, LIBASSUAN, GPGME]
    }
}

fn is_supported() -> bool {
    !cfg!(target_os = "windows")
}

fn static_linking_requested() -> bool {
    env::var("SYSTEM_DEPS_LINK").is_ok_and(|value| value == "static")
}

pub fn is_required() -> bool {
    is_supported() && static_linking_requested()
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

fn path_with_prefix(prefix: &Path) -> Result<OsString, DynError> {
    let mut paths = vec![prefix.join("bin")];
    if let Some(existing) = env::var_os("PATH") {
        paths.extend(env::split_paths(&existing));
    }
    Ok(env::join_paths(paths)?)
}

pub fn ensure_installed() -> Result<PathBuf, DynError> {
    let prefix = prefix();
    for dependency in dependencies() {
        ensure_dependency(dependency, &prefix)?;
    }
    Ok(prefix)
}

fn ensure_dependency(dependency: &Dependency, prefix: &Path) -> Result<(), DynError> {
    if is_installed(dependency, prefix) {
        return Ok(());
    }
    let cache_dir = cache_dir();
    fs::create_dir_all(&cache_dir)?;
    let tarball = ensure_tarball(dependency, &cache_dir)?;
    let src_dir = cache_dir.join(format!("src-{}", dependency.name));
    extract_sources(dependency, &tarball, &src_dir)?;
    build(dependency, &src_dir, prefix)?;
    install(&src_dir)
}

fn is_installed(dependency: &Dependency, prefix: &Path) -> bool {
    prefix
        .join("lib")
        .join(format!("lib{}.a", dependency.library))
        .is_file()
}

fn ensure_tarball(dependency: &Dependency, cache_dir: &Path) -> Result<PathBuf, DynError> {
    let tarball = cache_dir.join(format!(
        "{}-{}.tar.bz2",
        dependency.name, dependency.version
    ));
    if verify_integrity(&tarball, dependency.sha256, dependency.name).is_err() {
        let _ = fs::remove_file(&tarball);
        download(dependency, &tarball)?;
        verify_integrity(&tarball, dependency.sha256, dependency.name)?;
    }
    Ok(tarball)
}

fn extract_sources(
    dependency: &Dependency,
    tarball: &Path,
    src_dir: &Path,
) -> Result<(), DynError> {
    let extract_dir = src_dir.with_file_name(format!("extract-{}", dependency.name));
    if extract_dir.exists() {
        fs::remove_dir_all(&extract_dir)?;
    }
    if src_dir.exists() {
        fs::remove_dir_all(src_dir)?;
    }
    fs::create_dir_all(&extract_dir)?;
    let file = fs::File::open(tarball)?;
    Archive::new(BzDecoder::new(file)).unpack(&extract_dir)?;
    let top_level = extract_dir.join(format!("{}-{}", dependency.name, dependency.version));
    if !top_level.is_dir() {
        Err(format!(
            "{} archive does not contain {}-{}",
            dependency.name, dependency.name, dependency.version
        ))?;
    }
    fs::rename(&top_level, src_dir)?;
    fs::remove_dir_all(&extract_dir)?;
    Ok(())
}

fn build(dependency: &Dependency, src_dir: &Path, prefix: &Path) -> Result<(), DynError> {
    let jobs = std::thread::available_parallelism()
        .map(|jobs| jobs.get())
        .unwrap_or(1)
        .to_string();
    run(Command::new(src_dir.join("configure"))
        .current_dir(src_dir)
        .arg("--enable-static")
        .arg("--disable-shared")
        .arg("--with-pic")
        .arg("--prefix")
        .arg(prefix)
        .args(dependency.configure_args)
        .env("PATH", path_with_prefix(prefix)?)
        .env("PKG_CONFIG_PATH", pkg_config_path(prefix)?))?;
    run(Command::new("make")
        .current_dir(src_dir)
        .arg("-j")
        .arg(&jobs))
}

fn install(src_dir: &Path) -> Result<(), DynError> {
    run(Command::new("make").current_dir(src_dir).arg("install"))
}

fn download(dependency: &Dependency, tarball: &Path) -> Result<(), DynError> {
    println!("downloading {} {}", dependency.name, dependency.version);
    let url = format!(
        "https://gnupg.org/ftp/gcrypt/{}/{}-{}.tar.bz2",
        dependency.name, dependency.name, dependency.version
    );
    let mut response = reqwest::blocking::get(url)?.error_for_status()?;
    let mut file = fs::File::create(tarball)?;
    io::copy(&mut response, &mut file)?;
    Ok(())
}

fn verify_integrity(tarball: &Path, expected: &str, name: &str) -> Result<(), DynError> {
    let expected = expected.parse::<Integrity>()?;
    let mut file = fs::File::open(tarball)?;
    let mut hasher = IntegrityOpts::new().algorithm(Algorithm::Sha256);
    io::copy(&mut file, &mut hasher)?;
    let actual = hasher.result();
    if expected.matches(&actual).is_none() {
        Err(format!(
            "{name} checksum mismatch: expected {expected}, got {actual}"
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
    crate::project_root().join("target/static-gpg")
}

fn prefix() -> PathBuf {
    cache_dir().join("prefix")
}
