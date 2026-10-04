use crate::build::backend::{BuildBackend, BuildInfo, RunBuildArgs};
use crate::fs;
use crate::lockfile::{LockfileError, RemotePackageSourceUrl};
use crate::lua_installation::LuaInstallationError;
use crate::lua_rockspec::LuaVersionError;
use crate::operations::UnpackError;
use crate::rockspec::Rockspec;
use crate::tree::{EntryType, InstallTree, TreeError};
use bytes::Bytes;
use std::collections::HashMap;
use std::fs::DirEntry;
use std::path::PathBuf;
use std::{io, path::Path};
use tracing::Instrument;

use crate::{
    config::Config,
    hash::HasIntegrity,
    lockfile::{LockedPackage, LockedPackageSpec},
    lua_installation::LuaInstallation,
    lua_rockspec::BuildBackendSpec,
    operations::FetchSrcError,
};
use builtin::BuiltinBuildError;
use cmake::CMakeError;
use command::CommandError;
use external_dependency::{ExternalDependencyError, ExternalDependencyInfo};

use itertools::Itertools;
use luarocks::LuarocksBuildError;
use make::MakeError;

use miette::Diagnostic;
use patch::{Patch, PatchError};
use rust_binary::RustBinaryError;
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
mod rust_binary;
mod rust_mlua;
mod source;
mod treesitter_parser;

pub(crate) mod backend;
pub(crate) mod utils;

pub mod external_dependency;

#[derive(Debug)]
pub(crate) enum RemotePackageSourceSpec {
    RockSpec(Option<RemotePackageSourceUrl>),
    SrcRock(SrcRockSource),
}

/// A packed .src.rock archive.
#[derive(Debug)]
pub(crate) struct SrcRockSource {
    pub bytes: Bytes,
    pub source_url: RemotePackageSourceUrl,
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum BuildError {
    #[error("builtin build failed")]
    #[diagnostic(forward(0))]
    Builtin(#[from] BuiltinBuildError),
    #[error("cmake build failed")]
    #[diagnostic(forward(0))]
    CMake(#[from] CMakeError),
    #[error("make build failed")]
    #[diagnostic(forward(0))]
    Make(#[from] MakeError),
    #[error("command build failed")]
    #[diagnostic(forward(0))]
    Command(#[from] CommandError),
    #[error("rust-mlua build failed")]
    #[diagnostic(forward(0))]
    Rust(#[from] RustError),
    #[error("rust-binary build failed")]
    #[diagnostic(forward(0))]
    RustBinary(#[from] RustBinaryError),
    #[error("treesitter-parser build failed")]
    #[diagnostic(forward(0))]
    TreesitterBuild(#[from] TreesitterBuildError),
    #[error("luarocks build failed")]
    #[diagnostic(forward(0))]
    LuarocksBuild(#[from] LuarocksBuildError),
    #[error("building from rock source failed")]
    #[diagnostic(forward(0))]
    SourceBuild(#[from] Box<SourceBuildError>),
    #[error("IO operation failed")]
    Io(#[from] io::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::FsError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Lockfile(#[from] LockfileError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] TreeError),

    #[error(transparent)]
    #[diagnostic(transparent)]
    ExternalDependencyError(#[from] ExternalDependencyError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    PatchError(#[from] PatchError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    CompileCFiles(#[from] CompileCFilesError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaVersion(#[from] LuaVersionError),
    #[error(
        r#"source integrity mismatch.
- source: {src}
- expected: {expected}
- got: {actual}"#
    )]
    #[diagnostic(help(
        r#"the source may have been modified or a tag may have been moved.
check the source, then rerun the command with `--no-lock` to update the hash."#
    ))]
    SourceIntegrityMismatch {
        src: String,
        expected: Integrity,
        actual: Integrity,
    },
    #[error("failed to unpack src.rock")]
    #[diagnostic(forward(0))]
    UnpackSrcRock(#[source] UnpackError),
    #[error("failed to fetch rock source")]
    #[diagnostic(forward(0))]
    FetchSrcError(#[from] FetchSrcError),
    #[error("failed to install binary '{file_name}'")]
    InstallBinary {
        file_name: String,
        #[diagnostic_source]
        source: InstallBinaryError,
    },
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaInstallation(#[from] LuaInstallationError),
}

impl From<SourceBuildError> for BuildError {
    fn from(source: SourceBuildError) -> Self {
        Self::SourceBuild(Box::new(source))
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum BuildBehaviour {
    /// Don't force a rebuild if the package is already installed
    #[default]
    NoForce,
    /// Force a rebuild if the package is already installed
    Force,
}

#[tracing::instrument(level = "trace", skip_all)]
async fn run_build<R: Rockspec + HasIntegrity, T: InstallTree + Sync>(
    rockspec: &R,
    args: RunBuildArgs<'_, T>,
) -> Result<BuildInfo, BuildError> {
    Ok(
        match rockspec.build().current_platform().build_backend.to_owned() {
            Some(BuildBackendSpec::Builtin(build_spec)) => build_spec.run(args).await?,
            Some(BuildBackendSpec::Make(make_spec)) => make_spec.run(args).await?,
            Some(BuildBackendSpec::CMake(cmake_spec)) => cmake_spec.run(args).await?,
            Some(BuildBackendSpec::Command(command_spec)) => command_spec.run(args).await?,
            Some(BuildBackendSpec::RustMlua(rust_mlua_spec)) => rust_mlua_spec.run(args).await?,
            Some(BuildBackendSpec::RustBinary(rust_binary_spec)) => {
                rust_binary_spec.run(args).await?
            }
            Some(BuildBackendSpec::TreesitterParser(treesitter_parser_spec)) => {
                treesitter_parser_spec.run(args).await?
            }
            Some(BuildBackendSpec::LuaRock(build_backend_name)) => {
                luarocks::build(&build_backend_name, rockspec, args).await?
            }
            Some(BuildBackendSpec::Source) => source::build(args).await?,
            None => BuildInfo::default(),
        },
    )
}

#[allow(clippy::too_many_arguments)]
#[tracing::instrument(level = "trace", skip(rockspec, tree, config))]
async fn install<R: Rockspec + HasIntegrity, T: InstallTree>(
    rockspec: &R,
    tree: &T,
    package: &LockedPackageSpec,
    lua: &LuaInstallation,
    build_dir: &Path,
    entry_type: &EntryType,
    config: &Config,
) -> Result<(), BuildError> {
    let install_spec = &rockspec.build().current_platform().install;
    let layout = tree.layout_for(package);
    {
        let span = tracing::info_span!("Copying Lua modules");
        let _enter = span.enter();
        for (target, source) in &install_spec.lua {
            let _enter = span.enter();
            let absolute_source = build_dir.join(source);
            utils::copy_lua_to_module_path(&absolute_source, target, &layout.src)?;
        }
    }
    {
        let span = tracing::info_span!("Compiling C libraries");
        let _enter = span.enter();
        for (target, source) in &install_spec.lib {
            let absolute_source = build_dir.join(source);
            let resolved_target = layout.lib.join(target);
            fs::tokio::copy(&absolute_source, &resolved_target)
                .instrument(tracing::trace_span!("copying target"))
                .await?;
            utils::make_writable(&resolved_target).await?;
        }
    }
    if entry_type.is_entrypoint() {
        let span = tracing::info_span!("Installing binaries");
        let _enter = span.enter();
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
            .instrument(tracing::trace_span!("installing binary"))
            .await
            .map_err(|err| BuildError::InstallBinary {
                file_name: target.clone(),
                source: err,
            })?;
        }
    }
    if !install_spec.conf.is_empty() {
        let span = tracing::info_span!("Copying configuration files");
        let _enter = span.enter();
        for (target, source) in &install_spec.conf {
            let absolute_source = build_dir.join(source);
            let target = layout.conf.join(target);
            if let Some(parent_dir) = target.parent() {
                fs::tokio::create_dir_all(parent_dir)
                    .instrument(tracing::trace_span!("creating configuration directory"))
                    .await?;
            }
            fs::tokio::copy(&absolute_source, &target)
                .instrument(tracing::trace_span!("copying configuration file"))
                .await?;
            utils::make_writable(&target).await?;
        }
    }
    Ok(())
}

// TODO(vhyrro): break apart deployment into a separate step (once we implement transactions)
#[allow(clippy::too_many_arguments)]
pub(crate) async fn deploy<R: Rockspec + HasIntegrity, T: InstallTree + Sync>(
    rockspec: &R,
    tree: &T,
    package: LockedPackage,
    lua: &LuaInstallation,
    source_root: &Path,
    entry_type: EntryType,
    config: &Config,
    behaviour: BuildBehaviour,
) -> Result<LockedPackage, BuildError> {
    if behaviour == BuildBehaviour::NoForce {
        if let Some(existing) = tree.lockfile()?.get(&package.id()) {
            return Ok(existing.clone());
        }
    }

    // FIXME(vhyrro): Maybe make prepare/finalize a struct with Drop behaviour? If it makes sense only.
    tree.prepare(&package.spec)?;
    let layout = tree.layout_for(&package.spec);

    let rock_source = rockspec.source().current_platform();
    let build_dir = resolve_source_dir(
        source_root,
        rock_source.unpack_dir.as_deref(),
        &rockspec.build().current_platform().copy_directories,
    )?;

    Patch::new(&build_dir, &rockspec.build().current_platform().patches).apply()?;

    let external_dependencies = rockspec
        .external_dependencies()
        .current_platform()
        .iter()
        .map(|(name, dep)| {
            ExternalDependencyInfo::probe(name, dep, config.external_deps())
                .map(|info| (name.clone(), info))
        })
        .try_collect::<_, HashMap<_, _>, _>()?;

    let output = run_build(
        rockspec,
        RunBuildArgs::new()
            .package(&package.spec)
            .no_install(false)
            .lua(lua)
            .external_dependencies(&external_dependencies)
            .deploy(rockspec.deploy().current_platform())
            .config(config)
            .tree(tree)
            .build_dir(&build_dir)
            .build(),
    )
    .await?;

    let mut binaries = rockspec.binaries();
    binaries.extend(output.binaries);
    tree.lockfile()?
        .write_guard()
        .set_binaries(&package, binaries);

    install(
        rockspec,
        tree,
        &package.spec,
        lua,
        &build_dir,
        &entry_type,
        config,
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
        recursive_copy_dir(&build_dir.join(directory), &layout.etc.join(directory)).await?;
    }

    recursive_copy_doc_dir(&layout.doc, &build_dir).await?;

    if let Ok(rockspec_str) = rockspec.to_lua_remote_rockspec_string() {
        fs::sync::write(layout.rockspec_path(), rockspec_str)?;
    }

    tree.finalize(&package, entry_type)?;

    Ok(package)
}

fn is_source_or_etc_dir(dir: &DirEntry, copy_dirs: &[PathBuf]) -> bool {
    let dir_name = dir.file_name().to_string_lossy().to_string();
    matches!(dir_name.as_str(), "lua" | "src")
        || copy_dirs
            .iter()
            .any(|copy_dir_name| copy_dir_name == &PathBuf::from(&dir_name))
}

/// Resolve the directory containing a rock's sources within an extracted source archive.
/// When the rockspec has no `source.dir`, fall back to the archive's single top-level directory.
pub(crate) fn resolve_source_dir(
    source_root: &Path,
    unpack_dir: Option<&Path>,
    copy_dirs: &[PathBuf],
) -> Result<PathBuf, fs::FsError> {
    if let Some(unpack_dir) = unpack_dir {
        return Ok(source_root.join(unpack_dir));
    }
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
    let has_lua_or_c_sources = fs::sync::read_dir(source_root)?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .any(|entry| {
            entry.path().extension().is_some_and(|ext| {
                matches!(ext.to_string_lossy().to_string().as_str(), "lua" | "c")
            })
        });
    if has_lua_or_c_sources {
        return Ok(source_root.to_path_buf());
    }
    let dir_entries = fs::sync::read_dir(source_root)?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect_vec();
    if dir_entries.len() == 1 && !is_source_or_etc_dir(&dir_entries[0], copy_dirs) {
        Ok(dir_entries[0].path())
    } else {
        Ok(source_root.to_path_buf())
    }
}

#[tracing::instrument(level = "trace")]
async fn recursive_copy_doc_dir(target_doc_dir: &Path, build_dir: &Path) -> Result<(), BuildError> {
    let mut doc_dir = build_dir.join("doc");
    if !doc_dir.exists() {
        doc_dir = build_dir.join("docs");
    }
    recursive_copy_dir(&doc_dir, target_doc_dir).await?;
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
        config::ConfigBuilder,
        lockfile::{LockConstraint, LockedPackageHashes},
        lua_installation::{detect_installed_lua_version, LuaInstallation},
        lua_version::LuaVersion,
        operations::{unpack_rockspec, DownloadedPackedRockBytes},
        package::PackageSpec,
        project::Project,
        remote_package_source::RemotePackageSource,
        tree::Tree,
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
        let lua_version = config.lua_version().unwrap_or(&LuaVersion::Lua51);
        let lua = LuaInstallation::new(lua_version, &config).await.unwrap();
        let project = Project::from_exact(&project_root).unwrap().unwrap();
        let rockspec = project.toml().into_remote(None).unwrap();
        let package = LockedPackage::from(
            &PackageSpec::new(rockspec.package().clone(), rockspec.version().clone()),
            LockConstraint::Unconstrained,
            RemotePackageSource::Test,
            None,
            LockedPackageHashes {
                rockspec: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                    .parse()
                    .unwrap(),
                source: "sha256-uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek="
                    .parse()
                    .unwrap(),
            },
        );
        tree.prepare(&package.spec).unwrap();
        let src_dir = tree.layout_for(&package.spec).src;
        run_build(
            &rockspec,
            RunBuildArgs::new()
                .package(&package.spec)
                .no_install(false)
                .lua(&lua)
                .external_dependencies(&HashMap::default())
                .deploy(rockspec.deploy().current_platform())
                .config(&config)
                .tree(&tree)
                .build_dir(&build_dir)
                .build(),
        )
        .await
        .unwrap();
        let foo_dir = src_dir.join("foo");
        assert!(foo_dir.is_dir());
        let foo_init = foo_dir.join("init.lua");
        assert!(foo_init.is_file());
        assert!(std::fs::read_to_string(&foo_init)
            .unwrap()
            .contains("return true"));
        let foo_bar_dir = foo_dir.join("bar");
        assert!(foo_bar_dir.is_dir());
        let foo_bar_init = foo_bar_dir.join("init.lua");
        assert!(foo_bar_init.is_file());
        assert!(std::fs::read_to_string(&foo_bar_init)
            .unwrap()
            .contains("return true"));
        let foo_bar_baz = foo_bar_dir.join("baz.lua");
        assert!(foo_bar_baz.is_file());
        assert!(std::fs::read_to_string(&foo_bar_baz)
            .unwrap()
            .contains("return true"));
        let bin_file = tree_dir
            .child(lua_version.to_string())
            .child("bin")
            .child("hello");
        bin_file.assert(predicate::path::is_file());
        bin_file.assert(predicate::str::contains("#!/usr/bin/env bash"));
        bin_file.assert(predicate::str::contains("echo \"Hello\""));
    }

    const LUATEST_SRC_ROCK: &str = "resources/test/luatest-0.2-1.src.rock";
    /// `sha256` of the checked-in `.src.rock` archive, i.e. what `Materialize` must
    /// record as the `source` integrity.
    const LUATEST_SRC_ROCK_SOURCE_HASH: &str =
        "sha256-2jS0XOq0iIVhsZJ3BVqXSlKsx2vsqCAaaYyqcBEb7RI=";
    /// `sha256` of the rockspec text embedded in the `.src.rock`, i.e. the `rockspec` integrity.
    const LUATEST_ROCKSPEC_HASH: &str = "sha256-NljJ20A+VadUyhhBjrRnojeQjSqRpQy7FWcgjUt2Fdc=";

    fn luatest_src_rock_bytes() -> Bytes {
        Bytes::copy_from_slice(
            &std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(LUATEST_SRC_ROCK))
                .unwrap(),
        )
    }

    fn luatest_config(dir: &assert_fs::TempDir) -> Config {
        ConfigBuilder::new()
            .unwrap()
            .user_tree(Some(dir.to_path_buf()))
            .lua_version(Some(LuaVersion::Lua51))
            .build()
            .unwrap()
    }

    /// Builds `luatest` from the checked-in `.src.rock` fixture, entirely offline.
    /// This exercises the fetch + hash + build + deploy path end to end.
    async fn build_luatest(
        config: &Config,
        tree: &Tree,
        entry_type: EntryType,
        behaviour: BuildBehaviour,
    ) -> LockedPackage {
        let bytes = luatest_src_rock_bytes();
        let rock = DownloadedPackedRockBytes {
            name: "luatest".into(),
            version: "0.2-1".parse().unwrap(),
            bytes: bytes.clone(),
            file_name: "luatest-0.2-1.src.rock".into(),
            url: "https://example.org/luatest-0.2-1.src.rock"
                .parse()
                .unwrap(),
        };
        let rockspec = unpack_rockspec(&rock).await.unwrap();
        let lua = LuaInstallation::new_from_config(config).await.unwrap();
        crate::operations::pipeline::build_local::Build::new()
            .rockspec(&rockspec)
            .lua(&lua)
            .tree(tree)
            .entry_type(entry_type)
            .config(config)
            .behaviour(behaviour)
            .source_spec(RemotePackageSourceSpec::SrcRock(SrcRockSource {
                bytes,
                source_url: RemotePackageSourceUrl::Url {
                    url: "https://example.org/luatest-0.2-1.src.rock"
                        .parse()
                        .unwrap(),
                },
            }))
            .build()
            .await
            .unwrap()
    }

    /// The materialized `LockedPackage` must carry the exact source and rockspec
    /// integrities, derived from the fetched bytes. This is the contract that
    /// `lx generate-lockfile` and the manifest cache depend on.
    #[tokio::test]
    async fn materialize_src_rock_produces_expected_hashes() {
        let dir = assert_fs::TempDir::new().unwrap();
        let config = luatest_config(&dir);
        let tree = config.user_tree(LuaVersion::Lua51).unwrap();

        let package =
            build_luatest(&config, &tree, EntryType::Entrypoint, BuildBehaviour::Force).await;

        assert_eq!(
            package.hashes().source,
            LUATEST_SRC_ROCK_SOURCE_HASH.parse().unwrap()
        );
        assert_eq!(
            package.hashes().rockspec,
            LUATEST_ROCKSPEC_HASH.parse().unwrap()
        );
        // The source integrity is the hash of the fetched artifact.
        assert_eq!(
            package.hashes().source,
            luatest_src_rock_bytes().hash().await.unwrap()
        );
        // Entrypoints expose their binaries.
        assert!(tree.bin().join("luatest").is_file());
    }

    /// Two independent forced builds of the same rockspec must produce identical
    /// lockfile entries (id, source, source_url, hashes). This guards the
    /// determinism that reproducible lockfiles depend on.
    #[tokio::test]
    async fn force_build_is_deterministic() {
        let dir1 = assert_fs::TempDir::new().unwrap();
        let config1 = luatest_config(&dir1);
        let tree1 = config1.user_tree(LuaVersion::Lua51).unwrap();

        let dir2 = assert_fs::TempDir::new().unwrap();
        let config2 = luatest_config(&dir2);
        let tree2 = config2.user_tree(LuaVersion::Lua51).unwrap();

        let package1 = build_luatest(
            &config1,
            &tree1,
            EntryType::Entrypoint,
            BuildBehaviour::Force,
        )
        .await;
        let package2 = build_luatest(
            &config2,
            &tree2,
            EntryType::Entrypoint,
            BuildBehaviour::Force,
        )
        .await;

        assert_eq!(package1, package2);
        assert_eq!(package1.hashes(), package2.hashes());
    }

    /// `NoForce` must short-circuit on the tree lockfile without touching the
    /// tree, while `Force` must rebuild and redeploy.
    #[tokio::test]
    async fn noforce_skips_rebuild_when_package_is_locked() {
        let dir = assert_fs::TempDir::new().unwrap();
        let config = luatest_config(&dir);
        let tree = config.user_tree(LuaVersion::Lua51).unwrap();

        let package =
            build_luatest(&config, &tree, EntryType::Entrypoint, BuildBehaviour::Force).await;
        let bin = tree.bin().join("luatest");
        assert!(bin.is_file());

        // Simulate a committed install by writing the package into the tree lockfile.
        {
            let mut lockfile = tree.lockfile().unwrap().write_guard();
            lockfile.add_entrypoint(&package);
        }

        // Simulate a partially broken install.
        std::fs::remove_file(&bin).unwrap();

        let skipped = build_luatest(
            &config,
            &tree,
            EntryType::Entrypoint,
            BuildBehaviour::NoForce,
        )
        .await;
        assert_eq!(skipped.id(), package.id());
        assert!(
            !bin.is_file(),
            "NoForce must not rebuild a package that is already in the lockfile"
        );

        build_luatest(&config, &tree, EntryType::Entrypoint, BuildBehaviour::Force).await;
        assert!(bin.is_file(), "Force must rebuild the package");
    }
}
