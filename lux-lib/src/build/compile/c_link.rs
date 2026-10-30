use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    process::{ExitStatus, Output},
};

use itertools::Itertools;
use miette::Diagnostic;
use path_slash::PathExt;
use thiserror::Error;
use tokio::process::Command;

use crate::{
    build::external_dependency::ExternalDependencyInfo, config::Config, fs, lua, rockspec,
};

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error("IO operation failed while linking C modules")]
    Io(#[from] io::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
    #[error(transparent)]
    CC(#[from] cc::Error),
    #[error("compilation failed.\nstatus: {status}\nstdout: {stdout}\nstderr: {stderr}")]
    #[diagnostic(help(
        r#"check the compiler output above for details.
rerun with `--verbose` for more information."#
    ))]
    CommandFailure {
        status: ExitStatus,
        stdout: String,
        stderr: String,
    },
    #[error("compiling C modules succeeded, but the expected library {0} was not created")]
    #[diagnostic(help(
        "check the rockspec or lux.toml `modules` and `lib_modules` tables for correct paths."
    ))]
    LibOutputNotCreated(String),
}

fn validate_output(output: &Output) -> Result<(), Error> {
    if !output.status.success() {
        return Err(Error::CommandFailure {
            status: output.status,
            stdout: String::from_utf8_lossy(&output.stdout).into(),
            stderr: String::from_utf8_lossy(&output.stderr).into(),
        });
    }
    Ok(())
}

/// On MSVC, we need to create Lua definitions manually
#[tracing::instrument(level = "trace")]
fn mk_def_file(
    dir: &Path,
    output_file_name: &str,
    target_module: &rockspec::build::backend::Module,
) -> Result<PathBuf, fs::Error> {
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
    fs::sync::write(&def_file, content)?;
    Ok(def_file)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn artifacts(
    build: &mut cc::Build,
    lua: &lua::Installation,
    external_dependencies: &HashMap<String, ExternalDependencyInfo>,
    target_file_name: &str,
    libdir_args: Vec<String>,
    library_args: Vec<String>,
    target_module: &rockspec::build::backend::Module,
    target_parent_dir: &Path,
    objects: Vec<PathBuf>,
    config: &Config,
) -> Result<(), Error> {
    let output_path =
        std::path::absolute(target_parent_dir.join(target_file_name)).map_err(Error::Io)?;
    let compiler = build.try_get_compiler()?;
    // Linking from within a temp directory makes sure the linker doesn't create any build artifacts in
    // the current working directory.
    // See https://github.com/lumen-oss/lux/issues/1106
    let temp_work_dir = fs::tempfile::tempdir()?;
    let is_msvc = compiler.is_like_msvc();
    let cmd = build.try_get_compiler()?.to_command();
    let mut cmd: tokio::process::Command = cmd.into();
    cmd.current_dir(temp_work_dir.path());
    let output = if is_msvc {
        let def_file = mk_def_file(temp_work_dir.path(), target_file_name, target_module)?;
        cmd.args(&objects).arg("/LD");
        add_variable_if_set(config, "LDFLAGS", &mut cmd);
        cmd.arg("/link")
            .arg("/NOIMPLIB")
            .arg("/NOEXP")
            .arg(format!("/DEF:{}", def_file.display()))
            .arg(format!("/OUT:{}", output_path.display()))
            .args(lua_lib_link_args(lua, &compiler))
            .args(
                external_dependencies
                    .values()
                    .flat_map(|dep| dep.lib_link_args(&compiler)),
            )
            .args(libdir_args)
            .args(library_args)
            .output()
            .await?
    } else {
        add_variable_if_set(config, "LIBFLAG", &mut cmd);
        add_variable_if_set(config, "LDFLAGS", &mut cmd);
        cmd.args(vec!["-o".into(), output_path.to_string_lossy().to_string()])
            .args(lua_lib_link_args(lua, &build.try_get_compiler()?))
            .args(
                external_dependencies
                    .values()
                    .flat_map(|dep| dep.lib_link_args(&compiler)),
            )
            .args(&objects)
            .args(libdir_args)
            .args(library_args)
            .output()
            .await?
    };
    crate::build::command::trace_output(&output);
    validate_output(&output)?;

    if output_path.exists() {
        Ok(())
    } else {
        Err(Error::LibOutputNotCreated(
            output_path.to_slash_lossy().to_string(),
        ))
    }
}

/// NOTE: In luarocks, these are behind a link_lua_explicity config option
fn lua_lib_link_args(lua: &lua::Installation, compiler: &cc::Tool) -> Vec<String> {
    if cfg!(target_os = "macos") {
        // On macos, linking Lua can lead to duplicate symbol errors
        Vec::new()
    } else {
        lua.lib_link_args(compiler)
    }
}

/// Lookup a variable in the `Config` and if set and non-empty,
/// split it into multiple arguments and add them to the `Command`.
fn add_variable_if_set(config: &Config, name: &str, cmd: &mut Command) {
    let variables = config.variables();
    if let Some(var_str) = variables.get(name) {
        if !var_str.is_empty() {
            let vars = var_str.split_whitespace().collect_vec();
            cmd.args(vars);
        }
    }
}
