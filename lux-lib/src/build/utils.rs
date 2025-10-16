use crate::{
    config::Config,
    lua_installation::LuaInstallation,
    lua_rockspec::{DeploySpec, LuaModule, ModulePaths},
    path::{Paths, PathsError},
    tree::{RockLayout, Tree},
    variables::{self, Environment, VariableSubstitutionError},
};
use itertools::Itertools;
use mlua::{Lua, LuaSerdeExt};
use path_slash::PathExt;
use shlex::try_quote;
use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    process::{ExitStatus, Output, Stdio},
    string::FromUtf8Error,
};
use target_lexicon::Triple;
use thiserror::Error;
use tokio::process::Command;
use which::which;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use super::external_dependency::ExternalDependencyInfo;

/// Copies a lua source file to a specific destination. The destination is described by a
/// `module.path` syntax (equivalent to the syntax provided to Lua's `require()` function).
pub(crate) fn copy_lua_to_module_path(
    source: &PathBuf,
    target_module: &LuaModule,
    target_dir: &Path,
) -> io::Result<()> {
    let target = target_dir.join(target_module.to_lua_path());

    std::fs::create_dir_all(target.parent().unwrap())?;

    std::fs::copy(source, target)?;

    Ok(())
}

/// Get the files that Lux treats as project files
/// This respects ignore files and excludes hidden files and directories.
pub(crate) fn project_files(src: &PathBuf) -> Vec<PathBuf> {
    ignore::WalkBuilder::new(src)
        .follow_links(false)
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|ft| ft.is_file()))
        .map(|entry| entry.into_path())
        .collect_vec()
}

/// Recursively copy a directory.
/// This respects ignore files and excludes hidden files and directories.
pub(crate) async fn recursive_copy_dir(src: &PathBuf, dest: &Path) -> Result<(), io::Error> {
    if src.exists() {
        for file in project_files(src) {
            let relative_src_path: PathBuf =
                pathdiff::diff_paths(src.join(&file), src).expect("failed to copy directories!");
            let target = dest.join(relative_src_path);
            if let Some(parent) = target.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::copy(&file, target).await?;
        }
    }
    Ok(())
}

#[derive(Error, Debug)]
pub enum OutputValidationError {
    #[error("compilation failed.\nstatus: {status}\nstdout: {stdout}\nstderr: {stderr}")]
    CommandFailure {
        status: ExitStatus,
        stdout: String,
        stderr: String,
    },
}

fn validate_output(output: &Output) -> Result<(), OutputValidationError> {
    if !output.status.success() {
        return Err(OutputValidationError::CommandFailure {
            status: output.status,
            stdout: String::from_utf8_lossy(&output.stdout).into(),
            stderr: String::from_utf8_lossy(&output.stderr).into(),
        });
    }
    Ok(())
}

#[derive(Error, Debug)]
pub enum CompileCFilesError {
    #[error("IO operation while compiling C files: {0}")]
    Io(#[from] io::Error),
    #[error("failed to compile intermediates from C files: {0}")]
    CompileIntermediates(cc::Error),
    #[error("error compiling C files (compilation failed): {0}")]
    Compilation(#[from] cc::Error),
    #[error("error compiling C files (output validation failed): {0}")]
    OutputValidation(#[from] OutputValidationError),
    #[error("compiling C files succeeded, but the expected library {0} was not created")]
    LibOutputNotCreated(String),
}

/// Compiles a set of C files into a single dynamic library and places them under `{target_dir}/{target_file}`.
/// # Panics
/// Panics if no parent or no filename can be determined for the target path.
pub(crate) async fn compile_c_files(
    files: &Vec<PathBuf>,
    target_module: &LuaModule,
    target_dir: &Path,
    lua: &LuaInstallation,
    external_dependencies: &HashMap<String, ExternalDependencyInfo>,
    config: &Config,
) -> Result<(), CompileCFilesError> {
    let target = target_dir.join(target_module.to_lib_path());

    let parent = target.parent().expect("Couldn't determine parent");
    let file = target
        .file_name()
        .expect("Couldn't determine filename")
        .to_string_lossy()
        .to_string();

    std::fs::create_dir_all(parent)?;

    let host = Triple::host();

    // See https://github.com/rust-lang/cc-rs/issues/594#issuecomment-2110551057

    let mut build = cc::Build::new();
    let intermediate_dir = tempfile::tempdir()?;
    let build = build
        .cargo_output(config.verbose())
        .cargo_metadata(config.verbose())
        .cargo_warnings(config.verbose())
        .warnings(config.verbose())
        .files(files)
        .host(&host.to_string())
        .target(&host.to_string())
        .includes(lua.includes())
        .includes(
            external_dependencies
                .iter()
                .filter_map(|(_, dep)| dep.include_dir.as_ref()),
        )
        .opt_level(3)
        .out_dir(intermediate_dir);

    let compiler = build.try_get_compiler()?;
    // Suppress all warnings
    if compiler.is_like_msvc() {
        build.flag("-W0");
    } else {
        build.flag("-w");
    }
    for arg in lua.define_flags() {
        build.flag(&arg);
    }

    let objects = build
        .try_compile_intermediates()
        .map_err(CompileCFilesError::CompileIntermediates)?;

    let output_path = parent.join(&file);

    // Linking from within a temp directory makes sure the linker doesn't create any build artifacts in
    // the current working directory.
    // See https://github.com/lumen-oss/lux/issues/1106
    let temp_work_dir = tempfile::tempdir().map_err(|err| {
        io::Error::other(format!(
            "failed to create temporary directory for linking:\n{err}"
        ))
    })?;
    let output = if compiler.is_like_msvc() {
        let def_file = mk_def_file(temp_work_dir.path(), &file, target_module)?;
        let cmd = compiler.to_command();
        let mut cmd: tokio::process::Command = cmd.into();
        cmd.current_dir(temp_work_dir.path())
            .arg("/NOLOGO")
            .args(&objects)
            .arg("/LD")
            .arg("/link")
            .arg(format!("/DEF:{}", def_file.display()))
            .arg(format!("/OUT:{}", output_path.display()))
            .args(lua.lib_link_args(&compiler))
            .args(
                external_dependencies
                    .iter()
                    .flat_map(|(_, dep)| dep.lib_link_args(&compiler)),
            )
            .output()
            .await?
    } else {
        let cmd = build.shared_flag(true).try_get_compiler()?.to_command();
        let mut cmd: tokio::process::Command = cmd.into();
        cmd.current_dir(temp_work_dir.path())
            .args(vec!["-o".into(), output_path.to_string_lossy().to_string()])
            .args(lua.lib_link_args(&compiler))
            .args(
                external_dependencies
                    .iter()
                    .flat_map(|(_, dep)| dep.lib_link_args(&compiler)),
            )
            .args(&objects)
            .output()
            .await?
    };

    if config.verbose() {
        if !&output.stdout.is_empty() {
            println!("{}", String::from_utf8_lossy(&output.stdout));
        }
        if !&output.stderr.is_empty() {
            eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        }
    }

    validate_output(&output)?;
    log_command_output(&output, config);

    if output_path.exists() {
        Ok(())
    } else {
        Err(CompileCFilesError::LibOutputNotCreated(
            output_path.to_slash_lossy().to_string(),
        ))
    }
}

/// On MSVC, we need to create Lua definitions manually
fn mk_def_file(
    dir: &Path,
    output_file_name: &str,
    target_module: &LuaModule,
) -> io::Result<PathBuf> {
    let mut def_file: PathBuf = dir.join(output_file_name);
    def_file.set_extension(".def");
    let exported_name = target_module.to_string().replace(".", "_");
    let exported_name = exported_name
        .split_once('-')
        .map(|(_, after_hyphen)| after_hyphen.to_string())
        .unwrap_or_else(|| exported_name.clone());
    let content = format!(
        r#"EXPORTS
luaopen_{exported_name}
"#,
    );
    std::fs::write(&def_file, content)?;
    Ok(def_file)
}

// TODO: (#261): special cases for mingw/cygwin?

/// the extension for C shared libraries.
pub(crate) fn c_dylib_extension() -> &'static str {
    if cfg!(target_env = "msvc") {
        "dll"
    } else {
        "so"
    }
}

/// the extension for C static libraries.
pub(crate) fn c_lib_extension() -> &'static str {
    if cfg!(target_env = "msvc") {
        "lib"
    } else {
        "a"
    }
}

/// the extension for C objects.
pub(crate) fn c_obj_extension() -> &'static str {
    if cfg!(target_env = "msvc") {
        "obj"
    } else {
        "o"
    }
}

pub(crate) fn default_cflags() -> &'static str {
    if cfg!(target_env = "msvc") {
        "/NOLOGO /MD /O2"
    } else {
        "-O2"
    }
}

pub(crate) fn default_libflag() -> &'static str {
    if cfg!(target_os = "macos") {
        "-bundle -undefined dynamic_lookup -all_load"
    } else if cfg!(target_env = "msvc") {
        "/NOLOGO /DLL"
    } else {
        "-shared"
    }
}

#[derive(Error, Debug)]
pub enum CompileCModulesError {
    #[error("IO operation while compiling C modules: {0}")]
    Io(#[from] io::Error),
    #[error("failed to compile intermediates from C modules: {0}")]
    CompileIntermediates(cc::Error),
    #[error("error compiling C modules (compilation failed): {0}")]
    Compilation(#[from] cc::Error),
    #[error("error compiling C modules (output validation failed): {0}")]
    OutputValidation(#[from] OutputValidationError),
    #[error("compiling C modules succeeded, but the expected library {0} was not created")]
    LibOutputNotCreated(String),
}

/// Compiles a set of C files (with extra metadata) to a given destination.
/// # Panics
/// Panics if no filename for the target path can be determined.
pub(crate) async fn compile_c_modules(
    data: &ModulePaths,
    source_dir: &Path,
    target_module: &LuaModule,
    target_dir: &Path,
    lua: &LuaInstallation,
    external_dependencies: &HashMap<String, ExternalDependencyInfo>,
    config: &Config,
) -> Result<(), CompileCModulesError> {
    let target = target_dir.join(target_module.to_lib_path());

    let parent = target.parent().expect("Couldn't determine parent");
    std::fs::create_dir_all(parent)?;

    let host = Triple::host();

    let mut build = cc::Build::new();
    let source_files = data
        .sources
        .iter()
        .map(|dir| source_dir.join(dir))
        .collect_vec();
    let include_dirs = data
        .incdirs
        .iter()
        .map(|dir| source_dir.join(dir))
        .chain(
            external_dependencies
                .iter()
                .filter_map(|(_, dep)| dep.include_dir.clone()),
        )
        .collect_vec();

    let intermediate_dir = tempfile::tempdir()?;
    let build = build
        .cargo_output(config.verbose())
        .cargo_metadata(config.verbose())
        .cargo_warnings(config.verbose())
        .warnings(config.verbose())
        .files(source_files)
        .host(&host.to_string())
        .target(&host.to_string())
        .includes(&include_dirs)
        .includes(lua.includes())
        .includes(
            external_dependencies
                .iter()
                .filter_map(|(_, dep)| dep.include_dir.as_ref()),
        )
        .opt_level(3)
        .out_dir(intermediate_dir);

    let compiler = build.try_get_compiler()?;
    let is_msvc = compiler.is_like_msvc();
    // Suppress all warnings
    if is_msvc {
        build.flag("-W0");
    } else {
        build.flag("-w");
    }
    for arg in lua.define_flags() {
        build.flag(&arg);
    }

    // `cc::Build` has no `defines()` function, so we manually feed in the
    // definitions in a verbose loop
    for (name, value) in &data.defines {
        build.define(name, value.as_deref());
    }

    let file = target
        .file_name()
        .expect("Couldn't determine filename")
        .to_string_lossy()
        .to_string();
    // See https://github.com/rust-lang/cc-rs/issues/594#issuecomment-2110551057
    let objects = build
        .try_compile_intermediates()
        .map_err(CompileCModulesError::CompileIntermediates)?;

    let libdir_args = data.libdirs.iter().map(|libdir| {
        if is_msvc {
            format!("/LIBPATH:{}", source_dir.join(libdir).display())
        } else {
            format!("-L{}", source_dir.join(libdir).display())
        }
    });

    let library_args = data.libraries.iter().map(|library| {
        if is_msvc {
            format!("{}.lib", library.to_str().unwrap())
        } else {
            format!("-l{}", library.to_str().unwrap())
        }
    });

    // Linking from within a temp directory makes sure the linker doesn't create any build artifacts in
    // the current working directory.
    // See https://github.com/lumen-oss/lux/issues/1106
    let temp_work_dir = tempfile::tempdir().map_err(|err| {
        io::Error::other(format!(
            "failed to create temporary directory for linking:\n{err}"
        ))
    })?;
    let output_path = parent.join(&file);
    let output = if is_msvc {
        let def_file = mk_def_file(temp_work_dir.path(), &file, target_module)?;
        let cmd = build.try_get_compiler()?.to_command();
        let mut cmd: tokio::process::Command = cmd.into();
        cmd.current_dir(temp_work_dir.path())
            .arg("/NOLOGO")
            .args(&objects)
            .arg("/LD")
            .arg("/link")
            .arg(format!("/DEF:{}", def_file.display()))
            .arg(format!("/OUT:{}", output_path.display()))
            .args(lua.lib_link_args(&build.try_get_compiler()?))
            .args(
                external_dependencies
                    .iter()
                    .flat_map(|(_, dep)| dep.lib_link_args(&compiler)),
            )
            .args(libdir_args)
            .args(library_args)
            .output()
            .await?
    } else {
        let cmd = build.shared_flag(true).try_get_compiler()?.to_command();
        let mut cmd: tokio::process::Command = cmd.into();
        cmd.current_dir(temp_work_dir.path())
            .args(vec!["-o".into(), output_path.to_string_lossy().to_string()])
            .args(lua.lib_link_args(&build.try_get_compiler()?))
            .args(
                external_dependencies
                    .iter()
                    .flat_map(|(_, dep)| dep.lib_link_args(&compiler)),
            )
            .args(&objects)
            .args(libdir_args)
            .args(library_args)
            .output()
            .await?
    };

    if config.verbose() {
        if !&output.stdout.is_empty() {
            println!("{}", String::from_utf8_lossy(&output.stdout));
        }
        if !&output.stderr.is_empty() {
            eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        }
    }

    validate_output(&output)?;
    log_command_output(&output, config);

    if output_path.exists() {
        Ok(())
    } else {
        Err(CompileCModulesError::LibOutputNotCreated(
            output_path.to_slash_lossy().to_string(),
        ))
    }
}

#[derive(Debug, Error)]
pub enum InstallBinaryError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Paths(#[from] PathsError),
    #[error("error wrapping binary: {0}")]
    Wrap(#[from] WrapBinaryError),
}

#[derive(Debug, Error)]
pub enum WrapBinaryError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Utf8(#[from] FromUtf8Error),
    #[error("no `lua` executable found")]
    NoLuaBinary,
}

/// Returns the file path of the installed binary
pub(crate) async fn install_binary(
    source: &Path,
    target: &str,
    tree: &Tree,
    lua: &LuaInstallation,
    deploy: &DeploySpec,
    config: &Config,
) -> Result<PathBuf, InstallBinaryError> {
    tokio::fs::create_dir_all(&tree.bin()).await?;
    let paths = Paths::new(tree)?;
    let script =
        if deploy.wrap_bin_scripts && is_compatible_lua_script(source, lua, &paths, config).await {
            install_wrapped_binary(source, target, tree, lua, config).await?
        } else {
            let target = tree.bin().join(target);
            tokio::fs::copy(source, &target).await?;
            target
        };

    #[cfg(unix)]
    set_executable_permissions(&script).await?;

    Ok(script)
}

/// Logs the output's stdout and stderr in verbose mode
pub(crate) fn log_command_output(output: &Output, config: &Config) {
    if config.verbose() {
        if !output.stderr.is_empty() {
            println!("{}", String::from_utf8_lossy(&output.stderr));
        }
        if !output.stdout.is_empty() {
            println!("{}", String::from_utf8_lossy(&output.stdout));
        }
    }
}

async fn install_wrapped_binary(
    source: &Path,
    target: &str,
    tree: &Tree,
    lua: &LuaInstallation,
    config: &Config,
) -> Result<PathBuf, WrapBinaryError> {
    let unwrapped_bin_dir = tree.unwrapped_bin();
    tokio::fs::create_dir_all(&unwrapped_bin_dir).await?;
    let unwrapped_bin = unwrapped_bin_dir.join(target);
    tokio::fs::copy(source, &unwrapped_bin).await?;

    #[cfg(target_family = "unix")]
    let target = tree.bin().join(target);
    #[cfg(target_family = "windows")]
    let target = tree.bin().join(format!("{}.bat", target));

    let lua_bin = lua
        .lua_binary_or_config_override(config)
        .ok_or(WrapBinaryError::NoLuaBinary)?;

    #[cfg(target_family = "unix")]
    let content = format!(
        r#"#!/bin/sh

exec {0} "{1}" "$@"
"#,
        lua_bin,
        unwrapped_bin.display(),
    );
    #[cfg(target_family = "windows")]
    let content = format!(
        r#"@echo off
setlocal

{0} "{1}" %*

exit /b %ERRORLEVEL%
"#,
        lua_bin,
        unwrapped_bin.display(),
    );

    tokio::fs::write(&target, content).await?;
    Ok(target)
}

#[cfg(unix)]
async fn set_executable_permissions(script: &Path) -> std::io::Result<()> {
    let mut perms = tokio::fs::metadata(&script).await?.permissions();
    perms.set_mode(0o744);
    tokio::fs::set_permissions(&script, perms).await?;
    Ok(())
}

/// Tries to load the file with Lua. If the file can be loaded,
/// we treat it as a valid Lua script.
///
/// NOTE: Lua may successfully load very short bash scripts (like `echo "Hello"`).
/// But that's unlikely to be the case in practise.
/// If a script is mistaken for a Lua script, package authors can disable
/// wrapping with the `deploy.wrap_bin_scripts` rockspec config.
async fn is_compatible_lua_script(
    file: &Path,
    lua: &LuaInstallation,
    paths: &Paths,
    config: &Config,
) -> bool {
    let lua_path = paths.package_path_prepended().joined();
    let lua_cpath = paths.package_cpath_prepended().joined();
    let path = paths.path_prepended().joined();
    if let Some(lua_bin) = &lua.lua_binary_or_config_override(config) {
        let lua_bin_path = PathBuf::from(lua_bin);
        if lua_bin_path.is_file() || which(lua_bin).is_ok() {
            Command::new(lua_bin)
                .arg("-e")
                .arg(format!(
                    "if loadfile('{}') then os.exit(0) else os.exit(1) end",
                    // On Windows, Lua escapes path separators, so we ensure forward slashes
                    file.to_slash_lossy()
                ))
                .stderr(Stdio::null())
                .stdout(Stdio::null())
                .env("LUA_PATH", lua_path)
                .env("LUA_CPATH", lua_cpath)
                .env("PATH", path)
                .status()
                .await
                .is_ok_and(|status| status.success())
        } else {
            is_compatible_lua_script_fallback(file, lua, config).await
        }
    } else {
        false
    }
}

/// Fallback function for `is_compatible_lua_script`.
/// Uses the bundled Lua interpreter. This may be less reliable,
/// as it cannot load scripts for an incompatible Lua version.
/// But it works for test dependencies like nlua.
async fn is_compatible_lua_script_fallback(
    file: &Path,
    lua_installation: &LuaInstallation,
    config: &Config,
) -> bool {
    lua_installation
        .lua_binary_or_config_override(config)
        .is_some_and(|_| {
            let lua = Lua::new();
            lua.load(format!(
                "is_compatible_lua_script = loadfile('{}') ~= nil",
                file.to_slash_lossy()
            ))
            .exec()
            .is_ok_and(|()| {
                lua.globals()
                    .get("is_compatible_lua_script")
                    .is_ok_and(|value| {
                        lua.from_value(value)
                            .is_ok_and(|is_compatible| is_compatible)
                    })
            })
        })
}

pub(crate) fn substitute_variables(
    input: &str,
    output_paths: &RockLayout,
    lua: &LuaInstallation,
    external_dependencies: &HashMap<String, ExternalDependencyInfo>,
    config: &Config,
) -> Result<String, VariableSubstitutionError> {
    variables::substitute(
        &[
            output_paths,
            lua,
            external_dependencies,
            &Environment {},
            config,
        ],
        input,
    )
}

pub(crate) fn format_path(path: &Path) -> String {
    let path_str = path.to_slash_lossy();
    if cfg!(windows) {
        path_str.to_string()
    } else {
        try_quote(&path_str)
            .map(|str| str.to_string())
            .unwrap_or(format!("'{path_str}'"))
    }
}

#[cfg(test)]
mod tests {
    use tokio::process::Command;

    use crate::{
        config::{ConfigBuilder, LuaVersion},
        lua_installation::detect_installed_lua_version,
        progress::MultiProgress,
    };

    use super::*;

    #[tokio::test]
    async fn test_is_compatible_lua_script() {
        let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));
        let config = ConfigBuilder::new()
            .unwrap()
            .lua_version(lua_version)
            .build()
            .unwrap();
        let lua_version = config.lua_version().unwrap();
        let progress = MultiProgress::new(&config);
        let bar = progress.map(MultiProgress::new_bar);
        let lua = LuaInstallation::new(lua_version, &config, &bar)
            .await
            .unwrap();
        let valid_script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/sample_lua_bin_script_valid");
        let temp_dir = assert_fs::TempDir::new().unwrap().to_path_buf();
        let tree = Tree::new(temp_dir, lua_version.clone(), &config).unwrap();
        let paths = Paths::new(&tree).unwrap();
        assert!(is_compatible_lua_script(&valid_script, &lua, &paths, &config).await);
        let invalid_script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/sample_lua_bin_script_invalid");
        assert!(!is_compatible_lua_script(&invalid_script, &lua, &paths, &config).await);
    }

    #[tokio::test]
    async fn test_install_wrapped_binary() {
        let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));
        let temp = assert_fs::TempDir::new().unwrap();
        let config = ConfigBuilder::new()
            .unwrap()
            .lua_version(lua_version)
            .user_tree(Some(temp.to_path_buf()))
            .build()
            .unwrap();
        let lua_version = config.lua_version().unwrap();
        let progress = MultiProgress::new(&config);
        let bar = progress.map(MultiProgress::new_bar);
        let lua = LuaInstallation::new(lua_version, &config, &bar)
            .await
            .unwrap();
        let tree = config.user_tree(lua_version.clone()).unwrap();
        let valid_script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/sample_lua_bin_script_valid");
        let script_name = "test_script";
        let script_path = install_wrapped_binary(&valid_script, script_name, &tree, &lua, &config)
            .await
            .unwrap();

        #[cfg(unix)]
        set_executable_permissions(&script_path).await.unwrap();

        assert!(Command::new(script_path.to_string_lossy().to_string())
            .status()
            .await
            .is_ok_and(|status| status.success()));
    }
}
