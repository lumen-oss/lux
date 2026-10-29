use std::path::PathBuf;

use crate::fs;
use crate::project;
use crate::rockspec;
use std::string::FromUtf8Error;

use crate::{
    build::backend::{BuildBackend, BuildInfo, RunBuildArgs},
    project::{TomlDeError, TOML_FILE_NAME},
    rockspec::Rockspec,
    tree::InstallTree,
};
use miette::Diagnostic;
use thiserror::Error;

use super::{builtin, cmake, command, make, rust_binary, rust_mlua, treesitter_parser};

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
    #[error(transparent)]
    FromUtf8(#[from] FromUtf8Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Toml(#[from] TomlDeError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LocalProjectTomlValidation(#[from] rockspec::toml::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaRockspec(#[from] rockspec::lua::Error),
    #[error("builtin build failed")]
    #[diagnostic(forward(0))]
    Builtin(#[from] builtin::Error),
    #[error("cmake build failed")]
    #[diagnostic(forward(0))]
    CMake(#[from] cmake::Error),
    #[error("make build failed")]
    #[diagnostic(forward(0))]
    Make(#[from] make::Error),
    #[error("command build failed")]
    #[diagnostic(forward(0))]
    Command(#[from] command::Error),
    #[error("rust-mlua build failed")]
    #[diagnostic(forward(0))]
    Rust(#[from] rust_mlua::Error),
    #[error("rust-binary build failed")]
    #[diagnostic(forward(0))]
    RustBinary(#[from] rust_binary::Error),
    #[error("treesitter-parser build failed")]
    #[diagnostic(forward(0))]
    TreesitterBuild(#[from] treesitter_parser::Error),
    #[error("cannot build from a project source that requires a luarocks build backend: '{0}'")]
    #[diagnostic(help(
        r#"lux does not support building from source with a luarocks build backend.
Use a pre-built rock or a different build backend."#
    ))]
    UnsupportedLuarocksBuildBackend(String),
}

#[tracing::instrument(name = "source::build", skip_all, level = "debug")]
pub(crate) async fn build<T>(args: RunBuildArgs<'_, T>) -> Result<BuildInfo, Error>
where
    T: InstallTree + Sync,
{
    let package = args.package;
    let tree = args.tree;
    let layout = tree.layout_for(package);
    let build_dir = args.build_dir;

    let mut build_spec = rockspec::build::BuildSpec::default();
    let mut copy_directories = None;
    for path in fs::sync::read_dir(build_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
    {
        if path.file_name().is_some_and(|name| name == TOML_FILE_NAME) {
            let toml_content = String::from_utf8(fs::tokio::read(path).await?)?;
            let project_toml =
                rockspec::toml::Partial::new(TOML_FILE_NAME, &toml_content, PathBuf::new())?
                    .into_local()?;
            build_spec = project_toml.build().current_platform().clone();
            copy_directories = Some(build_spec.copy_directories);
            break;
        } else if path.extension().is_some_and(|ext| ext == "rockspec") {
            let rockspec_content = String::from_utf8(fs::tokio::read(path).await?)?;
            let rockspec = rockspec::lua::Local::new(&rockspec_content, PathBuf::new())?;
            build_spec = rockspec.build().current_platform().clone();
            copy_directories = Some(build_spec.copy_directories);
            break;
        }
    }
    let build_info = match build_spec.build_backend {
        Some(rockspec::build::BuildBackendSpec::Builtin(build_spec)) => {
            build_spec
                .run(args)
                .await?
        }
        Some(rockspec::build::BuildBackendSpec::Make(make_spec)) => {
            make_spec
                .run(args)
                .await?
        }
        Some(rockspec::build::BuildBackendSpec::CMake(cmake_spec)) => {
            cmake_spec
                .run(args)
                .await?
        }
        Some(rockspec::build::BuildBackendSpec::Command(command_spec)) => {
            command_spec
                .run(args)
                .await?
        }
        Some(rockspec::build::BuildBackendSpec::RustMlua(rust_mlua_spec)) => {
            rust_mlua_spec
                .run(args)
                .await?
        }
        Some(rockspec::build::BuildBackendSpec::RustBinary(rust_binary_spec)) => {
            rust_binary_spec
                .run(args)
                .await?
        }
        Some(rockspec::build::BuildBackendSpec::TreesitterParser(treesitter_parser_spec)) => {
            treesitter_parser_spec
                .run(args)
                .await?
        }
        Some(rockspec::build::BuildBackendSpec::LuaRock(build_backend)) => return Err(Error::UnsupportedLuarocksBuildBackend(build_backend)),
        Some(rockspec::build::BuildBackendSpec::Source) | // This should not be possible. Let's ignore it.
        None => BuildInfo::default(),
    };
    match copy_directories {
        Some(copy_directories) => {
            for directory in copy_directories.iter().filter(|dir| {
                dir.file_name()
                    .is_some_and(|name| name != "doc" && name != "docs")
            }) {
                project::copy_dir(&build_dir.join(directory), &layout.etc).await?;
            }
        }
        None => {
            // We copy all directories if there is no rockspec
            for subdirectory in fs::sync::read_dir(build_dir)?
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let path = entry.path();
                    if path.is_dir()
                        && path.file_name().is_some_and(|name| {
                            !matches!(
                                name.to_string_lossy().to_string().as_str(),
                                "lua" | "src" | "doc" | "docs"
                            )
                        })
                    {
                        path.file_name()
                            .map(|name| name.to_string_lossy().to_string())
                    } else {
                        None
                    }
                })
            {
                project::copy_dir(
                    &build_dir.join(&subdirectory),
                    &layout.etc.join(&subdirectory),
                )
                .await?;
            }
        }
    }
    Ok(build_info)
}
