use crate::build::backend::{BuildBackend, BuildInfo, RunBuildArgs};
use crate::lockfile::{LockfileError, OptState, RemotePackageSourceUrl};
use crate::lua_installation::LuaInstallationError;
use crate::lua_rockspec::LuaVersionError;
use crate::operations::{RemotePackageSourceMetadata, UnpackError};
use crate::rockspec::{LuaVersionCompatibility, Rockspec};
use crate::tree::{self, EntryType, TreeError};
use bytes::Bytes;
use std::collections::HashMap;
use std::fs::DirEntry;
use std::io::Cursor;
use std::path::PathBuf;
use std::{io, path::Path};

use crate::{
    config::Config,
    hash::HasIntegrity,
    lockfile::{LocalPackage, LocalPackageHashes, LockConstraint, PinnedState},
    lua_installation::LuaInstallation,
    lua_rockspec::BuildBackendSpec,
    operations::{self, FetchSrcError},
    package::PackageSpec,
    progress::{Progress, ProgressBar},
    remote_package_source::RemotePackageSource,
    tree::{RockLayout, Tree},
};
use bon::Builder;
use builtin::BuiltinBuildError;
use cmake::CMakeError;
use command::CommandError;
use external_dependency::{ExternalDependencyError, ExternalDependencyInfo};

use indicatif::style::TemplateError;
use itertools::Itertools;
use luarocks::LuarocksBuildError;
use make::MakeError;
use mlua::FromLua;
use patch::{Patch, PatchError};
use rust_mlua::RustError;
use source::SourceBuildError;
use ssri::Integrity;
use thiserror::Error;
use treesitter_parser::TreesitterBuildError;
use utils::{recursive_copy_dir, CompileCFilesError, InstallBinaryError};

mod builtin;
mod cmake;
mod command;
mod luarocks;
mod make;
mod patch;
mod rust_mlua;
mod source;
mod treesitter_parser;

pub(crate) mod backend;
pub(crate) mod utils;

pub mod external_dependency;

/// A rocks package builder, providing fine-grained control
/// over how a package should be built.
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct Build<'a, R: Rockspec + HasIntegrity> {
    rockspec: &'a R,
    tree: &'a Tree,
    entry_type: tree::EntryType,
    config: &'a Config,
    progress: &'a Progress<ProgressBar>,
    lua: &'a LuaInstallation,

    #[builder(default)]
    pin: PinnedState,
    #[builder(default)]
    opt: OptState,
    #[builder(default)]
    constraint: LockConstraint,
    #[builder(default)]
    behaviour: BuildBehaviour,

    #[builder(setters(vis = "pub(crate)"))]
    source_spec: Option<RemotePackageSourceSpec>,

    // TODO(vhyrro): Remove this and enforce that this is provided at a type level.
    #[builder(setters(vis = "pub(crate)"))]
    source: Option<RemotePackageSource>,
}

pub(crate) enum RemotePackageSourceSpec {
    RockSpec(Option<RemotePackageSourceUrl>),
    SrcRock(SrcRockSource),
}

/// A packed .src.rock archive.
pub(crate) struct SrcRockSource {
    pub bytes: Bytes,
    pub source_url: RemotePackageSourceUrl,
}

// Overwrite the `build()` function to use our own instead.
impl<R: Rockspec + HasIntegrity, State> BuildBuilder<'_, R, State>
where
    State: build_builder::State + build_builder::IsComplete,
{
    pub async fn build(self) -> Result<LocalPackage, BuildError> {
        do_build(self._build()).await
    }
}

#[derive(Error, Debug)]
pub enum BuildError {
    #[error("builtin build failed: {0}")]
    Builtin(#[from] BuiltinBuildError),
    #[error("cmake build failed: {0}")]
    CMake(#[from] CMakeError),
    #[error("make build failed: {0}")]
    Make(#[from] MakeError),
    #[error("command build failed: {0}")]
    Command(#[from] CommandError),
    #[error("rust-mlua build failed: {0}")]
    Rust(#[from] RustError),
    #[error("treesitter-parser build failed: {0}")]
    TreesitterBuild(#[from] TreesitterBuildError),
    #[error("luarocks build failed: {0}")]
    LuarocksBuild(#[from] LuarocksBuildError),
    #[error("building from rock source failed: {0}")]
    SourceBuild(#[from] SourceBuildError),
    #[error("IO operation failed: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Lockfile(#[from] LockfileError),
    #[error(transparent)]
    Tree(#[from] TreeError),
    #[error("failed to create spinner: {0}")]
    SpinnerFailure(#[from] TemplateError),
    #[error(transparent)]
    ExternalDependencyError(#[from] ExternalDependencyError),
    #[error(transparent)]
    PatchError(#[from] PatchError),
    #[error(transparent)]
    CompileCFiles(#[from] CompileCFilesError),
    #[error(transparent)]
    LuaVersion(#[from] LuaVersionError),
    #[error("source integrity mismatch.\nExpected: {expected},\nbut got: {actual}")]
    SourceIntegrityMismatch {
        expected: Integrity,
        actual: Integrity,
    },
    #[error("failed to unpack src.rock:\n{0}")]
    UnpackSrcRock(UnpackError),
    #[error("failed to fetch rock source:\n{0}")]
    FetchSrcError(#[from] FetchSrcError),
    #[error("failed to install binary {0}: {1}")]
    InstallBinary(String, InstallBinaryError),
    #[error(transparent)]
    LuaInstallation(#[from] LuaInstallationError),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum BuildBehaviour {
    /// Don't force a rebuild if the package is already installed
    #[default]
    NoForce,
    /// Force a rebuild if the package is already installed
    Force,
}

impl FromLua for BuildBehaviour {
    fn from_lua(value: mlua::Value, lua: &mlua::Lua) -> mlua::Result<Self> {
        Ok(bool::from_lua(value, lua)?.into())
    }
}

impl From<bool> for BuildBehaviour {
    fn from(value: bool) -> Self {
        if value {
            Self::Force
        } else {
            Self::NoForce
        }
    }
}

async fn run_build<R: Rockspec + HasIntegrity>(
    rockspec: &R,
    args: RunBuildArgs<'_>,
) -> Result<BuildInfo, BuildError> {
    let progress = args.progress;
    progress.map(|p| p.set_message("🛠️ Building..."));

    Ok(
        match rockspec.build().current_platform().build_backend.to_owned() {
            Some(BuildBackendSpec::Builtin(build_spec)) => build_spec.run(args).await?,
            Some(BuildBackendSpec::Make(make_spec)) => make_spec.run(args).await?,
            Some(BuildBackendSpec::CMake(cmake_spec)) => cmake_spec.run(args).await?,
            Some(BuildBackendSpec::Command(command_spec)) => command_spec.run(args).await?,
            Some(BuildBackendSpec::RustMlua(rust_mlua_spec)) => rust_mlua_spec.run(args).await?,
            Some(BuildBackendSpec::TreesitterParser(treesitter_parser_spec)) => {
                treesitter_parser_spec.run(args).await?
            }
            Some(BuildBackendSpec::LuaRock(_)) => luarocks::build(rockspec, args).await?,
            Some(BuildBackendSpec::Source) => source::build(args).await?,
            None => BuildInfo::default(),
        },
    )
}

#[allow(clippy::too_many_arguments)]
async fn install<R: Rockspec + HasIntegrity>(
    rockspec: &R,
    tree: &Tree,
    output_paths: &RockLayout,
    lua: &LuaInstallation,
    build_dir: &Path,
    entry_type: &EntryType,
    progress: &Progress<ProgressBar>,
    config: &Config,
) -> Result<(), BuildError> {
    progress.map(|p| {
        p.set_message(format!(
            "💻 Installing {} {}",
            rockspec.package(),
            rockspec.version()
        ))
    });

    let install_spec = &rockspec.build().current_platform().install;
    let lua_len = install_spec.lua.len();
    let lib_len = install_spec.lib.len();
    let bin_len = install_spec.bin.len();
    let conf_len = install_spec.conf.len();
    let total_len = lua_len + lib_len + bin_len + conf_len;
    progress.map(|p| p.set_position(total_len as u64));

    if lua_len > 0 {
        progress.map(|p| p.set_message("📋 Copying Lua modules..."));
    }
    for (target, source) in &install_spec.lua {
        let absolute_source = build_dir.join(source);
        utils::copy_lua_to_module_path(&absolute_source, target, &output_paths.src)?;
        progress.map(|p| p.set_position(p.position() + 1));
    }
    if lib_len > 0 {
        progress.map(|p| p.set_message("📋 Compiling C libraries..."));
    }
    for (target, source) in &install_spec.lib {
        let absolute_source = build_dir.join(source);
        let resolved_target = output_paths.lib.join(target);
        tokio::fs::copy(absolute_source, resolved_target).await?;
        progress.map(|p| p.set_position(p.position() + 1));
    }
    if entry_type.is_entrypoint() {
        if bin_len > 0 {
            progress.map(|p| p.set_message("💻 Installing binaries..."));
        }
        let deploy_spec = rockspec.deploy().current_platform();
        for (target, source) in &install_spec.bin {
            utils::install_binary(
                &build_dir.join(source),
                target,
                tree,
                lua,
                deploy_spec,
                config,
            )
            .await
            .map_err(|err| BuildError::InstallBinary(target.clone(), err))?;
            progress.map(|p| p.set_position(p.position() + 1));
        }
    }
    if conf_len > 0 {
        progress.map(|p| p.set_message("📋 Copying configuration files..."));
        for (target, source) in &install_spec.conf {
            let absolute_source = build_dir.join(source);
            let target = output_paths.conf.join(target);
            if let Some(parent_dir) = target.parent() {
                tokio::fs::create_dir_all(parent_dir).await?;
            }
            tokio::fs::copy(absolute_source, target).await?;
            progress.map(|p| p.set_position(p.position() + 1));
        }
    }
    Ok(())
}

async fn do_build<R>(build: Build<'_, R>) -> Result<LocalPackage, BuildError>
where
    R: Rockspec + HasIntegrity,
{
    let rockspec = build.rockspec;

    build.progress.map(|p| {
        p.set_message(format!(
            "🛠️ Building {}@{}...",
            rockspec.package(),
            rockspec.version()
        ))
    });

    let lua = build.lua;

    rockspec.validate_lua_version(&lua.version)?;

    let tree = build.tree;

    let temp_dir = tempfile::tempdir()?;

    let source_metadata = match build.source_spec {
        Some(RemotePackageSourceSpec::SrcRock(SrcRockSource { bytes, source_url })) => {
            let hash = bytes.hash()?;
            let cursor = Cursor::new(&bytes);
            operations::unpack_src_rock(cursor, temp_dir.path().to_path_buf(), build.progress)
                .await
                .map_err(BuildError::UnpackSrcRock)?;
            RemotePackageSourceMetadata { hash, source_url }
        }
        Some(RemotePackageSourceSpec::RockSpec(source_url)) => {
            operations::FetchSrc::new(temp_dir.path(), rockspec, build.config, build.progress)
                .maybe_source_url(source_url)
                .fetch_internal()
                .await?
        }
        None => {
            operations::FetchSrc::new(temp_dir.path(), rockspec, build.config, build.progress)
                .fetch_internal()
                .await?
        }
    };

    let hashes = LocalPackageHashes {
        rockspec: rockspec.hash()?,
        source: source_metadata.hash.clone(),
    };

    let mut package = LocalPackage::from(
        &PackageSpec::new(rockspec.package().clone(), rockspec.version().clone()),
        build.constraint,
        rockspec.binaries(),
        build
            .source
            .map(Result::Ok)
            .unwrap_or_else(|| {
                rockspec
                    .to_lua_remote_rockspec_string()
                    .map(RemotePackageSource::RockspecContent)
            })
            .unwrap_or(RemotePackageSource::Local),
        Some(source_metadata.source_url.clone()),
        hashes,
    );
    package.spec.pinned = build.pin;
    package.spec.opt = build.opt;

    match tree.lockfile()?.get(&package.id()) {
        Some(package) if build.behaviour == BuildBehaviour::NoForce => Ok(package.clone()),
        _ => {
            let output_paths = match build.entry_type {
                tree::EntryType::Entrypoint => tree.entrypoint(&package)?,
                tree::EntryType::DependencyOnly => tree.dependency(&package)?,
            };

            let rock_source = rockspec.source().current_platform();
            let build_dir = match &rock_source.unpack_dir {
                Some(unpack_dir) => temp_dir.path().join(unpack_dir),
                None => {
                    // Some older/off-spec rockspecs don't specify a `source.dir`.
                    // After unpacking the archive, if
                    //
                    //   - there exist no Lua or C sources
                    //   - there exists a single subdirectory that is not a source
                    //     or etc directory
                    //
                    // we assume it's the `source.dir`.
                    // Unlike the LuaRocks implementation - which filters when fetching sources -
                    // we only infer `source.dir` if the directory name is not 'src', 'lua'
                    // or one of the `build.copy_directories`.
                    // This allows us to build local projects with only a `src` directory.
                    //
                    // LuaRocks implementation:
                    // https://github.com/luarocks/luarocks/blob/4188fdb235aca66530d274c782374cf6afba09b8/src/luarocks/fetch.tl?plain=1#L526
                    let has_lua_or_c_sources = std::fs::read_dir(temp_dir.path())?
                        .filter_map(Result::ok)
                        .filter(|f| f.path().is_file())
                        .any(|f| {
                            f.path().extension().is_some_and(|ext| {
                                matches!(ext.to_string_lossy().to_string().as_str(), "lua" | "c")
                            })
                        });
                    if has_lua_or_c_sources {
                        temp_dir.path().into()
                    } else {
                        let dir_entries = std::fs::read_dir(temp_dir.path())?
                            .filter_map(Result::ok)
                            .filter(|f| f.path().is_dir())
                            .collect_vec();
                        if dir_entries.len() == 1
                            && !is_source_or_etc_dir(
                                unsafe { dir_entries.first().unwrap_unchecked() },
                                rockspec,
                            )
                        {
                            unsafe {
                                temp_dir
                                    .path()
                                    .join(dir_entries.first().unwrap_unchecked().path())
                            }
                        } else {
                            temp_dir.path().into()
                        }
                    }
                }
            };

            Patch::new(
                &build_dir,
                &rockspec.build().current_platform().patches,
                build.progress,
            )
            .apply()?;

            let external_dependencies = rockspec
                .external_dependencies()
                .current_platform()
                .iter()
                .map(|(name, dep)| {
                    ExternalDependencyInfo::probe(name, dep, build.config.external_deps())
                        .map(|info| (name.clone(), info))
                })
                .try_collect::<_, HashMap<_, _>, _>()?;

            let output = run_build(
                rockspec,
                RunBuildArgs::new()
                    .output_paths(&output_paths)
                    .no_install(false)
                    .lua(lua)
                    .external_dependencies(&external_dependencies)
                    .deploy(rockspec.deploy().current_platform())
                    .config(build.config)
                    .tree(tree)
                    .build_dir(&build_dir)
                    .progress(build.progress)
                    .build(),
            )
            .await?;

            package.spec.binaries.extend(output.binaries);

            install(
                rockspec,
                tree,
                &output_paths,
                lua,
                &build_dir,
                &build.entry_type,
                build.progress,
                build.config,
            )
            .await?;

            for directory in rockspec
                .build()
                .current_platform()
                .copy_directories
                .iter()
                .filter(|dir| {
                    dir.file_name()
                        .is_some_and(|name| name != "doc" && name != "docs")
                })
            {
                recursive_copy_dir(
                    &build_dir.join(directory),
                    &output_paths.etc.join(directory),
                )
                .await?;
            }

            recursive_copy_doc_dir(&output_paths, &build_dir).await?;

            if let Ok(rockspec_str) = rockspec.to_lua_remote_rockspec_string() {
                std::fs::write(output_paths.rockspec_path(), rockspec_str)?;
            }

            Ok(package)
        }
    }
}

fn is_source_or_etc_dir<R>(dir: &DirEntry, rockspec: &R) -> bool
where
    R: Rockspec + HasIntegrity,
{
    let copy_dirs = &rockspec.build().current_platform().copy_directories;
    let dir_name = dir.file_name().to_string_lossy().to_string();
    matches!(dir_name.as_str(), "lua" | "src")
        || copy_dirs
            .iter()
            .any(|copy_dir_name| copy_dir_name == &PathBuf::from(&dir_name))
}

async fn recursive_copy_doc_dir(
    output_paths: &RockLayout,
    build_dir: &Path,
) -> Result<(), BuildError> {
    let mut doc_dir = build_dir.join("doc");
    if !doc_dir.exists() {
        doc_dir = build_dir.join("docs");
    }
    recursive_copy_dir(&doc_dir, &output_paths.doc).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use predicates::prelude::*;
    use std::path::PathBuf;

    use assert_fs::{
        assert::PathAssert,
        prelude::{PathChild, PathCopy},
    };

    use crate::{
        config::{ConfigBuilder, LuaVersion},
        lua_installation::{detect_installed_lua_version, LuaInstallation},
        progress::MultiProgress,
        project::Project,
        tree::RockLayout,
    };

    #[tokio::test]
    async fn test_builtin_build() {
        let lua_version = detect_installed_lua_version().or(Some(LuaVersion::Lua51));
        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/sample-projects/no-build-spec/");
        let tree_dir = assert_fs::TempDir::new().unwrap();
        let config = ConfigBuilder::new()
            .unwrap()
            .lua_version(lua_version)
            .user_tree(Some(tree_dir.to_path_buf()))
            .build()
            .unwrap();
        let build_dir = assert_fs::TempDir::new().unwrap();
        build_dir.copy_from(&project_root, &["**"]).unwrap();
        let tree = config
            .user_tree(config.lua_version().cloned().unwrap())
            .unwrap();
        let dest_dir = assert_fs::TempDir::new().unwrap();
        let rock_layout = RockLayout {
            rock_path: dest_dir.to_path_buf(),
            etc: dest_dir.join("etc"),
            lib: dest_dir.join("lib"),
            src: dest_dir.join("src"),
            bin: tree.bin(),
            conf: dest_dir.join("conf"),
            doc: dest_dir.join("doc"),
        };
        let lua_version = config.lua_version().unwrap_or(&LuaVersion::Lua51);
        let progress = MultiProgress::new(&config);
        let bar = progress.map(MultiProgress::new_bar);
        let lua = LuaInstallation::new(lua_version, &config, &bar)
            .await
            .unwrap();
        let project = Project::from(&project_root).unwrap().unwrap();
        let rockspec = project.toml().into_remote(None).unwrap();
        let progress = MultiProgress::new(&config);
        run_build(
            &rockspec,
            RunBuildArgs::new()
                .output_paths(&rock_layout)
                .no_install(false)
                .lua(&lua)
                .external_dependencies(&HashMap::default())
                .deploy(rockspec.deploy().current_platform())
                .config(&config)
                .tree(&tree)
                .build_dir(&build_dir)
                .progress(&progress.map(|p| p.new_bar()))
                .build(),
        )
        .await
        .unwrap();
        let foo_dir = dest_dir.child("src").child("foo");
        foo_dir.assert(predicate::path::is_dir());
        let foo_init = foo_dir.child("init.lua");
        foo_init.assert(predicate::path::is_file());
        foo_init.assert(predicate::str::contains("return true"));
        let foo_bar_dir = foo_dir.child("bar");
        foo_bar_dir.assert(predicate::path::is_dir());
        let foo_bar_init = foo_bar_dir.child("init.lua");
        foo_bar_init.assert(predicate::path::is_file());
        foo_bar_init.assert(predicate::str::contains("return true"));
        let foo_bar_baz = foo_bar_dir.child("baz.lua");
        foo_bar_baz.assert(predicate::path::is_file());
        foo_bar_baz.assert(predicate::str::contains("return true"));
        let bin_file = tree_dir
            .child(lua_version.to_string())
            .child("bin")
            .child("hello");
        bin_file.assert(predicate::path::is_file());
        bin_file.assert(predicate::str::contains("#!/usr/bin/env bash"));
        bin_file.assert(predicate::str::contains("echo \"Hello\""));
    }
}
