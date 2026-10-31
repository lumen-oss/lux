use crate::build::backend::{BuildBackend, BuildInfo, RunBuildArgs};
use crate::fs;
use crate::lockfile::{self, RemotePackageSourceUrl};
use crate::operations::{fetch, unpack};
use crate::project;
use crate::rockspec::Rockspec;
use crate::tree::{self, EntryType, InstallTree};
use bon::Builder;
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
    lua, rockspec,
};
use external_dependency::ExternalDependencyInfo;

use itertools::Itertools;

use miette::Diagnostic;
use patch::Patch;
use ssri::Integrity;
use thiserror::Error;

pub mod builtin;
pub mod cmake;
pub mod command;
pub mod compile;
pub mod install;
pub mod luarocks;
#[cfg(feature = "luau-transpile")]
pub mod luau;
pub mod make;
pub mod patch;
pub mod rust_binary;
pub mod rust_mlua;
pub mod source;
pub mod treesitter_parser;

pub(crate) mod backend;
pub(crate) mod cargo;
pub(crate) use builtin::autodetect_modules;

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
pub enum Error {
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
    #[error("luarocks build failed")]
    #[diagnostic(forward(0))]
    LuarocksBuild(#[from] luarocks::Error),
    #[error("building from rock source failed")]
    #[diagnostic(forward(0))]
    SourceBuild(#[from] Box<source::Error>),
    #[error("unsupported build backend for luau")]
    #[diagnostic(help("lux can build luau packages with the following build backends: 'builtin', 'source' & 'none'"))]
    LuauUnsupportedBuildBackend,
    #[error("luau does not support native dependencies")]
    #[diagnostic(help("the luau runtime cannot load C libraries. use pure-Luau packages or Lua"))]
    LuauNativeDepsUnsupported,
    #[error("IO operation failed")]
    Io(#[from] io::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Fs(#[from] fs::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Lockfile(#[from] lockfile::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] tree::Error),

    #[error(transparent)]
    #[diagnostic(transparent)]
    ExternalDependency(#[from] external_dependency::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Patch(#[from] patch::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    CompileCFiles(#[from] compile::c_files::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaVersion(#[from] rockspec::Error),
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
    UnpackSrcRock(#[source] unpack::Error),
    #[error("failed to fetch rock source")]
    #[diagnostic(forward(0))]
    FetchSrc(#[from] fetch::Error),
    #[error("failed to install binary '{file_name}'")]
    InstallBinary {
        file_name: String,
        #[diagnostic_source]
        source: install::Error,
    },
    #[error(transparent)]
    #[diagnostic(transparent)]
    LuaInstallation(#[from] lua::installation::Error),
}

impl From<source::Error> for Error {
    fn from(source: source::Error) -> Self {
        Self::SourceBuild(Box::new(source))
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum BuildBehaviour {
    /// If a package with the same version is already installed, do nothing.
    /// Otherwise, behave like [`BuildBehaviour::Conflict`].
    Ignore,
    /// Error if an entrypoint with the same name is already installed, even if
    /// it is the same version.
    Conflict,
    /// Remove any installed entrypoint with the same name and rebuild.
    Force,
}

#[tracing::instrument(level = "trace", skip_all)]
async fn run_build<R: Rockspec + HasIntegrity, T: InstallTree + Sync>(
    rockspec: &R,
    args: RunBuildArgs<'_, T>,
) -> Result<BuildInfo, Error> {
    if args.tree.version() == &lua::Version::Luau
        && rockspec
            .build()
            .current_platform()
            .backend
            .as_ref()
            .is_some_and(|backend| {
                !matches!(
                    backend,
                    rockspec::build::backend::Spec::Builtin(_)
                        | rockspec::build::backend::Spec::Source
                )
            })
    {
        Err(Error::LuauUnsupportedBuildBackend)
    } else {
        Ok(
            match rockspec.build().current_platform().backend.to_owned() {
                Some(rockspec::build::backend::Spec::Builtin(build_spec)) => {
                    build_spec.run(args).await?
                }
                Some(rockspec::build::backend::Spec::Make(make_spec)) => {
                    make_spec.run(args).await?
                }
                Some(rockspec::build::backend::Spec::CMake(cmake_spec)) => {
                    cmake_spec.run(args).await?
                }
                Some(rockspec::build::backend::Spec::Command(command_spec)) => {
                    command_spec.run(args).await?
                }
                Some(rockspec::build::backend::Spec::RustMlua(rust_mlua_spec)) => {
                    rust_mlua_spec.run(args).await?
                }
                Some(rockspec::build::backend::Spec::RustBinary(rust_binary_spec)) => {
                    rust_binary_spec.run(args).await?
                }
                Some(rockspec::build::backend::Spec::TreesitterParser(treesitter_parser_spec)) => {
                    treesitter_parser_spec.run(args).await?
                }
                Some(rockspec::build::backend::Spec::LuaRock(build_backend_name)) => {
                    luarocks::build(&build_backend_name, rockspec, args).await?
                }
                Some(rockspec::build::backend::Spec::Source) => source::build(args).await?,
                None => BuildInfo::default(),
            },
        )
    }
}

#[allow(clippy::too_many_arguments)]
#[tracing::instrument(level = "trace", skip(rockspec, tree, config))]
async fn install<R: Rockspec + HasIntegrity, T: InstallTree>(
    rockspec: &R,
    tree: &T,
    package: &LockedPackageSpec,
    lua: &lua::Installation,
    build_dir: &Path,
    entry_type: &EntryType,
    config: &Config,
) -> Result<(), Error> {
    let install_spec = &rockspec.build().current_platform().install;
    let layout = tree.layout_for(package);
    {
        let span = tracing::info_span!("Copying Lua modules");
        let _enter = span.enter();
        for (target, source) in &install_spec.lua {
            let _enter = span.enter();
            let absolute_source = build_dir.join(source);
            install::lua_module(&absolute_source, target, &layout.src)?;
        }
    }
    {
        let span = tracing::info_span!("Compiling C libraries");
        let _enter = span.enter();
        for (target, source) in &install_spec.lib {
            let absolute_source = build_dir.join(source);
            let resolved_target = layout.lib.join(target);
            // NOTE: Some rockspecs have redundant module definitions,
            // assuming that the linker outputs them to the build dir root.
            // See https://github.com/lumen-oss/lux/issues/2101
            if !resolved_target.exists() {
                if let Some(parent) = resolved_target.parent() {
                    fs::tokio::create_dir_all(&parent).await?;
                }
                fs::tokio::copy(&absolute_source, &resolved_target)
                    .instrument(tracing::trace_span!("copying target"))
                    .await?;
            }
            fs::make_writable(&resolved_target).await?;
        }
    }
    if entry_type.is_entrypoint() {
        let span = tracing::info_span!("Installing binaries");
        let _enter = span.enter();
        let deploy_spec = rockspec.deploy().current_platform();
        for (target, source) in &install_spec.bin {
            install::binary(
                &build_dir.join(source),
                target,
                tree,
                lua,
                deploy_spec,
                config,
            )
            .instrument(tracing::trace_span!("installing binary"))
            .await
            .map_err(|err| Error::InstallBinary {
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
            let target = {
                let joined = layout.conf.join(target);
                match joined.file_name() {
                    Some(_) => joined,
                    None => joined.join(absolute_source.file_name().unwrap_or_default()),
                }
            };
            if let Some(parent_dir) = target.parent() {
                fs::tokio::create_dir_all(parent_dir)
                    .instrument(tracing::trace_span!("creating configuration directory"))
                    .await?;
            }
            fs::tokio::copy(&absolute_source, &target)
                .instrument(tracing::trace_span!("copying configuration file"))
                .await?;
            fs::make_writable(&target).await?;
        }
    }
    Ok(())
}

// TODO(vhyrro): break apart deployment into a separate step (once we implement transactions)
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub(crate) struct Deploy<'a, R: Rockspec + HasIntegrity, T: InstallTree + Sync> {
    #[builder(start_fn)]
    rockspec: &'a R,
    #[builder(start_fn)]
    tree: &'a T,
    package: LockedPackage,
    lua: &'a lua::Installation,
    source_root: &'a Path,
    entry_type: EntryType,
    config: &'a Config,
    behaviour: BuildBehaviour,
}

impl<R, T, State> DeployBuilder<'_, R, T, State>
where
    R: Rockspec + HasIntegrity,
    T: InstallTree + Sync,
    State: deploy_builder::State + deploy_builder::IsComplete,
{
    pub(crate) async fn deploy(self) -> Result<LockedPackage, Error> {
        do_deploy(self._build()).await
    }
}

#[tracing::instrument(level = "trace", skip_all, fields(package = deploy.rockspec.package().to_string()))]
async fn do_deploy<R, T>(deploy: Deploy<'_, R, T>) -> Result<LockedPackage, Error>
where
    R: Rockspec + HasIntegrity,
    T: InstallTree + Sync,
{
    let Deploy {
        rockspec,
        tree,
        package,
        lua,
        source_root,
        entry_type,
        config,
        behaviour,
    } = deploy;

    if matches!(behaviour, BuildBehaviour::Ignore | BuildBehaviour::Conflict) {
        if let Some(existing) = tree.lockfile()?.get(&package.id()) {
            return Ok(existing.clone());
        }
    }

    // TODO(vhyrro): When we implement transactionality make this a Drop guard
    // or something else.
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
        project::copy_dir(&build_dir.join(directory), &layout.etc.join(directory)).await?;
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
) -> Result<PathBuf, fs::Error> {
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
                matches!(
                    ext.to_string_lossy().to_string().as_str(),
                    "lua" | "luau" | "c"
                )
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
async fn recursive_copy_doc_dir(target_doc_dir: &Path, build_dir: &Path) -> Result<(), Error> {
    let mut doc_dir = build_dir.join("doc");
    if !doc_dir.exists() {
        doc_dir = build_dir.join("docs");
    }
    project::copy_dir(&doc_dir, target_doc_dir).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package;
    use crate::remote;
    use predicates::prelude::*;
    use std::{
        io::{Cursor, Write},
        path::PathBuf,
    };
    use zip::write::SimpleFileOptions;

    use assert_fs::{
        assert::PathAssert,
        prelude::{PathChild, PathCopy},
    };

    use is_executable::IsExecutable;

    use crate::pipeline::build_local::Build;
    use crate::{
        config,
        lockfile::{self, LockedPackageHashes, LockedPackageSpec, OptState, PinnedState},
        lua,
        project::Project,
    };
    #[cfg(feature = "impure_tests")]
    use crate::{
        operations::{self, download},
        tree::Tree,
    };

    #[tokio::test]
    async fn test_builtin_build() {
        let lua_version = lua::detect::installed_version().or(Some(lua::Version::Lua51));
        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/test/sample-projects/no-build-spec/");
        let tree_dir = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::new()
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
        let lua_version = config.lua_version().unwrap_or(&lua::Version::Lua51);
        let lua = lua::Installation::new(lua_version, &config).await.unwrap();
        let project = Project::from_exact(&project_root).unwrap().unwrap();
        let rockspec = project.toml().into_remote(None).unwrap();
        let package = LockedPackage::from(
            &package::Spec::new(rockspec.package().clone(), rockspec.version().clone()),
            lockfile::Constraint::Unconstrained,
            remote::package::Source::Test,
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

    #[cfg(unix)]
    #[tokio::test]
    async fn test_install_bin_sets_executable_permissions() {
        let rockspec = rockspec::lua::Remote::new(
            r#"
rockspec_format = "3.0"
package = "test-builtin-bin"
version = "1.0-1"
source = { url = "https://github.com/lumen-oss/luarocks-stub" }
build = {
    type = "builtin",
    install = { bin = { mytool = "mytool" } },
}
deploy = { wrap_bin_scripts = false }
"#,
        )
        .unwrap();
        let lua_version = lua::detect::installed_version().or(Some(lua::Version::Lua51));
        let tree_dir = assert_fs::TempDir::new().unwrap();
        let build_dir = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::new()
            .unwrap()
            .lua_version(lua_version)
            .user_tree(Some(tree_dir.to_path_buf()))
            .build()
            .unwrap();
        let lua = lua::Installation::new_from_config(&config).await.unwrap();
        let tree = config
            .user_tree(config.lua_version().cloned().unwrap())
            .unwrap();
        let package = LockedPackageSpec::new(
            rockspec.package(),
            rockspec.version(),
            lockfile::Constraint::Unconstrained,
            Vec::new(),
            Vec::new(),
            &PinnedState::default(),
            &OptState::default(),
        );
        tree.prepare(&package).unwrap();
        crate::fs::tokio::write(build_dir.path().join("mytool"), "#!/bin/sh\necho hi\n")
            .await
            .unwrap();
        install(
            &rockspec,
            &tree,
            &package,
            &lua,
            build_dir.path(),
            &EntryType::Entrypoint,
            &config,
        )
        .await
        .unwrap();
        assert!(tree.bin().join("mytool").is_executable());
    }

    #[tokio::test]
    async fn git_src_rock_with_repo_directory_builds() {
        let dir = assert_fs::TempDir::new().unwrap();
        let lua_version = lua::detect::installed_version().or(Some(lua::Version::Lua51));
        let config = config::Builder::new()
            .unwrap()
            .user_tree(Some(dir.to_path_buf()))
            .lua_version(lua_version)
            .build()
            .unwrap();
        let lua = lua::Installation::new_from_config(&config).await.unwrap();
        let tree = config.user_tree(lua.version.clone()).unwrap();

        let rockspec_content = r#"
package = "git-src-rock"
version = "1.0.0-1"
source = { url = "git+https://example.com/example/git-src-rock.git" }
dependencies = { "lua >= 5.1" }
build = {
    type = "builtin",
    modules = { ["git_src_rock"] = "lua/git_src_rock.lua" },
}
"#;
        let rockspec = rockspec::lua::Remote::new(rockspec_content).unwrap();

        let mut archive = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut archive);
            zip.start_file(
                "git-src-rock-1.0.0-1.rockspec",
                SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(rockspec_content.as_bytes()).unwrap();
            zip.start_file(
                "git-src-rock/lua/git_src_rock.lua",
                SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(b"return {}").unwrap();
            zip.finish().unwrap();
        }
        let bytes = Bytes::from(archive.into_inner());

        let package = Build::new()
            .rockspec(&rockspec)
            .lua(&lua)
            .tree(&tree)
            .entry_type(EntryType::Entrypoint)
            .config(&config)
            .behaviour(BuildBehaviour::Force)
            .source_spec(RemotePackageSourceSpec::SrcRock(SrcRockSource {
                bytes,
                source_url: RemotePackageSourceUrl::Url {
                    url: "https://example.org/git-src-rock-1.0.0-1.src.rock"
                        .parse()
                        .unwrap(),
                },
            }))
            .build()
            .await
            .unwrap();

        assert!(tree
            .layout_for(package.spec())
            .src
            .join("git_src_rock.lua")
            .is_file());
    }

    #[tokio::test]
    async fn test_install_conf_directory_target() {
        let rockspec = rockspec::lua::Remote::new(
            r#"
rockspec_format = "3.0"
package = "test-install-conf"
version = "1.0-1"
source = { url = "https://github.com/lumen-oss/luarocks-stub" }
build = {
    type = "builtin",
    install = {
        conf = {
            [".."] = "shell.nix",
            ["plain.conf"] = "plain.conf",
        },
    },
}
"#,
        )
        .unwrap();
        let lua_version = lua::detect::installed_version().or(Some(lua::Version::Lua51));
        let tree_dir = assert_fs::TempDir::new().unwrap();
        let build_dir = assert_fs::TempDir::new().unwrap();
        let config = config::Builder::new()
            .unwrap()
            .lua_version(lua_version)
            .user_tree(Some(tree_dir.to_path_buf()))
            .build()
            .unwrap();
        let lua = lua::Installation::new_from_config(&config).await.unwrap();
        let tree = config
            .user_tree(config.lua_version().cloned().unwrap())
            .unwrap();
        let package = LockedPackageSpec::new(
            rockspec.package(),
            rockspec.version(),
            lockfile::Constraint::Unconstrained,
            Vec::new(),
            Vec::new(),
            &PinnedState::default(),
            &OptState::default(),
        );
        tree.prepare(&package).unwrap();
        for file in ["shell.nix", "plain.conf"] {
            crate::fs::tokio::write(build_dir.path().join(file), "return {}")
                .await
                .unwrap();
        }
        install(
            &rockspec,
            &tree,
            &package,
            &lua,
            build_dir.path(),
            &EntryType::Entrypoint,
            &config,
        )
        .await
        .unwrap();
        let layout = tree.layout_for(&package);
        assert!(layout.etc.join("shell.nix").is_file());
        assert!(layout.conf.join("plain.conf").is_file());
    }

    #[cfg(feature = "impure_tests")]
    const LUATEST_SRC_ROCK: &str = "resources/test/luatest-0.2-1.src.rock";
    #[cfg(feature = "impure_tests")]
    const LUATEST_SRC_ROCK_SOURCE_HASH: &str =
        "sha256-2jS0XOq0iIVhsZJ3BVqXSlKsx2vsqCAaaYyqcBEb7RI=";
    #[cfg(feature = "impure_tests")]
    const LUATEST_ROCKSPEC_HASH: &str = "sha256-NljJ20A+VadUyhhBjrRnojeQjSqRpQy7FWcgjUt2Fdc=";

    #[cfg(feature = "impure_tests")]
    fn luatest_src_rock_bytes() -> Bytes {
        Bytes::copy_from_slice(
            &std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(LUATEST_SRC_ROCK))
                .unwrap(),
        )
    }

    #[cfg(feature = "impure_tests")]
    fn luatest_config(dir: &assert_fs::TempDir) -> Config {
        config::Builder::new()
            .unwrap()
            .user_tree(Some(dir.to_path_buf()))
            .lua_version(Some(lua::Version::Lua51))
            .build()
            .unwrap()
    }

    #[cfg(feature = "impure_tests")]
    async fn build_luatest(
        config: &Config,
        tree: &Tree,
        entry_type: EntryType,
        behaviour: BuildBehaviour,
    ) -> LockedPackage {
        let bytes = luatest_src_rock_bytes();
        let rock = download::PackedRockBytes {
            name: "luatest".into(),
            version: "0.2-1".parse().unwrap(),
            bytes: bytes.clone(),
            file_name: "luatest-0.2-1.src.rock".into(),
            url: "https://example.org/luatest-0.2-1.src.rock"
                .parse()
                .unwrap(),
        };
        let rockspec = download::unpack_rockspec(&rock).await.unwrap();
        let lua = lua::Installation::new_from_config(config).await.unwrap();
        Build::new()
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

    #[cfg(feature = "impure_tests")]
    #[tokio::test]
    async fn luatest_src_rock_produces_expected_hashes() {
        let dir = assert_fs::TempDir::new().unwrap();
        let config = luatest_config(&dir);
        let tree = config.user_tree(lua::Version::Lua51).unwrap();

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
        assert_eq!(
            package.hashes().source,
            luatest_src_rock_bytes().hash().await.unwrap()
        );
        assert!(tree.bin().join("luatest").is_file());
    }

    #[cfg(feature = "impure_tests")]
    #[tokio::test]
    async fn force_build_is_deterministic() {
        let dir1 = assert_fs::TempDir::new().unwrap();
        let config1 = luatest_config(&dir1);
        let tree1 = config1.user_tree(lua::Version::Lua51).unwrap();

        let dir2 = assert_fs::TempDir::new().unwrap();
        let config2 = luatest_config(&dir2);
        let tree2 = config2.user_tree(lua::Version::Lua51).unwrap();

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

    #[cfg(feature = "impure_tests")]
    #[tokio::test]
    async fn noforce_skips_rebuild_when_package_is_locked() {
        let dir = assert_fs::TempDir::new().unwrap();
        let config = luatest_config(&dir);
        let tree = config.user_tree(lua::Version::Lua51).unwrap();

        let package =
            build_luatest(&config, &tree, EntryType::Entrypoint, BuildBehaviour::Force).await;
        let bin = tree.bin().join("luatest");
        assert!(bin.is_file());

        // Simulate a committed install
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
            BuildBehaviour::Ignore,
        )
        .await;
        assert_eq!(skipped.id(), package.id());
        assert!(!bin.is_file());

        build_luatest(&config, &tree, EntryType::Entrypoint, BuildBehaviour::Force).await;
        assert!(bin.is_file());
    }
}
