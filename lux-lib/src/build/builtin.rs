use itertools::Itertools;
use miette::Diagnostic;
use std::{
    collections::{HashMap, HashSet},
    io,
    path::{Path, PathBuf},
    str::FromStr,
};
use thiserror::Error;
use walkdir::{DirEntry, WalkDir};

use crate::{
    build::{
        backend::{BuildBackend, BuildInfo, RunBuildArgs},
        utils,
    },
    lua_rockspec::{BuiltinBuildSpec, LuaModule, ModuleSpec, ParseLuaModuleError},
    tree::{InstallTree, TreeError},
};

use super::utils::{CompileCFilesError, CompileCModulesError, InstallBinaryError};

#[derive(Error, Debug, Diagnostic)]
pub enum BuiltinBuildError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    CompileCFiles(#[from] CompileCFilesError),
    #[error(transparent)]
    #[diagnostic(transparent)]
    CompileCModules(#[from] CompileCModulesError),
    #[error("failed to install binary '{file_name}'")]
    InstallBinary {
        file_name: String,
        #[diagnostic_source]
        source: InstallBinaryError,
    },
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Tree(#[from] TreeError),
    #[error("module auto-detection failed")]
    #[diagnostic(transparent)]
    AutoDetectModules(#[from] AutoDetectModulesError),
}

impl BuildBackend for BuiltinBuildSpec {
    type Err = BuiltinBuildError;

    #[tracing::instrument(name = "🛠️ builtin::run", skip_all, level = "debug")]
    async fn run<T>(self, args: RunBuildArgs<'_, T>) -> Result<BuildInfo, Self::Err>
    where
        T: InstallTree + Sync,
    {
        let output_paths = args.output_paths;
        let lua = args.lua;
        let external_dependencies = args.external_dependencies;
        let config = args.config;
        let tree = args.tree;
        let build_dir = args.build_dir;

        // Detect all Lua modules
        let modules = autodetect_modules(build_dir, source_paths(build_dir, &self.modules))?
            .into_iter()
            .chain(self.modules)
            .collect::<HashMap<_, _>>();

        for (destination_path, module_type) in modules.iter() {
            match module_type {
                ModuleSpec::SourcePath(source) => {
                    if source.extension().map(|ext| ext == "c").unwrap_or(false) {
                        let absolute_source_paths = vec![build_dir.join(source)];
                        utils::compile_c_files(
                            &absolute_source_paths,
                            destination_path,
                            &output_paths.lib,
                            lua,
                            external_dependencies,
                            config,
                        )
                        .await?
                    } else {
                        let absolute_source_path = build_dir.join(source);
                        utils::copy_lua_to_module_path(
                            &absolute_source_path,
                            destination_path,
                            &output_paths.src,
                        )?
                    }
                }
                ModuleSpec::SourcePaths(files) => {
                    let absolute_source_paths =
                        files.iter().map(|file| build_dir.join(file)).collect();
                    utils::compile_c_files(
                        &absolute_source_paths,
                        destination_path,
                        &output_paths.lib,
                        lua,
                        external_dependencies,
                        config,
                    )
                    .await?
                }
                ModuleSpec::ModulePaths(data) => {
                    utils::compile_c_modules(
                        data,
                        build_dir,
                        destination_path,
                        &output_paths.lib,
                        lua,
                        external_dependencies,
                        config,
                    )
                    .await?
                }
            }
        }

        let mut binaries = Vec::new();
        for bin_script in autodetect_bin_scripts(build_dir) {
            if let Some(target) = bin_script.file_name() {
                let file_name = target.to_string_lossy().to_string();
                let installed_bin_script =
                    utils::install_binary(&bin_script, &file_name, tree, lua, args.deploy, config)
                        .await
                        .map_err(|err| BuiltinBuildError::InstallBinary {
                            file_name: file_name.clone(),
                            source: err,
                        })?;
                if let Some(bin_script_file_name) = installed_bin_script.file_name() {
                    binaries.push(bin_script_file_name.into());
                }
            }
        }

        Ok(BuildInfo { binaries })
    }
}

fn source_paths(build_dir: &Path, modules: &HashMap<LuaModule, ModuleSpec>) -> HashSet<PathBuf> {
    modules
        .values()
        .flat_map(|spec| match spec {
            ModuleSpec::SourcePath(path_buf) => vec![path_buf],
            ModuleSpec::SourcePaths(vec) => vec.iter().collect_vec(),
            ModuleSpec::ModulePaths(module_paths) => module_paths.sources.iter().collect_vec(),
        })
        .map(|path| build_dir.join(path))
        .collect()
}

#[derive(Error, Debug, Diagnostic)]
pub enum AutoDetectModulesError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    ParseLuaModule(#[from] ParseLuaModuleError),
}

fn autodetect_modules(
    build_dir: &Path,
    exclude: HashSet<PathBuf>,
) -> Result<HashMap<LuaModule, ModuleSpec>, AutoDetectModulesError> {
    WalkDir::new(build_dir.join("src"))
        .into_iter()
        .chain(WalkDir::new(build_dir.join("lua")))
        .chain(WalkDir::new(build_dir.join("lib")))
        .filter_map(|file| {
            file.ok().and_then(|file| {
                let is_lua_file = PathBuf::from(file.file_name())
                    .extension()
                    .map(|ext| ext == "lua")
                    .unwrap_or(false);
                if is_lua_file && !exclude.contains(&file.clone().into_path()) {
                    Some(file)
                } else {
                    None
                }
            })
        })
        .map(|file| {
            let diff: PathBuf = unsafe {
                pathdiff::diff_paths(build_dir.join(file.clone().into_path()), build_dir)
                    .unwrap_unchecked()
            };

            // NOTE(vhyrro): You may ask why we convert all paths to Lua module paths
            // just to convert them back later in the `run()` stage.
            //
            // The rockspec requires the format to be like this, and representing our
            // data in this form allows us to respect any overrides made by the user (which follow
            // the `module.name` format, not our internal one).
            let mut pathbuf = diff.components().skip(1).collect::<PathBuf>();
            let lua_module = if pathbuf
                .parent()
                .is_none_or(|parent| parent.as_os_str().is_empty())
            {
                pathbuf.set_extension("");
                Ok(LuaModule::from_pathbuf(pathbuf)?)
            } else {
                let mut lua_module = LuaModule::from_pathbuf(pathbuf)?;
                // NOTE(mrcjkb): `LuaModule` does not parse as "<module>.init" from files named "init.lua"
                // To make sure we don't change the file structure when installing, we append it here.
                if file.file_name().to_string_lossy().as_bytes() == b"init.lua" {
                    unsafe {
                        lua_module =
                            lua_module.join(&LuaModule::from_str("init").unwrap_unchecked())
                    }
                }
                Ok(lua_module)
            };
            lua_module.map(|lua_module| (lua_module, ModuleSpec::SourcePath(diff)))
        })
        .try_collect()
}

fn autodetect_bin_scripts(build_dir: &Path) -> Vec<PathBuf> {
    WalkDir::new(build_dir.join("src").join("bin"))
        .into_iter()
        .chain(WalkDir::new(build_dir.join("bin")))
        .filter_map(|file| file.ok())
        .filter(|file| file.clone().into_path().is_file())
        .map(DirEntry::into_path)
        .collect()
}
