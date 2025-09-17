use std::{
    env,
    io::{self, Cursor},
    path::Path,
    process::{ExitStatus, Stdio},
};

use crate::{
    build::utils,
    config::{Config, LuaVersion},
    hash::HasIntegrity,
    operations::{self, UnpackError},
    progress::{Progress, ProgressBar},
};
use bon::Builder;
use git2::{build::RepoBuilder, FetchOptions};
use ssri::Integrity;
use target_lexicon::Triple;
use tempfile::tempdir;
use thiserror::Error;
use tokio::{fs, process::Command};
use url::Url;

const LUA51_VERSION: &str = "5.1.5";
const LUA51_HASH: &str = "sha256-JkD8VqeV8p0o7xXhPDSkfiI5YLAkDoywqC2bBzhpUzM=";
const LUA52_VERSION: &str = "5.2.4";
const LUA52_HASH: &str = "sha256-ueLkqtZ4mztjoFbUQveznw7Pyjrg8fwK5OlhRAG2n0s=";
const LUA53_VERSION: &str = "5.3.6";
const LUA53_HASH: &str = "sha256-/F/Wm7hzYyPwJmcrG3I12mE9cXfnJViJOgvc0yBGbWA=";
const LUA54_VERSION: &str = "5.4.8";
const LUA54_HASH: &str = "sha256-TxjdrhVOeT5G7qtyfFnvHAwMK3ROe5QhlxDXb1MGKa4=";
// XXX: there's no tag with lua 5.2 compatibility, so we have to use the v2.1 branch for now
// this is unstable and might break the build.
const LUAJIT_MM_VERSION: &str = "2.1";

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct BuildLua<'a> {
    lua_version: &'a LuaVersion,
    install_dir: &'a Path,
    config: &'a Config,
    progress: &'a Progress<ProgressBar>,
}

#[derive(Debug, Error)]
pub enum BuildLuaError {
    #[error(transparent)]
    Request(#[from] reqwest::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Unpack(#[from] UnpackError),
    #[error(transparent)]
    Git(#[from] git2::Error),
    #[error(transparent)]
    CC(#[from] cc::Error),
    #[error("failed to find cl.exe")]
    ClNotFound,
    #[error("failed to find LINK.exe")]
    LinkNotFound,
    #[error("source integrity mismatch.\nExpected: {expected},\nbut got: {actual}")]
    SourceIntegrityMismatch {
        expected: Integrity,
        actual: Integrity,
    },
    #[error("{name} failed.\n\n{status}\n\nstdout:\n{stdout}\n\nstderr:\n{stderr}")]
    CommandFailure {
        name: String,
        status: ExitStatus,
        stdout: String,
        stderr: String,
    },
}

impl<State: build_lua_builder::State + build_lua_builder::IsComplete> BuildLuaBuilder<'_, State> {
    pub async fn build(self) -> Result<(), BuildLuaError> {
        let args = self._build();
        let lua_version = args.lua_version;
        match lua_version {
            LuaVersion::Lua51 | LuaVersion::Lua52 | LuaVersion::Lua53 | LuaVersion::Lua54 => {
                do_build_lua(args).await
            }
            LuaVersion::LuaJIT | LuaVersion::LuaJIT52 => do_build_luajit(args).await,
        }
    }
}

async fn do_build_luajit(args: BuildLua<'_>) -> Result<(), BuildLuaError> {
    let progress = args.progress;

    let build_dir = tempdir().expect("failed to create lua_installation temp directory");
    // XXX luajit.org responds with an invalid content-type, so we'll use the github mirror for now.
    // let luajit_url = "https://luajit.org/git/luajit.git";
    let luajit_url = "https://github.com/LuaJIT/LuaJIT.git";
    progress.map(|p| p.set_message(format!("🦠 Cloning {luajit_url}")));
    {
        // We create a new scope because we have to drop fetch_options before the await
        let mut fetch_options = FetchOptions::new();
        fetch_options.update_fetchhead(false);
        let mut repo_builder = RepoBuilder::new();
        repo_builder.fetch_options(fetch_options);
        let repo = repo_builder.clone(luajit_url, build_dir.path())?;
        let (object, _) = repo.revparse_ext(&format!("v{LUAJIT_MM_VERSION}"))?;
        repo.checkout_tree(&object, None)?;
    }
    if cfg!(target_env = "msvc") {
        do_build_luajit_msvc(args, build_dir.path()).await
    } else {
        do_build_luajit_unix(args, build_dir.path()).await
    }
}

async fn do_build_luajit_unix(args: BuildLua<'_>, build_dir: &Path) -> Result<(), BuildLuaError> {
    let lua_version = args.lua_version;
    let config = args.config;
    let install_dir = args.install_dir;
    let progress = args.progress;
    progress.map(|p| p.set_message(format!("🛠️ Building Luajit {LUAJIT_MM_VERSION}")));

    let host = Triple::host();

    let mut cc = cc::Build::new();
    cc.cargo_output(false)
        .cargo_metadata(false)
        .cargo_warnings(false)
        .warnings(config.verbose())
        .opt_level(3)
        .host(&host.to_string())
        .target(&host.to_string());
    let compiler = cc.try_get_compiler()?;
    let compiler_path = compiler.path().to_str().unwrap();
    let mut make_cmd = Command::new(config.make_cmd());
    make_cmd.current_dir(build_dir.join("src"));
    make_cmd.arg("-e");
    make_cmd.stdout(Stdio::piped());
    make_cmd.stderr(Stdio::piped());
    let target = host.to_string();
    match target.as_str() {
        "x86_64-apple-darwin" if env::var_os("MACOSX_DEPLOYMENT_TARGET").is_none() => {
            make_cmd.env("MACOSX_DEPLOYMENT_TARGET", "10.11");
        }
        "aarch64-apple-darwin" if env::var_os("MACOSX_DEPLOYMENT_TARGET").is_none() => {
            make_cmd.env("MACOSX_DEPLOYMENT_TARGET", "11.0");
        }
        _ if target.contains("linux") => {
            make_cmd.env("TARGET_SYS", "Linux");
        }
        _ => {}
    }
    let compiler_path =
        which::which(compiler_path).unwrap_or_else(|_| panic!("cannot find {compiler_path}"));
    let compiler_path = compiler_path.to_str().unwrap();
    let compiler_args = compiler.cflags_env();
    let compiler_args = compiler_args.to_str().unwrap();
    if env::var_os("STATIC_CC").is_none() {
        make_cmd.env("STATIC_CC", format!("{compiler_path} {compiler_args}"));
    }
    if env::var_os("TARGET_LD").is_none() {
        make_cmd.env("TARGET_LD", format!("{compiler_path} {compiler_args}"));
    }
    let mut xcflags = vec!["-fPIC"];
    if lua_version == &LuaVersion::LuaJIT52 {
        xcflags.push("-DLUAJIT_ENABLE_LUA52COMPAT");
    }
    if cfg!(debug_assertions) {
        xcflags.push("-DLUA_USE_ASSERT");
        xcflags.push("-DLUA_USE_APICHECK");
    }
    make_cmd.env("BUILDMODE", "static");
    make_cmd.env("XCFLAGS", xcflags.join(" "));

    match make_cmd.output().await {
        Ok(output) if output.status.success() => utils::log_command_output(&output, config),
        Ok(output) => {
            return Err(BuildLuaError::CommandFailure {
                name: "build".into(),
                status: output.status,
                stdout: String::from_utf8_lossy(&output.stdout).into(),
                stderr: String::from_utf8_lossy(&output.stderr).into(),
            });
        }
        Err(err) => return Err(BuildLuaError::Io(err)),
    };

    progress.map(|p| p.set_message(format!("💻 Installing Luajit {LUAJIT_MM_VERSION}")));

    match Command::new(config.make_cmd())
        .current_dir(build_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .arg("install")
        .arg(format!("PREFIX={}", install_dir.display()))
        .output()
        .await
    {
        Ok(output) if output.status.success() => utils::log_command_output(&output, config),
        Ok(output) => {
            return Err(BuildLuaError::CommandFailure {
                name: "install".into(),
                status: output.status,
                stdout: String::from_utf8_lossy(&output.stdout).into(),
                stderr: String::from_utf8_lossy(&output.stderr).into(),
            });
        }
        Err(err) => return Err(BuildLuaError::Io(err)),
    };
    move_luajit_includes(install_dir).await?;
    Ok(())
}

/// luajit installs the includes to a subdirectory.
/// For consistency, we want them in the `include` directory
async fn move_luajit_includes(install_dir: &Path) -> io::Result<()> {
    let include_dir = install_dir.join("include");
    let include_subdir = include_dir.join(format!("luajit-{LUAJIT_MM_VERSION}"));
    if !include_subdir.is_dir() {
        return Ok(());
    }
    let mut dir = fs::read_dir(&include_subdir).await?;
    while let Some(entry) = dir.next_entry().await? {
        let file_name = entry.file_name();
        let src_path = entry.path();
        let dest_path = include_dir.join(&file_name);
        fs::copy(&src_path, &dest_path).await?;
    }
    fs::remove_dir_all(&include_subdir).await?;
    Ok(())
}

async fn do_build_luajit_msvc(args: BuildLua<'_>, build_dir: &Path) -> Result<(), BuildLuaError> {
    let lua_version = args.lua_version;
    let config = args.config;
    let install_dir = args.install_dir;
    let lib_dir = install_dir.join("lib");
    fs::create_dir_all(&lib_dir).await?;
    let include_dir = install_dir.join("include");
    fs::create_dir_all(&include_dir).await?;
    let bin_dir = install_dir.join("bin");
    fs::create_dir_all(&bin_dir).await?;

    let progress = args.progress;

    progress.map(|p| p.set_message(format!("🛠️ Building Luajit {LUAJIT_MM_VERSION}")));

    let src_dir = build_dir.join("src");
    let mut msvcbuild = Command::new(src_dir.join("msvcbuild.bat"));
    msvcbuild.current_dir(&src_dir);
    if lua_version == &LuaVersion::LuaJIT52 {
        msvcbuild.arg("lua52compat");
    }
    msvcbuild.arg("static");
    let host = Triple::host();
    let target = host.to_string();
    let cl = cc::windows_registry::find_tool(&target, "cl.exe").ok_or(BuildLuaError::ClNotFound)?;
    for (k, v) in cl.env() {
        msvcbuild.env(k, v);
    }
    fs::create_dir_all(&install_dir).await?;
    match msvcbuild.output().await {
        Ok(output) if output.status.success() => utils::log_command_output(&output, config),
        Ok(output) => {
            return Err(BuildLuaError::CommandFailure {
                name: "build".into(),
                status: output.status,
                stdout: String::from_utf8_lossy(&output.stdout).into(),
                stderr: String::from_utf8_lossy(&output.stderr).into(),
            });
        }
        Err(err) => return Err(BuildLuaError::Io(err)),
    };

    progress.map(|p| p.set_message(format!("💻 Installing Luajit {LUAJIT_MM_VERSION}")));
    copy_includes(&src_dir, &include_dir).await?;
    fs::copy(src_dir.join("lua51.lib"), lib_dir.join("luajit.lib")).await?;
    fs::copy(src_dir.join("luajit.exe"), bin_dir.join("luajit.exe")).await?;
    Ok(())
}

async fn do_build_lua(args: BuildLua<'_>) -> Result<(), BuildLuaError> {
    let lua_version = args.lua_version;
    let progress = args.progress;

    let build_dir = tempdir().expect("failed to create lua_installation temp directory");

    let (source_integrity, pkg_version): (Integrity, &str) = match lua_version {
        LuaVersion::Lua51 => (LUA51_HASH.parse().unwrap(), LUA51_VERSION),
        LuaVersion::Lua52 => (LUA52_HASH.parse().unwrap(), LUA52_VERSION),
        LuaVersion::Lua53 => (LUA53_HASH.parse().unwrap(), LUA53_VERSION),
        LuaVersion::Lua54 => (LUA54_HASH.parse().unwrap(), LUA54_VERSION),
        LuaVersion::LuaJIT | LuaVersion::LuaJIT52 => unreachable!(),
    };

    let file_name = format!("lua-{pkg_version}.tar.gz");

    let source_url: Url = format!("https://www.lua.org/ftp/{file_name}")
        .parse()
        .unwrap();

    progress.map(|p| p.set_message(format!("📥 Downloading {}", &source_url)));

    let response = reqwest::Client::new()
        .get(source_url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    let hash = response.hash()?;

    if hash.matches(&source_integrity).is_none() {
        return Err(BuildLuaError::SourceIntegrityMismatch {
            expected: source_integrity,
            actual: hash,
        });
    }

    let cursor = Cursor::new(response);
    let mime_type = infer::get(cursor.get_ref()).map(|file_type| file_type.mime_type());
    operations::unpack::unpack(
        mime_type,
        cursor,
        true,
        file_name,
        build_dir.path(),
        progress,
    )
    .await?;

    if cfg!(target_env = "msvc") {
        do_build_lua_msvc(args, build_dir.path(), lua_version, pkg_version).await
    } else {
        do_build_lua_unix(args, build_dir.path(), lua_version, pkg_version).await
    }
}

async fn do_build_lua_unix(
    args: BuildLua<'_>,
    build_dir: &Path,
    lua_version: &LuaVersion,
    pkg_version: &str,
) -> Result<(), BuildLuaError> {
    let config = args.config;
    let progress = args.progress;
    let install_dir = args.install_dir;

    progress.map(|p| p.set_message(format!("🛠️ Building Lua {}", &pkg_version)));

    let build_target = if cfg!(target_os = "linux") && matches!(&lua_version, LuaVersion::Lua54) {
        "linux"
    } else {
        // For older lua versions, the "linux" target uses readline.
        "generic"
    };

    match Command::new(config.make_cmd())
        .current_dir(build_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .arg(build_target)
        .output()
        .await
    {
        Ok(output) if output.status.success() => utils::log_command_output(&output, config),
        Ok(output) => {
            return Err(BuildLuaError::CommandFailure {
                name: "build".into(),
                status: output.status,
                stdout: String::from_utf8_lossy(&output.stdout).into(),
                stderr: String::from_utf8_lossy(&output.stderr).into(),
            });
        }
        Err(err) => return Err(BuildLuaError::Io(err)),
    };

    progress.map(|p| p.set_message(format!("💻 Installing Lua {}", &pkg_version)));

    match Command::new(config.make_cmd())
        .current_dir(build_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .arg("install")
        .arg(format!("INSTALL_TOP={}", install_dir.display()))
        .output()
        .await
    {
        Ok(output) if output.status.success() => utils::log_command_output(&output, config),
        Ok(output) => {
            return Err(BuildLuaError::CommandFailure {
                name: "install".into(),
                status: output.status,
                stdout: String::from_utf8_lossy(&output.stdout).into(),
                stderr: String::from_utf8_lossy(&output.stderr).into(),
            });
        }
        Err(err) => return Err(BuildLuaError::Io(err)),
    };

    Ok(())
}

async fn do_build_lua_msvc(
    args: BuildLua<'_>,
    build_dir: &Path,
    lua_version: &LuaVersion,
    pkg_version: &str,
) -> Result<(), BuildLuaError> {
    let config = args.config;
    let progress = args.progress;
    let install_dir = args.install_dir;

    progress.map(|p| p.set_message(format!("🛠️ Building Lua {}", &pkg_version)));

    let lib_dir = install_dir.join("lib");
    fs::create_dir_all(&lib_dir).await?;
    let include_dir = install_dir.join("include");
    fs::create_dir_all(&include_dir).await?;
    let bin_dir = install_dir.join("bin");
    fs::create_dir_all(&bin_dir).await?;

    let src_dir = build_dir.join("src");

    let lib_name = match lua_version {
        LuaVersion::Lua51 => "lua5.1",
        LuaVersion::Lua52 => "lua5.2",
        LuaVersion::Lua53 => "lua5.3",
        LuaVersion::Lua54 => "lua5.4",
        LuaVersion::LuaJIT | LuaVersion::LuaJIT52 => unreachable!(),
    };
    let host = Triple::host();
    let mut cc = cc::Build::new();
    cc.cargo_output(false)
        .cargo_metadata(false)
        .cargo_warnings(false)
        .warnings(config.verbose())
        .opt_level(3)
        .host(&host.to_string())
        .target(&host.to_string());

    cc.define("LUA_USE_WINDOWS", None);

    let mut lib_c_files = Vec::new();
    let mut read_dir = fs::read_dir(&src_dir).await?;
    while let Some(entry) = read_dir.next_entry().await? {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "c")
            && path
                .with_extension("")
                .file_name()
                .is_some_and(|name| name != "lua" && name != "luac")
        {
            lib_c_files.push(path);
        }
    }
    cc.include(&src_dir)
        .files(lib_c_files)
        .out_dir(&lib_dir)
        .try_compile(lib_name)?;

    let bin_objects = cc
        .include(&src_dir)
        .file(src_dir.join("lua.c"))
        .file(src_dir.join("luac.c"))
        .out_dir(&src_dir)
        .try_compile_intermediates()?;

    progress.map(|p| p.set_message(format!("💻 Installing Lua {}", &pkg_version)));

    let target = host.to_string();
    let link =
        cc::windows_registry::find_tool(&target, "link.exe").ok_or(BuildLuaError::LinkNotFound)?;

    for name in ["lua", "luac"] {
        let bin = bin_dir.join(format!("{name}.exe"));
        let objects = bin_objects.iter().filter(|file| {
            file.with_extension("").file_name().is_some_and(|fname| {
                fname
                    .to_string_lossy()
                    .to_string()
                    .ends_with(&format!("-{name}"))
            })
        });
        match Command::new(link.path())
            .arg(format!("/OUT:{}", bin.display()))
            .args(objects)
            .arg(format!("{}.lib", lib_dir.join(lib_name).display()))
            .output()
            .await
        {
            Ok(output) if output.status.success() => utils::log_command_output(&output, config),
            Ok(output) => {
                return Err(BuildLuaError::CommandFailure {
                    name: format!("install {name}.exe"),
                    status: output.status,
                    stdout: String::from_utf8_lossy(&output.stdout).into(),
                    stderr: String::from_utf8_lossy(&output.stderr).into(),
                });
            }
            Err(err) => return Err(BuildLuaError::Io(err)),
        };
    }

    copy_includes(&src_dir, &include_dir).await?;

    Ok(())
}

async fn copy_includes(src_dir: &Path, include_dir: &Path) -> Result<(), io::Error> {
    for f in &[
        "lauxlib.h",
        "lua.h",
        "luaconf.h",
        "luajit.h",
        "lualib.h",
        "lua.hpp",
    ] {
        let src_file = src_dir.join(f);
        if src_file.is_file() {
            fs::copy(src_file, include_dir.join(f)).await?;
        }
    }
    Ok(())
}
